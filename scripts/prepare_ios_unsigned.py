"""Prepare runtime resources and unsigned iOS bundle configuration."""
import plistlib
import shutil
import sys
import zipfile
from pathlib import Path, PurePosixPath

REQUIRED = ("font.ttf", "bold.ttf", "phigros.ttf", "background.jpg", "res/bgm")


def check_assets(root):
    missing = [name for name in REQUIRED if not (root / name).is_file()]
    if missing:
        raise ValueError("Missing runtime assets: " + ", ".join(missing))


def fill_assets(apk):
    destination = Path("assets").resolve()
    with zipfile.ZipFile(apk) as archive:
        for entry in archive.infolist():
            if not entry.filename.startswith("assets/") or entry.is_dir():
                continue
            relative = PurePosixPath(entry.filename[len("assets/"):])
            if relative.is_absolute() or ".." in relative.parts or "\\" in str(relative):
                raise ValueError("Unsafe asset path: " + entry.filename)
            target = destination.joinpath(*relative.parts).resolve()
            if destination not in target.parents:
                raise ValueError("Asset escapes destination: " + entry.filename)
            if target.exists():
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
    check_assets(destination)


def write_plist(path):
    check_assets(Path("assets"))
    config = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleExecutable": "phira-main",
        "CFBundleIdentifier": "$(PRODUCT_BUNDLE_IDENTIFIER)",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "$(PRODUCT_NAME)",
        "CFBundleDisplayName": "PhiraiAd",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "$(MARKETING_VERSION)",
        "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
        "LSRequiresIPhoneOS": True,
        "UILaunchStoryboardName": "LaunchScreen",
        "UIRequiresFullScreen": True,
        "UIStatusBarHidden": True,
        "UIViewControllerBasedStatusBarAppearance": False,
        "UISupportedInterfaceOrientations": [
            "UIInterfaceOrientationLandscapeLeft", "UIInterfaceOrientationLandscapeRight"
        ],
        "NSPhotoLibraryAddUsageDescription": "用于将 Bn 成绩长图保存到系统相册。",
        "NSPhotoLibraryUsageDescription": "用于将 Bn 成绩长图保存到系统相册。",
        "UIFileSharingEnabled": True,
        "LSSupportsOpeningDocumentsInPlace": True,
        "CFBundleURLTypes": [{"CFBundleURLName": "Phira", "CFBundleURLSchemes": ["phira"]}],
    }
    Path(path).write_bytes(plistlib.dumps(config))


def verify_bundle(app):
    app = Path(app)
    with (app / "Info.plist").open("rb") as source:
        config = plistlib.load(source)
    expected = {"CFBundleExecutable": "phira-main", "CFBundleDisplayName": "PhiraiAd",
                "CFBundleShortVersionString": "1.1.0", "CFBundlePackageType": "APPL"}
    for name, value in expected.items():
        if config.get(name) != value:
            raise ValueError(f"Unexpected {name}: {config.get(name)!r}")
    for key in ("NSPhotoLibraryAddUsageDescription", "NSPhotoLibraryUsageDescription"):
        if not isinstance(config.get(key), str) or not config[key].strip():
            raise ValueError(f"Missing photo library usage description: {key}")
    if config.get("CFBundleSupportedPlatforms") != ["iPhoneOS"]:
        raise ValueError("Bundle is not an iPhoneOS device build")
    if not (app / "phira-main").is_file():
        raise ValueError("Missing iOS executable")
    check_assets(app / "assets")
    print("Device bundle and required assets verified")


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("assets", "plist", "verify"):
        raise SystemExit("Usage: prepare_ios_unsigned.py {assets|plist|verify} PATH")
    {"assets": fill_assets, "plist": write_plist, "verify": verify_bundle}[sys.argv[1]](sys.argv[2])
