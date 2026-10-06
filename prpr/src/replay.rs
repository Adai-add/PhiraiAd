//! Authoritative replay tape. Song time may rewind; tape time never does.
use crate::{
    config::Config,
    core::{Chart, Resource},
    fs::FileSystem,
    info::ChartInfo,
    judge::{Judge, JudgeStatus, Judgement},
    task::Task,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use macroquad::prelude::{Color, TouchPhase};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub const VERSION: u32 = 3;
fn supported_version(version: u32) -> bool {
    version == VERSION
}
static REVISION: AtomicU64 = AtomicU64::new(0);
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}
pub type Assets = Arc<Mutex<BTreeMap<String, Arc<Vec<u8>>>>>;
static ROOT: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));
static STORAGE: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static SAVES: Lazy<Mutex<Vec<Task<Result<String>>>>> = Lazy::new(|| Mutex::new(Vec::new()));
pub fn set_root(root: impl AsRef<Path>) {
    *ROOT.lock().unwrap() = Some(root.as_ref().join("replays-v3"));
}

/// Set the replay storage directory directly instead of appending the legacy
/// `replays-v3` subdirectory. Desktop frontends use this when replay files are
/// intentionally kept next to the executable.
pub fn set_root_path(root: impl AsRef<Path>) {
    *ROOT.lock().unwrap() = Some(root.as_ref().to_path_buf());
}
fn root() -> Result<PathBuf> {
    ROOT.lock().unwrap().clone().context("回放目录尚未初始化")
}
fn id_path(id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id)?;
    Ok(root()?.join(id))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn blob_path(hash: &str) -> Result<PathBuf> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid replay asset hash");
    }
    Ok(root()?.join("assets").join(hash))
}

