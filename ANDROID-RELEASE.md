# r23.0-bugfix-1 Android 构建

以 r23.0 为源码与安装包基线，Android 外壳沿用 r14，保留 FFmpeg 视频功能，不包含 r15 的 Android 触摸插桩。

环境：Linux x86_64、Python 3、Java 17、CMake、Rust nightly-2026-09-14（aarch64-linux-android 目标）、Android NDK r27d、Build Tools 35.0.0。

Cargo.lock、原 miniquad/macroquad 和 FFmpeg 静态库未修改。工具链、基线 APK、签名密钥不嵌入源码包。

在源码根目录配置 ANDROID_NDK_HOME、PATH 后执行：

```bash
rustup target add aarch64-linux-android
read -r -s -p 'Keystore password: ' PHIRA_KS_PASS
export PHIRA_KS_PASS
read -r -s -p 'Private key password: ' PHIRA_KEY_PASS
export PHIRA_KEY_PASS
bash scripts/build_android_release.sh \
  --base-apk /absolute/path/Phira-Replica-0.8.2-r23.0.apk \
  --build-tools /absolute/path/build-tools/35.0.0 \
  --keystore /absolute/path/phira-replica-release.jks \
  --key-alias YOUR_EXISTING_ALIAS \
  --out /absolute/path/Phira-Replica-0.8.2-r23.0-bugfix-1.apk
```

包名 `org.flos.phira.replica`，版本名 `0.8.2-replica23.0-bugfix1`，versionCode `10033`。

打包脚本要求视频编译标记，更新版本信息和原生库，执行 zipalign、官方 apksigner 签名及验证，检查证书、对齐、CRC 和库内容。修改 APK 后必须重新签名。

本源码包的 Android 打包脚本默认将手机桌面显示名（`android:label`）设为 `PhiraiAd`，并在最终 APK 上验证 application/launcher label。也可通过 `--app-label NAME` 临时覆盖。

改版证书 SHA-256：`a59eb8345a617d28e3b2032849aa043d1b00fcbc2545a436fdc15c57bab29936`。

```bash
cargo test --locked --manifest-path tests/chart-play/Cargo.toml
python3 tests/verify_hold_renderer.py
```

AI 与界面测试命令见 AI-INFERENCE.md；实际结果见交付验证文件。原有宿主回归保留，含 6 项练习滑条交互和 5 项设置分组布局及输入回归。界面适配器不测试真实字体、GPU 和弹出菜单动画；手机安装、实际界面操作及视频播放尚未实机验证。具体使用及每文件接口说明见 REPLICA-SETTINGS.md。

本版打包时调用 scripts/dex_density_dpi.py，将外壳传给 QuadNative.setDpi 的值改成 DisplayMetrics.densityDpi。仅替换已核对的等长指令块并更新 DEX 校验和；不匹配的外壳拒绝打包，已修复的外壳可重复使用。原触摸分发代码不变。

练习音高修复的 SoundTouch 2.3.3 源码随包提供，prpr/build.rs 通过 cc 调用 NDK C++ 编译器静态链接，不新增 APK 动态库。音频单元测试：cargo test --locked --manifest-path tests/practice-audio/Cargo.toml。
