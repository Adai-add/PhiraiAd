# Phira Replica 0.8.2-r21.0

## r21.0 改版设置与在线谱成绩

- 新增独立改版栏，按成绩规则、判定与长条、报告与可视化分组排版。
- 在线谱开启改版功能默认不保存新本地或云端成绩；新增仅本地记录开关，默认关闭。
- 在线谱个人最佳单独保存，准确率取本地与已同步云端记录最高值。
- 历史记录保留，手动自定义 RKS 准确率仍优先；详见 REPLICA-SETTINGS.md。
- 以下旧版本成绩限制说明以本版规则为准。

## r20.0 长条头部打击特效

- 设置 → 谱面增加默认开启的“长条头部打击特效”。
- 成功头部判定立即在判定线上播放一次 Tap 样式特效，复用资源包颜色；支持两套判定、键盘和自动播放。
- 下一次持续特效按原周期安排，避免头部连闪；不改变判定或成绩资格。
- 详情与接口见 HOLD-HEAD-EFFECT.md。

## r19.1 RKS 操作改进

- RKS 设置顶部增加“全选 / 全取消”，分别作用于全部本地谱面或全部额外条目（跨分页、不受搜索过滤）。
- 勾选框触摸范围的宽、高均为原来的 130%，以原中心向四周扩展；外观大小不变。
- 自定义准确率在卡片上显示绿色，跟随记录时恢复原色；数值仍保留两位小数。
- 保留所有定数、准确率与计算规则；配置格式兼容 r19。
- 41 项 Rust 测试及原长条绘制一致性检查通过；Android 安装包验证见 verification.json，尚未实机验证。

## r19 自定义 RKS

本地谱面库新增 RKS 按钮、候选绿勾、实际定数和准确率。顶部切换编辑模式，支持来源选择、手动定数/准确率、额外条目及汇总数量设置。RKS 与准确率显示两位小数，定数最多一位；真实成绩和谱面文件不修改。详情、接口和测试见 CUSTOM-RKS.md。

## r18 长条渲染数据修复

- 撤销 r17 每帧限制剩余条身的做法，恢复 r14 的原长条绘制逻辑。
- 只在提交给渲染器时提供缩减后的尾部时间、尾部高度；实际判定的 NoteKind 不修改。
- 完整条身最短 0.01 绘制单位；剩余条身允许自然缩短到更小，不会被固定在下限。
- 只剩头部一次判定时，原渲染器在头部时间结束显示，不额外延迟显示条身。
- 全局开关、禁止上传和不更新普通本地最佳成绩的限制继续生效。
- 下方各版本记录属于历史说明；当前实现与测试见 CHART-PLAY-ASSISTS.md。

## r17 全局长条缩减

- 将长条缩减移到“设置 → 谱面”，默认关闭，全局保存，正常、练习及预览共用。
- 缩减后条身在游戏绘制坐标中最短 0.1；原安全放手时间和实际判定不变。
- 开启后禁止上传成绩，也不更新普通本地最佳记录；报告记录开关与原因。
- 旧谱面长条开关被忽略，倒打区间保留；谱面菜单改名“自动倒打”。
- 原签名、ARM64 视频构建；20 项宿主测试通过，尚未实机验证。
- 操作、文件与接口说明见 CHART-PLAY-ASSISTS.md。

## r16 历史：长条缩减与自动倒打（基于 r14）

以下是 r16 的历史行为，长条开关位置、最小长度及上传限制以 r17 说明为准。

- 在谱面菜单中增加“长条缩减 / 自动倒打”，与练习、调整延迟并列，进入实时自动播放预览。
- 长条缩减按当前 Hold 判定的可放手阈值计算，仅改变绘制和末尾持续粒子；实际音符时间、判定和计分不变。短至头部的长条显示零长度头部，不产生反向长度。
- 预览支持播放/暂停、拖动进度、前后跳转、标记区间开始/结束、0.01 秒微调、添加、选择修改、删除、保存或取消。
- 已保存倒打区间在正常及练习中按谱面时间上下翻转，起点绿色、终点红色，标在正常进度条、练习拖动条和编辑进度条。
- 触摸反变换、持久手指坐标和 Flick 历史同步；全屏特效补偿避免重复翻转。只要配置有效倒打区间，就关闭成绩上传并不更新普通本地最佳记录。
- JSON 谱面将设置写入自身 phiraReplica 字段；PEC/PBC 保存到谱面 info.yml；内置谱面使用对应谱面的本地设置文件。
- 沿用 r14 外壳、原签名和 FFmpeg 视频构建；不合入 r15 追踪插桩。
- 操作、接口、边界与验证见 CHART-PLAY-ASSISTS.md；构建见 ANDROID-RELEASE.md。

