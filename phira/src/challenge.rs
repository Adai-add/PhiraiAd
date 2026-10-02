//! Local course records and deterministic badge rules, separate from custom RKS.
use serde::{Deserialize, Serialize};

pub struct ChallengeSelection<T> {
    pub slots: [Option<T>; 3],
    pub focused: usize,
}
impl<T> Default for ChallengeSelection<T> {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            focused: 0,
        }
    }
}
impl<T: Clone> ChallengeSelection<T> {
    pub fn ready(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }
    pub fn select(&mut self, chart: T) {
        self.slots[self.focused] = Some(chart);
        self.focused = self.slots.iter().position(Option::is_none).unwrap_or(self.focused);
    }
    pub fn select_unique(&mut self, chart: T, same: impl Fn(&T, &T) -> bool) -> bool {
        if self.slots.iter().flatten().any(|old| same(old, &chart)) {
            return false;
        }
        self.select(chart);
        true
    }
    pub fn remove(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = None;
            self.focused = index;
        }
    }
    pub fn course(&self) -> Option<[T; 3]> {
        Some([
            self.slots[0].as_ref()?.clone(),
            self.slots[1].as_ref()?.clone(),
            self.slots[2].as_ref()?.clone(),
        ])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChallengeSong {
    pub name: String,
    pub level: String,
    pub difficulty: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChallengeResult {
    pub song: ChallengeSong,
    pub score: u32,
    pub accuracy: f64,
    pub counts: [u32; 4],
    pub early: u32,
    pub late: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BadgeTier {
    Green,
    Blue,
    Orange,
    Yellow,
    Rainbow,
}

impl BadgeTier {
    pub fn from_score(score: u32) -> Option<Self> {
        match score {
            2_460_000..=2_699_999 => Some(Self::Green),
            2_700_000..=2_849_999 => Some(Self::Blue),
            2_850_000..=2_939_999 => Some(Self::Orange),
            2_940_000..=2_999_999 => Some(Self::Yellow),
            3_000_000 => Some(Self::Rainbow),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BadgeNumber {
    Integer,
    Precise,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChallengeBadge {
    pub tier: BadgeTier,
    pub number: BadgeNumber,
    pub value: f64,
    pub results: [ChallengeResult; 3],
}
impl ChallengeBadge {
    pub fn label(&self) -> String {
        match self.number {
            BadgeNumber::Integer => format!("{:.0}", self.value),
            BadgeNumber::Precise => format!("{:.1}", self.value),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChallengeSlots {
    pub slots: [Option<ChallengeBadge>; 5],
    pub displayed: Option<usize>,
}
impl ChallengeSlots {
    pub fn display(&self) -> Option<&ChallengeBadge> {
        self.displayed.and_then(|index| self.slots.get(index)).and_then(Option::as_ref)
    }
    pub fn save(&mut self, index: usize, badge: ChallengeBadge) -> bool {
        let Some(slot) = self.slots.get_mut(index) else { return false };
        *slot = Some(badge);
        true
    }
    pub fn select(&mut self, index: usize) -> bool {
        if self.slots.get(index).is_some_and(Option::is_some) {
            self.displayed = Some(index);
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
pub struct ChallengeProgress {
    pub results: Vec<ChallengeResult>,
}
impl ChallengeProgress {
    pub fn index(&self) -> usize {
        self.results.len()
    }
    pub fn restart(&mut self) {
        self.results.clear();
    }
    pub fn complete(&mut self, result: ChallengeResult) -> bool {
        if self.results.len() >= 3 {
            return false;
        }
        self.results.push(result);
        true
    }
    pub fn total(&self) -> u32 {
        self.results.iter().map(|r| r.score).sum()
    }
    pub fn badge(&self, number: BadgeNumber) -> Option<ChallengeBadge> {
        let results: [ChallengeResult; 3] = self.results.clone().try_into().ok()?;
        if results.iter().any(|r| !r.song.difficulty.is_finite() || r.song.difficulty < 0.) {
            return None;
        }
        let value = match number {
            BadgeNumber::Integer => results.iter().map(|r| r.song.difficulty.floor()).sum(),
            BadgeNumber::Precise => results.iter().map(|r| r.song.difficulty).sum(),
        };
        Some(ChallengeBadge {
            tier: BadgeTier::from_score(self.total())?,
            number,
            value,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_selection_replacement_and_empty_start() {
        let mut selection = ChallengeSelection::default();
        for chart in ["first", "second"] {
            selection.select(chart);
            assert!(!selection.ready());
            assert!(selection.course().is_none());
        }
        selection.select("third");
        assert_eq!(selection.course(), Some(["first", "second", "third"]));
        selection.remove(1);
        assert!(!selection.ready());
        assert_eq!(selection.focused, 1);
        selection.select("replacement");
        assert_eq!(selection.course(), Some(["first", "replacement", "third"]));
        selection.focused = 0;
        selection.select("new first");
        assert_eq!(selection.course(), Some(["new first", "replacement", "third"]));
    }
    fn result(difficulty: f64, score: u32) -> ChallengeResult {
        ChallengeResult {
            song: ChallengeSong {
                name: "测试".into(),
                level: "AT".into(),
                difficulty,
            },
            score,
            accuracy: 1.,
            counts: [1, 0, 0, 0],
            early: 0,
            late: 0,
        }
    }
    #[test]
    fn all_tier_boundaries() {
        for (score, tier) in [
            (2_459_999, None),
            (2_460_000, Some(BadgeTier::Green)),
            (2_699_999, Some(BadgeTier::Green)),
            (2_700_000, Some(BadgeTier::Blue)),
            (2_849_999, Some(BadgeTier::Blue)),
            (2_850_000, Some(BadgeTier::Orange)),
            (2_939_999, Some(BadgeTier::Orange)),
            (2_940_000, Some(BadgeTier::Yellow)),
            (2_999_999, Some(BadgeTier::Yellow)),
            (3_000_000, Some(BadgeTier::Rainbow)),
            (3_000_001, None),
        ] {
            assert_eq!(BadgeTier::from_score(score), tier);
        }
    }
    #[test]
    fn course_restart_and_two_numbers() {
        let mut run = ChallengeProgress::default();
        run.complete(result(16.9, 1_000_000));
        assert_eq!(run.index(), 1);
        assert!(run.badge(BadgeNumber::Integer).is_none());
        run.restart();
        assert_eq!(run.index(), 0);
        assert_eq!(run.total(), 0);
        for difficulty in [16.9, 16.8, 16.7] {
            assert!(run.complete(result(difficulty, 1_000_000)));
        }
        assert!(!run.complete(result(1., 1_000_000)));
        assert_eq!(run.badge(BadgeNumber::Integer).unwrap().label(), "48");
        assert_eq!(run.badge(BadgeNumber::Precise).unwrap().label(), "50.4");
    }
    #[test]
    fn slots_overwrite_roundtrip_and_selection() {
        let mut run = ChallengeProgress::default();
        for _ in 0..3 {
            run.complete(result(16.8, 1_000_000));
        }
        let mut slots = ChallengeSlots::default();
        assert!(!slots.select(0));
        assert!(slots.save(0, run.badge(BadgeNumber::Integer).unwrap()));
        assert!(slots.select(0));
        assert!(slots.save(0, run.badge(BadgeNumber::Precise).unwrap()));
        assert_eq!(slots.display().unwrap().label(), "50.4");
        assert!(slots.save(4, run.badge(BadgeNumber::Integer).unwrap()));
        assert!(!slots.save(5, run.badge(BadgeNumber::Integer).unwrap()));
        let restored: ChallengeSlots = serde_json::from_str(&serde_json::to_string(&slots).unwrap()).unwrap();
        assert_eq!(restored.slots, slots.slots);
        assert_eq!(restored.displayed, Some(0));
    }
}

/// Same label policy for course selection, settlement and Bn.
pub fn difficulty_label(level: &str, difficulty: f64) -> &'static str {
    let prefix = level.trim().split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("");
    match prefix.to_ascii_uppercase().as_str() {
        "EZ" => "EZ",
        "HD" => "HD",
        "IN" => "IN",
        "AT" => "AT",
        _ if difficulty < 8. => "EZ",
        _ if difficulty < 13. => "HD",
        _ if difficulty < 16.5 => "IN",
        _ => "AT",
    }
}

#[cfg(test)]
mod selection_regression_tests {
    use super::*;
    #[test]
    fn duplicates_do_not_replace_a_slot_or_advance_focus() {
        let mut selection = ChallengeSelection::<u32>::default();
        assert!(selection.select_unique(1, |a, b| a == b));
        assert!(!selection.select_unique(1, |a, b| a == b));
        assert_eq!(selection.focused, 1);
        assert!(selection.slots[1].is_none());
        assert!(selection.select_unique(2, |a, b| a == b));
        assert!(selection.select_unique(3, |a, b| a == b));
        assert!(selection.ready());
    }
}
