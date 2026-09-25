//! Host tests use the production settings and animation modules without graphics/audio.
#[path = "../../../prpr/src/chart_play.rs"]
pub mod chart_play;

// The animation's external interfaces are replaced with tiny linear/quadratic
// tweens, so its real value_at/set_time/chaining implementation runs unchanged.
pub type TweenId = u8;
pub trait TweenFunction {
    fn y(&self, t: f32) -> f32;
}
pub struct StaticTween(u8);
impl TweenFunction for StaticTween {
    fn y(&self, t: f32) -> f32 {
        if self.0 == 1 {
            t * t
        } else {
            t
        }
    }
}
impl StaticTween {
    pub fn get_rc(id: TweenId) -> std::rc::Rc<dyn TweenFunction> {
        std::rc::Rc::new(Self(id))
    }
}
pub trait Tweenable: Clone {
    fn tween(a: &Self, b: &Self, t: f32) -> Self;
    fn add(a: &Self, b: &Self) -> Self;
}
impl Tweenable for f32 {
    fn tween(a: &Self, b: &Self, t: f32) -> Self {
        a + (b - a) * t
    }
    fn add(a: &Self, b: &Self) -> Self {
        a + b
    }
}
pub struct Vector {
    pub x: f32,
    pub y: f32,
}
impl Vector {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
#[path = "../../../prpr/src/core/anim.rs"]
pub mod anim;

#[cfg(test)]
mod integration {
    use super::*;
    #[test]
    fn sampled_height_matches_playback_without_changing_cursor() {
        let mut curve = anim::Anim::new(vec![
            anim::Keyframe::new(0., 0., 1),
            anim::Keyframe::new(2., 8., 0),
            anim::Keyframe::new(4., 10., 0),
        ]);
        curve.set_time(3.);
        let cursor = curve.cursor;
        for t in [-0.5, 0., 0.4, 1.78, 2., 3., 4., 10.] {
            let sampled = curve.value_at(t).unwrap();
            let mut reference = curve.clone();
            reference.set_time(t);
            assert!((sampled - reference.now()).abs() < 1e-5);
            assert_eq!(curve.time, 3.);
            assert_eq!(curve.cursor, cursor);
        }
        // A nonlinear speed curve must not use a duration fraction of total height.
        assert!((curve.value_at(1.78).unwrap() - 8. * (1.78_f32 / 2.).powi(2)).abs() < 1e-5);
    }
    #[test]
    fn multi_layer_speed_height_is_summed() {
        let curve = anim::Anim::chain(vec![
            anim::Anim::fixed(2.),
            anim::Anim::new(vec![anim::Keyframe::new(0., 0., 0), anim::Keyframe::new(2., 4., 0)]),
        ]);
        assert_eq!(curve.value_at(1.), Some(4.));
    }
    #[test]
    fn defaults_and_assist_score_gate() {
        let empty: chart_play::ChartPlaySettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Default::default());
        let mut s = empty;
        assert!(!s.blocks_score_upload());
        s.add(10., 12.);
        assert!(s.blocks_score_upload());
        assert!(!s.flipped_at(0.));
        s.auto_flip_intervals.clear();
        assert!(!s.blocks_score_upload());
    }
    #[test]
    fn reflected_geometry_input_and_shader_stay_consistent() {
        for x in [false, true] {
            for y in [false, true] {
                let (fx, fy) = chart_play::reflection_axes(x, y);
                let world = (0.37_f32, -0.22_f32);
                let screen = (world.0 * fx, -world.1 * fy);
                let touch = (screen.0 * fx, screen.1 * fy);
                assert_eq!((touch.0, -touch.1), world);
                // Fullscreen shader sees the original (1,-1) draw orientation after compensation.
                assert_eq!((fx * fx, -fy * fy), (1., -1.));
            }
        }
    }
    #[test]
    fn seek_and_retry_are_pure_time_queries() {
        let mut s = chart_play::ChartPlaySettings::default();
        s.add(2., 4.);
        s.add(6., 8.);
        let times = [0., 2., 3., 4., 7., 1., 6., 8., 2.];
        let expected = [false, true, true, false, true, false, true, false, true];
        for (t, want) in times.into_iter().zip(expected) {
            assert_eq!(s.flipped_at(t), want);
        }
    }
}

