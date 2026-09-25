# 权重格式与网络公式

## 二进制资产

`portable_models/manifest.json` 索引 3 个种子 × 5 折，共 15 模型。每个模型有一个 JSON 和一个 BIN。BIN 仅包含按 JSON 的 `tensors` 顺序拼接的张量，无文件头；每个元素为小端 IEEE-754 float32，连续 row-major，无量化、无额外 padding。每模型 27,713 个数，110,852 字节；共 415,695 个数。训练检查点 `.pt` 另外保留。

张量索引包含 name、shape、offset_bytes、nbytes。Linear 权重形状为 [out_features,in_features]，数学运算 `x @ W.T + b`。按 SHA-256 校验元数据和权重；模型 config、analysis_config、event_contract、preprocessor 全部在同模型 JSON 中。manifest 的模型顺序是参考输出顺序。

## 网络结构

记 N 为音符数，G 为同时组数。以下均为推理阶段，禁用 Dropout，固定 phase=0。所有非线性/归一化在 float32 中执行，最终模型均值建议 float64 累加。

1. **音符编码**：17→32 Linear，ReLU，32→32 Linear，ReLU，得到 [N,32]。
2. **同时组编码**：按 note_groups 分别求 32 维均值与最大值，拼接该组 6 维标准化特征，得到 70 维；Linear 70→32 + ReLU。
3. **块与上下文**：核心块为连续 32 组；两边各扩展最多 4 组作为上下文，谱头尾截断。最后核心块可不足 32。没有显式位置编码。上下文组能重复出现在相邻块，但只汇总属于当前核心块的输出。
4. **局部 Transformer**：每块扩展序列独立通过 1 层 TransformerEncoderLayer，维数 32，4 个头，每头 8，FFN 64，ReLU，post-norm，LayerNorm epsilon=1e-5。代码中的 block_micro_batch=256 仅限制块批处理量，不改变模型语义；原生逐块计算也可。
5. **块编码**：Transformer 输出的核心部分按组做无权均值和最大值，拼接 64 维 → Linear 32 + ReLU。块持续时间是核心组持续时间之和。
6. **全谱汇总**：块序列分别作持续时间加权均值（32）、最大值（32）、GRU 最终隐藏状态（32）、两头门控注意力汇总（64），按此顺序拼成 160 维。
7. **定数头**：`hidden = ReLU(Linear160→32(pooled) + Linear18→32(stats))`，统计分支无 bias。最后 Linear32→1，无 sigmoid、无裁剪。

## Transformer 精确算子

- `in_proj_weight` 为 [96,32]；输出依次分成 Q、K、V，各 32 维。
- 每个 Q/K/V reshape 为 [length,4,8]，按头分别计算 `softmax(Q K^T / sqrt(8)) V`，softmax 对 key 维。
- 拼回 32 维，经过 self_attn.out_proj。
- `z = LN(x + attention_output)`。
- `y = LN(z + linear2(ReLU(linear1(z))))`。
- LayerNorm 每个位置按 32 个通道计算总体方差 `mean((x−mean(x))²)`，不是样本方差；再乘 norm.weight、加 norm.bias。
- 批量填充实现要对无效 key 屏蔽，核心池化也不能计入 padding。参考 NumPy 逐块不填充，与之等价。

## GRU 精确算子

单层、单向，输入与 hidden 均 32 维，初始 h=0。注意 PyTorch 的权重门顺序为 **r、z、n**；reset 门包含在 recurrent candidate bias 外侧，这是移植常见差异点。

```
gi = W_ih @ x + b_ih
gh = W_hh @ h + b_hh
r = sigmoid(gi_r + gh_r)
z = sigmoid(gi_z + gh_z)
n = tanh(gi_n + r * gh_n)
h = (1 - z) * n + z * h
```

使用最后一个真实块后的 h。GRU 不改变另三条汇总分支的输入；它们仍使用 GRU 前的块向量。

## 门控注意力与权重

每个块向量 b：

```
u = tanh(value_gate(b)) * sigmoid(sigmoid_gate(b))  # 16维
logits = score(u) + log(max(block_duration, 1e-12)) # 2维，score无bias
alpha = softmax(logits, 按块维分别对每个头计算)
head_k = sum(alpha[:, k] * blocks)
```

按 head0 的32维、head1 的32维依次展平。持续时间在 softmax 中作为先验，不能去掉。全谱普通均值分支也按块持续时间加权，最大值分支不加权。最终 **模型之间** 则是等权平均，没有额外学习的模型融合权重。

## 数值对照

`python tools/verify_bundle.py` 只需 NumPy，核对原谱提取结果、每模型输出与平均值。`python tools/verify_bundle.py --torch` 还现场比较 PyTorch。移植允许的测试向量绝对误差设为 1e-4；本次实际最大误差约 1.91e-6。该容差用于判断实现是否对齐，与定数预测 MAE 是不同概念。

不建议未验证就改用 float16、量化、替换 LayerNorm、减少模型数或删除分支。那些是新的推理方案，需要单独评估误差与精度。通用导出脚本 `tools/export_portable.py` 可从随包原始检查点重新生成资产，没有训练步骤。
