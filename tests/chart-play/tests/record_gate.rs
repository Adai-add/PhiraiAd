use phira_chart_play_tests::config::{Config, Mods};
use serde::{Deserialize, Serialize};
include!(concat!(env!("OUT_DIR"), "/record_gate.rs"));

struct Resource { config: Config }
struct ResultData { score: i32, accuracy: f32, max_combo: i32, num_of_notes: i32 }
struct Judge;
impl Judge {
    fn result(&self) -> ResultData {
        ResultData { score: 987654, accuracy: 0.985, max_combo: 100, num_of_notes: 100 }
    }
}
struct Scene { res: Resource, mode: GameMode, judge: Judge }

#[test]
fn actual_settlement_gate_saves_manual_runs_but_excludes_other_modes() {
    for (mode, expected) in [
        (GameMode::Normal, true), (GameMode::NoRetry, true),
        (GameMode::Exercise, false), (GameMode::View, false),
        (GameMode::TweakOffset, false), (GameMode::EditChartPlay, false),
    ] {
        let mut config = Config::default();
        config.shorten_holds = true;
        let mut scene = Scene { res: Resource { config }, mode, judge: Judge };
        assert_eq!(scene.record().is_some(), expected);
        scene.res.config.mods.insert(Mods::AUTOPLAY);
        assert!(scene.record().is_none());
    }
}

#[test]
fn generated_record_roundtrips_and_preserves_independent_bests() {
    let mut config = Config::default();
    config.shorten_holds = true;
    let scene = Scene { res: Resource { config }, mode: GameMode::Normal, judge: Judge };
    let record = scene.record().expect("local manual settlement must produce a record");
    let file = tempfile::NamedTempFile::new().unwrap();
    serde_json::to_writer(file.as_file(), &record).unwrap();
    let mut restored: SimpleRecord = serde_json::from_reader(std::fs::File::open(file.path()).unwrap()).unwrap();
    assert_eq!(restored.score, 987654);
    assert_eq!(restored.accuracy, 0.985);
    assert!(restored.full_combo);
    assert!(!restored.update(&SimpleRecord { score: 800000, accuracy: 0.9, full_combo: false }));
    assert!(restored.update(&SimpleRecord { score: 900000, accuracy: 0.99, full_combo: false }));
    assert_eq!(restored.score, 987654);
    assert_eq!(restored.accuracy, 0.99);
    assert!(restored.full_combo);
}
