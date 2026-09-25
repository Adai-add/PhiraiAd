//! One cooperative background worker. UI only updates small shared snapshots.
use crate::ai_model::{
    cache::{Document, Source},
    index::{chart_root, Index},
    Predictor,
};
use anyhow::{bail, Result};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
const INTERRUPTED: &str = "AI_WORK_INTERRUPTED";
#[derive(Clone)]
struct Entry {
    stamp: Option<Source>,
    file: Option<PathBuf>,
    value: Option<f64>,
    message: String,
}
struct State {
    paths: BTreeSet<String>,
    priority: BTreeSet<String>,
    automatic: bool,
    heartbeat: Instant,
    last_catalog: Instant,
    entries: HashMap<String, Entry>,
}
struct Service {
    state: Mutex<State>,
    revision: AtomicU64,
    root: PathBuf,
    index_file: PathBuf,
}
static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();
fn service() -> Option<&'static Arc<Service>> {
    SERVICE.get()
}
pub fn tick() {
    let service = SERVICE.get_or_init(|| {
        let root = PathBuf::from(crate::dir::charts().unwrap_or_default());
        let priority = crate::dir::root()
            .ok()
            .and_then(|p| crate::custom_rks::Settings::load(&PathBuf::from(p).join("custom-rks.json")).ok())
            .map(|s| {
                s.charts
                    .into_iter()
                    .filter(|(_, c)| c.difficulty_source == crate::custom_rks::DifficultySource::Ai)
                    .map(|(p, _)| p)
                    .collect()
            })
            .unwrap_or_default();
        let index_file = PathBuf::from(crate::dir::root().unwrap_or_default()).join("ai-difficulty-index.json");
        let index = Index::load(&index_file);
        let entries = index
            .records
            .iter()
            .filter_map(|(path, record)| {
                let source = record.restore(&chart_root(&root, path)?)?;
                Some((
                    path.clone(),
                    Entry {
                        file: Some(source.file.clone()),
                        stamp: Some(source),
                        value: Some(record.prediction.mean),
                        message: "已缓存 · 5 模型均值".into(),
                    },
                ))
            })
            .collect();
        let s = Arc::new(Service {
            state: Mutex::new(State {
                paths: Default::default(),
                priority,
                automatic: false,
                heartbeat: Instant::now(),
                last_catalog: Instant::now() - Duration::from_secs(60),
                entries,
            }),
            revision: AtomicU64::new(1),
            index_file,
            root,
        });
        let worker = s.clone();
        std::thread::Builder::new()
            .name("phira-ai-low".into())
            .spawn(move || worker.run(index))
            .expect("AI worker thread");
        s
    });
    let mut s = service.state.lock().unwrap();
    s.automatic = crate::get_data().config.ai_auto_predict;
    s.heartbeat = Instant::now();
    // Chart paths are cheap identifiers; only rescan the catalog when it changes.
    if s.last_catalog.elapsed() >= Duration::from_secs(5) {
        s.paths.extend(crate::get_data().charts.iter().map(|c| c.local_path.clone()));
        s.last_catalog = Instant::now();
    }
}
pub fn pause() {
    if let Some(s) = service() {
        s.state.lock().unwrap().heartbeat = Instant::now() - Duration::from_secs(60);
    }
}
pub fn catalog(paths: impl IntoIterator<Item = String>) {
    if let Some(s) = service() {
        s.state.lock().unwrap().paths.extend(paths);
    }
}
pub fn priorities(paths: impl IntoIterator<Item = String>) {
    if let Some(s) = service() {
        s.state.lock().unwrap().priority = paths.into_iter().collect();
    }
}
pub fn revision() -> u64 {
    service().map_or(0, |s| s.revision.load(Ordering::Relaxed))
}
pub fn value(path: &str) -> Option<f64> {
    service()?.state.lock().ok()?.entries.get(path)?.value
}
pub fn status(path: &str) -> String {
    service()
        .and_then(|s| s.state.lock().ok()?.entries.get(path).map(|e| e.message.clone()))
        .unwrap_or_else(|| "等待预测".into())
}
impl Service {
    fn allowed(s: &State, path: &str) -> bool {
        crate::ai_model::scheduler::allowed(path, &s.priority, s.automatic)
    }
    fn update(&self, path: &str, entry: Entry) {
        self.state.lock().unwrap().entries.insert(path.into(), entry);
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
    fn checkpoint(&self, path: &str, last: &mut Instant) -> Result<()> {
        loop {
            let s = self.state.lock().unwrap();
            if !Self::allowed(&s, path) {
                bail!(INTERRUPTED);
            }
            // A newly selected AI chart preempts background work at the next chunk.
            if !s.priority.contains(path) && s.priority.iter().any(|p| !s.entries.get(p).is_some_and(|e| e.stamp.is_some())) {
                bail!(INTERRUPTED);
            }
            let idle = s.heartbeat.elapsed() < Duration::from_millis(300);
            drop(s);
            if idle {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            *last = Instant::now();
        }
        let work = last.elapsed();
        if work >= Duration::from_millis(2) {
            std::thread::sleep((work * 9).min(Duration::from_millis(100)));
            *last = Instant::now();
        }
        Ok(())
    }
    fn run(self: Arc<Self>, mut index: Index) {
        let mut predictor: Option<Predictor> = None;
        let mut staged = BTreeSet::new();
        loop {
            let paths = {
                let s = self.state.lock().unwrap();
                if s.heartbeat.elapsed() > Duration::from_millis(300) {
                    Vec::new()
                } else {
                    crate::ai_model::scheduler::order(&s.paths, &s.priority, s.automatic)
                }
            };
            let mut pending = Vec::new();
            let mut index_dirty = false;
            // Hydrate all reusable caches before running any expensive models.
            for path in paths {
                let Some(root) = chart_root(&self.root, &path) else {
                    continue;
                };
                let mut last = Instant::now();
                if self.checkpoint(&path, &mut last).is_err() {
                    continue;
                }
                if path.starts_with(':') && !staged.contains(&path) {
                    if let Err(error) = stage_builtin(&path, &root) {
                        self.update(
                            &path,
                            Entry {
                                stamp: None,
                                file: None,
                                value: None,
                                message: format!("内置谱面读取失败：{error}"),
                            },
                        );
                        continue;
                    }
                    staged.insert(path.clone());
                }
                if !root.exists() {
                    let mut s = self.state.lock().unwrap();
                    s.paths.remove(&path);
                    s.priority.remove(&path);
                    s.entries.remove(&path);
                    drop(s);
                    index_dirty |= index.records.remove(&path).is_some();
                    self.revision.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let source = match Source::probe(&root) {
                    Ok(source) => source,
                    Err(error) => {
                        index_dirty |= index.records.remove(&path).is_some();
                        self.update(
                            &path,
                            Entry {
                                stamp: None,
                                file: None,
                                value: None,
                                message: format!("无法读取谱面：{error}"),
                            },
                        );
                        continue;
                    }
                };
                if self
                    .state
                    .lock()
                    .unwrap()
                    .entries
                    .get(&path)
                    .is_some_and(|old| old.stamp.as_ref() == Some(&source))
                {
                    continue;
                }
                index_dirty |= index.records.remove(&path).is_some();
                self.update(
                    &path,
                    Entry {
                        stamp: None,
                        file: Some(source.file.clone()),
                        value: None,
                        message: "正在读取定数缓存".into(),
                    },
                );
                let cached =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Option<(Source, String, crate::ai_model::Prediction)>> {
                        let doc = Document::load(&root)?;
                        let Some(mut result) = doc.cached() else {
                            return Ok(None);
                        };
                        if doc.needs_migration() {
                            result = doc.save(&result)?;
                        }
                        Ok(Some((Source::probe(&root)?, doc.format, result)))
                    }));
                match cached {
                    Ok(Ok(Some((source, format, prediction)))) => {
                        if index.insert(&path, &root, &source, &format, prediction.clone()).is_ok() {
                            index_dirty = true;
                        }
                        self.update(
                            &path,
                            Entry {
                                file: Some(source.file.clone()),
                                stamp: Some(source),
                                value: Some(prediction.mean),
                                message: "已缓存 · 5 模型均值".into(),
                            },
                        );
                    }
                    Ok(Ok(None)) => pending.push((path, root)),
                    error => {
                        let message = match error {
                            Ok(Err(e)) => format!("无法预测：{e}"),
                            _ => "无法预测：谱面解析失败".into(),
                        };
                        self.update(
                            &path,
                            Entry {
                                file: Some(source.file.clone()),
                                stamp: Some(source),
                                value: None,
                                message,
                            },
                        );
                    }
                }
                if index_dirty {
                    if let Err(error) = index.save(&self.index_file) {
                        tracing::warn!("AI index save failed: {error}");
                    } else {
                        index_dirty = false;
                    }
                }
            }
            if index_dirty {
                let _ = index.save(&self.index_file);
            }
            for (path, root) in pending {
                let mut last = Instant::now();
                let operation =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(Source, String, crate::ai_model::Prediction)> {
                        self.checkpoint(&path, &mut last)?;
                        let mut doc = Document::load(&root)?;
                        // A copied cache may have appeared while this job was queued.
                        if let Some(mut prediction) = doc.cached() {
                            if doc.needs_migration() {
                                prediction = doc.save(&prediction)?;
                            }
                            return Ok((Source::probe(&root)?, doc.format, prediction));
                        }
                        self.update(
                            &path,
                            Entry {
                                stamp: None,
                                file: Some(doc.file.clone()),
                                value: None,
                                message: "正在预测 · 5 个模型".into(),
                            },
                        );
                        if predictor.is_none() {
                            predictor = Some(Predictor::new()?);
                        }
                        let mut check = || self.checkpoint(&path, &mut last);
                        let features = crate::ai_chart_adapter::extract(&doc, &mut check)?;
                        doc.release_input();
                        let prediction = predictor.as_ref().unwrap().predict(&features, doc.hash.clone(), &mut check)?;
                        drop(features);
                        check()?;
                        let prediction = doc.save(&prediction)?;
                        Ok((Source::probe(&root)?, doc.format, prediction))
                    }));
                match operation {
                    Ok(Ok((source, format, prediction))) => {
                        match index
                            .insert(&path, &root, &source, &format, prediction.clone())
                            .and_then(|_| index.save(&self.index_file))
                        {
                            Ok(()) => {}
                            Err(error) => tracing::warn!("AI index save failed: {error}"),
                        }
                        self.update(
                            &path,
                            Entry {
                                file: Some(source.file.clone()),
                                stamp: Some(source),
                                value: Some(prediction.mean),
                                message: "已缓存 · 5 模型均值".into(),
                            },
                        );
                    }
                    Ok(Err(e)) if e.to_string() == INTERRUPTED || e.to_string().starts_with("谱面已变更") => {
                        self.update(
                            &path,
                            Entry {
                                stamp: None,
                                file: None,
                                value: None,
                                message: "等待预测".into(),
                            },
                        );
                    }
                    error => {
                        let message = match error {
                            Ok(Err(e)) => format!("无法预测：{e}"),
                            _ => "无法预测：谱面解析失败".into(),
                        };
                        let source = Source::probe(&root).ok();
                        self.update(
                            &path,
                            Entry {
                                file: source.as_ref().map(|s| s.file.clone()),
                                stamp: source,
                                value: None,
                                message,
                            },
                        );
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

fn stage_builtin(path: &str, root: &Path) -> Result<()> {
    use crate::ai_model::cache::atomic_write;
    let (song, diff) = path
        .strip_prefix(':')
        .and_then(|p| p.split_once(':'))
        .ok_or_else(|| anyhow::anyhow!("内置谱面路径无效"))?;
    anyhow::ensure!(
        [song, diff]
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')),
        "内置谱面路径无效"
    );
    let asset = format!("res/song/{song}/{diff}");
    let (tx, rx) = std::sync::mpsc::channel();
    miniquad::fs::load_file(&asset, move |r| {
        let _ = tx.send(r);
    });
    let bytes = crate::resolve_res_data(rx.recv_timeout(Duration::from_secs(5))?.map_err(|e| anyhow::anyhow!("{e:?}"))?);
    std::fs::create_dir_all(root)?;
    let name = "ai-chart-source";
    let file = root.join(name);
    let size_file = root.join("ai-original-byte-count");
    let size = bytes.len().to_string();
    let equivalent = file.exists() && std::fs::read_to_string(&size_file).ok().as_deref() == Some(&size);
    if !equivalent {
        // Adopt a previously staged legacy file without hashing it. Its embedded
        // cache will be migrated by the normal cache-loading pass.
        if !file.exists() || size_file.exists() {
            atomic_write(&file, &bytes)?;
        }
        atomic_write(&size_file, size.as_bytes())?;
    }
    if !root.join("info.yml").exists() {
        let info = prpr::info::ChartInfo {
            chart: name.into(),
            name: song.into(),
            ..Default::default()
        };
        atomic_write(&root.join("info.yml"), serde_yaml::to_string(&info)?.as_bytes())?;
    }
    Ok(())
}
