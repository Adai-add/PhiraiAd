//! Per-attempt gameplay report collection.
//!
//! The judge remains the sole authority for note outcomes.  This module only
//! consumes the judge's read-only event copy and serializes derived statistics.

use crate::{
    config::{Config, JudgementMode, Mods},
    core::{Chart, NoteKind},
    info::{ChartFormat, ChartInfo},
    judge::{
        JudgeReportEvent, JudgeTouchDebugRecord, Judgement, PlayResult, TouchDebugClearEvent, TouchDebugClearSource, TouchDebugFingerState,
        TouchDebugFrame, TouchDebugPhase, TouchDebugSample, TouchDebugSampleSource, TouchDebugTrackerState,
    },
};
use chrono::{DateTime, Local};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub const REPORT_SCHEMA_VERSION: u32 = 2;
pub const REPLICA_VERSION: &str = "0.8.2-replica.23.0-bugfix1";
const TOUCH_DEBUG_SCHEMA_VERSION: u32 = 1;
const TOUCH_DEBUG_MAX_FRAMES: usize = 200_000;
const TOUCH_DEBUG_MAX_SAMPLES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportEndReason {
    Completed,
    Retry,
    Exit,
    PracticeRangeCompleted,
    PracticeSettingsChanged,
}

impl ReportEndReason {
    fn is_complete(self) -> bool {
        matches!(self, Self::Completed | Self::PracticeRangeCompleted)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportGameMode {
    Normal,
    Exercise,
    NoRetry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportNoteKind {
    Tap,
    Hold,
    Drag,
    Flick,
}

impl From<&NoteKind> for ReportNoteKind {
    fn from(value: &NoteKind) -> Self {
        match value {
            NoteKind::Click => Self::Tap,
            NoteKind::Hold { .. } => Self::Hold,
            NoteKind::Drag => Self::Drag,
            NoteKind::Flick => Self::Flick,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportJudgement {
    Perfect,
    Good,
    Bad,
    Miss,
}

impl From<Judgement> for ReportJudgement {
    fn from(value: Judgement) -> Self {
        match value {
            Judgement::Perfect => Self::Perfect,
            Judgement::Good => Self::Good,
            Judgement::Bad => Self::Bad,
            Judgement::Miss => Self::Miss,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteTypeCounts {
    pub total: u32,
    pub tap: u32,
    pub hold: u32,
    pub drag: u32,
    pub flick: u32,
}

impl NoteTypeCounts {
    fn add(&mut self, kind: ReportNoteKind) {
        self.total += 1;
        match kind {
            ReportNoteKind::Tap => self.tap += 1,
            ReportNoteKind::Hold => self.hold += 1,
            ReportNoteKind::Drag => self.drag += 1,
            ReportNoteKind::Flick => self.flick += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeCounts {
    pub perfect: u32,
    pub good: u32,
    pub bad: u32,
    pub miss: u32,
}

impl GradeCounts {
    fn add(&mut self, judgement: ReportJudgement) {
        match judgement {
            ReportJudgement::Perfect => self.perfect += 1,
            ReportJudgement::Good => self.good += 1,
            ReportJudgement::Bad => self.bad += 1,
            ReportJudgement::Miss => self.miss += 1,
        }
    }

    fn total(self) -> u32 {
        self.perfect + self.good + self.bad + self.miss
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryCounts {
    pub perfect: u32,
    pub miss: u32,
}

impl BinaryCounts {
    fn add(&mut self, judgement: ReportJudgement) {
        match judgement {
            ReportJudgement::Miss => self.miss += 1,
            _ => self.perfect += 1,
        }
    }

    fn total(self) -> u32 {
        self.perfect + self.miss
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldCompletionCounts {
    pub completed: u32,
    pub released_early: u32,
    pub head_missed: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteResultRecord {
    pub global_note_no: u32,
    pub line_id: u32,
    pub line_note_id: u32,
    pub note_type: ReportNoteKind,
    pub phase: &'static str,
    pub note_time_seconds: f64,
    pub judged_time_seconds: Option<f64>,
    pub judgement: ReportJudgement,
    /// Positive means early and negative means late, as requested by the user.
    pub delay_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub miss_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelayHistogramBucket {
    pub from_ms: f64,
    pub to_ms: f64,
    pub count: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelayStatistics {
    pub sample_count: u32,
    pub early_count: u32,
    pub late_count: u32,
    pub exact_count: u32,
    pub mean_ms: Option<f64>,
    pub median_ms: Option<f64>,
    /// Population variance (division by N), in ms².
    pub variance_ms2: Option<f64>,
    pub standard_deviation_ms: Option<f64>,
    pub mean_absolute_delay_ms: Option<f64>,
    pub maximum_early_ms: Option<f64>,
    pub maximum_late_ms: Option<f64>,
    pub histogram_10ms: Vec<DelayHistogramBucket>,
}

impl DelayStatistics {
    fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let median = if n.is_multiple_of(2) {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.
        } else {
            sorted[n / 2]
        };
        let variance = sorted.iter().map(|it| (it - mean).powi(2)).sum::<f64>() / n as f64;
        let mean_abs = sorted.iter().map(|it| it.abs()).sum::<f64>() / n as f64;

        let first_bucket = (sorted[0] / 10.).floor() as i32;
        let last_bucket = (sorted[n - 1] / 10.).floor() as i32;
        let mut histogram = Vec::with_capacity((last_bucket - first_bucket + 1).max(0) as usize);
        for bucket in first_bucket..=last_bucket {
            let from = bucket as f64 * 10.;
            histogram.push(DelayHistogramBucket {
                from_ms: from,
                to_ms: from + 10.,
                count: sorted.iter().filter(|value| **value >= from && **value < from + 10.).count() as u32,
            });
        }

        Self {
            sample_count: n as u32,
            early_count: sorted.iter().filter(|it| **it > 0.).count() as u32,
            late_count: sorted.iter().filter(|it| **it < 0.).count() as u32,
            exact_count: sorted.iter().filter(|it| **it == 0.).count() as u32,
            mean_ms: Some(mean),
            median_ms: Some(median),
            variance_ms2: Some(variance),
            standard_deviation_ms: Some(variance.sqrt()),
            mean_absolute_delay_ms: Some(mean_abs),
            maximum_early_ms: sorted.iter().copied().filter(|it| *it > 0.).max_by(f64::total_cmp),
            maximum_late_ms: sorted.iter().copied().filter(|it| *it < 0.).min_by(f64::total_cmp),
            histogram_10ms: histogram,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingSummary {
    pub sign_convention: &'static str,
    pub tap: DelayStatistics,
    pub hold_head: DelayStatistics,
    pub combined: DelayStatistics,
    /// Keys are stable, one-based note numbers in the whole chart.  Missed
    /// heads are present with a null value because no input delay exists.
    pub delay_by_global_note_no_ms: PerNoteDelays,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerNoteDelays {
    pub tap: BTreeMap<u32, Option<f64>>,
    pub hold_head: BTreeMap<u32, Option<f64>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSong {
    pub name: String,
    pub level: String,
    pub difficulty: f32,
    pub composer: String,
    pub charter: String,
    pub chart_id: Option<i32>,
    pub chart_format: ChartFormat,
    pub chart_sha256: String,
    pub replica_play: crate::chart_play::ChartPlaySettings,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSettings {
    pub mode: ReportGameMode,
    pub playback_speed: f32,
    pub note_flow_speed: f32,
    pub note_flow_speed_locked: bool,
    pub tap_drag_hold_judgement: JudgementMode,
    pub flick_judgement: JudgementMode,
    pub phigros_strict_judgement: bool,
    pub use_keyboard: bool,
    pub touch_input_debug_report: bool,
    pub shorten_holds: bool,
    pub auto_flip_enabled: bool,
    pub autoplay: bool,
    pub mods_bits: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRange {
    pub configured_start_seconds: f64,
    pub configured_end_seconds: f64,
    pub actual_start_seconds: f64,
    pub actual_end_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportResult {
    /// The exact score currently held by the engine when the attempt ends.
    pub score: u32,
    /// Accuracy over notes that have actually resolved, matching the in-game live accuracy.
    pub accuracy: f64,
    pub full_chart_accuracy: f64,
    pub max_combo: u32,
    pub score_is_provisional: bool,
    pub resolved_notes: u32,
    pub configured_range_notes: u32,
    pub configured_range_accuracy: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgementSummary {
    pub tap: GradeCounts,
    pub hold_head: GradeCounts,
    pub hold_completion: HoldCompletionCounts,
    pub drag: BinaryCounts,
    pub flick: BinaryCounts,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugTrajectoryPoint {
    pub frame_song_time_seconds: f64,
    pub report_real_time_seconds: f64,
    #[serde(flatten)]
    pub sample: TouchDebugSample,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugBoundary {
    pub trigger_source: String,
    pub sample_sequence: Option<u64>,
    pub frame_index: Option<u64>,
    pub phase: Option<TouchDebugPhase>,
    pub song_time_seconds: f64,
    pub report_real_time_seconds: f64,
    pub judge_time_seconds: f64,
    pub captured_uptime_seconds: f64,
    pub platform_event_time_seconds: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchContactRecord {
    pub contact_no: u32,
    pub finger_id: u64,
    pub press: TouchDebugBoundary,
    pub release: Option<TouchDebugBoundary>,
    pub end_status: String,
    pub trajectory: Vec<TouchDebugTrajectoryPoint>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugFrameSummary {
    pub sequence: u64,
    pub frame_index: u64,
    pub song_time_seconds: f64,
    pub report_real_time_seconds: f64,
    pub judge_time_seconds: f64,
    pub tracker_clock_seconds: f64,
    pub captured_uptime_seconds: f64,
    pub frame_delta_seconds: f64,
    pub tap_drag_hold_uses_phigros: bool,
    pub flick_uses_phigros: bool,
    pub raw_event_sequences: Vec<u64>,
    pub persistent_snapshot_sequences: Vec<u64>,
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
pub struct TouchDebugClearRecord {
    pub observed_song_time_seconds: f64,
    pub observed_report_real_time_seconds: f64,
    #[serde(flatten)]
    pub event: TouchDebugClearEvent,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchDebugReport {
    pub schema_version: u32,
    pub coordinate_systems: &'static str,
    pub frame_count: u64,
    pub contact_count: u64,
    pub trajectory_sample_count: u64,
    pub raw_event_sample_count: u64,
    pub persistent_snapshot_sample_count: u64,
    pub clear_event_count: u64,
    pub truncated: bool,
    pub dropped_frames: u64,
    pub dropped_samples: u64,
    pub contacts: Vec<TouchContactRecord>,
    pub frames: Vec<TouchDebugFrameSummary>,
    pub clear_events: Vec<TouchDebugClearRecord>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayReport {
    pub schema_version: u32,
    pub replica_version: &'static str,
    pub report_id: Uuid,
    pub recorded_at: DateTime<Local>,
    pub wall_clock_started_at: DateTime<Local>,
    pub wall_clock_ended_at: DateTime<Local>,
    pub end_reason: ReportEndReason,
    pub song: ReportSong,
    pub settings: ReportSettings,
    pub range: ReportRange,
    pub wall_clock_duration_seconds: f64,
    pub active_play_duration_seconds: f64,
    pub pause_count: u32,
    pub pause_duration_seconds: f64,
    pub global_note_numbering: &'static str,
    pub configured_range_note_counts: NoteTypeCounts,
    pub actual_range_note_counts: NoteTypeCounts,
    pub judged_note_counts: NoteTypeCounts,
    pub result: ReportResult,
    pub judgements: JudgementSummary,
    pub timing: TimingSummary,
    pub judgement_range_overlay_used: bool,
    pub upload_eligible: bool,
    pub upload_ineligible_reasons: Vec<&'static str>,
    pub note_records: Vec<NoteResultRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touch_debug: Option<TouchDebugReport>,
}

impl PlayReport {
    pub fn suggested_filename(&self) -> String {
        format!("{}-{}.json", self.song.name, self.recorded_at.format("%Y-%m-%d_%H-%M-%S%.3f"))
    }
}

#[derive(Clone, Debug)]
struct CatalogNote {
    global_note_no: u32,
    line_id: u32,
    line_note_id: u32,
    kind: ReportNoteKind,
    chart_time: f64,
    song_time: f64,
}

#[derive(Clone, Debug, Default)]
struct TouchDebugBuilder {
    contacts: Vec<TouchContactRecord>,
    active_contacts: HashMap<u64, usize>,
    recent_terminal_contacts: HashMap<u64, (u64, usize)>,
    frames: Vec<TouchDebugFrameSummary>,
    clear_events: Vec<TouchDebugClearRecord>,
    trajectory_sample_count: u64,
    raw_event_sample_count: u64,
    persistent_snapshot_sample_count: u64,
    total_frames: u64,
    dropped_frames: u64,
    dropped_samples: u64,
}

impl TouchDebugBuilder {
    fn sample_source_name(source: TouchDebugSampleSource) -> &'static str {
        match source {
            TouchDebugSampleSource::MiniquadTouchEvent => "miniquad_touch_event",
            TouchDebugSampleSource::MouseButtonEvent => "mouse_button_event",
            TouchDebugSampleSource::MouseHeldSynthesis => "mouse_held_synthesis",
            TouchDebugSampleSource::MacroquadPersistentSnapshot => "macroquad_persistent_snapshot",
            TouchDebugSampleSource::MacroquadMouseSnapshot => "macroquad_mouse_snapshot",
        }
    }

    fn clear_source_name(source: TouchDebugClearSource) -> &'static str {
        match source {
            TouchDebugClearSource::JudgeReset => "judge_reset",
            TouchDebugClearSource::ExerciseSettingsReset => "exercise_settings_reset",
            TouchDebugClearSource::PauseButton => "pause_button",
            TouchDebugClearSource::AutoFlipTransition => "auto_flip_transition",
            TouchDebugClearSource::AppLifecyclePause => "app_lifecycle_pause",
            TouchDebugClearSource::InstantDeath => "instant_death",
            TouchDebugClearSource::KeyboardPause => "keyboard_pause",
            TouchDebugClearSource::ExplicitUnknown => "explicit_unknown",
        }
    }

    fn boundary_from_sample(
        sample: &TouchDebugSample,
        frame_index: u64,
        song_time: f64,
        real_time: f64,
        trigger_source: String,
    ) -> TouchDebugBoundary {
        TouchDebugBoundary {
            trigger_source,
            sample_sequence: Some(sample.sequence),
            frame_index: Some(frame_index),
            phase: Some(sample.phase),
            song_time_seconds: song_time,
            report_real_time_seconds: real_time,
            judge_time_seconds: sample.mapped_judge_time_seconds,
            captured_uptime_seconds: sample.captured_uptime_seconds,
            platform_event_time_seconds: sample.platform_event_time_seconds,
        }
    }

    fn create_contact(&mut self, sample: &TouchDebugSample, frame_index: u64, song_time: f64, real_time: f64, source: String) -> usize {
        let index = self.contacts.len();
        self.contacts.push(TouchContactRecord {
            contact_no: index as u32 + 1,
            finger_id: sample.finger_id,
            press: Self::boundary_from_sample(sample, frame_index, song_time, real_time, source),
            release: None,
            end_status: "open_at_report_end".to_owned(),
            trajectory: Vec::new(),
        });
        self.active_contacts.insert(sample.finger_id, index);
        index
    }

    fn close_contact(&mut self, finger_id: u64, boundary: TouchDebugBoundary, status: &str) {
        let Some(index) = self.active_contacts.remove(&finger_id) else {
            return;
        };
        let contact = &mut self.contacts[index];
        contact.release = Some(boundary);
        contact.end_status = status.to_owned();
    }

    fn record_trajectory(&mut self, contact_index: usize, sample: TouchDebugSample, song_time: f64, real_time: f64) {
        if self.trajectory_sample_count < TOUCH_DEBUG_MAX_SAMPLES as u64 {
            self.contacts[contact_index].trajectory.push(TouchDebugTrajectoryPoint {
                frame_song_time_seconds: song_time,
                report_real_time_seconds: real_time,
                sample,
            });
            self.trajectory_sample_count += 1;
        } else {
            self.dropped_samples += 1;
        }
    }

    fn record_sample(&mut self, sample: TouchDebugSample, frame_index: u64, song_time: f64, real_time: f64) {
        let source_name = Self::sample_source_name(sample.source);
        let is_snapshot =
            matches!(sample.source, TouchDebugSampleSource::MacroquadPersistentSnapshot | TouchDebugSampleSource::MacroquadMouseSnapshot);
        let existing = self.active_contacts.get(&sample.finger_id).copied();
        let is_terminal = matches!(sample.phase, TouchDebugPhase::Ended | TouchDebugPhase::Cancelled);
        if is_terminal && existing.is_none() {
            if let Some((closed_frame, contact_index)) = self.recent_terminal_contacts.get(&sample.finger_id).copied() {
                if closed_frame == frame_index {
                    self.record_trajectory(contact_index, sample, song_time, real_time);
                    return;
                }
            }
        }
        let contact_index = match sample.phase {
            TouchDebugPhase::Started if is_snapshot && existing.is_some() => existing.unwrap(),
            TouchDebugPhase::Started => {
                if existing.is_some() {
                    let boundary = Self::boundary_from_sample(&sample, frame_index, song_time, real_time, "replaced_by_new_started".to_owned());
                    self.close_contact(sample.finger_id, boundary, "replaced_by_new_started");
                }
                self.create_contact(&sample, frame_index, song_time, real_time, format!("{source_name}_started"))
            }
            _ => {
                existing.unwrap_or_else(|| self.create_contact(&sample, frame_index, song_time, real_time, format!("{source_name}_without_started")))
            }
        };

        self.record_trajectory(contact_index, sample.clone(), song_time, real_time);

        match sample.phase {
            TouchDebugPhase::Ended => {
                let boundary = Self::boundary_from_sample(&sample, frame_index, song_time, real_time, format!("{source_name}_ended"));
                self.close_contact(sample.finger_id, boundary, "ended");
                self.recent_terminal_contacts.insert(sample.finger_id, (frame_index, contact_index));
            }
            TouchDebugPhase::Cancelled => {
                let boundary = Self::boundary_from_sample(&sample, frame_index, song_time, real_time, format!("{source_name}_cancelled"));
                self.close_contact(sample.finger_id, boundary, "cancelled");
                self.recent_terminal_contacts.insert(sample.finger_id, (frame_index, contact_index));
            }
            _ => {}
        }
    }

    fn record_frame(&mut self, frame: TouchDebugFrame, song_time: f64, real_time: f64) {
        self.total_frames += 1;
        self.raw_event_sample_count += frame.raw_events.len() as u64;
        self.persistent_snapshot_sample_count += frame.persistent_snapshot.len() as u64;
        let raw_event_sequences = frame.raw_events.iter().map(|it| it.sequence).collect();
        let persistent_snapshot_sequences = frame.persistent_snapshot.iter().map(|it| it.sequence).collect();
        for sample in frame.raw_events.iter().cloned() {
            self.record_sample(sample, frame.frame_index, song_time, real_time);
        }
        for sample in frame.persistent_snapshot.iter().cloned() {
            self.record_sample(sample, frame.frame_index, song_time, real_time);
        }

        if self.frames.len() >= TOUCH_DEBUG_MAX_FRAMES {
            self.dropped_frames += 1;
            return;
        }
        self.frames.push(TouchDebugFrameSummary {
            sequence: frame.sequence,
            frame_index: frame.frame_index,
            song_time_seconds: song_time,
            report_real_time_seconds: real_time,
            judge_time_seconds: frame.judge_time_seconds,
            tracker_clock_seconds: frame.tracker_clock_seconds,
            captured_uptime_seconds: frame.captured_uptime_seconds,
            frame_delta_seconds: frame.frame_delta_seconds,
            tap_drag_hold_uses_phigros: frame.tap_drag_hold_uses_phigros,
            flick_uses_phigros: frame.flick_uses_phigros,
            raw_event_sequences,
            persistent_snapshot_sequences,
            active_fingers_before: frame.active_fingers_before,
            active_fingers_after: frame.active_fingers_after,
            finger_order_before: frame.finger_order_before,
            finger_order_after: frame.finger_order_after,
            trackers_before: frame.trackers_before,
            trackers_after: frame.trackers_after,
            tracker_started_ids: frame.tracker_started_ids,
            tracker_rebound_ids: frame.tracker_rebound_ids,
            tracker_removed_ids: frame.tracker_removed_ids,
            synthesized_stationary_ids: frame.synthesized_stationary_ids,
            key_down_count_before: frame.key_down_count_before,
            key_down_count_after: frame.key_down_count_after,
            raw_key_delta: frame.raw_key_delta,
        });
    }

    fn record_clear(&mut self, event: TouchDebugClearEvent, song_time: f64, real_time: f64) {
        let trigger = format!("lifecycle_clear_{}", Self::clear_source_name(event.source));
        let active_ids: Vec<_> = self.active_contacts.keys().copied().collect();
        for finger_id in active_ids {
            self.close_contact(
                finger_id,
                TouchDebugBoundary {
                    trigger_source: trigger.clone(),
                    sample_sequence: None,
                    frame_index: None,
                    phase: None,
                    song_time_seconds: song_time,
                    report_real_time_seconds: real_time,
                    judge_time_seconds: event.judge_clock_seconds,
                    captured_uptime_seconds: event.captured_uptime_seconds,
                    platform_event_time_seconds: None,
                },
                &trigger,
            );
        }
        self.clear_events.push(TouchDebugClearRecord {
            observed_song_time_seconds: song_time,
            observed_report_real_time_seconds: real_time,
            event,
        });
    }

    fn record(&mut self, record: JudgeTouchDebugRecord, song_time: f64, real_time: f64) {
        match record {
            JudgeTouchDebugRecord::Frame(frame) => self.record_frame(*frame, song_time, real_time),
            JudgeTouchDebugRecord::Clear(event) => self.record_clear(event, song_time, real_time),
        }
    }

    fn into_report(self) -> TouchDebugReport {
        TouchDebugReport {
            schema_version: TOUCH_DEBUG_SCHEMA_VERSION,
            coordinate_systems: "screen_x/y_px=raw_platform_pixels; judge_x/y=viewport_normalized_game_space; phigros_motion_x/y=judge_space_divided_by_phigros_x_unit",
            frame_count: self.total_frames,
            contact_count: self.contacts.len() as u64,
            trajectory_sample_count: self.trajectory_sample_count,
            raw_event_sample_count: self.raw_event_sample_count,
            persistent_snapshot_sample_count: self.persistent_snapshot_sample_count,
            clear_event_count: self.clear_events.len() as u64,
            truncated: self.dropped_frames != 0 || self.dropped_samples != 0,
            dropped_frames: self.dropped_frames,
            dropped_samples: self.dropped_samples,
            contacts: self.contacts,
            frames: self.frames,
            clear_events: self.clear_events,
        }
    }
}

#[derive(Clone, Debug)]
struct Attempt {
    report_id: Uuid,
    started_at: DateTime<Local>,
    started_real_time: f64,
    actual_start: f64,
    actual_end: f64,
    configured_start: f64,
    configured_end: f64,
    settings: ReportSettings,
    pause_count: u32,
    pause_total: f64,
    pause_started: Option<f64>,
    tap: GradeCounts,
    hold_head: GradeCounts,
    hold_completion: HoldCompletionCounts,
    drag: BinaryCounts,
    flick: BinaryCounts,
    final_grades: GradeCounts,
    judged_counts: NoteTypeCounts,
    head_seen: HashSet<(u32, u32)>,
    final_seen: HashSet<(u32, u32)>,
    tap_delays: Vec<f64>,
    hold_delays: Vec<f64>,
    tap_delay_by_note_no: BTreeMap<u32, Option<f64>>,
    hold_delay_by_note_no: BTreeMap<u32, Option<f64>>,
    records: Vec<NoteResultRecord>,
    touch_debug: Option<TouchDebugBuilder>,
}

pub struct PlayReportRecorder {
    enabled: bool,
    song: ReportSong,
    catalog: Vec<CatalogNote>,
    by_key: HashMap<(u32, u32), usize>,
    attempt: Option<Attempt>,
}

impl PlayReportRecorder {
    pub fn new(chart: &Chart, info: &ChartInfo, chart_bytes: &[u8], chart_format: ChartFormat, enabled: bool) -> Self {
        let mut ordered = Vec::new();
        for (line_id, line) in chart.lines.iter().enumerate() {
            for (note_id, note) in line.notes.iter().enumerate() {
                if !note.fake {
                    ordered.push((note.time, line_id as u32, note_id as u32, ReportNoteKind::from(&note.kind)));
                }
            }
        }
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));

        let mut catalog = Vec::with_capacity(ordered.len());
        let mut by_key = HashMap::with_capacity(ordered.len());
        for (index, (chart_time, line_id, line_note_id, kind)) in ordered.into_iter().enumerate() {
            by_key.insert((line_id, line_note_id), index);
            catalog.push(CatalogNote {
                global_note_no: index as u32 + 1,
                line_id,
                line_note_id,
                kind,
                chart_time,
                song_time: chart_time,
            });
        }

        Self {
            enabled,
            song: ReportSong {
                name: info.name.clone(),
                level: info.level.clone(),
                difficulty: info.difficulty,
                composer: info.composer.clone(),
                charter: info.charter.clone(),
                chart_id: info.id,
                chart_format,
                chart_sha256: hex::encode(Sha256::digest(chart_bytes)),
                replica_play: info.replica_play.clone(),
            },
            catalog,
            by_key,
            attempt: None,
        }
    }

    pub fn set_chart_offset(&mut self, offset: f64) {
        for note in &mut self.catalog {
            note.song_time = note.chart_time + offset;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_if_needed(
        &mut self,
        mode: ReportGameMode,
        config: &Config,
        configured_start: f64,
        configured_end: f64,
        actual_start: f64,
        real_time: f64,
        note_flow_speed: f32,
        note_flow_speed_locked: bool,
    ) {
        if !self.enabled || self.attempt.is_some() {
            return;
        }
        self.attempt = Some(Attempt {
            report_id: Uuid::new_v4(),
            started_at: Local::now(),
            started_real_time: real_time,
            actual_start,
            actual_end: actual_start,
            configured_start,
            configured_end,
            settings: ReportSettings {
                mode,
                playback_speed: config.speed,
                note_flow_speed,
                note_flow_speed_locked,
                tap_drag_hold_judgement: config.judgement_mode,
                flick_judgement: config.flick_judgement_mode(),
                phigros_strict_judgement: config.phigros_strict_judgement,
                use_keyboard: config.use_keyboard,
                touch_input_debug_report: config.touch_input_debug_report,
                shorten_holds: config.shorten_holds,
                auto_flip_enabled: config.auto_flip_enabled,
                autoplay: config.autoplay(),
                mods_bits: config.mods.bits(),
            },
            pause_count: 0,
            pause_total: 0.,
            pause_started: None,
            tap: GradeCounts::default(),
            hold_head: GradeCounts::default(),
            hold_completion: HoldCompletionCounts::default(),
            drag: BinaryCounts::default(),
            flick: BinaryCounts::default(),
            final_grades: GradeCounts::default(),
            judged_counts: NoteTypeCounts::default(),
            head_seen: HashSet::new(),
            final_seen: HashSet::new(),
            tap_delays: Vec::new(),
            hold_delays: Vec::new(),
            tap_delay_by_note_no: BTreeMap::new(),
            hold_delay_by_note_no: BTreeMap::new(),
            records: Vec::new(),
            touch_debug: config.touch_input_debug_report.then(TouchDebugBuilder::default),
        });
    }

    pub fn observe(&mut self, song_time: f64, real_time: f64, paused: bool) {
        let Some(attempt) = &mut self.attempt else {
            return;
        };
        attempt.actual_end = song_time;
        match (paused, attempt.pause_started) {
            (true, None) => {
                attempt.pause_count += 1;
                attempt.pause_started = Some(real_time);
            }
            (false, Some(started)) => {
                attempt.pause_total += (real_time - started).max(0.);
                attempt.pause_started = None;
            }
            _ => {}
        }
    }

    pub fn record_events(&mut self, events: impl IntoIterator<Item = JudgeReportEvent>) {
        let Some(attempt) = &mut self.attempt else {
            return;
        };
        for event in events {
            let Some(&catalog_index) = self.by_key.get(&(event.line_id, event.note_id)) else {
                continue;
            };
            let note = &self.catalog[catalog_index];
            let key = (event.line_id, event.note_id);
            match event.judgement {
                Err(perfect) if note.kind == ReportNoteKind::Hold => {
                    if !attempt.head_seen.insert(key) {
                        continue;
                    }
                    let judgement = if perfect { ReportJudgement::Perfect } else { ReportJudgement::Good };
                    let delay = (note.chart_time - event.time) / attempt.settings.playback_speed as f64 * 1000.;
                    attempt.hold_head.add(judgement);
                    attempt.hold_delays.push(delay);
                    attempt.hold_delay_by_note_no.insert(note.global_note_no, Some(delay));
                    attempt.records.push(NoteResultRecord {
                        global_note_no: note.global_note_no,
                        line_id: note.line_id,
                        line_note_id: note.line_note_id,
                        note_type: note.kind,
                        phase: "head",
                        note_time_seconds: note.song_time,
                        judged_time_seconds: Some(event.time + (note.song_time - note.chart_time)),
                        judgement,
                        delay_ms: Some(delay),
                        miss_reason: None,
                    });
                }
                Err(_) => {}
                Ok(raw_judgement) => {
                    if !attempt.final_seen.insert(key) {
                        continue;
                    }
                    let judgement = ReportJudgement::from(raw_judgement);
                    attempt.judged_counts.add(note.kind);
                    attempt.final_grades.add(judgement);
                    match note.kind {
                        ReportNoteKind::Tap => {
                            attempt.tap.add(judgement);
                            let delay = (!matches!(judgement, ReportJudgement::Miss)).then_some(-event.difference * 1000.);
                            if let Some(delay) = delay {
                                attempt.tap_delays.push(delay);
                            }
                            attempt.tap_delay_by_note_no.insert(note.global_note_no, delay);
                            attempt.records.push(NoteResultRecord {
                                global_note_no: note.global_note_no,
                                line_id: note.line_id,
                                line_note_id: note.line_note_id,
                                note_type: note.kind,
                                phase: "final",
                                note_time_seconds: note.song_time,
                                judged_time_seconds: delay.map(|_| note.song_time + event.difference * attempt.settings.playback_speed as f64),
                                judgement,
                                delay_ms: delay,
                                miss_reason: matches!(judgement, ReportJudgement::Miss).then_some("timeout"),
                            });
                        }
                        ReportNoteKind::Hold => {
                            let head_seen = attempt.head_seen.contains(&key);
                            if !head_seen && matches!(judgement, ReportJudgement::Miss) {
                                attempt.hold_head.add(ReportJudgement::Miss);
                                attempt.hold_delay_by_note_no.insert(note.global_note_no, None);
                                attempt.records.push(NoteResultRecord {
                                    global_note_no: note.global_note_no,
                                    line_id: note.line_id,
                                    line_note_id: note.line_note_id,
                                    note_type: note.kind,
                                    phase: "head",
                                    note_time_seconds: note.song_time,
                                    judged_time_seconds: None,
                                    judgement: ReportJudgement::Miss,
                                    delay_ms: None,
                                    miss_reason: Some("timeout"),
                                });
                            }
                            match judgement {
                                ReportJudgement::Perfect | ReportJudgement::Good => attempt.hold_completion.completed += 1,
                                ReportJudgement::Miss if head_seen => attempt.hold_completion.released_early += 1,
                                ReportJudgement::Miss => attempt.hold_completion.head_missed += 1,
                                ReportJudgement::Bad => {}
                            }
                            attempt.records.push(NoteResultRecord {
                                global_note_no: note.global_note_no,
                                line_id: note.line_id,
                                line_note_id: note.line_note_id,
                                note_type: note.kind,
                                phase: "completion",
                                note_time_seconds: note.song_time,
                                judged_time_seconds: (!matches!(judgement, ReportJudgement::Miss))
                                    .then_some(event.time + (note.song_time - note.chart_time)),
                                judgement,
                                delay_ms: None,
                                miss_reason: matches!(judgement, ReportJudgement::Miss).then_some(if head_seen {
                                    "released_early"
                                } else {
                                    "head_missed"
                                }),
                            });
                        }
                        ReportNoteKind::Drag => {
                            attempt.drag.add(judgement);
                            attempt.records.push(Self::binary_record(note, event, judgement));
                        }
                        ReportNoteKind::Flick => {
                            attempt.flick.add(judgement);
                            attempt.records.push(Self::binary_record(note, event, judgement));
                        }
                    }
                }
            }
        }
    }

    pub fn record_touch_debug(&mut self, records: impl IntoIterator<Item = JudgeTouchDebugRecord>, song_time: f64, real_time: f64) {
        let Some(debug) = self.attempt.as_mut().and_then(|attempt| attempt.touch_debug.as_mut()) else {
            return;
        };
        for record in records {
            debug.record(record, song_time, real_time);
        }
    }

    fn binary_record(note: &CatalogNote, event: JudgeReportEvent, judgement: ReportJudgement) -> NoteResultRecord {
        NoteResultRecord {
            global_note_no: note.global_note_no,
            line_id: note.line_id,
            line_note_id: note.line_note_id,
            note_type: note.kind,
            phase: "final",
            note_time_seconds: note.song_time,
            judged_time_seconds: (!matches!(judgement, ReportJudgement::Miss)).then_some(event.time + (note.song_time - note.chart_time)),
            judgement,
            delay_ms: None,
            miss_reason: matches!(judgement, ReportJudgement::Miss).then_some("timeout"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        &mut self,
        reason: ReportEndReason,
        song_time: f64,
        real_time: f64,
        result: &PlayResult,
        real_time_accuracy: f64,
        judgement_range_overlay_used: bool,
        upload_ineligible_reasons: Vec<&'static str>,
    ) -> Option<PlayReport> {
        let mut attempt = self.attempt.take()?;
        if let Some(started) = attempt.pause_started.take() {
            attempt.pause_total += (real_time - started).max(0.);
        }
        attempt.actual_end = song_time;
        let ended_at = Local::now();
        let wall_duration = (real_time - attempt.started_real_time).max(0.);

        let mut configured_counts = NoteTypeCounts::default();
        let mut actual_counts = NoteTypeCounts::default();
        let actual_start = attempt.actual_start.min(attempt.actual_end);
        let actual_end = attempt.actual_start.max(attempt.actual_end);
        for note in &self.catalog {
            if note.song_time >= attempt.configured_start && note.song_time <= attempt.configured_end {
                configured_counts.add(note.kind);
            }
            if note.song_time >= actual_start && note.song_time <= actual_end {
                actual_counts.add(note.kind);
            }
        }

        let resolved = attempt.tap.total()
            + attempt.hold_completion.completed
            + attempt.hold_completion.released_early
            + attempt.hold_completion.head_missed
            + attempt.drag.total()
            + attempt.flick.total();
        let perfect = attempt.final_grades.perfect;
        let good = attempt.final_grades.good;
        let configured_accuracy = if configured_counts.total == 0 {
            1.
        } else {
            (perfect as f64 + good as f64 * 0.65) / configured_counts.total as f64
        };

        let mut combined_delays = attempt.tap_delays.clone();
        combined_delays.extend_from_slice(&attempt.hold_delays);

        Some(PlayReport {
            schema_version: REPORT_SCHEMA_VERSION,
            replica_version: REPLICA_VERSION,
            report_id: attempt.report_id,
            recorded_at: ended_at,
            wall_clock_started_at: attempt.started_at,
            wall_clock_ended_at: ended_at,
            end_reason: reason,
            song: self.song.clone(),
            settings: attempt.settings,
            range: ReportRange {
                configured_start_seconds: attempt.configured_start,
                configured_end_seconds: attempt.configured_end,
                actual_start_seconds: attempt.actual_start,
                actual_end_seconds: attempt.actual_end,
            },
            wall_clock_duration_seconds: wall_duration,
            active_play_duration_seconds: (wall_duration - attempt.pause_total).max(0.),
            pause_count: attempt.pause_count,
            pause_duration_seconds: attempt.pause_total,
            global_note_numbering: "one_based_by_note_time_then_line_id_then_line_note_id_excluding_fake_notes",
            configured_range_note_counts: configured_counts,
            actual_range_note_counts: actual_counts,
            judged_note_counts: attempt.judged_counts,
            result: ReportResult {
                score: result.score,
                accuracy: real_time_accuracy,
                full_chart_accuracy: result.accuracy,
                max_combo: result.max_combo,
                score_is_provisional: !reason.is_complete(),
                resolved_notes: resolved,
                configured_range_notes: configured_counts.total,
                configured_range_accuracy: configured_accuracy,
            },
            judgements: JudgementSummary {
                tap: attempt.tap,
                hold_head: attempt.hold_head,
                hold_completion: attempt.hold_completion,
                drag: attempt.drag,
                flick: attempt.flick,
            },
            timing: TimingSummary {
                sign_convention: "positive_is_early_negative_is_late",
                tap: DelayStatistics::from_values(&attempt.tap_delays),
                hold_head: DelayStatistics::from_values(&attempt.hold_delays),
                combined: DelayStatistics::from_values(&combined_delays),
                delay_by_global_note_no_ms: PerNoteDelays {
                    tap: attempt.tap_delay_by_note_no,
                    hold_head: attempt.hold_delay_by_note_no,
                },
            },
            judgement_range_overlay_used,
            upload_eligible: upload_ineligible_reasons.is_empty(),
            upload_ineligible_reasons,
            note_records: attempt.records,
            touch_debug: attempt.touch_debug.map(TouchDebugBuilder::into_report),
        })
    }

    pub fn discard_unstarted(&mut self) {
        self.attempt = None;
    }
}

pub fn upload_ineligible_reasons(config: &Config, mode: ReportGameMode, judgement_range_overlay_used: bool) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if mode != ReportGameMode::Normal {
        reasons.push("non_normal_mode");
    }
    if config.offline_mode {
        reasons.push("offline_mode");
    }
    if config.mods.intersects(Mods::UNRATED) {
        reasons.push("unrated_mod");
    }
    if config.use_keyboard {
        reasons.push("keyboard_input");
    }
    if config.speed < 1.0 - 1e-3 {
        reasons.push("playback_speed_below_one");
    }
    if config.has_custom_judgement() {
        reasons.push("custom_judgement");
    }
    if config.shorten_holds {
        reasons.push("shorten_holds");
    }
    if config.touch_input_debug_report {
        reasons.push("touch_input_debug_report");
    }
    if judgement_range_overlay_used {
        reasons.push("judgement_range_overlay");
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_sample(sequence: u64, finger_id: u64, phase: TouchDebugPhase) -> TouchDebugSample {
        TouchDebugSample {
            sequence,
            finger_id,
            phase,
            source: TouchDebugSampleSource::MiniquadTouchEvent,
            platform_event_time_seconds: Some(sequence as f64 / 100.),
            captured_uptime_seconds: 10. + sequence as f64 / 100.,
            frame_judge_time_seconds: sequence as f64 / 100.,
            mapped_judge_time_seconds: sequence as f64 / 100.,
            tap_drag_hold_effective_time_seconds: sequence as f64 / 100.,
            flick_effective_time_seconds: sequence as f64 / 100.,
            screen_x_px: sequence as f32,
            screen_y_px: finger_id as f32,
            judge_x: sequence as f32 / 100.,
            judge_y: finger_id as f32 / 100.,
            phigros_motion_x: sequence as f32 / 10.,
            phigros_motion_y: finger_id as f32 / 10.,
            present_in_persistent_snapshot: !matches!(phase, TouchDebugPhase::Ended | TouchDebugPhase::Cancelled),
        }
    }

    #[test]
    fn delay_statistics_use_population_variance_and_requested_sign() {
        let stats = DelayStatistics::from_values(&[20., -10., 0., 10.]);
        assert_eq!(stats.sample_count, 4);
        assert_eq!(stats.early_count, 2);
        assert_eq!(stats.late_count, 1);
        assert_eq!(stats.exact_count, 1);
        assert_eq!(stats.mean_ms, Some(5.));
        assert_eq!(stats.median_ms, Some(5.));
        assert_eq!(stats.variance_ms2, Some(125.));
    }

    #[test]
    fn empty_delay_statistics_are_null_not_zero() {
        let stats = DelayStatistics::from_values(&[]);
        assert_eq!(stats.sample_count, 0);
        assert_eq!(stats.mean_ms, None);
        assert_eq!(stats.median_ms, None);
        assert_eq!(stats.variance_ms2, None);
    }

    #[test]
    fn phira_default_upload_reasons_remain_empty() {
        let config = Config::default();
        assert!(config.auto_export_play_report);
        assert!(upload_ineligible_reasons(&config, ReportGameMode::Normal, false).is_empty());
    }

    #[test]
    fn touch_debug_groups_press_motion_and_release_into_one_contact() {
        let mut debug = TouchDebugBuilder::default();
        debug.record_sample(touch_sample(1, 42, TouchDebugPhase::Started), 10, 1., 11.);
        debug.record_sample(touch_sample(2, 42, TouchDebugPhase::Moved), 11, 1.01, 11.01);
        debug.record_sample(touch_sample(3, 42, TouchDebugPhase::Ended), 12, 1.02, 11.02);

        let report = debug.into_report();
        assert_eq!(report.contact_count, 1);
        assert_eq!(report.trajectory_sample_count, 3);
        let contact = &report.contacts[0];
        assert_eq!(contact.finger_id, 42);
        assert_eq!(contact.press.trigger_source, "miniquad_touch_event_started");
        assert_eq!(contact.release.as_ref().unwrap().trigger_source, "miniquad_touch_event_ended");
        assert_eq!(contact.end_status, "ended");
        assert_eq!(contact.trajectory.len(), 3);
        assert_eq!(contact.trajectory[1].sample.phase, TouchDebugPhase::Moved);
    }

    #[test]
    fn touch_debug_deduplicates_raw_and_snapshot_release_in_the_same_frame() {
        let mut debug = TouchDebugBuilder::default();
        debug.record_sample(touch_sample(1, 42, TouchDebugPhase::Started), 10, 1., 11.);
        debug.record_sample(touch_sample(2, 42, TouchDebugPhase::Ended), 11, 1.01, 11.01);
        let mut snapshot_release = touch_sample(3, 42, TouchDebugPhase::Ended);
        snapshot_release.source = TouchDebugSampleSource::MacroquadPersistentSnapshot;
        debug.record_sample(snapshot_release, 11, 1.01, 11.01);

        let report = debug.into_report();
        assert_eq!(report.contact_count, 1);
        assert_eq!(report.trajectory_sample_count, 3);
        assert_eq!(report.contacts[0].trajectory.len(), 3);
        assert_eq!(report.contacts[0].trajectory[2].sample.source, TouchDebugSampleSource::MacroquadPersistentSnapshot);
        assert_eq!(report.contacts[0].end_status, "ended");
    }

    #[test]
    fn touch_debug_lifecycle_clear_closes_every_active_contact_with_source() {
        let mut debug = TouchDebugBuilder::default();
        debug.record_sample(touch_sample(1, 7, TouchDebugPhase::Started), 1, 2., 12.);
        debug.record_sample(touch_sample(2, 8, TouchDebugPhase::Started), 1, 2., 12.);
        debug.record_clear(
            TouchDebugClearEvent {
                sequence: 3,
                source: TouchDebugClearSource::AppLifecyclePause,
                judge_clock_seconds: 2.1,
                captured_uptime_seconds: 12.1,
                active_fingers_cleared: vec![TouchDebugFingerState {
                    finger_id: 7,
                    judge_x: 0.1,
                    judge_y: 0.2,
                }],
                finger_order_cleared: vec![7, 8],
                trackers_cleared: Vec::new(),
            },
            2.1,
            12.1,
        );

        let report = debug.into_report();
        assert_eq!(report.clear_event_count, 1);
        assert_eq!(report.contacts.len(), 2);
        assert!(report
            .contacts
            .iter()
            .all(|contact| contact.end_status == "lifecycle_clear_app_lifecycle_pause"));
        assert!(report.contacts.iter().all(|contact| contact.release.is_some()));
        assert_eq!(report.clear_events[0].event.finger_order_cleared, vec![7, 8]);
    }

    #[test]
    fn touch_debug_report_setting_is_an_explicit_upload_blocker() {
        let mut config = Config::default();
        config.touch_input_debug_report = true;
        assert_eq!(upload_ineligible_reasons(&config, ReportGameMode::Normal, false), vec!["touch_input_debug_report"]);
    }
}
