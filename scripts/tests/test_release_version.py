"""Check release identity and exercise production mobile version selection."""
import argparse
import importlib.util
import json
import plistlib
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseVersionTests(unittest.TestCase):
    def test_workspace_release_identity(self):
        release = json.loads((ROOT / "build_all_new.config.json").read_text())
        self.assertEqual(release["version"], "1.6.0")
        self.assertEqual(release["minimumVersionCode"], 16000)
        cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(cargo["workspace"]["package"]["version"], release["version"])
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        for package in lock["package"]:
            if package["name"] in {"phira", "phira-main", "prpr", "prpr-auto-offset", "prpr-auto-offset-cli"}:
                self.assertEqual(package["version"], release["version"])
        self.assertIn("MARKETING_VERSION=" + release["version"], (ROOT / "xcode/Shared.xcconfig").read_text())

    def test_ios_verifier_accepts_current_and_rejects_stale_version(self):
        ios = load_script("prepare_ios_unsigned")
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory)
            config = {"CFBundleExecutable": "phira-main", "CFBundleDisplayName": "PhiraiAd",
                      "CFBundleShortVersionString": ios.release_version(), "CFBundlePackageType": "APPL",
                      "CFBundleSupportedPlatforms": ["iPhoneOS"],
                      "NSPhotoLibraryAddUsageDescription": "Save scores", "NSPhotoLibraryUsageDescription": "Scores"}
            (app / "phira-main").touch()
            for asset in ios.REQUIRED:
                path = app / "assets" / asset
                path.parent.mkdir(parents=True, exist_ok=True)
                path.touch()
            (app / "Info.plist").write_bytes(plistlib.dumps(config))
            ios.verify_bundle(app)
            config["CFBundleShortVersionString"] = "1.5.2"
            (app / "Info.plist").write_bytes(plistlib.dumps(config))
            with self.assertRaisesRegex(ValueError, "CFBundleShortVersionString"):
                ios.verify_bundle(app)

    def test_android_parser_uses_release_config_and_keeps_overrides(self):
        android = load_script("package_android_release")
        original_parse = argparse.ArgumentParser.parse_args
        required = [argument for name in ("base-apk", "native-lib", "cxx-lib", "build-tools", "keystore", "out", "key-alias")
                    for argument in ("--" + name, "fixture")]

        class Captured(Exception):
            pass

        for overrides, expected in (([], ("1.6.0", 16000)),
                                    (["--version-name", "1.6.1", "--version-code", "16001"], ("1.6.1", 16001))):
            def capture(parser):
                args = original_parse(parser, required + overrides)
                self.assertEqual((args.version_name, args.version_code), expected)
                raise Captured()

            with patch.object(argparse.ArgumentParser, "parse_args", capture), self.assertRaises(Captured):
                android.main()


if __name__ == "__main__":
    unittest.main()