## r14 打包修复

- 修复 r13 的 v2 签名块结构错误，改用 Android 官方 `apksigner` 生成并验证 v1/v2/v3 签名，沿用最初的签名证书。
- ARM64 构建明确启用 `video`，恢复 FFmpeg 视频解码路径；新增仅在启用视频时存在的构建标记，打包器会拒绝缺少该功能的库。
- 新增可复用的构建和打包脚本，输出前验证签名、证书一致性、版本、ZIP CRC 与对齐。
- 保留 r13 全部功能，报告版本更新为 `0.8.2-replica.14`。判定与触摸算法未改。
- 构建方式、每文件职责和接口说明见 `ANDROID-RELEASE.md`。

本修改版在 Phira 0.8.2 源码基础上增加可持久化、按音符类型拆分的判定模式：

- `Phira`：保留原有判定逻辑，默认模式。
- `PhigrosReplica`：复刻 Phigros 3.20.0 普通模式判定，可额外启用严格判定。

设置入口：`设置 → 谱面`。Tap、Drag、Hold 共用一个选择器，Flick 使用独立选择器；切换后从下一次开始游玩起生效。

## 复刻范围

- Tap：Perfect ±80 ms、Good ±180 ms、仅早方向存在最大约 220 ms 的 Bad；早按外缘随横向距离线性收窄。
- Drag：横向 `< 2.1`，±100 ms 内经过即可预捕获，接近音符时间后结算 Perfect。
- Hold：头部 P/G、允许早期预捕获；任意手指均可维持；断触采用 `safeFrame = 2`；尾端前约 220 ms 完成。
- Flick：DPI 修正速度阈值、方向投影和迟滞；横向 `< 2.1`，时间窗约 ±140 ms，单次 Flick 事件只消费一次。
- 候选：10 ms 内按音符类型优先级和判定线局部坐标的位置代价选择。

严格判定使用最近最多 10 帧的平均帧时长加入半帧补偿，只收紧 Tap、Hold 头部和 Flick 音符时间窗。本实现依据 3.20.0 二进制静态分析的等价行为说明，不是官方 C# 源码。

## 修改文件

- `prpr/src/config.rs`：新增 `JudgementMode` 和持久化配置字段。
- `prpr/src/judge.rs`：复刻判定常量、候选择优、四类音符状态机和 Flick 追踪器。
- `prpr/src/judgement_range.rs`：只读判定范围可视化；按当前判定方式、谱面动画、音符流速和实际音符宽度绘制。
- `prpr/src/core/chart.rs`、`prpr/src/scene/game.rs`：将判定范围接入谱面渲染，并在本局使用过可视化后阻止上传成绩。
- `prpr/src/core/note.rs`：Hold 早期预捕获期间暂停粒子效果。
- `phira/src/page/settings.rs`：新增判定模式选择器、扩展播放速度范围，并加入折叠式判定范围调试设置。
- `phira/src/lib.rs`、`phira/src/scene/main.rs`、`phira/src/page/home.rs`、`phira/src/scene.rs`：恢复开源构建中被关闭的内置资源加载。
- `assets/background.jpg`、`assets/font.ttf`、`assets/res/*`：补齐可独立构建所需的官方 0.8.2 内置资源。
- `phira/locales/*/settings.ftl`：新增界面文本；简体/繁体中文已本地化，其余语言暂用英文。

## 主要接口

- `prpr::config::JudgementMode::{Phira, PhigrosReplica}`：判定模式枚举。
- `Config::judgement_mode`：序列化到现有 `data.json`，旧配置缺少该字段时自动使用 `Phira`。
- `Judge::update(...)`：根据当前配置选择判定规则；输入采集、坐标变换、计分和表现层继续共用。
- `Config::has_custom_judgement()`：判断是否实际启用了 Phigros 判定，用于禁止自定义判定成绩上传。
- `Config::blocks_score_upload()`：统一判断自定义判定或判定范围可视化是否禁止成绩上传。

## 早期版本的构建验证

- `cargo check -p phira --target aarch64-linux-android`：通过。
- `cargo clippy -p prpr --target aarch64-linux-android -- -D warnings`：通过。
- Phira 本地化资源完整性检查：通过（原项目已有的未翻译项仍只产生警告）。
- Android ARM64 Release 构建：通过。