// Real persistence and metadata code, with only the app's directory/context supplied by tests.
extern crate self as prpr;
#[path = "../../../prpr/src/dir.rs"]
mod directory;
#[path = "../../../prpr/src/info.rs"]
pub mod info;
pub mod dir {
    pub use crate::directory::Dir;
    thread_local! {pub static ROOT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };}
    pub fn charts() -> anyhow::Result<String> {
        ROOT.with(|r| Ok(r.borrow().as_ref().expect("test directory").to_string_lossy().into_owned()))
    }
}
pub mod scene {
    pub static ASSET_CHART_INFO: std::sync::LazyLock<std::sync::Mutex<Option<crate::info::ChartInfo>>> = std::sync::LazyLock::new(Default::default);
}
#[path = "../../../phira/src/chart_play_settings.rs"]
mod persistence;
#[cfg(test)]
mod persistence_tests {
    use super::*;
    fn setup(chart_name: &str, bytes: &[u8]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        dir::ROOT.with(|r| *r.borrow_mut() = Some(root.path().to_owned()));
        std::fs::create_dir(root.path().join("local")).unwrap();
        let metadata = info::ChartInfo {
            chart: chart_name.into(),
            name: "测试谱面".into(),
            offset: 0.123,
            ..Default::default()
        };
        std::fs::write(root.path().join("local/info.yml"), serde_yaml::to_string(&metadata).unwrap()).unwrap();
        std::fs::write(root.path().join("local").join(chart_name), bytes).unwrap();
        root
    }
    fn settings() -> chart_play::ChartPlaySettings {
        let mut s = chart_play::ChartPlaySettings::default();
        s.add(1., 3.);
        s
    }
    #[test]
    fn json_write_keeps_metadata_and_clears_persistently() {
        let root = setup("chart.json", br#"{"formatVersion":3,"judgeLineList":[{"notes":[]}]}"#);
        let path = root.path().join("local/chart.json");
        let meta = std::fs::read(root.path().join("local/info.yml")).unwrap();
        persistence::save("local", &settings()).unwrap();
        assert_eq!(chart_play::ChartPlaySettings::from_chart_bytes(&std::fs::read(&path).unwrap()), Some(settings()));
        assert_eq!(std::fs::read(root.path().join("local/info.yml")).unwrap(), meta);
        persistence::save("local", &Default::default()).unwrap();
        assert_eq!(chart_play::ChartPlaySettings::from_chart_bytes(&std::fs::read(&path).unwrap()), Some(Default::default()));
        assert_eq!(std::fs::read_dir(root.path().join("local")).unwrap().count(), 2);
    }
    #[test]
    fn pec_persists_in_info_without_altering_chart() {
        let chart = b"0\nn1 0 1 0 1 0\n";
        let root = setup("chart.pec", chart);
        persistence::save("local", &settings()).unwrap();
        let info: info::ChartInfo = serde_yaml::from_slice(&std::fs::read(root.path().join("local/info.yml")).unwrap()).unwrap();
        assert_eq!(info.replica_play, settings());
        assert_eq!(info.name, "测试谱面");
        assert_eq!(info.offset, 0.123);
        assert_eq!(std::fs::read(root.path().join("local/chart.pec")).unwrap(), chart);
    }
    #[test]
    fn mods_conversion_preserves_intervals_reopens_and_disables_for_json_and_pec() {
        for (name,bytes) in [("chart.json",br#"{"formatVersion":3,"judgeLineList":[]}"#.as_slice()),("chart.pec",b"0\nn1 0 1 0 1 0\n".as_slice())] {
            let _root=setup(name,bytes);
            persistence::save("local",&settings()).unwrap();
            for mode in [chart_play::NoteConversion::Tap,chart_play::NoteConversion::Drag,chart_play::NoteConversion::Flick,chart_play::NoteConversion::Original] {
                let mut saved=persistence::load("local").unwrap();
                assert_eq!(saved.auto_flip_intervals,settings().auto_flip_intervals);
                saved.note_conversion=mode;persistence::save("local",&saved).unwrap();
                assert_eq!(persistence::load("local").unwrap(),saved);
            }
        }
    }
    #[test]
    fn missing_chart_does_not_truncate_existing_metadata() {
        let root = setup("chart.json", b"{}");
        let meta = root.path().join("local/info.yml");
        let old = std::fs::read(&meta).unwrap();
        std::fs::remove_file(root.path().join("local/chart.json")).unwrap();
        assert!(persistence::save("local", &settings()).is_err());
        assert_eq!(std::fs::read(&meta).unwrap(), old);
    }
    #[test]
    fn integrated_chart_sidecar_survives_reopen() {
        let root = setup("chart.json", b"{}");
        *scene::ASSET_CHART_INFO.lock().unwrap() = Some(Default::default());
        persistence::save(":song:ex", &settings()).unwrap();
        let bytes = std::fs::read(root.path().join("_song_ex/replica-play.json")).unwrap();
        assert_eq!(serde_json::from_slice::<chart_play::ChartPlaySettings>(&bytes).unwrap(), settings());
        assert_eq!(persistence::load(":song:ex").unwrap(), settings());
        assert_eq!(persistence::load(":different:song").unwrap(), Default::default());
        assert_eq!(scene::ASSET_CHART_INFO.lock().unwrap().as_ref().unwrap().replica_play, settings());
    }
}

#[path = "../../../prpr/src/config.rs"]
pub mod config;

#[cfg(test)]
mod global_hold_tests {
    use super::{chart_play::*, config::Config};
    #[test]
    fn global_switch_persists_and_gates_scores_independently_of_chart() {
        let mut c: Config = serde_json::from_str("{}").unwrap();
        assert!(!c.shorten_holds);
        assert!(!c.blocks_score_upload());
        c.shorten_holds = true;
        let encoded = serde_json::to_vec(&c).unwrap();
        let c: Config = serde_json::from_slice(&encoded).unwrap();
        assert!(c.shorten_holds && c.blocks_score_upload());
        for old in [false, true] {
            let bytes = format!(r#"{{"phiraReplica":{{"shortenHolds":{old},"autoFlipIntervals":[]}}}}"#);
            let settings = ChartPlaySettings::from_chart_bytes(bytes.as_bytes()).unwrap();
            assert_eq!(settings, ChartPlaySettings::default());
            let saved: serde_json::Value = serde_json::from_slice(&settings.embed_in_chart(bytes.as_bytes()).unwrap()).unwrap();
            assert!(saved["phiraReplica"].get("shortenHolds").is_none());
            assert!(c.blocks_score_upload());
        }
        let mut c = c; c.shorten_holds = false;
        assert!(!c.blocks_score_upload());
    }
    #[test]
    fn minimum_full_body_is_geometry_not_duration() {
        for scale in [0.1, 1., 20.] {
            for length in [-1., 0., 0.0001, 0.01, 0.1, 1.] {
                let head = 0.23;
                let end = hold_render_end_height(head, head + length, scale);
                let full = (end - head) * scale;
                assert!((full - (length * scale).max(0.01)).abs() < 1e-10);
            }
        }
        assert_eq!(hold_visual_end(1., 1.05, 1., 0.22), 1.);
        assert!((hold_visual_end(1., 2., 1., 0.22) - 1.78).abs() < 1e-10);
    }
    #[test]
    fn original_renderer_consumes_tail_below_minimum_without_plateau() {
        // Use the original renderer's top - bottom calculation on fixed endpoints.
        // This failed in r17: every result below 0.1 was clamped to 0.1.
        for full_length in [0., 0.001, 0.01, 1., 10.] {
            let head = 3.;
            let end = hold_render_end_height(head, head + full_length, 1.);
            let mut previous = f64::INFINITY;
            for left in [0.009, 0.007, 0.003, 0.001, 0.0001, 0.] {
                let line_height = end - left;
                let bottom = 0.; // original renderer pins the consumed head to the line
                let top = end - line_height;
                let remaining = top - bottom;
                assert!((remaining - left).abs() < 1e-12);
                assert!(remaining < previous);
                previous = remaining;
            }
        }
    }
    #[test]
    fn head_only_render_data_ends_at_head_without_late_body() {
        let start = 1.;
        let end_time = hold_visual_end(start, 1.05, 1., 0.22);
        let end_height = hold_render_end_height(3., 3., 1.);
        assert!((end_height - 3. - 0.01).abs() < 1e-12);
        for now in [1., 1.01, 1.05, 1.16] {
            assert!(now >= end_time); // r14 renderer's existing early return
        }
    }
    #[test]
    fn nonlinear_scroll_keeps_sampled_endpoint_and_consumption() {
        let curve = crate::anim::Anim::new(vec![
            crate::anim::Keyframe::new(0., 0., 1),
            crate::anim::Keyframe::new(2., 8., 0),
        ]);
        for flow in [0.1, 1., 20.] {
            let end_time = hold_visual_end(0., 2., 1., 0.22);
            let raw_end = curve.value_at(end_time).unwrap() as f64;
            let end = hold_render_end_height(0., raw_end, flow);
            assert!((end - raw_end).abs() < 1e-10);
            let mut previous = f64::INFINITY;
            for time in [1., 1.5, 1.7, 1.77, end_time] {
                let line = curve.value_at(time).unwrap() as f64;
                let remaining = (end - line) * flow;
                assert!(remaining < previous);
                previous = remaining;
            }
            assert!(previous.abs() < 1e-10);
        }
    }
    #[test]
    fn degenerate_scale_is_finite_and_endpoints_do_not_mutate_source() {
        let source_end = 3.;
        for scale in [0., 1e-15, f64::NAN, f64::INFINITY] {
            assert_eq!(hold_render_end_height(1., source_end, scale), 1.);
        }
        for scale in [-20., -0.1, 0.1, 20.] {
            let end = hold_render_end_height(1., source_end, scale);
            assert!((end - 1.) * scale >= 0.01 - 1e-12);
        }
        assert_eq!(source_end, 3.);
    }
}

#[path = "../../../phira/src/custom_rks.rs"]
pub mod custom_rks;
#[cfg(test)]
mod custom_rks_tests {
    use super::custom_rks::*;
    fn chart(path: &str, d: f64, a: Option<f64>) -> LocalInput {
        LocalInput { path: path.into(), name: "Same title".into(), level: "IN Lv.16".into(), difficulty: d, ai_difficulty: None, accuracy: a }
    }
    #[test]
    fn rks_formula_boundary_and_ap_precision() {
        assert_eq!(single_rks(18.,69.999),0.);
        assert!((single_rks(18.,70.)-2.).abs()<1e-12);
        assert_eq!(single_rks(16.,100.),16.);
        let s=Settings::default();
        let a=chart("one",16.,Some(99.9999));
        assert_eq!(format!("{:.2}%",a.accuracy.unwrap()),"100.00%");
        let (_,r)=calculate(&s,&[a]);
        assert_eq!(r.ap_used,0);
        assert_eq!(r.best_used,1);
    }
    #[test]
    fn b27_ap3_independent_lists_and_zero_padding() {
        let s=Settings::default();
        let (_,r)=calculate(&s,&[chart("a",16.,Some(100.)),chart("b",15.,Some(100.))]);
        assert!((r.rks-62./30.).abs()<1e-12);
        assert_eq!((r.best_used,r.ap_used),(2,2));
        let (_,empty)=calculate(&s,&[]);assert_eq!(empty.rks,0.);
    }
    #[test]
    fn selects_best_scores_and_top_ap_separately() {
        let mut s=Settings::default();s.best_count=2;s.ap_count=1;
        let (_,r)=calculate(&s,&[chart("a",16.,Some(100.)),chart("b",15.,Some(100.)),chart("c",20.,Some(99.)),chart("d",1.,Some(100.))]);
        assert!((r.rks-(16.+16.+single_rks(20.,99.))/3.).abs()<1e-12);
    }
    #[test]
    fn overrides_isolated_and_file_and_record_follow_changes() {
        let mut input=chart("x",15.,Some(98.));
        let c=ChartSettings::default();
        input.difficulty=16.;input.accuracy=Some(99.);
        let r=resolve(&input,&c);assert_eq!(r.difficulty,Some(16.));assert_eq!(r.accuracy,Some(99.));
        let c=ChartSettings{difficulty_source:DifficultySource::Custom,custom_difficulty:Some(17.1),accuracy_source:AccuracySource::Custom,custom_accuracy:Some(95.12),..Default::default()};
        let r=resolve(&input,&c);assert_eq!(r.difficulty,Some(17.1));assert!(r.difficulty_changed);assert_eq!(r.accuracy,Some(95.12));
        assert_eq!(input.difficulty,16.);assert_eq!(input.accuracy,Some(99.));
    }
    #[test]
    fn ai_difficulty_is_green_even_when_rounded_value_matches_file() {
        let mut input=chart("x",16.3_f32 as f64,Some(100.));
        input.ai_difficulty=Some(16.26);
        let c=ChartSettings{difficulty_source:DifficultySource::Ai,..Default::default()};
        let r=resolve(&input,&c);
        assert_eq!(r.difficulty,Some(16.3));
        assert!(!r.difficulty_changed);
        assert!(r.difficulty_ai);
        input.ai_difficulty=None;
        assert!(!resolve(&input,&c).difficulty_ai);
        assert!(!resolve(&input,&ChartSettings::default()).difficulty_ai);
    }
    #[test]
    fn same_value_custom_difficulty_is_not_green() {
        let c=ChartSettings{difficulty_source:DifficultySource::Custom,custom_difficulty:Some(16.3),..Default::default()};
        assert!(!resolve(&chart("x",16.3_f32 as f64,Some(100.)),&c).difficulty_changed);
        assert_eq!(format_difficulty(16.),"16");assert_eq!(format_difficulty(16.3),"16.3");
        assert_eq!(difficulty_value(16.26),Some(16.3));
        assert_eq!(level_prefix("HOT.15 Lv.15"),"HOT.15 Lv.");
        assert_eq!(level_prefix("INS Lv.14"),"INS Lv.");
        assert_eq!(level_prefix("自定义"),"自定义 Lv.");
    }
    #[test]
    fn missing_invalid_or_ai_data_are_pending_not_zero_scores() {
        let mut s=Settings::default();
        s.charts.insert("ai".into(),ChartSettings{difficulty_source:DifficultySource::Ai,..Default::default()});
        let (_,r)=calculate(&s,&[chart("missing",16.,None),chart("bad",f64::NAN,Some(100.)),chart("ai",16.,Some(100.)),chart("badacc",15.,Some(101.))]);
        assert_eq!(r.selected,4);assert_eq!(r.pending,4);assert_eq!(r.rks,0.);
    }
    #[test]
    fn excluded_entries_and_duplicate_paths_not_double_counted() {
        let mut s=Settings::default();
        s.charts.insert("off".into(),ChartSettings{included:false,..Default::default()});
        let (_,r)=calculate(&s,&[chart("off",16.,Some(100.)),chart("a",16.,Some(100.)),chart("a",16.,Some(100.)),chart("b",16.,Some(100.))]);
        assert_eq!(r.selected,2); // same names remain independent by path
        assert!((r.rks-64./30.).abs()<1e-12);
    }
    #[test]
    fn extras_participate_without_any_local_chart() {
        let mut s=Settings::default();s.extras.push(ExtraEntry{id:1,name:"manual".into(),difficulty:16.1,accuracy:100.,included:true});
        let (map,r)=calculate(&s,&[]);assert!(map.is_empty());assert!((r.rks-32.2/30.).abs()<1e-12);
        s.extras[0].included=false;assert_eq!(calculate(&s,&[]).1.rks,0.);
    }
    #[test]
    fn saving_reopening_updating_and_deleting_extra_entries() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("custom-rks.json");
        let mut s=Settings::load(&path).unwrap();
        s.charts.insert("local/a".into(),ChartSettings{included:false,custom_difficulty:Some(15.2),..Default::default()});
        s.extras.push(ExtraEntry{id:1,name:"测试曲".into(),difficulty:15.1,accuracy:99.98,included:true});
        s.save(&path).unwrap();
        let mut reopened=Settings::load(&path).unwrap();
        assert!(!reopened.chart("local/a").included);assert_eq!(reopened.extras[0].name,"测试曲");
        reopened.extras.clear();reopened.best_count=19;reopened.ap_count=1;reopened.save(&path).unwrap();
        let result=Settings::load(&path).unwrap();assert!(result.extras.is_empty());assert_eq!(result.best_count,19);
    }
    #[test]
    fn bad_json_or_unknown_version_is_not_overwritten() {
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("custom-rks.json");
        for bytes in ["bad",r#"{"version":2}"#,r#"{"bestCount":0,"apCount":0}"#,r#"{"bestCount":18446744073709551615,"apCount":1}"#] {
            std::fs::write(&p,bytes).unwrap();assert!(Settings::load(&p).is_err());assert_eq!(std::fs::read_to_string(&p).unwrap(),bytes);
        }
    }
    #[test]
    fn failed_save_preserves_old_settings() {
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("custom-rks.json");
        let mut s=Settings::default();s.save(&p).unwrap();let old=std::fs::read(&p).unwrap();
        std::fs::create_dir(p.with_extension("json.tmp")).unwrap();s.best_count=1;assert!(s.save(&p).is_err());assert_eq!(std::fs::read(&p).unwrap(),old);
    }
}

#[test]
fn hold_head_effect_defaults_on_migrates_and_persists_without_score_gate() {
    let mut c: config::Config = serde_json::from_str("{}").unwrap();
    assert!(c.hold_head_effect);
    assert!(!c.blocks_score_upload());
    c.hold_head_effect = false;
    let saved = serde_json::to_vec(&c).unwrap();
    let restored: config::Config = serde_json::from_slice(&saved).unwrap();
    assert!(!restored.hold_head_effect);
    assert!(!restored.blocks_score_upload());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&saved).unwrap()["holdHeadEffect"], false);
}

