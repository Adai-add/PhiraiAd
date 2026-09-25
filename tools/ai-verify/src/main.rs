#[path = "../../../phira/src/ai_chart_adapter.rs"]
mod ai_chart_adapter;
#[path = "../../../phira/src/ai_model/mod.rs"]
mod ai_model;
fn main() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let vectors = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ai-vectors");
    let raw = std::fs::read(vectors.join("one_group.raw.json"))?;
    std::fs::write(root.path().join("chart.json"), raw)?;
    std::fs::write(root.path().join("info.yml"), "chart: chart.json\n")?;
    let doc = ai_model::cache::Document::load(root.path())?;
    let f = ai_chart_adapter::extract(&doc, &mut || Ok(()))?;
    let model = ai_model::Predictor::new()?;
    println!("PGR: {:?}", model.predict(&f, doc.hash, &mut || Ok(()))?.values);
    let mut old: serde_json::Value = serde_json::from_slice(&std::fs::read(vectors.join("one_group.raw.json"))?)?;
    old["formatVersion"] = 1.into();
    for line in old["judgeLineList"].as_array_mut().unwrap() {
        if line.get("judgeLineMoveEvents").is_none() {
            line["judgeLineMoveEvents"] = serde_json::json!([{ "startTime":-9999,"endTime":9999,"start":0.5,"end":0.5,"start2":0.5,"end2":0.5 }]);
        }
        for e in line["judgeLineMoveEvents"].as_array_mut().unwrap() {
            for (a, b) in [("start", "start2"), ("end", "end2")] {
                e[a] = (e[a].as_f64().unwrap() * 880. * 1000. + e[b].as_f64().unwrap() * 520.).into();
                e.as_object_mut().unwrap().remove(b);
            }
        }
    }
    std::fs::write(root.path().join("chart.json"), serde_json::to_vec(&old)?)?;
    let olddoc = ai_model::cache::Document::load(root.path())?;
    let oldf = ai_chart_adapter::extract(&olddoc, &mut || Ok(()))?;
    for (a, b) in f.events.iter().flatten().zip(oldf.events.iter().flatten()) {
        anyhow::ensure!((a - b).abs() < 1e-7, "format1 coordinate mismatch");
    }
    println!("PGR1: {:?}", model.predict(&oldf, olddoc.hash, &mut || Ok(()))?.values);
    let pec="0\nbp 0 120\ncp 0 0 1024 700\nca 0 0 255\ncd 0 0 0\ncv 0 0 7\nn1 0 1 1024 1 0\n# 1\n& 1\nn2 0 2 4 1024 1 0\n# 1\n& 1\nn3 0 5 1024 1 0\n# 1\n& 1\nn4 0 6 1024 1 0\n# 1\n& 1\n";
    std::fs::write(root.path().join("chart.pec"), pec)?;
    std::fs::write(root.path().join("info.yml"), "chart: chart.pec\n")?;
    let doc = ai_model::cache::Document::load(root.path())?;
    let f = ai_chart_adapter::extract(&doc, &mut || Ok(()))?;
    anyhow::ensure!(f.events.len() == 4, "PEC note mapping");
    println!("PEC: {:?}", model.predict(&f, doc.hash, &mut || Ok(()))?.values);

    // Equivalent constant-line RPE chart, including an ignored fake note and missing visual/sound assets.
    let event = |v: f64| serde_json::json!({"startTime":[0,0,1],"endTime":[16,0,1],"start":v,"end":v});
    let note = |kind: u8, beat: u8, end: u8, fake: u8| serde_json::json!({"type":kind,"above":1,"startTime":[beat,0,1],"endTime":[end,0,1],"positionX":0,"yOffset":0,"alpha":255,"size":1,"speed":1,"isFake":fake,"visibleTime":999,"hitsound":"does-not-exist.wav"});
    let rpe = serde_json::json!({"META":{"offset":0,"RPEVersion":170},"BPMList":[{"startTime":[0,0,1],"bpm":120}],"judgeLineList":[{"Name":"test","Texture":"does-not-exist.png","isCover":1,"eventLayers":[{"moveXEvents":[event(0.)],"moveYEvents":[event(0.)],"rotateEvents":[event(0.)],"alphaEvents":[event(255.)],"speedEvents":[event(7.)]}],"notes":[note(1,1,1,0),note(2,2,4,0),note(3,5,5,0),note(4,6,6,0),note(1,7,7,1)]}]});
    std::fs::write(root.path().join("chart.json"), serde_json::to_vec(&rpe)?)?;
    std::fs::write(root.path().join("info.yml"), "chart: chart.json\nuseRpe170Speed: true\n")?;
    let doc = ai_model::cache::Document::load(root.path())?;
    let f = ai_chart_adapter::extract(&doc, &mut || Ok(()))?;
    anyhow::ensure!(f.events.len() == 4, "RPE fake notes must be excluded");
    println!("RPE: {:?}", model.predict(&f, doc.hash, &mut || Ok(()))?.values);
    // PBC conversion uses the actual binary writer/reader; no texture resources.
    let chart = prpr::parse::parse_pec(pec, prpr::core::ChartExtra::default())?;
    let mut bytes = Vec::new();
    prpr::bin::BinaryWriter::new(&mut bytes).write(&chart)?;
    std::fs::write(root.path().join("chart.pbc"), bytes)?;
    std::fs::write(root.path().join("info.yml"), "chart: chart.pbc\nformat: pbc\n")?;
    let doc = ai_model::cache::Document::load(root.path())?;
    let f = ai_chart_adapter::extract(&doc, &mut || Ok(()))?;
    anyhow::ensure!(f.events.len() == 4, "PBC note mapping");
    println!("PBC: {:?}", model.predict(&f, doc.hash, &mut || Ok(()))?.values);
    Ok(())
}
