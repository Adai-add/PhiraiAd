//! Boundary scheduler independent of rendering. Resume rewind never replays a
//! consumed boundary. Explicit seeks/restarts reset this state deliberately.
use crate::chart_play::ChartPlaySettings;
pub const ROTATION_SECONDS: f64 = 0.8;
#[derive(Clone, Debug)]
struct Rotation {
    started: f64,
    boundary: f64,
    target: bool,
}
#[derive(Default, Debug)]
pub struct AutoFlipPlayback {
    pub flipped: bool,
    cursor: Option<f64>,
    rewind_until: Option<f64>,
    rotation: Option<Rotation>,
}
impl AutoFlipPlayback {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn sync(&mut self, settings: &ChartPlaySettings, time: f64, enabled: bool) {
        self.flipped = enabled && settings.flipped_at(time);
        self.cursor = Some(time);
        self.rewind_until = None;
        self.rotation = None;
    }
    /// Manual resume also rewinds playback; do not replay already crossed boundaries.
    pub fn resume_at(&mut self, time: f64) {
        self.cursor = Some(time);
        self.rewind_until = Some(time);
    }
    pub fn rotating(&self) -> bool {
        self.rotation.is_some()
    }
    pub fn angle(&self, now: f64) -> f32 {
        let base = if self.flipped { std::f64::consts::PI } else { 0. };
        let progress = self
            .rotation
            .as_ref()
            .map_or(0., |r| ((now - r.started) / ROTATION_SECONDS).clamp(0., 1.));
        (base + std::f64::consts::PI * progress * progress * (3. - 2. * progress)) as f32
    }
    pub fn begin_due(&mut self, settings: &ChartPlaySettings, time: f64, now: f64) -> Option<f64> {
        if self.rotating() || !time.is_finite() {
            return None;
        }
        if let Some(until) = self.rewind_until {
            if time < until {
                return None;
            }
            self.cursor = Some(until);
            self.rewind_until = None;
        }
        let last = self.cursor.unwrap_or(-1e-6);
        if time < last - 1e-6 {
            self.sync(settings, time, true);
            return None;
        }
        let next = settings
            .auto_flip_intervals
            .iter()
            .flat_map(|r| [(r.start, true), (r.end, false)])
            .find(|(boundary, _)| *boundary > last && *boundary <= time);
        if let Some((boundary, target)) = next {
            self.rotation = Some(Rotation {
                started: now,
                boundary,
                target,
            });
            self.cursor = Some(boundary);
            Some(boundary)
        } else {
            self.cursor = Some(time);
            None
        }
    }
    pub fn finish(&mut self, now: f64) -> Option<f64> {
        let r = self.rotation.as_ref()?;
        if now - r.started < ROTATION_SECONDS {
            return None;
        }
        let r = self.rotation.take().unwrap();
        self.flipped = r.target;
        self.rewind_until = Some(r.boundary);
        Some(r.boundary)
    }
    /// A lifecycle pause must not cause an automatic resume while in background.
    pub fn interrupt(&mut self) {
        if let Some(r) = self.rotation.take() {
            self.flipped = r.target;
            self.rewind_until = Some(r.boundary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> ChartPlaySettings {
        let mut s = ChartPlaySettings::default();
        s.add(2., 4.);
        s.add(4.1, 5.);
        s
    }
    #[test]
    fn enters_and_exits_in_point_eight_real_seconds() {
        let s = settings();
        let mut f = AutoFlipPlayback::default();
        assert_eq!(f.begin_due(&s, 1.9, 10.), None);
        assert_eq!(f.begin_due(&s, 2.1, 11.), Some(2.));
        assert!(f.rotating());
        assert_eq!(f.angle(11.), 0.);
        assert!((f.angle(11.4) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!(f.finish(11.79), None);
        assert_eq!(f.finish(11.81), Some(2.));
        assert!(f.flipped);
        for t in [0., 0.5, 1., 1.9, 2.] {
            assert_eq!(f.begin_due(&s, t, 15.), None);
            assert!(f.flipped);
        }
        assert_eq!(f.begin_due(&s, 4., 16.), Some(4.));
        assert!((f.angle(16.4) - std::f32::consts::PI * 1.5).abs() < 1e-5);
        assert_eq!(f.finish(16.81), Some(4.));
        assert!(!f.flipped);
        assert_eq!(f.begin_due(&s, 4.11, 20.), Some(4.1));
    }
    #[test]
    fn seeks_retries_disable_and_lifecycle_interrupt() {
        let s = settings();
        let mut f = AutoFlipPlayback::default();
        f.sync(&s, 3., true);
        assert!(f.flipped);
        assert_eq!(f.begin_due(&s, 3.1, 0.), None);
        f.sync(&s, 1., true);
        assert!(!f.flipped);
        assert_eq!(f.begin_due(&s, 2., 1.), Some(2.));
        f.interrupt();
        assert!(!f.rotating());
        assert!(f.flipped);
        assert_eq!(f.finish(999.), None);
        assert_eq!(f.begin_due(&s, 1., 999.), None);
        f.reset();
        assert!(!f.flipped);
        assert_eq!(f.begin_due(&s, 2., 3.), Some(2.));
        f.sync(&s, 3., false);
        assert!(!f.flipped);
        assert!(!f.rotating());
    }
    #[test]
    fn manual_resume_after_interruption_does_not_replay_boundary() {
        let s = settings();
        let mut f = AutoFlipPlayback::default();
        f.begin_due(&s, 2., 0.);
        f.interrupt();
        f.sync(&s, 2., true);
        f.resume_at(2.);
        for t in [0., 1., 1.81, 2., 2.1] {
            assert_eq!(f.begin_due(&s, t, 10.), None);
            assert!(f.flipped);
        }
        assert_eq!(f.begin_due(&s, 4., 12.), Some(4.));
    }
    #[test]
    fn zero_start_and_dropped_frame_process_earliest_boundary_once() {
        let mut s = ChartPlaySettings::default();
        s.add(0., 0.1);
        s.add(0.2, 0.3);
        let mut f = AutoFlipPlayback::default();
        assert_eq!(f.begin_due(&s, 0.5, 1.), Some(0.));
        f.finish(2.);
        assert_eq!(f.begin_due(&s, 0.5, 3.), Some(0.1));
        f.finish(4.);
        assert_eq!(f.begin_due(&s, 0.5, 5.), Some(0.2));
    }
}