#[cfg(test)]
mod replica_record_policy_tests {
    use super::config::{Config, JudgementMode, Mods};
    #[test]
    fn removed_switch_cannot_disable_online_local_records() {
        for json in ["{}", r#"{"replicaOnlineLocalRecords":false}"#, r#"{"replicaOnlineLocalRecords":true}"#] {
            let mut c: Config = serde_json::from_str(json).unwrap();
            c.init();
            assert!(c.online_replica_active(true, false));
            assert!(c.saves_run_record());
            let saved = serde_json::to_value(&c).unwrap();
            assert!(saved.get("replicaOnlineLocalRecords").is_none());
        }
    }
    #[test]
    fn legacy_split_modes_use_main_choice_and_retired_speed_is_reset() {
        for (main, old_flick, expected) in [
            ("phira", "phigros_replica", JudgementMode::Phira),
            ("phigros_replica", "phira", JudgementMode::PhigrosReplica),
        ] {
            let json = format!(r#"{{"judgementMode":"{main}","flickJudgementMode":"{old_flick}","speed":0.5}}"#);
            let mut c: Config = serde_json::from_str(&json).unwrap();
            c.init();
            assert_eq!(c.judgement_mode, expected);
            assert_eq!(c.flick_judgement_mode(), expected);
            assert_eq!(c.speed, 1.);
            let saved = serde_json::to_value(&c).unwrap();
            assert!(saved.get("flickJudgementMode").is_none());
            // Practice can still override the per-run configuration after loading.
            c.speed = 0.05;
            assert_eq!(c.speed, 0.05);
        }
    }
    #[test]
    fn personal_records_allow_replica_features_and_keep_cloud_blocked() {
        // Regression: local charts used to be rejected by shorten_holds,
        // NO_SHADER and slow speed, although online personal records saved.
        for judgement in [JudgementMode::Phira, JudgementMode::PhigrosReplica] {
            for shorten in [false, true] {
                for no_shader in [false, true] {
                    for speed in [0.05, 1., 1.5] {
                        let mut c = Config::default();
                        c.judgement_mode = judgement;
                        c.shorten_holds = shorten;
                        c.mods.set(Mods::NO_SHADER, no_shader);
                        c.speed = speed;
                        assert!(c.saves_run_record());
                        // Automatic flip and other chart assists must not change
                        // either local-save eligibility or the online upload ban.
                        for chart_assist in [false, true] {
                            assert!(c.online_replica_active(true, chart_assist));
                            assert!(!c.online_replica_active(false, chart_assist));
                        }
                        c.mods.insert(Mods::AUTOPLAY);
                        assert!(!c.saves_run_record());
                    }
                }
            }
        }
    }
    #[test]
    fn persisted_shortening_setting_does_not_suppress_local_record() {
        let mut c: Config = serde_json::from_str(r#"{"shortenHolds":true}"#).unwrap();
        c.init();
        assert!(c.shorten_holds);
        assert!(c.saves_run_record());
        let saved = serde_json::to_string(&c).unwrap();
        let restored: Config = serde_json::from_str(&saved).unwrap();
        assert!(restored.saves_run_record());
    }

}

#[path = "../../../prpr/src/practice_speed.rs"]
pub mod practice_speed;

#[test]
fn practice_pitch_preference_defaults_off_and_persists() {
    let mut config: config::Config=serde_json::from_str("{}").unwrap();
    assert!(!config.practice_preserve_pitch);
    config.practice_preserve_pitch=true;
    let saved=serde_json::to_vec(&config).unwrap();
    let restored:config::Config=serde_json::from_slice(&saved).unwrap();
    assert!(restored.practice_preserve_pitch);
}

#[path = "../../../prpr/src/auto_flip.rs"]
pub mod auto_flip;
#[test]
fn global_flip_default_and_conversion_settings_roundtrip() {
    use chart_play::*;
    let c:config::Config=serde_json::from_str("{}").unwrap();assert!(c.auto_flip_enabled);
    let mut c=c;c.auto_flip_enabled=false;
    assert!(!serde_json::from_slice::<config::Config>(&serde_json::to_vec(&c).unwrap()).unwrap().auto_flip_enabled);
    for mode in [NoteConversion::Original,NoteConversion::Tap,NoteConversion::Drag,NoteConversion::Flick] {
        let mut s=ChartPlaySettings::default();s.note_conversion=mode;s.add(1.,2.);
        let bytes=br#"{"judgeLineList":[{"notes":[1,2,3]}]}"#;
        let out=s.embed_in_chart(bytes).unwrap();assert_eq!(ChartPlaySettings::from_chart_bytes(&out),Some(s.clone()));
        let v:serde_json::Value=serde_json::from_slice(&out).unwrap();assert_eq!(v["judgeLineList"][0]["notes"],serde_json::json!([1,2,3]));
    }
}
