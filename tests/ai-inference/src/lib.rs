#[path = "../../../phira/src/ai_model/mod.rs"]
pub mod ai_model;
#[path = "../../../phira/src/custom_rks.rs"]
pub mod custom_rks;
#[cfg(test)]
mod tests {
    use super::ai_model::*;
    use std::path::PathBuf;
    #[test]
    fn portable_reference_features_and_all_five_outputs_match() {
        let predictor = Predictor::new().unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ai-vectors");
        for name in ["one_group", "exact_block", "block_plus_one", "geometry_holds", "zero_speed_event"] {
            let read = |suffix: &str| {
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(root.join(format!("{name}.{suffix}.json"))).unwrap()).unwrap()
            };
            let raw = read("raw");
            let expected = read("expected");
            let arrays = read("features");
            let features = features::extract(&raw, &mut || Ok(())).unwrap();
            let actual = serde_json::to_value(&features).unwrap();
            fn compare(a: &serde_json::Value, b: &serde_json::Value, name: &str) {
                if let Some(a) = a.as_array() {
                    let b = b.as_array().unwrap();
                    assert_eq!(a.len(), b.len(), "{name}");
                    for (a, b) in a.iter().zip(b) {
                        compare(a, b, name);
                    }
                } else {
                    let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                    assert!((a - b).abs() < 1e-7, "{name}: {a} != {b}");
                }
            }
            for key in ["events", "note_groups", "group_features", "group_times", "group_durations", "stats"] {
                compare(&actual[key], &arrays[key], key);
            }
            let result = predictor.predict(&features, "reference-vector".into(), &mut || Ok(())).unwrap();

            // Expected file structure is validated below against the supplied kit.
            let refs = expected["models"].as_array().unwrap();
            let scores = refs
                .iter()
                .filter(|r| r["seed"] == 42)
                .map(|r| r["prediction"].as_f64().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(scores.len(), 5);
            for (a, b) in result.values.iter().zip(&scores) {
                assert!((a - b).abs() < 1e-4, "{name}: {a} != {b}");
            }
            assert!((result.mean - scores.iter().sum::<f64>() / 5.).abs() < 1e-4);
            assert!(result.valid("reference-vector"));
        }
    }
}
#[cfg(test)]
mod cache_tests {
    use super::ai_model::{self, cache::Document, Prediction};
    fn prepare() -> (tempfile::TempDir, serde_json::Value) {
        let root = tempfile::tempdir().unwrap();
        let raw: serde_json::Value = serde_json::from_slice(include_bytes!("../../ai-vectors/one_group.raw.json")).unwrap();
        std::fs::write(root.path().join("chart.json"), serde_json::to_vec(&raw).unwrap()).unwrap();
        std::fs::write(root.path().join("info.yml"), "chart: chart.json\nname: Original\ndifficulty: 16.3\n").unwrap();
        (root, raw)
    }
    fn prediction(doc: &Document) -> Prediction {
        Prediction {
            fallback: false,
            version: ai_model::VERSION.into(),
            chart_hash: doc.hash.clone(),
            model_hash: ai_model::model_hash(),
            model_count: 5,
            values: vec![15., 16., 17., 18., 19.],
            mean: 17.,
        }
    }
    #[test]
    fn large_chart_predicts_all_five_models_and_reuses_saved_cache() {
        use std::io::Write;
        let (root, raw) = prepare();
        let file = root.path().join("chart.json");
        // A real >32 MiB JSON string, not just padding: exercises size tracking and
        // streaming serialization while leaving the model's note inputs intact.
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&file).unwrap());
        writer.write_all(b"{\"decoration\":\"").unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..33 {
            writer.write_all(&chunk).unwrap();
        }
        writer.write_all(b"\",").unwrap();
        writer.write_all(&serde_json::to_vec(&raw).unwrap()[1..]).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let mut doc = Document::load(root.path()).unwrap();
        assert_eq!(doc.bytes.capacity(), 0);
        let features = ai_model::features::extract(doc.raw.as_ref().unwrap(), &mut || Ok(())).unwrap();
        doc.release_input();
        assert!(doc.raw.is_none());
        let model = ai_model::Predictor::new().unwrap();
        let p = model.predict(&features, doc.hash.clone(), &mut || Ok(())).unwrap();
        assert_eq!(p.values.len(), 5);
        doc.save(&p).unwrap();
        let cached = Document::load(root.path()).unwrap();
        assert_eq!(cached.cached().unwrap().values, p.values);
        assert_eq!(cached.raw.as_ref().unwrap()["decoration"].as_str().unwrap().len(), 33 * 1024 * 1024);
        drop(cached);
        // Releasing the source must not weaken the concurrent-edit check.
        std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(doc.save(&p).unwrap_err().to_string().contains("谱面已变更"));
    }
    #[test]
    fn accepts_exactly_127_mib_and_rejects_one_extra_byte() {
        use std::io::Write;
        let (root, _) = prepare();
        let file = root.path().join("chart.json");
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&file).unwrap());
        writer.write_all(b"{}").unwrap();
        let chunk = vec![b' '; 1024 * 1024];
        let mut remaining = ai_model::cache::MAX_CHART_BYTES - 2;
        while remaining != 0 {
            let n = remaining.min(chunk.len() as u64) as usize;
            writer.write_all(&chunk[..n]).unwrap();
            remaining -= n as u64;
        }
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(file.metadata().unwrap().len(), 127 * 1024 * 1024);
        assert!(Document::load(root.path()).unwrap().raw.is_some());
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(b" ").unwrap();
        let error = Document::load(root.path()).err().unwrap();
        assert!(error.to_string().contains("127 MiB"));
    }
    #[test]
    fn embedded_cache_reuses_and_detects_chart_and_model_changes() {
        let (root, raw) = prepare();
        let doc = Document::load(root.path()).unwrap();
        let result = prediction(&doc);
        doc.save(&result).unwrap();
        let cached = Document::load(root.path()).unwrap();
        assert_ne!(cached.hash, doc.hash); // Embedding increased file size.
        assert!(cached.hash.starts_with("bytes:"));
        assert_eq!(cached.cached().unwrap().mean, 17.);
        let moved = tempfile::tempdir().unwrap();
        std::fs::copy(&doc.file, moved.path().join("renamed.json")).unwrap();
        std::fs::write(moved.path().join("info.yml"), "chart: renamed.json\nformat: pgr\nname: Renamed\n").unwrap();
        assert_eq!(Document::load(moved.path()).unwrap().cached().unwrap().mean, 17.);

        let mut saved = cached.raw.unwrap();
        saved.as_object_mut().unwrap().remove(ai_model::CACHE_FIELD);
        assert_eq!(saved, raw);
        let mut changed = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&doc.file).unwrap()).unwrap();
        changed["offset"] = 123.into();
        std::fs::write(&doc.file, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(Document::load(root.path()).unwrap().cached().is_none());
        assert!(doc.save(&result).is_err()); // old background result must not overwrite a new chart
        let mut invalid = result;
        invalid.model_hash = "old model".into();
        assert!(!invalid.valid(&doc.hash));
    }
    #[test]
    fn text_cache_is_inside_chart_package_and_preserves_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("chart.pec"), "0\nbp 0 120\n").unwrap();
        std::fs::write(root.path().join("info.yml"), "chart: chart.pec\nname: Song\ncustomField: keep\n").unwrap();
        let doc = Document::load(root.path()).unwrap();
        doc.save(&prediction(&doc)).unwrap();
        assert_eq!(Document::load(root.path()).unwrap().cached().unwrap().mean, 17.);
        let y: serde_yaml::Value = serde_yaml::from_slice(&std::fs::read(root.path().join("info.yml")).unwrap()).unwrap();
        assert_eq!(y["customField"].as_str(), Some("keep"));
        assert_eq!(std::fs::read(root.path().join("chart.pec")).unwrap(), b"0\nbp 0 120\n");
        std::fs::write(root.path().join("chart.pec"), "0\nbp 0 1300\n").unwrap();
        assert!(Document::load(root.path()).unwrap().cached().is_none());
    }
    #[test]
    fn cache_rejects_corruption_and_tracks_parser_options() {
        let (root, _) = prepare();
        let doc = Document::load(root.path()).unwrap();
        let mut p = prediction(&doc);
        p.values.pop();
        assert!(doc.save(&p).is_err());
        let p = prediction(&doc);
        doc.save(&p).unwrap();
        std::fs::write(root.path().join("info.yml"), "chart: chart.json\nformat: rpe\nuseRpe170Speed: true\n").unwrap();
        assert!(Document::load(root.path()).unwrap().cached().is_none());
    }
    #[test]
    fn equal_byte_edits_keep_cache_but_length_changes_invalidate() {
        let (root, _) = prepare();
        let doc = Document::load(root.path()).unwrap();
        doc.save(&prediction(&doc)).unwrap();
        let mut bytes = std::fs::read(&doc.file).unwrap();
        let needle = b"\"offset\":";
        let offset = bytes.windows(needle.len()).position(|w| w == needle).unwrap();
        bytes[offset + needle.len() + 3] = b'9';
        std::fs::write(&doc.file, &bytes).unwrap();
        assert!(Document::load(root.path()).unwrap().cached().is_some());
        bytes.push(b' ');
        std::fs::write(&doc.file, bytes).unwrap();
        assert!(Document::load(root.path()).unwrap().cached().is_none());
    }
    #[test]
    fn legacy_embedded_result_migrates_without_running_models() {
        let (root, mut raw) = prepare();
        let doc = Document::load(root.path()).unwrap();
        let mut old = prediction(&doc);
        old.chart_hash = "a".repeat(64);
        raw[ai_model::CACHE_FIELD] = serde_json::to_value(&old).unwrap();
        std::fs::write(&doc.file, serde_json::to_vec(&raw).unwrap()).unwrap();
        let loaded = Document::load(root.path()).unwrap();
        assert!(loaded.needs_migration());
        let saved = loaded.save(&loaded.cached().unwrap()).unwrap();
        let loaded = Document::load(root.path()).unwrap();
        assert!(!loaded.needs_migration());
        assert_eq!(saved.values, old.values);
        assert_eq!(loaded.cached().unwrap().chart_hash, loaded.hash);
    }
    #[test]
    fn index_restart_restores_rks_without_parsing_charts() {
        use ai_model::{cache::Source, index::Index};
        let (root, _) = prepare();
        let doc = Document::load(root.path()).unwrap();
        let result = doc.save(&prediction(&doc)).unwrap();
        let source = Source::probe(root.path()).unwrap();
        let mut index = Index::default();
        index.insert("chart", root.path(), &source, "pgr", result).unwrap();
        let file = root.path().join("index.json");
        index.save(&file).unwrap();
        // Deliberately unreadable JSON of the same byte length proves restart
        // restoration relies on metadata, not chart parsing or content hashing.
        std::fs::write(&source.file, vec![b'x'; source.bytes as usize]).unwrap();
        let restarted = Index::load(&file);
        let r = &restarted.records["chart"];
        assert!(r.restore(root.path()).is_some());
        let mut settings = crate::custom_rks::Settings::default();
        settings.charts.entry("chart".into()).or_default().difficulty_source = crate::custom_rks::DifficultySource::Ai;
        let inputs = [crate::custom_rks::LocalInput {
            path: "chart".into(),
            name: "Test".into(),
            level: "IN".into(),
            difficulty: 15.,
            ai_difficulty: Some(r.prediction.mean),
            accuracy: Some(100.),
        }];
        assert!(crate::custom_rks::calculate(&settings, &inputs).1.rks > 0.);
        std::fs::write(&source.file, b"changed size").unwrap();
        assert!(r.restore(root.path()).is_none());
        std::fs::remove_file(&source.file).unwrap();
        assert!(r.restore(root.path()).is_none());
    }
    #[test]
    fn index_rejects_bad_model_and_unsafe_paths_and_survives_corruption() {
        use ai_model::{cache::Source, index::Index};
        let (root, _) = prepare();
        let doc = Document::load(root.path()).unwrap();
        let source = Source::probe(root.path()).unwrap();
        let mut index = Index::default();
        index.insert("chart", root.path(), &source, "pgr", prediction(&doc)).unwrap();
        let file = root.path().join("index.json");
        index.records.get_mut("chart").unwrap().prediction.model_hash = "outdated".into();
        assert!(index.records["chart"].restore(root.path()).is_none());
        std::fs::write(&file, b"truncated {").unwrap();
        assert!(Index::load(&file).records.is_empty());
        assert!(ai_model::index::chart_root(root.path(), "../escape").is_none());
        assert!(ai_model::index::chart_root(root.path(), "/absolute").is_none());
    }
    #[test]
    fn cache_write_failure_preserves_result_but_changed_chart_is_rejected() {
        let (root, _) = prepare();
        let mut doc = Document::load(root.path()).unwrap();
        let expected = prediction(&doc);
        let blocked = root.path().join("blocked-cache-target");
        std::fs::create_dir(&blocked).unwrap();
        doc.file = blocked;
        assert!(doc.save(&expected).is_err());
        let kept = doc.save_or_keep(&expected).unwrap();
        assert_eq!(kept.values, expected.values);
        assert_eq!(kept.chart_hash, expected.chart_hash);
        std::fs::write(root.path().join("chart.json"), b"{}").unwrap();
        assert!(doc.save_or_keep(&expected).is_err());
    }
    #[test]
    fn explicit_empty_fallback_survives_embedded_cache_round_trip() {
        let (root, _) = prepare();
        std::fs::write(root.path().join("chart.json"), br#"{"formatVersion":3,"judgeLineList":[]}"#).unwrap();
        let doc = Document::load(root.path()).unwrap();
        let f = ai_model::features::extract(doc.raw.as_ref().unwrap(), &mut || Ok(())).unwrap();
        let p = ai_model::Predictor::new().unwrap().predict(&f, doc.hash.clone(), &mut || Ok(())).unwrap();
        doc.save(&p).unwrap();
        let cached = Document::load(root.path()).unwrap().cached().unwrap();
        assert!(cached.fallback);
        assert_eq!(cached.mean, 0.);
        assert!(cached.message().contains("兜底"));
    }
    #[test]
    fn broken_json_is_not_disguised_as_empty_prediction() {
        let (root, _) = prepare();
        std::fs::write(root.path().join("chart.json"), b"{broken JSON").unwrap();
        assert!(Document::load(root.path()).is_err());
    }
    #[test]
    fn malformed_local_fields_are_tolerated_and_cancellation_survives() {
        let (_, mut raw) = prepare();
        raw["judgeLineList"][0]["bpm"] = 0.into();
        assert!(ai_model::features::extract(&raw, &mut || Ok(())).is_ok());
        let (_, mut raw) = prepare();
        raw["judgeLineList"][0]["speedEvents"] = serde_json::json!([{"startTime":0,"endTime":0,"value":1}]);
        assert!(ai_model::features::extract(&raw, &mut || Ok(())).is_ok());
        let (_, raw) = prepare();
        let mut calls = 0;
        let e = ai_model::features::extract(&raw, &mut || {
            calls += 1;
            anyhow::bail!("cancelled")
        });
        assert!(e.is_err());
        assert!(calls > 0);
    }
}
#[cfg(test)]
mod queue_tests {
    use super::ai_model::scheduler::*;
    use std::collections::BTreeSet;
    #[test]
    fn ai_requests_precede_background_and_work_with_auto_off() {
        let paths = ["a", "b", "c"].map(str::to_owned).into_iter().collect::<BTreeSet<_>>();
        let priority = ["c", "d"].map(str::to_owned).into_iter().collect::<BTreeSet<_>>();
        assert_eq!(order(&paths, &priority, true), ["c", "d", "a", "b"]);
        assert_eq!(order(&paths, &priority, false), ["c", "d"]);
        assert!(allowed("c", &priority, false));
        assert!(!allowed("a", &priority, false));
        assert!(order(&paths, &BTreeSet::new(), false).is_empty());
    }
}

