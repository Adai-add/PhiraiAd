use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Features {
    pub events: Vec<Vec<f64>>,
    pub note_groups: Vec<usize>,
    pub group_features: Vec<Vec<f64>>,
    pub group_times: Vec<f64>,
    pub group_durations: Vec<f64>,
    pub stats: Vec<f64>,
}
impl Features {
    pub fn empty() -> Self {
        Self {
            events: Vec::new(),
            note_groups: Vec::new(),
            group_features: Vec::new(),
            group_times: Vec::new(),
            group_durations: Vec::new(),
            stats: vec![0.; 18],
        }
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        let n = self.events.len();
        let g = self.group_features.len();
        if n == 0 {
            ensure!(
                g == 0
                    && self.note_groups.is_empty()
                    && self.group_times.is_empty()
                    && self.group_durations.is_empty()
                    && self.stats.len() == 18
                    && self.stats.iter().all(|v| v.is_finite()),
                "空谱特征无效"
            );
            return Ok(());
        }
        ensure!(n > 0 && n <= 200_000 && g > 0 && self.note_groups.len() == n, "音符/同刻组数量无效");
        for (rows, width) in [(&self.events, 17), (&self.group_features, 6)] {
            ensure!(rows.iter().all(|r| r.len() == width && r.iter().all(|v| v.is_finite())), "特征无效");
        }
        ensure!(self.stats.len() == 18 && self.stats.iter().all(|v| v.is_finite()), "整谱统计无效");
        ensure!(self.group_durations.len() == g && self.group_durations.iter().all(|v| v.is_finite() && *v > 0.), "同刻组时长无效");
        ensure!(
            self.group_times.len() == g && self.group_times.iter().all(|v| v.is_finite()) && self.group_times.windows(2).all(|w| w[1] > w[0]),
            "时间顺序无效"
        );
        ensure!(
            self.note_groups[0] == 0 && self.note_groups[n - 1] == g - 1 && self.note_groups.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1),
            "同刻组编号无效"
        );
        Ok(())
    }
}
pub fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|v| v.is_finite())
}
fn num(v: &Value) -> Result<f64> {
    Ok(number(v).map(|v| finite(v, 0.)).unwrap_or(0.))
}
fn optional(v: &Value, key: &str, default: f64) -> Result<f64> {
    Ok(v.get(key).and_then(number).map(|v| finite(v, default)).unwrap_or(default))
}
pub fn finite(v: f64, default: f64) -> f64 {
    if v.is_finite() {
        v.clamp(-1e12, 1e12)
    } else {
        default
    }
}
fn round9(v: f64) -> f64 {
    format!("{v:.9}").parse().unwrap_or(v)
}
#[derive(Clone)]
struct Event {
    a: f64,
    b: f64,
    v: Vec<f64>,
}
struct Track {
    events: Vec<Event>,
    default: Vec<f64>,
}
impl Track {
    fn new(v: Option<&Value>, keys: &[&str], default: &[f64]) -> Result<Self> {
        let mut events = Vec::new();
        if let Some(array) = v.and_then(Value::as_array) {
            for e in array {
                let (Some(a), Some(b)) = (number(&e["startTime"]), number(&e["endTime"])) else {
                    continue;
                };
                if b < a {
                    continue;
                } // Same treatment as Phira's invalid-time filter.
                let mut values = Vec::new();
                for (i, key) in keys.iter().enumerate() {
                    let fallback = if keys.len() > 1 && i % 2 == 1 {
                        values[i - 1]
                    } else {
                        default[if keys.len() == 1 { 0 } else { i / 2 }]
                    };
                    values.push(number(&e[*key]).map(|v| finite(v, fallback)).unwrap_or(fallback));
                }
                events.push(Event { a, b, v: values });
            }
        }
        // Keep legacy reference behavior for speed sentinels when real intervals
        // exist, but retain instant values if they are the only speed events.
        if keys == ["value"] && events.iter().any(|e| e.b > e.a) {
            events.retain(|e| e.b > e.a);
        }
        events.sort_by(|a, b| a.a.total_cmp(&b.a));
        Ok(Self {
            events,
            default: default.into(),
        })
    }
    fn at(&self, t: f64, before: bool) -> Vec<f64> {
        if self.events.is_empty() {
            return self.default.clone();
        }
        let i = self.events.partition_point(|e| if before { e.a < t } else { e.a <= t }).saturating_sub(1);
        let e = &self.events[i];
        if e.v.len() == 1 {
            return e.v.clone();
        }
        let f = if e.b == e.a {
            if t >= e.a {
                1.
            } else {
                0.
            }
        } else {
            ((t - e.a) / (e.b - e.a)).clamp(0., 1.)
        };
        e.v.chunks_exact(2).map(|v| v[0] + (v[1] - v[0]) * f).collect()
    }
}
struct Line {
    bpm: f64,
    movement: Track,
    rotation: Track,
    speed: Track,
}
#[derive(Clone, Debug)]
pub struct Note {
    pub line: usize,
    pub side: i32,
    pub kind: usize,
    pub time: f64,
    pub end: f64,
    pub x: f64,
    pub speed: f64,
    pub line_speed: f64,
    pub point: [f64; 2],
    pub past: [f64; 2],
    pub rotation: f64,
    pub past_rotation: f64,
}
impl Line {
    fn point(&self, tick: f64, x: f64) -> [f64; 2] {
        let p = self.movement.at(tick, false);
        let r = self.rotation.at(tick, false)[0].to_radians();
        let d = x * 0.05625;
        [p[0] + d * r.cos(), p[1] / 1.777778 + d * r.sin()]
    }
}
pub fn extract(raw: &Value, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    let offset = optional(raw, "offset", 0.)?;
    let lines = raw["judgeLineList"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    ensure!(lines.len() <= 10000, "判定线过多");
    let fallback_bpm = lines.iter().filter_map(|l| number(&l["bpm"])).find(|b| *b > 0.).unwrap_or(120.);
    let mut notes = Vec::new();
    for (index, raw) in lines.iter().take(10000).enumerate() {
        checkpoint()?;
        let bpm = number(&raw["bpm"]).filter(|b| *b > 0.).unwrap_or(fallback_bpm).clamp(0.001, 1e6);
        let line = Line {
            bpm,
            movement: Track::new(raw.get("judgeLineMoveEvents"), &["start", "end", "start2", "end2"], &[0.5, 0.5])?,
            rotation: Track::new(raw.get("judgeLineRotateEvents"), &["start", "end"], &[0.])?,
            speed: Track::new(raw.get("speedEvents"), &["value"], &[1.])?,
        };
        // Opacity is not a model feature; malformed visual events are irrelevant.
        for (field, side) in [("notesAbove", 1), ("notesBelow", -1)] {
            if let Some(v) = raw.get(field).and_then(Value::as_array) {
                for n in v {
                    if notes.len() % 128 == 0 {
                        checkpoint()?;
                    }
                    ensure!(notes.len() < 200_000, "音符过多");
                    let Some(kind) = number(&n["type"])
                        .filter(|v| v.fract() == 0. && (1. ..=4.).contains(v))
                        .map(|v| v as usize)
                    else {
                        continue;
                    };
                    let fake = &n["isFake"];
                    if fake == &Value::Bool(true) || number(fake).is_some_and(|v| v != 0.) {
                        continue;
                    }
                    let Some(tick) = number(&n["time"]) else {
                        continue;
                    };
                    let hold = if kind == 3 { optional(n, "holdTime", 0.)?.max(0.) } else { 0. };
                    let time = round9(tick * 1.875 / bpm + offset);
                    let end = round9(time + hold * 1.875 / bpm);
                    let x = num(&n["positionX"])?;
                    let speed = optional(n, "speed", 1.)?;
                    // floorPosition is not used by feature extraction.
                    let past_tick = (time - 0.15 - offset) * line.bpm / 1.875;
                    notes.push(Note {
                        line: index,
                        side,
                        kind,
                        time,
                        end,
                        x,
                        speed,
                        line_speed: line.speed.at(tick, true)[0],
                        point: line.point(tick, x),
                        past: line.point(past_tick, x),
                        rotation: line.rotation.at(tick, false)[0],
                        past_rotation: line.rotation.at(past_tick, false)[0],
                    });
                }
            }
        }
    }
    from_notes(notes, checkpoint)
}
#[derive(PartialEq)]
struct End(f64);
impl Eq for End {}
impl PartialOrd for End {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for End {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}
fn weight(k: usize) -> f64 {
    match k {
        1 | 3 => 1.,
        4 => 0.5,
        _ => 0.,
    }
}
fn log(v: f64) -> f64 {
    v.abs().ln_1p().copysign(v)
}
fn quantile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    let i = (v.len() - 1) as f64 * q;
    let lo = i.floor() as usize;
    let hi = i.ceil() as usize;
    v[lo] + (v[hi] - v[lo]) * (i - lo as f64)
}
pub fn from_notes(mut notes: Vec<Note>, checkpoint: &mut dyn FnMut() -> Result<()>) -> Result<Features> {
    checkpoint()?;
    notes.retain(|n| (1..=4).contains(&n.kind) && n.time.is_finite());
    ensure!(notes.len() <= 200_000, "音符过多");
    for n in &mut notes {
        n.time = finite(n.time, 0.);
        n.end = finite(n.end, n.time).max(n.time);
        n.x = finite(n.x, 0.);
        n.speed = finite(n.speed, 1.);
        n.line_speed = finite(n.line_speed, 1.);
        n.rotation = finite(n.rotation, 0.);
        n.past_rotation = finite(n.past_rotation, n.rotation);
        for i in 0..2 {
            n.point[i] = finite(n.point[i], if i == 0 { 0.5 } else { 0.5 / 1.777778 });
            n.past[i] = finite(n.past[i], n.point[i]);
        }
    }
    if notes.is_empty() {
        return Ok(Features::empty());
    }
    notes.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then(a.line.cmp(&b.line))
            .then(a.side.cmp(&b.side))
            .then(a.x.total_cmp(&b.x))
            .then(a.kind.cmp(&b.kind))
            .then(a.end.total_cmp(&b.end))
    });
    let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        if i == 0 || n.time != notes[i - 1].time {
            groups.push(i..i + 1);
        } else {
            groups.last_mut().unwrap().end = i + 1;
        }
    }
    let first = notes[0].time;
    let end = notes.iter().map(|n| n.end).fold(f64::NEG_INFINITY, f64::max);
    let duration = (end - first).max(0.001);
    let mut f = Features {
        events: Vec::new(),
        note_groups: Vec::new(),
        group_features: Vec::new(),
        group_times: Vec::new(),
        group_durations: Vec::new(),
        stats: Vec::new(),
    };
    let mut active: std::collections::BinaryHeap<std::cmp::Reverse<End>> = Default::default();
    let mut times: Vec<f64> = Vec::new();
    let mut weights = Vec::new();
    let mut edges = Vec::new();
    let mut press_times = Vec::new();
    let mut counts = [0.; 4];
    for (i, g) in groups.iter().enumerate() {
        if i % 64 == 0 {
            checkpoint()?;
        }
        let t = notes[g.start].time;
        while active.peek().is_some_and(|v| v.0 .0 <= t) {
            active.pop();
        }
        let w = notes[g.clone()].iter().map(|n| weight(n.kind)).sum::<f64>();
        f.group_features.push(vec![
            if i == 0 { 0. } else { (t - times[i - 1]).ln_1p() },
            (t - first).ln_1p(),
            (t - first) / duration,
            (g.len() as f64).ln_1p(),
            w.ln_1p(),
            (active.len() as f64).ln_1p(),
        ]);
        times.push(t);
        weights.push(w);
        if w > 0. {
            press_times.push(t);
        }
        let relative = t - first;
        // Large offsets can make subtraction erase a small positive gap.
        // group_times are metadata; keep them ordered without changing durations.
        let relative = f
            .group_times
            .last()
            .copied()
            .filter(|last| relative <= *last)
            .map_or(relative, f64::next_up);
        f.group_times.push(relative);
        let next = if i + 1 < groups.len() {
            notes[groups[i + 1].start].time
        } else {
            end.max(t + 0.001)
        };
        let gap = next - t;
        f.group_durations.push(if gap.is_finite() && gap > 0. { gap } else { 0.001 });
        for n in &notes[g.clone()] {
            counts[n.kind - 1] += 1.;
            let v = n.line_speed * if n.kind == 3 { 1. } else { n.speed };
            let defined = v.abs() >= 1e-12;
            let angle = (n.rotation - 90. * n.side as f64 + if v < 0. { 180. } else { 0. })
                .rem_euclid(360.)
                .to_radians();
            let delta = (n.rotation - n.past_rotation).to_radians();
            let mut event = (1..=4).map(|k| if k == n.kind { 1. } else { 0. }).collect::<Vec<_>>();
            event.extend([
                n.point[0],
                n.point[1],
                (n.end - n.time).ln_1p(),
                if defined { angle.sin() } else { 0. },
                if defined { angle.cos() } else { 0. },
                if defined { 1. } else { 0. },
                n.side as f64,
                log(n.speed),
                log(n.line_speed),
                n.point[0] - n.past[0],
                n.point[1] - n.past[1],
                delta.sin(),
                delta.cos(),
            ]);
            f.events.push(event);
            f.note_groups.push(i);
            if n.end > t {
                active.push(std::cmp::Reverse(End(n.end)));
                edges.push((t, 1));
                edges.push((n.end, -1));
            }
        }
    }
    let mut press = Vec::new();
    let (mut left, mut sum) = (0, 0.);
    for i in 0..times.len() {
        sum += weights[i];
        while times[left] <= times[i] - 1. {
            sum -= weights[left];
            left += 1;
        }
        press.push(sum);
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut coverage, mut overlap, mut running, mut prev) = (0., 0., 0, first);
    for (t, change) in edges {
        if running > 0 {
            coverage += t - prev;
        }
        if running > 1 {
            overlap += t - prev;
        }
        prev = t;
        running += change;
    }
    let gaps = press_times.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>();
    let n = notes.len() as f64;
    f.stats = vec![
        duration.ln_1p(),
        n.ln_1p(),
        n / duration,
        weights.iter().sum::<f64>() / duration,
        press.iter().copied().fold(0., f64::max),
        quantile(&press, 0.9),
    ];
    f.stats.extend(counts.map(|c| c / n));
    f.stats.extend([
        coverage / duration,
        overlap / duration,
        groups.iter().filter(|g| g.len() > 1).count() as f64 / groups.len() as f64,
        groups.iter().map(|g| g.len()).max().unwrap() as f64,
        quantile(&gaps, 0.5),
        gaps.iter().copied().reduce(f64::min).unwrap_or(0.),
        gaps.iter().filter(|v| **v > 1.).sum::<f64>() / duration,
        notes.iter().map(|n| n.end - n.time).sum::<f64>() / duration,
    ]);
    for row in f.events.iter_mut().chain(&mut f.group_features).chain(std::iter::once(&mut f.stats)) {
        for v in row {
            *v = finite(*v, 0.);
        }
    }
    f.validate()?;
    Ok(f)
}
