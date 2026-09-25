//! Local-only RKS settings, resolved chart inputs and deterministic calculation.
//! No chart metadata, real records, upload eligibility or judgement is changed.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DifficultySource {
    #[default]
    File,
    Custom,
    Ai,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccuracySource {
    #[default]
    Record,
    Custom,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChartSettings {
    pub included: bool,
    pub difficulty_source: DifficultySource,
    pub custom_difficulty: Option<f64>,
    pub accuracy_source: AccuracySource,
    pub custom_accuracy: Option<f64>, // percentage, never a fraction
}
impl Default for ChartSettings {
    fn default() -> Self {
        Self {
            included: true,
            difficulty_source: DifficultySource::File,
            custom_difficulty: None,
            accuracy_source: AccuracySource::Record,
            custom_accuracy: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraEntry {
    pub id: u64,
    pub name: String,
    pub difficulty: f64,
    pub accuracy: f64,
    #[serde(default = "yes")]
    pub included: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub charts: HashMap<String, ChartSettings>,
    pub extras: Vec<ExtraEntry>,
    pub best_count: usize,
    pub ap_count: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            charts: HashMap::new(),
            extras: Vec::new(),
            best_count: 27,
            ap_count: 3,
        }
    }
}
impl Settings {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let s: Self = serde_json::from_slice(&bytes)?;
                if s.version != 1 || s.best_count > 1000 || s.ap_count > 1000 || s.best_count + s.ap_count == 0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Unsupported RKS settings or invalid counts"));
                }
                let mut ids = std::collections::HashSet::new();
                if s.extras.iter().any(|e| !ids.insert(e.id)) {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Duplicate extra entry ID"));
                }
                Ok(s)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    /// Write and sync a sibling temp before replacing the settings. Failure never
    /// truncates the old settings, and callers commit their in-memory copy last.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(tmp, path)
    }
    pub fn chart(&self, path: &str) -> ChartSettings {
        self.charts.get(path).cloned().unwrap_or_default()
    }
    pub fn next_id(&self) -> u64 {
        self.extras.iter().map(|e| e.id).max().unwrap_or(0).saturating_add(1)
    }
}

#[derive(Clone, Debug)]
pub struct LocalInput {
    pub path: String,
    pub name: String,
    pub level: String,
    pub difficulty: f64,
    pub ai_difficulty: Option<f64>,
    pub accuracy: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct Resolved {
    pub included: bool,
    pub difficulty: Option<f64>,
    pub accuracy: Option<f64>,
    pub difficulty_changed: bool,
    pub difficulty_ai: bool,
    pub accuracy_custom: bool,
    pub single_rks: Option<f64>,
}
pub fn difficulty_value(v: f64) -> Option<f64> {
    // Round only constants, as requested. Real record accuracy remains exact.
    (v.is_finite() && v >= 0. && v <= 1e6).then(|| (v * 10.).round() / 10.)
}
pub fn accuracy_value(v: f64) -> Option<f64> {
    (v.is_finite() && (0. ..=100.).contains(&v)).then_some(v)
}
pub fn format_difficulty(v: f64) -> String {
    let s = format!("{:.1}", v);
    s.strip_suffix(".0").unwrap_or(&s).to_owned()
}
/// Keep arbitrary difficulty prefixes (including HOT.15 and INS) intact.
pub fn level_prefix(level: &str) -> String {
    level
        .rfind("Lv.")
        .map(|p| level[..p + 3].to_owned())
        .unwrap_or_else(|| format!("{level} Lv."))
}
pub fn single_rks(difficulty: f64, accuracy: f64) -> f64 {
    if accuracy < 70. {
        0.
    } else {
        difficulty * ((accuracy - 55.) / 45.).powi(2)
    }
}
pub fn resolve(input: &LocalInput, settings: &ChartSettings) -> Resolved {
    let original = difficulty_value(input.difficulty);
    let difficulty = match settings.difficulty_source {
        DifficultySource::File => original,
        DifficultySource::Custom => settings.custom_difficulty.and_then(difficulty_value),
        DifficultySource::Ai => input.ai_difficulty.and_then(difficulty_value),
    };
    let accuracy = match settings.accuracy_source {
        AccuracySource::Record => input.accuracy.and_then(accuracy_value),
        AccuracySource::Custom => settings.custom_accuracy.and_then(accuracy_value),
    };
    Resolved {
        included: settings.included,
        difficulty_changed: difficulty.is_some() && difficulty != original,
        difficulty_ai: settings.difficulty_source == DifficultySource::Ai && difficulty.is_some(),
        accuracy_custom: settings.accuracy_source == AccuracySource::Custom,
        difficulty,
        accuracy,
        single_rks: difficulty.zip(accuracy).map(|(d, a)| single_rks(d, a)),
    }
}
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub rks: f64,
    pub selected: usize,
    pub pending: usize,
    pub best_used: usize,
    pub ap_used: usize,
}
/// Both lists independently select from the candidates: AP entries can also
/// appear in Best. Missing slots contribute zero; the divisor stays N + M.
pub fn calculate(settings: &Settings, locals: &[LocalInput]) -> (HashMap<String, Resolved>, Summary) {
    let mut map = HashMap::new();
    let mut summary = Summary::default();
    let mut best = Vec::new();
    let mut ap = Vec::new();
    let mut push = |r: &Resolved| {
        if !r.included {
            return;
        }
        summary.selected += 1;
        if let Some(value) = r.single_rks {
            best.push(value);
            if r.accuracy == Some(100.) {
                ap.push(value);
            }
        } else {
            summary.pending += 1;
        }
    };
    for input in locals {
        if map.contains_key(&input.path) {
            continue;
        }
        let r = resolve(input, &settings.chart(&input.path));
        push(&r);
        map.insert(input.path.clone(), r);
    }
    for extra in &settings.extras {
        let d = difficulty_value(extra.difficulty);
        let a = accuracy_value(extra.accuracy);
        push(&Resolved {
            included: extra.included,
            difficulty: d,
            accuracy: a,
            difficulty_changed: false,
            difficulty_ai: false,
            accuracy_custom: true,
            single_rks: d.zip(a).map(|(d, a)| single_rks(d, a)),
        });
    }
    best.sort_by(|a, b| b.total_cmp(a));
    ap.sort_by(|a, b| b.total_cmp(a));
    summary.best_used = best.len().min(settings.best_count);
    summary.ap_used = ap.len().min(settings.ap_count);
    let divisor = settings.best_count.saturating_add(settings.ap_count);
    if divisor > 0 {
        summary.rks = (best.iter().take(settings.best_count).sum::<f64>() + ap.iter().take(settings.ap_count).sum::<f64>()) / divisor as f64;
    }
    (map, summary)
}

/// Compare available real-record sources before display rounding; custom overrides remain independent.
pub fn highest_record_accuracy(ordinary_or_cloud: Option<f64>, personal_local: Option<f64>) -> Option<f64> {
    ordinary_or_cloud
        .into_iter()
        .chain(personal_local)
        .filter_map(accuracy_value)
        .max_by(f64::total_cmp)
}
