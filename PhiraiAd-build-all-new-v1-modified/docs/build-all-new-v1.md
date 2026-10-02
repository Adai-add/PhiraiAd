# 一键清理并构建三个平台（v1）

## 安装与运行

将压缩包按原目录结构解压到：
`\\wsl.localhost\Ubuntu\home\adai_add\phira-build`

在 Windows PowerShell 执行：

```powershell
& "\\wsl.localhost\Ubuntu\home\adai_add\phira-build\build_all_new.cmd"
```

也可双击该 CMD。不要在 Ubuntu bash 里直接运行 PS1。

首次自动生成 `~/phira-build/build_all_new.config.json`。里面只有路径和工具参数，没有密码。可先检查 WSL 输入文件及预览同步（不清缓存、不编译）：

```powershell
& "\\wsl.localhost\Ubuntu\home\adai_add\phira-build\build_all_new.cmd" -CheckOnly
```

`CheckOnly` 会生成配置与日志目录、检查 WSL 路径及密码文件可读取，然后用 robocopy /L 列出同步计划；Windows 编译器、Android 证书和完整链接在实际构建时验证。

## 默认配置

- 源码：WSL `/home/adai_add/phira-build`（唯一同步源）。
- Windows 源码：`E:\PhiraiAd-windows`。
- 原始底包：`C:\Users\admin\Desktop\phira-rep\签名\Phira-Replica-0.8.2-r23.0.apk`。
- 密钥库：同目录 `phira-replica-release.jks`；alias 为 `phira-replica`。
- 密码：同目录 `key.txt`，完整内容同时用于密钥库密码与私钥密码。UTF-8（可有 BOM）或带 BOM 的 UTF-16；空格、CR/LF 均不自动去除，编码 BOM 仅作为编码标识。
- NDK：`/home/adai_add/Android/Sdk/ndk/27.3.13750724`。
- Build Tools：`/home/adai_add/Android/Sdk/build-tools/35.0.0`。
- vcpkg：`E:\PhiraiAd-vcpkg`。
- Windows 代理：仅在 `127.0.0.1:51081` 可连接时启用。可把 config 中 proxy 改为空字符串禁用自动检测；已有环境代理仍会保留。WSL 使用自己的 bash 登录环境代理配置。

底包必须使用原来成功打包采用的、适合原打包程序 DEX 修补的底包。不要直接把已再次修补的成品当底包。若默认路径不同，在自动生成的 JSON 里修改。所有 WSL 文件路径填写 Linux 路径，例如 `/mnt/c/...`。

## 同步和缓存

先使用 robocopy /MIR 将 WSL 文件镜像到 E 盘，删除 Windows 源码中在 WSL 已不存在的普通文件。E 盘源码的独立修改会被 WSL 版本覆盖。

不参与同步/清除：两侧 `.git`、`.cargo`、存档 data/cache、dist、IDE 配置、签名目录、密钥、密码、环境配置、日志和 FFmpeg 下载 static-lib。独立 Windows 构建脚本在源存在时刷新，源缺失时保留。临时配置不复制到 E 盘。

同步成功才删除两侧整个 `target`。所有默认 Cargo 编译缓存被清理，随后重新编译；Rust 安装、Cargo 依赖下载、NDK、SDK、vcpkg/zlib、FFmpeg 下载缓存保留。不调用 cargo clean 去删除外部自定义目录；本脚本固定 Cargo 输出为源码下 target。

确认源码包含 fonts/background/res 等运行资源，且运行时不要编辑源码或同时启动另一构建。本脚本以锁文件阻止自身重复运行；不会阻止其他手动 Cargo 命令。

## 输出和失败行为

输出目录 `C:\Users\admin\Desktop\phira-rep`：

| 平台 | 文件 |
| --- | --- |
| Android ARM64 | PhiraiAd-arm64-new.apk |
| Windows x64 | PhiraiAd-windows_x64-new.zip |
| Linux x64 | PhiraiAd-linux-new.tar.gz |

Android 沿用已有打包程序，检查架构、标记、图标、字体、版本、签名和 ZIP；versionCode 取最低配置值、底包+1、已有同名 APK+1 三者最大值，默认最低 11000。
Windows 沿用已成功的独立脚本，程序和 assets 一起打包。
Linux 包含顶层 `PhiraiAd-linux-new` 目录，内有 `PhiraiAd` 可执行文件和 assets；保留执行权限，并检查本机 ldd。真实 Linux 平台兼容性仍由使用的 glibc/系统依赖决定。

每个平台失败后继续其他平台；同步或清缓存失败则停止。仅新构建成功后覆盖对应旧成品，失败的平台留下旧文件，因此必须看最后 OK/FAILED 汇总，不能只根据文件存在判断本次成功。
日志：`phira-rep\build-logs\时间戳`。密码只存在于签名工作进程环境中，不显示、不作为命令行参数、不保存到日志/配置/产物。

## 修改文件

- build_all_new.cmd：Windows 双击入口。
- scripts/build_all_new.ps1：配置生成、镜像同步、Windows 清理、三平台调度、日志与汇总。
- scripts/build_all_new_wsl.py：WSL 缓存清理、Android 签名、版本递增、Linux 构建打包。
- scripts/build_windows_release.ps1、build_windows.cmd：附带前面已使用的 Windows 构建脚本，便于 WSL 源码同步后使用。

不修改游戏逻辑。未在用户 Windows/WSL 上执行完整三平台编译；交付前验证脚本解析、密码保真、缓存删除保护、失败保留旧产物、Linux 包执行权限及 ZIP 完整性。
