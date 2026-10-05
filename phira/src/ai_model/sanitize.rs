//! Prediction-only repair. Never rewrites gameplay chart data.
use super::features::{self, number, Features, Note};
use anyhow::{ensure, Result};
use serde_json::{json, Value};

pub fn beat(v: &Value) -> Option<f64> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let d = number(&a[2])?;
    if d == 0. {
        return None;
    }
    let b = number(&a[0])? + number(&a[1])? / d;
    b.is_finite().then_some(b)
}
fn triple(v: &Value) -> Option<Value> {
    beat(v)?;
    let a = v.as_array()?;
    let mut out = Vec::new();
    for x in a {
        let n = number(x)?;
        if n.fract() != 0. || n.abs() > 1_000_000. {
            return None;
        }
        out.push(n as i64);
    }
    Some(json!(out))
}
fn field(v: &mut Value, k: &str, default: f64) {
    v[k] = json!(number(&v[k]).unwrap_or(default).clamp(-1_000_000., 1_000_000.));
}
pub fn rpe(raw: &Value) -> Value {
    let mut raw = raw.clone();
    if !raw["META"].is_object() {
        raw["META"] = json!({});
    }
    raw["META"]["offset"] = json!(number(&raw["META"]["offset"]).unwrap_or(0.).clamp(-1e6, 1e6) as i32);
    let mut bpm = Vec::new();
    if let Some(items) = raw["BPMList"].as_array() {
        for e in items {
            if let (Some(t), Some(b)) = (triple(&e["startTime"]), number(&e["bpm"]).filter(|b| *b > 0.)) {
                bpm.push(json!({"startTime":t,"bpm":b.clamp(0.001,1e6)}));
            }
        }
    }
    bpm.sort_by(|a, b| beat(&a["startTime"]).unwrap().total_cmp(&beat(&b["startTime"]).unwrap()));
    if bpm.is_empty() {
        bpm.push(json!({"startTime":[0,0,1],"bpm":120}));
    }
    raw["BPMList"] = json!(bpm);
    let source = raw["judgeLineList"].as_array().cloned().unwrap_or_default();
    let count = source.len();
    let mut lines = Vec::new();
    for mut line in source.into_iter().take(10000) {
        if !line.is_object() {
            line = json!({});
        }
        if !line["Name"].is_string() {
            line["Name"] = json!("");
        }
        line["Texture"] = json!("line.png");
        line["isCover"] = json!(number(&line["isCover"]).unwrap_or(0.).clamp(0., 1.) as u8);
        line["zOrder"] = json!(number(&line["zOrder"]).unwrap_or(0.).clamp(-1e6, 1e6) as i32);
        line["father"] = json!(number(&line["father"])
            .filter(|p| p.fract() == 0. && *p >= 0. && *p < count as f64)
            .map(|p| p as i64)
            .unwrap_or(-1));
        line["rotateWithFather"] = json!(line["rotateWithFather"].as_bool().unwrap_or(false));
        for k in ["attachUI", "posControl", "sizeControl", "alphaControl", "yControl", "extended"] {
            line.as_object_mut().unwrap().remove(k);
        }
        let mut layers = Vec::new();
        if let Some(src) = line["eventLayers"].as_array() {
            for layer in src.iter().filter(|l| l.is_object()) {
                let mut layer = layer.clone();
                layer.as_object_mut().unwrap().remove("alphaEvents");
                for k in ["moveXEvents", "moveYEvents", "rotateEvents", "speedEvents"] {
                    let mut events = Vec::new();
                    if let Some(src) = layer[k].as_array() {
                        for e in src {
                            let (Some(a), Some(b)) = (triple(&e["startTime"]), triple(&e["endTime"])) else {
                                continue;
                            };
                            if beat(&b) < beat(&a) {
                                continue;
                            }
                            let mut e = e.clone();
                            e["startTime"] = a;
                            e["endTime"] = b;
                            field(&mut e, "start", if k == "speedEvents" { 1. } else { 0. });
                            let start = number(&e["start"]).unwrap();
                            field(&mut e, "end", start);
                            e["easingType"] = json!(number(&e["easingType"]).unwrap_or(1.).clamp(1., 28.) as i32);
                            field(&mut e, "easingLeft", 0.);
                            field(&mut e, "easingRight", 1.);
                            // Invalid optional Bezier data must not poison the entire chart.
                            if e["bezierPoints"]
                                .as_array()
                                .is_none_or(|a| a.len() != 4 || a.iter().any(|v| number(v).is_none()))
                            {
                                e.as_object_mut().unwrap().remove("bezierPoints");
                                e.as_object_mut().unwrap().remove("bezier");
                            }
                            events.push(e);
                        }
                    }
                    events.sort_by(|a, b| beat(&a["startTime"]).unwrap().total_cmp(&beat(&b["startTime"]).unwrap()));
                    layer[k] = json!(events);
                }
                layers.push(layer);
            }
        }
        line["eventLayers"] = json!(layers);
        let mut notes = Vec::new();
        if let Some(src) = line["notes"].as_array() {
            for n in src {
                let Some(kind) = number(&n["type"]).filter(|v| v.fract() == 0. && (1. ..=4.).contains(v)) else {
                    continue;
                };
                let Some(a) = triple(&n["startTime"]) else {
                    continue;
                };
                if n["isFake"].as_bool() == Some(true) || number(&n["isFake"]).is_some_and(|v| v != 0.) {
                    continue;
                }
                let b = triple(&n["endTime"]).filter(|b| beat(b) >= beat(&a)).unwrap_or_else(|| a.clone());
                let mut n = n.clone();
                n["type"] = json!(kind as u8);
                n["startTime"] = a;
                n["endTime"] = b;
                n["above"] = json!(number(&n["above"]).unwrap_or(1.).clamp(0., 1.) as u8);
                n["isFake"] = json!(0);
                for (k, d) in [("positionX", 0.), ("yOffset", 0.), ("size", 1.), ("speed", 1.), ("visibleTime", 999.)] {
                    field(&mut n, k, d);
                }
                n["alpha"] = json!(number(&n["alpha"]).unwrap_or(255.).clamp(0., 65535.) as u16);
                for k in ["hitsound", "tint", "tintHitEffects", "judgeArea"] {
                    n.as_object_mut().unwrap().remove(k);
                }
                notes.push(n);
            }
        }
        line["notes"] = json!(notes);
        lines.push(line);
    }
    // Break each cycle and excessively long chain before the parser sees it.
    for i in 0..lines.len() {
        let mut seen = vec![i];
        let mut at = i;
        while let Some(p) = lines[at]["father"].as_i64().filter(|p| *p >= 0).map(|p| p as usize) {
            if p >= lines.len() || seen.contains(&p) || seen.len() >= 128 {
                lines[at]["father"] = json!(-1);
                break;
            }
            seen.push(p);
            at = p;
        }
    }
    raw["judgeLineList"] = json!(lines);
    raw
}
fn bpm_time(b: f64, list: &[(f64, f64)]) -> f64 {
    let mut t = 0.;
    let mut cursor = 0.;
    let mut bpm = list.first().map_or(120., |v| v.1);
    for &(at, next) in list {
        if at > b {
            break;
        }
        t += (at - cursor) * 60. / bpm;
        cursor = at;
        bpm = next;
    }
    features::finite(t + (b - cursor) * 60. / bpm, 0.)
}
fn stationary(line: usize, side: i32, kind: usize, time: f64, end: f64, x: f64, speed: f64) -> Note {
    let p = [0.5 + x, 0.5 / 1.777778];
    Note {
        line,
        side,
        kind,
        time,
        end,
        x: x / 0.05625,
        speed,
        line_speed: 1.,
        point: p,
        past: p,
        rotation: 0.,
        past_rotation: 0.,
    }
}
/// Last resort: keep valid notes/BPM, ignore malformed line animation.
pub fn rpe_notes(raw: &Value, check: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    let raw = rpe(raw);
    let bpm = raw["BPMList"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (beat(&e["startTime"]).unwrap(), number(&e["bpm"]).unwrap()))
        .collect::<Vec<_>>();
    let offset = number(&raw["META"]["offset"]).unwrap_or(0.) / 1000.;
    let mut notes = Vec::new();
    for (i, line) in raw["judgeLineList"].as_array().unwrap().iter().enumerate() {
        check()?;
        for n in line["notes"].as_array().unwrap() {
            let kind = match n["type"].as_u64().unwrap() {
                1 => 1,
                2 => 3,
                3 => 4,
                _ => 2,
            };
            let time = bpm_time(beat(&n["startTime"]).unwrap(), &bpm) + offset;
            let end = if kind == 3 {
                bpm_time(beat(&n["endTime"]).unwrap(), &bpm) + offset
            } else {
                time
            };
            notes.push(stationary(
                i,
                if n["above"] == 0 { -1 } else { 1 },
                kind,
                time,
                end,
                number(&n["positionX"]).unwrap() / 1350.,
                number(&n["speed"]).unwrap(),
            ));
            ensure!(notes.len() <= 200_000, "音符过多");
        }
    }
    features::from_notes(notes, check)
}
/// PEC fallback deliberately ignores animation, unknown commands and orphan modifiers.
pub fn pec_notes(text: &str, check: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    ensure!(
        text.lines().next().and_then(|s| s.trim().parse::<f64>().ok()).is_some()
            || text
                .lines()
                .any(|s| matches!(s.split_whitespace().next(), Some("bp" | "n1" | "n2" | "n3" | "n4"))),
        "无法识别 PEC 谱面"
    );
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let offset = lines
        .next()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.)
        / 1000.
        - 0.15;
    let rows = lines.map(|s| s.split_whitespace().collect::<Vec<_>>()).collect::<Vec<_>>();
    let num = |s: Option<&&str>| s.and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite());
    let mut bpm = rows
        .iter()
        .filter(|r| r.first() == Some(&"bp"))
        .filter_map(|r| Some((num(r.get(1))?, num(r.get(2)).filter(|v| *v > 0.).unwrap_or(120.))))
        .collect::<Vec<_>>();
    bpm.sort_by(|a, b| a.0.total_cmp(&b.0));
    if bpm.is_empty() {
        bpm.push((0., 120.));
    }
    let mut notes: Vec<Note> = Vec::new();
    let mut previous: Option<usize> = None;
    for (ri, r) in rows.iter().enumerate() {
        if ri % 128 == 0 {
            check()?;
        }
        let Some(cmd) = r.first() else {
            continue;
        };
        if *cmd == "#" {
            if let Some(i) = previous {
                notes[i].speed = num(r.get(1)).unwrap_or(1.);
            }
            continue;
        }
        if !["n1", "n2", "n3", "n4"].contains(cmd) {
            previous = None;
            continue;
        }
        previous = None;
        let Some(line) = num(r.get(1)).filter(|v| v.fract() == 0. && *v >= 0. && *v < 10000.) else {
            continue;
        };
        let Some(start) = num(r.get(2)) else {
            continue;
        };
        let hold = *cmd == "n2";
        let pos = if hold { 4 } else { 3 };
        if num(r.get(pos + 2)).is_some_and(|v| v != 0.) {
            continue;
        }
        let kind = match *cmd {
            "n1" => 1,
            "n2" => 3,
            "n3" => 4,
            _ => 2,
        };
        let time = bpm_time(start, &bpm) + offset;
        let end = if hold {
            bpm_time(num(r.get(3)).unwrap_or(start).max(start), &bpm) + offset
        } else {
            time
        };
        let speed = r.iter().position(|s| *s == "#").and_then(|i| num(r.get(i + 1))).unwrap_or(1.);
        notes.push(stationary(
            line as usize,
            if num(r.get(pos + 1)).unwrap_or(1.) == 1. { 1 } else { -1 },
            kind,
            time,
            end,
            num(r.get(pos)).unwrap_or(0.) / 2048.,
            speed,
        ));
        previous = Some(notes.len() - 1);
        ensure!(notes.len() <= 200_000, "音符过多");
    }
    features::from_notes(notes, check)
}

