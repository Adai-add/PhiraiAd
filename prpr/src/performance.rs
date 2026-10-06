//! Opt-in, bounded single-chart profiling. CPU spans are inclusive; nested spans must not be summed.
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{ Arc, atomic::{ AtomicBool, AtomicU64, Ordering } },
    time::{ Duration, Instant },
};
const MAX_SECONDS: usize = 7200;
const MAX_SLOW: usize = 32;
const HIST_BINS: usize = 20001; // 0.1ms bins; last bin means >= 2000ms.
pub type Sink = Box<dyn FnOnce(Report)>;
static GENERATION: AtomicU64 = AtomicU64::new(1);
static AUDIO_ID: AtomicU64 = AtomicU64::new(0);
static AUDIO_CALLS: AtomicU64 = AtomicU64::new(0);
static AUDIO_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_MAX_NS: AtomicU64 = AtomicU64::new(0);
static AUDIO_OVERRUNS: AtomicU64 = AtomicU64::new(0);
#[derive(Default, Serialize)]
pub struct AudioStats {
    pub corrected_callback_calls: u64,
    pub corrected_callback_total_ms: f64,
    pub corrected_callback_max_ms: f64,
    pub corrected_callbacks_over_buffer_duration: u64,
}
pub struct AudioSpan {
    started: Option<Instant>,
    id: u64,
    budget_ns: u64,
}
pub fn audio_span(frames: usize, sample_rate: u32) -> AudioSpan {
    let id = AUDIO_ID.load(Ordering::Relaxed);
    AudioSpan {
        started: (id != 0).then(Instant::now),
        id,
        budget_ns: if sample_rate > 0 {
            ((frames as u64) * 1_000_000_000) / (sample_rate as u64)
        } else {
            u64::MAX
        },
    }
}
impl Drop for AudioSpan {
    fn drop(&mut self) {
        if let Some(t) = self.started {
            if AUDIO_ID.load(Ordering::Relaxed) == self.id {
                let n = t
                    .elapsed()
                    .as_nanos()
                    .min(u64::MAX as u128) as u64;
                AUDIO_CALLS.fetch_add(1, Ordering::Relaxed);
                AUDIO_NS.fetch_add(n, Ordering::Relaxed);
                AUDIO_MAX_NS.fetch_max(n, Ordering::Relaxed);
                if n > self.budget_ns {
                    AUDIO_OVERRUNS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}
#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub calls: u64,
    pub total_ms: f64,
    pub max_ms: f64,
}
impl Stats {
    fn add(&mut self, ms: f64) {
        self.calls += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }
}
#[derive(Default, Serialize)]
pub struct FrameStats {
    pub frames: u64,
    pub total_ms: f64,
    pub max_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub above_16_67_ms: u64,
    pub above_33_33_ms: u64,
    pub above_50_ms: u64,
    pub above_100_ms: u64,
    #[serde(skip)] hist: Vec<u64>,
}
impl FrameStats {
    fn add(&mut self, ms: f64) {
        if self.hist.is_empty() {
            self.hist.resize(HIST_BINS, 0);
        }
        self.frames += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        self.above_16_67_ms += u64::from(ms > 1000.0 / 60.0);
        self.above_33_33_ms += u64::from(ms > 1000.0 / 30.0);
        self.above_50_ms += u64::from(ms > 50.0);
        self.above_100_ms += u64::from(ms > 100.0);
        self.hist[((ms * 10.0).round() as usize).min(HIST_BINS - 1)] += 1;
    }
    fn quantile(&self, p: f64) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        let rank = ((self.frames as f64) * p).ceil() as u64;
        let mut n = 0;
        for (i, v) in self.hist.iter().enumerate() {
            n += v;
            if n >= rank {
                return (i as f64) / 10.0;
            }
        }
        0.0
    }
    fn finalize(&mut self) {
        self.p50_ms = self.quantile(0.5);
        self.p95_ms = self.quantile(0.95);
        self.p99_ms = self.quantile(0.99);
    }
}
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub song_seconds: f64,
    pub music_seconds: f64,
    pub sync_error_ms: f64,
    pub sync_correction_ms: f64,
    pub paused: bool,
    pub phase: String,
}
#[derive(Clone, Serialize)]
pub struct SlowFrame {
    pub elapsed_seconds: f64,
    pub frame_ms: f64,
    pub snapshot: Snapshot,
    pub cpu_ms: BTreeMap<String, f64>,
    pub counters: BTreeMap<String, u64>,
}
#[derive(Serialize)]
pub struct Second {
    pub elapsed_second: u64,
    pub phase: String,
    pub frames: u64,
    pub total_frame_ms: f64,
    pub max_frame_ms: f64,
    pub cpu_ms: BTreeMap<String, f64>,
    pub counters: BTreeMap<String, u64>,
    pub rss_bytes: Option<u64>,
    pub sync_max_abs_ms: f64,
}
#[derive(Serialize)]
pub struct Report {
    pub schema: u32,
    pub started_unix_ms: u128,
    pub reason: String,
    pub elapsed_seconds: f64,
    pub metadata: serde_json::Value,
    pub cpu_inclusive: BTreeMap<String, Stats>,
    pub audio: AudioStats,
    pub gpu_sampled: BTreeMap<String, Stats>,
    pub gpu_supported: Option<bool>,
    pub phases: BTreeMap<String, FrameStats>,
    pub seconds: Vec<Second>,
    pub slowest_playing_frames: Vec<SlowFrame>,
    pub rss_peak_sampled_bytes: Option<u64>,
    pub process_cpu_seconds: Option<f64>,
    pub timeline_truncated: bool,
    pub notes: Vec<String>,
}
struct Session {
    id: u128,
    started: Instant,
    report: Report,
    sink: Option<Sink>,
    frame_start: Option<Instant>,
    frame_phase: String,
    snapshot: Snapshot,
    frame_cpu: BTreeMap<String, f64>,
    frame_counters: BTreeMap<String, u64>,
    finish: Option<String>,
    alive: Arc<AtomicBool>,
    rss: Arc<AtomicU64>,
    cpu_start: Option<f64>,
}
pub fn active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}
pub fn start(metadata: serde_json::Value, sink: Sink) -> bool {
    if active() {
        return false;
    }
    let unix_ms = std::time::SystemTime
        ::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = GENERATION.fetch_add(1, Ordering::Relaxed) as u128;
    let alive = Arc::new(AtomicBool::new(true));
    let rss = Arc::new(AtomicU64::new(0));
    let a = alive.clone();
    let m = rss.clone();
    std::thread::spawn(move || {
        while a.load(Ordering::Relaxed) {
            if let Some(n) = memory_rss() {
                m.store(n, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    for counter in [&AUDIO_CALLS, &AUDIO_NS, &AUDIO_MAX_NS, &AUDIO_OVERRUNS] {
        counter.store(0, Ordering::Relaxed);
    }
    AUDIO_ID.store((id as u64).max(1), Ordering::Relaxed);
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            id,
            started: Instant::now(),
            sink: Some(sink),
            frame_start: None,
            frame_phase: "loading".into(),
            snapshot: Snapshot { phase: "loading".into(), ..Default::default() },
            frame_cpu: BTreeMap::new(),
            frame_counters: BTreeMap::new(),
            finish: None,
            alive,
            rss,
            cpu_start: process_cpu(),
            report: Report {
                audio: AudioStats::default(),
                schema: 1,
                started_unix_ms: unix_ms,
                reason: String::new(),
                elapsed_seconds: 0.0,
                metadata,
                cpu_inclusive: BTreeMap::new(),
                gpu_sampled: BTreeMap::new(),
                gpu_supported: None,
                phases: BTreeMap::new(),
                seconds: Vec::new(),
                slowest_playing_frames: Vec::new(),
                rss_peak_sampled_bytes: None,
                process_cpu_seconds: None,
                timeline_truncated: false,
                notes: vec![
                    "CPU spans are inclusive and nested: do not sum parent and child spans.".into(),
                    "Loading spans measure wall time, including asynchronous waits.".into(),
                    "Frame interval includes submission/pacing; next_frame is not a pure GPU timer.".into(),
                    "GPU timings are sampled noise passes only; unsupported or invalid samples are omitted.".into(),
                    "Frame percentiles use 0.1ms histogram bins, capped at >=2000ms.".into(),
                    "Process CPU/RSS include background work and profiler overhead; RSS is sampled once per second.".into(),
                    "Audio position difference is software clock drift, not measured speaker/display latency.".into(),
                    "Corrected audio callback timing is buffer wall time, not a hardware underrun measurement; the original sasa renderer is external and not instrumented.".into()
                ],
            },
        });
    });
    true
}
pub fn metadata(key: &str, value: serde_json::Value) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if let Some(o) = s.report.metadata.as_object_mut() {
                o.insert(key.into(), value);
            }
        }
    });
}
pub fn milestone(key: &str) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if let Some(o) = s.report.metadata.as_object_mut() {
                o.insert(key.into(), serde_json::json!(s.started.elapsed().as_secs_f64()));
            }
        }
    });
}
pub struct Span {
    start: Option<Instant>,
    label: &'static str,
    id: u128,
}
pub fn span(label: &'static str) -> Span {
    let id = SESSION.with(|s|
        s
            .borrow()
            .as_ref()
            .map(|s| s.id)
    );
    Span { start: id.map(|_| Instant::now()), label, id: id.unwrap_or(0) }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            record(self.id, self.label, start.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
fn record(id: u128, label: &str, ms: f64) {
    SESSION.with(|s| {
        if
            let Some(s) = s
                .borrow_mut()
                .as_mut()
                .filter(|s| s.id == id)
        {
            if let Some(v) = s.report.cpu_inclusive.get_mut(label) {
                v.add(ms);
            } else {
                let mut v = Stats::default();
                v.add(ms);
                s.report.cpu_inclusive.insert(label.into(), v);
            }
            if let Some(v) = s.frame_cpu.get_mut(label) {
                *v += ms;
            } else {
                s.frame_cpu.insert(label.into(), ms);
            }
        }
    });
}
pub fn external_cpu(label: &str, ns: u64) {
    let id = SESSION.with(|s|
        s
            .borrow()
            .as_ref()
            .map(|s| s.id)
    );
    if let Some(id) = id {
        record(id, label, (ns as f64) / 1e6);
    }
}
pub fn gpu(label: &str, ns: u64) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if let Some(v) = s.report.gpu_sampled.get_mut(label) {
                v.add((ns as f64) / 1e6);
            } else {
                let mut v = Stats::default();
                v.add((ns as f64) / 1e6);
                s.report.gpu_sampled.insert(label.into(), v);
            }
        }
    });
}
pub fn gpu_supported(supported: bool) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.report.gpu_supported = Some(s.report.gpu_supported.unwrap_or(false) || supported);
        }
    });
}
pub fn count(label: &str, n: u64) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if let Some(v) = s.frame_counters.get_mut(label) {
                *v += n;
            } else {
                s.frame_counters.insert(label.into(), n);
            }
        }
    });
}
pub fn observe(song: f64, music: f64, paused: bool, phase: &str) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.snapshot = Snapshot {
                song_seconds: if song.is_finite() {
                    song
                } else {
                    0.0
                },
                music_seconds: if music.is_finite() {
                    music
                } else {
                    0.0
                },
                sync_error_ms: if (music - song).is_finite() {
                    (music - song) * 1000.0
                } else {
                    0.0
                },
                sync_correction_ms: 0.0,
                paused,
                phase: phase.into(),
            };
        }
    });
}
pub fn sync(before: f64, after: f64, music: f64) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if before.is_finite() && after.is_finite() && music.is_finite() {
                s.snapshot.song_seconds = after;
                s.snapshot.music_seconds = music;
                s.snapshot.sync_error_ms = (music - after) * 1000.0;
                s.snapshot.sync_correction_ms = (after - before) * 1000.0;
            }
        }
    });
}
pub fn frame_begin() {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.frame_start = Some(Instant::now());
            s.frame_phase = s.snapshot.phase.clone();
        }
    });
}
pub fn request_finish(reason: &str) {
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if s.finish.is_none() {
                s.finish = Some(reason.into());
            }
        }
    });
}
pub fn frame_end() {
    let completed = SESSION.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(s) = slot.as_mut() else {
            return None;
        };
        if let Some(start) = s.frame_start.take() {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            // Classify by state at frame start; a transition frame cannot become an ordinary playing frame.
            let phase = if s.frame_phase != s.snapshot.phase {
                "transition".to_owned()
            } else {
                s.frame_phase.clone()
            };
            s.report.phases.entry(phase.clone()).or_default().add(ms);
            let elapsed = s.started.elapsed().as_secs_f64();
            let second = elapsed.floor() as u64;
            let rss = s.rss.load(Ordering::Relaxed);
            if rss > 0 {
                s.report.rss_peak_sampled_bytes = Some(
                    s.report.rss_peak_sampled_bytes.unwrap_or(0).max(rss)
                );
            }
            if
                s.report.seconds
                    .last()
                    .is_none_or(|x| (x.elapsed_second != second || x.phase != phase))
            {
                if s.report.seconds.len() < MAX_SECONDS {
                    s.report.seconds.push(Second {
                        elapsed_second: second,
                        phase: phase.clone(),
                        frames: 0,
                        total_frame_ms: 0.0,
                        max_frame_ms: 0.0,
                        cpu_ms: BTreeMap::new(),
                        counters: BTreeMap::new(),
                        rss_bytes: None,
                        sync_max_abs_ms: 0.0,
                    });
                } else {
                    s.report.timeline_truncated = true;
                }
            }
            if
                let Some(b) = s.report.seconds
                    .last_mut()
                    .filter(|b| b.elapsed_second == second && b.phase == phase)
            {
                b.frames += 1;
                b.total_frame_ms += ms;
                b.max_frame_ms = b.max_frame_ms.max(ms);
                if rss > 0 {
                    b.rss_bytes = Some(rss);
                }
                if !s.snapshot.paused && phase == "playing" {
                    b.sync_max_abs_ms = b.sync_max_abs_ms.max(s.snapshot.sync_error_ms.abs());
                }
                for (k, v) in &s.frame_cpu {
                    if let Some(x) = b.cpu_ms.get_mut(k) {
                        *x += v;
                    } else {
                        b.cpu_ms.insert(k.clone(), *v);
                    }
                }
                for (k, v) in &s.frame_counters {
                    if let Some(x) = b.counters.get_mut(k) {
                        *x += v;
                    } else {
                        b.counters.insert(k.clone(), *v);
                    }
                }
            }
            if
                phase == "playing" &&
                (s.report.slowest_playing_frames.len() < MAX_SLOW ||
                    s.report.slowest_playing_frames.last().is_some_and(|f| ms > f.frame_ms))
            {
                s.report.slowest_playing_frames.push(SlowFrame {
                    elapsed_seconds: elapsed,
                    frame_ms: ms,
                    snapshot: s.snapshot.clone(),
                    cpu_ms: s.frame_cpu.clone(),
                    counters: s.frame_counters.clone(),
                });
                s.report.slowest_playing_frames.sort_by(|a, b| b.frame_ms.total_cmp(&a.frame_ms));
                s.report.slowest_playing_frames.truncate(MAX_SLOW);
            }
            s.frame_cpu.values_mut().for_each(|v| {
                *v = 0.0;
            });
            s.frame_counters.values_mut().for_each(|v| {
                *v = 0;
            });
        }
        if s.finish.is_some() {
            slot.take()
        } else {
            None
        }
    });
    if let Some(mut s) = completed {
        s.alive.store(false, Ordering::Relaxed);
        AUDIO_ID.store(0, Ordering::Relaxed);
        s.report.audio = AudioStats {
            corrected_callback_calls: AUDIO_CALLS.load(Ordering::Relaxed),
            corrected_callback_total_ms: (AUDIO_NS.load(Ordering::Relaxed) as f64) / 1e6,
            corrected_callback_max_ms: (AUDIO_MAX_NS.load(Ordering::Relaxed) as f64) / 1e6,
            corrected_callbacks_over_buffer_duration: AUDIO_OVERRUNS.load(Ordering::Relaxed),
        };
        s.report.reason = s.finish.take().unwrap();
        s.report.elapsed_seconds = s.started.elapsed().as_secs_f64();
        for f in s.report.phases.values_mut() {
            f.finalize();
        }
        s.report.process_cpu_seconds = process_cpu()
            .zip(s.cpu_start)
            .map(|(a, b)| (a - b).max(0.0));
        if let Some(sink) = s.sink.take() {
            sink(s.report);
        }
    }
}
#[cfg(any(target_os = "android", target_os = "linux"))]
fn memory_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|l|
        l
            .strip_prefix("VmRSS:")
            .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
            .map(|n| n * 1024)
    )
}
#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn memory_rss() -> Option<u64> {
    None
}
#[cfg(unix)]
fn process_cpu() -> Option<f64> {
    unsafe {
        let mut t: libc::timespec = std::mem::zeroed();
        if libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut t) == 0 {
            Some((t.tv_sec as f64) + (t.tv_nsec as f64) / 1e9)
        } else {
            None
        }
    }
}
#[cfg(not(unix))]
fn process_cpu() -> Option<f64> {
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histogram_and_thresholds() {
        let mut f = FrameStats::default();
        for ms in [8.0, 9.0, 10.0, 17.0, 34.0, 51.0, 101.0] {
            f.add(ms);
        }
        f.finalize();
        assert_eq!(f.p50_ms, 17.0);
        assert_eq!(f.above_16_67_ms, 4);
        assert_eq!(f.above_100_ms, 1);
    }
    #[test]
    fn session_is_opt_in_and_finishes_once() {
        assert!(!active());
        count("ignored", 99);
        assert!(
            start(
                serde_json::json!({}),
                Box::new(|r| {
                    assert_eq!(r.reason, "exit");
                    assert!(!r.cpu_inclusive.contains_key("ignored"));
                })
            )
        );
        assert!(
            !start(
                serde_json::json!({}),
                Box::new(|_| panic!())
            )
        );
        frame_begin();
        {
            let _s = span("load");
        }
        request_finish("exit");
        request_finish("completed");
        frame_end();
        assert!(!active());
        frame_end();
    }
    #[test]
    fn a_stale_span_cannot_leak_into_another_session() {
        assert!(
            start(
                serde_json::json!({}),
                Box::new(|_| {})
            )
        );
        let old = span("old_session");
        request_finish("exit");
        frame_end();
        assert!(
            start(
                serde_json::json!({}),
                Box::new(|r| assert!(!r.cpu_inclusive.contains_key("old_session")))
            )
        );
        drop(old);
        request_finish("exit");
        frame_end();
    }
    #[test]
    fn transitions_and_pauses_are_not_playing_frames() {
        assert!(
            start(
                serde_json::json!({}),
                Box::new(|r| {
                    assert_eq!(r.phases.get("paused").unwrap().frames, 1);
                    assert_eq!(r.phases.get("transition").unwrap().frames, 1);
                    assert!(r.phases.get("playing").is_none());
                    assert!(r.slowest_playing_frames.is_empty());
                })
            )
        );
        observe(0.0, 0.0, true, "paused");
        frame_begin();
        frame_end();
        frame_begin();
        observe(1.0, 1.0, false, "playing");
        request_finish("exit");
        frame_end();
    }
}