APK 使用独立包名 `org.flos.phira.replica`，因此可与官方 Phira 同时安装。开源构建为未评级构建，不上传成绩。

## r2 修复

- 补齐官方 Android 0.8.2 外壳要求的 `QuadNative.preprocessInput` JNI 兼容入口。
- 该入口缺失会在外壳收到触摸事件时触发 `UnsatisfiedLinkError` 并退出；兼容入口保留公开源码原有的 `surfaceOnTouch` 输入处理。

## r3 修复

- 将公开源码初始化时必需的 `assets/export.png` 补入 Android APK。
- r2 日志中的直接退出原因是 `AndroidAssetLoadingError: Couldn't load file export.png`；随后出现的 SIGSEGV 是 MuMu ARM 转译环境在退出阶段产生的次生错误。

## r4 修复

- 修正官方复刻模式中 Flick 输入位移的坐标单位：先应用 Phira 视口变换，再转换为 Phigros 世界单位。
- `isNewFlick` 只在成功捕获 Flick Note 后消费，不再每帧清除；Flick 的原有帧时间差计算保持不变。
- 为官方复刻模式增加按 finger ID 维护的持久手指表，只有 `Ended` / `Cancelled` 会移除手指，避免高帧率或模拟器丢失静止帧时 Hold 被误判为断触。
- 原版 `Phira` 判定分支保持不变；Phigros 3.20.0 的时间窗、空间范围、候选规则、Flick 速度常量、Hold `safeFrame = 2` 和尾部窗口均未修改。

## r5 修复

- 官方复刻模式的 Hold 仍在尾端前 220 ms 提前完成成绩结算，但不再立即切换到通用 `Judged` 视觉状态。
- 完成结算后进入现有的 tail-armed 纯视觉阶段：不再检查手指接触，Hold 本体保持正常亮度，周期打击特效继续生成。
- 只在 Hold 的实际 `end_time` 到达时结束视觉状态，且不会重复提交成绩。原版 `Phira` 分支与全部判定常量未改。

## r6 修改

- 练习模式与谱面设置共用的播放速度范围由 `0.5x–2.0x` 扩展为 `0.05x–10.0x`，步长仍为 `0.05x`。
- 修复公开源码在 `cfg(not(closed))` 构建下主动禁用主界面 BGM、内置角色图片和内置章节资源的问题。
- 为公开构建补入与官方 0.8.2 资源格式兼容的块变换与 Zstandard 解码，并恢复 BGM、角色和内置章节的加载路径。
- 判定代码、判定窗口、候选规则和 Hold/Flick 状态机均未修改。

## r7 修改

- 练习模式增加独立“音符流速”，范围 `0.100x–20.000x`、步长 `0.05x`、每次进入练习模式默认 `1.000x`。
- 最终视觉流速为“谱面播放速度 × 音符流速”；只缩放音符相对判定线的行进距离和 Hold 身体长度，不修改音乐、谱面时间、判定窗或 Hold 生命周期。
- 增加“锁定为播放速度的倒数”。锁定值按 `round(1 / 播放速度, 3)` 计算，完整覆盖当前 `0.05x–10.0x` 播放速度范围，对应 `20.000x–0.100x` 音符流速。
- 低流速下的离屏音符裁剪同步使用视觉倍率，避免本应可见的远处音符被提前裁掉。
- 判定设置拆分为“Tap / Drag / Hold”和“Flick（红键）”两个选择器，均可独立选择 Phira 或 Phigros 3.20.0 复刻。
- 从 r6 更新时，旧的统一判定模式会先继承给 Flick，因此首次升级后的实际判定行为保持不变；之后可单独修改 Flick。
- 两项选择相同时保留 r6 原有的纯 Phira / 纯 Phigros 路径；混合模式会隔离跨引擎候选，避免另一引擎的音符抢占当前输入事件。

## r8 修改

- 新增“Phigros 严格判定”开关；它只影响已选择 Phigros 判定的按键组，纯 Phira 分支完全不受影响。
- 严格窗口维护最近最多 10 帧的 `deltaTime` 平均值，并计算 `Perfect = 40 ms + 半帧`、`Good = 90 ms + 半帧`、`Bad = 140 ms + 半帧`、`Flick = 1.75 × Perfect`。
- Tap 使用严格 P/G/B；Hold 只有头判使用严格 P/G；Flick 只有音符时间捕获窗收紧。
- Drag ±100 ms、所有横向范围、Hold 换指与 `safeFrame = 2`、Hold 尾端提前约 220 ms、Flick DPI/速度/迟滞手势状态机均保持普通 Phigros 原值。
- 两项判定设置及严格开关的说明均加入“启用更改后无法上传成绩”；运行时也会阻止任何实际启用了 Phigros 判定的成绩上传。

