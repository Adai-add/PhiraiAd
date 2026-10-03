//! Configuration module of the playing environment.\
//! e.g. player name, volume, speed, autoplay, etc.

use bitflags::bitflags;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

pub const PLAYBACK_SPEED_MIN: f32 = 0.05;
pub const PLAYBACK_SPEED_MAX: f32 = 10.0;
pub const PLAYBACK_SPEED_STEP: f32 = 0.05;
pub const NOTE_FLOW_SPEED_MIN: f32 = 0.1;
pub const NOTE_FLOW_SPEED_MAX: f32 = 20.0;
pub const NOTE_FLOW_SPEED_STEP: f32 = 0.05;
pub const JUDGEMENT_RANGE_HORIZON_MIN: f32 = 0.1;
pub const JUDGEMENT_RANGE_HORIZON_MAX: f32 = 30.0;
pub const JUDGEMENT_RANGE_HORIZON_STEP: f32 = 0.1;
pub const JUDGEMENT_RANGE_ALPHA_MIN: f32 = 0.0;
pub const JUDGEMENT_RANGE_ALPHA_MAX: f32 = 0.5;
pub const JUDGEMENT_RANGE_ALPHA_STEP: f32 = 0.01;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementMode {
    #[default]
    Phira,
    PhigrosReplica,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementRangeAnchor {
    #[default]
    JudgeLine,
    Note,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct TapJudgementRangeDebug {
    pub enabled: bool,
    pub perfect: bool,
    pub good: bool,
    pub bad: bool,
    pub miss: bool,
}

impl Default for TapJudgementRangeDebug {
    fn default() -> Self {
        Self {
            enabled: true,
            perfect: true,
            good: true,
            bad: true,
            miss: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct HoldJudgementRangeDebug {
    pub enabled: bool,
    pub head_perfect: bool,
    pub head_good: bool,
    pub head_miss: bool,
    pub tail: bool,
}

impl Default for HoldJudgementRangeDebug {
    fn default() -> Self {
        Self {
            enabled: true,
            head_perfect: true,
            head_good: true,
            head_miss: true,
            tail: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct JudgementRangeDebug {
    pub enabled: bool,
    pub anchor: JudgementRangeAnchor,
    pub horizon: f32,
    pub fill_alpha: f32,
    pub tap: TapJudgementRangeDebug,
    pub hold: HoldJudgementRangeDebug,
    pub drag: bool,
    pub flick: bool,
}

impl Default for JudgementRangeDebug {
    fn default() -> Self {
        Self {
            enabled: false,
            anchor: JudgementRangeAnchor::JudgeLine,
            horizon: 2.0,
            fill_alpha: 0.12,
            tap: TapJudgementRangeDebug::default(),
            hold: HoldJudgementRangeDebug::default(),
            drag: true,
            flick: true,
        }
    }
}

/// Cosmetic timing HUD; positions are fractions of the screen half-width/height.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TimingBarConfig {
    pub enabled: bool,
    pub record_counts: bool,
    pub curved: bool,
    pub record_seconds: f32,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}
impl Default for TimingBarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            record_counts: true,
            curved: false,
            record_seconds: 3.,
            x: 0.,
            y: -0.65,
            size: 100.,
        }
    }
}
impl TimingBarConfig {
    pub fn retention_seconds(&self) -> f64 {
        if self.record_seconds.is_finite() {
            self.record_seconds.clamp(0.5, 10.) as f64
        } else {
            3.
        }
    }
}

pub static TIPS: Lazy<Vec<String>> = Lazy::new(|| include_str!("tips.txt").split('\n').map(str::to_owned).collect());

bitflags! {
    #[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Debug)]
    #[serde(transparent)]
    pub struct Mods: i32 {
        const AUTOPLAY = 0x0001;
        const FLIP_X = 0x0002;
        const FADE_OUT = 0x0004;
        const FADE_IN = 0x0008;
        const NIGHTCORE = 0x0010;
        const RAINBOW = 0x0020;
        const NO_SHADER = 0x0040;
        const INSTANT_DEATH_AP = 0x0080;
        const INSTANT_DEATH_FC = 0x0100;

        const UNRATED = Self::AUTOPLAY.bits() | Self::NO_SHADER.bits();
    }
}

impl Mods {
    pub fn toggle_mod(&mut self, flag: Mods) {
        if self.contains(flag) {
            self.remove(flag);
        } else {
            for &conflict in Mods::conflicts(flag) {
                self.remove(conflict);
            }
            self.insert(flag);
        }
    }
    fn conflicts(flag: Mods) -> &'static [Mods] {
        match flag {
            Mods::FADE_IN => &[Mods::FADE_OUT],
            Mods::FADE_OUT => &[Mods::FADE_IN],
            Mods::INSTANT_DEATH_AP => &[Mods::INSTANT_DEATH_FC],
            Mods::INSTANT_DEATH_FC => &[Mods::INSTANT_DEATH_AP],
            _ => &[],
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(rename = "adjust_time_new")]
    pub adjust_time: bool,
    pub aggressive: bool,
    pub ap_fc_indicator: bool,
    pub auto_export_play_report: bool,
    /// Include high-volume raw touch diagnostics in the exported play report.
    /// Disabled by default because it records one trajectory sample per active
    /// finger per frame and can make reports substantially larger.
    pub touch_input_debug_report: bool,
    pub aspect_ratio: Option<f32>,
    pub audio_buffer_size: Option<u32>,
    pub chart_debug: bool,
    pub disable_effect: bool,
    pub double_click_to_pause: bool,
    pub double_hint: bool,
    pub fullscreen_mode: bool,
    pub fxaa: bool,
    pub interactive: bool,
    pub judgement_range_debug: JudgementRangeDebug,
    pub timing_bar: TimingBarConfig,
    /// Phigros block-area rendering and audio switches.
    pub noise_area: crate::noise_area::NoiseAreaConfig,
    pub judgement_mode: JudgementMode,
    pub phigros_strict_judgement: bool,
    /// Runtime-only course flag; never modifies saved normal-play preferences.
    #[serde(skip)]
    pub challenge_mode: bool,
    pub mods: Mods,
    pub mp_address: String,
    pub mp_enabled: bool,
    pub note_scale: f32,
    /// Global visual flow multiplier, edited from any chart Mods panel.
    pub note_flow_speed: f32,
    pub offline_mode: bool,
    pub offset: f32,
    pub particle: bool,
    pub player_name: String,
    pub player_rks: f32,
    pub preferred_sample_rate: Option<u32>,
    pub res_pack_path: Option<String>,
    pub sample_count: u32,
    /// Global visual Hold assist; enabled runs cannot upload scores.
    pub shorten_holds: bool,
    /// Predict the local library in a low-duty background worker.
    pub ai_auto_predict: bool,
    /// Global gate; per-chart intervals are retained when disabled.
    pub auto_flip_enabled: bool,
    /// Cosmetic burst when a Hold head is successfully judged.
    pub hold_head_effect: bool,
    pub show_acc: bool,
    pub show_avg_fps: bool,
    pub speed: f32,
    /// Practice-only tempo changes that retain the original music pitch.
    pub practice_preserve_pitch: bool,
    pub touch_debug: bool,
    pub use_keyboard: bool,
    pub volume_bgm: f32,
    pub volume_music: f32,
    pub volume_sfx: f32,

    // for compatibility
    autoplay: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            adjust_time: false,
            aggressive: true,
            ap_fc_indicator: true,
            auto_export_play_report: false,
            touch_input_debug_report: false,
            aspect_ratio: None,
            audio_buffer_size: None,
            chart_debug: false,
            disable_effect: false,
            double_click_to_pause: true,
            double_hint: true,
            fxaa: false,
            interactive: true,
            judgement_range_debug: JudgementRangeDebug::default(),
            timing_bar: TimingBarConfig::default(),
            noise_area: crate::noise_area::NoiseAreaConfig::default(),
            judgement_mode: JudgementMode::Phira,
            phigros_strict_judgement: false,
            challenge_mode: false,
            mods: Mods::default(),
            mp_address: "mp2.phira.cn:12345".to_owned(),
            mp_enabled: false,
            note_scale: 1.0,
            note_flow_speed: 1.,
            offline_mode: false,
            fullscreen_mode: false,
            offset: 0.,
            particle: true,
            player_name: "Mivik".to_string(),
            player_rks: 15.,
            preferred_sample_rate: None,
            res_pack_path: None,
            sample_count: 1,
            shorten_holds: false,
            ai_auto_predict: false,
            auto_flip_enabled: true,
            hold_head_effect: false,
            show_acc: false,
            show_avg_fps: false,
            speed: 1.,
            practice_preserve_pitch: false,
            touch_debug: false,
            use_keyboard: false,
            volume_music: 1.,
            volume_sfx: 1.,
            volume_bgm: 1.,

            autoplay: None,
        }
    }
}

impl Config {
    pub fn enable_challenge(&mut self) {
        self.challenge_mode = true;
        self.judgement_mode = JudgementMode::PhigrosReplica;
        self.phigros_strict_judgement = true;
        self.autoplay = None;
        self.mods.remove(Mods::AUTOPLAY);
    }
    pub fn apply_challenge_chart(&self, settings: &mut crate::chart_play::ChartPlaySettings) {
        if self.challenge_mode {
            settings.note_conversion = crate::chart_play::NoteConversion::Original;
        }
    }

    #[inline]
    pub fn flick_judgement_mode(&self) -> JudgementMode {
        self.judgement_mode
    }

    /// Whether gameplay uses any non-default judgement engine. Such results
    /// are intentionally excluded from online score upload.
    #[inline]
    pub fn has_custom_judgement(&self) -> bool {
        self.judgement_mode != JudgementMode::Phira
    }

    /// Whether a setting that changes or exposes judgement behaviour makes an
    /// online result ineligible for upload.
    #[inline]
    pub fn blocks_score_upload(&self) -> bool {
        self.has_custom_judgement() || self.judgement_range_debug.enabled || self.touch_input_debug_report || self.shorten_holds
    }

    /// All online charts use the replica's isolated local record store, regardless of settings.
    pub fn online_replica_active(&self, online: bool, _chart_assist: bool) -> bool {
        online
    }

    /// Manual runs may save personal records for both local and online charts.
    /// Upload eligibility is separate; callers exclude practice and viewing modes.
    pub fn saves_run_record(&self) -> bool {
        !self.autoplay()
    }

    /// Keep malformed/legacy saved data away from the renderer.
    pub fn global_note_flow_speed(&self) -> f32 {
        if self.note_flow_speed.is_finite() {
            self.note_flow_speed.clamp(0.1, 5.)
        } else {
            1.
        }
    }

    /// Practice retains its independent multiplier (including the inverse-speed lock).
    pub fn effective_note_flow_speed(&self, practice_multiplier: f32) -> f32 {
        self.global_note_flow_speed() * practice_multiplier
    }

    pub fn init(&mut self) {
        // Removed split-Flick and online-local switches deserialize as ignored legacy keys.
        self.speed = if self.speed.is_finite() {
            self.speed.clamp(0.5, 2.)
        } else {
            1.
        };
        self.note_flow_speed = self.global_note_flow_speed();
        self.judgement_range_debug.horizon = self
            .judgement_range_debug
            .horizon
            .clamp(JUDGEMENT_RANGE_HORIZON_MIN, JUDGEMENT_RANGE_HORIZON_MAX);
        self.judgement_range_debug.fill_alpha = self
            .judgement_range_debug
            .fill_alpha
            .clamp(JUDGEMENT_RANGE_ALPHA_MIN, JUDGEMENT_RANGE_ALPHA_MAX);
        if let Some(flag) = self.autoplay {
            self.mods.set(Mods::AUTOPLAY, flag);
        }
        #[cfg(target_env = "ohos")]
        {
            // Due to the fucking poor performance of the Maloon GPU, the sample count must be set to 1.
            self.sample_count = 1;
        }
    }

    #[inline]
    pub fn has_mod(&self, m: Mods) -> bool {
        self.mods.contains(m)
    }

    #[inline]
    pub fn autoplay(&self) -> bool {
        self.has_mod(Mods::AUTOPLAY)
    }

    #[inline]
    pub fn flip_x(&self) -> bool {
        self.has_mod(Mods::FLIP_X)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_global_mode_is_inherited_by_flick() {
        let mut config = Config {
            judgement_mode: JudgementMode::PhigrosReplica,
            ..Default::default()
        };
        config.init();
        assert_eq!(config.flick_judgement_mode(), JudgementMode::PhigrosReplica);
    }

    #[test]
    fn strict_switch_alone_does_not_change_phira_or_upload_eligibility() {
        let config = Config {
            phigros_strict_judgement: true,
            ..Default::default()
        };
        assert!(!config.has_custom_judgement());
    }

    #[test]
    fn one_selector_controls_all_note_types() {
        let mut config = Config::default();
        for mode in [JudgementMode::PhigrosReplica, JudgementMode::Phira] {
            config.judgement_mode = mode;
            assert_eq!(config.flick_judgement_mode(), mode);
            assert_eq!(config.has_custom_judgement(), mode != JudgementMode::Phira);
        }
    }

    #[test]
    fn judgement_range_overlay_blocks_upload_without_changing_engine_mode() {
        let mut config = Config::default();
        assert!(!config.has_custom_judgement());
        assert!(!config.blocks_score_upload());
        config.judgement_range_debug.enabled = true;
        assert!(!config.has_custom_judgement());
        assert!(config.blocks_score_upload());
    }

    #[test]
    fn touch_input_debug_report_blocks_upload() {
        let mut config = Config::default();
        assert!(!config.blocks_score_upload());
        config.touch_input_debug_report = true;
        assert!(config.blocks_score_upload());
    }

    #[test]
    fn judgement_range_values_are_sanitized_when_loading_old_or_edited_data() {
        let mut config = Config::default();
        config.judgement_range_debug.horizon = 999.;
        config.judgement_range_debug.fill_alpha = -2.;
        config.init();
        assert_eq!(config.judgement_range_debug.horizon, JUDGEMENT_RANGE_HORIZON_MAX);
        assert_eq!(config.judgement_range_debug.fill_alpha, JUDGEMENT_RANGE_ALPHA_MIN);
    }
}

#[cfg(test)]
mod challenge_tests {
    use super::*;
    use crate::chart_play::{AutoFlipInterval, ChartPlaySettings, NoteConversion};
    #[test]
    fn mandatory_overrides_are_isolated_and_keep_other_preferences() {
        let original = Config {
            mods: Mods::AUTOPLAY | Mods::FLIP_X | Mods::NO_SHADER,
            shorten_holds: true,
            auto_flip_enabled: true,
            speed: 0.75,
            volume_music: 0.3,
            ..Default::default()
        };
        let mut course = original.clone();
        course.enable_challenge();
        assert!(original.autoplay());
        assert_eq!(original.judgement_mode, JudgementMode::Phira);
        assert!(!course.autoplay());
        assert!(course.phigros_strict_judgement);
        assert_eq!(course.judgement_mode, JudgementMode::PhigrosReplica);
        assert_eq!(course.flick_judgement_mode(), JudgementMode::PhigrosReplica);
        assert!(course.mods.contains(Mods::FLIP_X | Mods::NO_SHADER));
        assert_eq!(course.speed, original.speed);
        assert_eq!(course.volume_music, original.volume_music);
        assert!(course.shorten_holds && course.auto_flip_enabled);
        assert!(serde_json::to_value(&course).unwrap().get("challengeMode").is_none());
        for mode in [NoteConversion::Tap, NoteConversion::Drag, NoteConversion::Flick] {
            let mut settings = ChartPlaySettings {
                note_conversion: mode,
                auto_flip_intervals: vec![AutoFlipInterval { start: 1., end: 2. }],
            };
            original.apply_challenge_chart(&mut settings);
            assert_eq!(settings.note_conversion, mode);
            course.apply_challenge_chart(&mut settings);
            assert_eq!(settings.note_conversion, NoteConversion::Original);
            assert_eq!(settings.auto_flip_intervals.len(), 1);
        }
    }
    #[test]
    fn legacy_autoplay_cannot_restore_itself_inside_course() {
        let mut config: Config = serde_json::from_value(serde_json::json!({"autoplay":true})).unwrap();
        config.init();
        assert!(config.autoplay());
        config.enable_challenge();
        config.init();
        assert!(!config.autoplay());
    }
}

#[cfg(test)]
mod speed_tests {
    use super::Config;

    #[test]
    fn playback_and_global_flow_preferences_persist_across_loading() {
        let mut config: Config = serde_json::from_str(r#"{"speed":0.75,"noteFlowSpeed":1.5}"#).unwrap();
        config.init();
        assert_eq!(config.speed, 0.75);
        assert_eq!(config.note_flow_speed, 1.5);
        let mut restored: Config = serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        restored.init();
        assert_eq!(restored.speed, 0.75);
        assert_eq!(restored.note_flow_speed, 1.5);
        assert_eq!(serde_json::from_str::<Config>("{}").unwrap().note_flow_speed, 1.);
    }

    #[test]
    fn flow_multiplies_practice_without_changing_playback_or_clipping_the_product() {
        let config = Config { speed: 0.5, note_flow_speed: 5., ..Default::default() };
        assert_eq!(config.effective_note_flow_speed(1.), 5.);
        assert_eq!(config.effective_note_flow_speed(2.), 10.);
        assert_eq!(config.effective_note_flow_speed(20.), 100.);
        assert_eq!(config.speed, 0.5);
        let config = Config { note_flow_speed: 1.5, ..Default::default() };
        assert!((config.effective_note_flow_speed(0.8) - 1.2).abs() < 1e-6);
    }

    #[test]
    fn invalid_speed_preferences_are_sanitized() {
        for (playback, flow, expected_playback, expected_flow) in [
            (f32::NAN, f32::INFINITY, 1., 1.),
            (0., -1., 0.5, 0.1),
            (100., 100., 2., 5.),
        ] {
            let mut config = Config { speed: playback, note_flow_speed: flow, ..Default::default() };
            config.init();
            assert_eq!(config.speed, expected_playback);
            assert_eq!(config.note_flow_speed, expected_flow);
        }
    }
}
