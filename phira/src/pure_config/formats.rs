//! Source adapters. RPE is kept as raw JSON to preserve unrelated fields.
use super::geometry::BpmMap;
use super::{array, beat, event, number, static_line, triple, Progress, Stage};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn root(lines: Vec<Value>, offset: f64) -> Result<Value> {
    let millis = (offset * 1000.).round();
    ensure!(millis.is_finite() && millis >= i32::MIN as f64 && millis <= i32::MAX as f64, "偏移量超出 RPE 毫秒范围");
    Ok(json!({"META":{"RPEVersion":160,"name":"","song":"","background":"","offset":millis as i32},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],"judgeLineGroup":["Default"],"judgeLineList":lines}))
}

/// Port of 2.5.19's _simplify_linear_segments. Reuse the fade sampler's
/// error-bounded simplifier, but never merge across gaps or discontinuities.
fn simplify_segments(mut segments: Vec<(f64, f64, f64, f64)>, tolerance: f64) -> Vec<Value> {
    segments.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out = vec![];
    let mut run: Vec<(f64, f64)> = vec![];
    let flush = |run: &mut Vec<(f64, f64)>, out: &mut Vec<Value>| {
        for pair in super::geometry::simplify(run, tolerance).windows(2) {
            out.push(event(pair[0].0, pair[1].0, pair[0].1, pair[1].1));
        }
        run.clear();
    };
    for (mut a, mut b, mut s, mut e) in segments {
        if b < a {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut s, &mut e);
        }
        if (b - a).abs() < 1e-12 {
            flush(&mut run, &mut out);
            out.push(event(a, b, s, e));
            continue;
        }
        if let Some(&(t, v)) = run.last() {
            if (a - t).abs() > 1e-9 || (s - v).abs() > 1e-8_f64.max(tolerance * 0.05) {
                flush(&mut run, &mut out);
            }
        }
        if run.is_empty() {
            run.push((a, s));
        }
        run.push((b, e));
    }
    flush(&mut run, &mut out);
    out
}
fn compact(es: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    for e in es {
        if let Some(last) = out.last_mut() {
            if last["endTime"] == e["startTime"] && last["start"] == last["end"] && e["start"] == e["end"] && last["end"] == e["start"] {
                last["endTime"] = e["endTime"].clone();
                continue;
            }
        }
        out.push(e);
    }
    out
}
#[cfg(test)]
pub fn pgr(data: &Value) -> Result<Value> { pgr_with_progress(data, &Progress::default()) }
pub fn pgr_with_progress(data: &Value, progress: &Progress) -> Result<Value> {
    let version = data["formatVersion"].as_i64().unwrap_or(0);
    ensure!(version == 1 || version == 3, "不支持的 Phigros 格式");
    let lines = array(&data["judgeLineList"]);
    ensure!(!lines.is_empty(), "谱面没有判定线");
    let mut max: f64 = 0.;
    for l in lines {
        let bpm = number(&l["bpm"], 0.)?;
        ensure!(bpm > 0., "BPM 必须大于零");
        for key in ["notesAbove", "notesBelow"] {
            for n in array(&l[key]) {
                let t = number(&n["time"], 0.)? + if n["type"] == 3 { number(&n["holdTime"], 0.)? } else { 0. };
                max = max.max(t * 60. / (bpm * 32.));
            }
        }
    }
    let end = max + 5.;
    let total = lines.iter().map(|l| array(&l["judgeLineMoveEvents"]).len() * 2
        + ["judgeLineRotateEvents", "judgeLineDisappearEvents", "speedEvents", "notesAbove", "notesBelow"].iter().map(|k| array(&l[*k]).len()).sum::<usize>()).sum::<usize>();
    let done = std::cell::Cell::new(0u64);
    let step = || -> Result<()> {
        let n = done.get() + 1; done.set(n);
        if n % 512 == 0 { progress.check()?; progress.update(Stage::Adapt, n, total as u64); }
        Ok(())
    };
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        progress.check()?;
        progress.update(Stage::Adapt, done.get(), total as u64);
        let bpm = number(&l["bpm"], 0.)?;
        // Phigros has 32 ticks per source beat; target RPE uses 120 BPM.
        // Match the reference's operation order as well as its units; tiny
        // rounding differences can choose different vertices during compaction.
        let to_beat = |t: f64| t * 60. / (bpm * 32.) * 120. / 60.;
        let limit = end * bpm * 32. / 60.;
        let scalar = |key: &str, component: usize, transform: fn(f64) -> f64, tolerance: f64| -> Result<Vec<Value>> {
            let mut events = vec![];
            for e in array(&l[key]) {
                step()?;
                let a = number(&e["startTime"], 0.)?;
                let b = number(&e["endTime"], a)?.min(limit);
                if a > limit || b < a {
                    continue;
                }
                let (mut s, mut z) = if component == 1 {
                    (number(&e["start"], 0.)?, number(&e["end"], 0.)?)
                } else {
                    (number(&e["start2"], 0.)?, number(&e["end2"], 0.)?)
                };
                if version == 1 && key == "judgeLineMoveEvents" {
                    let decode = |v: f64, c: usize| {
                        let rem = v % 1000.;
                        if c == 1 {
                            (v - rem) / 1000. / 880.
                        } else {
                            rem / 520.
                        }
                    };
                    s = decode(number(&e["start"], 0.)?, component);
                    z = decode(number(&e["end"], 0.)?, component);
                }
                events.push((to_beat(a), to_beat(b), transform(s), transform(z)));
            }
            Ok(simplify_segments(events, tolerance))
        };
        let xs = scalar("judgeLineMoveEvents", 1, |v| (v - 0.5) * 1350., 0.50)?;
        let ys = scalar("judgeLineMoveEvents", 2, |v| (v - 0.5) * 900., 0.50)?;
        let rots = scalar("judgeLineRotateEvents", 1, |v| -v, 0.05)?;
        let mut alpha = scalar("judgeLineDisappearEvents", 1, |v| v * 255., 0.75)?;
        let mut speeds = vec![];
        for e in array(&l["speedEvents"]) {
            step()?;
            let a = number(&e["startTime"], 0.)?;
            let b = number(&e["endTime"], a)?.min(limit);
            if a <= limit && b >= a {
                let v = number(&e["value"], 1.)? * 4.5;
                speeds.push(event(to_beat(a), to_beat(b), v, v));
            }
        }
        let speeds = compact(speeds);
        let speeds = if speeds.is_empty() {
            vec![event(0., end * 2., 4.5, 4.5)]
        } else {
            speeds
        };
        if alpha.is_empty() {
            alpha.push(event(0., end * 2., 255., 255.));
        }
        let mut notes = vec![];
        for (key, above) in [("notesAbove", 1), ("notesBelow", 0)] {
            for n in array(&l[key]) {
                step()?;
                let k = n["type"].as_i64().unwrap_or(0);
                let kind = match k {
                    1 => 1,
                    2 => 4,
                    3 => 2,
                    4 => 3,
                    _ => bail!("未知音符类型"),
                };
                let st = number(&n["time"], 0.)?;
                let et = st + if k == 3 { number(&n["holdTime"], 0.)? } else { 0. };
                ensure!(et >= st, "Hold 结束早于开始");
                notes.push(json!({"above":above,"alpha":255,"endTime":triple(to_beat(et)),"isFake":0,"positionX":number(&n["positionX"],0.)?*75.9375,"size":1,"speed":number(&n["speed"],1.)?,"startTime":triple(to_beat(st)),"type":kind,"visibleTime":999999,"yOffset":0}));
            }
        }
        let mut line = static_line(notes, 4.5, (end * 2.).max(4.));
        line["Name"] = json!(format!("Phigros Line {i}"));
        line["zOrder"] = json!(0);
        line["eventLayers"] = json!([{"alphaEvents":alpha,"moveXEvents":xs,"moveYEvents":ys,"rotateEvents":rots,"speedEvents":speeds}]);
        out.push(line);
    }
    root(out, number(&data["offset"], 0.)?)
}
#[derive(Default)]
struct PecLine {
    notes: Vec<Value>,
    events: [Vec<(f64, f64, f64, i64)>; 4],
    speeds: Vec<(f64, f64)>,
}
#[cfg(test)]
pub fn pec(text: &str) -> Result<Value> { pec_with_progress(text, &Progress::default()) }
pub fn pec_with_progress(text: &str, progress: &Progress) -> Result<Value> {
    let rows = text.lines().map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>();
    ensure!(!rows.is_empty(), "PEC 内容为空");
    let offset = rows[0].parse::<f64>().context("PEC 第一行需要毫秒偏移")? / 1000. - 0.15;
    ensure!(offset.is_finite(), "偏移必须有限");
    let mut bpms = vec![];
    let mut commands = vec![];
    let mut started = false;
    for (i, row) in rows.iter().enumerate().skip(1) {
        let p = row.split_whitespace().collect::<Vec<_>>();
        if p[0] == "bp" {
            ensure!(!started && p.len() == 3, "PEC 第 {} 行 BPM 无效", i + 1);
            bpms.push(json!({"startTime":p[1].parse::<f64>()?,"bpm":p[2].parse::<f64>()?}));
        } else {
            started = true;
            commands.push((i + 1, p));
        }
    }
    let bpm = BpmMap::new(&Value::Array(bpms))?;
    let mut lines = BTreeMap::<usize, PecLine>::new();
    let mut last: Option<(usize, usize)> = None;
    let command_count = commands.len();
    for (ci, (row, p)) in commands.into_iter().enumerate() {
        if ci % 256 == 0 { progress.check()?; progress.update(Stage::Adapt, ci as u64, command_count as u64); }
        let result = (|| -> Result<()> {
            let cmd = p[0];
            if cmd == "#" || cmd == "&" {
                ensure!(p.len() == 2, "附加参数数量错误");
                let (li, ni) = last.context("尚无音符")?;
                let v = p[1].parse::<f64>()?;
                ensure!(v.is_finite(), "音符参数必须有限");
                lines.get_mut(&li).unwrap().notes[ni][if cmd == "#" { "speed" } else { "size" }] = json!(v);
                return Ok(());
            }
            let nums = p[1..].iter().map(|v| v.parse::<f64>()).collect::<std::result::Result<Vec<_>, _>>();
            let note_cmd = matches!(cmd, "n1" | "n2" | "n3" | "n4");
            if note_cmd {
                let count = if cmd == "n2" { 6 } else { 5 };
                ensure!(p.len() >= count + 1, "音符参数不足");
                let n = p[1..=count]
                    .iter()
                    .map(|v| v.parse::<f64>())
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                ensure!(n.iter().all(|v| v.is_finite()), "音符参数必须有限");
                let li = n[0] as usize;
                ensure!(n[0] >= 0. && n[0].fract() == 0. && li <= 10000, "判定线编号无效");
                let head = bpm.time(n[1]);
                let tail = if cmd == "n2" { bpm.time(n[2]) } else { head };
                ensure!(tail >= head, "Hold 结束早于开始");
                let j = if cmd == "n2" { 3 } else { 2 };
                ensure!(n[j + 2] == 0. || n[j + 2] == 1., "假音符标记无效");
                let mut note = json!({"above":n[j+1]as i64,"alpha":255,"endTime":triple(tail*2.),"isFake":n[j+2]as i64,"positionX":n[j]/2048.*1350.,"size":1,"speed":1,"startTime":triple(head*2.),"type":match cmd{"n1"=>1,"n2"=>2,"n3"=>3,_=>4},"visibleTime":999999,"yOffset":0});
                let rest = &p[count + 1..];
                ensure!(rest.len() % 2 == 0, "音符附加参数不足");
                for pair in rest.chunks(2) {
                    ensure!(pair[0] == "#" || pair[0] == "&", "未知附加参数");
                    let v = pair[1].parse::<f64>()?;
                    ensure!(v.is_finite(), "音符参数必须有限");
                    note[if pair[0] == "#" { "speed" } else { "size" }] = json!(v);
                }
                let line = lines.entry(li).or_default();
                last = Some((li, line.notes.len()));
                line.notes.push(note);
                return Ok(());
            }
            let n = nums?;
            ensure!(n.iter().all(|v| v.is_finite()), "事件参数必须有限");
            let expected = match cmd {
                "cv" | "cd" | "ca" => 3,
                "cp" | "cf" => 4,
                "cr" => 5,
                "cm" => 6,
                _ => bail!("未知 PEC 指令 {cmd}"),
            };
            ensure!(n.len() == expected, "指令参数数量错误");
            ensure!(n[0] >= 0. && n[0].fract() == 0. && n[0] <= 10000., "判定线编号无效");
            let line = lines.entry(n[0] as usize).or_default();
            let a = bpm.time(n[1]);
            match cmd {
                "cv" => line.speeds.push((a, n[2] * 0.83175 / 5.85 * 4.5)),
                "cp" => {
                    line.events[0].push((a, a, n[2], -1));
                    line.events[1].push((a, a, n[3], -1));
                }
                "cd" => line.events[2].push((a, a, n[2], -1)),
                "ca" => line.events[3].push((a, a, n[2], -1)),
                "cm" => {
                    let b = bpm.time(n[2]);
                    line.events[0].push((a, b, n[3], n[5] as i64));
                    line.events[1].push((a, b, n[4], n[5] as i64));
                }
                "cr" => line.events[2].push((a, bpm.time(n[2]), n[3], n[4] as i64)),
                "cf" => line.events[3].push((a, bpm.time(n[2]), n[3], 1)),
                _ => {}
            }
            Ok(())
        })();
        result.with_context(|| format!("PEC 第 {row} 行"))?;
    }
    ensure!(!lines.is_empty(), "PEC 谱面没有判定线");
    let max_index = *lines.keys().last().unwrap();
    let mut end: f64 = 4.;
    for l in lines.values() {
        for n in &l.notes {
            end = end.max(beat(&n["endTime"])? / 2. + 5.);
        }
        for evs in &l.events {
            for e in evs {
                end = end.max(e.1 + 5.);
            }
        }
        for e in &l.speeds {
            end = end.max(e.0 + 5.);
        }
    }
    let mut out = vec![];
    for i in 0..=max_index {
        let mut l = lines.remove(&i).unwrap_or_default();
        let mut layer = json!({});
        for (k, key) in ["moveXEvents", "moveYEvents", "rotateEvents", "alphaEvents"].iter().enumerate() {
            let transform = |v: f64| match k {
                0 => (v / 2048. - 0.5) * 1350.,
                1 => (v / 1400. - 0.5) * 900.,
                2 => v,
                _ => {
                    if v >= 0. {
                        v.clamp(0., 255.)
                    } else {
                        0.
                    }
                }
            };
            l.events[k].sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
            let mut keys: Vec<(f64, f64, i64)> = vec![];
            let mut last_end = f64::NEG_INFINITY;
            for (a, b, v, ease) in &l.events[k] {
                ensure!(b >= a, "反向 PEC 事件");
                let a = a.max(last_end);
                if *b > a {
                    let prior = keys.last().context("PEC 动画缺少初始值")?.1;
                    keys.push((a, prior, *ease));
                }
                keys.push((*b, *v, -1));
                last_end = *b;
            }
            if keys.first().is_some_and(|v| v.0 > 0.) {
                let first = keys[0];
                keys.insert(0, (0., first.1, first.2));
            }
            let mut events = vec![];
            for j in 0..keys.len() {
                let (a, v, ease) = keys[j];
                let (b, z, _) = keys.get(j + 1).copied().unwrap_or((end, v, -1));
                let mut e = event(a * 2., b * 2., transform(v), transform(if ease == -1 { v } else { z }));
                e["easingType"] = json!(ease.max(1));
                events.push(e);
            }
            if events.is_empty() {
                let d = if k == 3 { 0. } else { 0. };
                events.push(event(0., end * 2., d, d));
            }
            layer[*key] = Value::Array(events);
        }
        l.speeds.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut speeds = vec![];
        for j in 0..l.speeds.len() {
            let (a, v) = l.speeds[j];
            let b = l.speeds.get(j + 1).map(|v| v.0).unwrap_or(end);
            speeds.push(event(a * 2., b * 2., v, v));
        }
        if speeds.is_empty() {
            speeds.push(event(0., end * 2., 4.5, 4.5));
        }
        layer["speedEvents"] = Value::Array(speeds);
        let mut line = static_line(l.notes, 4.5, end * 2.);
        line["Name"] = json!(format!("PEC Line {i}"));
        line["zOrder"] = json!(0);
        line["eventLayers"] = json!([layer]);
        out.push(line);
    }
    root(out, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Metadata {
        offset: i32,
    }

    #[test]
    fn adapters_emit_integer_millisecond_offsets() {
        for seconds in [0., 0.2, -0.15, 0.001] {
            let chart = root(vec![], seconds).unwrap();
            let encoded = serde_json::to_vec(&chart["META"]).unwrap();
            let decoded: Metadata = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded.offset, (seconds * 1000.).round() as i32);
            assert!(chart["META"]["offset"].is_i64());
        }
        let chart = pec("150\nbp 0 120\nn1 0 2 1024 1 0").unwrap();
        let decoded: Metadata = serde_json::from_value(chart["META"].clone()).unwrap();
        assert_eq!(decoded.offset, 0);
        assert!(root(vec![], f64::INFINITY).is_err());
        assert!(root(vec![], (i32::MAX as f64 + 1.) / 1000.).is_err());
    }

    #[test]
    fn compaction_preserves_gaps_jumps_and_instant_events() {
        let events = simplify_segments(vec![
            (0., 1., 0., 1.), (1., 2., 1., 2.),
            (2., 3., 10., 11.), (4., 5., 11., 12.),
            (5., 5., 12., 20.), (5., 6., 20., 21.),
        ], 0.5);
        assert_eq!(events.len(), 5);
        assert_eq!(events[0]["endTime"], triple(2.));
        assert_eq!(events[1]["start"], 10.);
        assert_eq!(events[2]["startTime"], triple(4.));
        assert_eq!(events[3]["startTime"], events[3]["endTime"]);
    }

    #[test]
    fn compaction_bounds_error_at_every_source_vertex() {
        let points = (0..1001).map(|i| (i as f64 * 0.01, (i as f64 * 0.013).sin() * 100.)).collect::<Vec<_>>();
        let segments = points.windows(2).map(|p| (p[0].0, p[1].0, p[0].1, p[1].1)).collect();
        let events = simplify_segments(segments, 0.5);
        assert!(events.len() < 100);
        for &(t, v) in &points {
            let e = events.iter().find(|e| beat(&e["startTime"]).unwrap() <= t + 1e-9 && beat(&e["endTime"]).unwrap() >= t - 1e-9).unwrap();
            let a = beat(&e["startTime"]).unwrap();
            let b = beat(&e["endTime"]).unwrap();
            let s = e["start"].as_f64().unwrap();
            let z = e["end"].as_f64().unwrap();
            assert!((v - (s + (z - s) * (t - a) / (b - a))).abs() <= 0.500001);
        }
    }

    #[test]
    fn phigros_adapter_compacts_linear_motion_and_emits_loadable_meta() {
        let movement = (0..1000).map(|i| json!({
            "startTime":i,"endTime":i+1,"start":0.1+i as f64*0.0001,
            "end":0.1+(i+1) as f64*0.0001,"start2":0.5,"end2":0.5,
        })).collect::<Vec<_>>();
        let chart = pgr(&json!({"formatVersion":3,"offset":0.0,"judgeLineList":[{
            "bpm":120,"notesAbove":[{"type":1,"time":1000,"positionX":0}],
            "judgeLineMoveEvents":movement,
        }]})).unwrap();
        let layer = &chart["judgeLineList"][0]["eventLayers"][0];
        assert_eq!(array(&layer["moveXEvents"]).len(), 1);
        assert_eq!(array(&layer["moveYEvents"]).len(), 1);
        let decoded: Metadata = serde_json::from_slice(&serde_json::to_vec(&chart["META"]).unwrap()).unwrap();
        assert_eq!(decoded.offset, 0);
        assert_eq!(chart["judgeLineList"][0]["notes"][0]["startTime"], triple(1000. / 32.));
    }
}
