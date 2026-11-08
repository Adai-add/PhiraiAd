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

/// The two independent RKS groups, ordered AP first and Best second.
#[derive(Clone, Debug)]
pub struct RankedEntry {
    pub path: Option<String>,
    pub name: String,
    pub difficulty: f64,
    pub accuracy: f64,
    pub rks: f64,
    pub contribution: f64,
    pub ap: bool,
    pub ap_group: bool,
    key: String,
}

pub fn ranked_entries(settings: &Settings, locals: &[LocalInput]) -> Vec<RankedEntry> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for input in locals {
        if !seen.insert(&input.path) {
            continue;
        }
        let r = resolve(input, &settings.chart(&input.path));
        if r.included {
            if let Some((difficulty, accuracy)) = r.difficulty.zip(r.accuracy) {
                candidates.push(RankedEntry {
                    path: Some(input.path.clone()),
                    name: input.name.clone(),
                    difficulty,
                    accuracy,
                    rks: single_rks(difficulty, accuracy),
                    contribution: 0.,
                    ap: accuracy == 100.,
                    ap_group: false,
                    key: format!("local:{}", input.path),
                });
            }
        }
    }
    for extra in &settings.extras {
        if extra.included {
            if let Some((difficulty, accuracy)) = difficulty_value(extra.difficulty).zip(accuracy_value(extra.accuracy)) {
                candidates.push(RankedEntry {
                    path: None,
                    name: extra.name.clone(),
                    difficulty,
                    accuracy,
                    rks: single_rks(difficulty, accuracy),
                    contribution: 0.,
                    ap: accuracy == 100.,
                    ap_group: false,
                    key: format!("extra:{}", extra.id),
                });
            }
        }
    }
    candidates.sort_by(|a, b| b.rks.total_cmp(&a.rks).then_with(|| a.key.cmp(&b.key)));
    let mut entries: Vec<_> = candidates
        .iter()
        .filter(|entry| entry.ap)
        .take(settings.ap_count)
        .cloned()
        .map(|mut entry| {
            entry.ap_group = true;
            entry
        })
        .collect();
    entries.extend(candidates.into_iter().take(settings.best_count));
    let divisor = settings.best_count.saturating_add(settings.ap_count) as f64;
    for entry in &mut entries {
        entry.contribution = if divisor > 0. { entry.rks / divisor } else { 0. };
    }
    entries
}

/// A library hint uses the same effective inputs and deterministic membership as
/// the actual Bn/AP board. TargetAccuracy stores absolute accuracy in hundredths
/// of a percent, independent of the chart's current accuracy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RksGainHint {
    Phi,
    NoGain,
    Ready,
    TargetAccuracy(u32),
    Pending,
}
impl RksGainHint {
    pub fn text(self) -> String {
        match self {
            Self::Phi => "φ".into(),
            Self::NoGain => "√".into(),
            Self::Ready => "↑".into(),
            Self::TargetAccuracy(cents) => format!("↑{}.{:02}%", cents / 100, cents % 100),
            Self::Pending => "—".into(),
        }
    }
}

pub fn improvement_hints(settings: &Settings, locals: &[LocalInput], resolved: &HashMap<String, Resolved>) -> HashMap<String, RksGainHint> {
    let board = ranked_entries(settings, locals);
    let best: Vec<_> = board.iter().filter(|e| !e.ap_group).collect();
    let ap: Vec<_> = board.iter().filter(|e| e.ap_group).collect();
    let members: std::collections::HashSet<_> = best.iter().filter_map(|e| e.path.as_deref()).collect();
    let best_full = best.len() >= settings.best_count;
    let best_cutoff = best.last().map_or(0., |e| e.rks);
    let ap_cutoff = if ap.len() < settings.ap_count {
        0.
    } else {
        ap.last().map_or(0., |e| e.rks)
    };
    resolved
        .iter()
        .map(|(path, value)| {
            let accuracy = value.accuracy.filter(|a| a.is_finite() && *a >= 0.).unwrap_or(0.);
            let hint = if accuracy >= 100. {
                RksGainHint::Phi
            } else if !value.included {
                RksGainHint::NoGain
            } else if let Some(difficulty) = value.difficulty.filter(|d| d.is_finite() && *d >= 0.) {
                let best_can_gain = settings.best_count > 0 && difficulty > if best_full { best_cutoff } else { 0. };
                let ap_can_gain = settings.ap_count > 0 && difficulty > ap_cutoff;
                if !best_can_gain && !ap_can_gain {
                    RksGainHint::NoGain
                } else if members.contains(path.as_str()) || settings.best_count > 0 && !best_full {
                    RksGainHint::Ready
                } else if best_can_gain {
                    // The formula is zero below 70%; include that discontinuity.
                    let target = (55. + 45. * (best_cutoff / difficulty).sqrt()).max(70.);
                    let mut cents = (target * 100.).floor() as u32;
                    // Verify the displayed absolute accuracy, including strict ties
                    // and floating point boundaries. best_can_gain guarantees
                    // that the search terminates at or below 100.00%.
                    while single_rks(difficulty, cents as f64 / 100.) <= best_cutoff {
                        cents += 1;
                    }
                    RksGainHint::TargetAccuracy(cents)
                } else {
                    // With Bn disabled (or unreachable), only becoming AP can help.
                    RksGainHint::TargetAccuracy(10000)
                }
            } else {
                RksGainHint::Pending
            };
            (path.clone(), hint)
        })
        .collect()
}

