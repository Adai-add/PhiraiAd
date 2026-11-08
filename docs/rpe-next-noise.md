# RPE-Next 原生噪域

对照 [rpe-next 0.14.0](https://github.com/kclg-YSJ/rpe-next/tree/1ad96d4a82e3909bcb60cc41e5a250ae08cdd676) 实现；保留现有官谱与 Recorder 噪域支持。
导出包含原生噪域的 **Next PEZ** 后按通常方式导入 PhiraiAd，无需手动改写噪域 JSON。

| 数据 | 支持行为 |
| --- | --- |
| 格式 220+ 的 `blockAreaList` | 根据字段识别原生 Next；官谱百分比坐标仍使用原逻辑 |
| `appearTime` / `disappearTime` | 全局拍数换算，支持 BPM 变化，与单条判定线 BPM 倍率无关 |
| `activeIntervals` | 多段激活、生命周期裁切、相邻激活、零时长预闪 |
| `readyFlash` | 布尔兼容值或秒数，毫秒舍入，自定义时长内三次连续明暗变化 |
| 平移、旋转、独立 X/Y 缩放 | 间隙保持端值、事件开始前默认值、各自锚点和累计变换 |
| `transformStart` / `transformEnd` / `transformStep` | 导入的基准变换与同拍瞬时变换链 |
| 标准缓动 | 29 种缓动、`easingLeft` / `easingRight` 裁切 |
| Bezier | 完整浮点控制点，数值求解时间参数 |
| 格式 250+ 的 `customEasing` | 分段三次曲线、auto/smooth/corner/linear/hold、裁切 domain 和源端值 |
| `noiseAreaLayers` / `layerId` | 按层顺序合成；反转不跨层抵消 |
| `isInvert` | 层内普通区域取并集，反转区域按“恰好一个覆盖”组成掩码；三重反转不按奇偶恢复 |
| 不同画面比例 | 中心随画面轴适配，尺寸沿自身轴适配，不把旋转矩形剪切成平行四边形 |

默认外观统一复用已有的官谱/Recorder 噪域渲染链：普通模式使用现有完整效果，低性能模式使用现有简化效果。Next 适配只提供事件、几何和分层反转规则，不再有独立 `NextGpu`、覆盖计数纹理、纯色着色器或另一套最终合成。
进入激活区域会沿用 PhiraiAd 的触摸屏蔽、松手解除及音频开关；这部分属于游戏行为，制谱器预览没有对应的判定输入。

## 当前边界

- 自定义皮肤接入游戏现有 GLES 材质后端；原版 NoiseField/Ocean 的 GLSL ES 100 资源已验证。依赖 WebGL2 专有功能的任意 GLSL 300 程序不保证可在该后端直接编译，失败会显示具体资源/着色器错误，不静默冒充已支持。
- 噪域数值事件里的空间 `trajectory` 扩展会明确报错。标准数值事件和上述自定义曲线不受影响。
- 不代表 RPE-Next 所有其他扩展（全局事件、空间摄像机等）已兼容。

## 验证

`scripts/tests/generate_rpe_next_reference.mjs` 执行未修改的上游 `NoiseAreaRuntime`、`TempoMap` 和画面适配函数，生成 50 个用例、2,984 组参考值。
测试数据与来源版本保存在 `prpr/tests/fixtures/rpe_next_noise_reference.json`。

```sh
node scripts/tests/generate_rpe_next_reference.mjs /path/to/rpe-next prpr/tests/fixtures/rpe_next_noise_reference.json
cargo test --locked -p prpr --lib
cargo run --locked -p prpr --example rpe_next_noise_regression
```

图形验证程序实际导入 Next JSON 并运行生产噪域渲染器，共检查 238 项画面结果，包括普通/低性能模式、零至四及 255–257 个重叠反转、不同层、预闪、激活、消失、回退时间、普通模式 10,000 个区域、低性能模式 256 个区域、偏心旋转、水平镜像和练习模式扩大视野合成。

## v1.6.0 UI 与暂停修复

Next 的全屏中间绘制结束后恢复谱面的局部 viewport。当前 macroquad 分支的相机栈不保存 viewport，仅恢复相机不足以完成状态还原；残留的全屏 viewport 会让带黑边画面中的 HUD 绘制和暂停点击坐标不匹配。
暂停按钮另外使用谱面在实际显示器上的 viewport 转换原始触摸，不再从当前离屏绘制状态推断坐标。噪域内的谱面判定仍然屏蔽，暂停按钮仍可响应，单双击设置沿用原规则。

复测命令：

```sh
cargo run --locked -p prpr --example rpe_next_ui_regression
cargo build --locked -p prpr --example rpe_next_pause_regression
xvfb-run -a python3 scripts/tests/run_next_pause_regression.py
```

第一个程序有 96 项 viewport/实际像素检查，覆盖横纵黑边、带偏移的主 viewport、预闪/激活/消失/回退、静态缓存、练习扩大视野，以及普通和低性能模式。
第二个程序通过生产 `GameScene` HUD 和暂停逻辑、生产输入队列及 X11 点击事件检查 8 种场景，直接验证暂停图标像素位置，再验证点击响应。场景包含判定线、音符、自定义线贴图、普通/低性能模式、单双击、4x MSAA、练习 50% 视野和自定义噪域贴图。测试调用正常场景进入逻辑、启用顶层 UI，不在测试里每帧设置相机。它固定谱面时间，不推进正常判定时间线；不代替用户具体谱面的 Android/Windows 人工实测。
点击驱动只用于 Linux/Xvfb，需可用的 X11/XTest 和音频输出（或 ALSA null 配置）。


## 自定义噪域皮肤和贴图

`noiseAreaLayers[].skin` 指向谱面包内的 `shader/名称`，读取 `main.glsl` 和可选的 `config.json`，通过已有 `Program`、`EffectRenderer::pass`、几何与贴图复制原语执行。新增部分仅解释资源与通道配置，不提供另一种默认噪域外观。

支持标准资源声明和配置：

- `uniform sampler2D 名称; // @texture 相对路径`，以及 `wrap=clamp/repeat/mirror`、`filter=nearest/linear`、`colorspace=linear/srgb`、`alphaMode=straight/premult`。
- `float/vec2/vec4` 注释默认值、层级 `parameters` 覆盖和 pass `constants`。
- `targets`、缩放尺寸、`r8/rg8/rgba8`、多 pass 输入输出、`.r/.g/.b/.a` 分通道写入。
- `lowPerformance` 通道、层级 `channel` 选择、`blockMask` 六个状态位以及屏幕/目标/采样纹理尺寸参数。
- padding 与逻辑 UV、纹理采样区域、持久 `noClear` 目标、同一时刻重绘缓存和时间回退清理。
- `touchCount` 与 32 个 `touchPoints` 的游戏触摸绑定。

普通判定线贴图仍走原有 RPE 图片/GIF 加载路径，不改成特殊的 Next 贴图渲染方法。

复测：

```sh
cargo test --locked -p prpr --lib
cargo run --locked -p prpr --example rpe_next_skin_regression -- /path/to/rpe-next
```

皮肤图形检查直接读取所给 rpe-next 源码树的 `assets/noise-skins/NoiseField` 和 `Ocean`，测试普通/低性能通道，同时验证自定义图片与参数、缩放 RG 通道写入、缓存重绘和时间回退。测试不向谱面自动补入或替换皮肤资源。
