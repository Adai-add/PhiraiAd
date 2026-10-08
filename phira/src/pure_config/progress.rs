//! Progress measures completed work; total percentage is a cost estimate.
use anyhow::{ensure, Result};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Read, Adapt, Geometry, Layout, Avoid, Fade, Finalize, Copy, Write, Commit }
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "读取谱面", Self::Adapt => "转换格式", Self::Geometry => "准备动画",
            Self::Layout => "音符布局", Self::Avoid => "音符避让", Self::Fade => "判定线渐隐",
            Self::Finalize => "整理谱面", Self::Copy => "复制资源", Self::Write => "保存谱面", Self::Commit => "提交谱面",
        }
    }
}
struct State {
    weights: [f64; 10], fractions: [f64; 10], stage: Stage,
    done: u64, total: u64, high_water: f64, committed: bool,
}
impl Default for State {
    fn default() -> Self { Self { weights: [1.; 10], fractions: [0.; 10], stage: Stage::Read, done: 0, total: 0, high_water: 0., committed: false } }
}
#[derive(Clone)]
pub struct Progress {
    pub cancelled: Arc<AtomicBool>, state: Arc<Mutex<State>>, started: Instant,
}
impl Default for Progress {
    fn default() -> Self { Self { cancelled: Arc::new(AtomicBool::new(false)), state: Arc::new(Mutex::new(State::default())), started: Instant::now() } }
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub stage: &'static str, pub overall: f32, pub fraction: f32,
    pub done: u64, pub total: u64, pub elapsed: f32,
}
impl Progress {
    pub fn check(&self) -> Result<()> { ensure!(!self.cancelled.load(Ordering::Relaxed), "已取消生成"); Ok(()) }
    pub fn plan(&self, source_bytes: usize, events: usize, notes: usize, fade: bool) {
        let mut s = self.state.lock().unwrap();
        let e = events as f64; let n = notes as f64;
        let copy = s.weights[Stage::Copy as usize];
        s.weights = [source_bytes as f64 / 200_000. + 1., e / 500. + 1., e / 1000. + 1., n / 50. + 1., n / 50. + 1.,
            if fade { e / 200. + n + 1. } else { 0. }, n / 100. + 1., copy, source_bytes as f64 / 100_000. + 1., 1.];
    }
    pub fn weight(&self, stage: Stage, weight: f64) { self.state.lock().unwrap().weights[stage as usize] = if weight.is_finite() { weight.max(0.) } else { 0. }; }
    pub fn update(&self, stage: Stage, done: u64, total: u64) {
        let mut s = self.state.lock().unwrap();
        s.stage = stage; s.done = done.min(total); s.total = total;
        let fraction = if total == 0 { 0. } else { s.done as f64 / total as f64 };
        s.fractions[stage as usize] = s.fractions[stage as usize].max(fraction);
    }
    pub fn finish(&self, stage: Stage) {
        let mut s = self.state.lock().unwrap();
        if s.stage == stage && s.total > 0 { s.done = s.total; } else { s.done = 1; s.total = 1; }
        s.stage = stage; s.fractions[stage as usize] = 1.;
    }
    pub fn committed(&self) { let mut s = self.state.lock().unwrap(); s.committed = true; s.stage = Stage::Commit; s.done = 1; s.total = 1; s.fractions[9] = 1.; }
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.state.lock().unwrap();
        let total = s.weights.iter().sum::<f64>().max(1.);
        let current = s.weights.iter().zip(s.fractions).map(|(w, f)| w * f).sum::<f64>() / total;
        s.high_water = s.high_water.max(current.min(0.999));
        Snapshot { stage: s.stage.label(), overall: if s.committed { 1. } else { s.high_water as f32 },
            fraction: s.fractions[s.stage as usize] as f32, done: s.done, total: s.total, elapsed: self.started.elapsed().as_secs_f32() }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_does_not_finish_before_publication_or_go_backwards() {
        let p = Progress::default(); p.plan(1000, 10, 5, true);
        p.finish(Stage::Read); let a = p.snapshot().overall;
        p.weight(Stage::Fade, 10000.); assert!(p.snapshot().overall >= a);
        p.update(Stage::Fade, 42, 100); assert_eq!(p.snapshot().done, 42);
        p.finish(Stage::Fade); assert_eq!(p.snapshot().done, 100); assert_eq!(p.snapshot().total, 100);
        for stage in [Stage::Read, Stage::Adapt, Stage::Geometry, Stage::Layout, Stage::Avoid, Stage::Fade, Stage::Finalize, Stage::Copy, Stage::Write, Stage::Commit] { p.finish(stage); }
        assert!(p.snapshot().overall < 1.); p.committed(); assert_eq!(p.snapshot().overall, 1.);
    }
}
