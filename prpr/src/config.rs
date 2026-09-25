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
    pub judgement_mode: JudgementMode,
    pub phigros_strict_judgement: bool,
    pub mods: Mods,
    pub mp_address: String,
    pub mp_enabled: bool,
    pub note_scale: f32,
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
            auto_export_play_report: true,
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
            judgement_mode: JudgementMode::Phira,
            phigros_strict_judgement: false,
            mods: Mods::default(),
            mp_address: "mp2.phira.cn:12345".to_owned(),
            mp_enabled: false,
            note_scale: 1.0,
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
            hold_head_effect: true,
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

    pub fn init(&mut self) {
        // Removed split-Flick and online-local switches deserialize as ignored legacy keys.
        // The settings-page playback override is retired; practice adjusts its own run config.
        self.speed = 1.;
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
