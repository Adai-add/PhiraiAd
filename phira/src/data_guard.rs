//! Recovery files intentionally live outside the legacy data directory.
use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

fn backups(root: &Path) -> [PathBuf; 2] {
    let base = root.parent().unwrap_or(root);
    [base.join("phiraiad-recovery-a/data.json"), base.join("phiraiad-recovery-b/data.json")]
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing data parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn load<T: serde::de::DeserializeOwned + Default>(root: &Path) -> Result<T> {
    let primary = root.join("data.json");
    let mut failures = Vec::new();
    for path in std::iter::once(primary.clone()).chain(backups(root)) {
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
                Ok(data) => {
                    // Preserve the exact pre-migration document, before init writes.
                    for backup in backups(root) {
                        if !backup.exists() {
                            atomic_write(&backup, &bytes)?;
                        }
                    }
                    if path != primary {
                        tracing::warn!(source = %path.display(), "restored saved data from recovery copy");
                        if primary.exists() {
                            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
                            std::fs::copy(&primary, root.join(format!("data-unreadable-{stamp}.json")))?;
                        }
                        atomic_write(&primary, &bytes)?;
                    }
                    return Ok(data);
                }
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    anyhow::ensure!(failures.is_empty(), "Saved data could not be read; original files were preserved: {}", failures.join("; "));
    Ok(T::default())
}

pub fn save<T: serde::Serialize + serde::de::DeserializeOwned>(root: &Path, data: &T) -> Result<()> {
    let bytes = serde_json::to_vec(data)?;
    // Check serialization before touching any existing copy.
    let _: T = serde_json::from_slice(&bytes)?;
    // A follows successful saves; B preserves the initial pre-migration snapshot.
    // Legacy startup resets cannot overwrite the protected baseline.
    let [latest, baseline] = backups(root);
    if !baseline.exists() {
        atomic_write(&baseline, &bytes)?;
    }
    atomic_write(&latest, &bytes)?;
    atomic_write(&root.join("data.json"), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct Data {
        score: u32,
    }
    #[test]
    fn restores_corrupt_primary_and_keeps_two_separate_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        save(&root, &Data::default()).unwrap();
        for path in backups(&root) {
            assert!(path.exists());
            assert!(!path.starts_with(&root));
        }
        std::fs::write(root.join("data.json"), b"broken").unwrap();
        let _: Data = load(&root).unwrap();
        assert!(std::fs::read_dir(&root)
            .unwrap()
            .any(|p| p.unwrap().file_name().to_string_lossy().starts_with("data-unreadable-")));
        let _: Data = serde_json::from_slice(&std::fs::read(root.join("data.json")).unwrap()).unwrap();
    }
    #[test]
    fn unreadable_data_is_not_replaced_with_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("data.json"), b"broken").unwrap();
        assert!(load::<Data>(tmp.path()).is_err());
        assert_eq!(std::fs::read(tmp.path().join("data.json")).unwrap(), b"broken");
    }
}

#[cfg(test)]
mod baseline_test {
    use super::*;
    #[test]
    fn later_saves_preserve_initial_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        save(&root, &vec![1_u32]).unwrap();
        save(&root, &vec![2_u32]).unwrap();
        let [latest, baseline] = backups(&root);
        assert_eq!(std::fs::read(baseline).unwrap(), b"[1]");
        assert_eq!(std::fs::read(latest).unwrap(), b"[2]");
        assert_eq!(load::<Vec<u32>>(&root).unwrap(), vec![2]);
    }
}

#[cfg(test)]
mod recovery_test {
    use super::*;
    #[test]
    fn falls_back_to_baseline_when_latest_is_also_broken() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        save(&root, &vec![42_u32]).unwrap();
        let [latest, baseline] = backups(&root);
        std::fs::write(root.join("data.json"), b"broken").unwrap();
        std::fs::write(&latest, b"broken").unwrap();
        assert_eq!(load::<Vec<u32>>(&root).unwrap(), vec![42]);
        assert_eq!(std::fs::read(&baseline).unwrap(), b"[42]");
    }
}
