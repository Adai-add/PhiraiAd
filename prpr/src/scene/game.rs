#![allow(unused)]

prpr_l10n::tl_file!("game");

use super::{
    draw_background,
    ending::RecordUpdateState,
    loading::{BasicPlayer, ReportFn, SaveFn, UpdateFn, UploadFn},
    request_input, return_input, show_message, take_input, EndingScene, NextScene, Scene,
};
use crate::practice_audio::Music;
use crate::{
    bin::BinaryReader,
    config::{
        Config, Mods, NOTE_FLOW_SPEED_MAX, NOTE_FLOW_SPEED_MIN, NOTE_FLOW_SPEED_STEP, PLAYBACK_SPEED_MAX, PLAYBACK_SPEED_MIN, PLAYBACK_SPEED_STEP,
    },
    core::{copy_fbo, BadNote, Chart, ChartExtra, Effect, Matrix, Point, Resource, UIElement, Vector, PGR_FONT},
    ext::{parse_time, screen_aspect, semi_white, RectExt, SafeTexture, ScaleType},
    fs::FileSystem,
    info::{ChartFormat, ChartInfo},
    judge::{Judge, TouchDebugClearSource},
    parse::{parse_extra, parse_pec, parse_phigros, parse_rpe},
    play_report::{upload_ineligible_reasons, PlayReportRecorder, ReportEndReason, ReportGameMode},
    task::Task,
    time::TimeManager,
    ui::{OffsetAnalysisPanel, OffsetPanelAction, OffsetPanelLabels, RectButton, TextPainter, Ui},
};
use anyhow::{bail, Context, Result};
use concat_string::concat_string;
use inputbox::InputBox;
use macroquad::{prelude::*, window::InternalGlContext};
use sasa::MusicParams;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    cell::RefCell,
    fs::File,
    io::{Cursor, ErrorKind},
    ops::{Deref, DerefMut, Range},
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use tracing::{debug, warn};

const PAUSE_CLICK_INTERVAL: f32 = 0.7;

#[rustfmt::skip]
#[cfg(closed)]
mod inner;
#[cfg(closed)]
use inner::*;

const WAIT_TIME: f64 = 0.5;
const AFTER_TIME: f64 = 0.7;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleRecord {
    pub score: i32,
    pub accuracy: f32,
    pub full_combo: bool,
}

impl SimpleRecord {
    pub fn update(&mut self, other: &SimpleRecord) -> bool {
        let mut changed = false;
        if other.score > self.score {
            self.score = other.score;
            changed = true;
        }
        if other.accuracy > self.accuracy {
            self.accuracy = other.accuracy;
            changed = true;
        }
        if other.full_combo & !self.full_combo {
            self.full_combo = other.full_combo;
            changed = true;
        }
        changed
    }
}

fn fmt_time(t: f32) -> String {
    let f = t < 0.;
    let t = t.abs();
    let secs = t % 60.;
    let mut t = (t / 60.) as u64;
    let mins = t % 60;
    t /= 60;
    let hrs = t % 100;
    format!("{}{hrs:02}:{mins:02}:{secs:05.2}", if f { "-" } else { "" })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    fn on_game_start();
}

#[derive(PartialEq, Eq)]
pub enum GameMode {
    Normal,
    TweakOffset,
    EditChartPlay,
    Exercise,
    NoRetry,
    View,
}

#[derive(Clone)]
enum State {
    Starting,
    BeforeMusic,
    Playing,
    Ending,
}

/// Course scene transitions bypass the ordinary single-chart ending screen.
pub enum ChallengeEvent {
    Completed(crate::judge::PlayResult),
    Restart,
    Aborted,
}

pub struct GameScene {
    should_exit: bool,
    next_scene: Option<NextScene>,

    pub mode: GameMode,
    pub res: Resource,
    pub chart: Chart,
    pub judge: Judge,
    pub gl: InternalGlContext<'static>,
    player: Option<BasicPlayer>,
    chart_bytes: Vec<u8>,
    chart_format: ChartFormat,
    info_offset: f32,
    effects: Vec<Effect>,
    offset_analysis: OffsetAnalysisPanel,
    chart_play_editor: crate::ui::chart_play_editor::ChartPlayEditor,
    note_conversion_backup: crate::core::NoteConversionBackup,
    auto_flip: crate::auto_flip::AutoFlipPlayback,

    first_in: bool,
    exercise_range: Range<f64>,
    exercise_press: Option<(i8, u64)>,
    exercise_btns: (RectButton, RectButton),
    exercise_note_flow_speed: f32,
    exercise_note_flow_locked: bool,
    exercise_judgement_percent: f32,
    exercise_judgement_locked: bool,
    exercise_extra_open: bool,

    pub music: Music,
    noise_renderer: Option<crate::noise_area::render::NoiseRenderer>,
    correct_sound_events: Option<Arc<Vec<crate::correct_sound::ScheduledHit>>>,

    state: State,
    pub last_update_time: f64,
    pause_rewind: Option<f64>,
    pause_first_time: f32,

    pub bad_notes: Vec<BadNote>,

    upload_fn: Option<UploadFn>,
    update_fn: Option<UpdateFn>,
    save_fn: Option<SaveFn>,
    report_fn: Option<ReportFn>,

    best_record: Option<SimpleRecord>,

    pub touch_points: Vec<(f32, f32)>,
    fps_frame_count: u32,
    fps_total_time: f64,
    fps_last_frame_time: f64,

    /// Once the debug range overlay has been visible, turning it off cannot
    /// make the current run upload-eligible again.
    judgement_range_debug_used: bool,
    play_report: PlayReportRecorder,
    timing_bar: crate::timing_bar::TimingBar,
    preview_combo: Option<u32>,
    exercise_state_reset_pending: bool,

    replay: Option<crate::replay::Recorder>,
    replay_assets: crate::replay::Assets,
    replay_armed: bool,
    replay_view_hud: Option<crate::replay::Hud>,
    replay_view_profile: Option<crate::judge::JudgementRangeProfile>,
    replay_view_angle: Option<f32>,
    replay_audio_speed: f64,
    replay_audio_playing: bool,
    replay_save_cooldown: f64,
    dead: bool,
}

macro_rules! reset {
    ($self:ident, $res:expr, $tm:ident) => {{
        $self.auto_flip.reset();
        $self.bad_notes.clear();
        $self.judge.reset();
        $self.timing_bar.clear();
        $self.chart.reset();
        $res.judge_line_color = $res.res_pack.info.color_perfect();
        $self.music.pause()?;
        $self.music.seek_to(0.)?;
        $tm.speed = $res.config.speed as _;
        $tm.reset();
        $self.last_update_time = $tm.now();
        $self.state = State::Starting;
        $self.fps_frame_count = 0;
        $self.fps_total_time = 0.0;
        $self.fps_last_frame_time = $tm.real_time();
        $self.dead = false;
        $self.pause_rewind = None;
        $self.reset_replay();
    }};
}

impl GameScene {
    pub const BEFORE_TIME: f64 = 0.7;
    pub const FADEOUT_TIME: f64 = WAIT_TIME + AFTER_TIME + 0.3;

    pub async fn load_chart_bytes(fs: &mut dyn FileSystem, info: &ChartInfo) -> Result<Vec<u8>> {
        if let Ok(bytes) = fs.load_file(&info.chart).await {
            return Ok(bytes);
        }
        if let Some(name) = info.chart.strip_suffix(".pec") {
            if let Ok(bytes) = fs.load_file(&concat_string!(name, ".json")).await {
                return Ok(bytes);
            }
        }
        bail!("Cannot find chart file")
    }

