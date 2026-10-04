//! Editable ordered timing bands. Positive milliseconds mean early input.
use crate::judge::Judgement;
use serde::{Deserialize, Serialize};
pub const MAX_SCHEMES: usize = 10;
const LABELS: [&str; 9] = [
    "bad",
    "early normal",
    "early good",
    "early cool",
    "perfect",
    "late cool",
    "late good",
    "late normal",
    "miss",
];
const BOUNDARIES: [f64; 9] = [180., 165., 150., 115., 80., -80., -115., -150., -165.];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Band {
    pub label: String,
    pub contribution: f64,
    pub effect_enabled: bool,
    pub color: [u8; 4],
    pub effect_size: f32,
}
impl Default for Band {
    fn default() -> Self {
        Self {
            label: "perfect".into(),
            contribution: 1.,
            effect_enabled: true,
            color: [255, 236, 159, 225],
            effect_size: 1.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Scheme {
    pub name: String,
    pub counts_local_score: bool,
    pub boundaries_ms: Vec<f64>,
    pub bands: Vec<Band>,
}
impl Default for Scheme {
    fn default() -> Self {
        Self::new(5)
    }
}
impl Scheme {
    pub fn indices(count: usize) -> &'static [usize] {
        match count {
            9 => &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            7 => &[0, 2, 3, 4, 5, 6, 8],
            _ => &[0, 2, 4, 6, 8],
        }
    }
    pub fn new(count: usize) -> Self {
        let indices = Self::indices(count);
        // Default 5/7-band layouts omit unused inner subdivisions while
        // retaining the supplied central perfect boundaries.
        let boundary_indices: &[usize] = match count {
            9 => &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            7 => &[0, 1, 3, 4, 5, 6, 8],
            _ => &[0, 1, 4, 5, 8],
        };
        Self {
            name: "新方案".into(),
            counts_local_score: false,
            boundaries_ms: boundary_indices.iter().map(|&i| BOUNDARIES[i]).collect(),
            bands: indices
                .iter()
                .map(|&i| {
                    let label = LABELS[i];
                    let (contribution, color) = if i == 4 {
                        (1., [255, 236, 159, 225])
                    } else if label.contains("cool") {
                        (0.75, [170, 245, 180, 255])
                    } else if label.contains("good") {
                        (0.65, [180, 225, 255, 235])
                    } else if label.contains("normal") {
                        (0.4, [255, 175, 175, 255])
                    } else {
                        (0., [255, 100, 100, 255])
                    };
                    Band {
                        label: label.into(),
                        contribution,
                        color,
                        effect_enabled: i != 0 && i != 8,
                        effect_size: 1.,
                    }
                })
                .collect(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if ![5, 7, 9].contains(&self.bands.len()) || self.boundaries_ms.len() != self.bands.len() {
            return Err("档位与边界数量不匹配".into());
        }
        if self.boundaries_ms.iter().any(|x| !x.is_finite()) || self.boundaries_ms.windows(2).any(|p| p[0] <= p[1]) {
            return Err("时间边界必须是有限数值，并严格逐项递减".into());
        }
        if self
            .bands
            .iter()
            .any(|b| !b.contribution.is_finite() || b.contribution < 0. || !b.effect_size.is_finite() || b.effect_size <= 0.)
        {
            return Err("ACC贡献不能为负，特效大小必须大于0".into());
        }
        Ok(())
    }
    /// First upper boundary is inclusive; exact inner boundaries enter the later band.
    pub fn classify(&self, early_ms: f64) -> Option<usize> {
        if !early_ms.is_finite() || early_ms > self.boundaries_ms[0] {
            return None;
        }
        Some(self.boundaries_ms.iter().rposition(|b| early_ms <= *b).unwrap_or(0))
    }
    pub fn outcome(&self, stage: usize) -> Judgement {
        if stage == 0 {
            Judgement::Bad
        } else if stage == self.bands.len() - 1 {
            Judgement::Miss
        } else if stage == self.bands.len() / 2 {
            Judgement::Perfect
        } else {
            Judgement::Good
        }
    }
    pub fn change_count(&mut self, count: usize) {
        if count == self.bands.len() {
            return;
        }
        let mut next = Self::new(count);
        next.name = self.name.clone();
        next.counts_local_score = self.counts_local_score;
        for band in &mut next.bands {
            if let Some(old) = self.bands.iter().find(|x| x.label == band.label) {
                *band = old.clone();
            }
        }
        if self.validate().is_ok() && self.boundaries_ms != Self::new(self.bands.len()).boundaries_ms {
            let old_mid = self.bands.len() / 2;
            let new_mid = next.bands.len() / 2;
            let early_outer = self.boundaries_ms[0];
            let early_perfect = self.boundaries_ms[old_mid];
            let late_perfect = self.boundaries_ms[old_mid + 1];
            let late_miss = *self.boundaries_ms.last().unwrap();
            for i in 0..=new_mid {
                next.boundaries_ms[i] = early_outer + (early_perfect - early_outer) * i as f64 / new_mid as f64;
            }
            let wings = next.bands.len() - new_mid - 2;
            for i in 0..=wings {
                next.boundaries_ms[new_mid + 1 + i] = late_perfect + (late_miss - late_perfect) * i as f64 / wings as f64;
            }
        }
        *self = next;
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomJudgementConfig {
    pub current: Scheme,
    pub schemes: Vec<Scheme>,
    pub selected: Option<usize>,
}
impl CustomJudgementConfig {
    pub fn effective(&self) -> &Scheme {
        if self.current.validate().is_ok() {
            &self.current
        } else {
            static FALLBACK: std::sync::LazyLock<Scheme> = std::sync::LazyLock::new(Scheme::default);
            &FALLBACK
        }
    }
    pub fn save(&mut self, as_new: bool) -> Result<(), String> {
        self.current.validate()?;
        if !as_new {
            if let Some(i) = self.selected.filter(|i| *i < self.schemes.len()) {
                self.schemes[i] = self.current.clone();
                return Ok(());
            }
        }
        if self.schemes.len() >= MAX_SCHEMES {
            return Err("最多保存10个方案，请覆盖或删除已有方案".into());
        }
        self.schemes.push(self.current.clone());
        self.selected = Some(self.schemes.len() - 1);
        Ok(())
    }
    pub fn select(&mut self, index: usize) {
        if let Some(s) = self.schemes.get(index) {
            self.current = s.clone();
            self.selected = Some(index);
        }
    }
    pub fn delete(&mut self, index: usize) {
        if index >= self.schemes.len() {
            return;
        }
        self.schemes.remove(index);
        self.selected = match self.selected {
            Some(i) if i == index => None,
            Some(i) if i > index => Some(i - 1),
            other => other,
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_score_permission_is_opt_in_and_survives_scheme_changes() {
        use crate::config::{Config, JudgementMode};
        let mut config = Config::default();
        config.judgement_mode = JudgementMode::Custom;
        assert!(!config.saves_run_record());
        let old: Scheme = serde_json::from_str(r#"{"name":"旧方案"}"#).unwrap();
        assert!(!old.counts_local_score);
        config.custom_judgement.current.counts_local_score = true;
        config.custom_judgement.current.change_count(9);
        config.custom_judgement.save(true).unwrap();
        config.custom_judgement.current = Scheme::default();
        config.custom_judgement.select(0);
        assert!(config.saves_run_record());
        assert!(config.blocks_score_upload());
        let restored: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.saves_run_record());
        config.judgement_mode = JudgementMode::Phira;
        config.custom_judgement.current.counts_local_score = false;
        assert!(config.saves_run_record());
    }
    #[test]
    fn old_config_migrates_and_custom_config_round_trips() {
        let mut c: crate::config::Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c.custom_judgement.current, Scheme::new(5));
        c.judgement_mode = crate::config::JudgementMode::Custom;
        c.custom_judgement.current = Scheme::new(9);
        c.custom_judgement.save(true).unwrap();
        let copy: crate::config::Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(copy.judgement_mode, crate::config::JudgementMode::Custom);
        assert_eq!(copy.custom_judgement, c.custom_judgement);
    }
    #[test]
    fn expanding_edited_windows_preserves_outer_and_perfect_bounds() {
        let mut s = Scheme::new(5);
        s.boundaries_ms = vec![320., 160., 20., -35., -70.];
        s.change_count(9);
        assert!(s.validate().is_ok());
        assert_eq!(s.boundaries_ms[0], 320.);
        assert_eq!(s.boundaries_ms[4], 20.);
        assert_eq!(s.boundaries_ms[5], -35.);
        assert_eq!(s.boundaries_ms[8], -70.);
    }
    #[test]
    fn defaults_and_every_boundary() {
        for n in [5, 7, 9] {
            let s = Scheme::new(n);
            assert!(s.validate().is_ok());
            assert_eq!(s.classify(181.), None);
            for (i, &b) in s.boundaries_ms.iter().enumerate() {
                assert_eq!(s.classify(b), Some(i));
                if i > 0 {
                    assert_eq!(s.classify(b + 0.001), Some(i - 1));
                }
            }
            assert_eq!(s.classify(-99999.), Some(n - 1));
            assert_eq!(s.classify(0.), Some(n / 2));
        }
        assert_eq!(Scheme::new(9).boundaries_ms, BOUNDARIES);
    }
    #[test]
    fn arbitrary_signs_and_strict_order() {
        let mut s = Scheme::new(5);
        s.boundaries_ms = vec![-1., -2., -3., -4., -5.];
        assert!(s.validate().is_ok());
        assert_eq!(s.classify(-3.5), Some(2));
        s.boundaries_ms[1] = -1.;
        assert!(s.validate().is_err());
        s.boundaries_ms[1] = f64::NAN;
        assert!(s.validate().is_err());
    }
    #[test]
    fn ten_schemes_round_trip_select_rename_delete() {
        let mut c = CustomJudgementConfig::default();
        for i in 0..10 {
            c.current.name = format!("方案{i}");
            c.save(true).unwrap();
        }
        assert!(c.save(true).is_err());
        c.select(3);
        c.current.name = "重命名".into();
        c.save(false).unwrap();
        assert_eq!(c.schemes[3].name, "重命名");
        c.delete(0);
        assert_eq!(c.selected, Some(2));
        c.delete(2);
        assert_eq!(c.selected, None);
        assert_eq!(c.schemes.len(), 8);
        let copy: CustomJudgementConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, copy);
    }
}
