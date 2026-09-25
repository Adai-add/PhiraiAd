//! Embedded prediction caches. Chart changes are detected by file byte length.
use super::{Prediction, CACHE_FIELD};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};
pub const MAX_CHART_BYTES: u64 = 127 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub file: PathBuf,
    pub bytes: u64,
    pub format: Option<String>,
    pub modern: bool,
}
impl Source {
    /// Read only info.yml and filesystem metadata, never the chart body.
    pub fn probe(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let yaml: serde_yaml::Value = serde_yaml::from_slice(&read_limited(&root.join("info.yml"))?)?;
        let rel = Path::new(yaml.get("chart").and_then(|v| v.as_str()).context("谱面信息缺少 chart")?);
        ensure!(safe_relative(rel), "谱面路径越界");
        let mut file = root.join(rel);
        if !file.exists() && file.extension().is_some_and(|x| x == "pec") {
            file.set_extension("json");
        }
        let file = file.canonicalize()?;
        ensure!(file.starts_with(&root), "谱面路径越界");
        let bytes = file.metadata()?.len();
        ensure!(bytes <= MAX_CHART_BYTES, "谱面文件超过 127 MiB");
        Ok(Self {
            file,
            bytes,
            format: yaml.get("format").and_then(|v| v.as_str()).map(str::to_owned),
            modern: yaml.get("useRpe170Speed").and_then(|v| v.as_bool()).unwrap_or(false),
        })
    }
    pub fn key(&self, detected_format: &str) -> String {
        let format = self.format.as_deref().unwrap_or(detected_format);
        // chartHash is retained as a JSON field for backwards compatibility;
        // its new value is an explicit size/options key, not a content hash.
        format!("bytes:{}:{}:{}", self.bytes, format, format == "rpe" && self.modern)
    }
}
pub fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}
#[derive(Clone)]
pub struct Document {
    pub file: PathBuf,
    pub info: PathBuf,
    pub bytes: Vec<u8>,
    pub raw: Option<Value>,
    pub hash: String,
    pub format: String,
    pub source: Source,
}
fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    ensure!(len <= MAX_CHART_BYTES, "谱面文件超过 127 MiB");
    let mut bytes = Vec::with_capacity(len as usize + 1);
    file.take(MAX_CHART_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_CHART_BYTES, "谱面文件超过 127 MiB");
    Ok(bytes)
}
impl Document {
    pub fn load(root: &Path) -> Result<Self> {
        let source = Source::probe(root)?;
        let info = root.canonicalize()?.join("info.yml");
        let mut bytes = read_limited(&source.file)?;
        ensure!(bytes.len() as u64 == source.bytes, "谱面已变更，重新排队");
        let parse = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let raw = serde_json::from_slice::<Value>(parse).ok().filter(Value::is_object);
        let format = source.format.clone().unwrap_or_else(|| {
            if let Some(raw) = &raw {
                if raw.get("META").is_some() {
                    "rpe"
                } else {
                    "pgr"
                }
            } else if source.file.extension().is_some_and(|v| v == "pbc") || std::str::from_utf8(&bytes).is_err() {
                "pbc"
            } else {
                "pec"
            }
            .into()
        });
        let hash = source.key(&format);
        if raw.is_some() {
            bytes = Vec::new();
        }
        Ok(Self {
            file: source.file.clone(),
            info,
            bytes,
            raw,
            hash,
            format,
            source,
        })
    }
    pub fn release_input(&mut self) {
        self.raw = None;
        self.bytes = Vec::new();
    }
    fn stored_prediction(&self) -> Option<Prediction> {
        let value = if let Some(raw) = &self.raw {
            raw.get(CACHE_FIELD)?.clone()
        } else {
            let yaml: serde_yaml::Value = serde_yaml::from_slice(&std::fs::read(&self.info).ok()?).ok()?;
            serde_json::to_value(yaml.get(CACHE_FIELD)?).ok()?
        };
        serde_json::from_value(value).ok()
    }
    pub fn needs_migration(&self) -> bool {
        self.stored_prediction().is_some_and(|p| !p.chart_hash.starts_with("bytes:"))
    }
    pub fn cached(&self) -> Option<Prediction> {
        let mut p = self.stored_prediction()?;
        if p.valid(&self.hash) {
            return Some(p);
        }
        // One-time adoption of a legacy embedded cache. Old caches did not store
        // a byte count: validate the model/results, then establish today's size.
        if p.chart_hash.len() == 64 && p.chart_hash.bytes().all(|b| b.is_ascii_hexdigit()) && p.valid(&p.chart_hash) {
            p.chart_hash = self.hash.clone();
            return Some(p);
        }
        None
    }
    /// Returns the cache with its post-write size key, including its own bytes.
    pub fn save(&self, prediction: &Prediction) -> Result<Prediction> {
        ensure!(prediction.valid(&self.hash), "拒绝保存无效预测缓存");
        let latest = Self::load(self.info.parent().unwrap())?;
        ensure!(latest.source == self.source, "谱面已变更，重新排队");
        let mut saved = prediction.clone();
        if let Some(mut raw) = latest.raw {
            let mut source = latest.source;
            // Embedding changes JSON length. Determine the final size before
            // writing, with a bounded-memory counting serializer and fixed point.
            let mut stable = false;
            for _ in 0..8 {
                raw.as_object_mut().unwrap().insert(CACHE_FIELD.into(), serde_json::to_value(&saved)?);
                let mut counter = ByteCounter(0);
                serde_json::to_writer(&mut counter, &raw)?;
                source.bytes = counter.0;
                let key = source.key(&self.format);
                if saved.chart_hash == key {
                    stable = true;
                    break;
                }
                saved.chart_hash = key;
            }
            ensure!(stable, "缓存长度计算失败");
            atomic_write_with(&self.file, |writer| {
                serde_json::to_writer(writer, &raw)?;
                Ok(())
            })?;
        } else {
            let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&std::fs::read(&self.info)?)?;
            yaml.as_mapping_mut()
                .context("info.yml 无效")?
                .insert(serde_yaml::Value::String(CACHE_FIELD.into()), serde_yaml::to_value(&saved)?);
            atomic_write(&self.info, serde_yaml::to_string(&yaml)?.as_bytes())?;
        }
        Ok(saved)
    }
}
struct ByteCounter(u64);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_with(path, |w| Ok(w.write_all(bytes)?))
}
fn atomic_write_with(path: &Path, write: impl FnOnce(&mut BufWriter<&mut std::fs::File>) -> Result<()>) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("缺少目录")?)?;
    {
        let mut writer = BufWriter::new(temp.as_file_mut());
        write(&mut writer)?;
        writer.flush()?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_write_failure_preserves_original_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("chart.json");
        std::fs::write(&file, b"original").unwrap();
        let result = atomic_write_with(&file, |w| {
            w.write_all(b"partial")?;
            anyhow::bail!("simulated failure")
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