    pub fn infer_chart_format(info: &ChartInfo, bytes: &[u8]) -> ChartFormat {
        info.format.clone().unwrap_or_else(|| {
            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                if text.starts_with('{') {
                    if text.contains("\"META\"") {
                        ChartFormat::Rpe
                    } else {
                        ChartFormat::Pgr
                    }
                } else {
                    ChartFormat::Pec
                }
            } else {
                ChartFormat::Pbc
            }
        })
    }

    pub async fn load_chart(fs: &mut dyn FileSystem, info: &ChartInfo) -> Result<(Chart, Vec<u8>, ChartFormat)> {
        let extra = fs.load_file("extra.json").await.ok().map(String::from_utf8).transpose()?;
        let extra = if let Some(extra) = extra {
            parse_extra(&extra, fs).await.context("Failed to parse extra")?
        } else {
            ChartExtra::default()
        };
        let bytes = Self::load_chart_bytes(fs, info).await.context("Failed to load chart")?;
        let format = Self::infer_chart_format(info, &bytes);
        let mut chart = match format {
            ChartFormat::Rpe => parse_rpe(&String::from_utf8_lossy(&bytes), fs, extra, info.use_rpe_170_speed.unwrap_or_default()).await,
            ChartFormat::Pgr => parse_phigros(&String::from_utf8_lossy(&bytes), extra),
            ChartFormat::Pec => parse_pec(&String::from_utf8_lossy(&bytes), extra),
            ChartFormat::Pbc => {
                let mut r = BinaryReader::new(Cursor::new(&bytes));
                r.read()
            }
        }?;
        chart.load_textures(fs).await?;
        chart.settings.hold_partial_cover = info.hold_partial_cover;
        Ok((chart, bytes, format))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        mode: GameMode,
        mut info: ChartInfo,
        mut config: Config,
        mut fs: Box<dyn FileSystem>,
        player: Option<BasicPlayer>,
        background: SafeTexture,
        illustration: SafeTexture,
        upload_fn: Option<UploadFn>,
        update_fn: Option<UpdateFn>,
        save_fn: Option<SaveFn>,
        report_fn: Option<ReportFn>,
    ) -> Result<Self> {
        if config.challenge_mode {
            config.enable_challenge();
        }
        let save_fn = if config.challenge_mode { None } else { save_fn };
        let update_fn = if config.challenge_mode { None } else { update_fn };
        match mode {
            GameMode::TweakOffset | GameMode::EditChartPlay => {
                config.mods.insert(Mods::AUTOPLAY);
            }
            _ => {}
        }

        let capture_fs = crate::replay::CaptureFs::new(fs).with_enabled(config.replay_recording_enabled);
        let replay_assets = capture_fs.assets.clone();
        fs = Box::new(capture_fs);
        // Also capture the illustration when LoadingScene used a preloaded texture.
        if config.replay_recording_enabled {
            let _ = fs.load_file(&info.illustration).await;
        }
        let (mut chart, chart_bytes, chart_format) = Self::load_chart(fs.deref_mut(), &info).await?;
        if let Some(settings) = crate::chart_play::ChartPlaySettings::from_chart_bytes(&chart_bytes) {
            info.replica_play = settings;
        }
        info.replica_play.normalize();
        // Embedded JSON settings are read above; override conversion only after that.
        config.apply_challenge_chart(&mut info.replica_play);
        let mut note_conversion_backup = crate::core::NoteConversionBackup::capture(&chart);
        if matches!(mode, GameMode::Normal | GameMode::NoRetry | GameMode::Exercise | GameMode::EditChartPlay) {
            note_conversion_backup.apply(&mut chart, info.replica_play.note_conversion);
        }
        let upload_fn = if config.challenge_mode
            || config.blocks_score_upload()
            || info.replica_play.blocks_score_upload()
            || config.online_replica_active(info.id.is_some(), info.replica_play.blocks_score_upload())
        {
            None
        } else {
            upload_fn
        };
        if config.mods.contains(Mods::NO_SHADER) {
            chart.extra.effects.clear();
            chart.extra.global_effects.clear();
        }
        let effects = std::mem::take(&mut chart.extra.global_effects);
        if config.fxaa {
            chart
                .extra
                .effects
                .push(Effect::new(0.0..f64::INFINITY, include_str!("fxaa.glsl"), Vec::new(), false).unwrap());
        }

        if config.has_mod(Mods::NIGHTCORE) {
            config.speed *= 1.5;
        }

        if config.has_mod(Mods::RAINBOW) {
            chart
                .extra
                .effects
                .push(Effect::new(0.0..f64::INFINITY, include_str!("rainbow.glsl"), Vec::new(), false).unwrap());
        }

        if config.replay_recording_enabled {
            replay_assets.lock().unwrap().insert(info.chart.clone(), Arc::new(chart_bytes.clone()));
        }
        let info_offset = info.offset;
        let mut res = Resource::new(
            config,
            info,
            fs,
            player.as_ref().and_then(|it| it.avatar.clone()),
            background,
            illustration,
            chart.extra.effects.is_empty() && effects.is_empty(),
        )
        .await
        .context("Failed to load resources")?;

        res.has_noise_area = res.config.noise_area.enabled && !chart.extra.block_areas.is_empty();
        let noise_renderer = if res.has_noise_area {
            Some(
                crate::noise_area::render::NoiseRenderer::new(res.config.noise_area.low_performance)
                    .await
                    .context("Failed to initialize noise-area renderer")?,
            )
        } else {
            None
        };
        let upload_fn = if !chart.extra.block_areas.is_empty() && !res.config.noise_area.enabled {
            None
        } else {
            upload_fn
        };
        res.rotate_chart = mode == GameMode::EditChartPlay
            || (res.config.auto_flip_enabled
                && matches!(mode, GameMode::Normal | GameMode::NoRetry | GameMode::Exercise)
                && !res.info.replica_play.auto_flip_intervals.is_empty());
        let correct_sound_events = Self::build_correct_sound_events(&chart, &res);
        // Prepare extra sfx from chart.hitsounds
        chart.hitsounds.iter().for_each(|(name, clip)| {
            if let Ok(clip) = res.create_sfx(clip.clone()) {
                res.extra_sfxs.insert(name.clone(), clip);
            }
        });
        // Keep custom clip references only when needed to rebuild the timeline
        // after changing note conversion in the chart playback editor.
        if !res.config.correct_sound {
            chart.hitsounds.clear();
        }

        let exercise_range = (chart.offset + info_offset + res.config.offset) as f64..res.track_length;

        let mut judge = Judge::new(&chart);
        judge.set_touch_debug_enabled(res.config.touch_input_debug_report);

        tracing::info!(stage = "music_begin", noise = res.has_noise_area, "noise-area initialization");
        let music = Self::new_music(&mut res, &mode, &correct_sound_events, (chart.offset + info_offset) as f64)?;
        tracing::info!(stage = "music_ready", "noise-area initialization");
        let judgement_range_debug_used = res.config.judgement_range_debug.enabled;
        let mut play_report = PlayReportRecorder::new(
            &chart,
            &res.info,
            &chart_bytes,
            chart_format.clone(),
            (res.config.auto_export_play_report || res.config.touch_input_debug_report) && report_fn.is_some(),
        );
        play_report.set_chart_offset((chart.offset + info_offset + res.config.offset) as f64);
        let replay = (res.config.replay_recording_enabled && matches!(mode, GameMode::Normal | GameMode::NoRetry))
            .then(|| crate::replay::Recorder::new(&res, replay_assets.clone(), false, [0., res.track_length]));
        res.replay_capture = replay.is_some();
        Ok(Self {
            replay,
            replay_assets,
            replay_armed: false,
            replay_view_hud: None,
            replay_view_profile: None,
            replay_view_angle: None,
            replay_audio_speed: 0.,
            replay_audio_playing: false,
            replay_save_cooldown: f64::NEG_INFINITY,
            should_exit: false,
            next_scene: None,

            mode,
            res,
            chart,
            judge,
            gl: unsafe { get_internal_gl() },
            player,
            chart_bytes,
            chart_format,
            effects,
            info_offset,

            offset_analysis: OffsetAnalysisPanel::new(),
            chart_play_editor: Default::default(),
            note_conversion_backup,
            auto_flip: Default::default(),

            first_in: false,
            exercise_range,
            exercise_press: None,
            exercise_btns: (RectButton::new(), RectButton::new()),
            exercise_note_flow_speed: 1.,
            exercise_note_flow_locked: false,
            exercise_judgement_percent: 100.,
            exercise_judgement_locked: false,
            exercise_extra_open: false,

            music,
            noise_renderer,
            correct_sound_events,

            state: State::Starting,
            last_update_time: 0.,
            pause_rewind: None,
            pause_first_time: f32::NEG_INFINITY,

            bad_notes: Vec::new(),

            upload_fn,
            update_fn,
            save_fn,
            report_fn,

            best_record: None,

            touch_points: Vec::new(),

            fps_frame_count: 0,
            fps_total_time: 0.0,
            fps_last_frame_time: 0.0,
            judgement_range_debug_used,
            play_report,
            timing_bar: crate::timing_bar::TimingBar::default(),
            preview_combo: None,
            exercise_state_reset_pending: false,

            dead: false,
        })
    }

    fn begin_armed_replay(&mut self) {
        if crate::replay::start_practice_recording(
            self.res.config.replay_recording_enabled,
            self.mode == GameMode::Exercise,
            self.replay_armed,
            &mut self.replay,
            || crate::replay::Recorder::new(&self.res, self.replay_assets.clone(), true, [self.exercise_range.start, self.exercise_range.end]),
        ) {
            self.res.replay_capture = true;
            self.res.replay_fx.clear();
            self.res.replay_sounds.clear();
        }
    }
    fn reset_replay(&mut self) {
        // Reset paths must save the tape before replacing the recorder, even
        // when an automatic boundary or a retry bypasses a render-time pause.
        self.finish_practice_replay();
        self.replay = (self.res.config.replay_recording_enabled && matches!(self.mode, GameMode::Normal | GameMode::NoRetry))
            .then(|| crate::replay::Recorder::new(&self.res, self.replay_assets.clone(), false, [0., self.res.track_length]));
        self.res.replay_capture = self.replay.is_some();
        self.res.replay_fx.clear();
        self.res.replay_sounds.clear();
    }
    fn capture_replay_frame(&mut self, tm: &TimeManager) {
        if let Some(recorder) = &mut self.replay {
            let paused = tm.paused() && !self.auto_flip.rotating();
            if !paused || !recorder.paused {
                recorder.frame(
                    &mut self.res,
                    &self.chart,
                    &self.judge,
                    tm.real_time(),
                    tm.now(),
                    self.auto_flip.angle(tm.real_time()),
                    self.pause_rewind.is_some(),
                    self.auto_flip.rotating(),
                    !self.music.paused(),
                    &self.bad_notes,
                    match self.state {
                        State::Starting => 0,
                        State::BeforeMusic => 1,
                        State::Playing => 2,
                        State::Ending => 3,
                    },
                    self.pause_rewind.and_then(|at| {
                        let dt = tm.now() - at;
                        let n = 3 - dt.floor() as i32;
                        (n > 0).then_some((n.clamp(1, 3) as u8, (1. - dt as f32 / 3.).clamp(0., 1.)))
                    }),
                );
            }
            recorder.paused = paused;
        }
    }
    fn save_replay(&mut self, real: f64) {
        if real - self.replay_save_cooldown < 1. {
            return;
        }
        if let Some(recorder) = &self.replay {
            match recorder.payload() {
                Ok(payload) => {
                    crate::replay::save_background(payload);
                    self.replay_save_cooldown = real;
                }
                Err(err) => {
                    show_message(format!("{err:#}")).error();
                }
            }
        }
    }
    fn finish_practice_replay(&mut self) {
        if let Some(recorder) = crate::replay::take_practice_recording(self.mode == GameMode::Exercise, &mut self.replay_armed, &mut self.replay) {
            self.res.replay_capture = false;
            tracing::info!(frames = recorder.frames.len(), "practice replay finalized before pause/reset");
            match recorder.into_payload() {
                Ok(payload) => crate::replay::save_background(payload),
                Err(err) => {
                    show_message(format!("{err:#}")).error();
                }
            }
        }
    }
    pub fn prepare_replay(&mut self) {
        self.replay = None;
        self.res.replay_capture = false;
        self.res.replay_view = true;
        self.res.config.interactive = false;
        self.res.config.timing_bar.enabled = false;
        self.note_conversion_backup
            .apply(&mut self.chart, self.res.info.replica_play.note_conversion);
        self.judge = Judge::new(&self.chart);
        self.correct_sound_events = Self::build_correct_sound_events(&self.chart, &self.res);
        self.res.rotate_chart = self.res.config.auto_flip_enabled && !self.res.info.replica_play.auto_flip_intervals.is_empty();
    }
    pub fn apply_replay_notes(&mut self, frame: &crate::replay::Frame) {
        for delta in &frame.notes {
            if let Some(note) = self
                .chart
                .lines
                .get_mut(delta.line as usize)
                .and_then(|l| l.notes.get_mut(delta.note as usize))
            {
                note.judge = delta.state.restore();
            }
        }
        for (line, (notes, cursor)) in self.chart.lines.iter().zip(&mut self.judge.notes) {
            *cursor = notes
                .iter()
                .position(|id| !matches!(line.notes[*id as usize].judge, crate::judge::JudgeStatus::Judged))
                .unwrap_or(notes.len());
        }
    }
    pub fn clear_replay_effects(&mut self) {
        self.res.emitter.emitter.clear();
        self.res.emitter.emitter_square.clear();
        self.bad_notes.clear();
    }
    pub fn emit_replay_effects(&mut self, frame: &crate::replay::Frame, sounds: bool) {
        for fx in &frame.fx {
            let size = self.res.emitter.emitter.config.size;
            let square = self.res.emitter.emitter_square.config.size;
            self.res.emitter.emitter.config.size *= fx.size_ratio;
            self.res.emitter.emitter_square.config.size *= fx.size_ratio;
            self.res
                .emitter
                .emit_at(vec2(fx.position[0], fx.position[1]), fx.rotation, crate::replay::color(fx.color));
            self.res.emitter.emitter.config.size = size;
            self.res.emitter.emitter_square.config.size = square;
        }
        if sounds {
            for sound in &frame.sounds {
                sound.play(&mut self.res);
            }
        }
    }
    pub fn sync_replay_audio(&mut self, frame: &crate::replay::Frame, rate: f32, force: bool, playing: bool) -> Result<()> {
        let target_speed = frame.speed as f64 * rate as f64;
        let needs_rebuild = (self.replay_audio_speed - target_speed).abs() > 1e-5 || self.music.preserves_pitch() != frame.preserve_pitch;
        if needs_rebuild {
            let offset = self.offset() as f64;
            self.music.pause()?;
            self.music = Music::new_replay(
                &mut self.res.audio,
                self.res.music.clone(),
                MusicParams {
                    amplifier: self.res.config.volume_music as _,
                    playback_rate: target_speed,
                    ..Default::default()
                },
                frame.preserve_pitch,
                self.res.has_noise_area && !self.res.config.noise_area.music_unaffected,
                self.correct_sound_events
                    .as_ref()
                    .map(|events| crate::correct_sound::CorrectSoundTrack::new(events.clone(), offset, self.res.config.volume_sfx)),
            )?;
            self.replay_audio_speed = target_speed;
            self.replay_audio_playing = false;
        }
        let audible = playing && frame.audible && frame.song >= 0. && frame.song < self.res.track_length;
        let song = frame.song.clamp(0., (self.res.track_length - 0.001).max(0.));
        if force || needs_rebuild || audible && (self.music.position() - song).abs() > 0.12 {
            self.music.seek_to(song)?;
        }
        // Compare requested state, not the asynchronous audio-thread acknowledgement.
        // A paused/count-in frame must not enqueue another Pause every render frame.
        if let Some(playing) = crate::replay::audio_transition(self.replay_audio_playing, audible) {
            if playing {
                self.music.play()?;
            } else {
                self.music.pause()?;
            }
            self.replay_audio_playing = playing;
        }
        self.music.set_noise_touch(!frame.blocked.is_empty(), get_frame_time());
        Ok(())
    }
    pub fn advance_replay_effects(&mut self, dt: f32) {
        self.res.emitter.emitter.advance(dt);
        self.res.emitter.emitter_square.advance(dt);
    }
    pub fn render_replay_frame(&mut self, frame: &crate::replay::Frame, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.res.update_size(ui.viewport);
        self.state = match frame.phase {
            0 => State::Starting,
            1 => State::BeforeMusic,
            3 => State::Ending,
            _ => State::Playing,
        };
        self.res.time = frame.chart;
        self.res.alpha = frame.alpha;
        self.res.note_flow_speed = frame.flow;
        self.res.config.speed = frame.speed;
        self.res.config.practice_judgement_multiplier = frame.judgement_multiplier;
        self.res.practice_view = crate::practice_view::PracticeView {
            scale_percent: frame.view[0],
            center_x: frame.view[1] * self.res.camera.viewport.map_or(frame.viewport_width, |vp| vp.2 as f32) / frame.viewport_width.max(1.),
            center_y: frame.view[2] * self.res.camera.viewport.map_or(frame.viewport_width, |vp| vp.2 as f32) / frame.viewport_width.max(1.),
        };
        self.res.auto_flip_y = frame.flip_y;
        self.res.judge_line_color = crate::replay::color(frame.line_color);
        self.bad_notes = frame
            .bad_visuals
            .iter()
            .map(|n| BadNote {
                time: n.time,
                kind: n.kind.clone(),
                matrix: Matrix::from_column_slice(&n.matrix),
            })
            .collect();
        self.replay_view_profile = Some(frame.profile);
        self.replay_view_angle = Some(frame.angle);
        self.replay_view_hud = Some(frame.hud.clone());
        self.judge.noise_state.blocked_ids = frame.blocked.iter().copied().collect();
        self.judge.noise_state.positions = frame.noise_positions.iter().map(|p| vec2(p[0], p[1])).collect();
        for (h, (id, p, scale)) in self.judge.noise_state.hovers.iter_mut().zip(&frame.hovers) {
            h.finger = *id;
            h.position = vec2(p[0], p[1]);
            h.scale = *scale;
        }
        self.chart.update(&mut self.res);
        tm.seek_to(frame.song);
        <Self as Scene>::render(self, tm, ui)
    }

    fn build_correct_sound_events(chart: &Chart, res: &Resource) -> Option<Arc<Vec<crate::correct_sound::ScheduledHit>>> {
        res.config.correct_sound.then(|| {
            let mut events = Vec::new();
            for line in &chart.lines {
                for note in line.notes.iter().filter(|note| !note.fake) {
                    let clip = match &note.hitsound {
                        crate::judge::HitSound::None => None,
                        crate::judge::HitSound::Click => Some(res.res_pack.sfx_click.clone()),
                        crate::judge::HitSound::Drag => Some(res.res_pack.sfx_drag.clone()),
                        crate::judge::HitSound::Flick => Some(res.res_pack.sfx_flick.clone()),
                        crate::judge::HitSound::Custom(name) => chart.hitsounds.get(name).cloned(),
                    };
                    if let Some(clip) = clip {
                        events.push(crate::correct_sound::ScheduledHit { time: note.time, clip });
                    }
                }
            }
            events.sort_by(|a, b| a.time.total_cmp(&b.time));
            Arc::new(events)
        })
    }

    fn new_music(
        res: &mut Resource,
        mode: &GameMode,
        events: &Option<Arc<Vec<crate::correct_sound::ScheduledHit>>>,
        chart_offset: f64,
    ) -> Result<Music> {
        let create = if *mode == GameMode::View { Music::new_replay } else { Music::new };
        create(
            &mut res.audio,
            res.music.clone(),
            MusicParams {
                amplifier: res.config.volume_music as _,
                playback_rate: res.config.speed as _,
                ..Default::default()
            },
            res.config.preserve_pitch_for(*mode == GameMode::Exercise),
            res.has_noise_area && !res.config.noise_area.music_unaffected,
            events.as_ref().map(|events| {
                crate::correct_sound::CorrectSoundTrack::new(events.clone(), chart_offset + res.config.offset as f64, res.config.volume_sfx)
            }),
        )
    }

    fn touch_scale(&self) -> f32 {
        (screen_width() / screen_height()) / self.res.aspect_ratio
    }

    fn locked_note_flow_speed(playback_speed: f32) -> f32 {
        ((1. / playback_speed.max(PLAYBACK_SPEED_MIN) * 1000.).round() / 1000.).clamp(NOTE_FLOW_SPEED_MIN, NOTE_FLOW_SPEED_MAX)
    }

    fn note_flow_locked(&self) -> bool {
        self.res
            .config
            .note_flow_inverse_for(self.mode == GameMode::Exercise, self.exercise_note_flow_locked)
    }

    fn practice_note_flow_speed(&self) -> f32 {
        if self.note_flow_locked() {
            Self::locked_note_flow_speed(self.res.config.speed)
        } else if self.mode == GameMode::Exercise {
            self.exercise_note_flow_speed
        } else {
            1.
        }
    }

    fn effective_note_flow_speed(&self) -> f32 {
        self.res.config.effective_note_flow_speed(self.practice_note_flow_speed())
    }

    fn practice_judgement_multiplier(&self) -> f32 {
        if self.mode != GameMode::Exercise {
            1.
        } else if self.exercise_judgement_locked {
            1. / self.res.config.speed.max(PLAYBACK_SPEED_MIN)
        } else {
            self.exercise_judgement_percent / 100.
        }
    }

    fn sync_note_flow_speed(&mut self) {
        if self.res.replay_view {
            return;
        }
        self.res.config.practice_judgement_multiplier = self.practice_judgement_multiplier();
        self.res.note_flow_speed = self.effective_note_flow_speed();
    }

    fn report_mode(&self) -> Option<ReportGameMode> {
        match self.mode {
            GameMode::Normal => Some(ReportGameMode::Normal),
            GameMode::Exercise => Some(ReportGameMode::Exercise),
            GameMode::NoRetry => Some(ReportGameMode::NoRetry),
            GameMode::TweakOffset | GameMode::EditChartPlay | GameMode::View => None,
        }
    }

    fn report_range(&self) -> Range<f64> {
        if self.mode == GameMode::Exercise {
            self.exercise_range.clone()
        } else {
            self.offset().min(0.) as f64..self.res.track_length
        }
    }

    fn begin_play_report_if_needed(&mut self, song_time: f64, real_time: f64) {
        let Some(mode) = self.report_mode() else {
            return;
        };
        let range = self.report_range();
        let note_flow_speed = self.effective_note_flow_speed();
        self.play_report.begin_if_needed(
            mode,
            &self.res.config,
            range.start,
            range.end,
            song_time,
            real_time,
            note_flow_speed,
            self.note_flow_locked(),
        );
    }

    fn capture_play_report_events(&mut self, song_time: f64, real_time: f64) {
        let events = self.judge.take_report_judgements();
        if let Some(recorder) = &mut self.replay {
            for event in &events {
                recorder.outcome(event, &self.chart, &self.res);
            }
        }
        // Hold heads are emitted immediately, while Tap finals are queued until
        // the end of judge.update. Restore input-time order within each frame.
        let mut ordered: Vec<_> = events.iter().collect();
        let input_time = |event: &crate::judge::JudgeReportEvent| {
            self.chart
                .lines
                .get(event.line_id as usize)
                .and_then(|line| line.notes.get(event.note_id as usize))
                .map_or(event.time, |note| {
                    if event.judgement == Ok(crate::judge::Judgement::Miss) {
                        event.time
                    } else {
                        note.time + event.difference * self.res.config.speed as f64
                    }
                })
        };
        ordered.sort_by(|a, b| input_time(a).total_cmp(&input_time(b)));
        for event in ordered {
            if let Some(note) = self
                .chart
                .lines
                .get(event.line_id as usize)
                .and_then(|line| line.notes.get(event.note_id as usize))
            {
                let mut display_event = *event;
                if event.judgement == Ok(crate::judge::Judgement::Miss) {
                    display_event.difference = (event.time - note.time) / self.res.config.speed as f64;
                }
                let scheme =
                    (self.res.config.judgement_mode == crate::config::JudgementMode::Custom).then(|| self.res.config.effective_custom_judgement());
                self.timing_bar.record_scaled(
                    &display_event,
                    &note.kind,
                    &self.res.config.timing_bar,
                    real_time,
                    crate::timing_bar::profile_extent(&self.judge.judgement_range_profile(&self.res.config), scheme.as_deref()),
                );
            }
        }
        self.play_report.record_events(events);
        self.play_report
            .record_touch_debug(self.judge.take_touch_debug_records(), song_time, real_time);
    }

    fn finish_play_report(&mut self, reason: ReportEndReason, song_time: f64, real_time: f64) {
        self.capture_play_report_events(song_time, real_time);
        let Some(mode) = self.report_mode() else {
            self.play_report.discard_unstarted();
            return;
        };
        let result = self.judge.result();
        let mut reasons = upload_ineligible_reasons(&self.res.config, mode, self.judgement_range_debug_used);
        if self.res.config.auto_flip_enabled && !self.res.info.replica_play.auto_flip_intervals.is_empty() {
            reasons.push("auto_flip_intervals");
        }
        if self.res.info.replica_play.note_conversion != crate::chart_play::NoteConversion::Original {
            reasons.push("note_conversion");
        }
        if self
            .res
            .config
            .online_replica_active(self.res.info.id.is_some(), self.res.info.replica_play.blocks_score_upload())
        {
            reasons.push("online_replica_features");
        }
        let report =
            self.play_report
                .finish(reason, song_time, real_time, &result, self.judge.real_time_accuracy(), self.judgement_range_debug_used, reasons);
        let (Some(report), Some(export)) = (report, self.report_fn.as_ref()) else {
            return;
        };
        match export(&report) {
            Ok(path) => show_message(tl!("report-exported", "path" => path)).duration(3.).ok(),
            Err(err) => show_message(tl!("report-export-failed", "error" => format!("{err:#}")))
                .duration(5.)
                .error(),
        };
    }

    fn reset_exercise_judgement_to(&mut self, song_time: f64) {
        self.bad_notes.clear();
        self.judge.reset_with_touch_debug_source(TouchDebugClearSource::ExerciseSettingsReset);
        self.timing_bar.clear();
        self.chart.reset();
        let chart_time = (song_time - self.offset() as f64).max(0.);
        self.judge.advance_to(&mut self.chart, chart_time);
        self.res.judge_line_color = self.res.res_pack.info.color_perfect();
    }

    /// Static editor preview: reuse the complete gameplay rendering path without
    /// playing audio, updating judgements, or invoking score/report callbacks.
    pub fn render_timing_preview(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.preview_combo = Some(666);
        self.state = State::Playing;
        self.res.alpha = 1.;
        self.res.time = 30.;
        tm.seek_to(30.);
        self.chart.update(&mut self.res);
        <Self as Scene>::render(self, tm, ui)
    }

    fn ui(&mut self, ui: &mut Ui, tm: &mut TimeManager) -> Result<()> {
        let chart_offset = self.offset() as f64;
        let time = tm.now();
        let p = match self.state {
            State::Starting => {
                if time <= Self::BEFORE_TIME {
                    1. - (1. - time / Self::BEFORE_TIME).powi(3)
                } else {
                    1.
                }
            }
            State::BeforeMusic => 1.,
            State::Playing => 1.,
            State::Ending => {
                let t = time - self.res.track_length - WAIT_TIME;
                1. - (t / (AFTER_TIME + 0.3)).min(1.).powi(2)
            }
        } as f32;
        let combo = self
            .replay_view_hud
            .as_ref()
            .map(|h| h.combo)
            .or(self.preview_combo)
            .unwrap_or_else(|| self.judge.combo());
        let replay_score = self.replay_view_hud.as_ref().map_or_else(|| self.judge.score(), |h| h.score);
        let replay_acc = self
            .replay_view_hud
            .as_ref()
            .map_or_else(|| self.judge.real_time_accuracy(), |h| h.accuracy);
        let res = &mut self.res;
        let eps = 2e-2 / res.aspect_ratio;
        let top = -1. / res.aspect_ratio;
        let pause_w = 0.015;
        let pause_h = pause_w * 3.2;
        let pause_center = Point::new(pause_w * 4.0 - 1., top + eps * 3.5 - (1. - p) * 0.4 + pause_h / 2.);
        if res.config.interactive
            && !tm.paused()
            && self.pause_rewind.is_none()
            && Judge::get_touches().iter().any(|touch| {
                touch.phase == TouchPhase::Started && {
                    let p = touch.position;
                    let p = Point::new(p.x, p.y);
                    (pause_center - p).norm() < 0.05
                }
            })
        {
            let t = tm.now() as f32;
            if t - self.pause_first_time > PAUSE_CLICK_INTERVAL && res.config.double_click_to_pause {
                self.pause_first_time = t;
            } else {
                self.pause_first_time = f32::NEG_INFINITY;
                Self::pause_playback(&mut self.music, tm, &mut self.judge, TouchDebugClearSource::PauseButton)?;
                #[cfg(target_env = "ohos")]
                miniquad::native::set_interceptor_state(false);
            }
        }
        ui.alpha(res.alpha, |ui| {
            ui.text("MAGIC BUGFIX TEXT").color(Color::new(0., 0., 0., 0.)).draw();
            if tm.now() as f32 - self.pause_first_time <= PAUSE_CLICK_INTERVAL {
                ui.fill_circle(pause_center.x, pause_center.y, 0.05, Color::new(1., 1., 1., 0.5));
            }

            let margin = 0.03;

            let legacy_aui = !res.info.use_attach_ui_fix.unwrap_or_default();
            let unit_h = if legacy_aui { ui.text("0").measure_using(&PGR_FONT).h } else { 0. };

            // score
            let h = 0.07;
            let score_top = top + eps * 2.2 - (1. - p) * 0.4;
            let score_right = 1. - margin;
            let score = format!("{:07}", replay_score);
            let scale_point = legacy_aui.then(|| {
                let ct = ui.text(&score).size(0.8).measure_using(&PGR_FONT).center();
                (score_right - ct.x, score_top + ct.y)
            });
            self.chart
                .with_element(ui, res, UIElement::Score, scale_point, (score_right, score_top), |ui, c| {
                    ui.text(&score)
                        .pos(score_right, score_top)
                        .anchor(1., 0.)
                        .size(0.8)
                        .color(c)
                        .draw_using(&PGR_FONT);
                    if res.config.show_acc {
                        ui.text(format!("{:05.2}%", replay_acc * 100.))
                            .pos(1. - margin, score_top + h)
                            .anchor(1., 0.)
                            .size(0.4)
                            .color(Color { a: c.a * 0.7, ..c })
                            .draw_using(&PGR_FONT);
                    }
                });

            self.chart.with_element(
                ui,
                res,
                UIElement::Pause,
                legacy_aui.then(|| (pause_center.x, pause_center.y)),
                (pause_center.x - pause_w * 1.5, pause_center.y - pause_h / 2.),
                |ui, c| {
                    let mut r = Rect::new(pause_center.x - pause_w * 1.5, pause_center.y - pause_h / 2., pause_w, pause_h);
                    ui.fill_rect(r, c);
                    r.x += pause_w * 2.;
                    ui.fill_rect(r, c);
                },
            );
            if combo >= 3 {
                if legacy_aui {
                    let combo_top = top + eps * 2. - (1. - p) * 0.4;
                    let btm = self
                        .chart
                        .with_element(ui, res, UIElement::ComboNumber, None, (0., combo_top + unit_h / 2.), |ui, c| {
                            ui.text(combo.to_string())
                                .pos(0., combo_top)
                                .anchor(0.5, 0.)
                                .color(c)
                                .draw_using(&PGR_FONT)
                                .bottom()
                        });
                    let combo_top = btm + 0.01;
                    self.chart
                        .with_element(ui, res, UIElement::Combo, None, (0., combo_top + unit_h * 0.2), |ui, c| {
                            ui.text(if res.config.autoplay() { "AUTOPLAY" } else { "COMBO" })
                                .pos(0., combo_top)
                                .anchor(0.5, 0.)
                                .size(0.4)
                                .color(c)
                                .draw_using(&PGR_FONT);
                        });
                } else {
                    let combo = combo.to_string();
                    let ct = ui.text(&combo).size(1.0).measure().center();
                    let combo_y = top + eps * 2. - (1. - p) * 0.4 + ct.y;
                    let btm = self.chart.with_element(ui, res, UIElement::ComboNumber, None, (0., combo_y), |ui, c| {
                        ui.text(&combo)
                            .pos(0., combo_y)
                            .anchor(0.5, 0.5)
                            .size(1.0)
                            .color(c)
                            .draw_using(&PGR_FONT)
                            .bottom()
                    });
                    let ct = ui.text("COMBO").size(0.4).measure().center();
                    let combo_top = btm + 0.01 + ct.y;
                    self.chart.with_element(ui, res, UIElement::Combo, None, (0., combo_top), |ui, c| {
                        ui.text(if res.config.autoplay() { "AUTOPLAY" } else { "COMBO" })
                            .pos(0., combo_top)
                            .anchor(0.5, 0.5)
                            .size(0.4)
                            .color(c)
                            .draw_using(&PGR_FONT);
                    });
                }
            }
            // magic to make score visible, refer to phira/src/rate.rs#L219
            ui.text("").draw_using(&PGR_FONT);
            let lf = -1. + margin;
            let bt = -top - eps * 2.8 + (1. - p) * 0.4;
            let scale_point = legacy_aui.then(|| {
                let ct = ui.text(&res.info.name).size(0.5).measure().center();
                (lf + ct.x, bt - ct.y)
            });
            self.chart.with_element(ui, res, UIElement::Name, scale_point, (lf, bt), |ui, c| {
                ui.text(&res.info.name)
                    .pos(lf, bt)
                    .anchor(0., 1.)
                    .size(0.5)
                    .color(c)
                    .max_width(0.8)
                    .draw();
            });

            let scale_point = legacy_aui.then(|| {
                let ct = ui.text(&res.info.level).size(0.5).measure().center();
                (-lf - ct.x, bt - ct.y)
            });
            self.chart.with_element(ui, res, UIElement::Level, scale_point, (-lf, bt), |ui, c| {
                ui.text(&res.info.level).pos(-lf, bt).anchor(1., 1.).size(0.5).color(c).draw();
            });

            let hw = 0.003;
            let height = eps * 1.0;
            let dest = (2. * (res.time + chart_offset) / res.track_length).clamp(0., 2.) as f32;
            self.chart
                .with_element(ui, res, UIElement::Bar, Some((-1., top + height / 2.)), (-1., top + height / 2.), |ui, color| {
                    ui.fill_rect(Rect::new(-1., top, dest, height), semi_white(0.6));
                    ui.fill_rect(Rect::new(-1. + dest - hw, top, hw * 2., height), WHITE);
                    crate::ui::chart_play_editor::draw_flip_markers(
                        ui,
                        &res.info.replica_play,
                        -1.,
                        top + height / 2.,
                        2.,
                        chart_offset,
                        res.track_length,
                    );
                });
        });
        Ok(())
    }

    fn overlay_ui(&mut self, ui: &mut Ui, tm: &mut TimeManager) -> Result<()> {
        ui.abs_scope(|ui| self.overlay_ui_absolute(ui, tm))
    }

    fn overlay_ui_absolute(&mut self, ui: &mut Ui, tm: &mut TimeManager) -> Result<()> {
        if !tm.paused() || self.mode != GameMode::Exercise {
            Ui::clear_practice_speed_drag();
        }
        if self.res.replay_view || self.mode == GameMode::EditChartPlay || self.auto_flip.rotating() {
            return Ok(());
        }
        let c = semi_white(self.res.alpha);
        let practice_report_cutoff = tm.now();
        let mut practice_settings_changed = false;
        let mut retry_requested = None;
        let mut resume_requested = false;
        let res = &mut self.res;
        if tm.paused() {
            let h = 1. / res.aspect_ratio;
            draw_rectangle(-1., -h, 2., h * 2., Color::new(0., 0., 0., 0.6));
            let o = if self.mode == GameMode::Exercise { -0.3 } else { 0. };
            let s = 0.06;
            let w = 0.05;
            let no_retry = self.mode == GameMode::NoRetry;
            draw_texture_ex(
                *res.icon_back,
                -s * 3. - w,
                -s + o,
                c,
                DrawTextureParams {
                    dest_size: Some(vec2(s * 2., s * 2.)),
                    ..Default::default()
                },
            );
            let r = Rect::new(0., o, 0., 0.).feather(s);
            let disabled_color = semi_white(res.alpha * 0.4);
            ui.fill_rect(r, (*res.icon_retry, r.feather(0.02), ScaleType::Fit, if no_retry { disabled_color } else { c }));
            draw_texture_ex(
                *res.icon_resume,
                s + w,
                -s + o,
                if self.dead { disabled_color } else { c },
                DrawTextureParams {
                    dest_size: Some(vec2(s * 2., s * 2.)),
                    ..Default::default()
                },
            );
            if res.config.interactive {
                let mut clicked = None;
                for touch in Judge::get_touches() {
                    if touch.phase != TouchPhase::Started {
                        continue;
                    }
                    let p = touch.position;
                    let p = Point::new(p.x, p.y);
                    for i in -1..=1 {
                        let ct = Point::new((s * 2. + w) * i as f32, o);
                        let d = p - ct;
                        if d.x.abs() <= s && d.y.abs() <= s {
                            clicked = Some(i);
                            break;
                        }
                    }
                }
                if no_retry && clicked == Some(0) || self.dead && clicked == Some(1) {
                    clicked = None;
                }
                let mut pos = self.music.position();
                if self.mode == GameMode::Exercise {
                    pos = tm.now();
                }
                let preserve_pitch = res.config.preserve_pitch_for(self.mode == GameMode::Exercise);
                if clicked == Some(0) && ((tm.speed - res.config.speed as f64).abs() > 1e-6 || self.music.preserves_pitch() != preserve_pitch) {
                    self.music = Self::new_music(res, &self.mode, &self.correct_sound_events, (self.chart.offset + self.info_offset) as f64)?;
                }
                match clicked {
                    Some(-1) => {
                        self.should_exit = true;
                        #[cfg(target_env = "ohos")]
                        miniquad::native::set_interceptor_state(false);
                    }
                    Some(0) => {
                        retry_requested = Some(pos);
                        #[cfg(target_env = "ohos")]
                        miniquad::native::set_interceptor_state(true);
                    }
                    Some(1) => {
                        resume_requested = true;
                    }
                    _ => {}
                }
            }
            if self.mode == GameMode::Exercise {
                let previous_playback_speed = self.res.config.speed;
                let previous_manual_note_flow_speed = self.exercise_note_flow_speed;
                let previous_note_flow_lock = self.exercise_note_flow_locked;
                let previous_judgement_percent = self.exercise_judgement_percent;
                let previous_judgement_lock = self.exercise_judgement_locked;
                let asp = self.touch_scale();
                for touch in ui.ensure_touches() {
                    touch.position *= asp;
                }
                let previous_pitch = self.res.config.practice_preserve_pitch;
                ui.scope(|ui| {
                    ui.dx(-0.9);
                    ui.dy(-0.48);
                    ui.with(Matrix::new_scaling(crate::practice_view::EXTRA_PANEL_SCALE), |ui| {
                        let label = if self.exercise_extra_open {
                            "▾ 额外功能"
                        } else {
                            "▸ 额外功能"
                        };
                        if ui.button("exercise_extra", Rect::new(0., -0.01, 0.38, 0.065), label) {
                            self.exercise_extra_open ^= true;
                            Ui::clear_practice_speed_drag();
                        }
                        if self.exercise_extra_open {
                            ui.dy(crate::practice_view::extra_panel_offset(0.08));
                            ui.text("改变视角").size(0.4).draw();
                            ui.dy(crate::practice_view::extra_panel_offset(0.055));
                            ui.practice_value_slider(
                                "exercise_view_scale",
                                "大小",
                                crate::practice_view::SCALE,
                                &mut self.res.practice_view.scale_percent,
                                Some(0.40),
                            );
                            ui.dy(crate::practice_view::extra_panel_offset(0.175));
                            ui.practice_value_slider(
                                "exercise_view_x",
                                "中心 X",
                                crate::practice_view::CENTER,
                                &mut self.res.practice_view.center_x,
                                Some(0.40),
                            );
                            ui.dy(crate::practice_view::extra_panel_offset(0.175));
                            ui.practice_value_slider(
                                "exercise_view_y",
                                "中心 Y",
                                crate::practice_view::CENTER,
                                &mut self.res.practice_view.center_y,
                                Some(0.40),
                            );
                        }
                    });
                });
                ui.scope(|ui| {
                    ui.dx(-0.22);
                    ui.dy(-0.49);
                    ui.checkbox(tl!("preserve-pitch"), &mut self.res.config.practice_preserve_pitch);
                });
                ui.scope(|ui| {
                    ui.dx(0.3);
                    ui.dy(-0.48);
                    ui.practice_speed_slider(
                        "exercise_speed",
                        tl!("speed"),
                        PLAYBACK_SPEED_MIN..PLAYBACK_SPEED_MAX,
                        PLAYBACK_SPEED_STEP,
                        &mut self.res.config.speed,
                        Some(0.46),
                    );
                    ui.dy(0.165);
                    let mut displayed = self.practice_note_flow_speed();
                    let previous = displayed;
                    ui.practice_speed_slider(
                        "exercise_note_flow",
                        tl!("note-flow-speed"),
                        NOTE_FLOW_SPEED_MIN..NOTE_FLOW_SPEED_MAX,
                        NOTE_FLOW_SPEED_STEP,
                        &mut displayed,
                        Some(0.46),
                    );
                    if (displayed - previous).abs() > f32::EPSILON {
                        self.exercise_note_flow_speed = displayed;
                        self.exercise_note_flow_locked = false;
                    }
                    ui.scope(|ui| {
                        ui.dx(0.40);
                        ui.with(Matrix::new_scaling(0.65), |ui| {
                            ui.checkbox_with_id("exercise_note_flow_inverse", "速度的倒数", &mut self.exercise_note_flow_locked);
                        });
                    });
                    ui.dy(0.165);
                    let mut displayed = self.practice_judgement_multiplier() * 100.;
                    let previous = displayed;
                    ui.practice_value_slider(
                        "exercise_judgement",
                        "判定区间倍数",
                        crate::practice_view::SliderSpec {
                            min: 10.,
                            max: 300.,
                            midpoint: 100.,
                            step: 1.,
                            suffix: "%",
                        },
                        &mut displayed,
                        Some(0.46),
                    );
                    if (displayed - previous).abs() > f32::EPSILON {
                        self.exercise_judgement_percent = displayed;
                        self.exercise_judgement_locked = false;
                    }
                    ui.scope(|ui| {
                        ui.dx(0.40);
                        ui.with(Matrix::new_scaling(0.65), |ui| {
                            ui.checkbox_with_id("exercise_judgement_inverse", "速度的倒数", &mut self.exercise_judgement_locked);
                        });
                    });
                });
                if self.res.config.practice_preserve_pitch != previous_pitch {
                    if let Err(error) = crate::practice_audio::save_preference(self.res.config.practice_preserve_pitch) {
                        self.res.config.practice_preserve_pitch = previous_pitch;
                        show_message(format!("{error:#}")).error();
                    }
                }
                practice_settings_changed |= (self.res.config.speed - previous_playback_speed).abs() > f32::EPSILON
                    || (self.exercise_note_flow_speed - previous_manual_note_flow_speed).abs() > f32::EPSILON
                    || self.exercise_note_flow_locked != previous_note_flow_lock
                    || self.exercise_judgement_percent != previous_judgement_percent
                    || self.exercise_judgement_locked != previous_judgement_lock;
                let viewport_width = self.res.camera.viewport.map_or(screen_width(), |vp| vp.2 as f32);
                let timeline_y = crate::practice_view::timeline_position(screen_height(), viewport_width);
                ui.dy(timeline_y);
                let hw = 0.7;
                let h = 0.06;
                let rad = 0.03;
                let eh = 0.12_f32.min((1. / self.res.aspect_ratio - timeline_y - rad - 0.01).max(0.));
                let sp = self.offset().min(0.) as f64;
                ui.fill_rect(Rect::new(-hw, -h, hw * 2., h * 2.), GRAY);
                let st = -hw + ((self.exercise_range.start - sp) / (self.res.track_length - sp)) as f32 * hw * 2.;
                let en = -hw + ((self.exercise_range.end - sp) / (self.res.track_length - sp)) as f32 * hw * 2.;
                let t = tm.now();
                let cur = -hw + ((t - sp) / (self.res.track_length - sp)) as f32 * hw * 2.;
                ui.fill_rect(Rect::new(st, -h, en - st, h * 2.), WHITE);
                crate::ui::chart_play_editor::draw_flip_markers(
                    ui,
                    &self.res.info.replica_play,
                    -hw,
                    0.,
                    hw * 2.,
                    self.offset() as f64 - sp,
                    self.res.track_length - sp,
                );
                ui.fill_rect(Rect::new(st, -eh, 0., eh + h).feather(0.005), BLUE);
                ui.fill_circle(st, -eh, rad, BLUE);
                if self.exercise_press.is_none() {
                    let r = ui.rect_to_global(Rect::new(st, -eh, 0., 0.).feather(rad));
                    self.exercise_press = Judge::get_touches()
                        .iter()
                        .find(|it| it.phase == TouchPhase::Started && r.contains(it.position))
                        .map(|it| (-1, it.id));
                }
                ui.fill_rect(Rect::new(en, -h, 0., eh + h).feather(0.005), RED);
                ui.fill_circle(en, eh, rad, RED);
                if self.exercise_press.is_none() {
                    let r = ui.rect_to_global(Rect::new(en, eh, 0., 0.).feather(rad));
                    self.exercise_press = Judge::get_touches()
                        .iter()
                        .find(|it| it.phase == TouchPhase::Started && r.contains(it.position))
                        .map(|it| (1, it.id));
                }
                ui.fill_rect(Rect::new(cur, -h, 0., h * 2.).feather(0.005), GREEN);
                ui.fill_circle(cur, 0., rad, GREEN);
                if self.exercise_press.is_none() {
                    let r = ui.rect_to_global(Rect::new(cur, 0., 0., 0.).feather(rad));
                    self.exercise_press = Judge::get_touches()
                        .iter()
                        .find(|it| it.phase == TouchPhase::Started && r.contains(it.position))
                        .map(|it| (0, it.id));
                }
                ui.text(fmt_time(t as f32)).pos(0., -0.23).anchor(0.5, 0.).size(0.8).draw();
                if let Some((ctrl, id)) = &self.exercise_press {
                    if let Some(touch) = Judge::get_touches().iter().rfind(|it| it.id == *id) {
                        let x = touch.position.x;
                        let p = (x + hw) as f64 / (hw * 2.) as f64 * (self.res.track_length - sp) + sp;
                        let p = if self.res.track_length - sp <= 3. || *ctrl == 0 {
                            p.clamp(sp, self.res.track_length)
                        } else {
                            p.clamp(
                                if *ctrl == -1 { sp } else { self.exercise_range.start + 3. },
                                if *ctrl == -1 {
                                    self.exercise_range.end - 3.
                                } else {
                                    self.res.track_length
                                },
                            )
                        };
                        if *ctrl == 0 {
                            tm.seek_to(p);
                            self.music.seek_to(p)?;
                            practice_settings_changed = true;
                        } else {
                            let previous = if *ctrl == -1 {
                                self.exercise_range.start
                            } else {
                                self.exercise_range.end
                            };
                            *(if *ctrl == -1 {
                                &mut self.exercise_range.start
                            } else {
                                &mut self.exercise_range.end
                            }) = p;
                            practice_settings_changed |= (p - previous).abs() > f64::EPSILON;
                        }
                        if matches!(touch.phase, TouchPhase::Cancelled | TouchPhase::Ended) {
                            self.exercise_press = None;
                        }
                    }
                }
                ui.dy(0.2);
                let r = ui.text(tl!("to")).size(0.8).anchor(0.5, 0.).draw();
                let mut tx = ui
                    .text(fmt_time(self.exercise_range.start as f32))
                    .pos(r.x - 0.02, 0.)
                    .anchor(1., 0.)
                    .size(0.8)
                    .color(BLACK);
                let re = tx.measure();
                self.exercise_btns.0.set(tx.ui, re);
                tx.ui
                    .fill_rect(re.feather(0.01), Color::new(1., 1., 1., if self.exercise_btns.0.touching() { 0.5 } else { 1. }));
                tx.draw();

                let mut tx = ui
                    .text(fmt_time(self.exercise_range.end as f32))
                    .pos(r.right() + 0.02, 0.)
                    .size(0.8)
                    .color(BLACK);
                let re = tx.measure();
                self.exercise_btns.1.set(tx.ui, re);
                tx.ui
                    .fill_rect(re.feather(0.01), Color::new(1., 1., 1., if self.exercise_btns.1.touching() { 0.5 } else { 1. }));
                tx.draw();
                for touch in ui.ensure_touches() {
                    touch.position /= asp;
                }
            }
        }
        if let Some(pos) = retry_requested {
            self.finish_play_report(ReportEndReason::Retry, pos, tm.real_time());
            if self.res.config.challenge_mode {
                self.music.pause()?;
                self.next_scene = Some(NextScene::PopWithResult(Box::new(ChallengeEvent::Restart)));
                return Ok(());
            }
            reset!(self, self.res, tm);
            if self.mode == GameMode::Exercise {
                let chart_time = (self.exercise_range.start - self.offset() as f64).max(0.);
                self.judge.advance_to(&mut self.chart, chart_time);
            }
            self.exercise_state_reset_pending = false;
            // Restart is another run-start, just like Continue. Start only after
            // reset has finished the previous tape and restored the chart state.
            self.begin_armed_replay();
        }
        if practice_settings_changed {
            self.finish_play_report(ReportEndReason::PracticeSettingsChanged, practice_report_cutoff, tm.real_time());
            self.exercise_state_reset_pending = true;
        }
        if resume_requested {
            self.begin_armed_replay();
            self.resume_from_pause(tm, None)?;
            if self.res.rotate_chart {
                let at = (tm.now() + 3. - self.offset() as f64).max(0.);
                self.auto_flip.sync(&self.res.info.replica_play, at, true);
                self.auto_flip.resume_at(at);
                self.sync_auto_flip();
            }
        }
        if let Some(time) = self.pause_rewind {
            let dt = tm.now() - time;
            let t = 3 - dt.floor() as i32;
            if t <= 0 {
                self.pause_rewind = None;
            } else {
                let a = (1. - dt as f32 / 3.) * 1.;
                let h = 1. / self.res.aspect_ratio;
                draw_rectangle(-1., -h, 2., h * 2., Color::new(0., 0., 0., a));
                ui.text(t.to_string()).anchor(0.5, 0.5).size(1.).color(c).draw();
            }
        }
        if self.res.config.touch_debug {
            for touch in Judge::get_touches() {
                ui.fill_circle(touch.position.x, touch.position.y, 0.04, Color { a: 0.4, ..RED });
            }
        }
        for pos in &self.touch_points {
            ui.fill_circle(pos.0, pos.1, 0.04, Color { a: 0.4, ..BLUE });
        }
        Ok(())
    }

    fn pause_replay_rect(top: f32, practice: bool) -> Rect {
        let (width, height) = if practice { (0.60 * 0.7, 0.075 * 0.7) } else { (0.31, 0.075) };
        Rect::new(0.96 - width, -top + 0.025, width, height)
    }

    fn render_pause_replay_control(&mut self, tm: &mut TimeManager, ui: &mut Ui) {
        if !tm.paused()
            || !self.res.config.interactive
            || !self.res.config.replay_recording_enabled
            || self.res.replay_view
            || self.mode == GameMode::EditChartPlay
            || self.auto_flip.rotating()
        {
            return;
        }
        let previous_viewport = self.gl.quad_gl.get_viewport();
        push_camera_state();
        set_camera(&Camera2D {
            render_target: self.res.camera.render_target,
            ..ui.camera()
        });
        ui.abs_scope(|ui| {
            ui.screen_touch_scope(|ui| {
                let practice = self.mode == GameMode::Exercise;
                let rect = Self::pause_replay_rect(ui.top, practice);
                if practice {
                    let label = if self.replay_armed {
                        "✓ 记录回放直到下一次暂停"
                    } else {
                        "记录回放直到下一次暂停"
                    };
                    if ui.button_with_size("replay_arm", rect, label, 0.42 * 0.7) {
                        self.replay_armed = !self.replay_armed;
                    }
                } else if self.replay.is_some() && ui.button("replay_save_pause", rect, "保存目前回放") {
                    self.save_replay(tm.real_time());
                }
            })
        });
        pop_camera_state();
        self.gl.quad_gl.viewport(previous_viewport);
    }

    fn interactive(res: &Resource, state: &State) -> bool {
        res.config.interactive && matches!(state, State::Playing)
    }

    fn offset(&self) -> f32 {
        self.chart.offset + self.res.config.offset + self.info_offset
    }

    /// All button/automatic pauses use the same music, clock and input cleanup.
    fn pause_playback(music: &mut Music, tm: &mut TimeManager, judge: &mut Judge, source: TouchDebugClearSource) -> Result<()> {
        if !music.paused() {
            music.pause()?;
        }
        if !tm.paused() {
            tm.pause();
        }
        judge.clear_touch_input_with_source(source);
        #[cfg(target_env = "ohos")]
        miniquad::native::set_interceptor_state(false);
        Ok(())
    }

    /// Original Phira resume/count-in path, shared by the menu and auto rotation.
    /// Automatic callers supply the exact frozen boundary, not a stale audio sample.
    fn resume_from_pause(&mut self, tm: &mut TimeManager, boundary: Option<f64>) -> Result<()> {
        if !tm.paused() {
            return Ok(());
        }
        if boundary.is_none() {
            self.begin_armed_replay();
        }
        let preserve = self.res.config.preserve_pitch_for(self.mode == GameMode::Exercise);
        let mut pos = boundary.unwrap_or_else(|| {
            if self.mode == GameMode::Exercise {
                tm.now()
            } else {
                self.music.position()
            }
        });
        if (tm.speed - self.res.config.speed as f64).abs() > 1e-6 || self.music.preserves_pitch() != preserve {
            self.music = Self::new_music(&mut self.res, &self.mode, &self.correct_sound_events, (self.chart.offset + self.info_offset) as f64)?;
        }
        if self.mode == GameMode::Exercise && (tm.now() > self.exercise_range.end || tm.now() < self.exercise_range.start) {
            pos = self.exercise_range.start;
            tm.seek_to(pos);
            self.music.seek_to(pos)?;
        }
        self.music.play()?;
        self.res.time -= 3.;
        let dst = pos - 3.;
        let reset_at = (self.mode == GameMode::Exercise && self.exercise_state_reset_pending).then_some(dst.max(self.exercise_range.start));
        if dst < 0. {
            self.music.pause()?;
            self.state = State::BeforeMusic;
        } else {
            self.music.seek_to(dst)?;
        }
        let now = tm.now();
        tm.speed = self.res.config.speed as _;
        tm.resume();
        tm.seek_to(now - 3.);
        self.pause_rewind = Some(tm.now() - 0.2);
        if let Some(time) = reset_at {
            self.reset_exercise_judgement_to(time);
            self.exercise_state_reset_pending = false;
        }
        #[cfg(target_env = "ohos")]
        miniquad::native::set_interceptor_state(true);
        Ok(())
    }

    fn sync_auto_flip(&mut self) {
        let flipped = if self.mode == GameMode::EditChartPlay {
            self.res.info.replica_play.flipped_at(self.res.time)
        } else {
            self.res.rotate_chart && self.auto_flip.flipped
        };
        if flipped != self.res.auto_flip_y {
            self.judge.rotate_input_half_turn();
            self.res.auto_flip_y = flipped;
        }
    }

    fn seek_chart_preview(&mut self, chart_time: f64, tm: &mut TimeManager) -> Result<()> {
        let time = chart_time.clamp(0., (self.res.track_length - self.offset() as f64).max(0.));
        let song_time = (time + self.offset() as f64).min((self.res.track_length - 0.001).max(0.));
        let paused = tm.paused();
        self.music.seek_to(song_time.max(0.))?;
        tm.seek_to(song_time);
        self.state = if song_time < 0. { State::BeforeMusic } else { State::Playing };
        if paused || song_time < 0. {
            self.music.pause()?;
        } else {
            self.music.play()?;
        }
        self.reset_exercise_judgement_to(song_time);
        // Mid-Hold preview remains visibly held after scrubbing, without a real input.
        for line in &mut self.chart.lines {
            for note in &mut line.notes {
                if let crate::core::NoteKind::Hold { end_time, .. } = note.kind {
                    if note.time < time && time < end_time {
                        note.judge = crate::judge::JudgeStatus::Hold(true, time, time, false, f64::INFINITY, 2, false);
                    }
                }
            }
        }
        // Restored in-progress Holds must remain in the autoplay update range,
        // so their normal endpoint can finish particles/state after a seek.
        for (line, (indices, cursor)) in self.chart.lines.iter().zip(self.judge.notes.iter_mut()) {
            *cursor = indices
                .iter()
                .position(|id| !matches!(line.notes[*id as usize].judge, crate::judge::JudgeStatus::Judged))
                .unwrap_or(indices.len());
        }
        self.res.time = time;
        self.res.alpha = 1.;
        self.pause_rewind = None;
        self.sync_auto_flip();
        self.chart.update(&mut self.res);
        Ok(())
    }

    fn edit_chart_play(&mut self, ui: &mut Ui, tm: &mut TimeManager) -> Result<()> {
        use crate::ui::chart_play_editor::ChartPlayEditorAction as Action;
        let duration = (self.res.track_length - self.offset() as f64).max(0.);
        let conversion = self.res.info.replica_play.note_conversion;
        let action = self
            .chart_play_editor
            .render(ui, &mut self.res.info.replica_play, self.res.time, duration, tm.paused());
        if conversion != self.res.info.replica_play.note_conversion {
            self.note_conversion_backup
                .apply(&mut self.chart, self.res.info.replica_play.note_conversion);
            self.judge = Judge::new(&self.chart);
            if self.res.config.correct_sound {
                self.correct_sound_events = Self::build_correct_sound_events(&self.chart, &self.res);
                self.music = Self::new_music(&mut self.res, &self.mode, &self.correct_sound_events, (self.chart.offset + self.info_offset) as f64)?;
            }
            self.seek_chart_preview(self.res.time, tm)?;
        }
        self.sync_auto_flip();
        match action {
            Some(Action::Seek(time)) => self.seek_chart_preview(time, tm)?,
            Some(Action::TogglePause) => {
                if tm.paused() {
                    tm.resume();
                    if matches!(self.state, State::Playing) {
                        self.music.play()?;
                    }
                } else {
                    self.music.pause()?;
                    tm.pause();
                }
            }
            Some(Action::Save) => self.next_scene = Some(NextScene::PopWithResult(Box::new(self.res.info.replica_play.clone()))),
            Some(Action::Cancel) => self.next_scene = Some(NextScene::Pop),
            None => {}
        }
        Ok(())
    }

    fn tweak_offset(&mut self, ui: &mut Ui, ita: bool) {
        let labels = OffsetPanelLabels {
            adjust_offset: tl!("adjust-offset"),
            auto_offset: tl!("auto-offset-btn"),
            analysis_prompt: tl!("analysis-prompt"),
            analysis_computing: tl!("analysis-computing"),
            cancel: tl!("offset-cancel"),
            reset: tl!("offset-reset"),
            save: tl!("offset-save"),
        };
        match self.offset_analysis.render(ui, &self.chart, &mut self.info_offset, ita, &labels) {
            Some(OffsetPanelAction::Cancel) => self.next_scene = Some(NextScene::PopWithResult(Box::new(None::<f32>))),
            Some(OffsetPanelAction::Reset) => self.info_offset = 0.,
            Some(OffsetPanelAction::Save(offset)) => self.next_scene = Some(NextScene::PopWithResult(Box::new(Some(offset)))),
            None => {}
        }
    }
    pub fn get_avg_fps(&self) -> Option<f32> {
        if self.fps_frame_count > 0 && self.fps_total_time > 0.0 {
            Some(self.fps_frame_count as f32 / self.fps_total_time as f32)
        } else {
            None
        }
    }
}

