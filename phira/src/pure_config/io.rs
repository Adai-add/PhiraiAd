//! Resource copying and buffered JSON output with byte-level progress.
use super::{Progress, Stage};
use anyhow::{ensure, Result};
use serde_json::Value;
use std::{fs::{self, File}, io::{self, BufWriter, Read, Write}, path::{Path, PathBuf}};

pub struct CopyPlan { entries: Vec<(PathBuf, PathBuf, bool)>, pub bytes: u64 }
impl CopyPlan {
    pub fn new(source: &Path, progress: &Progress) -> Result<Self> {
        let mut plan = Self { entries: Vec::new(), bytes: 0 };
        fn visit(source: &Path, relative: &Path, plan: &mut CopyPlan, progress: &Progress) -> Result<()> {
            for entry in fs::read_dir(source.join(relative))? {
                progress.check()?;
                let entry = entry?;
                let name = relative.join(entry.file_name());
                let kind = entry.file_type()?;
                ensure!(!kind.is_symlink(), "资源不能包含符号链接：{}", name.display());
                // Only root files that will be replaced by publication are skipped.
                if relative.as_os_str().is_empty() && kind.is_file() && matches!(name.to_str(), Some("chart.json" | "info.yml" | "pure-config-report.json")) { continue; }
                if kind.is_dir() {
                    plan.entries.push((entry.path(), name.clone(), true));
                    visit(source, &name, plan, progress)?;
                } else if kind.is_file() {
                    plan.bytes = plan.bytes.checked_add(entry.metadata()?.len()).ok_or_else(|| anyhow::anyhow!("资源大小超出范围"))?;
                    plan.entries.push((entry.path(), name, false));
                }
            }
            Ok(())
        }
        visit(source, Path::new(""), &mut plan, progress)?;
        progress.weight(Stage::Copy, plan.bytes as f64 / 2_000_000. + 1.);
        Ok(plan)
    }
    pub fn execute(&self, destination: &Path, progress: &Progress) -> Result<()> {
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut done = 0;
        progress.update(Stage::Copy, 0, self.bytes);
        for (src, relative, directory) in &self.entries {
            progress.check()?;
            let dst = destination.join(relative);
            if *directory { fs::create_dir_all(dst)?; continue; }
            if let Some(parent) = dst.parent() { fs::create_dir_all(parent)?; }
            let mut input = File::open(src)?; let mut output = File::create(dst)?;
            loop {
                progress.check()?;
                let n = input.read(&mut buffer)?; if n == 0 { break; }
                output.write_all(&buffer[..n])?;
                done += n as u64;
                progress.update(Stage::Copy, done, self.bytes);
            }
        }
        progress.finish(Stage::Copy);
        Ok(())
    }
}
struct Counter<'a> { bytes: u64, next_check: u64, progress: &'a Progress }
impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes += bytes.len() as u64;
        if self.bytes >= self.next_check { self.progress.check().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?; self.next_check = self.bytes + 65536; }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
struct Meter<'a> { file: File, bytes: u64, total: u64, progress: &'a Progress }
impl Write for Meter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.progress.check().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let n = self.file.write(bytes)?; self.bytes += n as u64;
        self.progress.update(Stage::Write, self.bytes, self.total);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> { self.file.flush() }
}
pub fn write_json(path: &Path, value: &Value, progress: &Progress) -> Result<()> {
    progress.check()?;
    progress.update(Stage::Write, 0, 0);
    // Count without retaining another serialized chart in memory. The same
    // serializer runs twice, but only the buffered pass performs filesystem I/O.
    let mut counter = Counter { bytes: 0, next_check: 0, progress };
    serde_json::to_writer(&mut counter, value)?;
    let total = counter.bytes;
    progress.weight(Stage::Write, total as f64 / 100_000. + 1.);
    progress.update(Stage::Write, 0, total);
    let mut writer = BufWriter::with_capacity(256 * 1024, Meter { file: File::create(path)?, bytes: 0, total, progress });
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    writer.get_ref().file.sync_all()?;
    progress.finish(Stage::Write);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn directory() -> PathBuf {
        let p = std::env::temp_dir().join(format!("pure-config-io-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&p).unwrap(); p
    }
    #[test]
    fn resources_are_counted_and_preserved_except_replaced_root_files() {
        let root = directory(); let src = root.join("src"); let dst = root.join("dst"); fs::create_dir_all(src.join("nested")).unwrap(); fs::create_dir_all(&dst).unwrap();
        for name in ["chart.json", "info.yml", "pure-config-report.json"] { fs::write(src.join(name), b"replaced").unwrap(); }
        fs::write(src.join("nested/chart.json"), b"resource").unwrap(); fs::write(src.join("music.wav"), vec![7u8; 2_100_000]).unwrap();
        let p = Progress::default(); let plan = CopyPlan::new(&src, &p).unwrap(); assert_eq!(plan.bytes, 2_100_008); plan.execute(&dst, &p).unwrap();
        assert!(!dst.join("chart.json").exists()); assert_eq!(fs::read(dst.join("nested/chart.json")).unwrap(), b"resource"); assert_eq!(fs::read(dst.join("music.wav")).unwrap(), fs::read(src.join("music.wav")).unwrap());
        assert_eq!(p.snapshot().fraction, 1.); fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn buffered_json_is_exact_and_cancellation_is_honored() {
        let root = directory(); let path = root.join("chart.json"); let p = Progress::default();
        let value = serde_json::json!({"text":"换行\n引号\"", "notes":vec![123.456; 100000]}); write_json(&path, &value, &p).unwrap();
        assert_eq!(fs::read(&path).unwrap(), serde_json::to_vec(&value).unwrap()); assert!(p.snapshot().overall < 1.);
        p.cancelled.store(true, Ordering::Relaxed);
        assert!(write_json(&path, &value, &p).is_err());
        let mut counter = Counter { bytes: 0, next_check: 0, progress: &p };
        assert!(counter.write_all(b"cancelled").is_err());
        let mut meter = Meter { file: File::create(root.join("cancelled.json")).unwrap(), bytes: 0, total: 9, progress: &p };
        assert!(meter.write_all(b"cancelled").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