pub struct CaptureFs {
    inner: Box<dyn FileSystem>,
    pub assets: Assets,
    enabled: bool,
}
impl CaptureFs {
    pub fn new(inner: Box<dyn FileSystem>) -> Self {
        Self {
            inner,
            assets: Arc::default(),
            enabled: true,
        }
    }
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}
#[async_trait]
impl FileSystem for CaptureFs {
    async fn load_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let bytes = self.inner.load_file(path).await?;
        if self.enabled {
            let mut assets = self.assets.lock().unwrap();
            assets.entry(path.to_owned()).or_insert_with(|| Arc::new(bytes.clone()));
        }
        Ok(bytes)
    }
    async fn exists(&mut self, path: &str) -> Result<bool> {
        self.inner.exists(path).await
    }
    fn list_root(&self) -> Result<Vec<String>> {
        self.inner.list_root()
    }
    fn clone_box(&self) -> Box<dyn FileSystem> {
        Box::new(Self {
            inner: self.inner.clone_box(),
            assets: self.assets.clone(),
            enabled: self.enabled,
        })
    }
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}
#[derive(Clone)]
pub struct SnapshotFs(pub BTreeMap<String, String>);
#[async_trait]
impl FileSystem for SnapshotFs {
    async fn load_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let hash = self.0.get(path).context("录像缺少此谱面资源")?.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = fs::read(blob_path(&hash)?)?;
            if digest(&bytes) != hash {
                bail!("录像资源损坏");
            }
            Ok(bytes)
        })
        .await?
    }
    async fn exists(&mut self, path: &str) -> Result<bool> {
        Ok(self.0.contains_key(path))
    }
    fn list_root(&self) -> Result<Vec<String>> {
        Ok(self.0.keys().cloned().collect())
    }
    fn clone_box(&self) -> Box<dyn FileSystem> {
        Box::new(self.clone())
    }
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HoldState {
    pub perfect: bool,
    pub at: f64,
    pub difference: f64,
    pub tail: bool,
    pub released_at: Option<f64>,
    pub safe_frame: i8,
    pub pending: bool,
}
/// Keep ordinary states compact; long charts contain far more Tap/Flick/Drag
/// snapshots than active Holds. Boxing avoids reserving 64 bytes for every Tap.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NoteState {
    Waiting,
    Armed,
    Finished,
    Hold(Box<HoldState>),
}
impl NoteState {
    pub fn capture(status: &JudgeStatus) -> Self {
        match *status {
            JudgeStatus::NotJudged => Self::Waiting,
            JudgeStatus::PreJudge => Self::Armed,
            JudgeStatus::Judged => Self::Finished,
            JudgeStatus::Hold(perfect, at, difference, tail, up, safe_frame, pending) => Self::Hold(Box::new(HoldState {
                perfect,
                at,
                difference,
                tail,
                released_at: up.is_finite().then_some(up),
                safe_frame,
                pending,
            })),
        }
    }
    fn matches_status(&self, status: &JudgeStatus) -> bool {
        match (self, status) {
            (Self::Waiting, JudgeStatus::NotJudged) | (Self::Armed, JudgeStatus::PreJudge) | (Self::Finished, JudgeStatus::Judged) => true,
            (Self::Hold(h), JudgeStatus::Hold(perfect, at, difference, tail, up, safe_frame, pending)) => {
                h.perfect == *perfect
                    && h.at == *at
                    && h.difference == *difference
                    && h.tail == *tail
                    && h.released_at == up.is_finite().then_some(*up)
                    && h.safe_frame == *safe_frame
                    && h.pending == *pending
            }
            _ => false,
        }
    }
    pub fn restore(&self) -> JudgeStatus {
        match self {
            Self::Waiting => JudgeStatus::NotJudged,
            Self::Armed => JudgeStatus::PreJudge,
            Self::Finished => JudgeStatus::Judged,
            Self::Hold(h) => {
                JudgeStatus::Hold(h.perfect, h.at, h.difference, h.tail, h.released_at.unwrap_or(f64::INFINITY), h.safe_frame, h.pending)
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteDelta {
    pub line: u32,
    pub note: u32,
    pub state: NoteState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub line: u32,
    pub note: u32,
    pub delivered: f64,
    pub hit_time: f64,
    pub difference: f64,
    pub result: Result<Judgement, bool>,
    pub custom_stage: Option<usize>,
    pub color: [f32; 4],
    pub center: [f32; 2],
    #[serde(default)]
    pub rendered_center: Option<[f32; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finger {
    pub id: u64,
    pub position: [f32; 2],
    pub phase: u8,
    pub event_time: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fx {
    pub position: [f32; 2],
    pub rotation: f32,
    pub color: [f32; 4],
    pub size_ratio: f32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Hud {
    pub combo: u32,
    pub score: u32,
    pub accuracy: f64,
    pub counts: [u32; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BadVisual {
    pub time: f64,
    pub kind: crate::core::NoteKind,
    pub matrix: [f32; 9],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub tape: f64,
    pub phase: u8,
    pub countdown: Option<(u8, f32)>,
    pub song: f64,
    pub chart: f64,
    pub alpha: f32,
    pub line_color: [f32; 4],
    pub hud: Hud,
    pub profile: crate::judge::JudgementRangeProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bad_visuals: Vec<BadVisual>,
    pub viewport_width: f32,
    pub speed: f32,
    pub flow: f32,
    pub judgement_multiplier: f32,
    pub view: [f32; 3],
    pub preserve_pitch: bool,
    pub angle: f32,
    pub flip_y: bool,
    pub rewind: bool,
    pub rotating: bool,
    pub audible: bool,
    pub checkpoint: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<Outcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx: Vec<Fx>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<crate::judge::HitSound>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fingers: Vec<Finger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<Finger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hovers: Vec<(Option<u64>, [f32; 2], f32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub noise_positions: Vec<[f32; 2]>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub created: String,
    pub duration: f64,
    pub info: ChartInfo,
    pub config: Config,
    pub practice: bool,
    pub range: [f64; 2],
    pub recorded_range: [f64; 2],
    pub chart_hash: String,
    pub chart_id: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, String>,
    pub respack: Option<String>,
    pub tape_hash: String,
    pub note_counts: Vec<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tape {
    pub version: u32,
    pub frames: Vec<Frame>,
}
#[derive(Clone)]
pub struct Payload {
    pub manifest: Manifest,
    pub tape: Tape,
    pub assets: Assets,
    pub pack_assets: Assets,
}
/// Drain a practice recording exactly once, before a reset can discard it.
/// A pending arm without a recording is preserved for the next resume.
/// Audio acknowledgements arrive asynchronously. Track commands already requested,
/// so silent countdown/paused frames never fill a bounded music command queue.
pub(crate) fn audio_transition(requested_playing: bool, audible: bool) -> Option<bool> {
    (requested_playing != audible).then_some(audible)
}
pub(crate) fn start_practice_recording<T>(enabled: bool, practice: bool, armed: bool, recording: &mut Option<T>, create: impl FnOnce() -> T) -> bool {
    if !enabled || !practice || !armed || recording.is_some() {
        return false;
    }
    *recording = Some(create());
    true
}
pub(crate) fn take_practice_recording<T>(practice: bool, armed: &mut bool, recording: &mut Option<T>) -> Option<T> {
    if !practice {
        return None;
    }
    let recording = recording.take()?;
    *armed = false;
    Some(recording)
}

pub(crate) fn practice_range_finished(song: f64, end: f64, track: f64, paused: bool) -> bool {
    !paused && song.is_finite() && end.is_finite() && track.is_finite() && song >= end.min(track)
}

pub struct Recorder {
    pub assets: Assets,
    pub pack_assets: Assets,
    pub config: Config,
    pub info: ChartInfo,
    pub practice: bool,
    pub range: [f64; 2],
    pub frames: Vec<Frame>,
    previous: Vec<Vec<NoteState>>,
    last_real: Option<f64>,
    tape: f64,
    last_checkpoint: f64,
    pub pending: Vec<Outcome>,
    contacts: BTreeMap<u64, Finger>,
    pub paused: bool,
}
pub fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}
pub fn color(c: [f32; 4]) -> Color {
    Color::new(c[0], c[1], c[2], c[3])
}
impl Recorder {
    pub fn new(res: &Resource, assets: Assets, practice: bool, range: [f64; 2]) -> Self {
        let mut config = res.config.clone();
        config.aspect_ratio = Some(res.aspect_ratio);
        Self {
            assets,
            pack_assets: res.replay_pack_assets.clone(),
            config,
            info: res.info.clone(),
            practice,
            range,
            frames: Vec::new(),
            previous: Vec::new(),
            last_real: None,
            tape: 0.,
            last_checkpoint: -3.,
            pending: Vec::new(),
            contacts: BTreeMap::new(),
            paused: false,
        }
    }
    pub fn reset_clock(&mut self) {
        self.last_real = None;
        self.contacts.clear();
    }
    pub fn outcome(&mut self, event: &crate::judge::JudgeReportEvent, chart: &Chart, res: &Resource) {
        let Some(line) = chart.lines.get(event.line_id as usize) else {
            return;
        };
        let Some(note) = line.notes.get(event.note_id as usize) else {
            return;
        };
        let miss = event.judgement == Ok(Judgement::Miss);
        let diff = if miss {
            (event.time - note.time) / res.config.speed as f64
        } else {
            event.difference
        };
        let stage = (res.config.judgement_mode == crate::config::JudgementMode::Custom
            && !matches!(note.kind, crate::core::NoteKind::Drag | crate::core::NoteKind::Flick))
        .then(|| res.config.effective_custom_judgement().classify(-diff * 1000.))
        .flatten();
        let fallback = match event.judgement {
            Ok(Judgement::Perfect) | Err(true) => res.res_pack.info.fx_perfect(),
            Ok(Judgement::Good) | Err(false) => res.res_pack.info.fx_good(),
            _ => macroquad::prelude::RED,
        };
        let fx_color = stage
            .map(|i| {
                let b = res.config.effective_custom_judgement().bands[i].color;
                Color::from_rgba(b[0], b[1], b[2], b[3])
            })
            .unwrap_or(note.fx_color.unwrap_or(fallback));
        let matrix = line.now_transform(res, &chart.lines) * note.object.now(res);
        let p = matrix.transform_point(&crate::core::Point::default());
        self.pending.push(Outcome {
            line: event.line_id,
            note: event.note_id,
            delivered: event.time,
            hit_time: if miss {
                event.time
            } else {
                note.time + event.difference * res.config.speed as f64
            },
            difference: diff,
            result: event.judgement,
            custom_stage: stage,
            color: rgba(fx_color),
            center: [p.x, -p.y],
            rendered_center: None,
        });
    }
    pub fn frame(
        &mut self,
        res: &mut Resource,
        chart: &Chart,
        judge: &Judge,
        real: f64,
        song: f64,
        angle: f32,
        rewind: bool,
        rotating: bool,
        audible: bool,
        bad_notes: &[crate::core::BadNote],
        phase: u8,
        countdown: Option<(u8, f32)>,
    ) {
        if self.frames.is_empty() {
            self.config.aspect_ratio = Some(res.aspect_ratio);
            self.info.force_aspect_ratio = true;
        }
        if let Some(last) = self.last_real {
            self.tape += (real - last).max(0.);
        }
        self.last_real = Some(real);
        let checkpoint = self.previous.is_empty() || self.tape - self.last_checkpoint >= 2.;
        if self.previous.is_empty() {
            self.previous = chart.lines.iter().map(|l| vec![NoteState::Waiting; l.notes.len()]).collect();
        }
        let mut notes = Vec::new();
        for (i, line) in chart.lines.iter().enumerate() {
            for (j, note) in line.notes.iter().enumerate() {
                if checkpoint || !self.previous[i][j].matches_status(&note.judge) {
                    let state = NoteState::capture(&note.judge);
                    notes.push(NoteDelta {
                        line: i as u32,
                        note: j as u32,
                        state: state.clone(),
                    });
                    self.previous[i][j] = state;
                }
            }
        }
        if checkpoint {
            self.last_checkpoint = self.tape;
        }
        let fingers: Vec<_> = Judge::replay_touches(res.camera.viewport.unwrap_or(res.last_vp))
            .iter()
            .map(|t| Finger {
                id: t.id,
                event_time: t.time.is_finite().then_some(t.time),
                position: [t.position.x, t.position.y],
                phase: match t.phase {
                    TouchPhase::Started => 0,
                    TouchPhase::Moved => 1,
                    TouchPhase::Stationary => 2,
                    TouchPhase::Ended => 3,
                    TouchPhase::Cancelled => 4,
                },
            })
            .collect();
        for f in &fingers {
            if f.phase >= 3 {
                self.contacts.remove(&f.id);
            } else {
                self.contacts.insert(f.id, f.clone());
            }
        }
        for event in &mut self.pending {
            event.rendered_center = res.replay_note_centers.get(&(event.line, event.note)).copied();
        }
        self.frames.push(Frame {
            tape: self.tape,
            phase,
            countdown,
            song,
            chart: res.time,
            alpha: res.alpha,
            line_color: rgba(res.judge_line_color),
            hud: Hud {
                combo: judge.combo(),
                score: judge.score(),
                accuracy: judge.real_time_accuracy(),
                counts: judge.counts(),
            },
            profile: judge.judgement_range_profile(&res.config),
            bad_visuals: bad_notes
                .iter()
                .map(|n| BadVisual {
                    time: n.time,
                    kind: n.kind.clone(),
                    matrix: n.matrix.as_slice().try_into().unwrap(),
                })
                .collect(),
            viewport_width: res.camera.viewport.map_or(1., |vp| vp.2 as f32),
            speed: res.config.speed,
            flow: res.note_flow_speed,
            judgement_multiplier: res.config.practice_judgement_multiplier,
            view: [res.practice_view.scale_percent, res.practice_view.center_x, res.practice_view.center_y],
            preserve_pitch: res.config.preserve_pitch_for(self.practice),
            angle,
            flip_y: res.auto_flip_y,
            rewind,
            rotating,
            audible,
            checkpoint,
            notes,
            outcomes: std::mem::take(&mut self.pending),
            fx: std::mem::take(&mut res.replay_fx),
            sounds: std::mem::take(&mut res.replay_sounds),
            fingers,
            contacts: self.contacts.values().cloned().collect(),
            blocked: judge.noise_state.blocked_ids.iter().copied().collect(),
            hovers: judge
                .noise_state
                .hovers
                .iter()
                .map(|h| (h.finger, [h.position.x, h.position.y], h.scale))
                .collect(),
            noise_positions: judge.noise_state.positions.iter().map(|p| [p.x, p.y]).collect(),
        });
    }
    pub fn payload(&self) -> Result<Payload> {
        self.payload_with_frames(self.frames.clone())
    }
    pub fn into_payload(mut self) -> Result<Payload> {
        let frames = std::mem::take(&mut self.frames);
        self.payload_with_frames(frames)
    }
    fn payload_with_frames(&self, frames: Vec<Frame>) -> Result<Payload> {
        if frames.is_empty() {
            bail!("本次还没有可保存的回放");
        }
        let created = chrono::Local::now();
        let chart_bytes = self.assets.lock().unwrap().get(&self.info.chart).cloned().context("无法保存谱面快照")?;
        Ok(Payload {
            manifest: Manifest {
                version: VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{}-{}-{:.1}-{}", self.info.name, self.info.level, self.info.difficulty, created.format("%Y%m%d-%H%M%S")),
                created: created.to_rfc3339(),
                duration: frames.last().unwrap().tape,
                info: self.info.clone(),
                config: self.config.clone(),
                practice: self.practice,
                range: self.range,
                recorded_range: [
                    frames.iter().map(|f| f.song.max(0.)).fold(f64::INFINITY, f64::min),
                    frames.iter().map(|f| f.song.min(self.range[1]).max(0.)).fold(0., f64::max),
                ],
                chart_hash: digest(&chart_bytes),
                chart_id: 0,
                assets: BTreeMap::new(),
                respack: None,
                tape_hash: String::new(),
                note_counts: self.previous.iter().map(Vec::len).collect(),
            },
            tape: Tape { version: VERSION, frames },
            assets: self.assets.clone(),
            pack_assets: self.pack_assets.clone(),
        })
    }
}
fn zip_tape(tape: &Tape) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "tape.json",
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Zstd)
            .compression_level(Some(6)),
    )?;
    serde_json::to_writer(&mut zip, tape)?;
    Ok(zip.finish()?.into_inner())
}
fn put_blob(bytes: &[u8]) -> Result<String> {
    let hash = digest(bytes);
    let path = blob_path(&hash)?;
    if path.exists() && fs::read(&path).is_ok_and(|stored| digest(&stored) == hash) {
        return Ok(hash);
    }
    fs::create_dir_all(path.parent().unwrap())?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(&path)?;
    Ok(hash)
}
/// Numbered immutable chart snapshots. Replay manifests reference chart_id;
/// hydrated manifests carry the snapshot's assets while the viewer is loading.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChartRecord {
    pub id: u64,
    pub created: String,
    pub hash: String,
    pub info: ChartInfo,
    pub assets: BTreeMap<String, String>,
}
impl ChartRecord {
    pub fn display_name(&self) -> String {
        format!("{} · #{} · {}", self.info.name, self.id, self.created.get(..10).unwrap_or(&self.created))
    }
}
#[derive(Serialize, Deserialize)]
struct ChartCatalog {
    next_id: u64,
    records: BTreeMap<u64, ChartRecord>,
}
impl Default for ChartCatalog {
    fn default() -> Self {
        Self {
            next_id: 1,
            records: BTreeMap::new(),
        }
    }
}
fn catalog_at(base: &Path) -> Result<ChartCatalog> {
    let bytes = match fs::read(base.join("charts.json")) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(ChartCatalog::default()),
        Err(err) => return Err(err.into()),
    };
    let catalog: ChartCatalog = serde_json::from_slice(&bytes).context("谱面记录索引损坏")?;
    if catalog.next_id == 0
        || catalog
            .records
            .iter()
            .any(|(id, r)| *id == 0 || *id != r.id || *id >= catalog.next_id || r.assets.get(&r.info.chart) != Some(&r.hash))
    {
        bail!("谱面记录编号或快照索引无效");
    }
    Ok(catalog)
}
fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("缺少存储目录")?)?;
    serde_json::to_writer(&mut temp, value)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
fn write_catalog(base: &Path, catalog: &ChartCatalog) -> Result<()> {
    atomic_json(&base.join("charts.json"), catalog)
}
pub fn charts() -> Result<Vec<ChartRecord>> {
    let _guard = STORAGE.lock().unwrap();
    let mut entries: Vec<_> = catalog_at(&root()?)?.records.into_values().collect();
    entries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(entries)
}
fn read_entries(base: &Path) -> Result<Vec<Manifest>> {
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(entry.path().join("manifest.json"))?).context("录像索引损坏，停止清理以保护资源")?;
        if manifest.id != id || manifest.version != VERSION {
            bail!("录像索引无效");
        }
        entries.push(manifest);
    }
    entries.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(entries)
}
pub fn save(mut payload: Payload) -> Result<String> {
    validate(&payload.tape, &payload.manifest.note_counts)?;
    if payload.manifest.version != VERSION {
        bail!("不支持旧录像保存格式");
    }
    let _guard = STORAGE.lock().unwrap();
    let base = root()?;
    fs::create_dir_all(&base)?;
    let destination = id_path(&payload.manifest.id)?;
    if destination.exists() {
        bail!("录像编号已存在");
    }
    let mut catalog = catalog_at(&base)?;
    let assets = payload.assets.lock().unwrap();
    let chart_bytes = assets.get(&payload.manifest.info.chart).context("无法保存谱面快照")?;
    let hash = digest(chart_bytes);
    if hash != payload.manifest.chart_hash {
        bail!("录制谱面与保存快照不一致");
    }
    let existing = catalog.records.values().find(|r| r.hash == hash).map(|r| r.id);
    let chart_id = if let Some(id) = existing {
        // Reuse the first immutable snapshot, repairing missing/corrupt blobs only
        // when the captured bytes have exactly the same content hash.
        for (path, expected) in &catalog.records[&id].assets {
            let valid = fs::read(blob_path(expected)?).is_ok_and(|b| digest(&b) == *expected);
            if !valid {
                let bytes = assets
                    .get(path)
                    .filter(|b| digest(b) == *expected)
                    .context("已有谱面快照资源损坏，无法复用")?;
                put_blob(bytes)?;
            }
        }
        id
    } else {
        let id = catalog.next_id;
        catalog.next_id = id.checked_add(1).context("谱面编号已用尽")?;
        let mut snapshot = BTreeMap::new();
        for (path, bytes) in assets.iter() {
            snapshot.insert(path.clone(), put_blob(bytes)?);
        }
        catalog.records.insert(
            id,
            ChartRecord {
                id,
                created: payload.manifest.created.clone(),
                hash: hash.clone(),
                info: payload.manifest.info.clone(),
                assets: snapshot,
            },
        );
        id
    };
    // Keep per-recording resource changes as sparse overrides. A chart-only
    // SHA match must not silently replace newly recorded music/illustrations
    // with the first recording's media; blobs are still content-deduplicated.
    payload.manifest.assets.clear();
    let record = &catalog.records[&chart_id];
    for (path, bytes) in assets.iter() {
        let hash = digest(bytes);
        if record.assets.get(path) != Some(&hash) {
            payload.manifest.assets.insert(path.clone(), put_blob(bytes)?);
        }
    }
    drop(assets);
    payload.manifest.chart_id = chart_id;
    payload.manifest.chart_hash = hash;
    let pack = payload.pack_assets.lock().unwrap();
    if !pack.is_empty() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in pack.iter() {
            zip.start_file(path, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated))?;
            zip.write_all(bytes)?;
        }
        payload.manifest.respack = Some(put_blob(&zip.finish()?.into_inner())?);
    }
    let bytes = zip_tape(&payload.tape)?;
    payload.manifest.tape_hash = digest(&bytes);
    let temp = tempfile::tempdir_in(&base)?;
    let mut tape_file = fs::File::create(temp.path().join("tape.zip"))?;
    tape_file.write_all(&bytes)?;
    tape_file.sync_all()?;
    // Windows refuses to rename a directory while a file inside it still has
    // an open handle. Close tape.zip before publishing the temporary replay
    // directory, otherwise fs::rename below fails with ERROR_ACCESS_DENIED (5).
    drop(tape_file);
    atomic_json(&temp.path().join("manifest.json"), &payload.manifest)?;
    // Publish the chart before the referencing replay. An interrupted save can
    // leave an unused chart, never a replay pointing to an unpublished chart.
    write_catalog(&base, &catalog)?;
    if let Err(err) = fs::rename(temp.path(), destination) {
        if existing.is_none() {
            catalog.records.remove(&chart_id);
            // Keep the consumed number; deleted/failed record IDs are never reused.
            let _ = write_catalog(&base, &catalog);
        }
        return Err(err.into());
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(payload.manifest.name)
}
pub fn save_background(payload: Payload) {
    SAVES
        .lock()
        .unwrap()
        .push(Task::new(async move { tokio::task::spawn_blocking(move || save(payload)).await? }));
    crate::scene::show_message("正在保存回放…");
}
pub fn poll_saves() {
    let mut tasks = SAVES.lock().unwrap();
    tasks.retain_mut(|task| match task.take() {
        None => true,
        Some(Ok(name)) => {
            crate::scene::show_message(format!("回放已保存：{name}")).duration(3.).ok();
            false
        }
        Some(Err(err)) => {
            crate::scene::show_message(format!("回放保存失败：{err:#}")).duration(5.).error();
            false
        }
    });
}
pub fn list() -> Result<Vec<Manifest>> {
    let _guard = STORAGE.lock().unwrap();
    read_entries(&root()?)
}
#[derive(Default)]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub per_replay: BTreeMap<String, u64>,
    pub per_chart: BTreeMap<u64, u64>,
}
pub struct LibrarySnapshot {
    pub entries: Vec<Manifest>,
    pub charts: Vec<ChartRecord>,
    pub usage: StorageUsage,
    pub revision: u64,
}
/// One storage lock keeps tab contents, reference counts and sizes consistent
/// even when a background recording finishes saving during refresh.
pub fn library_snapshot() -> Result<LibrarySnapshot> {
    let _guard = STORAGE.lock().unwrap();
    let base = root()?;
    let entries = read_entries(&base)?;
    let mut charts: Vec<_> = catalog_at(&base)?.records.into_values().collect();
    charts.sort_by(|a, b| b.id.cmp(&a.id));
    let usage = storage_usage_at(&base, &entries)?;
    Ok(LibrarySnapshot {
        entries,
        charts,
        usage,
        revision: revision(),
    })
}
fn directory_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let bytes = if kind.is_dir() {
            directory_bytes(&entry.path())?
        } else if kind.is_file() {
            entry.metadata()?.len()
        } else {
            0
        };
        total = total.saturating_add(bytes);
    }
    Ok(total)
}
pub fn storage_usage(entries: &[Manifest]) -> Result<StorageUsage> {
    let _guard = STORAGE.lock().unwrap();
    storage_usage_at(&root()?, entries)
}
fn storage_usage_at(base: &Path, entries: &[Manifest]) -> Result<StorageUsage> {
    let mut usage = StorageUsage {
        total_bytes: directory_bytes(base)?,
        ..Default::default()
    };

    for entry in entries {
        if uuid::Uuid::parse_str(&entry.id).is_err() {
            continue;
        }
        let mut bytes = directory_bytes(&base.join(&entry.id))?;
        let hashes: std::collections::BTreeSet<_> = entry.assets.values().chain(entry.respack.iter()).collect();
        for hash in hashes {
            let size = fs::metadata(base.join("assets").join(hash)).map(|m| m.len()).unwrap_or(0);
            bytes = bytes.saturating_add(size);
        }
        usage.per_replay.insert(entry.id.clone(), bytes);
    }
    for record in catalog_at(base)?.records.values() {
        let mut bytes = serde_json::to_vec(record)?.len() as u64;
        let hashes: std::collections::BTreeSet<_> = record.assets.values().collect();
        for hash in hashes {
            bytes = bytes.saturating_add(fs::metadata(base.join("assets").join(hash)).map(|m| m.len()).unwrap_or(0));
        }
        usage.per_chart.insert(record.id, bytes);
    }
    Ok(usage)
}
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024. * 1024.))
    } else {
        format!("{:.2} GiB", bytes as f64 / (1024. * 1024. * 1024.))
    }
}
pub fn rename(id: &str, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 160 {
        bail!("名称须为 1–160 个字符");
    }
    let _guard = STORAGE.lock().unwrap();
    let path = id_path(id)?.join("manifest.json");
    let mut entry: Manifest = serde_json::from_slice(&fs::read(&path)?)?;
    entry.name = name.to_owned();
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.write_all(&serde_json::to_vec(&entry)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
pub fn delete(ids: &[String]) -> Result<()> {
    let _guard = STORAGE.lock().unwrap();
    delete_locked(ids, &[])
}
pub fn delete_charts(ids: &[u64]) -> Result<()> {
    let _guard = STORAGE.lock().unwrap();
    let ids: std::collections::BTreeSet<_> = ids.iter().copied().collect();
    let replays = read_entries(&root()?)?
        .into_iter()
        .filter(|r| ids.contains(&r.chart_id))
        .map(|r| r.id)
        .collect::<Vec<_>>();
    delete_locked(&replays, &ids.into_iter().collect::<Vec<_>>())
}
fn delete_locked(ids: &[String], charts: &[u64]) -> Result<()> {
    let base = root()?;
    fs::create_dir_all(&base)?;
    let entries = read_entries(&base)?;
    let mut catalog = catalog_at(&base)?;
    if entries
        .iter()
        .any(|r| catalog.records.get(&r.chart_id).is_none_or(|c| c.hash != r.chart_hash))
    {
        bail!("录像与谱面记录索引不一致，停止删除以保护资源");
    }
    let wanted: std::collections::BTreeSet<_> = ids.iter().collect();
    // Validate all IDs before moving any directory.
    let paths: Vec<_> = ids.iter().map(|id| id_path(id)).collect::<Result<_>>()?;
    let mut moved = Vec::new();
    for path in paths {
        if path.exists() {
            let trash = base.join(format!(".trash-{}", uuid::Uuid::new_v4()));
            if let Err(err) = fs::rename(&path, &trash) {
                for (original, staged) in moved.iter().rev() {
                    let _ = fs::rename(staged, original);
                }
                return Err(err.into());
            }
            moved.push((path, trash));
        }
    }
    let remaining: Vec<_> = entries.into_iter().filter(|r| !wanted.contains(&r.id)).collect();
    let referenced: std::collections::BTreeSet<_> = remaining.iter().map(|r| r.chart_id).collect();
    catalog.records.retain(|id, _| referenced.contains(id) && !charts.contains(id));
    if let Err(err) = write_catalog(&base, &catalog) {
        for (original, staged) in moved.iter().rev() {
            let _ = fs::rename(staged, original);
        }
        return Err(err);
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    for (_, trash) in moved {
        if let Err(err) = fs::remove_dir_all(trash) {
            tracing::warn!("replay trash cleanup: {err}");
        }
    }
    // Protect any recoverable staged directory if cleanup failed/interrupted.
    if fs::read_dir(&base)?.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(".trash-"))) {
        return Ok(());
    }
    let mut retained = std::collections::BTreeSet::new();
    for record in catalog.records.values() {
        retained.extend(record.assets.values().cloned());
    }
    for replay in remaining {
        retained.extend(replay.assets.into_values());
        retained.extend(replay.respack);
    }
    if let Ok(entries) = fs::read_dir(base.join("assets")) {
        for entry in entries.flatten() {
            let hash = entry.file_name().to_string_lossy().into_owned();
            if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !retained.contains(&hash) {
                if let Err(err) = fs::remove_file(entry.path()) {
                    tracing::warn!("replay asset cleanup: {err}");
                }
            }
        }
    }
    Ok(())
}
pub fn load(id: &str) -> Result<(Manifest, Tape)> {
    let _guard = STORAGE.lock().unwrap();
    let path = id_path(id)?;
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(path.join("manifest.json"))?)?;
    if !supported_version(manifest.version) {
        bail!("不支持此回放版本");
    }
    let catalog = catalog_at(&root()?)?;
    let record = catalog.records.get(&manifest.chart_id).context("录像对应的谱面记录不存在")?;
    if record.hash != manifest.chart_hash {
        bail!("录像与谱面记录不匹配");
    }
    let overrides = std::mem::take(&mut manifest.assets);
    manifest.assets = record.assets.clone();
    manifest.assets.extend(overrides);
    let bytes = fs::read(path.join("tape.zip"))?;
    if digest(&bytes) != manifest.tape_hash {
        bail!("回放数据损坏");
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes))?;
    let mut entry = zip.by_name("tape.json")?;
    if entry.size() > 512 * 1024 * 1024 {
        bail!("回放数据过大");
    }
    let tape: Tape = serde_json::from_reader((&mut entry).take(512 * 1024 * 1024 + 1))?;
    if manifest.assets.get(&manifest.info.chart) != Some(&manifest.chart_hash) {
        bail!("回放谱面校验失败");
    }
    validate(&tape, &manifest.note_counts)?;
    if let Some(hash) = &manifest.respack {
        let path = blob_path(hash)?;
        if digest(&fs::read(&path)?) != *hash {
            bail!("回放资源包损坏");
        }
        manifest.config.res_pack_path = Some(path.to_string_lossy().into_owned());
    }
    manifest.config.challenge_mode = false;
    manifest.config.interactive = false;
    manifest.config.auto_export_play_report = false;
    manifest.config.touch_input_debug_report = false;
    Ok((manifest, tape))
}
pub fn validate(tape: &Tape, counts: &[usize]) -> Result<()> {
    if !supported_version(tape.version) || tape.frames.is_empty() || !tape.frames[0].checkpoint {
        bail!("回放数据不完整");
    }
    let mut previous = -1.;
    for frame in &tape.frames {
        if !frame.tape.is_finite()
            || frame.tape < previous
            || !frame.song.is_finite()
            || !frame.chart.is_finite()
            || !frame.speed.is_finite()
            || frame.speed <= 0.
        {
            bail!("Invalid replay clock");
        }
        if !frame.flow.is_finite()
            || !frame.judgement_multiplier.is_finite()
            || frame.judgement_multiplier <= 0.
            || !frame.angle.is_finite()
            || !frame.view.iter().all(|v| v.is_finite())
            || !frame.hud.accuracy.is_finite()
        {
            bail!("Invalid replay settings");
        }
        previous = frame.tape;
        let mut covered = std::collections::BTreeSet::new();
        for note in &frame.notes {
            if counts.get(note.line as usize).is_none_or(|n| note.note as usize >= *n) {
                bail!("Invalid replay note index");
            }
            if !covered.insert((note.line, note.note)) {
                bail!("Duplicate replay note state");
            }
        }
        if frame.checkpoint && covered.len() != counts.iter().sum::<usize>() {
            bail!("Incomplete replay checkpoint");
        }
        for event in &frame.outcomes {
            if counts.get(event.line as usize).is_none_or(|n| event.note as usize >= *n)
                || !event.difference.is_finite()
                || !event.hit_time.is_finite()
            {
                bail!("Invalid replay outcome");
            }
        }
    }
    Ok(())
}
/// Return checkpoint and forward range. Restoring by song time loses pause rewinds.
pub fn seek_range(tape: &Tape, position: f64) -> (usize, usize) {
    let end = tape.frames.partition_point(|f| f.tape <= position).saturating_sub(1);
    let begin = tape.frames[..=end].iter().rposition(|f| f.checkpoint).unwrap_or(0);
    (begin, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn practice_restart_and_continue_both_start_one_shot_recording() {
        for restart in [false, true] {
            let mut armed = true;
            let mut recording: Option<Vec<f64>> = None;
            if restart {
                // Reset must preserve an armed request if there is no active tape.
                assert!(take_practice_recording(true, &mut armed, &mut recording).is_none());
                recording = None;
            }
            assert!(start_practice_recording(true, true, armed, &mut recording, Vec::new));
            assert!(!start_practice_recording(true, true, armed, &mut recording, || panic!("must not replace active tape")));
            recording.as_mut().unwrap().extend([-3., -2., -1., 0., 0.5]);
            assert_eq!(take_practice_recording(true, &mut armed, &mut recording).unwrap(), [-3., -2., -1., 0., 0.5]);
            assert!(!armed);
            assert!(!start_practice_recording(true, true, armed, &mut recording, Vec::new));
        }
    }
    #[test]
    fn disabled_recording_never_creates_practice_tape() {
        let mut recording: Option<()> = None;
        assert!(!start_practice_recording(false, true, true, &mut recording, || panic!("disabled")));
        assert!(recording.is_none());
    }
    #[test]
    fn recording_switch_defaults_on_for_old_settings() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.replay_recording_enabled);
        let config: Config = serde_json::from_str(r#"{"replayRecordingEnabled":false}"#).unwrap();
        assert!(!config.replay_recording_enabled);
        let restored: Config = serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap();
        assert!(!restored.replay_recording_enabled);
    }
    #[test]
    fn borrowed_hold_comparison_tracks_all_state_changes() {
        let status = JudgeStatus::Hold(true, 1., -0.123456789, false, f64::INFINITY, 4, false);
        let state = NoteState::capture(&status);
        assert!(state.matches_status(&status));
        assert!(state.matches_status(&state.restore()));
        assert!(!state.matches_status(&JudgeStatus::Hold(true, 1., -0.123456788, false, f64::INFINITY, 4, false)));
        assert!(!state.matches_status(&JudgeStatus::Hold(true, 1., -0.123456789, false, 2., 4, false)));
        assert!(!state.matches_status(&JudgeStatus::Judged));
    }
    #[test]
    fn compressed_tape_round_trip_with_empty_arrays() {
        let mut tape = tape();
        tape.frames[1].fingers.push(Finger {
            id: 123,
            position: [0.1234567, -0.2345678],
            phase: 1,
            event_time: Some(1.123456789),
        });
        for method in [CompressionMethod::Zstd, CompressionMethod::Deflated] {
            let bytes = if method == CompressionMethod::Zstd {
                zip_tape(&tape).unwrap()
            } else {
                let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
                zip.start_file("tape.json", SimpleFileOptions::default().compression_method(method))
                    .unwrap();
                let mut value = serde_json::to_value(&tape).unwrap();
                value["version"] = VERSION.into();
                // Old writers explicitly serialized empty arrays.
                for frame in value["frames"].as_array_mut().unwrap() {
                    for field in [
                        "notes",
                        "outcomes",
                        "fingers",
                        "fx",
                        "sounds",
                        "contacts",
                        "blocked",
                        "hovers",
                        "noise_positions",
                        "bad_visuals",
                    ] {
                        frame.as_object_mut().unwrap().entry(field).or_insert(serde_json::json!([]));
                    }
                }
                serde_json::to_writer(&mut zip, &value).unwrap();
                zip.finish().unwrap().into_inner()
            };
            let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
            let restored: Tape = serde_json::from_reader(archive.by_name("tape.json").unwrap()).unwrap();
            validate(&restored, &[2]).unwrap();
            assert_eq!(serde_json::to_value(&restored.frames).unwrap(), serde_json::to_value(&tape.frames).unwrap());
        }
    }
    #[test]
    fn storage_sizes_skip_links_and_count_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("one"), [0; 17]).unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("nested/two"), [0; 29]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path(), temp.path().join("loop")).unwrap();
        assert_eq!(directory_bytes(temp.path()).unwrap(), 46);
        assert_eq!(format_size(1024), "1.0 KiB");
    }
    #[test]
    fn disabled_capture_does_not_retain_loaded_resources() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("chart.json"), b"chart").unwrap();
            let mut capture = CaptureFs::new(crate::fs::fs_from_file(temp.path()).unwrap()).with_enabled(false);
            assert_eq!(capture.load_file("chart.json").await.unwrap(), b"chart");
            assert!(capture.assets.lock().unwrap().is_empty());
            let mut cloned = capture.clone_box();
            assert_eq!(cloned.load_file("chart.json").await.unwrap(), b"chart");
            assert!(capture.assets.lock().unwrap().is_empty());
        });
    }
    #[test]
    fn library_total_deduplicates_shared_resources() {
        let temp = tempfile::tempdir().unwrap();
        let hash = "a".repeat(64);
        fs::create_dir(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets").join(&hash), [0; 100]).unwrap();
        let make_entry = || Manifest {
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            created: String::new(),
            duration: 1.,
            info: ChartInfo::default(),
            config: Config::default(),
            practice: false,
            range: [0., 1.],
            recorded_range: [0., 1.],
            chart_hash: hash.clone(),
            chart_id: 0,
            assets: BTreeMap::from([("chart".into(), hash.clone()), ("duplicate".into(), hash.clone())]),
            respack: Some(hash.clone()),
            tape_hash: String::new(),
            note_counts: vec![],
        };
        let entries = vec![make_entry(), make_entry()];
        for entry in &entries {
            fs::create_dir(temp.path().join(&entry.id)).unwrap();
            fs::write(temp.path().join(&entry.id).join("tape.zip"), [0; 20]).unwrap();
        }
        let usage = storage_usage_at(temp.path(), &entries).unwrap();
        assert_eq!(usage.total_bytes, 140);
        for entry in &entries {
            assert_eq!(usage.per_replay[&entry.id], 120);
        }
    }
    #[test]
    fn representative_compression_comparison() {
        let mut tape = tape();
        tape.frames = (0..7200)
            .map(|i| {
                let t = i as f64 / 60.;
                let mut f = frame(t, t, i % 120 == 0, vec![]);
                f.hud.combo = i / 30;
                f.hud.score = i * 130;
                if i % 60 < 20 {
                    f.fingers.push(Finger {
                        id: 2,
                        position: [(t as f32).sin(), (t as f32).cos()],
                        phase: 1,
                        event_time: Some(t),
                    });
                }
                f
            })
            .collect();
        let mut legacy = serde_json::to_value(&tape).unwrap();
        for frame in legacy["frames"].as_array_mut().unwrap() {
            for field in [
                "notes",
                "outcomes",
                "fingers",
                "fx",
                "sounds",
                "contacts",
                "blocked",
                "hovers",
                "noise_positions",
                "bad_visuals",
            ] {
                frame.as_object_mut().unwrap().entry(field).or_insert(serde_json::json!([]));
            }
        }
        let mut old = ZipWriter::new(Cursor::new(Vec::new()));
        old.start_file("tape.json", SimpleFileOptions::default().compression_method(CompressionMethod::Deflated))
            .unwrap();
        serde_json::to_writer(&mut old, &legacy).unwrap();
        let old_bytes = old.finish().unwrap().into_inner().len();
        let new_bytes = zip_tape(&tape).unwrap().len();
        println!("synthetic 7200 frames: legacy {old_bytes} bytes, new {new_bytes} bytes");
        assert!(new_bytes < old_bytes);
    }
    fn frame(tape: f64, song: f64, checkpoint: bool, notes: Vec<NoteDelta>) -> Frame {
        use crate::judge::{JudgementRangeProfile, JudgementTimeWindow};
        let window = JudgementTimeWindow { early: 0.08, late: 0.08 };
        Frame {
            tape,
            phase: 2,
            countdown: None,
            song,
            chart: song.max(0.),
            alpha: 1.,
            line_color: [1.; 4],
            hud: Hud::default(),
            profile: JudgementRangeProfile {
                tap_perfect: window,
                tap_good: window,
                tap_outer: window,
                hold_perfect: window,
                hold_outer: window,
                drag_outer: window,
                flick_outer: window,
                hold_tail: 0.22,
                tap_x: 0.2,
                hold_x: 0.2,
                drag_x: 0.2,
                flick_x: 0.2,
            },
            bad_visuals: vec![],
            viewport_width: 1920.,
            speed: 1.,
            flow: 1.,
            judgement_multiplier: 1.,
            view: [100., 0., 0.],
            preserve_pitch: false,
            angle: 0.,
            flip_y: false,
            rewind: false,
            rotating: false,
            audible: true,
            checkpoint,
            notes,
            outcomes: vec![],
            fx: vec![],
            sounds: vec![],
            fingers: vec![],
            contacts: vec![],
            blocked: vec![],
            hovers: vec![],
            noise_positions: vec![],
        }
    }
    fn delta(note: u32, state: NoteState) -> NoteDelta {
        NoteDelta { line: 0, note, state }
    }
    fn tape() -> Tape {
        Tape {
            version: VERSION,
            frames: vec![
                frame(0., 0., true, vec![delta(0, NoteState::Waiting), delta(1, NoteState::Waiting)]),
                frame(2., 2., false, vec![delta(0, NoteState::Finished)]),
                frame(2.1, -1., false, vec![]), // Pause rewind: the first note remains judged.
                frame(4., 1., true, vec![delta(0, NoteState::Finished), delta(1, NoteState::Armed)]),
                frame(5., 2., false, vec![delta(1, NoteState::Finished)]),
            ],
        }
    }
    #[test]
    fn seeking_uses_tape_order_across_song_rewind() {
        let tape = tape();
        validate(&tape, &[2]).unwrap();
        assert_eq!(seek_range(&tape, 2.15), (0, 2));
        assert_eq!(seek_range(&tape, 4.5), (3, 3));
        assert_eq!(seek_range(&tape, 999.), (3, 4));
        let (begin, end) = seek_range(&tape, 2.15);
        let mut states = [NoteState::Waiting, NoteState::Waiting];
        for frame in &tape.frames[begin..=end] {
            for note in &frame.notes {
                states[note.note as usize] = note.state.clone();
            }
        }
        assert_eq!(states[0], NoteState::Finished);
        assert_eq!(tape.frames[end].song, -1.);
    }
    #[test]
    fn damaged_tape_is_rejected() {
        let mut bad = tape();
        bad.frames[3].notes.pop();
        assert!(validate(&bad, &[2]).is_err());
        let mut bad = tape();
        bad.frames[2].tape = 1.;
        assert!(validate(&bad, &[2]).is_err());
        let mut bad = tape();
        bad.frames[1].notes[0].note = 10;
        assert!(validate(&bad, &[2]).is_err());
    }
    #[test]
    fn save_load_rename_delete_and_shared_assets() {
        let temp = tempfile::tempdir().unwrap();
        set_root(temp.path());
        let bytes = Arc::new(b"{\"format\":1}".to_vec());
        let hash = digest(&bytes);
        let mut info = ChartInfo::default();
        info.chart = "chart.json".into();
        info.name = "Test".into();
        let manifest = Manifest {
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "one".into(),
            created: "2026-10-04T00:00:00Z".into(),
            duration: 5.,
            info,
            config: Config::default(),
            practice: false,
            range: [0., 5.],
            recorded_range: [0., 2.],
            chart_hash: hash.clone(),
            chart_id: 0,
            assets: BTreeMap::new(),
            respack: None,
            tape_hash: String::new(),
            note_counts: vec![2],
        };
        let assets = Arc::new(Mutex::new(BTreeMap::from([("chart.json".into(), bytes.clone())])));
        let payload = Payload {
            manifest,
            tape: tape(),
            assets,
            pack_assets: Arc::default(),
        };
        let first = payload.manifest.id.clone();
        save(payload.clone()).unwrap();
        let mut another = payload.clone();
        another.manifest.id = uuid::Uuid::new_v4().to_string();
        let second = another.manifest.id.clone();
        another.manifest.created = "2026-10-05T00:00:00Z".into();
        another.manifest.info.name = "Renamed source".into();
        save(another).unwrap();
        assert_eq!(charts().unwrap().len(), 1);
        assert_eq!(charts().unwrap()[0].id, 1);
        assert_eq!(charts().unwrap()[0].created, payload.manifest.created);
        assert_eq!(charts().unwrap()[0].info.name, "Test");
        assert!(list().unwrap().iter().all(|r| r.chart_id == 1 && r.assets.is_empty()));
        assert_eq!(load(&first).unwrap().0.assets["chart.json"], hash);
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 1);
        rename(&first, "new name").unwrap();
        assert_eq!(load(&first).unwrap().0.name, "new name");
        // Invalid batch input must not partially delete a valid recording.
        assert!(delete(&[first.clone(), "../invalid".into()]).is_err());
        assert!(load(&first).is_ok());
        delete(&[first]).unwrap();
        assert_eq!(charts().unwrap().len(), 1);
        assert_eq!(list().unwrap().len(), 1);
        load(&second).unwrap();
        // Check the asset even after deleting another replay sharing it.
        assert_eq!(fs::read(blob_path(&hash).unwrap()).unwrap(), *bytes);
        fs::write(id_path(&second).unwrap().join("tape.zip"), b"broken").unwrap();
        assert!(load(&second).is_err());
        delete(&[second]).unwrap();
        assert!(list().unwrap().is_empty());
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 0);
        assert!(charts().unwrap().is_empty());
        // Emptying the entire library must not recycle the previous chart ID.
        let mut again = payload.clone();
        again.manifest.id = uuid::Uuid::new_v4().to_string();
        let third = again.manifest.id.clone();
        save(again.clone()).unwrap();
        assert_eq!(load(&third).unwrap().0.chart_id, 2);
        again.manifest.id = uuid::Uuid::new_v4().to_string();
        let fourth = again.manifest.id.clone();
        save(again).unwrap();
        // Same name but different contents gets a different immutable record.
        let new_bytes = Arc::new(b"{\"format\":2}".to_vec());
        let mut changed = payload.clone();
        changed.manifest.id = uuid::Uuid::new_v4().to_string();
        changed.manifest.chart_hash = digest(&new_bytes);
        changed.assets = Arc::new(Mutex::new(BTreeMap::from([("chart.json".into(), new_bytes)])));
        let fifth = changed.manifest.id.clone();
        save(changed).unwrap();
        assert_eq!(load(&fifth).unwrap().0.chart_id, 3);
        assert_eq!(charts().unwrap().len(), 2);
        let usage = storage_usage(&list().unwrap()).unwrap();
        assert_eq!(usage.total_bytes, directory_bytes(&root().unwrap()).unwrap());
        assert!(usage.per_chart[&2] > 0 && usage.per_chart[&3] > 0);
        assert_eq!(usage.per_replay[&third], directory_bytes(&id_path(&third).unwrap()).unwrap());
        // Corrupt catalog must block deletion before touching any replay.
        let catalog_path = root().unwrap().join("charts.json");
        let index_bytes = fs::read(&catalog_path).unwrap();
        fs::write(&catalog_path, b"broken").unwrap();
        assert!(delete_charts(&[2]).is_err());
        assert!(id_path(&third).unwrap().exists() && id_path(&fourth).unwrap().exists());
        fs::write(&catalog_path, index_bytes).unwrap();
        delete_charts(&[2]).unwrap();
        assert!(!id_path(&third).unwrap().exists() && !id_path(&fourth).unwrap().exists());
        assert_eq!(charts().unwrap().len(), 1);
        assert!(load(&fifth).is_ok());
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 1);
        delete_charts(&[3]).unwrap();
        assert!(list().unwrap().is_empty() && charts().unwrap().is_empty());
        assert_eq!(catalog_at(&root().unwrap()).unwrap().next_id, 4);
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 0);
        let mut packed = payload.clone();
        packed.manifest.id = uuid::Uuid::new_v4().to_string();
        packed.pack_assets = Arc::new(Mutex::new(BTreeMap::from([("info.yml".into(), Arc::new(b"shared pack".to_vec()))])));
        let packed_first = packed.manifest.id.clone();
        save(packed.clone()).unwrap();
        let pack_hash = load(&packed_first).unwrap().0.respack.unwrap();
        packed.manifest.id = uuid::Uuid::new_v4().to_string();
        let new_bytes = Arc::new(b"{\"format\":4}".to_vec());
        packed.manifest.chart_hash = digest(&new_bytes);
        packed.assets = Arc::new(Mutex::new(BTreeMap::from([("chart.json".into(), new_bytes)])));
        let packed_second = packed.manifest.id.clone();
        save(packed).unwrap();
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 3);
        delete_charts(&[4]).unwrap();
        assert!(load(&packed_second).is_ok());
        assert!(blob_path(&pack_hash).unwrap().exists());
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 2);
        let snapshot = library_snapshot().unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.charts.len(), 1);
        assert_eq!(snapshot.entries[0].chart_id, snapshot.charts[0].id);
        delete_charts(&[5]).unwrap();
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 0);
        // A reused chart number must still reproduce changed recording media.
        let mut media = payload.clone();
        media.manifest.id = uuid::Uuid::new_v4().to_string();
        media.assets = Arc::new(Mutex::new(BTreeMap::from([
            ("chart.json".into(), bytes.clone()),
            ("music.ogg".into(), Arc::new(b"first music".to_vec())),
        ])));
        let media_first = media.manifest.id.clone();
        save(media.clone()).unwrap();
        media.manifest.id = uuid::Uuid::new_v4().to_string();
        media.assets = Arc::new(Mutex::new(BTreeMap::from([
            ("chart.json".into(), bytes.clone()),
            ("music.ogg".into(), Arc::new(b"new music".to_vec())),
        ])));
        let media_second = media.manifest.id.clone();
        save(media).unwrap();
        assert_eq!(charts().unwrap().len(), 1);
        assert_eq!(load(&media_first).unwrap().0.chart_id, load(&media_second).unwrap().0.chart_id);
        assert_eq!(load(&media_second).unwrap().0.assets["music.ogg"], digest(b"new music"));
        assert_eq!(load(&media_first).unwrap().0.assets["music.ogg"], digest(b"first music"));
        delete(&[media_first]).unwrap();
        assert_eq!(load(&media_second).unwrap().0.assets["music.ogg"], digest(b"new music"));
        delete(&[media_second]).unwrap();
        assert!(charts().unwrap().is_empty());
        assert_eq!(fs::read_dir(root().unwrap().join("assets")).unwrap().count(), 0);
        // Old root is isolated and never migrated/deleted.
        fs::create_dir_all(temp.path().join("replays")).unwrap();
        fs::write(temp.path().join("replays/old-tape"), b"old").unwrap();
        assert!(list().unwrap().is_empty());
        assert_eq!(fs::read(temp.path().join("replays/old-tape")).unwrap(), b"old");
    }
    #[test]
    fn unsupported_old_tape_version_is_rejected() {
        let mut old = tape();
        old.version = 2;
        assert!(validate(&old, &[2]).is_err());
    }
    #[test]
    fn catalog_rejects_recycled_or_mutated_ids() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(catalog_at(temp.path()).unwrap().next_id, 1);
        fs::write(temp.path().join("charts.json"), b"{\"next_id\":0,\"records\":{}}").unwrap();
        assert!(catalog_at(temp.path()).is_err());
        fs::write(temp.path().join("charts.json"), b"not json").unwrap();
        assert!(catalog_at(temp.path()).is_err());
    }
    #[test]
    fn hold_infinite_release_roundtrip() {
        let source = JudgeStatus::Hold(true, 1., -0.02, false, f64::INFINITY, 2, false);
        let serialized = serde_json::to_vec(&NoteState::capture(&source)).unwrap();
        let restored: NoteState = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored, NoteState::capture(&source));
        assert!(matches!(restored.restore(),JudgeStatus::Hold(_,_,_,_,u,_,_) if u.is_infinite()));
    }
    #[test]
    fn asset_hash_rejects_path_traversal() {
        assert!(blob_path("../../file").is_err());
        assert!(id_path("../file").is_err());
    }
}

