//! Portable pure-configuration conversion, based on Python converter 2.5.19.
//! This module and its children only depend on anyhow/serde/serde_json/std.
#[path = "formats.rs"]
mod formats;
#[path = "geometry.rs"]
mod geometry;
use anyhow::{bail, ensure, Result};
use geometry::{BpmMap, Geometry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub speed: f32,
    pub hide_seconds: f32,
    pub proximity_fade: bool,
    pub delete_notes: bool,
    pub delete_lines: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            speed: 11.,
            hide_seconds: 3.5,
            proximity_fade: true,
            delete_notes: false,
            delete_lines: false,
        }
    }
}
#[path = "progress.rs"]
mod progress;
pub use progress::{Progress, Stage};
#[path = "io.rs"]
pub mod io;
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub source_notes: usize,
    pub moved: usize,
    pub unresolved: usize,
    pub source_lines: usize,
}
pub fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn number(v: &Value, default: f64) -> Result<f64> {
    let n = if v.is_null() {
        default
    } else {
        v.as_f64().ok_or_else(|| anyhow::anyhow!("需要数字：{v}"))?
    };
    ensure!(n.is_finite(), "数值必须有限");
    Ok(n)
}
pub fn beat(v: &Value) -> Result<f64> {
    if v.is_number() {
        return number(v, 0.);
    }
    let a = array(v);
    ensure!(a.len() == 3, "拍数需要 [整数,分子,分母]");
    let d = number(&a[2], 1.)?;
    ensure!(d != 0., "拍数分母不能为 0");
    Ok(number(&a[0], 0.)? + number(&a[1], 0.)? / d)
}
pub fn triple(v: f64) -> Value {
    // Continued fractions, matching Fraction.limit_denominator(1_000_000).
    // Fixed decimal rounding can move a hard alpha cut to the other side of
    // an exact frame boundary (for example 2/3 -> 666667/1000000).
    let whole = v.floor();
    let f = v - whole;
    let (mut p0, mut q0, mut p1, mut q1) = (0i64, 1i64, 1i64, 0i64);
    let mut x = f;
    for _ in 0..64 {
        let a = x.floor();
        if a * q1 as f64 + q0 as f64 > 1_000_000. {
            break;
        }
        let a = a as i64;
        let (p2, q2) = (p0 + a * p1, q0 + a * q1);
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        let remainder = x - a as f64;
        if remainder == 0. {
            return json!([whole as i64 + p1 / q1, p1 % q1, q1]);
        }
        x = 1. / remainder;
    }
    let k = (1_000_000 - q0) / q1;
    let (a, b) = (p0 + k * p1, q0 + k * q1);
    let (n, d) = if (p1 as f64 / q1 as f64 - f).abs() <= (a as f64 / b as f64 - f).abs() {
        (p1, q1)
    } else {
        (a, b)
    };
    json!([whole as i64 + n / d, n % d, d])
}

pub fn event(a: f64, b: f64, s: f64, e: f64) -> Value {
    json!({"startTime":triple(a),"endTime":triple(b),"start":s,"end":e,"easingType":1,"easingLeft":0,"easingRight":1,"bezier":0,"bezierPoints":[0,0,0,0],"linkgroup":0})
}
fn is_noise_carrier(line: &Value) -> bool {
    line.get("Texture")
        .and_then(Value::as_str)
        .is_some_and(|t| prpr::noise_area::recorder::texture_kind(t).is_some())
}

