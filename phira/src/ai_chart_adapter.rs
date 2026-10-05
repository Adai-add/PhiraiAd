//! Convert non-format3 charts through Phira's own parser, without loading graphics/audio.
//! Sampling keeps the training coordinate system, never the device aspect or gameplay modifiers.
use crate::ai_model::{
    cache::Document,
    features::{self, Features, Note},
    sanitize,
};
use anyhow::{ensure, Context, Result};
use prpr::core::{AnimFloat, Chart, ChartExtra, JudgeLine, NoteKind, HEIGHT_RATIO};
const ASPECT: f64 = 1.777778;
fn value(a: &AnimFloat, t: f64) -> f64 {
    a.value_at(t).unwrap_or(0.) as f64
}
fn geometry(lines: &[JudgeLine], index: usize, t: f64, _: usize) -> Result<([f64; 2], f64)> {
    let mut chain = Vec::new();
    let mut at = index;
    while at < lines.len() && !chain.contains(&at) && chain.len() < 128 {
        chain.push(at);
        match lines[at].parent {
            Some(p) => at = p,
            None => break,
        }
    }
    let mut p = [0., 0.];
    let mut rotation: f64 = 0.;
    for i in chain.into_iter().rev() {
        let line = &lines[i];
        let local = [
            features::finite(value(&line.object.translation.0, t), 0.),
            features::finite(value(&line.object.translation.1, t) / ASPECT, 0.),
        ];
        let r = rotation.to_radians();
        p = [
            p[0] + local[0] * r.cos() - local[1] * r.sin(),
            p[1] + local[0] * r.sin() + local[1] * r.cos(),
        ];
        let own = features::finite(value(&line.object.rotation, t), 0.);
        rotation = if line.rot_with_parent { rotation + own } else { own };
    }
    Ok((p, rotation))
}
fn speed(a: &AnimFloat, t: f64) -> f64 {
    speed_at(a, t, 0)
}
fn speed_at(a: &AnimFloat, t: f64, depth: usize) -> f64 {
    if depth >= 128 {
        return 0.;
    }
    let i = a.keyframes.partition_point(|k| k.time < t).saturating_sub(1);
    let own = if let (Some(k), Some(next)) = (a.keyframes.get(i), a.keyframes.get(i + 1)) {
        let dt = next.time - k.time;
        if !dt.is_finite() || dt <= 0. {
            0.
        } else {
            let u = ((t - k.time) / dt).clamp(0., 1.);
            let lo = (u - 0.001).max(0.);
            let hi = (u + 0.001).min(1.);
            features::finite((next.value - k.value) as f64 / dt * (k.tween.y(hi as f32) - k.tween.y(lo as f32)) as f64 / (hi - lo), 0.)
        }
    } else {
        0.
    };
    features::finite(own + a.next.as_ref().map_or(0., |n| speed_at(n, t, depth + 1)), 0.)
}
pub fn extract(doc: &Document, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    if let Some(raw) = &doc.raw {
        if features::number(&raw["formatVersion"]) == Some(3.) {
            return features::extract(raw, checkpoint);
        }
        if features::number(&raw["formatVersion"]) == Some(1.) {
            let mut raw = raw.clone();
            raw["formatVersion"] = 3.into();
            for line in raw["judgeLineList"].as_array_mut().into_iter().flatten() {
                if !line.is_object() {
                    continue;
                }
                for e in line["judgeLineMoveEvents"].as_array_mut().into_iter().flatten() {
                    for (a, b) in [("start", "start2"), ("end", "end2")] {
                        if !e.is_object() {
                            continue;
                        }
                        let v = features::number(&e[a]).unwrap_or(440260.);
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
        ensure!(raw["judgeLineList"].as_array().is_none_or(|a| a.len() <= 10000), "判定线过多");
        let raw = sanitize::rpe(raw);
        let mut fs = prpr::fs::fs_from_file(doc.info.parent().unwrap())?;
        let info: prpr::info::ChartInfo = serde_yaml::from_slice(&std::fs::read(&doc.info)?)?;
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pollster::block_on(prpr::parse::parse_rpe(
                &serde_json::to_string(&raw).unwrap(),
                fs.as_mut(),
                ChartExtra::default(),
                info.use_rpe_170_speed.unwrap_or_default(),
            ))
        }));
        match parsed {
            Ok(Ok(chart)) => chart,
            error => {
                let reason = match error {
                    Ok(Err(e)) => e.to_string(),
                    _ => "parser panic".into(),
                };
                tracing::warn!("AI RPE animation fallback: {reason}");
                return sanitize::rpe_notes(&raw, checkpoint);
            }
        }
    } else if doc.format == "pbc" {
        let mut r = prpr::bin::BinaryReader::new(std::io::Cursor::new(&doc.bytes));
        r.read()?
    } else {
        let text = std::str::from_utf8(&doc.bytes)
            .context("不支持的谱面编码")?
            .trim_start_matches('\u{feff}');
        let repaired = sanitize::pec(text);
        if !repaired
            .lines()
            .any(|s| matches!(s.split_whitespace().next(), Some("n1" | "n2" | "n3" | "n4")))
        {
            return sanitize::pec_notes(text, checkpoint);
        }
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prpr::parse::parse_pec(&repaired, ChartExtra::default())));
        match parsed {
            Ok(Ok(chart)) => chart,
            error => {
                let reason = match error {
                    Ok(Err(e)) => e.to_string(),
                    _ => "parser panic".into(),
                };
                tracing::warn!("AI PEC animation fallback: {reason}");
                return sanitize::pec_notes(text, checkpoint);
            }
        }
    };
    ensure!(chart.lines.len() <= 10000, "判定线过多");
    let mut notes = Vec::new();
    for (index, line) in chart.lines.iter().enumerate() {
        for n in &line.notes {
            if n.fake || !n.time.is_finite() {
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
