//! Convert non-format3 charts through Phira's own parser, without loading graphics/audio.
//! Sampling keeps the training coordinate system, never the device aspect or gameplay modifiers.
use crate::ai_model::{
    cache::Document,
    features::{self, Features, Note},
};
use anyhow::{ensure, Context, Result};
use prpr::core::{AnimFloat, Chart, ChartExtra, JudgeLine, NoteKind, HEIGHT_RATIO};
const ASPECT: f64 = 1.777778;
fn value(a: &AnimFloat, t: f64) -> f64 {
    a.value_at(t).unwrap_or(0.) as f64
}
fn geometry(lines: &[JudgeLine], index: usize, t: f64, depth: usize) -> Result<([f64; 2], f64)> {
    ensure!(depth < 128 && index < lines.len(), "判定线父子关系无效");
    let line = &lines[index];
    let mut p = [value(&line.object.translation.0, t), value(&line.object.translation.1, t) / ASPECT];
    let mut rotation = value(&line.object.rotation, t);
    if let Some(parent) = line.parent {
        let (pp, pr) = geometry(lines, parent, t, depth + 1)?;
        let r = pr.to_radians();
        p = [pp[0] + p[0] * r.cos() - p[1] * r.sin(), pp[1] + p[0] * r.sin() + p[1] * r.cos()];
        if line.rot_with_parent {
            rotation += pr;
        }
    }
    Ok((p, rotation))
}
fn speed(a: &AnimFloat, t: f64) -> f64 {
    let i = a.keyframes.partition_point(|k| k.time < t).saturating_sub(1);
    let own = if let (Some(k), Some(next)) = (a.keyframes.get(i), a.keyframes.get(i + 1)) {
        let dt = next.time - k.time;
        if dt <= 0. {
            0.
        } else {
            let u = ((t - k.time) / dt).clamp(0., 1.);
            let lo = (u - 0.001).max(0.);
            let hi = (u + 0.001).min(1.);
            (next.value - k.value) as f64 / dt * (k.tween.y(hi as f32) - k.tween.y(lo as f32)) as f64 / (hi - lo)
        }
    } else {
        0.
    };
    own + a.next.as_ref().map_or(0., |n| speed(n, t))
}
pub fn extract(doc: &Document, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    if let Some(raw) = &doc.raw {
        if raw["formatVersion"] == 3 {
            return features::extract(raw, checkpoint);
        }
        if raw["formatVersion"] == 1 {
            let mut raw = raw.clone();
            raw["formatVersion"] = 3.into();
            for line in raw["judgeLineList"].as_array_mut().context("缺少判定线")? {
                for e in line["judgeLineMoveEvents"].as_array_mut().context("缺少移动事件")? {
                    for (a, b) in [("start", "start2"), ("end", "end2")] {
                        let v = e[a].as_f64().context("移动事件无效")?;
                        let y = v % 1000.;
                        e[a] = serde_json::Value::from((v - y) / 1000. / 880.);
                        e[b] = serde_json::Value::from(y / 520.);
                    }
                }
            }
            return features::extract(&raw, checkpoint);
        }
    }
    checkpoint()?;
    let chart: Chart = if let Some(raw) = &doc.raw {
        ensure!(raw.get("META").is_some() && raw.get("judgeLineList").is_some(), "不支持的 JSON 谱面格式");
        let mut raw = raw.clone();
        for line in raw["judgeLineList"].as_array_mut().context("缺少判定线")? {
            line["Texture"] = "line.png".into();
            if let Some(ext) = line.get_mut("extended").and_then(|v| v.as_object_mut()) {
                for k in ["textEvents", "gifEvents", "paintEvents"] {
                    ext.remove(k);
                }
            }
            if let Some(notes) = line.get_mut("notes").and_then(|v| v.as_array_mut()) {
                for note in notes {
                    if let Some(m) = note.as_object_mut() {
                        m.remove("hitsound");
                    }
                }
            }
        }
        let mut fs = prpr::fs::fs_from_file(doc.info.parent().unwrap())?;
        let info: prpr::info::ChartInfo = serde_yaml::from_slice(&std::fs::read(&doc.info)?)?;
        pollster::block_on(prpr::parse::parse_rpe(
            &serde_json::to_string(&raw)?,
            fs.as_mut(),
            ChartExtra::default(),
            info.use_rpe_170_speed.unwrap_or_default(),
        ))?
    } else if doc.format == "pbc" {
        let mut r = prpr::bin::BinaryReader::new(std::io::Cursor::new(&doc.bytes));
        r.read()?
    } else {
        prpr::parse::parse_pec(
            std::str::from_utf8(&doc.bytes)
                .context("不支持的谱面编码")?
                .trim_start_matches('\u{feff}'),
            ChartExtra::default(),
        )?
    };
    ensure!(chart.lines.len() <= 10000, "判定线过多");
    let mut notes = Vec::new();
    for (index, line) in chart.lines.iter().enumerate() {
        for n in &line.notes {
            if n.fake {
                continue;
            }
            if notes.len() % 64 == 0 {
                checkpoint()?;
            }
            ensure!(notes.len() < 200_000, "音符过多");
            let (kind, end) = match n.kind {
                NoteKind::Click => (1, n.time),
                NoteKind::Drag => (2, n.time),
                NoteKind::Hold { end_time, .. } => (3, end_time),
                NoteKind::Flick => (4, n.time),
            };
            let (p, rotation) = geometry(&chart.lines, index, n.time, 0)?;
            let (past, past_rotation) = geometry(&chart.lines, index, n.time - 0.15, 0)?;
            let x = value(&n.object.translation.0, n.time) * 0.5;
            let point = |p: [f64; 2], r: f64| {
                let r = r.to_radians();
                [0.5 + p[0] * 0.5 + x * r.cos(), 0.5 / ASPECT + p[1] * 0.5 + x * r.sin()]
            };
            let time = n.time + chart.offset as f64;
            let end = end + chart.offset as f64;
            notes.push(Note {
                line: index,
                side: if n.above { 1 } else { -1 },
                kind,
                time,
                end,
                x: x / 0.05625,
                speed: n.speed,
                line_speed: speed(&line.height, n.time) * HEIGHT_RATIO,
                point: point(p, rotation),
                past: point(past, past_rotation),
                rotation,
                past_rotation,
            });
        }
    }
    features::from_notes(notes, checkpoint)
}
