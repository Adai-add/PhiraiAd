//! In-memory empty chart loaded through the normal GameScene resource pipeline.
use crate::{
    config::{Config, Mods},
    ext::SafeTexture,
    fs::{fs_from_assets, PatchedFileSystem},
    info::{ChartFormat, ChartInfo},
    scene::{GameMode, GameScene},
};
use anyhow::Result;
use macroquad::prelude::Texture2D;
use std::collections::HashMap;

const EMPTY_CHART: &str = r#"{
 "formatVersion":3,"offset":0,"judgeLineList":[{
 "bpm":120,"notesAbove":[],"notesBelow":[],
 "speedEvents":[{"startTime":0,"endTime":100000,"value":1}],
 "judgeLineDisappearEvents":[{"startTime":0,"endTime":100000,"start":1,"end":1}],
 "judgeLineRotateEvents":[{"startTime":0,"endTime":100000,"start":0,"end":0}],
 "judgeLineMoveEvents":[{"startTime":0,"endTime":100000,"start":0.5,"end":0.5,"start2":0.5,"end2":0.5}]
 }]}
"#;

// A valid silent PCM stream gives the regular loader a finite timeline. It is
// never played: this preview only updates chart objects and calls Scene::render.
fn silent_wav() -> Vec<u8> {
    let sample_rate: u32 = 8000;
    let data_len: u32 = sample_rate * 60 * 2;
    let mut wav = Vec::with_capacity(data_len as usize + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(data_len as usize + 44, 0);
    wav
}

pub async fn load(mut config: Config) -> Result<GameScene> {
    config.interactive = false;
    config.mods = Mods::default();
    config.speed = 1.;
    config.offset = 0.;
    config.volume_music = 0.;
    config.volume_sfx = 0.;
    config.auto_export_play_report = false;
    config.touch_input_debug_report = false;
    config.judgement_range_debug.enabled = false;
    // The editor paints its editable timing HUD once, after the actual chart.
    config.timing_bar.enabled = false;
    let info = ChartInfo {
        name: "空谱面".into(),
        level: "IN Lv.0".into(),
        difficulty: 0.,
        format: Some(ChartFormat::Pgr),
        music: "silent.wav".into(),
        use_attach_ui_fix: Some(true),
        ..Default::default()
    };
    let patches = HashMap::from([
        ("chart.json".into(), EMPTY_CHART.as_bytes().to_vec()),
        ("silent.wav".into(), silent_wav()),
    ]);
    let fs = Box::new(PatchedFileSystem(fs_from_assets("timing-preview/")?, patches));
    let background: SafeTexture = Texture2D::from_rgba8(1, 1, &[35, 43, 58, 255]).into();
    GameScene::new(GameMode::View, info, config, fs, None, background.clone(), background, None, None, None, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_chart_is_real_empty_pgr_with_a_visible_stationary_line() {
        let mut chart = crate::parse::parse_phigros(EMPTY_CHART, Default::default()).unwrap();
        assert_eq!(chart.lines.len(), 1);
        assert!(chart.lines[0].notes.is_empty());
        let line = &mut chart.lines[0];
        line.object.set_time(30.);
        assert_eq!(line.object.alpha.now(), 1.);
        assert_eq!(line.object.rotation.now(), 0.);
        assert_eq!(line.object.translation.0.now(), 0.);
        assert_eq!(line.object.translation.1.now(), 0.);
    }
    #[test]
    fn preview_audio_decodes_as_sixty_seconds_of_silence() {
        let wav = silent_wav();
        assert_eq!(wav.len(), 960044);
        let clip = sasa::AudioClip::new(wav).unwrap();
        assert!((clip.length() - 60.).abs() < 0.001);
    }
}