impl Scene for GameScene {
    fn enter(&mut self, tm: &mut TimeManager, target: Option<RenderTarget>) -> Result<()> {
        Ui::clear_practice_speed_drag();
        #[cfg(target_arch = "wasm32")]
        on_game_start();
        #[cfg(target_env = "ohos")]
        miniquad::native::set_interceptor_state(true);
        self.music = Self::new_music(&mut self.res, &self.mode, &self.correct_sound_events, (self.chart.offset + self.info_offset) as f64)?;
        self.res.camera.render_target = target;
        tm.speed = self.res.config.speed as _;
        tm.adjust_time = self.res.config.adjust_time;
        reset!(self, self.res, tm);
        set_camera(&self.res.camera);
        self.first_in = true;
        Ok(())
    }

    fn pause(&mut self, tm: &mut TimeManager) -> Result<()> {
        Ui::clear_practice_speed_drag();
        self.auto_flip.interrupt();
        self.sync_auto_flip();
        self.capture_replay_frame(tm);
        self.finish_practice_replay();
        if let Some(recorder) = &mut self.replay {
            recorder.reset_clock();
        }
        self.judge.clear_touch_input_with_source(TouchDebugClearSource::AppLifecyclePause);
        if !tm.paused() {
            self.pause_rewind = None;
            self.music.pause()?;
            tm.pause();
        }
        #[cfg(target_env = "ohos")]
        miniquad::native::set_interceptor_state(false);
        Ok(())
    }

