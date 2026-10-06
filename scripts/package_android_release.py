#!/usr/bin/env python3
"""Package a video-enabled ARM64 build using Android's official signing tools.

Preserves the Android shell, replaces launcher icons, updates the manifest version,
replaces native libraries, removes stale signatures, aligns, signs and verifies.
Passwords are read by apksigner from caller-named environment variables.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import zipfile
import zlib

from dex_density_dpi import patch_density_dpi, verify_density_dpi


VIDEO_MARKER = b"PHIRA_REPLICA_VIDEO_ENABLED_FFMPEG_V1\0"
PRACTICE_LAYOUT_MARKER = b"PHIRAIAD_PRACTICE_LAYOUT_70_SHIFT10_BN_ROWS_V2\0"
PACKAGE_NAME = "org.flos.phira.replica"
CERT_SHA256 = "a59eb8345a617d28e3b2032849aa043d1b00fcbc2545a436fdc15c57bab29936"
DEFAULT_APP_LABEL = "PhiraiAd"
ANDROID_ATTR_LABEL = 0x01010001


def run(argv):
    """Run a tool without a shell; propagate nonzero exit status and diagnostics."""
    return subprocess.check_output([str(x) for x in argv], stderr=subprocess.STDOUT, text=True)


def prepare_icon_replacements(base_apk, icon_path, badging):
    """Replace the verified r23.0 launcher PNG without rebuilding resources.arsc."""
    launcher = "res/mipmap/icon.png"
    paths = set(re.findall(r"^application-icon-[^:]+:'([^']+)'", badging, re.MULTILINE))
    if paths != {launcher}:
        raise ValueError(f"Unsupported launcher resources: {sorted(paths)}; expected {launcher}")
    data = icon_path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Icon must be a PNG image")
    pos, kinds = 8, []
    while pos < len(data):
        if pos + 12 > len(data):
            raise ValueError("Truncated PNG chunk")
        length = struct.unpack_from(">I", data, pos)[0]
        end = pos + 12 + length
        if end > len(data):
            raise ValueError("Truncated PNG payload")
        kind = data[pos + 4:pos + 8]
        if zlib.crc32(data[pos + 4:end - 4]) != struct.unpack_from(">I", data, end - 4)[0]:
            raise ValueError("Invalid PNG checksum")
        if not kinds:
            if kind != b"IHDR" or length != 13:
                raise ValueError("Invalid PNG header")
            width, height = struct.unpack_from(">II", data, pos + 8)
            if width != height or not 1 <= width <= 4096:
                raise ValueError("Icon must be square and at most 4096 x 4096 pixels")
        kinds.append(kind)
        pos = end
        if kind == b"IEND":
            if length or pos != len(data):
                raise ValueError("Invalid PNG ending")
            break
    if not kinds or kinds[-1] != b"IEND" or b"IDAT" not in kinds:
        raise ValueError("Incomplete PNG image")
    with zipfile.ZipFile(base_apk) as archive:
        names = set(archive.namelist())
        if launcher not in names:
            raise ValueError("Base APK is missing its launcher PNG")
        replacements = {launcher: data}
        if "assets/icon.png" in names:
            replacements["assets/icon.png"] = data
    return replacements


def check_arm64(data):
    """Reject a missing/incorrect ELF ABI before replacing an APK library."""
    if data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != 183:
        raise ValueError("Expected an ELF64 little-endian AArch64 library")


def replace_version_string(chunk, old_name, new_name):
    """Append a version string and retarget its pool index without moving other strings."""
    _, header, size = struct.unpack_from("<HHI", chunk)
    count, styles, flags, start, _ = struct.unpack_from("<5I", chunk, 8)
    if styles:
        raise ValueError("Styled manifest string pools are unsupported")
    utf8 = bool(flags & 0x100)
    def length_at(offset):
        if utf8:
            n = chunk[offset]
            return (((n & 0x7f) << 8) | chunk[offset+1], offset+2) if n & 0x80 else (n, offset+1)
        n = struct.unpack_from("<H", chunk, offset)[0]
        return (((n & 0x7fff) << 16) | struct.unpack_from("<H", chunk, offset+2)[0], offset+4) if n & 0x8000 else (n, offset+2)
    matches = []
    for i in range(count):
        offset = start + struct.unpack_from("<I", chunk, header + 4*i)[0]
        n, offset = length_at(offset)
        if utf8:
            n, offset = length_at(offset)
            value = chunk[offset:offset+n].decode("utf-8")
        else:
            value = chunk[offset:offset+2*n].decode("utf-16le")
        if value == old_name:
            matches.append(i)
    if len(matches) != 1:
        raise ValueError(f"Expected one version-name string, found {len(matches)}")
    units = len(new_name.encode("utf-16le")) // 2
    if utf8:
        def length(n):
            if n > 0x7fff: raise ValueError("Version string too long")
            return bytes([0x80 | (n >> 8), n & 0xff]) if n > 0x7f else bytes([n])
        value = new_name.encode("utf-8")
        encoded = length(units) + length(len(value)) + value + b"\0"
    else:
        if units > 0x7fff: raise ValueError("Version string too long")
        encoded = struct.pack("<H", units) + new_name.encode("utf-16le") + b"\0\0"
    result = bytearray(chunk + encoded)
    result.extend(b"\0" * (-len(result) % 4))
    struct.pack_into("<I", result, header + matches[0]*4, size-start)
    struct.pack_into("<I", result, 4, len(result))
    struct.pack_into("<I", result, 16, flags & ~1)  # The renamed pool need not remain sorted.
    return bytes(result)


def _decode_string_pool(chunk):
    """Return all strings in an unstyled Android string-pool chunk."""
    _, header, size = struct.unpack_from("<HHI", chunk)
    count, styles, flags, start, _ = struct.unpack_from("<5I", chunk, 8)
    if styles:
        raise ValueError("Styled manifest string pools are unsupported")
    utf8 = bool(flags & 0x100)

    def length_at(offset):
        if utf8:
            n = chunk[offset]
            return ((((n & 0x7f) << 8) | chunk[offset + 1]), offset + 2) if n & 0x80 else (n, offset + 1)
        n = struct.unpack_from("<H", chunk, offset)[0]
        return ((((n & 0x7fff) << 16) | struct.unpack_from("<H", chunk, offset + 2)[0]), offset + 4) if n & 0x8000 else (n, offset + 2)

    values = []
    for i in range(count):
        offset = start + struct.unpack_from("<I", chunk, header + 4 * i)[0]
        n, offset = length_at(offset)
        if utf8:
            n, offset = length_at(offset)
            values.append(chunk[offset:offset + n].decode("utf-8"))
        else:
            values.append(chunk[offset:offset + 2 * n].decode("utf-16le"))
    return values, flags, header


def _encode_pool_string(value, utf8):
    units = len(value.encode("utf-16le")) // 2
    if utf8:
        raw = value.encode("utf-8")
        def enc_len(n):
            if n > 0x7fff:
                raise ValueError("Manifest string too long")
            return bytes([0x80 | (n >> 8), n & 0xff]) if n > 0x7f else bytes([n])
        return enc_len(units) + enc_len(len(raw)) + raw + b"\0"
    if units > 0x7fffffff:
        raise ValueError("Manifest string too long")
    if units > 0x7fff:
        prefix = struct.pack("<HH", 0x8000 | (units >> 16), units & 0xffff)
    else:
        prefix = struct.pack("<H", units)
    return prefix + value.encode("utf-16le") + b"\0\0"


def append_manifest_string(chunk, value):
    """Append a string while preserving all existing string-pool indices."""
    values, flags, header = _decode_string_pool(chunk)
    new_index = len(values)
    values.append(value)
    utf8 = bool(flags & 0x100)
    offsets = []
    payload = bytearray()
    for item in values:
        offsets.append(len(payload))
        payload.extend(_encode_pool_string(item, utf8))
    payload.extend(b"\0" * (-len(payload) % 4))
    strings_start = header + 4 * len(values)
    out = bytearray(chunk[:header])
    struct.pack_into("<I", out, 8, len(values))
    struct.pack_into("<I", out, 12, 0)
    struct.pack_into("<I", out, 16, flags & ~1)
    struct.pack_into("<I", out, 20, strings_start)
    struct.pack_into("<I", out, 24, 0)
    out.extend(struct.pack(f"<{len(offsets)}I", *offsets))
    out.extend(payload)
    struct.pack_into("<I", out, 4, len(out))
    return bytes(out), new_index, values


def _manifest_chunks(data):
    pos = struct.unpack_from("<H", data, 2)[0]
    while pos < len(data):
        kind, header, size = struct.unpack_from("<HHI", data, pos)
        if size < header or size == 0 or pos + size > len(data):
            raise ValueError("Invalid AXML chunk")
        yield pos, kind, header, size
        pos += size


def _component_role(name, values):
    """Use stable component/intent metadata, never the old user-visible label."""
    def detect(text):
        text = re.sub(r"[^a-z0-9]", "", text.lower())
        if text in {"res", "resource"} or any(word in text for word in ("respack", "resourcepack", "texturepack", "importresource")):
            return "respack"
        if "chart" in text:
            return "chart"
        return None
    # Only the last class-name segment: package names must not select a role.
    role = detect(name.rsplit(".", 1)[-1])
    if role:
        return role
    roles = {detect(value) for value in values} - {None}
    if len(roles) == 1:
        return roles.pop()
    if len(roles) > 1:
        raise ValueError("Ambiguous chart/resource-pack Android entry: " + name)
    # The original Android shell uses ImportActivity for chart import and
    # accepts */*, so it has neither "chart" in its name nor a ZIP MIME hint.
    # Prefer explicit metadata above; only this known generic component falls back.
    if name.rsplit(".", 1)[-1].lower() == "importactivity":
        return "chart"
    return None


def patch_manifest_label(data, app_label):
    """Assign launcher/chart/respack labels independently, including old patched APKs."""
    data = bytearray(data)
    label_indices = {}
    strings = None
    for pos, kind, header, size in _manifest_chunks(data):
        if kind == 1:
            pool = bytes(data[pos:pos + size])
            for role, label in (("app", app_label), ("chart", "导入到" + app_label + "(谱面)"),
                                ("respack", "导入到" + app_label + "(资源包)")):
                pool, index, strings = append_manifest_string(pool, label)
                label_indices[role] = index
            data[pos:pos + size] = pool
            break
    if strings is None:
        raise ValueError("Manifest string pool not found")
    struct.pack_into("<I", data, 4, len(data))

    resource_ids = []
    components = []
    stack = []
    android_name = 0x01010003
    android_ns = "http://schemas.android.com/apk/res/android"
    label_attr_name = None
    for pos, kind, header, size in _manifest_chunks(data):
        if kind == 0x0180:
            resource_ids = list(struct.unpack_from(f"<{(size - header) // 4}I", data, pos + header))
            if ANDROID_ATTR_LABEL in resource_ids:
                label_attr_name = resource_ids.index(ANDROID_ATTR_LABEL)
        elif kind == 0x0102:
            ext = pos + header
            name_index = struct.unpack_from("<I", data, ext + 4)[0]
            element = strings[name_index]
            start, stride, count = struct.unpack_from("<HHH", data, ext + 8)
            if stride < 20 or ext + start + stride * count > pos + size:
                raise ValueError("Invalid AXML attributes")
            attributes = []
            for i in range(count):
                attr = ext + start + i * stride
                namespace, attr_name, raw = struct.unpack_from("<III", data, attr)
                resource = resource_ids[attr_name] if attr_name < len(resource_ids) else 0
                value_type, value = data[attr + 15], struct.unpack_from("<I", data, attr + 16)[0]
                text_index = raw if raw != 0xffffffff else value if value_type == 3 else 0xffffffff
                text = strings[text_index] if text_index < len(strings) else ""
                attributes.append((attr, resource, text, namespace))
            current = stack[-1] if stack else None
            if element in {"application", "activity", "activity-alias"}:
                component_name = next((text for _, resource, text, _ in attributes if resource == android_name), "")
                current = {"pos": pos, "header": header, "size": size, "ext": ext, "start": start,
                           "stride": stride, "count": count, "element": element, "name": component_name,
                           "values": [], "labels": [a for a in attributes if a[1] == ANDROID_ATTR_LABEL],
                           "launcher": False, "file_handler": False,
                           "has_component_label": any(a[1] == ANDROID_ATTR_LABEL for a in attributes)}
                components.append(current)
            # Exclude current labels and targetActivity names from role detection.
            if current and element not in {"application", "activity", "activity-alias"}:
                if element == "intent-filter":
                    current["labels"].extend(a for a in attributes if a[1] == ANDROID_ATTR_LABEL)
                for _, resource, text, _ in attributes:
                    if resource == ANDROID_ATTR_LABEL:
                        continue
                    current["values"].append(text)
                    current["launcher"] |= text == "android.intent.category.LAUNCHER"
                    current["file_handler"] |= text in {"application/zip", "application/x-zip-compressed"}
            stack.append(current)
        elif kind == 0x0103:
            if not stack:
                raise ValueError("Unbalanced AXML")
            stack.pop()
    if label_attr_name is None:
        raise ValueError("Manifest android:label resource ID missing")
    namespace_index = strings.index(android_ns)
    changes = 0
    # Backwards mutation allows insertion into nodes without invalidating earlier offsets.
    for component in reversed(components):
        role = "app" if component["element"] == "application" or component["launcher"] else _component_role(component["name"], component["values"])
        if role is None and component["file_handler"]:
            raise ValueError("Cannot identify ZIP entry as chart or respack: " + component["name"])
        if role is None:
            continue  # Unrelated activities retain their labels.
        label_index = label_indices[role]
        if component["labels"]:
            for attr, _, _, _ in component["labels"]:
                struct.pack_into("<I", data, attr + 8, label_index)
                struct.pack_into("<HBBI", data, attr + 12, 8, 0, 3, label_index)
        if not component["has_component_label"]:
            attr = component["ext"] + component["start"] + component["count"] * component["stride"]
            encoded = struct.pack("<IIIHBBI", namespace_index, label_attr_name, label_index, 8, 0, 3, label_index)
            encoded += bytes(component["stride"] - 20)
            data[attr:attr] = encoded
            struct.pack_into("<H", data, component["ext"] + 12, component["count"] + 1)
            struct.pack_into("<I", data, component["pos"] + 4, component["size"] + component["stride"])
        changes += 1
        print("Android entry label:", component["name"] or component["element"], "->", strings[label_index])
    if not changes:
        raise ValueError("No Android application/entry label patched")
    struct.pack_into("<I", data, 4, len(data))
    return bytes(data)


def patch_manifest(data, old_name, new_name, version_code, app_label):
    """Update AXML versionCode and versionName, including longer patch-version names."""
    data = bytearray(data)
    offset = struct.unpack_from("<H", data, 2)[0]
    pool_count = 0
    while offset < len(data):
        kind, header, size = struct.unpack_from("<HHI", data, offset)
        if size < header or size == 0 or offset + size > len(data):
            raise ValueError("Invalid AXML chunk")
        if kind == 1:
            pool = replace_version_string(bytes(data[offset:offset+size]), old_name, new_name)
            data[offset:offset+size] = pool
            size = len(pool)
            pool_count += 1
        offset += size
    if pool_count != 1:
        raise ValueError("Expected one manifest string pool")
    struct.pack_into("<I", data, 4, len(data))
    pos = struct.unpack_from("<H", data, 2)[0]
    resource_ids = []
    changes = 0
    while pos < len(data):
        kind, header, size = struct.unpack_from("<HHI", data, pos)
        if size < header or size == 0 or pos + size > len(data):
            raise ValueError("Invalid AXML chunk")
        if kind == 0x0180:
            resource_ids = list(struct.unpack_from(f"<{(size-header)//4}I", data, pos+header))
        elif kind == 0x0102:
            ext = pos + header
            start, stride, count = struct.unpack_from("<HHH", data, ext + 8)
            for index in range(count):
                attr = ext + start + index * stride
                name_index = struct.unpack_from("<I", data, attr + 4)[0]
                if name_index < len(resource_ids) and resource_ids[name_index] == 0x0101021B:
                    if data[attr + 15] not in (0x10, 0x11):
                        raise ValueError("versionCode must be an integer")
                    struct.pack_into("<I", data, attr + 16, version_code)
                    changes += 1
        pos += size
    if changes != 1:
        raise ValueError(f"Expected one versionCode attribute, found {changes}")
    return patch_manifest_label(bytes(data), app_label)


def prepare_font_replacements(base_apk: Path) -> dict[str, bytes]:
    """Always replace inherited fonts with the checked-in official Phira fonts."""
    font_dir = Path(__file__).resolve().parents[1] / "assets"
    replacements = {f"assets/{name}": (font_dir / name).read_bytes()
                    for name in ("font.ttf", "bold.ttf", "phigros.ttf")}
    with zipfile.ZipFile(base_apk) as base:
        missing = set(replacements) - set(base.namelist())
        if missing:
            raise ValueError(f"Base APK is missing font assets: {sorted(missing)}")
    return replacements


def verify_native_build_markers(lib: bytes):
    if VIDEO_MARKER not in lib:
        raise ValueError("Native library has no video build marker; rebuild with --features video")
    if PRACTICE_LAYOUT_MARKER not in lib:
        raise ValueError("Native library is missing the updated practice layout / Bn marker; "
                         "rebuild phira and use the new libphira.so instead of an older build")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("base-apk", "native-lib", "cxx-lib", "build-tools", "keystore", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--key-alias", required=True)
    parser.add_argument("--ks-pass-env", default="PHIRA_KS_PASS")
    parser.add_argument("--key-pass-env", default="PHIRA_KEY_PASS")
    parser.add_argument("--version-name", default="0.8.2-replica23.0-bugfix1")
    parser.add_argument("--version-code", type=int, default=10033)
    parser.add_argument("--app-label", default=DEFAULT_APP_LABEL)
    icon_group = parser.add_mutually_exclusive_group()
    icon_group.add_argument("--icon", type=Path,
                            default=Path(__file__).resolve().parents[1] / "assets" / "icon.png",
                            help="Launcher PNG (default: project assets/icon.png)")
    icon_group.add_argument("--keep-base-icon", action="store_true",
                            help="Preserve the base APK icons")
    args = parser.parse_args()
    for name in (args.ks_pass_env, args.key_pass_env):
        if not os.environ.get(name):
            parser.error(f"Missing password environment variable: {name}")

    lib, cxx = args.native_lib.read_bytes(), args.cxx_lib.read_bytes()
    check_arm64(lib)
    check_arm64(cxx)
    verify_native_build_markers(lib)
    aapt, align, signer = [args.build_tools / n for n in ("aapt", "zipalign", "apksigner")]
    before = run([aapt, "dump", "badging", args.base_apk])
    if f"name='{PACKAGE_NAME}'" not in before.splitlines()[0]:
        raise ValueError("The Android shell must belong to Phira Replica")
    icon_replacements = {} if args.keep_base_icon else prepare_icon_replacements(args.base_apk, args.icon, before)
    font_replacements = prepare_font_replacements(args.base_apk)
    asset_replacements = {**icon_replacements, **font_replacements}
    old_name = re.search(r"versionName='([^']+)'", before).group(1)
    old_code = int(re.search(r"versionCode='(\d+)'", before).group(1))
    if args.version_code <= old_code:
        raise ValueError("The new versionCode must be greater than the base APK's")

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="phira-package-", dir=args.out.parent) as tmp:
        tmp = Path(tmp)
        unsigned, aligned, signed = [tmp / n for n in ("unsigned.apk", "aligned.apk", "signed.apk")]
        with zipfile.ZipFile(args.base_apk) as src, zipfile.ZipFile(unsigned, "w") as dst:
            if len(src.namelist()) != len(set(src.namelist())):
                raise ValueError("Base APK contains duplicate entries")
            seen = set()
            for info in src.infolist():
                name = info.filename
                if name.startswith("META-INF/"):
                    continue
                content = src.read(info)
                if name in asset_replacements:
                    content = asset_replacements[name]
                elif name == "AndroidManifest.xml":
                    content = patch_manifest(content, old_name, args.version_name, args.version_code, args.app_label)
                elif name == "classes.dex":
                    content = patch_density_dpi(content)
                elif name == "lib/arm64-v8a/libphira.so":
                    content = lib
                    seen.add(name)
                elif name == "lib/arm64-v8a/libc++_shared.so":
                    content = cxx
                    seen.add(name)
                info.extra = b""  # Old alignment padding is regenerated below.
                dst.writestr(info, content)
            if len(seen) != 2:
                raise ValueError("Base APK does not contain the expected ARM64 library slots")

        run([align, "-f", "-P", "16", "4", unsigned, aligned])
        run([signer, "sign", "--ks", args.keystore, "--ks-key-alias", args.key_alias,
             "--ks-pass", "env:" + args.ks_pass_env, "--key-pass", "env:" + args.key_pass_env,
             "--v1-signing-enabled", "true", "--v2-signing-enabled", "true",
             "--v3-signing-enabled", "true", "--v4-signing-enabled", "false",
             "--out", signed, aligned])
        verification = run([signer, "verify", "--verbose", "--print-certs", signed])
        if CERT_SHA256 not in verification:
            raise ValueError("Signing certificate differs from the existing Phira Replica certificate")
        for scheme in ("v1", "v2", "v3"):
            if not re.search(rf"Verified using {scheme} scheme[^\n]*: true", verification):
                raise ValueError(f"APK did not verify with {scheme}")
        run([align, "-c", "-P", "16", "-v", "4", signed])
        badging = run([aapt, "dump", "badging", signed])
        if f"versionCode='{args.version_code}'" not in badging or f"versionName='{args.version_name}'" not in badging:
            raise ValueError("Final manifest version validation failed")
        if f"application-label:'{args.app_label}'" not in badging:
            raise ValueError("Final application label validation failed")
        with zipfile.ZipFile(signed) as z:
            if z.testzip() is not None:
                raise ValueError("Final APK ZIP CRC verification failed")
            for name, expected in asset_replacements.items():
                if z.read(name) != expected:
                    raise ValueError(f"Final APK asset validation failed: {name}")
            verify_density_dpi(z.read("classes.dex"))
            if z.read("lib/arm64-v8a/libphira.so") != lib:
                raise ValueError("Final APK does not contain the intended native library")
            if z.read("lib/arm64-v8a/libc++_shared.so") != cxx:
                raise ValueError("Final APK does not contain the intended C++ runtime")
            with zipfile.ZipFile(args.base_apk) as original:
                def payload_names(archive):
                    return {i.filename for i in archive.infolist()
                            if not i.is_dir() and not i.filename.startswith("META-INF/")}
                names = payload_names(original)
                if names != payload_names(z):
                    raise ValueError("Final APK added or lost a non-signature file")
                expected_changes = {"AndroidManifest.xml", "lib/arm64-v8a/libphira.so",
                                    "lib/arm64-v8a/libc++_shared.so", "classes.dex"}
                expected_changes.update(asset_replacements)
                if z.read("classes.dex") != patch_density_dpi(original.read("classes.dex")):
                    raise ValueError("Unexpected DEX changes")
                if any(original.read(n) != z.read(n) for n in names - expected_changes):
                    raise ValueError("Final APK unexpectedly changed an Android shell/resource file")
        signed.replace(args.out)

    report = {"file": args.out.name, "sizeBytes": args.out.stat().st_size,
              "sha256": hashlib.sha256(args.out.read_bytes()).hexdigest(),
              "videoBuildMarker": True, "practiceLayoutRevision": "70_SHIFT10_BN_ROWS_V2", "certificateSha256": CERT_SHA256,
              "package": badging.splitlines()[0], "signatureVerification": verification,
              "alignmentVerified": True, "androidDpiSource": "DisplayMetrics.densityDpi",
              "appLabel": args.app_label, "deviceInstallationTested": False,
              "fontSha256": {n: hashlib.sha256(data).hexdigest() for n, data in font_replacements.items()},
              "iconResources": sorted(icon_replacements),
              "iconSha256": hashlib.sha256(next(iter(icon_replacements.values()))).hexdigest() if icon_replacements else None}
    report_path = args.out.with_suffix(".verification.json")
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.output) from error
