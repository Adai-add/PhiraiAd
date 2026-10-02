#!/usr/bin/env python3
"""Compare an APK's three font assets with the current project files."""
import argparse
import hashlib
from pathlib import Path
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    matches = True
    with zipfile.ZipFile(args.apk) as apk:
        for name in ("font.ttf", "bold.ttf", "phigros.ttf"):
            key = "assets/" + name
            source = (root / key).read_bytes()
            try:
                packaged = apk.read(key)
            except KeyError:
                print(key, "MISSING FROM APK")
                matches = False
                continue
            same = packaged == source
            matches &= same
            print(key, "MATCH" if same else "DIFFERENT")
            print("  source:", hashlib.sha256(source).hexdigest())
            print("  APK:   ", hashlib.sha256(packaged).hexdigest())
    return 0 if matches else 1


if __name__ == "__main__":
    raise SystemExit(main())
