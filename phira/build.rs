fn build_ios_photos() {
    use std::{path::PathBuf, process::Command};
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("ios") {
        return;
    }
    println!("cargo:rerun-if-changed=native/ios_photo_export.m");
    println!("cargo:rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    let target = std::env::var("TARGET").unwrap();
    let simulator = target.ends_with("-sim") || target.starts_with("x86_64-");
    let sdk = if simulator { "iphonesimulator" } else { "iphoneos" };
    let sdk_output = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .expect("iOS 相册桥接需要 Xcode/xcrun");
    assert!(sdk_output.status.success(), "找不到 iOS SDK");
    let sdk_path = String::from_utf8(sdk_output.stdout).unwrap();
    let arch = if target.starts_with("aarch64-") { "arm64" } else { "x86_64" };
    let deployment = std::env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "12.0".into());
    let clang_target = format!("{arch}-apple-ios{deployment}{}", if simulator { "-simulator" } else { "" });
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let object = out.join("ios_photo_export.o");
    let library = out.join("libphiraiad_ios_photos.a");
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            sdk,
            "clang",
            "-target",
            &clang_target,
            "-isysroot",
            sdk_path.trim(),
            "-fobjc-arc",
            "-fblocks",
            "-O2",
            "-Wall",
            "-Wextra",
            "-c",
            "native/ios_photo_export.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("无法运行 iOS clang");
    assert!(status.success(), "iOS 相册桥接编译失败");
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(&library)
        .arg(&object)
        .status()
        .expect("无法运行 ar");
    assert!(status.success(), "iOS 相册桥接打包失败");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=phiraiad_ios_photos");
    println!("cargo:rustc-link-lib=framework=Photos");
    println!("cargo:rustc-link-lib=framework=Foundation");
}

fn git_stdout(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?.trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

fn main() {
    build_ios_photos();
    dotenv_build::output(dotenv_build::Config::default()).unwrap();

    let git_dir = git_stdout(&["rev-parse", "--git-dir"]).unwrap_or_else(|| ".git".to_string());
    println!("cargo:rerun-if-changed={}/HEAD", git_dir);
    println!("cargo:rerun-if-changed={}/packed-refs", git_dir);

    if let Some(ref_path) = git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed={}/{}", git_dir, ref_path);
    }

    let git_hash = git_stdout(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
}
