#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
    echo "This script requires macOS and Xcode." >&2
    exit 1
fi
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-09-14}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/xcode}"
export IPHONEOS_DEPLOYMENT_TARGET=12.0
mkdir -p dist build
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/phiraiad-ios.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

# Existing fonts/icons are preserved; no Android executable is copied into IPA.
if [[ ! -f assets/background.jpg || ! -f assets/res/bgm ]]; then
    : "${ASSETS_APK_URL:?Missing assets: set ASSETS_APK_URL to a downloadable APK URL}"
    curl --fail --location --retry 3 --connect-timeout 30 \
        --output "$work_dir/assets.apk" "$ASSETS_APK_URL"
    python3 scripts/prepare_ios_unsigned.py assets "$work_dir/assets.apk"
fi
python3 scripts/prepare_ios_unsigned.py plist "$work_dir/Info.plist"

# Build with the lockfile before the project's Cargo build phase.
cargo build --locked -p phira-main --bin phira-main --release --target aarch64-apple-ios
cp "$CARGO_TARGET_DIR/aarch64-apple-ios/release/phira-main" build/phira-main
xcodebuild -project phira.xcodeproj -scheme phira \
    -configuration Release -destination 'generic/platform=iOS' \
    -archivePath "$work_dir/PhiraiAd.xcarchive" archive \
    CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY='' \
    DEVELOPMENT_TEAM='' PROVISIONING_PROFILE_SPECIFIER='' \
    INFOPLIST_FILE="$work_dir/Info.plist" \
    PRODUCT_BUNDLE_IDENTIFIER=com.adaiadd.phiraiad \
    MARKETING_VERSION=1.1.0 CURRENT_PROJECT_VERSION=110 \
    INFOPLIST_KEY_CFBundleDisplayName=PhiraiAd \
    ENABLE_USER_SCRIPT_SANDBOXING=NO \
    2>&1 | tee dist/ios-xcodebuild.log

git diff --exit-code -- Cargo.lock
app_dir="$work_dir/PhiraiAd.xcarchive/Products/Applications/Phira.app"
python3 scripts/prepare_ios_unsigned.py verify "$app_dir"
xcrun lipo "$app_dir/phira-main" -verify_arch arm64
xcrun vtool -show-build "$app_dir/phira-main"
mkdir -p "$work_dir/package/Payload"
ditto "$app_dir" "$work_dir/package/Payload/PhiraiAd.app"
ipa="$PWD/dist/PhiraiAd-v1.1.0-ios-unsigned.ipa"
ditto -c -k --keepParent "$work_dir/package/Payload" "$work_dir/output.ipa"
unzip -t "$work_dir/output.ipa"
mv -f "$work_dir/output.ipa" "$ipa"
(cd dist && shasum -a 256 "$(basename "$ipa")" > "$(basename "$ipa").sha256")
echo "Built unsigned IPA: $ipa"