## r9 修复

- Phigros 判定事件统一使用同一帧的谱面时间，不再使用 Android `MotionEvent` 的逐事件时间戳；Phira 判定仍保留原有事件时间路径。
- Phigros Flick 速度改为每个游戏帧只用最新手指位置更新一次，原始 MotionEvent 数量以及其他手指事件不再改变速度计算。
- 按 FingerManagement 的持久插入顺序处理多指，移除 `HashMap` 遍历顺序造成的同帧随机性。
- 触摸开始候选恢复为 Phigros 3.20.0 的四键扫描：Tap/Hold 具有候选优先级，Drag 可进入预判，Flick 只能占用候选而不能被点按判定；混合模式仍隔离选择为 Phira 的 Flick。
- 每帧使用平台持久触摸快照校准手指表和 Phigros Flick 追踪器，避免暂停期间遗漏抬起事件后留下幽灵手指。
- 按要求保留两项已知差异：非 `1.0x` 播放速度下候选 `10 ms` 不随速度缩放；键盘模式继续使用 Phira 原有键盘判定路径。
- 所有普通/严格时间窗、横向范围、Flick 阈值和状态机常量、Hold `safeFrame = 2` 与尾端逻辑均保持 r8 原值。

## r10 修改

- 在 `设置 → 调试 → 判定范围可视化` 增加总开关；可选择将范围绘制在判定线、音符或两者上，并设置提前显示时间和填充透明度。
- Tap 可分别显示 Perfect、Good、Bad 与 Miss 外缘；Hold 可分别显示头部 Perfect、Good、Miss 外缘与尾判结束区；Drag、Flick 分别显示当前模式的最大捕获范围。
- Tap 与 Hold 头部的嵌套等级采用互不重叠的差集填充，避免半透明颜色叠加；全部边界外框保持完全不透明。
- “画在音符上”的宽度跟随引擎实际绘制宽度，包括全局音符尺寸、双押提示纹理宽度、RPE 横向缩放和 `sizeControl`；再叠加当前判定模式的横向系数及谱面 `judgeArea`。
- 范围位置按当前音符速度、`speedControl`、练习模式音符流速、判定线动画、`yOffset` 和倾斜变换计算；不修改谱面时间、音乐、判定时刻或 Judge 状态。
- 只遍历即将在设置秒数内进入实际判定窗的音符；Phira 原版的晚判额外 70 ms 已合并到对应范围中，Phigros 严格模式则实时读取最近帧平均值。
- 可视化在本局任意时刻启用过后，即使中途关闭，本局成绩仍禁止上传；设置页同时明确标注这一限制。

## r11 修改

- 新增“自动导出打歌报告”，默认开启，可在 `设置 → 调试` 中关闭；报告采用 UTF-8 JSON，文件名为 `歌曲名-记录时间.json`。
- 完整结算、暂停菜单重来、退出歌曲、练习区间结束，以及练习模式修改播放速度、音符流速、锁定状态、游玩位置或区间时，都会封存当前已开始的游玩段并导出一次。
- Android 10 及以上优先保存到公共 `Download/Phira-Reports`；公共目录不可用时回退到应用数据目录 `data/play-reports`，界面会显示实际保存位置或错误。
- 报告记录歌曲/谱面身份、墙钟起止时间、设置区间与实际游玩区间、分数、实时准确率、完整谱面准确率、最大连击、暂停时间，以及设置区间、实际区间和实际判定三种口径的总物量与分类物量。
- Tap 与 Hold 头分别统计 Miss、Bad、Good、Perfect；Drag 与 Flick 分别统计 Miss、Perfect；Hold 另记录完成、提前松开和头部漏接数量。
- Tap 与 Hold 头的每次判定按全谱稳定的 1-based 物量编号记录延迟；正数代表提前，负数代表落后，Miss 保留编号且延迟为 `null`。
- Tap、Hold 头和两者合计均提供平均数、中位数、总体方差、标准差、平均绝对延迟、最早/最晚极值，以及 10 ms 直方图。
- 报告只消费 Judge 事件的只读副本，不改变原判定队列、候选顺序、计分、上传资格或表现层；自动导出本身不会禁止上传成绩。

### r11 修改文件与接口