/// Filter malformed local commands before the parser can allocate an invalid line ID.
pub fn pec(text: &str) -> String {
    let mut rows = text.trim_start_matches('\u{feff}').lines();
    let header = rows.next().unwrap_or("0");
    let number = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite());
    let mut output = number(header.trim()).unwrap_or(0.).to_string();
    output.push('\n');
    for row in rows {
        let mut words = row.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let Some(cmd) = words.first().cloned() else {
            continue;
        };
        let count = match cmd.as_str() {
            "bp" => 3,
            "n1" | "n3" | "n4" => 6,
            "n2" => 7,
            "cv" | "cd" | "ca" => 4,
            "cp" => 5,
            "cm" => 7,
            "cr" => 6,
            "cf" => 5,
            "#" | "&" => 2,
            _ => continue,
        };
        if words.len() < count {
            continue;
        }
        if !matches!(cmd.as_str(), "bp" | "#" | "&") {
            if !number(&words[1]).is_some_and(|n| n.fract() == 0. && n >= 0. && n < 10000.) || number(&words[2]).is_none() {
                continue;
            }
        }
        for (i, word) in words.iter_mut().enumerate().take(count).skip(1) {
            if number(word).is_none() {
                *word = if matches!(cmd.as_str(), "cv" | "#" | "&") || (cmd == "bp" && i == 2) {
                    "1".into()
                } else {
                    "0".into()
                };
            }
        }
        if cmd == "bp" && number(&words[2]).unwrap() <= 0. {
            words[2] = "120".into();
        }
        if matches!(cmd.as_str(), "cm" | "cr" | "cf") && number(&words[3]) < number(&words[2]) {
            continue;
        }
        if cmd == "n2" && number(&words[3]) < number(&words[2]) {
            words[3] = words[2].clone();
        }
        // Keep valid inline modifiers intact; invalid ones are handled by the fallback.
        output.push_str(&words.join(" "));
        output.push('\n');
    }
    let mut rows = output.lines();
    let header = rows.next().unwrap_or("0");
    let body = rows.map(str::to_owned).collect::<Vec<_>>();
    let mut bp = body.iter().filter(|s| s.starts_with("bp ")).cloned().collect::<Vec<_>>();
    bp.sort_by(|a, b| {
        number(a.split_whitespace().nth(1).unwrap())
            .unwrap()
            .total_cmp(&number(b.split_whitespace().nth(1).unwrap()).unwrap())
    });
    if bp.is_empty() {
        bp.push("bp 0 120".into());
    }
    let mut first = std::collections::BTreeMap::<usize, [Option<bool>; 4]>::new();
    for row in &body {
        let r = row.split_whitespace().collect::<Vec<_>>();
        if matches!(r[0], "bp" | "#" | "&") {
            continue;
        }
        let id = number(r[1]).unwrap() as usize;
        let state = first.entry(id).or_insert([None; 4]);
        let cat = match r[0] {
            "cp" | "cm" => Some(0),
            "cd" | "cr" => Some(1),
            "ca" | "cf" => Some(2),
            "cv" => Some(3),
            _ => None,
        };
        if let Some(cat) = cat {
            state[cat].get_or_insert(matches!(r[0], "cp" | "cd" | "ca" | "cv"));
        }
    }
    let mut repaired = format!("{header}\n{}\n", bp.join("\n"));
    if let Some(&max) = first.keys().next_back() {
        for id in 0..=max {
            let state = first.get(&id).copied().unwrap_or([None; 4]);
            for (cat, command) in [
                format!("cp {id} 0 1024 700"),
                format!("cd {id} 0 0"),
                format!("ca {id} 0 255"),
                format!("cv {id} 0 7"),
            ]
            .into_iter()
            .enumerate()
            {
                if state[cat] != Some(true) {
                    repaired.push_str(&command);
                    repaired.push('\n');
                }
            }
        }
    }
    for row in body.into_iter().filter(|s| !s.starts_with("bp ")) {
        repaired.push_str(&row);
        repaired.push('\n');
    }
    repaired
}
