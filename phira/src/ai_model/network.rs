use super::{hash, Features};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
#[derive(Deserialize)]
struct Index {
    name: String,
    shape: Vec<usize>,
    offset_bytes: usize,
    nbytes: usize,
}
struct Tensor {
    shape: Vec<usize>,
    data: Vec<f32>,
}
pub struct Model {
    weights: HashMap<String, Tensor>,
    meta: serde_json::Value,
}
fn relu(mut v: Vec<f32>) -> Vec<f32> {
    for x in &mut v {
        *x = x.max(0.);
    }
    v
}
fn sigmoid(x: f32) -> f32 {
    let e = (-x.abs()).exp();
    if x >= 0. {
        1. / (1. + e)
    } else {
        e / (1. + e)
    }
}
fn softmax(v: &mut [f32]) {
    let m = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut s = 0.;
    for x in v.iter_mut() {
        *x = (*x - m).exp();
        s += *x;
    }
    for x in v {
        *x /= s;
    }
}
fn pool(rows: &[Vec<f32>]) -> Vec<f32> {
    let d = rows[0].len();
    let mut mean = vec![0.; d];
    let mut max = vec![f32::NEG_INFINITY; d];
    for r in rows {
        for j in 0..d {
            mean[j] += r[j];
            max[j] = max[j].max(r[j]);
        }
    }
    for x in &mut mean {
        *x /= rows.len() as f32;
    }
    mean.extend(max);
    mean
}
impl Model {
    pub fn new(meta: &[u8], bytes: &[u8]) -> Result<Self> {
        let meta: serde_json::Value = serde_json::from_slice(meta)?;
        ensure!(meta["format"] == "phira-events-stats-f32-1" && meta["config"]["strategy"] == "events_stats", "模型格式不支持");
        ensure!(meta["weights_sha256"].as_str() == Some(hash(bytes).as_str()), "模型权重校验失败");
        let mut weights = HashMap::new();
        let mut end = 0;
        for t in serde_json::from_value::<Vec<Index>>(meta["tensors"].clone())? {
            ensure!(t.offset_bytes == end && t.nbytes == t.shape.iter().product::<usize>() * 4 && end + t.nbytes <= bytes.len(), "权重索引无效");
            let data = bytes[end..end + t.nbytes]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            ensure!(data.iter().all(|x| x.is_finite()), "权重数值无效");
            end += t.nbytes;
            weights.insert(t.name, Tensor { shape: t.shape, data });
        }
        ensure!(end == bytes.len(), "权重长度无效");
        Ok(Self { weights, meta })
    }
    fn w(&self, key: &str) -> &Tensor {
        &self.weights[key]
    }
    fn linear(&self, x: &[f32], name: &str) -> Vec<f32> {
        let w = self.w(&format!("{name}.weight"));
        let b = self.weights.get(&format!("{name}.bias"));
        assert_eq!(w.shape[1], x.len());
        w.data
            .chunks_exact(x.len())
            .enumerate()
            .map(|(i, row)| row.iter().zip(x).map(|(a, b)| a * b).sum::<f32>() + b.map_or(0., |b| b.data[i]))
            .collect()
    }
    fn norm(&self, mut x: Vec<f32>, name: &str) -> Vec<f32> {
        let mean = x.iter().sum::<f32>() / x.len() as f32;
        let var = x.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / x.len() as f32;
        let scale = (var + 1e-5).sqrt();
        let w = self.w(&format!("{name}.weight"));
        let b = self.w(&format!("{name}.bias"));
        for (i, v) in x.iter_mut().enumerate() {
            *v = (*v - mean) / scale * w.data[i] + b.data[i];
        }
        x
    }
    fn transformer(&self, x: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let w = self.w("local.0.self_attn.in_proj_weight");
        let bias = &self.w("local.0.self_attn.in_proj_bias").data;
        let qkv = x
            .iter()
            .map(|r| {
                w.data
                    .chunks_exact(32)
                    .enumerate()
                    .map(|(j, w)| w.iter().zip(r).map(|(a, b)| a * b).sum::<f32>() + bias[j])
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut result = Vec::new();
        for i in 0..x.len() {
            let mut attention = vec![0.; 32];
            for h in 0..4 {
                let mut scores = (0..x.len())
                    .map(|j| (0..8).map(|k| qkv[i][h * 8 + k] * qkv[j][32 + h * 8 + k]).sum::<f32>() / 8f32.sqrt())
                    .collect::<Vec<_>>();
                softmax(&mut scores);
                for j in 0..x.len() {
                    for k in 0..8 {
                        attention[h * 8 + k] += scores[j] * qkv[j][64 + h * 8 + k];
                    }
                }
            }
            let a = self.linear(&attention, "local.0.self_attn.out_proj");
            let z = self.norm(x[i].iter().zip(a).map(|(a, b)| a + b).collect(), "local.0.norm1");
            let f = self.linear(&relu(self.linear(&z, "local.0.linear1")), "local.0.linear2");
            result.push(self.norm(z.iter().zip(f).map(|(a, b)| a + b).collect(), "local.0.norm2"));
        }
        result
    }
    fn standardize(&self, row: &[f64], name: &str) -> Result<Vec<f32>> {
        let p = &self.meta["preprocessor"][name];
        let mean = p["mean"].as_array().context("缺少标准化均值")?;
        let scale = p["scale"].as_array().context("缺少标准化尺度")?;
        ensure!(mean.len() == row.len() && scale.len() == row.len(), "标准化维度不匹配");
        row.iter()
            .enumerate()
            .map(|(i, x)| {
                let s = scale[i].as_f64().context("标准化尺度无效")?;
                let m = mean[i].as_f64().context("均值无效")?;
                ensure!(s > 0. && s.is_finite(), "标准化尺度无效");
                let v = ((x - m) / s) as f32;
                ensure!(v.is_finite(), "输入超出模型数值范围");
                Ok(v)
            })
            .collect()
    }
    pub fn predict(&self, f: &Features, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<f32> {
        let mut groups = Vec::new();
        let mut ni = 0;
        for g in 0..f.group_features.len() {
            if g % 16 == 0 {
                checkpoint()?;
            }
            let mut mean = vec![0.; 32];
            let mut max = vec![f32::NEG_INFINITY; 32];
            let mut count = 0;
            while ni < f.events.len() && f.note_groups[ni] == g {
                if ni % 128 == 0 {
                    checkpoint()?;
                }
                let r = self.standardize(&f.events[ni], "events")?;
                let h = relu(self.linear(&relu(self.linear(&r, "note_encoder.0")), "note_encoder.2"));
                for j in 0..32 {
                    mean[j] += h[j];
                    max[j] = max[j].max(h[j]);
                }
                count += 1;
                ni += 1;
            }
            for x in &mut mean {
                *x /= count as f32;
            }
            mean.extend(max);
            mean.extend(self.standardize(&f.group_features[g], "group_features")?);
            groups.push(relu(self.linear(&mean, "group_encoder.0")));
        }
        let mut blocks = Vec::new();
        let mut durations = Vec::new();
        for start in (0..groups.len()).step_by(32) {
            checkpoint()?;
            let stop = (start + 32).min(groups.len());
            let left = start.saturating_sub(4);
            let right = (stop + 4).min(groups.len());
            let z = self.transformer(&groups[left..right]);
            blocks.push(relu(self.linear(&pool(&z[start - left..stop - left]), "block_projection.0")));
            durations.push(f.group_durations[start..stop].iter().map(|x| *x as f32).sum::<f32>());
        }
        let mut state = vec![0.; 32];
        let mat = |x: &[f32], wn: &str, bn: &str| {
            let b = &self.w(bn).data;
            self.w(wn)
                .data
                .chunks_exact(32)
                .enumerate()
                .map(|(i, r)| r.iter().zip(x).map(|(a, b)| a * b).sum::<f32>() + b[i])
                .collect::<Vec<_>>()
        };
        let mut logits = Vec::new();
        for (i, x) in blocks.iter().enumerate() {
            if i % 32 == 0 {
                checkpoint()?;
            }
            let gi = mat(x, "gru.weight_ih_l0", "gru.bias_ih_l0");
            let gh = mat(&state, "gru.weight_hh_l0", "gru.bias_hh_l0");
            for j in 0..32 {
                let r = sigmoid(gi[j] + gh[j]);
                let z = sigmoid(gi[32 + j] + gh[32 + j]);
                let n = (gi[64 + j] + r * gh[64 + j]).tanh();
                state[j] = (1. - z) * n + z * state[j];
            }
            let a = self.linear(x, "attention.value_gate");
            let b = self.linear(x, "attention.sigmoid_gate");
            let gate = a.iter().zip(b).map(|(a, b)| a.tanh() * sigmoid(b)).collect::<Vec<_>>();
            let mut l = self.linear(&gate, "attention.score");
            for v in &mut l {
                *v += durations[i].max(1e-12).ln();
            }
            logits.push(l);
        }
        let total = durations.iter().sum::<f32>();
        let mut pooled = vec![0.; 32];
        let mut max = vec![f32::NEG_INFINITY; 32];
        for (i, b) in blocks.iter().enumerate() {
            for j in 0..32 {
                pooled[j] += b[j] * (durations[i] / total);
                max[j] = max[j].max(b[j]);
            }
        }
        pooled.extend(max);
        pooled.extend(state);
        for h in 0..2 {
            let mut alpha = logits.iter().map(|l| l[h]).collect::<Vec<_>>();
            softmax(&mut alpha);
            let mut v = vec![0.; 32];
            for (i, b) in blocks.iter().enumerate() {
                for j in 0..32 {
                    v[j] += alpha[i] * b[j];
                }
            }
            pooled.extend(v);
        }
        let a = self.linear(&pooled, "head.0");
        let b = self.linear(&self.standardize(&f.stats, "stats")?, "stats_projection");
        let h = relu(a.iter().zip(b).map(|(a, b)| a + b).collect());
        let value = self.linear(&h, "head.3")[0];
        ensure!(value.is_finite(), "预测溢出");
        Ok(value)
    }
}