#[cfg(test)]
mod board_tests {
    use super::*;
    fn input(path: &str, difficulty: f64, accuracy: f64) -> LocalInput {
        LocalInput {
            path: path.into(),
            name: path.into(),
            level: "IN".into(),
            difficulty,
            ai_difficulty: None,
            accuracy: Some(accuracy),
        }
    }
    #[test]
    fn ap_first_independent_best_and_missing_slots() {
        let settings = Settings {
            best_count: 4,
            ap_count: 2,
            ..Default::default()
        };
        let locals = vec![input("normal", 17., 99.), input("ap", 16., 100.)];
        let entries = ranked_entries(&settings, &locals);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].ap_group);
        assert_eq!(entries[0].path.as_deref(), Some("ap"));
        assert_eq!(entries[1].path.as_deref(), Some("normal"));
        assert!(!entries[1].ap_group);
        assert_eq!(entries[2].path.as_deref(), Some("ap"));
        let sum: f64 = entries.iter().map(|entry| entry.rks).sum();
        assert!((sum / 6. - calculate(&settings, &locals).1.rks).abs() < 1e-10);
        assert!((entries.iter().map(|entry| entry.contribution).sum::<f64>() - calculate(&settings, &locals).1.rks).abs() < 1e-10);
    }
    #[test]
    fn exclusions_overrides_extras_and_deterministic_ties() {
        let mut settings = Settings {
            best_count: 4,
            ap_count: 2,
            ..Default::default()
        };
        settings.charts.insert(
            "excluded".into(),
            ChartSettings {
                included: false,
                ..Default::default()
            },
        );
        settings.charts.insert(
            "custom".into(),
            ChartSettings {
                difficulty_source: DifficultySource::Custom,
                custom_difficulty: Some(20.),
                accuracy_source: AccuracySource::Custom,
                custom_accuracy: Some(100.),
                ..Default::default()
            },
        );
        settings.extras.push(ExtraEntry {
            id: 1,
            name: "Extra".into(),
            difficulty: 18.,
            accuracy: 100.,
            included: true,
        });
        let locals = vec![
            input("excluded", 25., 100.),
            input("custom", 10., 70.),
            input("a", 16., 99.999999),
            input("b", 16., 99.999999),
        ];
        let entries = ranked_entries(&settings, &locals);
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[0].difficulty, 20.);
        assert_eq!(entries[1].path, None);
        assert!(entries.iter().all(|entry| entry.path.as_deref() != Some("excluded")));
        assert!(!entries[4].ap);
        assert_eq!(entries[4].path.as_deref(), Some("a"));
        let mut reversed = locals.clone();
        reversed.reverse();
        assert_eq!(
            entries.iter().map(|e| &e.key).collect::<Vec<_>>(),
            ranked_entries(&settings, &reversed).iter().map(|e| &e.key).collect::<Vec<_>>()
        );
    }
    #[test]
    fn empty_and_duplicate_paths_do_not_fill_slots() {
        let settings = Settings::default();
        assert!(ranked_entries(&settings, &[]).is_empty());
        let local = input("a", 15., 99.);
        assert_eq!(ranked_entries(&settings, &[local.clone(), local]).len(), 1);
    }
}

