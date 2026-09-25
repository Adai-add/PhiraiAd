//! Judgement system

use crate::{
    config::{Config, JudgementMode},
    core::{BadNote, Chart, Matrix, NoteKind, Point, Resource, Vector, NOTE_WIDTH_RATIO_BASE},
    ext::{get_viewport, NotNanExt},
};
use macroquad::prelude::{
    utils::{register_input_subscriber, repeat_all_miniquad_input},
    *,
};
use miniquad::{EventHandler, MouseButton};
use once_cell::sync::Lazy;
use sasa::{PlaySfxParams, Sfx};
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    mem,
    num::FpCategory,
};
use tracing::debug;

pub const FLICK_SPEED_THRESHOLD: f32 = 0.8;
pub const LIMIT_PERFECT: f64 = 0.08;
pub const LIMIT_GOOD: f64 = 0.16;
pub const LIMIT_BAD: f64 = 0.22;
pub const UP_TOLERANCE: f64 = 0.05;
pub const DIST_FACTOR: f64 = 0.2;

const EARLY_OFFSET: f64 = 0.07;
const PHIRA_X_MAX: f64 = 0.21 / (16. / 9.) * 2.;

// Phigros 3.20.0 normal-mode constants. PGR's horizontal note coordinate is
// converted to prpr's normalized coordinate by 2 * 9 / 160.
const PHIGROS_X_UNIT: f64 = 2. * 9. / 160.;
const PHIGROS_TAP_X: f64 = 1.9 * PHIGROS_X_UNIT;
const PHIGROS_DRAG_X: f64 = 2.1 * PHIGROS_X_UNIT;
const PHIGROS_LIMIT_PERFECT: f64 = 0.08;
const PHIGROS_LIMIT_GOOD: f64 = 0.18;
const PHIGROS_LIMIT_BAD: f64 = 0.22;
const PHIGROS_DRAG_WINDOW: f64 = 0.10;
const PHIGROS_FLICK_WINDOW: f64 = 1.75 * PHIGROS_LIMIT_PERFECT;
const PHIGROS_STRICT_PERFECT_BASE: f64 = 0.04;
const PHIGROS_STRICT_GOOD_BASE: f64 = 0.09;
const PHIGROS_STRICT_BAD_BASE: f64 = 0.14;
const PHIGROS_FRAME_TIME_SAMPLES: usize = 10;
const PHIGROS_RESOLVE_EARLY: f64 = 0.005;
const PHIGROS_HOLD_TAIL: f64 = 0.22;

pub fn hold_tail_window(config: &Config) -> f64 {
    if config.judgement_mode == JudgementMode::PhigrosReplica {
        PHIGROS_HOLD_TAIL
    } else {
        LIMIT_BAD
    }
}
const PHIGROS_CANDIDATE_EPSILON: f64 = 0.010;

#[derive(Debug, Clone)]
pub enum HitSound {
    None,
    Click,
    Flick,
    Drag,
    Custom(String),
}

impl HitSound {
    pub fn play(&self, res: &mut Resource) {
        match self {
            HitSound::None => {}
            HitSound::Click => play_sfx(&mut res.sfx_click, &res.config),
            HitSound::Flick => play_sfx(&mut res.sfx_flick, &res.config),
            HitSound::Drag => play_sfx(&mut res.sfx_drag, &res.config),
            HitSound::Custom(s) => {
                if let Some(sfx) = res.extra_sfxs.get_mut(s) {
                    play_sfx(sfx, &res.config);
                }
            }
        }
    }

    pub fn default_from_kind(kind: &NoteKind) -> Self {
        match kind {
            NoteKind::Click => HitSound::Click,
            NoteKind::Flick => HitSound::Flick,
            NoteKind::Drag => HitSound::Drag,
            NoteKind::Hold { .. } => HitSound::Click,
        }
    }
}

pub fn play_sfx(sfx: &mut Sfx, config: &Config) {
    if config.volume_sfx <= 1e-2 {
        return;
    }
    let _ = sfx.play(PlaySfxParams {
        amplifier: config.volume_sfx,
    });
}

#[cfg(all(not(target_os = "windows"), not(target_os = "ios")))]
fn get_uptime() -> f64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    assert!(ret == 0);
    time.tv_sec as f64 + time.tv_nsec as f64 * 1e-9
}

#[cfg(target_os = "ios")]
fn get_uptime() -> f64 {
    objc2_foundation::NSProcessInfo::processInfo().systemUptime()
}

#[cfg(target_os = "windows")]
fn get_uptime() -> f64 {
    miniquad::native::windows::get_uptime()
}

pub struct FlickTracker {
    threshold: f32,
    last_point: Point,
    last_delta: Option<Vector>,
    last_time: f32,
    flicked: bool,
    stopped: bool,
    official: bool,
    previous_flick_speed: f32,
}

impl FlickTracker {
    pub fn new(dpi: u32, time: f32, point: Point, official: bool) -> Self {
        let threshold = if official {
            0.06 * dpi.max(1) as f32 / 380.
        } else {
            // Preserve Phira's original fixed-DPI behaviour.
            FLICK_SPEED_THRESHOLD * 275. / 386.
        };
        Self {
            threshold,
            last_point: point,
            last_delta: None,
            last_time: time,
            flicked: false,
            stopped: true,
            official,
            previous_flick_speed: 0.,
        }
    }

    fn consume(&mut self) {
        self.flicked = false;
    }

    pub fn push(&mut self, time: f32, position: Point) {
        let delta = position - self.last_point;
        self.last_point = position;
        let dt = (time - self.last_time).max(1e-6);
        if self.official {
            let projected_speed = if self.last_delta.is_some_and(|it| it.magnitude() > 0.1) {
                let last = self.last_delta.unwrap();
                last.dot(&delta) / last.magnitude()
            } else {
                self.previous_flick_speed
            } / (60. * dt);
            if projected_speed < self.threshold || self.stopped {
                let current_speed = delta.magnitude() / (60. * dt);
                self.flicked = current_speed >= self.threshold * 5.;
                self.stopped = current_speed < self.threshold * 5.;
            }
            self.previous_flick_speed = projected_speed;
            self.last_delta = Some(delta);
            self.last_time = time;
            return;
        }
        if let Some(last_delta) = &self.last_delta {
            let speed = delta.dot(last_delta) / dt;
            if speed < self.threshold {
                self.stopped = true;
            }
            if self.stopped && !self.flicked {
                self.flicked = delta.magnitude() / dt >= self.threshold * 2.;
            }
            // if speed < self.threshold || self.stopped {
            // self.stopped = delta.magnitude() / dt < self.threshold * 5.;
            // self.flicked = self.threshold <= speed;
            // if self.flicked {
            // warn!("new flick!");
            // }
            // }
        }
        self.last_delta = Some(delta.normalize());
        self.last_time = time;
    }
}

#[inline]
fn to_phigros_motion_point(point: Vec2) -> Point {
    Point::new(point.x / PHIGROS_X_UNIT as f32, point.y / PHIGROS_X_UNIT as f32)
}

#[inline]
fn update_active_finger(fingers: &mut HashMap<u64, Vec2>, id: u64, phase: TouchPhase, point: Vec2) {
    match phase {
        TouchPhase::Started | TouchPhase::Moved | TouchPhase::Stationary => {
            fingers.insert(id, point);
        }
        TouchPhase::Ended | TouchPhase::Cancelled => {
            fingers.remove(&id);
        }
    }
}

#[inline]
fn remember_finger(order: &mut Vec<u64>, id: u64) {
    if !order.contains(&id) {
        order.push(id);
    }
}

#[inline]
fn forget_finger(order: &mut Vec<u64>, id: u64) {
    order.retain(|it| *it != id);
}

fn append_missing_fingers(order: &mut Vec<u64>, active_ids: &HashSet<u64>) {
    // The platform snapshot is useful for seeding fingers that were already
    // down when gameplay started, but it is not authoritative enough to
    // delete live Phigros state.  Some Android multi-touch paths can expose a
    // transiently incomplete snapshot; pruning here used to drop every
    // Finger/FlickTracker at once until the player lifted and pressed again.
    let mut unknown: Vec<_> = active_ids.iter().filter(|id| !order.contains(id)).copied().collect();
    unknown.sort_unstable();
    order.extend(unknown);
}

fn drain_touches_in_order(mut touches: HashMap<u64, Touch>, order: &[u64]) -> Vec<Touch> {
    let mut result = Vec::with_capacity(touches.len());
    for id in order {
        if let Some(touch) = touches.remove(id) {
            result.push(touch);
        }
    }
    let mut unknown: Vec<_> = touches.into_values().collect();
    unknown.sort_unstable_by_key(|touch| touch.id);
    result.extend(unknown);
    result
}

/// Fill only absent snapshots; preserve fresh phases/coordinates and never
/// resurrect fingers removed by an explicit terminal event.
fn complete_finger_snapshots(touches: &mut HashMap<u64, Touch>, active: &HashMap<u64, Vec2>, mut debug_ids: Option<&mut Vec<u64>>) {
    for (&id, &position) in active {
        if let std::collections::hash_map::Entry::Vacant(entry) = touches.entry(id) {
            entry.insert(Touch {
                id,
                phase: TouchPhase::Stationary,
                position,
                time: f64::NEG_INFINITY,
            });
            if let Some(ids) = debug_ids.as_deref_mut() {
                ids.push(id);
            }
        }
    }
}

fn rebind_official_flick_tracker(trackers: &mut HashMap<u64, FlickTracker>, id: u64, dpi: u32, frame_time: f32, position: Vec2) -> bool {
    if trackers.contains_key(&id) {
        return false;
    }
    trackers.insert(id, FlickTracker::new(dpi, frame_time, to_phigros_motion_point(position), true));
    true
}

#[inline]
fn event_judgement_time(frame_time: f64, raw_time: f64, official: bool) -> f64 {
    if official || raw_time.is_infinite() {
        frame_time
    } else {
        raw_time
    }
}

/// Stable, serializable copy of Macroquad's touch phase for diagnostic play
/// reports.  Keeping this separate prevents report schema changes when the
/// platform library changes its enum representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchDebugPhase {
    Started,
    Moved,
    Stationary,
    Ended,
    Cancelled,
}

