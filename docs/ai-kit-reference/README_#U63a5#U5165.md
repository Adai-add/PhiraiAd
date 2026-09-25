# Phigros → Phira 定数推理集成包 1.0.0

这是当前使用的 `events_stats` 方案的完整推理交付包：原谱解析 → 全音符特征与统计 → 每个模型独立标准化 → 局部 Transformer + GRU + 注意力汇总 → 模型平均。包含实际训练好的权重，直接预测，无需训练。

## 文件导航

| 路径 | 内容 |
|---|---|
| `reference_predictor/` | 当前 1.0.1 Python 预测器完整源码、图形界面、CLI、15 份 `.pt` 权重、区间均值评估脚本 |
| `portable_models/` | 同一批 15 个模型的无损 float32 二进制权重、张量索引、标准化参数、分析配置、特征契约、训练/验证身份记录 |
| `portable/runtime.py` | 仅依赖 NumPy 的完整网络推理参考，不导入 PyTorch；原谱提取复用 reference_predictor 中的纯 Python/NumPy 模块 |
| `predict_portable.py` | 通用权重版预测入口 |
| `算法与特征规范.md` | 时间、几何、音符、同刻组及整谱统计定义 |
| `权重格式与网络公式.md` | 二进制布局、全部网络算子、GRU 门顺序、归一化与汇总规则 |
| `Phira接入步骤.md` | 移植接口、逐层验证、后台执行与缓存建议 |
| `test_vectors/` | 5 个合成原谱、对应未标准化特征和 15 模型期望输出 |
| `verification_report.json` | 本次 CPU 数值一致性验证结果；另包含真实谱面的验证记录 |
| `tools/` | 可重复导出权重、生成测试向量、检查数值的程序 |
| `SHA256SUMS.txt` | 发布文件完整性清单 |

## 快速预测

在解压后的包根目录执行：

```bash
python -m pip install -r requirements_portable.txt
python predict_portable.py "你的谱面.json" --output result.json
python tools/verify_bundle.py
```

`predict_portable.py` 接受原版 `formatVersion: 3` JSON。如果输入是 ZIP/PEZ，使用已有完整预测器：

```bash
python -m pip install -r reference_predictor/requirements.txt
python reference_predictor/predict.py --help
```

也可以启动 `reference_predictor/start.bat` 使用桌面界面。解压后请保留目录结构。

## 当前推理规则

- `auto` 为默认值，与目前预测器一致：开发集内可识别的原谱使用三个折外模型；未匹配或原保留集谱面使用全部 15 个模型取均值。
- 原谱身份按原始 JSON 内容的规范化哈希识别。文件名和其中的定数不参与网络输入。
- `--selection all15` 可显式固定使用全部模型，适合开发时核对同一个运行路径；其中可能有模型训练过已知原版谱，不能把此结果当作折外评估。默认不启用。
- 不使用校准函数，不使用旧版 79 项分段输入，不把模型输出截断到训练区间。
- 保留每模型预测及模型间标准差；标准差是模型分歧，不是置信区间。

## 验证范围与限制

15 个模型在 5 个合成谱和一个真实谱上，与原 PyTorch 实现的最大预测差异为 `0.00000190735`。真实谱的事件数组与原提取器一致。测试包含多押、Hold 重叠、上下侧、移动旋转、负/零速度、32 组块边界以及零时长速度事件。

现有开发集记录：单模型跨折平均 OOF MAE 约 `0.52455`，三个种子的折外集成 MAE 约 `0.49500`。这些不是全部 15 模型对全新自制谱的已验证精度，不能作为对任意 Phira 谱面的准确率保证。

本包是源码和模型资产交付，**尚未编译进你的 Phira 改版，也没有在 Android/Rust 环境中执行验证**。`.bin + .json` 是这里定义的通用张量格式，不是 ONNX/TFLite，也不能直接交给这些运行时加载。`portable/runtime.py` 提供可核对的移植依据。未提供 Phira 改版源码，因此包内不包含猜测工程结构的 Android 补丁。

只支持原版 formatVersion=3。RPE、PEC、其他 Phira 格式需要经过保持时间和几何语义的转换；扩展假音符当前明确拒绝。定数训练范围约 9～17.6，对范围外或非常规手法谱仍需实际检验。推理按原始速度进行，不根据播放速度简单乘定数。

模型和源码随包自包含；不需要旧分析工作台配置、安装包、音频、曲绘或训练数据目录。真实原版谱面本体未复制进集成包，测试目录使用合成谱。