pub fn static_line(notes: Vec<Value>, speed: f64, end: f64) -> Value {
    json!({"Group":0,"Name":"Pure Config Line","Texture":"line.png","alphaControl":[{"alpha":1,"x":0,"easing":1},{"alpha":1,"x":9999999,"easing":1}],"bpmfactor":1,"eventLayers":[{"alphaEvents":[event(0.,end,255.,255.)],"moveXEvents":[event(0.,end,0.,0.)],"moveYEvents":[event(0.,end,-300.,-300.)],"rotateEvents":[event(0.,end,0.,0.)],"speedEvents":[event(0.,end,speed,speed)]}],"extended":{},"father":-1,"isCover":1,"numOfNotes":notes.len(),"notes":notes,"rotateWithFather":false,"zOrder":999})
}
fn fake(n: &Value) -> bool {
    n["isFake"].as_i64().unwrap_or(0) != 0 || n["isFake"].as_bool().unwrap_or(false)
}
fn time_key(t: f64) -> i64 {
    (t * 1e9).round() as i64
}
#[derive(Clone)]
struct CopyNote {
    note: Value,
    t: f64,
    end: f64,
    kind: i64,
    half: f64,
    x: f64,
    original_x: f64,
    hit_x: f64,
    line: usize,
    index: usize,
    ordinal: usize,
}
fn nearest(preferred: f64, half: f64, blockers: &[&CopyNote]) -> (f64, bool) {
    let left = -675. + half;
    let right = 675. - half;
    if left > right {
        return (0., !blockers.is_empty());
    }
    let preferred = preferred.clamp(left, right);
    let overlaps = |x: f64, b: &CopyNote| (x - b.x).abs() < half + b.half - 1e-7;
    if !blockers.iter().any(|b| overlaps(preferred, b)) {
        return (preferred, false);
    }
    let mut points = vec![left, right, preferred];
    for b in blockers {
        points.push((b.x - half - b.half).clamp(left, right));
        points.push((b.x + half + b.half).clamp(left, right));
    }
    for point in &mut points {
        *point = (*point * 1e9).round() / 1e9;
    }
    points.sort_by(f64::total_cmp);
    points.dedup();
    let valid = points
        .iter()
        .copied()
        .filter(|x| !blockers.iter().any(|b| overlaps(*x, b)))
        .min_by(|a, b| (a - preferred).abs().total_cmp(&(b - preferred).abs()).then(a.total_cmp(b)));
    if let Some(x) = valid {
        return (x, false);
    }
    let overlap = |x: f64| blockers.iter().map(|b| (half + b.half - (x - b.x).abs()).max(0.)).sum::<f64>();
    let x = points
        .into_iter()
        .min_by(|a, b| {
            overlap(*a)
                .total_cmp(&overlap(*b))
                .then((a - preferred).abs().total_cmp(&(b - preferred).abs()))
                .then(a.total_cmp(b))
        })
        .unwrap();
    (x, true)
}
fn hide_gate(line: &mut Value, distance: f64) {
    let distance = distance.max(0.);
    let eps = (distance * 1e-6).clamp(1e-4, 0.01);
    let after = distance + eps;
    let mut old = array(&line["alphaControl"]).to_vec();
    old.sort_by(|a, b| a["x"].as_f64().unwrap_or(0.).total_cmp(&b["x"].as_f64().unwrap_or(0.)));
    let value = |x: f64| {
        if old.is_empty() {
            return 1.;
        }
        let first = &old[0];
        if x <= first["x"].as_f64().unwrap_or(0.) {
            return first["alpha"].as_f64().unwrap_or(1.);
        }
        for p in old.windows(2) {
            let a = p[0]["x"].as_f64().unwrap_or(0.);
            let b = p[1]["x"].as_f64().unwrap_or(0.);
            if x <= b {
                let av = p[0]["alpha"].as_f64().unwrap_or(1.);
                let bv = p[1]["alpha"].as_f64().unwrap_or(1.);
                return if b <= a + 1e-12 { bv } else { av + (bv - av) * (x - a) / (b - a) };
            }
        }
        old.last().unwrap()["alpha"].as_f64().unwrap_or(1.)
    };
    let mut out = vec![
        json!({"x":0,"alpha":0,"easing":1}),
        json!({"x":distance,"alpha":0,"easing":1}),
        json!({"x":after,"alpha":value(after).max(0.),"easing":1}),
    ];
    out.extend(old.iter().filter(|v| v["x"].as_f64().unwrap_or(0.) > after + 1e-9).cloned());
    if out.last().unwrap()["x"].as_f64().unwrap_or(0.) < 9999999. {
        out.push(json!({"x":9999999,"alpha":value(9999999.).max(0.),"easing":1}));
    }
    line["alphaControl"] = Value::Array(out);
}

