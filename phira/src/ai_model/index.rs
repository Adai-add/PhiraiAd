//! Small app-local index for immediate restart recovery without chart parsing.
use super::{
    cache::{atomic_write, safe_relative, Source},
    Prediction,
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    file: PathBuf,
    bytes: u64,
    configured_format: Option<String>,
    modern: bool,
    pub format: String,
    pub prediction: Prediction,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Index {
    pub records: BTreeMap<String, Record>,
}
pub fn chart_root(charts: &Path, path: &str) -> Option<PathBuf> {
    let mapped = if path.starts_with(':') {
        path.replace(':', "_")
    } else {
        path.to_owned()
    };
    safe_relative(Path::new(&mapped)).then(|| charts.join(mapped))
}
impl Record {
    pub fn restore(&self, root: &Path) -> Option<Source> {
        if !safe_relative(&self.file) {
            return None;
        }
        let root = root.canonicalize().ok()?;
        let file = root.join(&self.file).canonicalize().ok()?;
        if !file.starts_with(&root) || file.metadata().ok()?.len() != self.bytes {
            return None;
        }
        let source = Source {
            file,
            bytes: self.bytes,
            format: self.configured_format.clone(),
            modern: self.modern,
        };
        self.prediction.valid(&source.key(&self.format)).then_some(source)
    }
}
impl Index {
    pub fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, file: &Path) -> Result<()> {
        atomic_write(file, &serde_json::to_vec(self)?)
    }
    pub fn insert(&mut self, path: &str, root: &Path, source: &Source, format: &str, prediction: Prediction) -> Result<()> {
        ensure!(prediction.valid(&source.key(format)), "索引定数无效");
        let root = root.canonicalize()?;
        let relative = source.file.strip_prefix(root)?;
        ensure!(safe_relative(relative), "索引路径越界");
        self.records.insert(
            path.into(),
            Record {
                file: relative.into(),
                bytes: source.bytes,
                configured_format: source.format.clone(),
                modern: source.modern,
                format: format.into(),
                prediction,
            },
        );
        Ok(())
    }
}
