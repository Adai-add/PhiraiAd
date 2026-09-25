//! Fixed five-model inference; no network, labels, player data or gameplay mods.
pub mod cache;
pub mod features;
pub mod index;
mod network;
pub mod scheduler;
use anyhow::{ensure, Result};
pub use features::Features;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const VERSION: &str = "kit1-whole-events1-seed42-fold1-5-native1";
pub const CACHE_FIELD: &str = "phiraAiDifficulty";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prediction {
    pub version: String,
    pub chart_hash: String,
    pub model_hash: String,
    pub model_count: usize,
    pub values: Vec<f64>,
    pub mean: f64,
}
impl Prediction {
    pub fn valid(&self, hash: &str) -> bool {
        self.version == VERSION
            && self.chart_hash == hash
            && self.model_hash == model_hash()
            && self.model_count == 5
            && self.values.len() == 5
            && self.values.iter().all(|x| x.is_finite())
            && self.mean.is_finite()
            && (self.mean - self.values.iter().sum::<f64>() / 5.).abs() < 1e-10
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn model_hash() -> String {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| hash(include_bytes!("assets/manifest.json"))).clone()
}
pub struct Predictor {
    models: Vec<network::Model>,
}
impl Predictor {
    pub fn new() -> Result<Self> {
        let manifest: serde_json::Value = serde_json::from_slice(include_bytes!("assets/manifest.json"))?;
        let assets: &[(&[u8], &[u8])] = &[
            (include_bytes!("assets/seed42_fold1.json"), include_bytes!("assets/seed42_fold1.bin")),
            (include_bytes!("assets/seed42_fold2.json"), include_bytes!("assets/seed42_fold2.bin")),
            (include_bytes!("assets/seed42_fold3.json"), include_bytes!("assets/seed42_fold3.bin")),
            (include_bytes!("assets/seed42_fold4.json"), include_bytes!("assets/seed42_fold4.bin")),
            (include_bytes!("assets/seed42_fold5.json"), include_bytes!("assets/seed42_fold5.bin")),
        ];
        let mut models = Vec::new();
        for (i, (meta, weights)) in assets.iter().enumerate() {
            ensure!(hash(meta) == manifest["models"][i]["metadata_sha256"].as_str().unwrap_or(""), "模型元数据校验失败");
            models.push(network::Model::new(meta, weights)?);
        }
        Ok(Self { models })
    }
    pub fn predict(&self, features: &Features, chart_hash: String, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<Prediction> {
        features.validate()?;
        let mut values = Vec::new();
        for model in &self.models {
            checkpoint()?;
            values.push(model.predict(features, checkpoint)? as f64);
        }
        let mean = values.iter().sum::<f64>() / 5.;
        ensure!(mean.is_finite(), "预测结果无效");
        Ok(Prediction {
            version: VERSION.into(),
            chart_hash,
            model_hash: model_hash(),
            model_count: 5,
            values,
            mean,
        })
    }
}