pub fn convert(source: &[u8], options: &Options, modern_speed: bool, progress: &Progress) -> Result<(Value, Report)> {
    ensure!(options.speed.is_finite() && options.speed > 0., "下落速度必须大于 0");
    ensure!(options.hide_seconds.is_finite() && options.hide_seconds >= 0., "下隐时间必须大于等于 0");
    progress.check()?;
    progress.update(Stage::Read, 0, 0);
    let text = std::str::from_utf8(source)?.trim_start_matches('\u{feff}');
    let parsed = serde_json::from_str::<Value>(text);
    let (events, notes) = parsed.as_ref().ok().map(|v| {
        let lines = array(&v["judgeLineList"]);
        let events = lines.iter().map(|l| {
            ["judgeLineMoveEvents", "judgeLineRotateEvents", "judgeLineDisappearEvents", "speedEvents"].iter().map(|k| array(&l[*k]).len()).sum::<usize>()
            + array(&l["eventLayers"]).iter().map(|layer| ["alphaEvents", "moveXEvents", "moveYEvents", "rotateEvents", "speedEvents"].iter().map(|k| array(&layer[*k]).len()).sum::<usize>()).sum::<usize>()
        }).sum();
        let notes = lines.iter().map(|l| ["notes", "notesAbove", "notesBelow"].iter().map(|k| array(&l[*k]).len()).sum::<usize>()).sum();
        (events, notes)
    }).unwrap_or((text.lines().count(), text.lines().count()));
    progress.plan(source.len(), events, notes, options.proximity_fade && !options.delete_lines);
    progress.finish(Stage::Read);
    progress.update(Stage::Adapt, 0, 1);
    let mut data = match parsed {
        Ok(v) if v.get("META").is_some() && v.get("BPMList").is_some() => v,
        Ok(v) if v["formatVersion"] == 1 || v["formatVersion"] == 3 => formats::pgr_with_progress(&v, progress)?,
        Ok(_) => bail!("支持 RPE、Phigros 1/3 和 PEC 谱面"),
        Err(_) => formats::pec_with_progress(text, progress)?,
    };
    data.as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("谱面需要 JSON 对象"))?
        .remove("blockAreaList");
    // Prediction belongs to original geometry; keep other custom fields intact.
    for key in ["phiraAiDifficulty", "phira_ai_difficulty"] {
        data.as_object_mut().unwrap().remove(key);
    }
    progress.finish(Stage::Adapt);
    progress.update(Stage::Geometry, 0, 1);
    let bpm = BpmMap::new(&data["BPMList"])?;
    let geometry = Geometry::new_with_progress(&data, &bpm, modern_speed, progress)?;
    progress.finish(Stage::Geometry);
    let raw = array(&data["judgeLineList"]);
    ensure!(!raw.is_empty(), "谱面没有判定线");
    let mut report = Report {
        source_lines: raw.len(),
        ..Default::default()
    };
    let mut counts = HashMap::<i64, usize>::new();
    for l in raw {
        if is_noise_carrier(l) {
            continue;
        }
        for n in array(&l["notes"]) {
            if !fake(n) {
                *counts.entry(time_key(bpm.time(beat(&n["startTime"])?))).or_default() += 1;
            }
        }
    }
    let mut copied = vec![];
    let mut end: f64 = 1.;
    let mut end_beat: f64 = 0.;
    let mut ordinal = 0;
    let layout_total = raw.iter().filter(|l| !is_noise_carrier(l)).map(|l| array(&l["notes"]).len() * 2).sum::<usize>();
    let mut layout_done = 0;
    for (li, l) in raw.iter().enumerate() {
        progress.check()?;
        if is_noise_carrier(l) {
            continue;
        }
        progress.update(Stage::Layout, layout_done as u64, layout_total as u64);
        let mut line_notes = vec![];
        for (ni, n) in array(&l["notes"]).iter().enumerate() {
            layout_done += 1;
            if layout_done % 256 == 0 { progress.check()?; progress.update(Stage::Layout, layout_done as u64, layout_total as u64); }
            end = end.max(bpm.time(beat(n.get("endTime").unwrap_or(&n["startTime"]))?) + 3.);
            if fake(n) {
                layout_done += 1;
                continue;
            }
            let t = bpm.time(beat(&n["startTime"])?);
            let tail = bpm.time(beat(n.get("endTime").unwrap_or(&n["startTime"]))?);
            ensure!(tail >= t, "Hold 结束早于开始");
            end_beat = end_beat.max(beat(n.get("endTime").unwrap_or(&n["startTime"]))?);
            let px = number(&n["positionX"], 0.)?;
            let size = number(&n["size"], 1.)?;
            ensure!(size >= 0., "音符大小不能为负");
            let kind = n["type"].as_i64().unwrap_or(1);
            ensure!((1..=4).contains(&kind), "未知音符类型");
            let half = 1350. / 7.4 * size * if counts[&time_key(t)] > 1 { 1.12 } else { 1. } / 2.;
            let (wx, _, r) = geometry.world(li, t);
            let hit_x = wx + px * r.to_radians().cos();
            let mut nn = n.clone();
            nn["isFake"] = json!(0);
            nn["above"] = json!(1);
            nn["speed"] = json!(1);
            nn["visibleTime"] = json!(3);
            nn.as_object_mut().unwrap().remove("_hideBeforeHitSeconds");
            line_notes.push(CopyNote {
                note: nn,
                t,
                end: tail,
                kind,
                half,
                x: px,
                original_x: px,
                hit_x,
                line: li,
                index: ni,
                ordinal,
            });
            ordinal += 1;
        }
        line_notes.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.index.cmp(&b.index)));
        let mut start = 0;
        while start < line_notes.len() {
            let mut stop = start + 1;
            while stop < line_notes.len() && line_notes[stop].t - line_notes[stop - 1].t < 0.5 - 1e-9 {
                stop += 1;
            }
            let seg = &mut line_notes[start..stop];
            let hit_left = seg.iter().map(|n| n.hit_x).fold(f64::INFINITY, f64::min);
            let hit_right = seg.iter().map(|n| n.hit_x).fold(f64::NEG_INFINITY, f64::max);
            let local_left = seg.iter().map(|n| n.original_x).fold(f64::INFINITY, f64::min);
            let local_right = seg.iter().map(|n| n.original_x).fold(f64::NEG_INFINITY, f64::max);
            let base = (hit_left + hit_right - local_left - local_right) / 2.;
            let left = seg.iter().map(|n| base + n.original_x - n.half).fold(f64::INFINITY, f64::min);
            let right = seg.iter().map(|n| base + n.original_x + n.half).fold(f64::NEG_INFINITY, f64::max);
            let shift = if right - left <= 1350. + 1e-7 {
                if 0. < -675. - left {
                    -675. - left
                } else if 0. > 675. - right {
                    675. - right
                } else {
                    0.
                }
            } else {
                0.
            };
            for n in seg {
                layout_done += 1;
                if layout_done % 256 == 0 { progress.check()?; progress.update(Stage::Layout, layout_done as u64, layout_total as u64); }
                let preferred = base + shift + n.original_x;
                n.x = if n.half > 675. {
                    0.
                } else {
                    preferred.clamp(-675. + n.half, 675. - n.half)
                };
            }
            start = stop;
        }
        copied.extend(line_notes);
    }
    progress.finish(Stage::Layout);
    report.source_notes = copied.len();
    copied.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.ordinal.cmp(&b.ordinal)));
    // Keep only currently active blockers, rather than storing a Hold in every 30ms bucket.
    let mut tap_hold: Vec<usize> = vec![];
    let mut flick: VecDeque<usize> = VecDeque::new();
    for i in 0..copied.len() {
        if i % 512 == 0 {
            progress.check()?;
            progress.update(Stage::Avoid, i as u64, copied.len() as u64);
        }
        let t = copied[i].t;
        tap_hold.retain(|j| {
            let n = &copied[*j];
            t - if n.kind == 2 { n.end } else { n.t } < 0.022 - 1e-12
        });
        while flick.front().is_some_and(|j| t - copied[*j].t >= 0.022 - 1e-12) {
            flick.pop_front();
        }
        let n = &copied[i];
        let blockers: Vec<&CopyNote> = if n.kind == 3 {
            flick.iter().map(|j| &copied[*j]).collect()
        } else if n.kind == 4 {
            tap_hold.iter().map(|j| &copied[*j]).filter(|b| t - b.t < 0.010 - 1e-12).collect()
        } else {
            tap_hold.iter().map(|j| &copied[*j]).collect()
        };
        let (nx, bad) = if n.kind == 4 {
            (
                blockers
                    .iter()
                    .min_by(|a, b| (a.x - n.x).abs().total_cmp(&(b.x - n.x).abs()).then(a.ordinal.cmp(&b.ordinal)))
                    .map(|b| b.x)
                    .unwrap_or(n.x),
                false,
            )
        } else {
            nearest(n.x, n.half, &blockers)
        };
        if (nx - n.x).abs() > 1e-7 {
            report.moved += 1;
        }
        if bad {
            report.unresolved += 1;
        }
        copied[i].x = nx;
        copied[i].note["positionX"] = json!((nx * 1e6).round() / 1e6);
        match copied[i].kind {
            1 | 2 => tap_hold.push(i),
            3 => flick.push_back(i),
            _ => {}
        }
    }
    progress.finish(Stage::Avoid);
    let delete_notes = options.delete_notes || options.delete_lines;
    if options.proximity_fade && !options.delete_lines {
        geometry::fade(&mut data, &bpm, &geometry, end, progress)?;
    }
    progress.finish(Stage::Fade);
    progress.update(Stage::Finalize, 0, 1);
    let originals: HashMap<(usize, usize), f64> = copied.iter().map(|n| ((n.line, n.index), n.t)).collect();
    let finalize_total = array(&data["judgeLineList"]).len();
    for (li, l) in data["judgeLineList"].as_array_mut().unwrap().iter_mut().enumerate() {
        progress.check()?;
        progress.update(Stage::Finalize, li as u64, finalize_total as u64);
        if is_noise_carrier(l) {
            // Keep its index/movement/rotation for children, but remove the
            // Recorder protocol even for readers that ignore our metadata.
            l["Texture"] = json!("line.png");
            l["notes"] = json!([]);
            l["numOfNotes"] = json!(0);
            if let Some(layers) = l["eventLayers"].as_array_mut() {
                if !layers.iter().any(Value::is_object) {
                    layers.push(json!({}));
                }
                for layer in layers.iter_mut().filter(|l| l.is_object()) {
                    layer["alphaEvents"] = json!([event(0., (end_beat + 8.).max(4.), 0., 0.)]);
                    layer["speedEvents"] = json!([]);
                }
            }
            if let Some(extended) = l.get_mut("extended").and_then(Value::as_object_mut) {
                for key in ["textEvents", "paintEvents", "gifEvents"] {
                    extended.remove(key);
                }
            }
            continue;
        }
        if delete_notes {
            l["notes"] = json!([]);
            l["numOfNotes"] = json!(0);
            continue;
        }
        if options.hide_seconds > 0. {
            let mut distances = vec![];
            for (ni, n) in array(&l["notes"]).iter().enumerate() {
                if let Some(t) = originals.get(&(li, ni)) {
                    let before = (t - options.hide_seconds as f64).max(0.);
                    let d = ((geometry.lines[li].height(*t) - geometry.lines[li].height(before)) * 450. + number(&n["yOffset"], 0.)?).abs();
                    if d.is_finite() {
                        distances.push(d);
                    }
                }
            }
            if !distances.is_empty() {
                distances.sort_by(f64::total_cmp);
                let k = distances.len() / 2;
                let d = if distances.len() % 2 == 0 {
                    (distances[k - 1] + distances[k]) / 2.
                } else {
                    distances[k]
                };
                hide_gate(l, d);
            }
        }
        if let Some(notes) = l["notes"].as_array_mut() {
            for (ni, n) in notes.iter_mut().enumerate() {
                if originals.contains_key(&(li, ni)) {
                    n["isFake"] = json!(1); /* fake time remains exact: parser uses real-note-only hints for generated charts */
                }
                if let Some(obj) = n.as_object_mut() {
                    obj.remove("_hideBeforeHitSeconds");
                }
            }
        }
    }
    let normal = static_line(copied.into_iter().map(|n| n.note).collect(), options.speed as f64, (end_beat + 8.).max(4.));
    if options.delete_lines {
        data["judgeLineList"] = json!([normal]);
    } else {
        data["judgeLineList"].as_array_mut().unwrap().push(normal);
    }
    data["phiraiadPureConfig"] = json!({"version":1,"referenceVersion":"2.5.19","options":options,"realNotesOnlyHints":true});
    progress.finish(Stage::Finalize);
    Ok((data, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn sample() -> Value {
        json!({"META":{"RPEVersion":160},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],"judgeLineList":[static_line(vec![json!({"type":1,"startTime":[2,0,1],"endTime":[2,0,1],"positionX":0,"isFake":0,"size":1})],11.,20.)],"blockAreaList":[{}]})
    }
    #[test]
    fn recorder_carrier_notes_are_not_copied() {
        let mut source = sample();
        let mut carrier = source["judgeLineList"][0].clone();
        carrier["Texture"] = json!("isSubtract1.png");
        source["judgeLineList"].as_array_mut().unwrap().push(carrier);
        let (out, report) = convert(&serde_json::to_vec(&source).unwrap(), &Options::default(), false, &Progress::default()).unwrap();
        assert_eq!(report.source_notes, 1);
        assert_eq!(
            out["judgeLineList"].as_array().unwrap().last().unwrap()["notes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(out["phiraiadPureConfig"]["realNotesOnlyHints"], true);
        assert_eq!(out["judgeLineList"][1]["Texture"], "line.png");
        assert!(out["judgeLineList"][1]["notes"].as_array().unwrap().is_empty());
        assert_eq!(out["judgeLineList"][1]["eventLayers"][0]["alphaEvents"][0]["start"], 0.);
        assert!(out["judgeLineList"][1]["eventLayers"][0]["speedEvents"].as_array().unwrap().is_empty());
    }
    #[test]
    fn preserves_times_and_removes_noise() {
        let v = sample();
        let (out, r) = convert(&serde_json::to_vec(&v).unwrap(), &Options::default(), false, &Progress::default()).unwrap();
        assert_eq!(r.source_notes, 1);
        assert!(out.get("blockAreaList").is_none());
        assert_eq!(out["judgeLineList"][0]["notes"][0]["isFake"], 1);
        assert_eq!(out["judgeLineList"][0]["notes"][0]["startTime"], v["judgeLineList"][0]["notes"][0]["startTime"]);
        assert_eq!(out["judgeLineList"][1]["notes"][0]["startTime"], v["judgeLineList"][0]["notes"][0]["startTime"]);
    }
    #[test]
    fn deletes_original_lines_and_notes() {
        let o = Options {
            delete_lines: true,
            ..Default::default()
        };
        let (out, _) = convert(&serde_json::to_vec(&sample()).unwrap(), &o, false, &Progress::default()).unwrap();
        assert_eq!(array(&out["judgeLineList"]).len(), 1);
        assert_eq!(out["judgeLineList"][0]["notes"][0]["isFake"], 0);
    }
    #[test]
    fn phigros_ticks_preserve_seconds() {
        for version in [1, 3] {
            let source = json!({"formatVersion":version,"offset":0.2,"judgeLineList":[{"bpm":100,"notesAbove":[{"type":3,"time":32,"holdTime":64,"positionX":0,"speed":1}],"notesBelow":[]}]});
            let (out, _) = convert(&serde_json::to_vec(&source).unwrap(), &Options::default(), false, &Progress::default()).unwrap();
            let bpm = BpmMap::new(&out["BPMList"]).unwrap();
            let note = &out["judgeLineList"][1]["notes"][0];
            assert!((bpm.time(beat(&note["startTime"]).unwrap()) - 0.6).abs() < 1e-12);
            assert!((bpm.time(beat(&note["endTime"]).unwrap()) - 1.8).abs() < 1e-12);
            assert_eq!(out["META"]["offset"].as_f64(), Some(200.));
        }
    }
    #[test]
    fn cancelled_work_fails() {
        let p = Progress::default();
        p.cancelled.store(true, Ordering::Relaxed);
        assert!(convert(b"{}", &Options::default(), false, &p).is_err());
    }
    #[test]
    fn drag_aligns_and_hold_blocks() {
        let mut v = sample();
        v["judgeLineList"][0]["notes"] = json!([{"type":2,"startTime":[2,0,1],"endTime":[4,0,1],"positionX":0},{"type":4,"startTime":[2,1,100],"endTime":[2,1,100],"positionX":10},{"type":1,"startTime":[3,0,1],"endTime":[3,0,1],"positionX":0}]);
        let (out, r) = convert(&serde_json::to_vec(&v).unwrap(), &Options::default(), false, &Progress::default()).unwrap();
        let n = &out["judgeLineList"][1]["notes"];
        assert_eq!(n[0]["positionX"], n[1]["positionX"]);
        assert_ne!(n[0]["positionX"], n[2]["positionX"]);
        assert_eq!(r.unresolved, 0);
    }
}
