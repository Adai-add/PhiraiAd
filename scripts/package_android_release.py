#!/usr/bin/env python3
"""Package a video-enabled ARM64 build using Android's official signing tools.

Preserves the existing Android shell/resources, updates the manifest version,
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

from dex_density_dpi import patch_density_dpi, verify_density_dpi


VIDEO_MARKER = b"PHIRA_REPLICA_VIDEO_ENABLED_FFMPEG_V1\0"
PACKAGE_NAME = "org.flos.phira.replica"
CERT_SHA256 = "a59eb8345a617d28e3b2032849aa043d1b00fcbc2545a436fdc15c57bab29936"
DEFAULT_APP_LABEL = "PhiraiAd"
ANDROID_ATTR_LABEL = 0x01010001


def run(argv):
    """Run a tool without a shell; propagate nonzero exit status and diagnostics."""
    return subprocess.check_output([str(x) for x in argv], stderr=subprocess.STDOUT, text=True)


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


def patch_manifest_label(data, app_label):
    """Force the Android application/activity label to a literal manifest string."""
    data = bytearray(data)
    offset = struct.unpack_from("<H", data, 2)[0]
    strings = None
    label_index = None
    while offset < len(data):
        kind, header, size = struct.unpack_from("<HHI", data, offset)
        if kind == 1:
            pool, label_index, strings = append_manifest_string(bytes(data[offset:offset + size]), app_label)
            data[offset:offset + size] = pool
            size = len(pool)
            break
        offset += size
    if strings is None or label_index is None:
        raise ValueError("Manifest string pool not found")
    struct.pack_into("<I", data, 4, len(data))

    pos = struct.unpack_from("<H", data, 2)[0]
    resource_ids = []
    changes = 0
    while pos < len(data):
        kind, header, size = struct.unpack_from("<HHI", data, pos)
        if size < header or size == 0 or pos + size > len(data):
            raise ValueError("Invalid AXML chunk")
        if kind == 0x0180:
            resource_ids = list(struct.unpack_from(f"<{(size - header) // 4}I", data, pos + header))
        elif kind == 0x0102:
            ext = pos + header
            element_name_index = struct.unpack_from("<I", data, ext + 4)[0]
            element_name = strings[element_name_index] if element_name_index < len(strings) else ""
            # Application is the authoritative app label; explicit activity labels can
            # override it on launchers, so normalize those too when present.
            if element_name in {"application", "activity", "activity-alias"}:
                start, stride, count = struct.unpack_from("<HHH", data, ext + 8)
                for index in range(count):
                    attr = ext + start + index * stride
                    name_index = struct.unpack_from("<I", data, attr + 4)[0]
                    if name_index < len(resource_ids) and resource_ids[name_index] == ANDROID_ATTR_LABEL:
                        struct.pack_into("<I", data, attr + 8, label_index)  # rawValue
                        struct.pack_into("<H", data, attr + 12, 8)
                        data[attr + 14] = 0
                        data[attr + 15] = 0x03  # TYPE_STRING
                        struct.pack_into("<I", data, attr + 16, label_index)
                        changes += 1
        pos += size
    if changes < 1:
        raise ValueError("No android:label attribute found in application/activity manifest entries")
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
    args = parser.parse_args()
    for name in (args.ks_pass_env, args.key_pass_env):
        if not os.environ.get(name):
            parser.error(f"Missing password environment variable: {name}")

    lib, cxx = args.native_lib.read_bytes(), args.cxx_lib.read_bytes()
    check_arm64(lib)
    check_arm64(cxx)
    if VIDEO_MARKER not in lib:
        raise ValueError("Native library has no video build marker; rebuild with --features video")
    aapt, align, signer = [args.build_tools / n for n in ("aapt", "zipalign", "apksigner")]
    before = run([aapt, "dump", "badging", args.base_apk])
    if f"name='{PACKAGE_NAME}'" not in before.splitlines()[0]:
        raise ValueError("The Android shell must belong to Phira Replica")
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
                if name == "AndroidManifest.xml":
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
                if z.read("classes.dex") != patch_density_dpi(original.read("classes.dex")):
                    raise ValueError("Unexpected DEX changes")
                if any(original.read(n) != z.read(n) for n in names - expected_changes):
                    raise ValueError("Final APK unexpectedly changed an Android shell/resource file")
        signed.replace(args.out)

    report = {"file": args.out.name, "sizeBytes": args.out.stat().st_size,
              "sha256": hashlib.sha256(args.out.read_bytes()).hexdigest(),
              "videoBuildMarker": True, "certificateSha256": CERT_SHA256,
              "package": badging.splitlines()[0], "signatureVerification": verification,
              "alignmentVerified": True, "androidDpiSource": "DisplayMetrics.densityDpi",
              "appLabel": args.app_label, "deviceInstallationTested": False}
    report_path = args.out.with_suffix(".verification.json")
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.output) from error