    fn resume(&mut self, tm: &mut TimeManager) -> Result<()> {
        if !matches!(self.state, State::Playing) {
            tm.resume();
        }
        Ok(())
    }

    fn update(&mut self, tm: &mut TimeManager) -> Result<()> {
        crate::replay::poll_saves();
        self.res.config.practice_judgement_multiplier = self.practice_judgement_multiplier();
        self.offset_analysis
            .update(&self.chart, &self.res, self.info_offset, tm.real_time() as f32);

        self.res.audio.recover_if_needed()?;
        if matches!(self.state, State::Playing) {
            tm.update(self.music.position());
        }
        if self.mode == GameMode::EditChartPlay && matches!(self.state, State::Playing) && tm.now() >= self.res.track_length {
            self.seek_chart_preview((self.res.track_length - self.offset() as f64).max(0.), tm)?;
            self.music.pause()?;
            tm.pause();
        }
        if self.auto_flip.rotating() {
            if let Some(boundary) = self.auto_flip.finish(tm.real_time()) {
                self.sync_auto_flip();
                self.resume_from_pause(tm, Some(boundary + self.offset() as f64))?;
            } else {
                return Ok(());
            }
        }
        if self.mode == GameMode::Exercise
            && crate::replay::practice_range_finished(tm.now(), self.exercise_range.end, self.res.track_length, tm.paused())
        {
            self.capture_replay_frame(tm);
            self.finish_practice_replay();
            self.finish_play_report(ReportEndReason::PracticeRangeCompleted, self.exercise_range.end, tm.real_time());
            let state = self.state.clone();
            reset!(self, self.res, tm);
            self.state = state;
            tm.seek_to(self.exercise_range.start);
            let chart_time = (self.exercise_range.start - self.offset() as f64).max(0.);
            self.judge.advance_to(&mut self.chart, chart_time);
            tm.pause();
            self.music.pause()?;
            #[cfg(target_env = "ohos")]
            miniquad::native::set_interceptor_state(false);
        }
        let offset = self.offset();
        let time = tm.now();
        let time = match self.state {
            State::Starting => {
                if time >= Self::BEFORE_TIME {
                    self.res.alpha = 1.;
                    self.state = State::BeforeMusic;
                    tm.reset();
                    tm.seek_to(if self.mode == GameMode::Exercise {
                        self.exercise_range.start
                    } else {
                        offset.min(0.) as f64
                    });
                    self.last_update_time = tm.real_time();
                    if self.first_in && self.mode == GameMode::Exercise {
                        tm.pause();
                        self.first_in = false;
                    }
                    tm.now()
                } else {
                    #[cfg(target_os = "windows")]
                    {
                        // wtf bro. why must particles exist on Windows?
                        let emitter_config = self.res.emitter.emitter.config.clone();
                        let emitter_square_config = self.res.emitter.emitter_square.config.clone();
                        self.res.emitter.emitter.config.size = 0.0;
                        self.res.emitter.emitter_square.config.size = 0.0;
                        self.res.emitter.emitter.emit(vec2(0.0, 0.0), 1);
                        self.res.emitter.emitter_square.emit(vec2(0.0, 0.0), 1);
                        self.res.emitter.emitter.config = emitter_config;
                        self.res.emitter.emitter_square.config = emitter_square_config;
                    }
                    self.res.alpha = (1. - (1. - time / Self::BEFORE_TIME).powi(3)) as f32;
                    if self.mode == GameMode::Exercise {
                        self.exercise_range.start
                    } else {
                        offset as f64
                    }
                }
            }
            State::BeforeMusic => {
                if time >= 0.0 {
                    self.music.seek_to(time)?;
                    if !tm.paused() {
                        self.music.play()?;
                    }
                    self.state = State::Playing;
                }
                time
            }
            State::Playing => {
                if time > self.res.track_length + WAIT_TIME {
                    self.state = State::Ending;
                    #[cfg(target_env = "ohos")]
                    miniquad::native::set_interceptor_state(false);
                }
                time
            }
            State::Ending => {
                let t = time - self.res.track_length - WAIT_TIME;
                if t >= AFTER_TIME + 0.3 {
                    self.capture_replay_frame(tm);
                    self.finish_practice_replay();
                    self.finish_play_report(ReportEndReason::Completed, self.res.track_length, tm.real_time());
                    if self.res.config.challenge_mode {
                        self.music.pause()?;
                        self.next_scene = Some(NextScene::PopWithResult(Box::new(ChallengeEvent::Completed(self.judge.result()))));
                        return Ok(());
                    }
                    let mut record_data = None;
                    // TODO strengthen the protection
                    #[cfg(closed)]
                    if let Some(upload_fn) = &self.upload_fn {
                        if !self.res.config.offline_mode
                            && !self.res.config.mods.intersects(Mods::UNRATED)
                            && !self.res.config.use_keyboard
                            && self.res.config.speed >= 1.0 - 1e-3
                            && !self
                                .res
                                .config
                                .online_replica_active(self.res.info.id.is_some(), self.res.info.replica_play.blocks_score_upload())
                            && !self.res.config.blocks_score_upload()
                            && !self.res.info.replica_play.blocks_score_upload()
                            && !self.judgement_range_debug_used
                        {
                            if let Some(player) = &self.player {
                                if let Some(chart) = &self.res.info.id {
                                    record_data = Some(encode_record(self, player.id, *chart));
                                }
                            }
                        }
                    }
                    let result = self.judge.result();
                    let record = if !self.res.config.saves_run_record()
                        || self.res.info.replica_play.note_conversion != crate::chart_play::NoteConversion::Original
                        || !matches!(self.mode, GameMode::Normal | GameMode::NoRetry)
                    {
                        None
                    } else {
                        Some(SimpleRecord {
                            score: result.score as _,
                            accuracy: result.accuracy as _,
                            full_combo: result.max_combo == result.num_of_notes,
                        })
                    };
                    self.next_scene = match self.mode {
                        GameMode::Normal | GameMode::NoRetry | GameMode::View => {
                            let historic_best = self.player.as_ref().map_or(0, |it| it.historic_best);
                            if let Some(new_rec) = &record {
                                if let Some(f) = &self.save_fn {
                                    f(new_rec.clone())?;
                                }
                                if let Some(best) = &mut self.best_record {
                                    best.update(new_rec);
                                } else {
                                    self.best_record = record.clone();
                                }
                                if let Some(best) = &self.best_record {
                                    if let Some(player) = &mut self.player {
                                        player.historic_best = player.historic_best.max(best.score as _);
                                    }
                                }
                            }
                            Some(NextScene::Overlay(Box::new(
                                EndingScene::new(
                                    self.res.background.clone(),
                                    self.res.illustration.clone(),
                                    self.res.player.clone(),
                                    self.res.icons.clone(),
                                    self.res.icon_retry.clone(),
                                    self.res.icon_proceed.clone(),
                                    self.res.mod_icons.clone(),
                                    self.res.info.clone(),
                                    self.judge.result(),
                                    &self.res.config,
                                    self.res.res_pack.ending.clone(),
                                    self.upload_fn.as_ref().map(Arc::clone),
                                    self.player.as_ref().map(|it| it.rks),
                                    historic_best,
                                    record_data,
                                    self.best_record.clone(),
                                    if self.res.config.show_avg_fps { self.get_avg_fps() } else { None },
                                )?
                                .with_replay({
                                    self.res.replay_capture = false;
                                    self.replay.take().and_then(|recorder| recorder.into_payload().ok())
                                }),
                            )))
                        }
                        GameMode::TweakOffset => Some(NextScene::PopWithResult(Box::new(None::<f32>))),
                        GameMode::Exercise | GameMode::EditChartPlay => None,
                    };
                }
                self.res.alpha = (1. - (t / AFTER_TIME).min(1.).powi(2)) as f32;
                self.res.track_length
            }
        };
        let song_time = time;
        let time = (song_time - offset as f64).max(0.);
        self.res.time = time;
        if self.res.rotate_chart && self.mode != GameMode::EditChartPlay {
            if tm.paused() && self.pause_rewind.is_none() {
                self.auto_flip.sync(&self.res.info.replica_play, time, true);
            } else if !tm.paused() && self.pause_rewind.is_none() && matches!(self.state, State::Playing) {
                if let Some(boundary) = self.auto_flip.begin_due(&self.res.info.replica_play, time, tm.real_time()) {
                    Self::pause_playback(&mut self.music, tm, &mut self.judge, TouchDebugClearSource::AutoFlipTransition)?;
                    self.play_report.observe(boundary + offset as f64, tm.real_time(), true);
                    tm.seek_to(boundary + offset as f64);
                    self.music.seek_to((boundary + offset as f64).max(0.))?;
                    self.res.time = boundary;
                    self.chart.update(&mut self.res);
                    return Ok(());
                }
            }
        }
        self.sync_auto_flip();
        if matches!(self.state, State::Playing) && !tm.paused() && self.pause_rewind.is_none() && self.mode != GameMode::View {
            self.begin_play_report_if_needed(song_time, tm.real_time());
        }
        self.play_report.observe(song_time, tm.real_time(), tm.paused());
        if !tm.paused() && self.pause_rewind.is_none() && self.mode != GameMode::View {
            self.gl.quad_gl.viewport(self.res.camera.viewport);
            self.judge.update(&mut self.res, &mut self.chart, &mut self.bad_notes);
            self.gl.quad_gl.viewport(None);
        }
        self.music.set_noise_touch(
            !tm.paused()
                && self.res.config.noise_area.enabled
                && !self.res.config.noise_area.music_unaffected
                && !self.judge.noise_state.blocked_ids.is_empty(),
            get_frame_time(),
        );
        self.capture_play_report_events(song_time, tm.real_time());
        self.timing_bar.animate(tm.real_time(), &self.res.config.timing_bar);
        if let Some(update) = &mut self.update_fn {
            update(self.res.time, &mut self.res, &mut self.judge);
        }
        let counts = self.judge.counts();
        self.res.judge_line_color = if counts[2] + counts[3] == 0 && self.res.config.ap_fc_indicator {
            if counts[1] == 0 {
                self.res.res_pack.info.color_perfect()
            } else {
                self.res.res_pack.info.color_good()
            }
        } else {
            WHITE
        };
        if !self.dead
            && matches!(self.state, State::Playing)
            && (self.res.config.mods.contains(Mods::INSTANT_DEATH_AP) && counts[1] + counts[2] + counts[3] > 0
                || self.res.config.mods.contains(Mods::INSTANT_DEATH_FC) && counts[2] + counts[3] > 0)
        {
            if !self.music.paused() {
                self.music.pause()?;
            }
            tm.pause();
            self.judge.clear_touch_input_with_source(TouchDebugClearSource::InstantDeath);
            self.dead = true;
            #[cfg(target_env = "ohos")]
            miniquad::native::set_interceptor_state(false);
            show_message(tl!("game-over")).error();
        }
        self.res.judge_line_color.a *= self.res.alpha;
        self.chart.update(&mut self.res);
        if tm.paused() && self.res.config.interactive && is_key_pressed(KeyCode::Space) {
            self.begin_armed_replay();
        }
        let res = &mut self.res;
        if res.config.interactive && is_key_pressed(KeyCode::Space) {
            if tm.paused() {
                if matches!(self.state, State::Playing) {
                    self.music.play()?;
                    tm.resume();
                }
            } else if matches!(self.state, State::Playing | State::BeforeMusic) {
                if !self.music.paused() {
                    self.music.pause()?;
                }
                tm.pause();
                self.judge.clear_touch_input_with_source(TouchDebugClearSource::KeyboardPause);
            }
        }
        if Self::interactive(res, &self.state) {
            if is_key_pressed(KeyCode::Left) && res.config.use_keyboard {
                res.time -= 1.;
                let dst = (self.music.position() - 1.).max(0.);
                self.music.seek_to(dst)?;
                tm.seek_to(dst);
                self.auto_flip
                    .sync(&res.info.replica_play, (dst - offset as f64).max(0.), res.rotate_chart);
            }
            if is_key_pressed(KeyCode::Right) && res.config.use_keyboard {
                res.time += 5.;
                let dst = (self.music.position() + 5.).min(res.track_length);
                self.music.seek_to(dst)?;
                tm.seek_to(dst);
                self.auto_flip
                    .sync(&res.info.replica_play, (dst - offset as f64).max(0.), res.rotate_chart);
            }
            if is_key_pressed(KeyCode::Q) {
                self.should_exit = true;
            }
        }
        for e in &mut self.effects {
            e.update(&self.res);
        }
        if let Some((id, text)) = take_input() {
            let offset = self.offset().min(0.);
            match id.as_str() {
                "exercise_speed" | "exercise_note_flow" => {
                    // Inputs belong to a paused practice run; ignore late results after leaving it.
                    if self.mode == GameMode::Exercise && tm.paused() {
                        let playback = id == "exercise_speed";
                        let range = if playback {
                            PLAYBACK_SPEED_MIN..PLAYBACK_SPEED_MAX
                        } else {
                            NOTE_FLOW_SPEED_MIN..NOTE_FLOW_SPEED_MAX
                        };
                        if let Some(value) = crate::practice_speed::parse(&text, &range) {
                            let changed = if playback {
                                (self.res.config.speed - value).abs() > f32::EPSILON
                            } else {
                                self.exercise_note_flow_locked || (self.exercise_note_flow_speed - value).abs() > f32::EPSILON
                            };
                            if changed {
                                self.finish_play_report(ReportEndReason::PracticeSettingsChanged, tm.now(), tm.real_time());
                                if playback {
                                    self.res.config.speed = value;
                                } else {
                                    self.exercise_note_flow_speed = value;
                                    self.exercise_note_flow_locked = false;
                                }
                                self.exercise_state_reset_pending = true;
                                self.sync_note_flow_speed();
                            }
                        } else {
                            show_message(tl!("ex-speed-invalid", "min" => range.start, "max" => range.end)).error();
                        }
                    }
                }
                "exercise_judgement" => {
                    if self.mode == GameMode::Exercise && tm.paused() {
                        if let Some(value) = crate::practice_view::parse(text.trim().trim_end_matches('%')).filter(|v| (10. ..=300.).contains(v)) {
                            if self.exercise_judgement_locked || self.exercise_judgement_percent != value {
                                self.finish_play_report(ReportEndReason::PracticeSettingsChanged, tm.now(), tm.real_time());
                                self.exercise_judgement_percent = value;
                                self.exercise_judgement_locked = false;
                                self.exercise_state_reset_pending = true;
                                self.sync_note_flow_speed();
                            }
                        } else {
                            show_message("请输入 10 到 300 之间的百分比").error();
                        }
                    }
                }
                "exercise_view_scale" | "exercise_view_x" | "exercise_view_y" => {
                    if self.mode == GameMode::Exercise && tm.paused() {
                        if let Some(value) = crate::practice_view::parse(&text) {
                            match id.as_str() {
                                "exercise_view_scale" => self.res.practice_view.scale_percent = value,
                                "exercise_view_x" => self.res.practice_view.center_x = value,
                                _ => self.res.practice_view.center_y = value,
                            }
                        }
                    }
                }
                "exercise_start" => {
                    if let Some(t) = parse_time(&text) {
                        if !(offset as f64..self.res.track_length.min(self.exercise_range.end - 3.).max(offset as f64)).contains(&t) {
                            show_message(tl!("ex-time-out-of-range")).error();
                        } else {
                            self.finish_play_report(ReportEndReason::PracticeSettingsChanged, tm.now(), tm.real_time());
                            self.exercise_range.start = t;
                            self.exercise_state_reset_pending = true;
                            show_message(tl!("ex-time-set")).ok();
                        }
                    } else {
                        show_message(tl!("ex-invalid-format")).error();
                    }
                }
                "exercise_end" => {
                    if let Some(t) = parse_time(&text) {
                        if !((self.exercise_range.start + 3.).max(offset as f64).min(self.res.track_length)..self.res.track_length).contains(&t) {
                            show_message(tl!("ex-time-out-of-range")).error();
                        } else {
                            self.finish_play_report(ReportEndReason::PracticeSettingsChanged, tm.now(), tm.real_time());
                            self.exercise_range.end = t;
                            self.exercise_state_reset_pending = true;
                            show_message(tl!("ex-time-set")).ok();
                        }
                    } else {
                        show_message(tl!("ex-invalid-format")).error();
                    }
                }
                _ => return_input(id, text),
            }
        }
        Ok(())
    }

