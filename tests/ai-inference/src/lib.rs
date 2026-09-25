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
    fn malformed_charts_do_not_become_zero_predictions() {
        let (_, mut raw) = prepare();
        raw["judgeLineList"][0]["bpm"] = 0.into();
        assert!(ai_model::features::extract(&raw, &mut || Ok(())).is_err());
        let (_, mut raw) = prepare();
        raw["judgeLineList"][0]["speedEvents"] = serde_json::json!([{"startTime":0,"endTime":0,"value":1}]);
        assert!(ai_model::features::extract(&raw, &mut || Ok(())).is_err());
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