impl From<TouchPhase> for TouchDebugPhase {
    fn from(value: TouchPhase) -> Self {
        match value {
            TouchPhase::Started => Self::Started,
            TouchPhase::Moved => Self::Moved,
            TouchPhase::Stationary => Self::Stationary,
            TouchPhase::Ended => Self::Ended,
            TouchPhase::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchDebugSampleSource {
    MiniquadTouchEvent,
    MouseButtonEvent,
    MouseHeldSynthesis,
    MacroquadPersistentSnapshot,
    MacroquadMouseSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchDebugClearSource {
    JudgeReset,
    ExerciseSettingsReset,
    PauseButton,
    AutoFlipTransition,
    AppLifecyclePause,
    InstantDeath,
    KeyboardPause,
    ExplicitUnknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugSample {
    pub sequence: u64,
    pub finger_id: u64,
    pub phase: TouchDebugPhase,
    pub source: TouchDebugSampleSource,
    pub platform_event_time_seconds: Option<f64>,
    pub captured_uptime_seconds: f64,
    pub frame_judge_time_seconds: f64,
    pub mapped_judge_time_seconds: f64,
    pub tap_drag_hold_effective_time_seconds: f64,
    pub flick_effective_time_seconds: f64,
    pub screen_x_px: f32,
    pub screen_y_px: f32,
    pub judge_x: f32,
    pub judge_y: f32,
    pub phigros_motion_x: f32,
    pub phigros_motion_y: f32,
    pub present_in_persistent_snapshot: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugFingerState {
    pub finger_id: u64,
    pub judge_x: f32,
    pub judge_y: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugTrackerState {
    pub finger_id: u64,
    pub official: bool,
    pub flicked: bool,
    pub stopped: bool,
    pub last_time_seconds: f32,
    pub last_x: f32,
    pub last_y: f32,
    pub previous_flick_speed: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugFrame {
    pub sequence: u64,
    pub frame_index: u64,
    pub judge_time_seconds: f64,
    pub tracker_clock_seconds: f64,
    pub captured_uptime_seconds: f64,
    pub frame_delta_seconds: f64,
    pub tap_drag_hold_uses_phigros: bool,
    pub flick_uses_phigros: bool,
    pub raw_events: Vec<TouchDebugSample>,
    pub persistent_snapshot: Vec<TouchDebugSample>,
    pub active_fingers_before: Vec<TouchDebugFingerState>,
    pub active_fingers_after: Vec<TouchDebugFingerState>,
    pub finger_order_before: Vec<u64>,
    pub finger_order_after: Vec<u64>,
    pub trackers_before: Vec<TouchDebugTrackerState>,
    pub trackers_after: Vec<TouchDebugTrackerState>,
    pub tracker_started_ids: Vec<u64>,
    pub tracker_rebound_ids: Vec<u64>,
    pub tracker_removed_ids: Vec<u64>,
    pub synthesized_stationary_ids: Vec<u64>,
    pub key_down_count_before: u32,
    pub key_down_count_after: u32,
    pub raw_key_delta: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugClearEvent {
    pub sequence: u64,
    pub source: TouchDebugClearSource,
    pub judge_clock_seconds: f64,
    pub captured_uptime_seconds: f64,
    pub active_fingers_cleared: Vec<TouchDebugFingerState>,
    pub finger_order_cleared: Vec<u64>,
    pub trackers_cleared: Vec<TouchDebugTrackerState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JudgeTouchDebugRecord {
    Frame(Box<TouchDebugFrame>),
    Clear(TouchDebugClearEvent),
}

#[derive(Clone)]
struct PendingSnapshotDebugSample {
    touch: Touch,
    raw_position: Vec2,
    source: TouchDebugSampleSource,
}

fn touch_debug_finger_states(fingers: &HashMap<u64, Vec2>) -> Vec<TouchDebugFingerState> {
    let mut states: Vec<_> = fingers
        .iter()
        .map(|(&finger_id, point)| TouchDebugFingerState {
            finger_id,
            judge_x: point.x,
            judge_y: point.y,
        })
        .collect();
    states.sort_unstable_by_key(|it| it.finger_id);
    states
}

fn touch_debug_tracker_states(trackers: &HashMap<u64, FlickTracker>) -> Vec<TouchDebugTrackerState> {
    let mut states: Vec<_> = trackers
        .iter()
        .map(|(&finger_id, tracker)| TouchDebugTrackerState {
            finger_id,
            official: tracker.official,
            flicked: tracker.flicked,
            stopped: tracker.stopped,
            last_time_seconds: tracker.last_time,
            last_x: tracker.last_point.x,
            last_y: tracker.last_point.y,
            previous_flick_speed: tracker.previous_flick_speed,
        })
        .collect();
    states.sort_unstable_by_key(|it| it.finger_id);
    states
}

fn raw_touch_debug_source(id: u64, phase: TouchPhase) -> TouchDebugSampleSource {
    if id <= u64::MAX - 4 {
        TouchDebugSampleSource::MiniquadTouchEvent
    } else if matches!(phase, TouchPhase::Moved) {
        TouchDebugSampleSource::MouseHeldSynthesis
    } else {
        TouchDebugSampleSource::MouseButtonEvent
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OfficialClickKind {
    Tap,
    Drag,
    Hold,
    Flick,
}

impl OfficialClickKind {
    #[inline]
    fn from_note(kind: &NoteKind) -> Self {
        match kind {
            NoteKind::Click => Self::Tap,
            NoteKind::Drag => Self::Drag,
            NoteKind::Hold { .. } => Self::Hold,
            NoteKind::Flick => Self::Flick,
        }
    }

    #[inline]
    fn is_tap_or_hold(self) -> bool {
        matches!(self, Self::Tap | Self::Hold)
    }
}

#[derive(Clone, Copy, Debug)]
struct OfficialClickCandidate {
    line_id: usize,
    note_id: u32,
    time: f64,
    spatial: f64,
    diff: f64,
    kind: OfficialClickKind,
}

fn choose_official_click_candidate(candidates: &mut [OfficialClickCandidate], frame_time: f64) -> Option<OfficialClickCandidate> {
    // Phigros scans one global, time-sorted note list. Drag/Flick are weak
    // candidates: a later eligible note may replace them. Once Tap/Hold owns
    // the candidate, only another Tap/Hold within 10 ms may replace it, and
    // then only when its local-space position cost is lower.
    candidates.sort_unstable_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then_with(|| a.line_id.cmp(&b.line_id))
            .then_with(|| a.note_id.cmp(&b.note_id))
    });
    let mut selected: Option<OfficialClickCandidate> = None;
    for candidate in candidates.iter().copied() {
        let Some(current) = selected else {
            selected = Some(candidate);
            continue;
        };
        if candidate.time - frame_time > (current.time - frame_time).abs() + PHIGROS_CANDIDATE_EPSILON {
            continue;
        }
        if !current.kind.is_tap_or_hold()
            || (candidate.kind.is_tap_or_hold()
                && (candidate.time - current.time).abs() <= PHIGROS_CANDIDATE_EPSILON
                && candidate.spatial < current.spatial)
        {
            selected = Some(candidate);
        }
    }
    selected
}

#[inline]
fn hold_contact_lost(safe_frame: &mut i8) -> bool {
    if *safe_frame >= 0 {
        *safe_frame -= 1;
        false
    } else {
        true
    }
}

#[inline]
fn arm_hold_visual_tail(tail_armed: &mut bool) -> bool {
    !mem::replace(tail_armed, true)
}

#[inline]
fn hold_visual_tail_ended(tail_armed: bool, end_time: f64, time: f64) -> bool {
    tail_armed && time >= end_time
}

#[inline]
fn note_uses_phigros_judgement(kind: &NoteKind, other_official: bool, flick_official: bool) -> bool {
    if matches!(kind, NoteKind::Flick) {
        flick_official
    } else {
        other_official
    }
}

#[derive(Clone, Copy, Debug)]
struct PhigrosWindows {
    perfect: f64,
    good: f64,
    bad: f64,
    flick: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JudgementTimeWindow {
    /// Seconds before the note time, measured on the real judgement clock.
    pub early: f64,
    /// Seconds after the note time, measured on the real judgement clock.
    pub late: f64,
}

impl JudgementTimeWindow {
    const fn symmetric(value: f64) -> Self {
        Self { early: value, late: value }
    }

    const fn phira(value: f64) -> Self {
        Self {
            early: value,
            // Phira deliberately keeps its original 70 ms late-input
            // compensation.  The overlay folds it into the normal band rather
            // than drawing a misleading separate region.
            late: value + EARLY_OFFSET,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JudgementRangeProfile {
    pub tap_perfect: JudgementTimeWindow,
    pub tap_good: JudgementTimeWindow,
    pub tap_outer: JudgementTimeWindow,
    pub hold_perfect: JudgementTimeWindow,
    pub hold_outer: JudgementTimeWindow,
    pub drag_outer: JudgementTimeWindow,
    pub flick_outer: JudgementTimeWindow,
    pub hold_tail: f64,
    pub tap_x: f64,
    pub hold_x: f64,
    pub drag_x: f64,
    pub flick_x: f64,
}

impl PhigrosWindows {
    fn normal() -> Self {
        Self {
            perfect: PHIGROS_LIMIT_PERFECT,
            good: PHIGROS_LIMIT_GOOD,
            bad: PHIGROS_LIMIT_BAD,
            flick: PHIGROS_FLICK_WINDOW,
        }
    }

    fn strict(average_frame_time: f64) -> Self {
        let half_frame = average_frame_time * 0.5;
        let perfect = PHIGROS_STRICT_PERFECT_BASE + half_frame;
        Self {
            perfect,
            good: PHIGROS_STRICT_GOOD_BASE + half_frame,
            bad: PHIGROS_STRICT_BAD_BASE + half_frame,
            flick: 1.75 * perfect,
        }
    }
}

#[derive(Debug, Default)]
struct RecentFrameTimes {
    samples: [f64; PHIGROS_FRAME_TIME_SAMPLES],
    count: usize,
    next: usize,
}

impl RecentFrameTimes {
    fn push(&mut self, delta: f64) {
        if !delta.is_finite() || delta <= 0. {
            return;
        }
        self.samples[self.next] = delta;
        self.next = (self.next + 1) % PHIGROS_FRAME_TIME_SAMPLES;
        self.count = (self.count + 1).min(PHIGROS_FRAME_TIME_SAMPLES);
    }

    fn average(&self) -> f64 {
        if self.count == 0 {
            1. / 60.
        } else {
            self.samples[..self.count].iter().sum::<f64>() / self.count as f64
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug)]
pub enum JudgeStatus {
    NotJudged,
    PreJudge,
    Judged,
    Hold(bool, f64, f64, bool, f64, i8, bool), // perfect, at, diff, tail-armed, up-time, safe-frame, head-pending
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize)]
pub enum Judgement {
    Perfect,
    Good,
    Bad,
    Miss,
}

#[cfg(not(closed))]
#[derive(Default)]
pub(crate) struct JudgeInner {
    diffs: Vec<f64>,

    combo: u32,
    max_combo: u32,
    counts: [u32; 4],
    num_of_notes: u32,
    early_kind: [u32; 4],
    late_kind: [u32; 4],
}

#[cfg(not(closed))]
impl JudgeInner {
    pub fn new(num_of_notes: u32) -> Self {
        Self {
            diffs: Vec::new(),

            combo: 0,
            max_combo: 0,
            counts: [0; 4],
            num_of_notes,
            early_kind: [0; 4],
            late_kind: [0; 4],
        }
    }

    pub fn commit(&mut self, what: Judgement, diff: f64) {
        use Judgement::*;
        if matches!(what, Judgement::Good) {
            self.diffs.push(diff);
        }
        if diff < 0. {
            self.early_kind[what as usize] += 1;
        } else if diff > 0. {
            self.late_kind[what as usize] += 1;
        }
        self.counts[what as usize] += 1;
        match what {
            Perfect | Good => {
                self.combo += 1;
                if self.combo > self.max_combo {
                    self.max_combo = self.combo;
                }
            }
            _ => {
                self.combo = 0;
            }
        }
    }

    pub fn reset(&mut self) {
        self.combo = 0;
        self.max_combo = 0;
        self.counts = [0; 4];
        self.diffs.clear();
        self.early_kind = [0; 4];
        self.late_kind = [0; 4];
    }

    pub fn accuracy(&self) -> f64 {
        (self.counts[0] as f64 + self.counts[1] as f64 * 0.65) / self.num_of_notes as f64
    }

    pub fn real_time_accuracy(&self) -> f64 {
        let cnt = self.counts.iter().sum::<u32>();
        if cnt == 0 {
            return 1.;
        }
        (self.counts[0] as f64 + self.counts[1] as f64 * 0.65) / cnt as f64
    }

    pub fn score(&self) -> u32 {
        const TOTAL: u32 = 1000000;
        if self.counts[0] == self.num_of_notes {
            TOTAL
        } else {
            let score = (0.9 * self.accuracy() + self.max_combo as f64 / self.num_of_notes as f64 * 0.1) * TOTAL as f64;
            score.round() as u32
        }
    }

    pub fn result(&self) -> PlayResult {
        let early = self.diffs.iter().filter(|it| **it < 0.).count() as u32;
        PlayResult {
            score: self.score(),
            accuracy: self.accuracy(),
            max_combo: self.max_combo,
            num_of_notes: self.num_of_notes,
            counts: self.counts,
            early,
            late: self.diffs.len() as u32 - early,
            std: 0.,
            early_kind: self.early_kind,
            late_kind: self.late_kind,
        }
    }

    pub fn combo(&self) -> u32 {
        self.combo
    }

    pub fn counts(&self) -> [u32; 4] {
        self.counts
    }
}

#[rustfmt::skip]
#[cfg(closed)]
pub mod inner;
#[cfg(closed)]
use inner::*;

type Judgements = Vec<(f64, u32, u32, Result<Judgement, bool>)>;

/// Read-only copy of an outcome emitted for gameplay reports.  `Err(bool)` is
/// the existing Hold-head protocol (`true` = Perfect, `false` = Good), while
/// `Ok(..)` is a final note outcome.  `difference` is the exact signed value
/// committed to scoring (negative is early in the engine).
#[derive(Debug, Copy, Clone)]
pub struct JudgeReportEvent {
    pub time: f64,
    pub line_id: u32,
    pub note_id: u32,
    pub judgement: Result<Judgement, bool>,
    pub difference: f64,
}

#[repr(C)]
pub struct Judge {
    // notes of each line in order
    // LinkedList::drain_filter is unstable...
    pub notes: Vec<(Vec<u32>, usize)>,
    pub trackers: HashMap<u64, FlickTracker>,
    pub last_time: f64,

    // Phigros keeps a persistent FingerManagement list. This is deliberately
    // separate from raw per-frame MotionEvents; the platform's persistent
    // snapshot is also used to recover if a terminal event was lost on pause.
    active_fingers: HashMap<u64, Vec2>,

    // FingerManagement uses a persistent List<Fingers>, so simultaneous
    // fingers are always processed in insertion order rather than hash order.
    finger_order: Vec<u64>,

    // Phigros Challenge/strict judgement adds half of the average of the
    // latest (up to) ten Time.deltaTime samples to its P/G/B base windows.
    recent_frame_times: RecentFrameTimes,

    touch_debug_enabled: bool,
    touch_debug_sequence: u64,
    touch_debug_frame_index: u64,
    touch_debug_records: Vec<JudgeTouchDebugRecord>,

    key_down_count: u32,

    pub(crate) inner: JudgeInner,
    pub judgements: RefCell<Judgements>,
    report_judgements: RefCell<Vec<JudgeReportEvent>>,
}

#[derive(Default)]
struct TouchStatus {
    touches: Vec<Touch>,
    key_delta: i32,
    keys_down: u32,
}

static SUBSCRIBER_ID: Lazy<usize> = Lazy::new(register_input_subscriber);
thread_local! {
    static TOUCHES: RefCell<TouchStatus> = RefCell::default();
    static WHEEL: RefCell<(f32, f32)> = RefCell::default();
}

pub fn take_wheel() -> (f32, f32) {
    WHEEL.with(|it| mem::take(&mut *it.borrow_mut()))
}

impl Judge {
    pub fn new(chart: &Chart) -> Self {
        let notes = chart
            .lines
            .iter()
            .map(|line| {
                let mut idx: Vec<u32> = (0..(line.notes.len() as u32)).filter(|it| !line.notes[*it as usize].fake).collect();
                idx.sort_by_key(|id| line.notes[*id as usize].time.not_nan());
                (idx, 0)
            })
            .collect();
        Self {
            notes,
            trackers: HashMap::new(),
            last_time: 0.,

            active_fingers: HashMap::new(),
            finger_order: Vec::new(),
            recent_frame_times: RecentFrameTimes::default(),

            touch_debug_enabled: false,
            touch_debug_sequence: 0,
            touch_debug_frame_index: 0,
            touch_debug_records: Vec::new(),

            key_down_count: 0,

            inner: JudgeInner::new(chart.lines.iter().map(|it| it.notes.iter().filter(|it| !it.fake).count() as u32).sum()),
            judgements: RefCell::new(Vec::new()),
            report_judgements: RefCell::new(Vec::new()),
        }
    }

    pub fn reset(&mut self) {
        self.reset_with_touch_debug_source(TouchDebugClearSource::JudgeReset);
    }

    pub fn reset_with_touch_debug_source(&mut self, source: TouchDebugClearSource) {
        self.notes.iter_mut().for_each(|it| it.1 = 0);
        self.clear_touch_input_with_source(source);
        self.recent_frame_times.clear();
        self.inner.reset();
        self.judgements.borrow_mut().clear();
        self.report_judgements.borrow_mut().clear();
    }

    /// Clear persistent touch state at an explicit gameplay lifecycle
    /// boundary.  Normal frames deliberately do not infer releases from an
    /// incomplete platform snapshot; pause/retry/scene transitions call this
    /// method instead so stale fingers cannot survive across a boundary.
    pub fn clear_touch_input(&mut self) {
        self.clear_touch_input_with_source(TouchDebugClearSource::ExplicitUnknown);
    }

    pub fn clear_touch_input_with_source(&mut self, source: TouchDebugClearSource) {
        if self.touch_debug_enabled {
            let sequence = self.next_touch_debug_sequence();
            self.touch_debug_records.push(JudgeTouchDebugRecord::Clear(TouchDebugClearEvent {
                sequence,
                source,
                judge_clock_seconds: self.last_time,
                captured_uptime_seconds: get_uptime(),
                active_fingers_cleared: touch_debug_finger_states(&self.active_fingers),
                finger_order_cleared: self.finger_order.clone(),
                trackers_cleared: touch_debug_tracker_states(&self.trackers),
            }));
        }
        self.trackers.clear();
        self.active_fingers.clear();
        self.finger_order.clear();
    }

    pub fn set_touch_debug_enabled(&mut self, enabled: bool) {
        self.touch_debug_enabled = enabled;
        if !enabled {
            self.touch_debug_records.clear();
        }
    }

    pub fn take_touch_debug_records(&mut self) -> Vec<JudgeTouchDebugRecord> {
        mem::take(&mut self.touch_debug_records)
    }

    fn next_touch_debug_sequence(&mut self) -> u64 {
        let sequence = self.touch_debug_sequence;
        self.touch_debug_sequence = self.touch_debug_sequence.saturating_add(1);
        sequence
    }

    /// Resolve the exact judgement windows currently used by each selected
    /// engine.  Debug drawing reads this snapshot so strict-mode frame
    /// compensation cannot drift away from the real judge.
    pub fn judgement_range_profile(&self, config: &Config) -> JudgementRangeProfile {
        let other_phigros = config.judgement_mode == JudgementMode::PhigrosReplica;
        let flick_phigros = config.flick_judgement_mode() == JudgementMode::PhigrosReplica;
        let windows = if config.phigros_strict_judgement && (other_phigros || flick_phigros) {
            PhigrosWindows::strict(self.recent_frame_times.average())
        } else {
            PhigrosWindows::normal()
        };

        let (tap_perfect, tap_good, tap_outer, hold_perfect, hold_outer, tap_x, hold_x, drag_outer, drag_x) = if other_phigros {
            (
                JudgementTimeWindow::symmetric(windows.perfect),
                JudgementTimeWindow::symmetric(windows.good),
                JudgementTimeWindow {
                    early: windows.bad,
                    late: windows.good,
                },
                JudgementTimeWindow::symmetric(windows.perfect),
                JudgementTimeWindow {
                    early: windows.bad,
                    late: windows.good,
                },
                PHIGROS_TAP_X,
                PHIGROS_TAP_X,
                JudgementTimeWindow::symmetric(PHIGROS_DRAG_WINDOW),
                PHIGROS_DRAG_X,
            )
        } else {
            (
                JudgementTimeWindow::phira(LIMIT_PERFECT),
                JudgementTimeWindow::phira(LIMIT_GOOD),
                JudgementTimeWindow::phira(LIMIT_BAD),
                JudgementTimeWindow::phira(LIMIT_PERFECT),
                JudgementTimeWindow::phira(LIMIT_BAD),
                PHIRA_X_MAX,
                PHIRA_X_MAX,
                JudgementTimeWindow::phira(LIMIT_BAD),
                PHIRA_X_MAX,
            )
        };
        let (flick_outer, flick_x) = if flick_phigros {
            (JudgementTimeWindow::symmetric(windows.flick), PHIGROS_DRAG_X)
        } else {
            (JudgementTimeWindow::phira(LIMIT_BAD), PHIRA_X_MAX)
        };

        JudgementRangeProfile {
            tap_perfect,
            tap_good,
            tap_outer,
            hold_perfect,
            hold_outer,
            drag_outer,
            flick_outer,
            hold_tail: if other_phigros { PHIGROS_HOLD_TAIL } else { LIMIT_BAD },
            tap_x,
            hold_x,
            drag_x,
            flick_x,
        }
    }

    /// Advance note pointers past notes before time `t`, marking them as judged.
    /// Used in exercise mode to skip notes before the exercise range start.
    pub fn advance_to(&mut self, chart: &mut Chart, t: f64) {
        for (line, (idx, st)) in chart.lines.iter_mut().zip(self.notes.iter_mut()) {
            while *st < idx.len() {
                let note = &mut line.notes[idx[*st] as usize];
                if note.time >= t {
                    break;
                }
                note.judge = JudgeStatus::Judged;
                *st += 1;
            }
        }
        self.last_time = t;
    }

    pub fn commit(&mut self, t: f64, what: Judgement, line_id: u32, note_id: u32, diff: f64) {
        self.judgements.borrow_mut().push((t, line_id, note_id, Ok(what)));
        self.report_judgements.borrow_mut().push(JudgeReportEvent {
            time: t,
            line_id,
            note_id,
            judgement: Ok(what),
            difference: diff,
        });
        self.inner.commit(what, diff);
    }

    fn commit_hold_head(
        judgements: &RefCell<Judgements>,
        report_judgements: &RefCell<Vec<JudgeReportEvent>>,
        t: f64,
        line_id: u32,
        note_id: u32,
        perfect: bool,
    ) {
        judgements.borrow_mut().push((t, line_id, note_id, Err(perfect)));
        report_judgements.borrow_mut().push(JudgeReportEvent {
            time: t,
            line_id,
            note_id,
            judgement: Err(perfect),
            difference: 0.,
        });
    }

    pub fn take_report_judgements(&self) -> Vec<JudgeReportEvent> {
        std::mem::take(&mut *self.report_judgements.borrow_mut())
    }

    #[inline]
    pub fn accuracy(&self) -> f64 {
        self.inner.accuracy()
    }

    #[inline]
    pub fn real_time_accuracy(&self) -> f64 {
        self.inner.real_time_accuracy()
    }

    #[inline]
    pub fn score(&self) -> u32 {
        self.inner.score()
    }

    pub(crate) fn on_new_frame() {
        let mut handler = Handler {
            status: TouchStatus::default(),
            wheel: (0., 0.),
        };
        repeat_all_miniquad_input(&mut handler, *SUBSCRIBER_ID);
        handler.finalize();
        TOUCHES.with(|it| {
            *it.borrow_mut() = handler.status;
        });
        WHEEL.with(|it| {
            *it.borrow_mut() = handler.wheel;
        });
    }

    pub fn rotate_input_half_turn(&mut self) {
        for position in self.active_fingers.values_mut() {
            position.x = -position.x;
            position.y = -position.y;
        }
        for tracker in self.trackers.values_mut() {
            if tracker.official {
                tracker.last_point.x = -tracker.last_point.x;
                tracker.last_point.y = -tracker.last_point.y;
                if let Some(delta) = &mut tracker.last_delta {
                    delta.x = -delta.x;
                    delta.y = -delta.y;
                }
            }
        }
    }

    fn touch_transform(flip_x: bool, flip_y: bool) -> impl Fn(&mut Touch) {
        let (scale_x, scale_y) = crate::chart_play::reflection_axes(flip_x, flip_y);
        let vp = get_viewport();
        move |touch| {
            let p = touch.position;
            touch.position = vec2(
                (p.x - vp.0 as f32) / vp.2 as f32 * 2. - 1.,
                ((p.y - (screen_height() - (vp.1 + vp.3) as f32)) / vp.3 as f32 * 2. - 1.) / (vp.2 as f32 / vp.3 as f32),
            );
            touch.position.x *= scale_x;
            touch.position.y *= scale_y;
        }
    }

    pub fn get_touches() -> Vec<Touch> {
        TOUCHES.with(|it| {
            let guard = it.borrow();
            let tr = Self::touch_transform(false, false);
            guard
                .touches
                .iter()
                .cloned()
                .map(|mut it| {
                    tr(&mut it);
                    it
                })
                .collect()
        })
    }

    pub fn update(&mut self, res: &mut Resource, chart: &mut Chart, bad_notes: &mut Vec<BadNote>) {
        let touch_debug_enabled = res.config.touch_input_debug_report;
        self.set_touch_debug_enabled(touch_debug_enabled);
        if res.config.autoplay() {
            self.auto_play_update(res, chart);
            return;
        }
        const X_DIFF_MAX: f64 = PHIRA_X_MAX;
        let other_official = res.config.judgement_mode == JudgementMode::PhigrosReplica;
        let flick_official = res.config.flick_judgement_mode() == JudgementMode::PhigrosReplica;
        let strict_official = res.config.phigros_strict_judgement && (other_official || flick_official);
        if strict_official {
            self.recent_frame_times.push(get_frame_time() as f64);
        }
        let phigros_windows = if strict_official {
            PhigrosWindows::strict(self.recent_frame_times.average())
        } else {
            PhigrosWindows::normal()
        };
        let spd = res.config.speed as f64;

        let uptime = get_uptime();

        let t = res.time;
        let flip_x = res.config.flip_x() ^ res.auto_flip_y;
        let debug_active_before = touch_debug_enabled.then(|| touch_debug_finger_states(&self.active_fingers));
        let debug_order_before = touch_debug_enabled.then(|| self.finger_order.clone());
        let debug_trackers_before = touch_debug_enabled.then(|| touch_debug_tracker_states(&self.trackers));
        let debug_key_down_before = self.key_down_count;
        let mut pending_snapshot_debug = Vec::new();
        // TODO optimize
        let mut touches: HashMap<u64, Touch> = {
            let mut touches: Vec<_> = touches()
                .into_iter()
                .map(|touch| (touch, TouchDebugSampleSource::MacroquadPersistentSnapshot))
                .collect();
            let btn = MouseButton::Left;
            let id = button_to_id(btn);
            if is_mouse_button_pressed(btn) {
                let p = mouse_position();
                touches.push((
                    Touch {
                        id,
                        phase: TouchPhase::Started,
                        position: vec2(p.0, p.1),
                        time: f64::NEG_INFINITY,
                    },
                    TouchDebugSampleSource::MacroquadMouseSnapshot,
                ));
            } else if is_mouse_button_down(btn) {
                let p = mouse_position();
                touches.push((
                    Touch {
                        id,
                        phase: TouchPhase::Moved,
                        position: vec2(p.0, p.1),
                        time: f64::NEG_INFINITY,
                    },
                    TouchDebugSampleSource::MacroquadMouseSnapshot,
                ));
            } else if is_mouse_button_released(btn) {
                let p = mouse_position();
                touches.push((
                    Touch {
                        id,
                        phase: TouchPhase::Ended,
                        position: vec2(p.0, p.1),
                        time: f64::NEG_INFINITY,
                    },
                    TouchDebugSampleSource::MacroquadMouseSnapshot,
                ));
            }
            let tr = Self::touch_transform(flip_x, res.auto_flip_y);
            touches
                .into_iter()
                .map(|(mut it, source)| {
                    let raw_position = it.position;
                    tr(&mut it);
                    if touch_debug_enabled {
                        pending_snapshot_debug.push(PendingSnapshotDebugSample {
                            touch: it.clone(),
                            raw_position,
                            source,
                        });
                    }
                    (it.id, it)
                })
                .collect()
        };
        let any_official = other_official || flick_official;
        let active_snapshot_ids: HashSet<u64> = touches
            .values()
            .filter(|touch| matches!(touch.phase, TouchPhase::Started | TouchPhase::Moved | TouchPhase::Stationary))
            .map(|touch| touch.id)
            .collect();
        if !any_official {
            self.finger_order.clear();
        }
        // A transformed MotionEvent fallback needed only by a mixed
        // Phigros-Flick/Phira-other setup. Keep it out of the passive touch
        // list so changing the Flick engine cannot change Phira Drag/Hold.
        let mut extra_flick_touches = HashMap::new();
        if other_official {
            // Seed and refresh positions from the persistent snapshot, but do
            // not use absence from one frame as a release.  Explicit terminal
            // events below and lifecycle-boundary cleanup are authoritative.
            for touch in touches.values() {
                update_active_finger(&mut self.active_fingers, touch.id, touch.phase, touch.position);
            }
        } else {
            self.active_fingers.clear();
        }
        let (events, keys_down, key_delta) = TOUCHES.with(|it| {
            let guard = it.borrow();
            let events = guard.touches.clone();
            if res.config.use_keyboard {
                (events, guard.keys_down, guard.key_delta)
            } else {
                (events, 0, 0)
            }
        });
        let raw_key_delta = key_delta;
        self.key_down_count = self.key_down_count.saturating_add_signed(key_delta);
        let mut debug_raw_events = Vec::new();
        let mut debug_tracker_started_ids = Vec::new();
        let mut debug_tracker_rebound_ids = Vec::new();
        let mut debug_tracker_removed_ids = Vec::new();
        let mut debug_synthesized_stationary_ids = Vec::new();
        {
            fn to_phira_tracker_point(Vec2 { x, y }: Vec2) -> Point {
                Point::new(x / screen_width() * 2. - 1., y / screen_height() * 2. - 1.)
            }
            let delta = (t / spd - self.last_time) / (events.len() + 1) as f64;
            let mut tracker_time = self.last_time;
            let official_frame_time = (t / spd) as f32;
            let mut official_started = HashSet::new();
            let event_transform = Self::touch_transform(flip_x, res.auto_flip_y);
            for Touch {
                id,
                phase,
                position: raw_point,
                time,
            } in events.into_iter()
            {
                tracker_time += delta;
                let sample_time = tracker_time as f32;
                let phira_tracker_point = to_phira_tracker_point(raw_point);
                let mut transformed_touch = Touch {
                    id,
                    phase,
                    position: raw_point,
                    time,
                };
                event_transform(&mut transformed_touch);
                let judge_point = transformed_touch.position;
                if touch_debug_enabled {
                    let mapped_time = if time.is_infinite() { t } else { t - (uptime - time) * spd };
                    let phigros_point = to_phigros_motion_point(judge_point);
                    let sequence = self.next_touch_debug_sequence();
                    debug_raw_events.push(TouchDebugSample {
                        sequence,
                        finger_id: id,
                        phase: phase.into(),
                        source: raw_touch_debug_source(id, phase),
                        platform_event_time_seconds: time.is_finite().then_some(time),
                        captured_uptime_seconds: uptime,
                        frame_judge_time_seconds: t,
                        mapped_judge_time_seconds: mapped_time,
                        tap_drag_hold_effective_time_seconds: event_judgement_time(t, mapped_time, other_official),
                        flick_effective_time_seconds: event_judgement_time(t, mapped_time, flick_official),
                        screen_x_px: raw_point.x,
                        screen_y_px: raw_point.y,
                        judge_x: judge_point.x,
                        judge_y: judge_point.y,
                        phigros_motion_x: phigros_point.x,
                        phigros_motion_y: phigros_point.y,
                        present_in_persistent_snapshot: active_snapshot_ids.contains(&id),
                    });
                }
                let tracker_point = if flick_official {
                    // The 0.06 * dpi / 380 Flick thresholds are expressed in
                    // Phigros world units, not prpr's normalized [-1, 1] space.
                    to_phigros_motion_point(judge_point)
                } else {
                    phira_tracker_point
                };

                if any_official {
                    match phase {
                        // Accept an orphan Moved/Stationary as an implicit
                        // reattachment. Android and a few emulator input
                        // bridges can resume a pointer without replaying Down.
                        TouchPhase::Started | TouchPhase::Moved | TouchPhase::Stationary => remember_finger(&mut self.finger_order, id),
                        TouchPhase::Ended | TouchPhase::Cancelled => forget_finger(&mut self.finger_order, id),
                    }
                }
                if other_official {
                    update_active_finger(&mut self.active_fingers, id, phase, judge_point);
                }
                match phase {
                    TouchPhase::Started => {
                        let tracker_start = if flick_official { official_frame_time } else { sample_time };
                        self.trackers
                            .insert(id, FlickTracker::new(res.dpi, tracker_start, tracker_point, flick_official));
                        if touch_debug_enabled {
                            debug_tracker_started_ids.push(id);
                        }
                        if flick_official {
                            official_started.insert(id);
                        }
                        if other_official {
                            touches.entry(id).or_insert(transformed_touch).phase = TouchPhase::Started;
                        } else {
                            // Keep the original Phira fallback path byte-for-byte
                            // equivalent when macroquad has no snapshot entry.
                            touches
                                .entry(id)
                                .or_insert_with(|| Touch {
                                    id,
                                    phase: TouchPhase::Started,
                                    position: vec2(phira_tracker_point.x, phira_tracker_point.y),
                                    time,
                                })
                                .phase = TouchPhase::Started;
                        }
                    }
                    TouchPhase::Moved | TouchPhase::Stationary => {
                        if !flick_official {
                            if let Some(tracker) = self.trackers.get_mut(&id) {
                                tracker.push(sample_time, tracker_point);
                            }
                        }
                        if flick_official && !touches.contains_key(&id) {
                            if other_official {
                                touches.insert(id, transformed_touch);
                            } else {
                                extra_flick_touches.insert(id, transformed_touch);
                            }
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if self.trackers.remove(&id).is_some() && touch_debug_enabled {
                            debug_tracker_removed_ids.push(id);
                        }
                    }
                }
            }
            // Complete retained fingers before velocity updates, so a missing
            // snapshot still supplies this frame's zero-displacement sample.
            // Explicit Ended/Cancelled events have already removed the finger.
            if other_official && flick_official {
                complete_finger_snapshots(&mut touches, &self.active_fingers, touch_debug_enabled.then_some(&mut debug_synthesized_stationary_ids));
            }
            if any_official {
                append_missing_fingers(&mut self.finger_order, &active_snapshot_ids);
            }
            if flick_official {
                // Recreate a missing official tracker at the latest position.
                // The current frame is treated as its zero-velocity baseline,
                // so recovery can never manufacture a Flick from the gap.
                for id in self.finger_order.clone() {
                    let touch = touches.get(&id).or_else(|| extra_flick_touches.get(&id));
                    let Some(touch) = touch.filter(|touch| matches!(touch.phase, TouchPhase::Started | TouchPhase::Moved | TouchPhase::Stationary))
                    else {
                        continue;
                    };
                    if rebind_official_flick_tracker(&mut self.trackers, id, res.dpi, official_frame_time, touch.position) {
                        official_started.insert(id);
                        if touch_debug_enabled {
                            debug_tracker_rebound_ids.push(id);
                        }
                    }
                }
                // FingerManagement updates every active Fingers object exactly
                // once per Unity frame using Time.deltaTime. Android may emit
                // several MotionEvent samples in that frame; only the latest
                // transformed position belongs in this velocity update.
                let mut tracker_ids = self.finger_order.clone();
                let mut unknown: Vec<_> = self.trackers.keys().filter(|id| !tracker_ids.contains(id)).copied().collect();
                unknown.sort_unstable();
                tracker_ids.extend(unknown);
                for id in tracker_ids {
                    if official_started.contains(&id) {
                        continue;
                    }
                    let touch = touches.get(&id).or_else(|| extra_flick_touches.get(&id));
                    let Some(touch) = touch.filter(|touch| matches!(touch.phase, TouchPhase::Started | TouchPhase::Moved | TouchPhase::Stationary))
                    else {
                        continue;
                    };
                    if let Some(tracker) = self.trackers.get_mut(&id) {
                        tracker.push(official_frame_time, to_phigros_motion_point(touch.position));
                    }
                }
            }
        }
        let with_event_time = |mut it: Touch| {
            it.time = if it.time.is_infinite() {
                f64::NEG_INFINITY
            } else {
                t - (uptime - it.time) * spd
            };
            it
        };
        let mut event_touch_map = touches.clone();
        event_touch_map.extend(extra_flick_touches);
        let touches = if any_official {
            drain_touches_in_order(touches, &self.finger_order)
        } else {
            touches.into_values().collect()
        }
        .into_iter()
        .map(with_event_time)
        .collect::<Vec<_>>();
        let event_touches = if any_official {
            drain_touches_in_order(event_touch_map, &self.finger_order)
        } else {
            event_touch_map.into_values().collect()
        }
        .into_iter()
        .map(with_event_time)
        .collect::<Vec<_>>();
        fn finite_point(point: Point) -> Option<Point> {
            fn ok(value: f32) -> bool {
                matches!(value.classify(), FpCategory::Zero | FpCategory::Subnormal | FpCategory::Normal)
            }
            (ok(point.x) && ok(point.y)).then_some(point)
        }

        // pos[line][touch]
        let mut pos = Vec::<Vec<Option<Point>>>::with_capacity(chart.lines.len());
        // event_pos is allowed to include the isolated official-Flick
        // MotionEvent fallback described above.
        let mut event_pos = Vec::<Vec<Option<Point>>>::with_capacity(chart.lines.len());
        // active_pos[line][persistent official finger]
        let mut active_pos = Vec::<Vec<Point>>::with_capacity(chart.lines.len());
        for id in 0..chart.lines.len() {
            chart.lines[id].object.set_time(t);
            let inv = chart.lines[id].now_transform(res, &chart.lines).try_inverse().unwrap();
            pos.push(
                touches
                    .iter()
                    .map(|touch| {
                        let p = touch.position;
                        finite_point(inv.transform_point(&Point::new(p.x, -p.y)))
                    })
                    .collect(),
            );
            event_pos.push(
                event_touches
                    .iter()
                    .map(|touch| {
                        let p = touch.position;
                        finite_point(inv.transform_point(&Point::new(p.x, -p.y)))
                    })
                    .collect(),
            );
            active_pos.push(
                self.active_fingers
                    .values()
                    .filter_map(|p| finite_point(inv.transform_point(&Point::new(p.x, -p.y))))
                    .collect(),
            );
        }
        let time_of = |touch: &Touch| {
            if touch.time.is_infinite() {
                t
            } else {
                touch.time
            }
        };
        let mut judgements = Vec::new();
        let mut hold_head_effects = Vec::new();
        // clicks & flicks
        for (id, touch) in event_touches.iter().enumerate() {
            let click = touch.phase == TouchPhase::Started;
            let flick =
                matches!(touch.phase, TouchPhase::Moved | TouchPhase::Stationary) && self.trackers.get_mut(&touch.id).is_some_and(|it| it.flicked);
            if !(click || flick) {
                continue;
            }
            let event_official = if click { other_official } else { flick_official };
            let t = event_judgement_time(t, time_of(touch), event_official);
            if event_official {
                let mut candidates = Vec::new();
                for (line_id, ((line, line_pos), (idx, st))) in chart.lines.iter_mut().zip(event_pos.iter()).zip(self.notes.iter_mut()).enumerate() {
                    let Some(p) = line_pos[id] else {
                        continue;
                    };
                    for note_id in &idx[*st..] {
                        let note = &mut line.notes[*note_id as usize];
                        if !matches!(note.judge, JudgeStatus::NotJudged) {
                            continue;
                        }
                        if (!click && !matches!(note.kind, NoteKind::Flick)) || (click && matches!(note.kind, NoteKind::Flick) && !flick_official) {
                            continue;
                        }

                        let d = (note.time - t) / spd;
                        let broad_early = if click { phigros_windows.bad } else { phigros_windows.flick };
                        if d > broad_early {
                            break;
                        }
                        let late_limit = if click { phigros_windows.good } else { phigros_windows.flick };
                        if d <= -late_limit {
                            continue;
                        }

                        let x = &mut note.object.translation.0;
                        x.set_time(t);
                        let dist = (x.now() - p.x).abs() as f64 / note.judge_area as f64;
                        let x_limit = if click { PHIGROS_TAP_X } else { PHIGROS_DRAG_X };
                        if dist >= x_limit {
                            continue;
                        }
                        let early_limit = if click {
                            let units = dist / PHIGROS_X_UNIT;
                            phigros_windows.bad - 0.5 * phigros_windows.perfect * (units - 0.9).max(0.)
                        } else {
                            phigros_windows.flick
                        };
                        if d > early_limit {
                            continue;
                        }

                        let spatial = dist / PHIGROS_X_UNIT + (p.y as f64 / PHIGROS_X_UNIT / 2.2).abs();
                        candidates.push(OfficialClickCandidate {
                            line_id,
                            note_id: *note_id,
                            time: note.time,
                            spatial,
                            diff: d,
                            kind: OfficialClickKind::from_note(&note.kind),
                        });
                    }
                }

                let closest = if click {
                    choose_official_click_candidate(&mut candidates, t)
                } else {
                    let mut closest = None;
                    for candidate in candidates {
                        let replace = closest.is_none_or(|best: OfficialClickCandidate| {
                            candidate.time < best.time - PHIGROS_CANDIDATE_EPSILON
                                || ((candidate.time - best.time).abs() <= PHIGROS_CANDIDATE_EPSILON && candidate.spatial < best.spatial)
                        });
                        if replace {
                            closest = Some(candidate);
                        }
                    }
                    closest
                };

                if let Some(OfficialClickCandidate {
                    line_id, note_id, diff: d, ..
                }) = closest
                {
                    let note = &mut chart.lines[line_id].notes[note_id as usize];
                    if click {
                        match note.kind {
                            NoteKind::Click => {
                                note.judge = if d >= phigros_windows.good {
                                    // Early Bad is final, but PreJudge keeps the
                                    // visual note alive until it reaches the line.
                                    JudgeStatus::PreJudge
                                } else {
                                    JudgeStatus::Judged
                                };
                                let judgement = if d.abs() < phigros_windows.perfect {
                                    Judgement::Perfect
                                } else if d.abs() < phigros_windows.good {
                                    Judgement::Good
                                } else {
                                    Judgement::Bad
                                };
                                judgements.push((judgement, line_id, note_id, Some(t)));
                            }
                            NoteKind::Hold { .. } => {
                                let pending = d >= phigros_windows.good;
                                let perfect = !pending && d.abs() < phigros_windows.perfect;
                                if !pending {
                                    note.hitsound.play(res);
                                    Self::commit_hold_head(&self.judgements, &self.report_judgements, t, line_id as _, note_id, perfect);
                                    hold_head_effects.push((line_id, note_id, perfect));
                                }
                                note.judge = JudgeStatus::Hold(perfect, t, t, false, f64::INFINITY, 2, pending);
                            }
                            NoteKind::Drag => {
                                note.judge = JudgeStatus::PreJudge;
                            }
                            NoteKind::Flick => {
                                // CheckNote lets Flick occupy the touch-start
                                // candidate but never judges it. CheckFlick is
                                // still the only path that consumes isNewFlick.
                            }
                        }
                    } else {
                        note.judge = JudgeStatus::PreJudge;
                        if let Some(tracker) = self.trackers.get_mut(&touch.id) {
                            tracker.consume();
                        }
                    }
                }
                continue;
            }

            let mut closest = (None, X_DIFF_MAX, LIMIT_BAD, LIMIT_BAD + (X_DIFF_MAX / NOTE_WIDTH_RATIO_BASE - 1.).max(0.) * DIST_FACTOR);
            for (line_id, ((line, pos), (idx, st))) in chart.lines.iter_mut().zip(event_pos.iter()).zip(self.notes.iter_mut()).enumerate() {
                let Some(pos) = pos[id] else {
                    continue;
                };
                for id in &idx[*st..] {
                    let note = &mut line.notes[*id as usize];
                    // With mixed selectors, a note owned by the other engine
                    // must not win this event's Phira candidate search.
                    // Matching selectors retain the original Phira ordering.
                    if other_official != flick_official
                        && (click && matches!(note.kind, NoteKind::Flick) || !click && !matches!(note.kind, NoteKind::Flick))
                    {
                        continue;
                    }
                    if !matches!(note.judge, JudgeStatus::NotJudged | JudgeStatus::PreJudge) {
                        continue;
                    }
                    if !click && matches!(note.kind, NoteKind::Click | NoteKind::Hold { .. }) {
                        continue;
                    }
                    let dt = (note.time - t) / spd;
                    if dt >= closest.3 {
                        break;
                    }
                    let dt = if dt < 0. { (dt + EARLY_OFFSET).min(0.).abs() } else { dt };
                    let x = &mut note.object.translation.0;
                    x.set_time(t);
                    let dist = (x.now() - pos.x).abs() as f64 / note.judge_area as f64;
                    if dist > X_DIFF_MAX {
                        continue;
                    }
                    if dt
                        > if matches!(note.kind, NoteKind::Click) {
                            LIMIT_BAD - LIMIT_PERFECT * (dist - 0.9).max(0.)
                        } else {
                            LIMIT_GOOD
                        }
                    {
                        continue;
                    }
                    let dt = if matches!(note.kind, NoteKind::Flick | NoteKind::Drag) {
                        dt + LIMIT_GOOD
                    } else {
                        dt
                    };
                    let key = dt + (dist / NOTE_WIDTH_RATIO_BASE - 1.).max(0.) * DIST_FACTOR;
                    if key < closest.3 {
                        closest = (Some((line_id, *id)), dist, dt, key);
                    }
                }
            }
            if let (Some((line_id, id)), _, dt, _) = closest {
                let line = &mut chart.lines[line_id];
                if matches!(line.notes[id as usize].kind, NoteKind::Drag) {
                    debug!("reject by drag");
                    continue;
                }
                if click {
                    // click & hold
                    let note = &mut line.notes[id as usize];
                    if matches!(note.kind, NoteKind::Flick) {
                        continue; // to next loop
                    }
                    if dt <= LIMIT_GOOD || matches!(note.kind, NoteKind::Hold { .. }) {
                        match note.kind {
                            NoteKind::Click => {
                                note.judge = JudgeStatus::Judged;
                                judgements.push((if dt <= LIMIT_PERFECT { Judgement::Perfect } else { Judgement::Good }, line_id, id, Some(t)));
                            }
                            NoteKind::Hold { .. } => {
                                note.hitsound.play(res);
                                Self::commit_hold_head(&self.judgements, &self.report_judgements, t, line_id as _, id, dt <= LIMIT_PERFECT);
                                hold_head_effects.push((line_id, id, dt <= LIMIT_PERFECT));
                                note.judge = JudgeStatus::Hold(dt <= LIMIT_PERFECT, t, t, false, f64::INFINITY, 2, false);
                            }
                            _ => unreachable!(),
                        };
                    } else {
                        // prevent extra judgements
                        if matches!(note.judge, JudgeStatus::NotJudged) {
                            // keep the note after bad judgement
                            line.notes[id as usize].judge = JudgeStatus::PreJudge;
                            judgements.push((Judgement::Bad, line_id, id, None));
                        }
                    }
                } else {
                    // flick
                    line.notes[id as usize].judge = JudgeStatus::PreJudge;
                    if let Some(tracker) = self.trackers.get_mut(&touch.id) {
                        tracker.flicked = false;
                    }
                }
            }
        }
        for _ in 0..keys_down {
            // find the earliest not judged click / hold note
            if let Some((line_id, id)) = chart
                .lines
                .iter()
                .zip(self.notes.iter())
                .enumerate()
                .filter_map(|(line_id, (line, (idx, st)))| {
                    idx[*st..]
                        .iter()
                        .cloned()
                        .find(|id| {
                            let note = &line.notes[*id as usize];
                            matches!(note.judge, JudgeStatus::NotJudged) && matches!(note.kind, NoteKind::Click | NoteKind::Hold { .. })
                        })
                        .map(|id| (line_id, id))
                })
                .min_by_key(|(line_id, id)| chart.lines[*line_id].notes[*id as usize].time.not_nan())
            {
                let note = &mut chart.lines[line_id].notes[id as usize];
                let dt = (t - note.time).abs() / spd;
                let hit_limit = if other_official {
                    if matches!(note.kind, NoteKind::Click) {
                        phigros_windows.bad
                    } else {
                        phigros_windows.good
                    }
                } else if matches!(note.kind, NoteKind::Click) {
                    LIMIT_BAD
                } else {
                    LIMIT_GOOD
                };
                if dt <= hit_limit {
                    match note.kind {
                        NoteKind::Click => {
                            note.judge = JudgeStatus::Judged;
                            let perfect_limit = if other_official { phigros_windows.perfect } else { LIMIT_PERFECT };
                            let good_limit = if other_official { phigros_windows.good } else { LIMIT_GOOD };
                            judgements.push((
                                if dt <= perfect_limit {
                                    Judgement::Perfect
                                } else if dt <= good_limit {
                                    Judgement::Good
                                } else {
                                    Judgement::Bad
                                },
                                line_id,
                                id,
                                None,
                            ));
                        }
                        NoteKind::Hold { .. } => {
                            let perfect_limit = if other_official { phigros_windows.perfect } else { LIMIT_PERFECT };
                            note.hitsound.play(res);
                            Self::commit_hold_head(&self.judgements, &self.report_judgements, t, line_id as _, id, dt <= perfect_limit);
                            hold_head_effects.push((line_id, id, dt <= perfect_limit));
                            note.judge = JudgeStatus::Hold(dt <= perfect_limit, t, t, false, f64::INFINITY, 2, false);
                        }
                        _ => unreachable!(),
                    };
                }
            } else {
                break;
            }
        }
        for (line_id, ((line, pos), (idx, st))) in chart.lines.iter_mut().zip(pos.iter()).zip(self.notes.iter()).enumerate() {
            line.object.set_time(t);
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                if let NoteKind::Hold { end_time, .. } = &note.kind {
                    if let JudgeStatus::Hold(
                        perfect,
                        ref mut at,
                        ref mut diff,
                        ref mut pre_judge,
                        ref mut up_time,
                        ref mut safe_frame,
                        ref mut head_pending,
                    ) = note.judge
                    {
                        // Official mode commits a successful Hold inside the
                        // 220 ms tail window, but the note must remain in its
                        // visual Hold state until the tail actually reaches the
                        // judge line.  Once armed, contact is no longer judged.
                        if other_official && *pre_judge {
                            continue;
                        }
                        let x = &mut note.object.translation.0;
                        x.set_time(t);
                        let x = x.now();
                        let x_limit = if other_official { PHIGROS_TAP_X } else { X_DIFF_MAX };
                        let finger_present = self.key_down_count != 0
                            || if other_official {
                                active_pos[line_id]
                                    .iter()
                                    .any(|it| (it.x - x).abs() as f64 / (note.judge_area as f64) < x_limit)
                            } else {
                                pos.iter()
                                    .any(|it| it.is_some_and(|it| (it.x - x).abs() as f64 / (note.judge_area as f64) < x_limit))
                            };
                        if other_official && *head_pending {
                            if !finger_present {
                                if hold_contact_lost(safe_frame) {
                                    note.judge = JudgeStatus::Judged;
                                    judgements.push((Judgement::Miss, line_id, *id, None));
                                }
                            } else {
                                *safe_frame = 2;
                                if (note.time - t) / spd < phigros_windows.good {
                                    *head_pending = false;
                                    *at = t;
                                    *diff = t;
                                    note.hitsound.play(res);
                                    Self::commit_hold_head(&self.judgements, &self.report_judgements, t, line_id as _, *id, false);
                                    hold_head_effects.push((line_id, *id, false));
                                }
                            }
                            continue;
                        }
                        let tail_window = hold_tail_window(&res.config);
                        if (*end_time - t) / spd <= tail_window {
                            if other_official {
                                if arm_hold_visual_tail(pre_judge) {
                                    let judgement = if perfect { Judgement::Perfect } else { Judgement::Good };
                                    let diff = *diff;
                                    judgements.push((judgement, line_id, *id, Some(diff)));
                                }
                            } else {
                                *pre_judge = true;
                            }
                            continue;
                        }
                        if !finger_present {
                            if other_official {
                                if hold_contact_lost(safe_frame) {
                                    note.judge = JudgeStatus::Judged;
                                    judgements.push((Judgement::Miss, line_id, *id, None));
                                }
                            } else if t > *up_time + UP_TOLERANCE {
                                note.judge = JudgeStatus::Judged;
                                judgements.push((Judgement::Miss, line_id, *id, None));
                            } else if up_time.is_infinite() {
                                *up_time = t;
                            }
                        } else {
                            *up_time = f64::INFINITY;
                            *safe_frame = 2;
                        }
                        continue;
                    }
                }
                if !matches!(note.judge, JudgeStatus::NotJudged) {
                    continue;
                }
                // process miss
                let dt = (t - note.time) / spd;
                let official = note_uses_phigros_judgement(&note.kind, other_official, flick_official);
                let miss_limit = if official {
                    match note.kind {
                        NoteKind::Click | NoteKind::Hold { .. } => phigros_windows.good,
                        NoteKind::Drag => PHIGROS_DRAG_WINDOW,
                        NoteKind::Flick => phigros_windows.flick,
                    }
                } else {
                    LIMIT_BAD
                };
                if dt > miss_limit {
                    note.judge = JudgeStatus::Judged;
                    judgements.push((Judgement::Miss, line_id, *id, None));
                    continue;
                }
                if -dt > if official { phigros_windows.bad } else { LIMIT_BAD } {
                    break;
                }
                if official {
                    if !matches!(note.kind, NoteKind::Drag) || dt.abs() > PHIGROS_DRAG_WINDOW {
                        continue;
                    }
                    let x = &mut note.object.translation.0;
                    x.set_time(t);
                    let x = x.now();
                    if self.key_down_count != 0
                        || active_pos[line_id]
                            .iter()
                            .any(|it| (it.x - x).abs() as f64 / (note.judge_area as f64) < PHIGROS_DRAG_X)
                    {
                        note.judge = JudgeStatus::PreJudge;
                    }
                    continue;
                }
                if !matches!(note.kind, NoteKind::Drag) && (self.key_down_count == 0 || !matches!(note.kind, NoteKind::Flick)) {
                    continue;
                }
                let dt = dt.abs();
                let x = &mut note.object.translation.0;
                x.set_time(t);
                let x = x.now();
                if self.key_down_count != 0
                    || pos.iter().any(|it| {
                        it.is_some_and(|it| {
                            let dx = (it.x - x).abs() as f64 / note.judge_area as f64;
                            dx <= X_DIFF_MAX && dt <= (LIMIT_BAD - LIMIT_PERFECT * (dx - 0.9).max(0.))
                        })
                    })
                {
                    note.judge = JudgeStatus::PreJudge;
                }
            }
        }
        // process pre-judge
        for (line_id, (line, (idx, st))) in chart.lines.iter_mut().zip(self.notes.iter()).enumerate() {
            line.object.set_time(t);
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                let official = note_uses_phigros_judgement(&note.kind, other_official, flick_official);
                if let JudgeStatus::Hold(perfect, .., diff, tail_armed, _, _, _) = note.judge {
                    if let NoteKind::Hold { end_time, .. } = &note.kind {
                        if hold_visual_tail_ended(tail_armed, *end_time, t) {
                            note.judge = JudgeStatus::Judged;
                            if !official {
                                judgements.push((if perfect { Judgement::Perfect } else { Judgement::Good }, line_id, *id, Some(diff)));
                            }
                            continue;
                        }
                    }
                }
                // TODO adjust
                let ghost_t = t + if official { phigros_windows.good } else { LIMIT_GOOD };
                if matches!(note.kind, NoteKind::Click) {
                    if ghost_t < note.time {
                        break;
                    }
                } else if official {
                    if (note.time - t) / spd >= PHIGROS_RESOLVE_EARLY {
                        continue;
                    }
                } else if t < note.time {
                    continue;
                }
                if matches!(note.judge, JudgeStatus::PreJudge) {
                    let diff = if let JudgeStatus::Hold(.., diff, _, _, _, _) = note.judge {
                        Some(diff)
                    } else {
                        None
                    };
                    note.judge = JudgeStatus::Judged;
                    if !matches!(note.kind, NoteKind::Click) {
                        judgements.push((Judgement::Perfect, line_id, *id, diff));
                    }
                }
            }
        }
        for (judgement, line_id, id, diff) in judgements {
            let line = &mut chart.lines[line_id];
            let note = &mut line.notes[id as usize];
            line.object.set_time(t);
            note.object.set_time(t);
            let line = &chart.lines[line_id];
            let note = &line.notes[id as usize];
            let line_tr = line.now_transform(res, &chart.lines);
            self.commit(
                t,
                judgement,
                line_id as _,
                id,
                if matches!(judgement, Judgement::Miss) {
                    0.25
                } else if matches!(note.kind, NoteKind::Drag | NoteKind::Flick) {
                    0.
                } else {
                    (diff.unwrap_or(t) - note.time) / spd
                },
            );
            if matches!(note.kind, NoteKind::Hold { .. }) {
                continue;
            }
            if match judgement {
                Judgement::Perfect => {
                    res.with_model(line_tr * note.object.now(res), |res| {
                        res.emit_at_origin(note.rotation(line), note.fx_color.unwrap_or_else(|| res.res_pack.info.fx_perfect()))
                    });
                    true
                }
                Judgement::Good => {
                    res.with_model(line_tr * note.object.now(res), |res| {
                        res.emit_at_origin(note.rotation(line), note.fx_color.unwrap_or_else(|| res.res_pack.info.fx_good()))
                    });
                    true
                }
                Judgement::Bad => {
                    if !matches!(note.kind, NoteKind::Hold { .. }) {
                        bad_notes.push(BadNote {
                            time: t,
                            kind: note.kind.clone(),
                            matrix: {
                                let mut mat = line_tr;
                                if !note.above {
                                    mat.append_nonuniform_scaling_mut(&Vector::new(1., -1.));
                                }
                                let incline_sin = line.incline.now_opt().map(|it| it.to_radians().sin()).unwrap_or_default();
                                mat *= note.now_transform(
                                    res,
                                    &line.ctrl_obj.borrow_mut(),
                                    ((note.height - line.height.now() as f64) / res.aspect_ratio as f64 * note.speed) as f32,
                                    incline_sin,
                                );
                                mat
                            },
                        });
                    }
                    false
                }
                _ => false,
            } {
                note.hitsound.play(res);
            }
        }
        for (line, (idx, st)) in chart.lines.iter().zip(self.notes.iter_mut()) {
            while idx
                .get(*st)
                .is_some_and(|id| matches!(line.notes[*id as usize].judge, JudgeStatus::Judged))
            {
                *st += 1;
            }
        }
        if touch_debug_enabled {
            let mut persistent_snapshot = Vec::with_capacity(pending_snapshot_debug.len());
            for pending in pending_snapshot_debug {
                let judge_point = pending.touch.position;
                let phigros_point = to_phigros_motion_point(judge_point);
                let sequence = self.next_touch_debug_sequence();
                persistent_snapshot.push(TouchDebugSample {
                    sequence,
                    finger_id: pending.touch.id,
                    phase: pending.touch.phase.into(),
                    source: pending.source,
                    platform_event_time_seconds: pending.touch.time.is_finite().then_some(pending.touch.time),
                    captured_uptime_seconds: uptime,
                    frame_judge_time_seconds: t,
                    mapped_judge_time_seconds: t,
                    tap_drag_hold_effective_time_seconds: t,
                    flick_effective_time_seconds: t,
                    screen_x_px: pending.raw_position.x,
                    screen_y_px: pending.raw_position.y,
                    judge_x: judge_point.x,
                    judge_y: judge_point.y,
                    phigros_motion_x: phigros_point.x,
                    phigros_motion_y: phigros_point.y,
                    present_in_persistent_snapshot: true,
                });
            }
            debug_tracker_started_ids.sort_unstable();
            debug_tracker_started_ids.dedup();
            debug_tracker_rebound_ids.sort_unstable();
            debug_tracker_rebound_ids.dedup();
            debug_tracker_removed_ids.sort_unstable();
            debug_tracker_removed_ids.dedup();
            debug_synthesized_stationary_ids.sort_unstable();
            let sequence = self.next_touch_debug_sequence();
            let frame_index = self.touch_debug_frame_index;
            self.touch_debug_frame_index = self.touch_debug_frame_index.saturating_add(1);
            self.touch_debug_records.push(JudgeTouchDebugRecord::Frame(Box::new(TouchDebugFrame {
                sequence,
                frame_index,
                judge_time_seconds: t,
                tracker_clock_seconds: t / spd,
                captured_uptime_seconds: uptime,
                frame_delta_seconds: get_frame_time() as f64,
                tap_drag_hold_uses_phigros: other_official,
                flick_uses_phigros: flick_official,
                raw_events: debug_raw_events,
                persistent_snapshot,
                active_fingers_before: debug_active_before.unwrap_or_default(),
                active_fingers_after: touch_debug_finger_states(&self.active_fingers),
                finger_order_before: debug_order_before.unwrap_or_default(),
                finger_order_after: self.finger_order.clone(),
                trackers_before: debug_trackers_before.unwrap_or_default(),
                trackers_after: touch_debug_tracker_states(&self.trackers),
                tracker_started_ids: debug_tracker_started_ids,
                tracker_rebound_ids: debug_tracker_rebound_ids,
                tracker_removed_ids: debug_tracker_removed_ids,
                synthesized_stationary_ids: debug_synthesized_stationary_ids,
                key_down_count_before: debug_key_down_before,
                key_down_count_after: self.key_down_count,
                raw_key_delta,
            })));
        }
        Self::emit_hold_head_effects(res, chart, hold_head_effects);
        self.last_time = t / spd;
    }

    /// Consume only newly committed heads; held frames and tail outcomes never enter this list.
    fn emit_hold_head_effects(res: &mut Resource, chart: &mut Chart, heads: Vec<(usize, u32, bool)>) {
        if !res.config.hold_head_effect || !res.config.particle {
            return;
        }
        for (line_id, id, perfect) in heads {
            let line = &mut chart.lines[line_id];
            line.object.set_time(res.time);
            let note = &mut line.notes[id as usize];
            if note.fake {
                continue;
            }
            note.object.set_time(res.time);
            // This burst replaces the first periodic burst, so the next frame cannot double-flash.
            if let JudgeStatus::Hold(_, ref mut at, ..) = note.judge {
                *at = res.time + crate::core::HOLD_PARTICLE_INTERVAL / res.config.speed as f64;
            }
            let line = &chart.lines[line_id];
            let note = &line.notes[id as usize];
            // Project to the judge line: animated note x, zero local y, then the full line transform.
            let transform = line.now_transform(res, &chart.lines) * Matrix::new_translation(&Vector::new(note.object.translation.0.now(), 0.));
            let color = note.fx_color.unwrap_or_else(|| {
                if perfect {
                    res.res_pack.info.fx_perfect()
                } else {
                    res.res_pack.info.fx_good()
                }
            });
            res.with_model(transform, |res| res.emit_at_origin(note.rotation(line), color));
        }
    }

    fn auto_play_update(&mut self, res: &mut Resource, chart: &mut Chart) {
        let t = res.time;
        let spd = res.config.speed as f64;
        let mut judgements = Vec::new();
        let mut hold_head_effects = Vec::new();
        for (line_id, (line, (idx, st))) in chart.lines.iter_mut().zip(self.notes.iter_mut()).enumerate() {
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                if let JudgeStatus::Hold(..) = note.judge {
                    if let NoteKind::Hold { end_time, .. } = note.kind {
                        if t >= end_time {
                            note.judge = JudgeStatus::Judged;
                            judgements.push((line_id, *id));
                            continue;
                        }
                    }
                }
                if !matches!(note.judge, JudgeStatus::NotJudged) {
                    continue;
                }
                if note.time > t {
                    break;
                }
                note.judge = if matches!(note.kind, NoteKind::Hold { .. }) {
                    note.hitsound.play(res);
                    Self::commit_hold_head(&self.judgements, &self.report_judgements, t, line_id as _, *id, true);
                    hold_head_effects.push((line_id, *id, true));
                    JudgeStatus::Hold(true, t, (t - note.time) / spd, false, f64::INFINITY, 2, false)
                } else {
                    judgements.push((line_id, *id));
                    JudgeStatus::Judged
                };
            }
            while idx
                .get(*st)
                .is_some_and(|id| matches!(line.notes[*id as usize].judge, JudgeStatus::Judged))
            {
                *st += 1;
            }
        }
        Self::emit_hold_head_effects(res, chart, hold_head_effects);
        for (line_id, id) in judgements.into_iter() {
            self.commit(t, Judgement::Perfect, line_id as _, id, 0.);
            let (note_transform, note_hitsound) = {
                let line = &mut chart.lines[line_id];
                let note = &mut line.notes[id as usize];
                let nt = if matches!(note.kind, NoteKind::Hold { .. }) { t } else { note.time };
                line.object.set_time(nt);
                note.object.set_time(nt);
                (note.object.now(res), note.hitsound.clone())
            };
            let line = &chart.lines[line_id];
            res.with_model(line.now_transform(res, &chart.lines) * note_transform, |res| {
                res.emit_at_origin(line.notes[id as usize].rotation(line), res.res_pack.info.fx_perfect())
            });
            if !matches!(chart.lines[line_id].notes[id as usize].kind, NoteKind::Hold { .. }) {
                note_hitsound.play(res);
            }
        }
    }

    #[inline]
    pub fn result(&self) -> PlayResult {
        self.inner.result()
    }

    #[inline]
    pub fn combo(&self) -> u32 {
        self.inner.combo()
    }

    #[inline]
    pub fn counts(&self) -> [u32; 4] {
        self.inner.counts()
    }
}

struct Handler {
    status: TouchStatus,
    wheel: (f32, f32),
}
impl Handler {
    fn finalize(&mut self) {
        if is_mouse_button_down(MouseButton::Left) {
            self.status.touches.push(Touch {
                id: button_to_id(MouseButton::Left),
                phase: TouchPhase::Moved,
                position: mouse_position().into(),
                time: f64::NEG_INFINITY,
            });
        }
    }
}

fn button_to_id(button: MouseButton) -> u64 {
    u64::MAX
        - match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::Unknown => 3,
        }
}

impl EventHandler for Handler {
    fn update(&mut self, _: &mut miniquad::Context) {}
    fn draw(&mut self, _: &mut miniquad::Context) {}
    fn touch_event(&mut self, _: &mut miniquad::Context, phase: miniquad::TouchPhase, id: u64, x: f32, y: f32, time: f64) {
        self.status.touches.push(Touch {
            id,
            phase: phase.into(),
            position: vec2(x, y),
            time,
        });
    }

    fn mouse_wheel_event(&mut self, _ctx: &mut miniquad::Context, x: f32, y: f32) {
        self.wheel.0 += x;
        self.wheel.1 += y;
    }

    fn mouse_button_down_event(&mut self, _ctx: &mut miniquad::Context, button: MouseButton, x: f32, y: f32) {
        self.status.touches.push(Touch {
            id: button_to_id(button),
            phase: TouchPhase::Started,
            position: vec2(x, y),
            time: f64::NEG_INFINITY,
        });
    }

    fn mouse_button_up_event(&mut self, _ctx: &mut miniquad::Context, button: MouseButton, x: f32, y: f32) {
        self.status.touches.push(Touch {
            id: button_to_id(button),
            phase: TouchPhase::Ended,
            position: vec2(x, y),
            time: f64::NEG_INFINITY,
        });
    }

    fn key_down_event(&mut self, _ctx: &mut miniquad::Context, _keycode: KeyCode, _keymods: miniquad::KeyMods, repeat: bool) {
        if !repeat {
            self.status.key_delta += 1;
            self.status.keys_down += 1;
        }
    }

    fn key_up_event(&mut self, _ctx: &mut miniquad::Context, _keycode: KeyCode, _keymods: miniquad::KeyMods) {
        self.status.key_delta -= 1;
    }
}

#[cfg(test)]
mod replica_input_tests {
    use super::*;
    use crate::core::{BpmList, ChartExtra, ChartSettings};

    #[test]
    fn report_queue_is_an_independent_exact_copy_of_judge_events() {
        let chart = Chart::new(0., Vec::new(), BpmList::default(), ChartSettings::default(), ChartExtra::default(), HashMap::new());
        let mut judge = Judge::new(&chart);
        judge.commit(1.25, Judgement::Good, 3, 7, -0.0125);
        Judge::commit_hold_head(&judge.judgements, &judge.report_judgements, 2.5, 4, 8, true);

        let events = judge.take_report_judgements();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, 1.25);
        assert_eq!(events[0].line_id, 3);
        assert_eq!(events[0].note_id, 7);
        assert_eq!(events[0].judgement, Ok(Judgement::Good));
        assert_eq!(events[0].difference, -0.0125);
        assert_eq!(events[1].judgement, Err(true));
        assert!(judge.take_report_judgements().is_empty());
        assert_eq!(judge.judgements.borrow().len(), 2);
    }

    #[test]
    fn flick_motion_is_converted_to_phigros_units() {
        let point = vec2(PHIGROS_X_UNIT as f32 * 0.31, 0.);
        let converted = to_phigros_motion_point(point);
        assert!((converted.x - 0.31).abs() < 1e-6);
    }

    #[test]
    fn missing_snapshot_clears_unconsumed_flick_on_stationary_frame() {
        let point = vec2(PHIGROS_X_UNIT as f32 * 0.4, 0.);
        let mut tracker = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        tracker.push(1. / 60., to_phigros_motion_point(point));
        assert!(tracker.flicked);
        let mut touches = HashMap::new();
        let active = HashMap::from([(7, point)]);
        let mut debug = Vec::new();
        complete_finger_snapshots(&mut touches, &active, Some(&mut debug));
        tracker.push(2. / 60., to_phigros_motion_point(touches[&7].position));
        assert!(!tracker.flicked);
        assert!(tracker.stopped);
        assert_eq!(debug, vec![7]);
    }

    #[test]
    fn missing_snapshot_rearms_consumed_flick_before_next_swipe() {
        let point = vec2(PHIGROS_X_UNIT as f32 * 0.4, 0.);
        let mut tracker = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        tracker.push(1. / 60., to_phigros_motion_point(point));
        tracker.consume();
        let mut touches = HashMap::new();
        complete_finger_snapshots(&mut touches, &HashMap::from([(7, point)]), None);
        tracker.push(2. / 60., to_phigros_motion_point(touches[&7].position));
        tracker.push(3. / 60., Point::new(1.2, 0.));
        assert!(tracker.flicked);
    }

    #[test]
    fn snapshot_completion_preserves_fresh_input_and_excludes_terminal_fingers() {
        let mut active = HashMap::from([(1, vec2(0., 0.)), (2, vec2(1., 1.)), (3, vec2(2., 2.))]);
        update_active_finger(&mut active, 2, TouchPhase::Cancelled, vec2(1., 1.));
        update_active_finger(&mut active, 3, TouchPhase::Ended, vec2(2., 2.));
        let mut touches = HashMap::from([(
            1,
            Touch {
                id: 1,
                phase: TouchPhase::Moved,
                position: vec2(0.5, 0.5),
                time: 123.,
            },
        )]);
        complete_finger_snapshots(&mut touches, &active, None);
        assert_eq!(touches.len(), 1);
        assert_eq!(touches[&1].phase, TouchPhase::Moved);
        assert_eq!(touches[&1].position, vec2(0.5, 0.5));
        assert_eq!(touches[&1].time, 123.);
    }

    #[test]
    fn official_flick_uses_documented_threshold_and_consumption() {
        let mut below = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        below.push(1. / 60., Point::new(0.29, 0.));
        assert!(!below.flicked);

        let mut hit = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        hit.push(1. / 60., Point::new(0.31, 0.));
        assert!(hit.flicked);
        hit.consume();
        assert!(!hit.flicked);
    }

    #[test]
    fn official_flick_velocity_uses_one_sample_per_frame() {
        let mut once = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        once.push(1. / 60., Point::new(0.2, 0.));
        assert!(!once.flicked);

        // Splitting that same frame interval around unrelated MotionEvents
        // would incorrectly double the measured speed and trigger a Flick.
        let mut incorrectly_split = FlickTracker::new(380, 0., Point::new(0., 0.), true);
        incorrectly_split.push(1. / 120., Point::new(0.2, 0.));
        assert!(incorrectly_split.flicked);
    }

    #[test]
    fn official_events_use_the_frame_clock_but_phira_keeps_raw_time() {
        assert_eq!(event_judgement_time(10., 9.95, true), 10.);
        assert_eq!(event_judgement_time(10., 9.95, false), 9.95);
        assert_eq!(event_judgement_time(10., f64::NEG_INFINITY, false), 10.);
    }

    #[test]
    fn active_finger_persists_until_terminal_event() {
        let mut fingers = HashMap::new();
        update_active_finger(&mut fingers, 7, TouchPhase::Started, vec2(0.1, 0.2));
        assert_eq!(fingers.get(&7).map(|it| (it.x, it.y)), Some((0.1, 0.2)));

        // A frame without a new MotionEvent must not drop a held finger.
        assert!(fingers.contains_key(&7));
        update_active_finger(&mut fingers, 7, TouchPhase::Ended, vec2(0.1, 0.2));
        assert!(!fingers.contains_key(&7));
    }

    #[test]
    fn finger_order_is_stable_and_empty_snapshot_is_non_destructive() {
        let mut order = Vec::new();
        remember_finger(&mut order, 9);
        remember_finger(&mut order, 3);
        remember_finger(&mut order, 9);
        assert_eq!(order, vec![9, 3]);

        append_missing_fingers(&mut order, &HashSet::new());
        assert_eq!(order, vec![9, 3]);

        let active = HashSet::from([3, 7]);
        append_missing_fingers(&mut order, &active);
        assert_eq!(order, vec![9, 3, 7]);

        let touches = HashMap::from([
            (
                7,
                Touch {
                    id: 7,
                    phase: TouchPhase::Stationary,
                    position: vec2(0., 0.),
                    time: 0.,
                },
            ),
            (
                3,
                Touch {
                    id: 3,
                    phase: TouchPhase::Stationary,
                    position: vec2(0., 0.),
                    time: 0.,
                },
            ),
        ]);
        let ids: Vec<_> = drain_touches_in_order(touches, &order).into_iter().map(|touch| touch.id).collect();
        assert_eq!(ids, vec![3, 7]);
    }

    #[test]
    fn orphan_move_rebind_starts_from_zero_velocity() {
        let mut trackers = HashMap::new();
        let position = vec2(0.75 * PHIGROS_X_UNIT as f32, -0.25 * PHIGROS_X_UNIT as f32);
        assert!(rebind_official_flick_tracker(&mut trackers, 4, 380, 1., position));
        assert!(!rebind_official_flick_tracker(&mut trackers, 4, 380, 2., vec2(9., 9.)));

        let tracker = trackers.get_mut(&4).unwrap();
        tracker.push(1. + 1. / 60., to_phigros_motion_point(position));
        assert!(!tracker.flicked);
        assert!(tracker.stopped);
    }

    #[test]
    fn explicit_lifecycle_cleanup_clears_all_touch_state() {
        let chart = Chart::new(0., Vec::new(), BpmList::default(), ChartSettings::default(), ChartExtra::default(), HashMap::new());
        let mut judge = Judge::new(&chart);
        judge.trackers.insert(4, FlickTracker::new(380, 0., Point::new(0., 0.), true));
        judge.active_fingers.insert(4, vec2(0., 0.));
        judge.finger_order.push(4);

        judge.clear_touch_input();

        assert!(judge.trackers.is_empty());
        assert!(judge.active_fingers.is_empty());
        assert!(judge.finger_order.is_empty());
    }

    #[test]
    fn official_touch_start_candidate_matches_phigros_type_priority() {
        fn candidate(time: f64, spatial: f64, kind: OfficialClickKind) -> OfficialClickCandidate {
            OfficialClickCandidate {
                line_id: 0,
                note_id: 0,
                time,
                spatial,
                diff: time - 1.,
                kind,
            }
        }

        let mut weak_then_tap = [
            candidate(0.990, 0.1, OfficialClickKind::Drag),
            candidate(1.000, 9., OfficialClickKind::Tap),
        ];
        assert_eq!(choose_official_click_candidate(&mut weak_then_tap, 1.).unwrap().kind, OfficialClickKind::Tap);

        let mut tap_then_weak = [
            candidate(0.990, 9., OfficialClickKind::Tap),
            candidate(1.000, 0.1, OfficialClickKind::Flick),
        ];
        assert_eq!(choose_official_click_candidate(&mut tap_then_weak, 1.).unwrap().kind, OfficialClickKind::Tap);

        let mut tap_tie = [
            candidate(0.990, 9., OfficialClickKind::Tap),
            candidate(0.995, 0.1, OfficialClickKind::Hold),
        ];
        assert_eq!(choose_official_click_candidate(&mut tap_tie, 1.).unwrap().kind, OfficialClickKind::Hold);

        let mut weak_notes = [
            candidate(0.900, 0.1, OfficialClickKind::Drag),
            candidate(0.950, 0.1, OfficialClickKind::Flick),
        ];
        assert_eq!(choose_official_click_candidate(&mut weak_notes, 1.).unwrap().kind, OfficialClickKind::Flick);
    }

    #[test]
    fn hold_safe_frame_rule_is_unchanged() {
        let mut safe_frame = 2;
        assert!(!hold_contact_lost(&mut safe_frame));
        assert_eq!(safe_frame, 1);
        assert!(!hold_contact_lost(&mut safe_frame));
        assert_eq!(safe_frame, 0);
        assert!(!hold_contact_lost(&mut safe_frame));
        assert_eq!(safe_frame, -1);
        assert!(hold_contact_lost(&mut safe_frame));
    }

    #[test]
    fn completed_hold_keeps_its_visual_tail_until_end_time() {
        let mut tail_armed = false;
        assert!(arm_hold_visual_tail(&mut tail_armed));
        assert!(tail_armed);
        assert!(!arm_hold_visual_tail(&mut tail_armed));

        let end_time = 10.;
        assert!(!hold_visual_tail_ended(tail_armed, end_time, end_time - PHIGROS_HOLD_TAIL));
        assert!(!hold_visual_tail_ended(tail_armed, end_time, end_time - 1e-9));
        assert!(hold_visual_tail_ended(tail_armed, end_time, end_time));
    }

    #[test]
    fn split_modes_route_only_flick_to_the_flick_selector() {
        let click = NoteKind::Click;
        let drag = NoteKind::Drag;
        let hold = NoteKind::Hold {
            end_time: 1.,
            end_height: 1.,
        };
        let flick = NoteKind::Flick;

        assert!(!note_uses_phigros_judgement(&click, false, true));
        assert!(!note_uses_phigros_judgement(&drag, false, true));
        assert!(!note_uses_phigros_judgement(&hold, false, true));
        assert!(note_uses_phigros_judgement(&flick, false, true));

        assert!(note_uses_phigros_judgement(&click, true, false));
        assert!(note_uses_phigros_judgement(&drag, true, false));
        assert!(note_uses_phigros_judgement(&hold, true, false));
        assert!(!note_uses_phigros_judgement(&flick, true, false));
    }

    #[test]
    fn strict_windows_include_half_of_the_average_of_up_to_ten_frames() {
        let mut frames = RecentFrameTimes::default();
        for _ in 0..10 {
            frames.push(1. / 120.);
        }
        let windows = PhigrosWindows::strict(frames.average());
        assert!((windows.perfect - 0.044_166_666_666_666_67).abs() < 1e-12);
        assert!((windows.good - 0.094_166_666_666_666_66).abs() < 1e-12);
        assert!((windows.bad - 0.144_166_666_666_67).abs() < 1e-12);
        assert!((windows.flick - windows.perfect * 1.75).abs() < 1e-12);

        frames.clear();
        for _ in 0..10 {
            frames.push(1. / 60.);
        }
        let windows = PhigrosWindows::strict(frames.average());
        assert!((windows.perfect - 0.048_333_333_333_333_33).abs() < 1e-12);
        assert!((windows.good - 0.098_333_333_333_333_33).abs() < 1e-12);
        assert!((windows.bad - 0.148_333_333_333_333_34).abs() < 1e-12);
        assert!((windows.flick - 0.084_583_333_333_333_33).abs() < 1e-12);
    }

    #[test]
    fn recent_frame_average_keeps_only_the_latest_ten_samples() {
        let mut frames = RecentFrameTimes::default();
        for _ in 0..10 {
            frames.push(0.01);
        }
        frames.push(0.02);
        assert!((frames.average() - 0.011).abs() < 1e-12);
    }

    #[test]
    fn normal_phigros_windows_are_unchanged() {
        let windows = PhigrosWindows::normal();
        assert_eq!(windows.perfect, PHIGROS_LIMIT_PERFECT);
        assert_eq!(windows.good, PHIGROS_LIMIT_GOOD);
        assert_eq!(windows.bad, PHIGROS_LIMIT_BAD);
        assert_eq!(windows.flick, PHIGROS_FLICK_WINDOW);
    }

    #[test]
    fn phira_debug_windows_include_original_late_compensation() {
        let perfect = JudgementTimeWindow::phira(LIMIT_PERFECT);
        let bad = JudgementTimeWindow::phira(LIMIT_BAD);
        assert_eq!(perfect.early, LIMIT_PERFECT);
        assert_eq!(perfect.late, LIMIT_PERFECT + EARLY_OFFSET);
        assert_eq!(bad.early, LIMIT_BAD);
        assert_eq!(bad.late, LIMIT_BAD + EARLY_OFFSET);
    }
}

#[derive(Default)]
pub struct PlayResult {
    pub score: u32,
    pub accuracy: f64,
    pub max_combo: u32,
    pub num_of_notes: u32,
    pub counts: [u32; 4],
    pub early: u32,
    pub late: u32,
    pub std: f32,
    pub early_kind: [u32; 4],
    pub late_kind: [u32; 4],
}

pub fn icon_index(score: u32, full_combo: bool) -> usize {
    match (score, full_combo) {
        (x, _) if x < 700000 => 0,
        (x, _) if x < 820000 => 1,
        (x, _) if x < 880000 => 2,
        (x, _) if x < 920000 => 3,
        (x, _) if x < 960000 => 4,
        (1000000, _) => 7,
        (_, false) => 5,
        (_, true) => 6,
    }
}
