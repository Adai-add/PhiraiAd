"""WSL build worker. Configuration contains paths only; secrets stay in env."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

TOOLCHAIN = "nightly-2026-09-14"
EXPECTED_ROOT = Path("/home/adai_add/phira-build")
EXPECTED_OUTPUT = Path("/mnt/c/Users/admin/Desktop/phira-rep")
ASSETS = ("font.ttf", "bold.ttf", "phigros.ttf", "background.jpg", "res/bgm")


def run(args, *, cwd, env):
    # Never print environment variables, in particular signing passwords.
    print("Running:", " ".join(str(x) for x in args), flush=True)
    subprocess.run([str(x) for x in args], cwd=cwd, env=env, check=True)


def need(path):
    path = Path(path)
    if not path.exists():
        raise ValueError(f"Required path missing: {path}")
    return path


def checked_root(config):
    root = Path(config["root"])
    if root != EXPECTED_ROOT or root.is_symlink() or root.resolve() != EXPECTED_ROOT:
        raise ValueError("Source root must be the actual /home/adai_add/phira-build directory")
    need(root / "Cargo.toml")
    if Path(config["output"]) != EXPECTED_OUTPUT:
        raise ValueError("Unexpected output directory")
    return root


def build_environment(root):
    env = os.environ.copy()
    # Pin our output location so cleaning the known target actually clears builds.
    for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET"):
        env.pop(name, None)
    env["RUSTUP_TOOLCHAIN"] = TOOLCHAIN
    env["CARGO_TARGET_DIR"] = str(root / "target")
    return env


def check_resources(root):
    for relative in ASSETS:
        need(root / "assets" / relative)


def android_environment(config, root):
    env = build_environment(root)
    ndk = need(config["ndk"])
    binary = ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin"
    need(binary / "aarch64-linux-android21-clang")
    need(binary / "aarch64-linux-android21-clang++")
    env["ANDROID_NDK_HOME"] = str(ndk)
    # Java 17 is the previously verified Android signing toolchain.
    java = Path("/usr/lib/jvm/java-17-openjdk-amd64")
    if java.is_dir():
        env["JAVA_HOME"] = str(java)
        env["PATH"] = str(java / "bin") + os.pathsep + env.get("PATH", "")
    for name in ("aapt", "zipalign", "apksigner"):
        need(Path(config["buildTools"]) / name)
    need(config["baseApk"])
    need(config["keystore"])
    need(config["passwordFile"])
    return env


def read_password(path):
    data = Path(path).read_bytes()
    try:
        text = data.decode("utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig")
    except UnicodeError:
        raise ValueError("Password file must use UTF-8 or BOM-marked UTF-16 encoding") from None
    # Preserve all spaces and CR/LF characters. Do not strip the password.
    if not text or "\0" in text:
        raise ValueError("Password file is empty or contains a NUL character")
    return text


def apk_version_code(apk, config, env, root):
    completed = subprocess.run([str(Path(config["buildTools"]) / "aapt"), "dump", "badging", str(apk)],
                               cwd=root, env=env, capture_output=True, text=True, check=True)
    found = re.search(r"versionCode='(\d+)'", completed.stdout)
    if not found:
        raise ValueError(f"Unable to read versionCode from {apk}")
    return int(found[1])


def install_target(root, env, target):
    run(["rustup", "toolchain", "install", TOOLCHAIN, "--profile", "minimal"], cwd=root, env=env)
    run(["rustup", "target", "add", target, "--toolchain", TOOLCHAIN], cwd=root, env=env)


def android(config, root):
    env = android_environment(config, root)
    need(root / "scripts/build_android_release.sh")
    output = Path(config["output"])
    final = output / "PhiraiAd-arm64-new.apk"
    previous = apk_version_code(final, config, env, root) if final.exists() else 0
    base = apk_version_code(config["baseApk"], config, env, root)
    code = max(int(config["minimumVersionCode"]), base + 1, previous + 1)
    install_target(root, env, "aarch64-linux-android")
    password = read_password(config["passwordFile"])
    env["PHIRA_KS_PASS"] = password
    env["PHIRA_KEY_PASS"] = password
    with tempfile.TemporaryDirectory(prefix=".android-build-", dir=output) as directory:
        staged = Path(directory) / "PhiraiAd-arm64-new.apk"
        try:
            run(["bash", "scripts/build_android_release.sh", "--base-apk", config["baseApk"],
                 "--build-tools", config["buildTools"], "--keystore", config["keystore"],
                 "--key-alias", config["keyAlias"], "--version-name", config["version"],
                 "--version-code", str(code), "--out", staged], cwd=root, env=env)
        finally:
            env.pop("PHIRA_KS_PASS", None)
            env.pop("PHIRA_KEY_PASS", None)
            password = None
        with zipfile.ZipFile(staged) as archive:
            if archive.testzip() is not None:
                raise ValueError("APK CRC check failed")
        os.replace(staged, final)
    print(f"OK: {final} (versionCode {code})", flush=True)


def linux(config, root):
    check_resources(root)
    env = build_environment(root)
    install_target(root, env, "x86_64-unknown-linux-gnu")
    run(["cargo", "build", "--locked", "-p", "phira-main", "--bin", "phira-main", "--release",
         "--target", "x86_64-unknown-linux-gnu"], cwd=root, env=env)
    binary = need(root / "target/x86_64-unknown-linux-gnu/release/phira-main")
    dependencies = subprocess.run(["ldd", str(binary)], capture_output=True, text=True, env=env)
    print(dependencies.stdout, flush=True)
    if dependencies.returncode != 0 or "not found" in dependencies.stdout:
        raise ValueError("Linux runtime dependencies could not be resolved")
    output = Path(config["output"])
    final = output / "PhiraiAd-linux-new.tar.gz"
    with tempfile.TemporaryDirectory(prefix=".linux-build-", dir=output) as directory:
        stage = Path(directory)
        package = stage / "PhiraiAd-linux-new"
        package.mkdir()
        shutil.copy2(binary, package / "PhiraiAd")
        (package / "PhiraiAd").chmod(0o755)
        shutil.copytree(root / "assets", package / "assets")
        staged = stage / final.name
        with tarfile.open(staged, "w:gz") as archive:
            archive.add(package, arcname=package.name)
        with tarfile.open(staged, "r:gz") as archive:
            executable = archive.getmember("PhiraiAd-linux-new/PhiraiAd")
            if executable.mode & 0o111 == 0:
                raise ValueError("Linux executable permission missing")
            for name in ASSETS:
                archive.getmember("PhiraiAd-linux-new/assets/" + name)
        os.replace(staged, final)
    print(f"OK: {final}", flush=True)


def clean(root):
    target = root / "target"
    if target.is_symlink():
        raise ValueError("Refusing to delete a symlinked target directory")
    if target.exists():
        shutil.rmtree(target)
    print("Removed WSL Cargo target build cache; downloaded dependencies retained.", flush=True)


def check(config, root):
    for command in ("rustup", "cargo", "python3", "java", "ldd", "tar"):
        if shutil.which(command) is None:
            raise ValueError(f"Command not found: {command}")
    check_resources(root)
    android_environment(config, root)
    read_password(config["passwordFile"])
    print("WSL paths, runtime assets and Android signing inputs found. Password not displayed.", flush=True)


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("check", "clean", "android", "linux"):
        raise ValueError("Usage: build_all_new_wsl.py {check|clean|android|linux} CONFIG.json")
    config = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8-sig"))
    root = checked_root(config)
    Path(config["output"]).mkdir(parents=True, exist_ok=True)
    if sys.argv[1] == "clean":
        clean(root)
    else:
        {"check": check, "android": android, "linux": linux}[sys.argv[1]](config, root)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # subprocess exceptions include arguments only, never signing env values.
        print(f"FAILED: {error}", file=sys.stderr, flush=True)
        sys.exit(1)