- `prpr/src/play_report.rs`（新增）：定义 `PlayReportRecorder`、`PlayReport`、`ReportEndReason`、延迟统计及 JSON schema v1；全谱编号按“音符时间 → 判定线编号 → 线内音符编号”排序，排除假音符。
- `prpr/src/judge.rs`：新增 `JudgeReportEvent` 与 `Judge::take_report_judgements()`；原判定事件提交时复制一份精确时间差到独立报告队列，原多人/结算事件队列保持不变。
- `prpr/src/scene/game.rs`：维护单次游玩报告生命周期，并接入结算、重来、退出、练习完成和练习设置变更触发点。
- `prpr/src/scene/loading.rs`、`prpr/src/scene.rs`：新增 `ReportFn` 导出回调并传入游戏场景。
- `prpr/src/config.rs`：新增持久化布尔字段 `Config::auto_export_play_report`，旧配置缺失时默认开启。
- `phira/src/play_report_export.rs`（新增）：JSON 序列化、文件名清理、重名避让、Android MediaStore 公共下载目录写入及应用目录原子写入回退。
- `phira/src/scene/song.rs`、`phira/src/scene/unlock.rs`、`phira-monitor/src/launch.rs`：在全部游戏加载入口传递报告回调；监视器/View 模式不生成报告。
- `phira/src/page/settings.rs`：在调试设置中增加自动报告开关。
- `phira/src/lib.rs`：注册导出模块并增加 `dir::play_reports()` 应用数据目录接口。
- `phira/locales/*/settings.ftl`、`prpr/locales/*/game.ftl`：增加设置说明与导出成功/失败提示。

报告中 `timing.delayByGlobalNoteNoMs.tap` 和 `timing.delayByGlobalNoteNoMs.holdHead` 是“全谱物量编号 → 延迟毫秒”映射；`timing.signConvention` 固定为 `positive_is_early_negative_is_late`。`result.scoreIsProvisional` 用于区分完整结束与中途封存，`range` 同时保留 configured/actual 两组起止时间。

## r12 修复

- 修复 Phigros 判定在多指输入下可能同时失去全部持续手指，导致 Drag 与 Flick 必须抬起重按才能恢复的问题。
- 正常游戏帧只使用平台触摸快照补充或刷新手指，不再因快照短暂缺失而删除持久手指、Finger 顺序或 Flick tracker；明确的 `Ended` / `Cancelled` 仍会立即删除对应手指。
- Phigros Flick 收到缺少 `Started` 的孤立 `Moved` / `Stationary` 时会安全重建 tracker，并以当前位置作为零速度基线，恢复帧不会凭空触发 Flick。
- 暂停、应用切后台、失败暂停、键盘暂停、重来和练习状态重置仍会显式清空全部触摸状态，避免幽灵手指跨越生命周期边界。
- Phira 判定分支、Phigros 普通/严格时间窗、候选优先级、Flick 阈值、Hold `safeFrame = 2`、判定范围可视化和打歌报告 schema 均未改变。

## r13 修改

- 在 `设置 → 调试` 新增“触摸输入 Debug 报告”开关；开启后即使普通“自动导出打歌报告”关闭，本局结束、重来、退出或练习设置变更时也会强制导出报告。
- 报告 schema 升级为 v2；保留 r11 的歌曲、分数、物量、分类判定、判定模式与延迟统计，并新增可选的 `touchDebug` 对象。
- 为每个 finger ID 按触点生命周期记录按下来源与时间、逐帧轨迹、抬起/取消来源与时间；缺失 `Started`、触点被新 `Started` 替换、报告结束时仍未抬起等异常状态均会明确标记。
- 每帧同时记录底层原始触摸事件、Macroquad 持久触摸快照、判定坐标、Phigros 运动坐标、平台事件时间、帧判定时间、实际用于两组判定的时间、稳定 Finger 顺序及键盘输入计数。
- 记录 Phigros `active_fingers` 和 Flick tracker 在帧前/帧后的完整状态，以及 tracker 的创建、孤立移动重建、删除和合成 Stationary 的 finger ID，可直接判断断触发生在平台事件、快照、持久手指表还是 tracker 层。
- 暂停按钮、应用切后台、失败暂停、键盘暂停、重来与练习重置等主动清理会保存独立 clear 事件，并保留被清理前的全部手指、顺序和 tracker 状态。
- 为限制极端长谱的文件大小，每份报告最多保存 200,000 帧和 1,000,000 个轨迹样本；超过后设置 `truncated` 并记录准确的丢弃数量，不会静默缺失。
- 开启 Debug 报告会禁止本局上传成绩；记录器只读取并复制输入状态，不参与候选、判定、计分或 Flick tracker 更新。
