#!/usr/bin/env bash
# Build the ARM64 Phira library with video enabled, then package and verify it.
# Required: Rust (rust-toolchain.toml), NDK r27d, Android Build Tools 35,
# Python 3, Java 17, C/C++ host tools. See ANDROID-RELEASE.md for arguments.
set -euo pipefail

release_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to the Android NDK r27d directory}"
ndk_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="$ndk_bin/aarch64-linux-android21-clang"
export CXX_aarch64_linux_android="$ndk_bin/aarch64-linux-android21-clang++"
export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

cd -- "$release_root"
cargo build --locked -p phira --lib --target aarch64-linux-android --release --features video
python3 scripts/package_android_release.py \
  --native-lib "${CARGO_TARGET_DIR:-target}/aarch64-linux-android/release/libphira.so" \
  --cxx-lib "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" \
  "$@"