#[cfg(test)]
mod tolerance_tests {
    use super::ai_model::{self, features, sanitize, Predictor};
    use serde_json::json;
    fn chart() -> serde_json::Value {
        json!({"formatVersion":3,"judgeLineList":[{"bpm":120,"notesAbove":[{"type":1,"time":32,"positionX":0,"speed":1}]}]})
    }
    #[test]
    fn reversed_unused_and_missing_events_do_not_change_stationary_notes() {
        let raw = chart();
        let expected = features::extract(&raw, &mut || Ok(())).unwrap();
        let mut raw = raw;
        raw["judgeLineList"][0]["judgeLineMoveEvents"] = json!([{"startTime":100,"endTime":20,"start":100,"end":200}]);
        raw["judgeLineList"][0]["judgeLineDisappearEvents"] = json!("invalid visual data");
        assert_eq!(serde_json::to_value(features::extract(&raw, &mut || Ok(())).unwrap()).unwrap(), serde_json::to_value(expected).unwrap());
    }
    #[test]
    fn strings_invalid_bpm_types_fake_notes_and_negative_holds() {
        let mut raw = chart();
        raw["judgeLineList"][0]["bpm"] = json!("not a bpm");
        raw["judgeLineList"][0]["notesAbove"] = json!([
            {"type":"1","time":"32","positionX":"0"},
            {"type":3,"time":64,"holdTime":-20},
            {"type":7,"time":20}, {"type":1,"time":"NaN"}, {"type":1,"time":20,"isFake":true}]);
        let f = features::extract(&raw, &mut || Ok(())).unwrap();
        assert_eq!(f.events.len(), 2);
        assert_eq!(f.group_times, vec![0., 0.5]);
        assert_eq!(f.events[1][6], 0.);
        assert!(Predictor::new()
            .unwrap()
            .predict(&f, "test".into(), &mut || Ok(()))
            .unwrap()
            .mean
            .is_finite());
    }
    #[test]
    fn gaps_overlaps_and_instant_events_remain_finite() {
        let mut raw = chart();
        raw["judgeLineList"][0]["judgeLineRotateEvents"] = json!([
            {"startTime":0,"endTime":10,"start":0,"end":45},
            {"startTime":5,"endTime":20,"start":30,"end":90},
            {"startTime":32,"endTime":32,"start":90,"end":180}]);
        let f = features::extract(&raw, &mut || Ok(())).unwrap();
        f.validate().unwrap();
        assert_eq!(f.events.len(), 1);
    }
    #[test]
    fn empty_result_is_explicit_and_cancellation_is_not_swallowed() {
        let f = features::extract(&json!({"judgeLineList":[]}), &mut || Ok(())).unwrap();
        let p = Predictor::new().unwrap();
        let result = p.predict(&f, "empty".into(), &mut || Ok(())).unwrap();
        assert_eq!(result.values, vec![0.; 5]);
        assert!(result.fallback);
        assert!(result.valid("empty"));
        assert!(result.message().contains("兜底"));
        assert!(p.predict(&f, "empty".into(), &mut || anyhow::bail!("cancelled")).is_err());
    }
    #[test]
    fn rpe_bad_bpm_parent_cycle_and_note_fields_are_repaired() {
        let raw = json!({"META":{"offset":"bad"},"BPMList":[{"bpm":0,"startTime":[0,0,0]}],"judgeLineList":[{"father":1,"notes":[{"type":1,"startTime":[1,0,1],"positionX":"bad"}]},{"father":0,"notes":[{"type":2,"startTime":[2,0,1],"endTime":[1,0,1]}]}]});
        let repaired = sanitize::rpe(&raw);
        assert_eq!(repaired["judgeLineList"][1]["father"], -1);
        let f = sanitize::rpe_notes(&raw, &mut || Ok(())).unwrap();
        assert_eq!(f.events.len(), 2);
        assert_eq!(f.events[1][6], 0.);
        f.validate().unwrap();
        assert!(Predictor::new()
            .unwrap()
            .predict(&f, "rpe".into(), &mut || Ok(()))
            .unwrap()
            .mean
            .is_finite());
    }
    #[test]
    fn pec_fallback_keeps_valid_notes_and_ignores_bad_commands() {
        let f = sanitize::pec_notes(
            "bad offset\nbp 0 0\n# 4\nunknown nonsense\ncm 0 5 2 0 0 1\nn1 0 1 0 1 0\nn2 0 2 1 0 1 0\nn1 0 invalid 0 1 0\nn3 0 3 0 1 1\n",
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(f.events.len(), 2);
        assert_eq!(f.events[1][6], 0.);
        f.validate().unwrap();
    }
    #[test]
    fn pec_command_filter_removes_reversed_events_and_unsafe_ids_only() {
        let text = "0\nbp 0 120\ncp 0 0 1024 700\ncm 0 5 2 0 0 1\nn1 999999999 1 0 1 0\nn1 0 1 0 1 0\nunknown command\n";
        let repaired = sanitize::pec(text);
        assert!(repaired.contains("cp 0 0 1024 700"));
        assert!(repaired.contains("n1 0 1 0 1 0"));
        assert!(!repaired.contains("cm "));
        assert!(!repaired.contains("999999999"));
        assert!(!repaired.contains("unknown"));
    }
    #[test]
    fn large_time_offsets_do_not_destroy_group_order() {
        let mut raw = chart();
        raw["judgeLineList"][0]["bpm"] = json!(1e6);
        raw["judgeLineList"][0]["notesAbove"] = json!([{"type":1,"time":-1e12},{"type":1,"time":0},{"type":1,"time":1}]);
        let f = features::extract(&raw, &mut || Ok(())).unwrap();
        assert_eq!(f.events.len(), 3);
        f.validate().unwrap();
        assert!(Predictor::new()
            .unwrap()
            .predict(&f, "time precision".into(), &mut || Ok(()))
            .unwrap()
            .mean
            .is_finite());
    }
    #[test]
    fn finite_extreme_feature_values_do_not_overflow_models() {
        let mut raw = chart();
        raw["judgeLineList"][0]["notesAbove"][0]["positionX"] = json!(1e250);
        let f = features::extract(&raw, &mut || Ok(())).unwrap();
        let p = Predictor::new().unwrap().predict(&f, "extreme".into(), &mut || Ok(())).unwrap();
        assert!(p.values.iter().all(|v| v.is_finite()));
    }
}
