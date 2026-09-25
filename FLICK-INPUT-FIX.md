# r22.1-bugfix-1：Flick 快照补齐与 Android DPI

基于 r22.1，仅修复用户指定的两项问题。

## 快照补齐顺序

`prpr/src/judge.rs` 的 `complete_finger_snapshots(touches, active, debug_ids)` 为仍活动但缺少快照的手指补入 Stationary。它保留已有快照的动作、位置和时间；终止事件已先更新 active 表，因此不会补回 Ended/Cancelled 手指。debug_ids 可选，用于原有触摸报告。

调用位置由 Flick 追踪更新之后移动到之前，使补入的静止坐标参与本帧速度和迟滞状态更新。避免漏更新时沿用未消费的手势，或停止后未重新进入可触发状态而漏掉下一次滑动。

正常情况下，没有新 MotionEvent 不等于快照缺失：Macroquad 保留 Stationary。这次修复针对快照缺失的异常分支，未证明它是历史多指 Cancelled 断流的根因。Ended/Cancelled 处理、采样时钟、判定时间窗、空间范围和候选规则均未改变。

## Android DPI

当前外壳原先传入整数化的 min(xdpi, ydpi)。打包时将其改为 DisplayMetrics.densityDpi，以对齐 Unity 2022.3 Android Screen.dpi 的文档行为：
https://docs.unity3d.com/2022.3/Documentation/ScriptReference/Screen-dpi.html

`scripts/dex_density_dpi.py` 提供：
- `patch_density_dpi(data: bytes) -> bytes`：按 DEX 表解析字段和方法 ID，匹配唯一的原调用序列并替换同长度指令，重算 SHA-1 和 Adler-32；重复应用不产生新变化。
- `verify_density_dpi(data: bytes)`：最终 APK 检查，取值仍为旧路径、校验和错误或调用序列不匹配时抛错。

`scripts/package_android_release.py` 自动应用并验证此补丁。外壳输入事件转发、资源及 JNI 签名均不修改。Phira 判定模式继续使用原有固定阈值。

`tests/verify_android_dpi.py BASE.apk` 使用 r22.1 APK 测试补丁范围、幂等性、损坏输入、未知指令及旧取值拒绝。Rust judge 测试增加未消费手势清理、已消费手势重新触发、保留新快照与终止手指排除三项回归。

尚未实机测试。

判定模块回归命令：`cargo test --locked -p prpr --features uuid/serde --lib judge::`。本次 20 项通过；DEX 补丁 5 项通过。