/// Clamp a recorded-frame step without assuming a fixed recording frame rate.
pub fn step_frame(current: usize, count: usize, delta: isize) -> usize {
    current.saturating_add_signed(delta).min(count.saturating_sub(1))
}

pub fn annotation_alpha(age: f64) -> f32 {
    if !(-1.0..0.3).contains(&age) {
        0.
    } else if age < 0. {
        1.
    } else {
        (1. - age / 0.3) as f32
    }
}

#[cfg(test)]
mod viewer_tests {
    #[test]
    fn previous_tapes_without_projected_centers_still_load() {
        let outcome = super::Outcome {
            line: 0,
            note: 1,
            delivered: 1.,
            hit_time: 1.,
            difference: 0.,
            result: Ok(crate::judge::Judgement::Perfect),
            custom_stage: None,
            color: [1., 0.8, 0., 1.],
            center: [0., 0.],
            rendered_center: Some([0.1, 0.2]),
        };
        let mut old = serde_json::to_value(&outcome).unwrap();
        old.as_object_mut().unwrap().remove("rendered_center");
        let decoded: super::Outcome = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.rendered_center, None);
        assert_eq!(decoded.result, outcome.result);
        let new: super::Outcome = serde_json::from_slice(&serde_json::to_vec(&outcome).unwrap()).unwrap();
        assert_eq!(new.rendered_center, Some([0.1, 0.2]));
    }
    #[test]
    fn recorded_frame_steps_clamp() {
        assert_eq!(super::step_frame(3, 12, -10), 0);
        assert_eq!(super::step_frame(3, 12, 1), 4);
        assert_eq!(super::step_frame(3, 12, 10), 11);
        assert_eq!(super::step_frame(11, 12, -1), 10);
    }
    #[test]
    fn labels_appear_one_second_early_and_fade_in_point_three() {
        assert_eq!(super::annotation_alpha(-1.01), 0.);
        assert_eq!(super::annotation_alpha(-1.), 1.);
        assert_eq!(super::annotation_alpha(0.), 1.);
        assert!((super::annotation_alpha(0.15) - 0.5).abs() < 1e-6);
        assert_eq!(super::annotation_alpha(0.3), 0.);
    }
}

#[cfg(test)]
mod practice_stop_tests {
    use super::*;
    #[test]
    fn reset_drains_recording_once_and_disarms() {
        let mut armed = true;
        let mut recording = Some(vec![1, 2, 3]);
        assert_eq!(take_practice_recording(true, &mut armed, &mut recording), Some(vec![1, 2, 3]));
        assert!(!armed);
        assert!(recording.is_none());
        assert_eq!(take_practice_recording(true, &mut armed, &mut recording), None);
    }
    #[test]
    fn pending_arm_and_normal_recordings_survive() {
        let mut armed = true;
        let mut none: Option<u8> = None;
        assert_eq!(take_practice_recording(true, &mut armed, &mut none), None);
        assert!(armed);
        let mut recording = Some(7);
        assert_eq!(take_practice_recording(false, &mut armed, &mut recording), None);
        assert_eq!(recording, Some(7));
        assert!(armed);
    }
    #[test]
    fn exact_boundary_and_end_of_track_finish_practice() {
        assert!(practice_range_finished(10., 10., 20., false));
        assert!(practice_range_finished(20., 25., 20., false));
        assert!(!practice_range_finished(9.99, 10., 20., false));
        assert!(!practice_range_finished(10., 10., 20., true));
    }
}