    fn touch(&mut self, tm: &mut TimeManager, touch: &Touch) -> Result<bool> {
        if self.auto_flip.rotating() {
            return Ok(true);
        }
        if self.mode == GameMode::TweakOffset {
            self.offset_analysis.touch(touch, tm.real_time() as f32);
        }
        if self.mode == GameMode::Exercise && tm.paused() {
            let touch = Touch {
                position: touch.position * self.touch_scale(),
                ..touch.clone()
            };
            if self.exercise_btns.0.touch(&touch) {
                request_input("exercise_start", InputBox::new().default_text(fmt_time(self.exercise_range.start as f32)));
                return Ok(true);
            }
            if self.exercise_btns.1.touch(&touch) {
                request_input("exercise_end", InputBox::new().default_text(fmt_time(self.exercise_range.end as f32)));
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.judgement_range_debug_used |= self.res.config.judgement_range_debug.enabled;
        if self.res.config.show_avg_fps {
            let current_time = tm.real_time();
            if matches!(self.state, State::Playing) && !tm.paused() {
                let frame_delta = current_time - self.fps_last_frame_time;
                self.fps_total_time += frame_delta;
                self.fps_frame_count += 1;
            }
            self.fps_last_frame_time = current_time;
        }

        self.sync_note_flow_speed();
        self.res.replay_note_centers.clear();
        if !self.res.replay_view {
            self.res.replay_note_targets.clear();
            if let Some(recorder) = &self.replay {
                self.res.replay_note_targets.extend(recorder.pending.iter().map(|e| (e.line, e.note)));
            }
        }
        self.res.chart_post_view = self.res.has_noise_area
            && (self.mode == GameMode::Exercise || self.res.replay_view)
            && self.res.practice_view != Default::default()
            && !self
                .res
                .practice_view
                .needs_expanded_view(self.res.camera.viewport.map_or(ui.viewport.2, |vp| vp.2) as f32, self.res.aspect_ratio);
        let res = &mut self.res;
        let asp = ui.viewport.2 as f32 / ui.viewport.3 as f32;
        if res.update_size(ui.viewport) || self.mode == GameMode::View {
            set_camera(&res.camera);
        }

        let msaa = res.config.sample_count > 1;

        let chart_onto = res
            .chart_target
            .as_ref()
            .map(|it| if msaa { it.input() } else { it.output() })
            .or(res.camera.render_target);
        push_camera_state();
        set_camera(&Camera2D {
            zoom: vec2(1., -asp),
            viewport: if res.chart_target.is_some() { None } else { Some(ui.viewport) },
            render_target: chart_onto,
            ..Default::default()
        });
        clear_background(BLACK);
        draw_background(*res.background);
        pop_camera_state();

        let chart_target_vp = if res.chart_target.is_some() {
            let vp = res.camera.viewport.unwrap();
            Some((vp.0 - ui.viewport.0, vp.1 - ui.viewport.1, vp.2, vp.3))
        } else {
            res.camera.viewport
        };
        self.gl.quad_gl.render_pass(chart_onto.map(|it| it.render_pass));
        self.gl.quad_gl.viewport(chart_target_vp);

        let h = 1. / res.aspect_ratio;
        draw_rectangle(-1., -h, 2., h * 2., Color::new(0., 0., 0., res.alpha * res.info.background_dim));

        let transformed_view =
            !res.chart_post_view && (self.mode == GameMode::Exercise || res.replay_view) && res.practice_view != Default::default();
        if transformed_view {
            push_camera_state();
            let (cx, cy) = res.practice_view.center(res.camera.viewport.unwrap().2 as f32);
            set_camera(&Camera2D {
                target: vec2(cx, cy),
                zoom: res.camera.zoom * res.practice_view.scale(),
                viewport: chart_target_vp,
                render_target: chart_onto,
                ..Default::default()
            });
            self.gl.quad_gl.render_pass(chart_onto.map(|it| it.render_pass));
            self.gl.quad_gl.viewport(chart_target_vp);
        }

        let noise_projection = self.gl.quad_gl.get_projection_matrix();
        let judgement_ranges = res.config.judgement_range_debug.enabled.then(|| {
            self.replay_view_profile
                .unwrap_or_else(|| self.judge.judgement_range_profile(&res.config))
        });
        self.chart
            .render(ui, res, judgement_ranges.as_ref().map(|profile| (profile, self.judge.notes.as_slice())));

        self.gl.quad_gl.render_pass(
            res.chart_target
                .as_ref()
                .map(|it| it.output().render_pass)
                .or_else(|| res.camera.render_pass()),
        );

        self.bad_notes.retain(|dummy| dummy.render(res));
        let t = tm.real_time();
        let dt = (t - std::mem::replace(&mut self.last_update_time, t)) as f32;
        if res.config.particle {
            res.emitter.draw(if res.replay_view || self.auto_flip.rotating() { 0. } else { dt });
        }
        if transformed_view {
            pop_camera_state();
            self.gl.quad_gl.render_pass(
                res.chart_target
                    .as_ref()
                    .map(|it| it.output().render_pass)
                    .or_else(|| res.camera.render_pass()),
            );
            self.gl.quad_gl.viewport(chart_target_vp);
        }
        if let Some(renderer) = &mut self.noise_renderer {
            renderer.render(res, &self.chart.extra.block_areas, &self.judge.noise_state, ui.viewport, noise_projection)?;
        }
        if res.chart_post_view {
            // Compose at the original chart camera first. Transform the completed
            // scene once, using a different target so no pass reads its own output.
            if let Some(target) = &mut res.chart_target {
                let token = self.noise_renderer.as_mut().map(|renderer| {
                    renderer.begin_composite(
                        res.config.noise_area.low_performance,
                        res.config.noise_area.precise_edges,
                        ui.viewport.2 as u32,
                        ui.viewport.3 as u32,
                    )
                });
                self.gl.flush();
                target.swap();
                let source = target.old().texture;
                source.set_filter(FilterMode::Linear);
                let output = target.output();
                let width = res.camera.viewport.unwrap().2 as f32;
                let (cx, cy) = res.practice_view.composite_target(width, ui.viewport.2 as f32);
                push_camera_state();
                set_camera(&Camera2D {
                    zoom: vec2(1., -asp),
                    render_target: Some(output),
                    ..Default::default()
                });
                gl_use_default_material();
                self.gl.quad_gl.scissor(None);
                clear_background(BLACK);
                draw_background(*res.background);
                set_camera(&Camera2D {
                    target: vec2(cx, cy),
                    zoom: vec2(1., asp) * res.practice_view.scale(),
                    render_target: Some(output),
                    ..Default::default()
                });
                draw_texture_ex(
                    source,
                    -1.,
                    -ui.top,
                    WHITE,
                    DrawTextureParams {
                        dest_size: Some(vec2(2., ui.top * 2.)),
                        ..Default::default()
                    },
                );
                self.gl.flush();
                pop_camera_state();
                self.gl.quad_gl.render_pass(Some(output.render_pass));
                self.gl.quad_gl.viewport(chart_target_vp);
                if let Some(token) = token {
                    self.noise_renderer.as_mut().unwrap().end_composite(token);
                }
            }
        }
        let rotating_canvas = self.res.rotate_chart;
        if !rotating_canvas {
            self.ui(ui, tm)?;
            self.overlay_ui(ui, tm)?;
        }

        if self.mode == GameMode::TweakOffset {
            push_camera_state();
            self.gl.quad_gl.viewport(None);
            set_camera(&Camera2D {
                zoom: vec2(1., -screen_aspect()),
                render_target: self.res.chart_target.as_ref().map(|it| it.output()).or(self.res.camera.render_target),
                ..Default::default()
            });
            self.tweak_offset(ui, Self::interactive(&self.res, &self.state));
            pop_camera_state();
        }

        if !self.res.no_effect && !self.effects.is_empty() {
            push_camera_state();
            set_camera(&Camera2D {
                zoom: vec2(1., asp),
                ..Default::default()
            });
            for e in &self.effects {
                e.render(&mut self.res);
            }
            pop_camera_state();
        }
        if msaa || !self.res.no_effect || self.res.rotate_chart || self.res.has_noise_area {
            // render the texture onto screen
            if let Some(target) = &self.res.chart_target {
                self.gl.flush();
                push_camera_state();
                self.gl.quad_gl.viewport(None);
                set_camera(&Camera2D {
                    zoom: vec2(1., asp),
                    render_target: self.res.camera.render_target,
                    viewport: Some(ui.viewport),
                    ..Default::default()
                });
                if self.res.rotate_chart {
                    clear_background(BLACK);
                }
                draw_texture_ex(
                    target.output().texture,
                    -1.,
                    -ui.top,
                    WHITE,
                    DrawTextureParams {
                        dest_size: Some(vec2(2., ui.top * 2.)),
                        rotation: if self.mode == GameMode::EditChartPlay {
                            if self.res.auto_flip_y {
                                std::f32::consts::PI
                            } else {
                                0.
                            }
                        } else {
                            self.replay_view_angle.unwrap_or_else(|| self.auto_flip.angle(tm.real_time()))
                        },
                        ..Default::default()
                    },
                );
                pop_camera_state();
            }
        }
        if rotating_canvas && self.mode != GameMode::EditChartPlay {
            push_camera_state();
            set_camera(&self.res.camera);
            self.ui(ui, tm)?;
            self.overlay_ui(ui, tm)?;
            pop_camera_state();
        }
        if self.mode == GameMode::EditChartPlay {
            push_camera_state();
            // The controls and hit regions must share Ui's screen viewport. The
            // currently active viewport can still be the letterboxed chart.
            set_camera(&crate::ui::chart_play_editor::ChartPlayEditor::camera(ui, self.res.camera.render_target));
            ui.abs_scope(|ui| self.edit_chart_play(ui, tm))?;
            pop_camera_state();
        }
        if self.mode != GameMode::EditChartPlay && !tm.paused() && self.res.config.timing_bar.enabled {
            let saved_viewport = self.gl.quad_gl.get_viewport();
            push_camera_state();
            set_camera(&Camera2D {
                render_target: self.res.camera.render_target,
                ..ui.camera()
            });
            let profile = self.judge.judgement_range_profile(&self.res.config);
            ui.abs_scope(|ui| {
                crate::timing_bar::render(
                    ui,
                    &self.res.config.timing_bar,
                    &self.timing_bar,
                    &profile,
                    (self.res.config.judgement_mode == crate::config::JudgementMode::Custom)
                        .then(|| self.res.config.effective_custom_judgement())
                        .as_deref(),
                )
            });
            pop_camera_state();
            self.gl.quad_gl.viewport(saved_viewport);
        }
        if !self.res.replay_view {
            self.capture_replay_frame(tm);
            if tm.paused() && !self.auto_flip.rotating() {
                self.finish_practice_replay();
                if let Some(recorder) = &mut self.replay {
                    recorder.reset_clock();
                }
            }
        }
        // Draw after chart compositing/rotation so this control is anchored to
        // the actual display, independent of chart aspect and practice camera.
        self.render_pause_replay_control(tm, ui);
        Ok(())
    }

    fn next_scene(&mut self, tm: &mut TimeManager) -> NextScene {
        if self.should_exit {
            self.finish_practice_replay();
            self.finish_play_report(ReportEndReason::Exit, tm.now(), tm.real_time());
            if tm.paused() {
                tm.resume();
            }
            tm.speed = 1.0;
            tm.adjust_time = false;
            if self.res.config.challenge_mode {
                return NextScene::PopWithResult(Box::new(ChallengeEvent::Aborted));
            }
            match self.mode {
                // return result to update score and refresh
                GameMode::Normal => {
                    if let Some(rec) = &self.best_record {
                        NextScene::PopWithResult(Box::new(rec.clone()))
                    } else {
                        NextScene::Pop
                    }
                }
                // not sure if they need result. just keep it
                GameMode::Exercise | GameMode::EditChartPlay | GameMode::NoRetry | GameMode::View => NextScene::Pop,
                GameMode::TweakOffset => NextScene::PopWithResult(Box::new(None::<f32>)),
            }
        } else if let Some(next_scene) = self.next_scene.take() {
            if !matches!(next_scene, NextScene::None) && tm.paused() {
                tm.resume();
            }
            tm.speed = 1.0;
            tm.adjust_time = false;
            next_scene
        } else {
            NextScene::None
        }
    }
}

#[cfg(test)]
mod exercise_note_flow_tests {
    use super::*;

    #[test]
    fn pause_replay_control_is_top_right_on_different_displays() {
        for (w, h) in [(1920, 1080), (2400, 1080), (2048, 1536)] {
            let top = h as f32 / w as f32;
            let rect = GameScene::pause_replay_rect(top, true);
            assert!((rect.right() - 0.96).abs() < 1e-6);
            assert!((rect.y + top - 0.025).abs() < 1e-6);
            let raw = vec2((rect.center().x + 1.) * w as f32 / 2., (rect.center().y + top) * w as f32 / 2.);
            let touch = Judge::screen_touch_position(raw, (0, 0, w, h), h as f32);
            assert!(rect.contains(touch));
        }
    }

    #[test]
    fn inverse_lock_is_rounded_to_three_decimals() {
        assert_eq!(GameScene::locked_note_flow_speed(0.4), 2.5);
        assert_eq!(GameScene::locked_note_flow_speed(0.3), 3.333);
    }

    #[test]
    fn inverse_lock_covers_the_full_playback_speed_range() {
        assert_eq!(GameScene::locked_note_flow_speed(PLAYBACK_SPEED_MIN), NOTE_FLOW_SPEED_MAX);
        assert_eq!(GameScene::locked_note_flow_speed(PLAYBACK_SPEED_MAX), NOTE_FLOW_SPEED_MIN);
    }
}
