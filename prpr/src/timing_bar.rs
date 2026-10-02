//! Cosmetic HUD fed by actual judgement events, never used to select outcomes.
use crate::{
    config::TimingBarConfig,
    core::NoteKind,
    judge::{JudgeReportEvent, Judgement, JudgementRangeProfile},
    ui::Ui,
};
use lyon::{math::point, path::Path};
use macroquad::prelude::*;
use std::collections::VecDeque;

/// A fixed real-time axis preserves physical length and timing positions across engines.
pub const EXTENT_SECONDS: f64 = 0.250;
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub difference: f64,
    pub judgement: Judgement,
    pub born_at: f64,
}
#[derive(Default)]
pub struct TimingBar {
    pub samples: VecDeque<Sample>,
    /// Perfect, early Good, late Good, Bad, Miss. Hold finals count only once.
    pub counts: [u32; 5],
    pub indicator: f64,
    last_real_time: Option<f64>,
    last_input_time: Option<f64>,
    last_three: VecDeque<Sample>,
}
impl TimingBar {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn record(&mut self, event: &JudgeReportEvent, kind: &NoteKind, config: &TimingBarConfig, real_time: f64) {
        self.expire(real_time, config);
        if let Ok(outcome) = event.judgement {
            if config.record_counts {
                let index = match outcome {
                    Judgement::Perfect => 0,
                    Judgement::Good if event.difference < 0. => 1,
                    Judgement::Good => 2,
                    Judgement::Bad => 3,
                    Judgement::Miss => 4,
                };
                self.counts[index] = self.counts[index].saturating_add(1);
            }
        }
        let outcome = match (&event.judgement, kind) {
            (Err(perfect), NoteKind::Hold { .. }) => Some(if *perfect { Judgement::Perfect } else { Judgement::Good }),
            (Ok(Judgement::Miss), _) => Some(Judgement::Miss),
            (Ok(j), NoteKind::Click) => Some(*j),
            _ => None,
        };
        if outcome.is_some() || (event.judgement.is_ok() && !matches!(kind, NoteKind::Hold { .. })) {
            self.last_input_time = Some(real_time);
        }
        if let Some(judgement) = outcome {
            if event.difference.is_finite() {
                let difference = if judgement == Judgement::Miss {
                    event.difference.clamp(-EXTENT_SECONDS, EXTENT_SECONDS)
                } else {
                    event.difference
                };
                let sample = Sample {
                    difference,
                    judgement,
                    born_at: real_time,
                };
                self.samples.push_back(sample);
                self.last_three.retain(|sample| real_time - sample.born_at < config.retention_seconds());
                self.last_three.push_back(sample);
                while self.last_three.len() > 3 {
                    self.last_three.pop_front();
                }
            }
        }
    }
    fn expire(&mut self, real_time: f64, config: &TimingBarConfig) {
        while self
            .samples
            .front()
            .is_some_and(|sample| real_time - sample.born_at >= config.retention_seconds())
        {
            self.samples.pop_front();
        }
    }
    pub fn target(&self) -> f64 {
        if self.last_three.is_empty() {
            return 0.;
        }
        (self.last_three.iter().map(|sample| sample.difference).sum::<f64>() / self.last_three.len() as f64).clamp(-EXTENT_SECONDS, EXTENT_SECONDS)
    }
    pub fn animate(&mut self, real_time: f64, config: &TimingBarConfig) {
        self.expire(real_time, config);
        let dt = self.last_real_time.map_or(0., |last| (real_time - last).clamp(0., 0.1));
        self.last_real_time = Some(real_time);
        let idle = self.last_input_time.is_none_or(|last| real_time - last >= 1.);
        let target = if idle { 0. } else { self.target() };
        let response = if idle { 0.25 } else { 0.09 };
        self.indicator += (target - self.indicator) * (1. - (-dt / response).exp());
    }
}
fn color(j: Judgement) -> Color {
    match j {
        Judgement::Perfect => GREEN,
        Judgement::Good => YELLOW,
        Judgement::Bad | Judgement::Miss => RED,
    }
}
/// The screen-space anchor is the centre of the bar's baseline, including for arcs.
pub fn position(config: &TimingBarConfig, half_height: f32, normalized: f32) -> Vec2 {
    let scale = if config.size.is_finite() {
        config.size.clamp(5., 500.) / 100.
    } else {
        1.
    };
    let anchor = vec2(if config.x.is_finite() { config.x } else { 0. }, if config.y.is_finite() { config.y * half_height } else { 0. });
    let n = normalized.clamp(-1., 1.);
    if config.curved {
        let angle = (90. - n * 60.).to_radians();
        anchor + vec2(angle.cos() * 0.40, (0.5 - angle.sin()) * 0.40) * scale
    } else {
        anchor + vec2(n * 0.35 * scale, 0.)
    }
}
fn segment(ui: &mut Ui, a: Vec2, b: Vec2, width: f32, color: Color) {
    let mut path = Path::builder();
    path.begin(point(a.x, a.y));
    path.line_to(point(b.x, b.y));
    path.end(false);
    ui.stroke_path(&path.build(), width, color);
}
pub fn render(ui: &mut Ui, config: &TimingBarConfig, state: &TimingBar, profile: &JudgementRangeProfile) {
    let scale = if config.size.is_finite() {
        config.size.clamp(5., 500.) / 100.
    } else {
        1.
    };
    let half_height = ui.top;
    let pos = |n| position(config, half_height, n);
    // Split at exact thresholds, including Phira's asymmetric late compensation.
    let mut boundaries = vec![
        -EXTENT_SECONDS,
        -profile.tap_outer.early,
        -profile.tap_good.early,
        -profile.tap_perfect.early,
        0.,
        profile.tap_perfect.late,
        profile.tap_good.late,
        profile.tap_outer.late,
        EXTENT_SECONDS,
    ];
    for b in &mut boundaries {
        *b = b.clamp(-EXTENT_SECONDS, EXTENT_SECONDS);
    }
    boundaries.sort_by(f64::total_cmp);
    boundaries.dedup();
    for pair in boundaries.windows(2) {
        let middle = (pair[0] + pair[1]) / 2.;
        let early = middle < 0.;
        let abs = middle.abs();
        let perfect = if early { profile.tap_perfect.early } else { profile.tap_perfect.late };
        let good = if early { profile.tap_good.early } else { profile.tap_good.late };
        let c = color(if abs <= perfect {
            Judgement::Perfect
        } else if abs <= good {
            Judgement::Good
        } else {
            Judgement::Bad
        });
        let n0 = (pair[0] / EXTENT_SECONDS) as f32;
        let n1 = (pair[1] / EXTENT_SECONDS) as f32;
        let pieces = if config.curved { ((n1 - n0) * 48.).ceil().max(1.) as usize } else { 1 };
        for i in 0..pieces {
            let a = n0 + (n1 - n0) * i as f32 / pieces as f32;
            let b = n0 + (n1 - n0) * (i + 1) as f32 / pieces as f32;
            segment(ui, pos(a), pos(b), 0.018 * scale, c);
        }
    }
    let centre = pos(0.);
    segment(ui, centre - vec2(0., 0.014 * scale), centre + vec2(0., 0.014 * scale), 0.003 * scale, WHITE);
    let now = state.last_real_time.unwrap_or(0.);
    for sample in &state.samples {
        let p = pos((sample.difference / EXTENT_SECONDS) as f32);
        let c = Color {
            a: (1. - ((now - sample.born_at).max(0.) / config.retention_seconds()) as f32).clamp(0., 1.),
            ..color(sample.judgement)
        };
        segment(ui, p - vec2(0., 0.045 * scale), p - vec2(0., 0.026 * scale), 0.006 * scale, c);
    }
    let p = pos((state.indicator / EXTENT_SECONDS) as f32) + vec2(0., 0.024 * scale);
    let mut triangle = Path::builder();
    triangle.begin(point(p.x, p.y));
    triangle.line_to(point(p.x - 0.012 * scale, p.y + 0.018 * scale));
    triangle.line_to(point(p.x + 0.012 * scale, p.y + 0.018 * scale));
    triangle.end(true);
    ui.fill_path(triangle.build().iter(), WHITE);
    if config.record_counts {
        let base = position(config, ui.top, 0.) + vec2(0., if config.curved { 0.23 * scale } else { 0.07 * scale });
        // Fixed centres, independent of string width: B, early G, P, late G, M.
        for (i, (index, c)) in [(3, RED), (1, YELLOW), (0, GREEN), (2, YELLOW), (4, RED)].into_iter().enumerate() {
            ui.text(state.counts[index].to_string())
                .pos(base.x + (i as f32 - 2.) * 0.145 * scale, base.y)
                .anchor(0.5, 0.5)
                .size(0.60 * scale)
                .color(c)
                .draw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(j: Result<Judgement, bool>, diff: f64) -> JudgeReportEvent {
        JudgeReportEvent {
            time: 1.,
            line_id: 0,
            note_id: 0,
            judgement: j,
            difference: diff,
        }
    }
    #[test]
    fn heads_misses_and_finals_keep_one_final_count() {
        let mut state = TimingBar::default();
        let cfg = TimingBarConfig::default();
        let hold = NoteKind::Hold {
            end_time: 3.,
            end_height: 0.,
        };
        state.record(&event(Err(false), -0.1), &hold, &cfg, 0.);
        assert_eq!(state.samples.len(), 1);
        assert_eq!(state.counts, [0; 5]);
        state.record(&event(Ok(Judgement::Miss), 2.), &hold, &cfg, 0.2);
        assert_eq!(state.samples.len(), 2);
        assert_eq!(state.counts, [0, 0, 0, 0, 1]);
        assert_eq!(state.samples.back().unwrap().difference, EXTENT_SECONDS);
        assert!(matches!(state.samples.back().unwrap().judgement, Judgement::Miss));
        for kind in [NoteKind::Drag, NoteKind::Flick] {
            state.record(&event(Ok(Judgement::Miss), 0.17), &kind, &cfg, 0.3);
        }
        assert_eq!(state.samples.len(), 4);
        assert_eq!(state.counts[4], 3);
    }
    #[test]
    fn retention_uses_seconds_not_count_and_keeps_whole_run_counts() {
        let cfg = TimingBarConfig {
            record_seconds: 0.5,
            ..Default::default()
        };
        let mut state = TimingBar::default();
        for n in 0..80 {
            state.record(&event(Ok(Judgement::Perfect), 0.03), &NoteKind::Click, &cfg, n as f64 * 0.001);
        }
        assert_eq!(state.samples.len(), 80);
        state.animate(0.52, &cfg);
        assert_eq!(state.samples.len(), 59);
        state.animate(0.60, &cfg);
        assert!(state.samples.is_empty());
        assert_eq!(state.counts[0], 80);
    }
    #[test]
    fn average_survives_short_retention_until_one_second_idle_then_returns_smoothly() {
        let cfg = TimingBarConfig {
            record_seconds: 0.5,
            ..Default::default()
        };
        let mut state = TimingBar::default();
        for diff in [-0.03, 0.09, 0.12] {
            state.record(&event(Ok(Judgement::Good), diff), &NoteKind::Click, &cfg, 0.);
        }
        assert!((state.target() - 0.06).abs() < 1e-9);
        state.animate(0., &cfg);
        for n in 1..10 {
            state.animate(n as f64 / 10., &cfg);
        }
        assert!(state.samples.is_empty());
        assert!(state.indicator > 0.055);
        let before = state.indicator;
        state.animate(1., &cfg);
        assert!(state.indicator > 0. && state.indicator < before);
        for n in 11..31 {
            state.animate(n as f64 / 10., &cfg);
        }
        assert!(state.indicator.abs() < 0.0001);
        state.record(&event(Ok(Judgement::Perfect), -0.03), &NoteKind::Click, &cfg, 3.1);
        state.animate(3.1, &cfg);
        assert_eq!(state.target(), -0.03);
        assert!(state.indicator < -0.001);
    }
    #[test]
    fn defaults_migrate_old_count_config_and_times_are_bounded() {
        let old: TimingBarConfig = serde_json::from_str(r#"{"history":50,"x":0.2,"enabled":true}"#).unwrap();
        assert_eq!(old.retention_seconds(), 3.);
        assert_eq!(old.x, 0.2);
        assert!(old.enabled);
        for (raw, expected) in [(0., 0.5), (0.5, 0.5), (10., 10.), (20., 10.), (f32::NAN, 3.)] {
            assert_eq!(
                TimingBarConfig {
                    record_seconds: raw,
                    ..Default::default()
                }
                .retention_seconds(),
                expected
            );
        }
    }
    #[test]
    fn directions_and_all_note_types_use_final_counts_only() {
        let mut state = TimingBar::default();
        let cfg = TimingBarConfig::default();
        for (kind, diff) in [
            (NoteKind::Click, -0.08),
            (NoteKind::Click, 0.09),
            (NoteKind::Drag, -0.08),
            (NoteKind::Flick, 0.09),
        ] {
            state.record(&event(Ok(Judgement::Good), diff), &kind, &cfg, 0.);
        }
        assert_eq!(state.samples.len(), 2);
        assert_eq!(state.counts, [0, 2, 2, 0, 0]);
        state.clear();
        assert!(state.samples.is_empty());
        assert_eq!(state.target(), 0.);
        assert_eq!(state.counts, [0; 5]);
    }
}