/// Compare available real-record sources before display rounding; custom overrides remain independent.
pub fn highest_record_accuracy(ordinary_or_cloud: Option<f64>, personal_local: Option<f64>) -> Option<f64> {
    ordinary_or_cloud
        .into_iter()
        .chain(personal_local)
        .filter_map(accuracy_value)
        .max_by(f64::total_cmp)
}

#[cfg(test)]
mod gain_tests {
    use super::*;
    fn input(path: &str, difficulty: f64, accuracy: Option<f64>) -> LocalInput {
        LocalInput {
            path: path.into(),
            name: path.into(),
            level: "IN".into(),
            difficulty,
            accuracy,
            ai_difficulty: None,
        }
    }
    fn hints(s: &Settings, locals: &[LocalInput]) -> HashMap<String, RksGainHint> {
        improvement_hints(s, locals, &calculate(s, locals).0)
    }
    #[test]
    fn priority_phi_no_gain_and_membership() {
        let s = Settings {
            best_count: 2,
            ap_count: 1,
            ..Default::default()
        };
        let locals = [input("ap", 20., Some(100.)), input("best", 16., Some(99.)), input("weak", 10., Some(80.))];
        let h = hints(&s, &locals);
        assert_eq!(h["ap"], RksGainHint::Phi);
        assert_eq!(h["best"], RksGainHint::Ready);
        assert_eq!(h["weak"], RksGainHint::NoGain);
        assert_eq!(RksGainHint::Phi.text(), "φ");
        assert_eq!(RksGainHint::NoGain.text(), "√");
        assert_eq!(RksGainHint::Ready.text(), "↑");
        assert_eq!(RksGainHint::TargetAccuracy(9901).text(), "↑99.01%");
        assert_eq!(RksGainHint::TargetAccuracy(10000).text(), "↑100.00%");
        let mut resolved = calculate(&s, &locals).0;
        resolved.get_mut("ap").unwrap().accuracy = Some(100.01);
        assert_eq!(improvement_hints(&s, &locals, &resolved)["ap"], RksGainHint::Phi);
    }
    #[test]
    fn tie_outside_bn_requires_a_strict_gain_and_stable_membership() {
        let s = Settings {
            best_count: 1,
            ap_count: 0,
            ..Default::default()
        };
        let locals = [input("a", 16., Some(99.)), input("b", 16., Some(99.))];
        let h = hints(&s, &locals);
        assert_eq!(h["a"], RksGainHint::Ready);
        assert_eq!(h["b"], RksGainHint::TargetAccuracy(9901));
        assert_eq!(h, hints(&s, &[locals[1].clone(), locals[0].clone()]));
    }
    #[test]
    fn unfilled_bn_includes_unplayed_charts_and_zero_bn_uses_ap_slots() {
        let mut s = Settings {
            best_count: 4,
            ap_count: 0,
            ..Default::default()
        };
        let locals = [input("record", 15., Some(99.)), input("unplayed", 16., None)];
        assert!(hints(&s, &locals).values().all(|h| *h == RksGainHint::Ready));
        s.best_count = 0;
        s.ap_count = 1;
        assert_eq!(hints(&s, &locals)["record"], RksGainHint::TargetAccuracy(10000));
        assert_eq!(hints(&s, &locals)["unplayed"], RksGainHint::TargetAccuracy(10000));
        let locals = [
            input("ap", 20., Some(100.)),
            input("weak", 19., Some(99.)),
            input("next", 21., Some(99.99)),
        ];
        let h = hints(&s, &locals);
        assert_eq!(h["weak"], RksGainHint::NoGain);
        assert_eq!(h["next"], RksGainHint::TargetAccuracy(10000));
        s.ap_count = 0;
        assert_eq!(hints(&s, &locals)["next"], RksGainHint::NoGain);
    }
    #[test]
    fn cap_ties_can_still_gain_through_an_unfilled_ap_group() {
        let mut s = Settings {
            best_count: 1,
            ap_count: 1,
            ..Default::default()
        };
        let locals = [input("ap", 15., Some(100.)), input("next", 15., Some(99.))];
        assert_eq!(hints(&s, &locals)["next"], RksGainHint::NoGain);
        s.ap_count = 2;
        assert_eq!(hints(&s, &locals)["next"], RksGainHint::TargetAccuracy(10000));
        s.best_count = 0;
        let near = [input("near", 16., Some(99.999999))];
        assert_eq!(hints(&s, &near)["near"], RksGainHint::TargetAccuracy(10000));
    }
    #[test]
    fn zero_cutoff_respects_seventy_percent_discontinuity() {
        let s = Settings {
            best_count: 1,
            ap_count: 0,
            ..Default::default()
        };
        let locals = [input("a-cut", 0., Some(100.)), input("z-next", 9., Some(65.))];
        assert_eq!(hints(&s, &locals)["z-next"], RksGainHint::TargetAccuracy(7000));
    }
    #[test]
    fn overrides_extras_exclusions_and_pending_ai_follow_actual_rks_inputs() {
        let mut s = Settings {
            best_count: 1,
            ap_count: 0,
            ..Default::default()
        };
        s.extras.push(ExtraEntry {
            id: 1,
            name: "extra".into(),
            difficulty: 17.,
            accuracy: 100.,
            included: true,
        });
        let locals = [input("local", 16., Some(99.)), input("ai", 20., Some(99.))];
        s.charts.insert(
            "ai".into(),
            ChartSettings {
                difficulty_source: DifficultySource::Ai,
                ..Default::default()
            },
        );
        assert_eq!(hints(&s, &locals)["ai"], RksGainHint::Pending);
        assert_eq!(hints(&s, &locals)["local"], RksGainHint::NoGain);
        s.charts.insert(
            "local".into(),
            ChartSettings {
                difficulty_source: DifficultySource::Custom,
                custom_difficulty: Some(18.),
                accuracy_source: AccuracySource::Custom,
                custom_accuracy: Some(99.9),
                ..Default::default()
            },
        );
        assert_eq!(hints(&s, &locals)["local"], RksGainHint::Ready);
        s.charts.get_mut("local").unwrap().included = false;
        assert_eq!(hints(&s, &locals)["local"], RksGainHint::NoGain);
    }
    #[test]
    fn displayed_absolute_targets_really_improve_the_calculator_and_are_minimal() {
        let s = Settings {
            best_count: 1,
            ap_count: 0,
            ..Default::default()
        };
        let mut checked = 0;
        for cutoff_d in [1., 10., 16., 20.] {
            for cutoff_acc in [0., 70., 90., 99.99, 100.] {
                for d in [0., 5., 15., 17., 21., 25.] {
                    for acc in [0., 65., 69.99, 70., 90., 98.1234, 99., 99.009, 99.999999] {
                        let mut locals = [input("a-cut", cutoff_d, Some(cutoff_acc)), input("z-next", d, Some(acc))];
                        let before = calculate(&s, &locals).1.rks;
                        match hints(&s, &locals)["z-next"] {
                            RksGainHint::TargetAccuracy(cents) => {
                                let target = cents as f64 / 100.;
                                assert!(target > acc && target <= 100.);
                                locals[1].accuracy = Some(target);
                                assert!(calculate(&s, &locals).1.rks > before, "d={d}, acc={acc}, cents={cents}");
                                locals[1].accuracy = Some((cents - 1) as f64 / 100.);
                                assert!(calculate(&s, &locals).1.rks <= before, "target was overstated");
                                checked += 1;
                            }
                            RksGainHint::NoGain => {
                                locals[1].accuracy = Some(100.);
                                assert_eq!(calculate(&s, &locals).1.rks, before);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        assert!(checked > 100);
    }

    #[test]
    fn absolute_target_is_independent_of_current_accuracy_and_rounds_strictly_up() {
        let s = Settings {
            best_count: 1,
            ap_count: 0,
            ..Default::default()
        };
        for (cutoff, expected) in [
            (70., 7001),
            (98.12, 9813),
            (99., 9901),
            (99.009, 9901),
            (99.99, 10000),
            (99.999999, 10000),
        ] {
            for accuracy in [0., 65., 80., 98.123456] {
                if accuracy > cutoff {
                    continue;
                }
                let locals = [input("a-cut", 16., Some(cutoff)), input("z-next", 16., Some(accuracy))];
                assert_eq!(hints(&s, &locals)["z-next"], RksGainHint::TargetAccuracy(expected));
            }
        }
    }
}
