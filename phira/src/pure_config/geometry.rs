//! CPU-only random-access animation evaluation. No graphics/audio context required.
use super::{array, beat, event, number};
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct BpmMap {
    pub beats: Vec<f64>,
    seconds: Vec<f64>,
    bpms: Vec<f64>,
}
impl BpmMap {
    pub fn new(items: &Value) -> Result<Self> {
        let mut pairs = array(items)
            .iter()
            .map(|e| Ok((beat(&e["startTime"])?, number(&e["bpm"], 0.)?)))
            .collect::<Result<Vec<_>>>()?;
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure!(!pairs.is_empty(), "谱面缺少 BPM");
        let mut r = Self {
            beats: vec![],
            seconds: vec![],
            bpms: vec![],
        };
        let mut sec = pairs[0].0 * 60. / pairs[0].1;
        for (b, p) in pairs {
            ensure!(p > 0., "BPM 必须大于 0");
            if let Some(last) = r.beats.last() {
                ensure!(b > *last, "同一拍出现多个 BPM");
                sec += (b - last) * 60. / r.bpms.last().unwrap();
            }
            r.beats.push(b);
            r.seconds.push(sec);
            r.bpms.push(p);
        }
        Ok(r)
    }
    pub fn time(&self, b: f64) -> f64 {
        let i = self.beats.partition_point(|v| *v <= b).saturating_sub(1);
        self.seconds[i] + (b - self.beats[i]) * 60. / self.bpms[i]
    }
    pub fn to_beat(&self, t: f64) -> f64 {
        let i = self.seconds.partition_point(|v| *v <= t).saturating_sub(1);
        self.beats[i] + (t - self.seconds[i]) * self.bpms[i] / 60.
    }
}

#[derive(Clone, Debug)]
struct Ease {
    kind: i64,
    left: f64,
    right: f64,
    points: Option<[f64; 4]>,
}
impl Ease {
    fn from(e: &Value) -> Result<Self> {
        let points = if e["bezier"].as_i64().unwrap_or(0) != 0 {
            let p = array(&e["bezierPoints"]);
            ensure!(p.len() == 4, "Bezier 需要四个控制点");
            let p = [number(&p[0], 0.)?, number(&p[1], 0.)?, number(&p[2], 0.)?, number(&p[3], 0.)?];
            ensure!((0. ..=1.).contains(&p[0]) && (0. ..=1.).contains(&p[2]), "Bezier X 超出范围");
            Some(p)
        } else {
            None
        };
        let r = Self {
            kind: e["easingType"].as_i64().unwrap_or(1).max(0),
            left: number(&e["easingLeft"], 0.)?.clamp(0., 1.),
            right: number(&e["easingRight"], 1.)?.clamp(0., 1.),
            points,
        };
        if r.kind > 1 && r.left < r.right && (r.left != 0. || r.right != 1.) && r.points.is_none() {
            ensure!((ease(r.kind, r.right) - ease(r.kind, r.left)).abs() > 1e-12, "缓动裁剪无法归一化");
        }
        Ok(r)
    }
    fn value(&self, x: f64) -> f64 {
        if self.kind == -1 {
            return 0.;
        }
        let x = x.clamp(0., 1.);
        if let Some([x1, y1, x2, y2]) = self.points {
            if x == 0. || x == 1. {
                return x;
            }
            let sample = |a: f64, b: f64, t: f64| 3. * (1. - t).powi(2) * t * a + 3. * (1. - t) * t * t * b + t * t * t;
            let (mut lo, mut hi) = (0., 1.);
            for _ in 0..34 {
                let mid = (lo + hi) / 2.;
                if sample(x1, x2, mid) < x {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return sample(y1, y2, (lo + hi) / 2.);
        }
        if self.kind <= 1 || self.left >= self.right || (self.left == 0. && self.right == 1.) {
            return ease(self.kind, x);
        }
        let a = ease(self.kind, self.left);
        (ease(self.kind, self.left + (self.right - self.left) * x) - a) / (ease(self.kind, self.right) - a)
    }
}
fn base(k: usize, x: f64) -> f64 {
    match k {
        0 => x,
        1 => 1. - (std::f64::consts::PI * x / 2.).cos(),
        2 => x * x,
        3 => x.powi(3),
        4 => x.powi(4),
        5 => x.powi(5),
        6 => 2f64.powf(10. * (x - 1.)),
        7 => 1. - (1. - x * x).max(0.).sqrt(),
        8 => (2.70158 * x - 1.70158) * x * x,
        9 => -2f64.powf(10. * x - 10.) * ((10. * x - 10.75) * 2. * std::f64::consts::PI / 3.).sin(),
        _ => {
            let u = 1. - x;
            let y = if u < 1. / 2.75 {
                7.5625 * u * u
            } else if u < 2. / 2.75 {
                7.5625 * (u - 1.5 / 2.75).powi(2) + 0.75
            } else if u < 2.5 / 2.75 {
                7.5625 * (u - 2.25 / 2.75).powi(2) + 0.9375
            } else {
                7.5625 * (u - 2.625 / 2.75).powi(2) + 0.984375
            };
            1. - y
        }
    }
}
fn ease(k: i64, x: f64) -> f64 {
    const MAP: [(usize, u8); 30] = [
        (0, 0),
        (0, 0),
        (1, 1),
        (1, 0),
        (2, 1),
        (2, 0),
        (1, 2),
        (2, 2),
        (3, 1),
        (3, 0),
        (4, 1),
        (4, 0),
        (3, 2),
        (4, 2),
        (5, 1),
        (5, 0),
        (6, 1),
        (6, 0),
        (7, 1),
        (7, 0),
        (8, 1),
        (8, 0),
        (7, 2),
        (8, 2),
        (9, 1),
        (9, 0),
        (10, 1),
        (10, 0),
        (10, 2),
        (9, 2),
    ];
    let (n, m) = MAP.get(k as usize).copied().unwrap_or((0, 0));
    let x = x.clamp(0., 1.);
    match m {
        0 => base(n, x),
        1 => 1. - base(n, 1. - x),
        _ => {
            if x < 0.5 {
                base(n, 2. * x) / 2.
            } else {
                1. - base(n, 2. - 2. * x) / 2.
            }
        }
    }
}
#[derive(Clone, Debug)]
struct Key {
    t: f64,
    v: f64,
    ease: Ease,
}
#[derive(Clone, Debug, Default)]
pub struct Track {
    keys: Vec<Key>,
}
impl Track {
    fn new(events: &Value, bpm: &BpmMap) -> Result<Self> {
        let mut keys = vec![];
        for e in array(events) {
            let a = bpm.time(beat(&e["startTime"])?);
            let b = bpm.time(beat(&e["endTime"])?);
            ensure!(b >= a, "事件结束早于开始");
            keys.push(Key {
                t: a,
                v: number(&e["start"], 0.)?,
                ease: Ease::from(e)?,
            });
            keys.push(Key {
                t: b,
                v: number(&e["end"], 0.)?,
                ease: Ease {
                    kind: -1,
                    left: 0.,
                    right: 1.,
                    points: None,
                },
            });
        }
        keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(Self { keys })
    }
    fn constant_between(&self, a: f64, b: f64) -> bool {
        if self.keys.is_empty() { return true; }
        let i = self.keys.partition_point(|k| k.t <= a).saturating_sub(1);
        let current = &self.keys[i];
        match self.keys.get(i + 1) {
            None => true,
            Some(next) => next.t >= b && (current.v == next.v || current.ease.kind == -1),
        }
    }
    pub fn value(&self, t: f64) -> f64 {
        if self.keys.is_empty() {
            return 0.;
        }
        let i = self.keys.partition_point(|k| k.t <= t).saturating_sub(1);
        let a = &self.keys[i];
        if let Some(b) = self.keys.get(i + 1) {
            a.v + (b.v - a.v) * a.ease.value(if b.t == a.t { 0. } else { (t - a.t) / (b.t - a.t) })
        } else {
            a.v
        }
    }
}
#[derive(Clone, Debug)]
struct SpeedSegment {
    a: f64,
    b: f64,
    s: f64,
    e: f64,
    ease: Ease,
    mode: u8,
    height: f64,
    table: Vec<f64>,
}
impl SpeedSegment {
    fn partial(&self, x: f64) -> f64 {
        let x = x.clamp(0., 1.);
        let d = self.e - self.s;
        if self.mode == 0 || self.ease.kind <= 1 {
            let end = if self.mode == 0 {
                self.e + (self.s - self.e) * (1e-4 / (self.b - self.a)).min(1.)
            } else if self.ease.kind == 0 {
                self.s
            } else {
                self.e
            };
            return self.s * x + (end - self.s) * x * x / 2.;
        }
        if self.mode == 1 {
            let h = 1e-6;
            let edge = 1e-7;
            let d0 = (self.ease.value(h) - self.ease.value(edge)) / (h - edge);
            let d1 = (self.ease.value(1. - edge) - self.ease.value(1. - h)) / (h - edge);
            if !(d1 - d0).is_finite() || (d1 - d0).abs() < 1e-8 {
                return self.s * x + d * x * x / 2.;
            }
            let k = d / (d1 - d0);
            return k * self.ease.value(x) + (self.s - k * d0) * x;
        }
        let i = ((x * 256.).floor() as usize).min(255);
        let integral = if x >= 1. {
            self.table[256]
        } else {
            self.table[i] + integrate(&self.ease, i as f64 / 256., x)
        };
        self.s * x + d * integral
    }
    fn integral(&self, x: f64) -> f64 {
        let v = self.partial(1.);
        if v.abs() < 1e-7 && self.mode != 0 && self.ease.kind > 1 {
            self.s * x + (self.e - self.s) * x * x / 2.
        } else {
            self.partial(x)
        }
    }
}
fn integrate(e: &Ease, a: f64, b: f64) -> f64 {
    let nodes = [0.1834346424956498, 0.525532409916329, 0.7966664774136267, 0.9602898564975363];
    let weights = [0.362683783378362, 0.3137066458778873, 0.2223810344533745, 0.1012285362903763];
    let m = (a + b) / 2.;
    let r = (b - a) / 2.;
    r * nodes
        .iter()
        .zip(weights)
        .map(|(n, w)| w * (e.value(m - r * n) + e.value(m + r * n)))
        .sum::<f64>()
}
#[derive(Clone, Debug, Default)]
struct SpeedCurve {
    segments: Vec<SpeedSegment>,
    end: f64,
    last: f64,
    height: f64,
}
impl SpeedCurve {
    fn new(es: &Value, bpm: &BpmMap, mode: u8) -> Result<Self> {
        let mut events = array(es)
            .iter()
            .map(|e| {
                Ok((
                    bpm.time(beat(&e["startTime"])?),
                    bpm.time(beat(&e["endTime"])?),
                    number(&e["start"], 0.)? * 10. / 45.,
                    number(&e["end"], 0.)? * 10. / 45.,
                    Ease::from(e)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut r = Self::default();
        for (a, b, s, e, ease) in events {
            ensure!(b >= a, "速度事件结束早于开始");
            let a = a.max(r.end);
            let b = b.max(a);
            if a > r.end {
                r.push(
                    r.end,
                    a,
                    r.last,
                    r.last,
                    Ease {
                        kind: 1,
                        left: 0.,
                        right: 1.,
                        points: None,
                    },
                    mode,
                );
            }
            if b > a {
                r.push(a, b, s, e, ease, mode);
            }
            r.end = b;
            r.last = e;
        }
        Ok(r)
    }
    fn push(&mut self, a: f64, b: f64, s: f64, e: f64, ease: Ease, mode: u8) {
        let mut seg = SpeedSegment {
            a,
            b,
            s,
            e,
            ease,
            mode,
            height: self.height,
            table: vec![],
        };
        if mode == 2 && seg.ease.kind > 1 {
            seg.table.push(0.);
            for j in 0..256 {
                let v = seg.table[j] + integrate(&seg.ease, j as f64 / 256., (j + 1) as f64 / 256.);
                seg.table.push(v);
            }
        }
        self.height += (b - a) * seg.integral(1.);
        self.segments.push(seg);
    }
    fn at(&self, t: f64) -> f64 {
        if t >= self.end {
            return self.height + (t - self.end) * self.last;
        }
        let count = self.segments.partition_point(|s| s.a <= t);
        if count == 0 {
            return 0.;
        }
        let s = &self.segments[count - 1];
        s.height + (s.b - s.a) * s.integral((t - s.a) / (s.b - s.a))
    }
}
#[derive(Clone, Debug)]
pub struct Line {
    tracks: [Vec<Track>; 4],
    speeds: Vec<SpeedCurve>,
    pub parent: Option<usize>,
    rotate_with_parent: bool,
}
impl Line {
    pub fn value(&self, k: usize, t: f64) -> f64 {
        self.tracks[k].iter().map(|v| v.value(t)).sum()
    }
    pub fn height(&self, t: f64) -> f64 {
        self.speeds.iter().map(|v| v.at(t)).sum()
    }
}
pub struct Geometry {
    pub lines: Vec<Line>,
    chains: Vec<Vec<usize>>,
}
impl Geometry {
    #[cfg(test)]
    pub fn new(data: &Value, bpm: &BpmMap, modern: bool) -> Result<Self> {
        Self::new_with_progress(data, bpm, modern, &super::Progress::default())
    }
    pub fn new_with_progress(data: &Value, bpm: &BpmMap, modern: bool, progress: &super::Progress) -> Result<Self> {
        let raw = array(&data["judgeLineList"]);
        let mode = if !modern {
            0
        } else if number(&data["META"]["RPEVersion"], 160.)? >= 170. {
            2
        } else {
            1
        };
        let total = raw.len() * 2 + raw.iter().flat_map(|l| array(&l["eventLayers"])).map(|layer| ["moveXEvents", "moveYEvents", "rotateEvents", "alphaEvents", "speedEvents"].iter().map(|k| array(&layer[*k]).len()).sum::<usize>()).sum::<usize>();
        let mut done = 0;
        let mut lines = vec![];
        for l in raw {
            progress.check()?;
            let mut tracks: [Vec<Track>; 4] = Default::default();
            let mut speeds = vec![];
            for layer in array(&l["eventLayers"]) {
                for (k, key) in ["moveXEvents", "moveYEvents", "rotateEvents", "alphaEvents"].iter().enumerate() {
                    if !array(&layer[*key]).is_empty() {
                        tracks[k].push(Track::new(&layer[*key], bpm)?);
                        done += array(&layer[*key]).len();
                        progress.update(super::Stage::Geometry, done as u64, total as u64);
                    }
                }
                if !array(&layer["speedEvents"]).is_empty() {
                    speeds.push(SpeedCurve::new(&layer["speedEvents"], bpm, mode)?);
                    done += array(&layer["speedEvents"]).len();
                    progress.update(super::Stage::Geometry, done as u64, total as u64);
                }
            }
            done += 1;
            let p = l["father"].as_i64().unwrap_or(-1);
            ensure!(p >= -1 && (p < raw.len() as i64), "父判定线编号无效");
            lines.push(Line {
                tracks,
                speeds,
                parent: if p < 0 { None } else { Some(p as usize) },
                rotate_with_parent: l["rotateWithFather"].as_bool().unwrap_or(false),
            });
        }
        let mut chains = Vec::with_capacity(lines.len());
        for i in 0..lines.len() {
            let mut n = Some(i);
            let mut chain = Vec::new();
            let mut count = 0;
            while let Some(j) = n {
                count += 1;
                if count > lines.len() {
                    bail!("判定线父子关系存在循环");
                }
                chain.push(j);
                n = lines[j].parent;
            }
            chains.push(chain);
            done += 1;
            progress.check()?;
            progress.update(super::Stage::Geometry, done as u64, total as u64);
        }
        Ok(Self { lines, chains })
    }
    pub fn world(&self, i: usize, t: f64) -> (f64, f64, f64) {
        let chain = &self.chains[i];
        let (mut x, mut y, mut r) = (0., 0., 0.);
        for &k in chain.iter().rev() {
            let l = &self.lines[k];
            let lx = ((0.5 + l.value(0, t) / 1350.) - 0.5) * 1350.;
            let ly = ((0.5 + l.value(1, t) / 900.) - 0.5) * 900.;
            let lr = l.value(2, t);
            if l.parent.is_none() {
                x = lx;
                y = ly;
                r = lr;
            } else {
                let a = r.to_radians();
                let dx = (0.5 + lx / 1350.) - 0.5;
                let dy = ((0.5 + ly / 900.) - 0.5) / (16. / 9.);
                x = ((0.5 + x / 1350.) + dx * a.cos() - dy * a.sin() - 0.5) * 1350.;
                y = ((0.5 + y / 900.) + (16. / 9.) * (dx * a.sin() + dy * a.cos()) - 0.5) * 900.;
                r = if l.rotate_with_parent { lr + r } else { lr };
            }
        }
        (x, y, r)
    }
    fn constant_between(&self, i: usize, a: f64, b: f64) -> bool {
        self.chains[i].iter().all(|&k| (0..if k == i { 4 } else { 3 }).all(|n| self.lines[k].tracks[n].iter().all(|tr| tr.constant_between(a, b))))
    }
    fn world_cached(&self, i: usize, t: f64, cache: &mut HashMap<(usize, u64), (f64, f64, f64)>) -> (f64, f64, f64) {
        if self.chains[i].len() == 1 { return self.world(i, t); }
        if cache.len() >= 32768 { cache.clear(); }
        let mut pose = (0., 0., 0.);
        for &k in self.chains[i].iter().rev() {
            let key = (k, t.to_bits());
            if let Some(&cached) = cache.get(&key) { pose = cached; continue; }
            let l = &self.lines[k];
            let lx = ((0.5 + l.value(0, t) / 1350.) - 0.5) * 1350.;
            let ly = ((0.5 + l.value(1, t) / 900.) - 0.5) * 900.;
            let lr = l.value(2, t);
            pose = if l.parent.is_none() { (lx, ly, lr) } else {
                let (x, y, r) = pose;
                let a = r.to_radians();
                let dx = (0.5 + lx / 1350.) - 0.5;
                let dy = ((0.5 + ly / 900.) - 0.5) / (16. / 9.);
                (((0.5 + x / 1350.) + dx * a.cos() - dy * a.sin() - 0.5) * 1350.,
                 ((0.5 + y / 900.) + (16. / 9.) * (dx * a.sin() + dy * a.cos()) - 0.5) * 900.,
                 if l.rotate_with_parent { lr + r } else { lr })
            };
            if cache.len() >= 32768 { cache.clear(); }
            cache.insert(key, pose);
        }
        pose
    }
    pub fn boundaries(&self, i: usize, end: f64) -> Vec<f64> {
        let mut out = vec![0., end];
        let mut j = Some(i);
        while let Some(k) = j {
            for n in 0..if k == i { 4 } else { 3 } {
                for tr in &self.lines[k].tracks[n] {
                    out.extend(tr.keys.iter().map(|v| v.t).filter(|v| *v > 0. && *v < end));
                }
            }
            j = self.lines[k].parent;
        }
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }
}
pub fn proximity(x: f64, y: f64, r: f64) -> (f64, f64, f64) {
    let angle = (r + 90.).rem_euclid(180.) - 90.;
    let signed = y - x * angle.to_radians().tan() + 300.;
    let dy = signed.abs();
    let a = angle.abs();
    let limit = if a >= 36. || dy >= 150. {
        255.
    } else if a < 18. && dy < 75. {
        0.
    } else {
        255. * ((dy - 75.) / 75.).clamp(0., 1.).max(((a - 18.) / 18.).clamp(0., 1.))
    };
    (limit, signed, angle)
}

pub fn simplify(points: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((i, j)) = stack.pop() {
        let dt = points[j].0 - points[i].0;
        let mut worst = tol;
        let mut at = None;
        for k in i + 1..j {
            let f = if dt.abs() < 1e-12 { 0. } else { (points[k].0 - points[i].0) / dt };
            let e = (points[k].1 - (points[i].1 + (points[j].1 - points[i].1) * f)).abs();
            if e > worst {
                worst = e;
                at = Some(k);
            }
        }
        if let Some(k) = at {
            keep[k] = true;
            stack.push((i, k));
            stack.push((k, j));
        }
    }
    points.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

pub fn fade(data: &mut Value, bpm: &BpmMap, g: &Geometry, end: f64, progress: &super::Progress) -> Result<()> {
    let boundaries = (0..g.lines.len()).map(|i| g.boundaries(i, end)).collect::<Vec<_>>();
    let work = |a: f64, b: f64| (((b - a).max(0.) * 60.).ceil() as u64).saturating_add(4);
    let total = boundaries.iter().flat_map(|bs| bs.windows(2)).map(|p| work(p[0], p[1])).fold(0u64, u64::saturating_add);
    let cost = boundaries.iter().enumerate().map(|(i, bs)| bs.windows(2).map(|p| work(p[0], p[1]) as f64 * g.chains[i].len() as f64).sum::<f64>()).sum::<f64>();
    progress.weight(super::Stage::Fade, cost / 500. + 1.);
    let mut done = 0u64;
    let mut world_cache = HashMap::new();
    for i in 0..g.lines.len() {
        progress.check()?;
        let boundaries = &boundaries[i];
        let mut eval_cache = [None; 8];
        let mut cache_slot = 0;
        let mut eval = |t: f64| {
            let key = t.to_bits();
            for entry in eval_cache.iter().flatten() {
                let (cached_key, value) = *entry;
                if cached_key == key { return value; }
            }
            let original = g.lines[i].value(3, t).clamp(0., 255.);
            let (x, y, r) = g.world_cached(i, t, &mut world_cache);
            let (limit, sy, angle) = proximity(x, y, r);
            let result = (original.min(limit), original, limit, sy, angle);
            eval_cache[cache_slot] = Some((key, result));
            cache_slot = (cache_slot + 1) % eval_cache.len();
            result
        };
        let mut events = vec![];
        let mut affected = false;
        let mut times = Vec::new();
        let mut points = Vec::new();
        for pair in boundaries.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b <= a + 1e-12 {
                continue;
            }
            progress.check()?;
            let interval_work = work(a, b);
            progress.update(super::Stage::Fade, done, total);
            if [0., 0.25, 0.5, 0.75, 1.].iter().all(|f| g.lines[i].value(3, a + (b - a) * f) <= 1e-9) {
                events.push(event(bpm.to_beat(a), bpm.to_beat(b), 0., 0.));
                done = done.saturating_add(interval_work);
                continue;
            }
            let jump = (eval(b).0 - eval((b - 1e-5).max(a)).0).abs() > 0.25;
            if g.constant_between(i, a, b) {
                let start = eval(a);
                let last = eval(if jump { (b - 1e-5).max(a) } else { b });
                affected |= start.2 < start.1 - 1e-7 || last.2 < last.1 - 1e-7;
                events.push(event(bpm.to_beat(a), bpm.to_beat(b), start.0, last.0));
                done = done.saturating_add(interval_work);
                continue;
            }
            times.clear();
            times.push(a);
            let mut k = (a * 30.).floor() as usize + 1;
            while (k as f64 / 30.) < b - 1e-10 {
                times.push(k as f64 / 30.);
                k += 1;
            }
            times.push(b);
            let mut prev = eval(a);
            points.clear();
            points.push((a, prev.0));
            affected |= prev.2 < prev.1 - 1e-7;
            for (sample, pair) in times.windows(2).enumerate() {
                if sample % 128 == 0 {
                    progress.check()?;
                    let partial = (((pair[0] - a) * 60.).ceil() as u64).min(interval_work);
                    progress.update(super::Stage::Fade, done.saturating_add(partial), total);
                }
                let (t0, t1) = (pair[0], pair[1]);
                let e = eval(if t1 == b && jump { (b - 1e-5).max(a) } else { t1 });
                affected |= e.2 < e.1 - 1e-7;
                let risky = (e.0 - prev.0).abs() > 4.
                    || (e.1 - prev.1).abs() > 4.
                    || (e.2 - prev.2).abs() > 4.
                    || [75., 150.]
                        .iter()
                        .any(|q| (prev.3.abs() - q) * (e.3.abs() - q) <= 0. && (prev.3.abs() - e.3.abs()).abs() > 1e-9)
                    || [18., 36.]
                        .iter()
                        .any(|q| (prev.4.abs() - q) * (e.4.abs() - q) <= 0. && (prev.4.abs() - e.4.abs()).abs() > 1e-9)
                    || prev.3 * e.3 < 0.
                    || prev.4 * e.4 < 0.
                    || (e.3 - prev.3).abs() > 90.
                    || (e.4 - prev.4).abs() > 12.;
                if risky {
                    let mut k = (t0 * 60.).floor() as usize + 1;
                    while (k as f64 / 60.) < t1 - 1e-10 {
                        let t = k as f64 / 60.;
                        // Midpoints do not feed the refinement decision. If
                        // original alpha is exactly zero, geometry cannot change
                        // either the final alpha or the affected flag here.
                        if g.lines[i].value(3, t).clamp(0., 255.) == 0. {
                            points.push((t, 0.));
                        } else {
                            let v = eval(t);
                            affected |= v.2 < v.1 - 1e-7;
                            points.push((t, v.0));
                        }
                        k += 1;
                    }
                }
                points.push((t1, e.0));
                prev = e;
            }
            done = done.saturating_add(interval_work);
            for p in simplify(&points, 1.).windows(2) {
                events.push(event(bpm.to_beat(p[0].0), bpm.to_beat(p[1].0), p[0].1, p[1].1));
            }
        }
        if affected && !events.is_empty() {
            let l = &mut data["judgeLineList"][i];
            if !l["eventLayers"].is_array() || array(&l["eventLayers"]).is_empty() {
                l["eventLayers"] = json!([{}]);
            }
            let layers = l["eventLayers"].as_array_mut().unwrap();
            if layers[0].is_null() {
                layers[0] = json!({});
            }
            layers[0]["alphaEvents"] = Value::Array(events);
            for layer in &mut layers[1..] {
                if let Some(layer) = layer.as_object_mut() {
                    // The first layer now contains the combined final alpha.
                    // Omit other alpha tracks: an empty array becomes an empty
                    // chained animation in existing RPE readers and can panic.
                    layer.remove("alphaEvents");
                }
            }
        }
    }
    progress.finish(super::Stage::Fade);
    Ok(())
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    fn baked_alpha_omits_later_tracks_and_keeps_other_events() {
        let bpm = BpmMap::new(&json!([{"bpm":120,"startTime":[0,0,1]}])).unwrap();
        let mut line = super::super::static_line(vec![], 11., 4.);
        let extra = json!({"alphaEvents":[event(0.,4.,20.,20.)],
            "moveXEvents":[event(0.,4.,10.,10.)],"rotateEvents":[event(0.,4.,0.,0.)]});
        line["eventLayers"].as_array_mut().unwrap().push(extra.clone());
        let mut data = json!({"META":{"RPEVersion":160},"judgeLineList":[line]});
        let g = Geometry::new(&data, &bpm, false).unwrap();
        fade(&mut data, &bpm, &g, 2., &super::super::Progress::default()).unwrap();
        let layers = &data["judgeLineList"][0]["eventLayers"];
        assert!(!array(&layers[0]["alphaEvents"]).is_empty());
        assert!(layers[1].get("alphaEvents").is_none());
        assert_eq!(layers[1]["moveXEvents"], extra["moveXEvents"]);
        assert_eq!(layers[1]["rotateEvents"], extra["rotateEvents"]);
    }
    #[test]
    fn cached_world_preserves_parent_transform_operation_order() {
        let bpm = BpmMap::new(&json!([{"bpm":120,"startTime":[0,0,1]}])).unwrap();
        let mut parent = super::super::static_line(vec![], 11., 20.);
        parent["eventLayers"][0]["rotateEvents"] = json!([event(0.,20.,-35.,75.)]);
        parent["eventLayers"][0]["moveXEvents"] = json!([event(0.,20.,-170.,250.)]);
        let mut child = super::super::static_line(vec![], 11., 20.);
        child["father"] = json!(0); child["rotateWithFather"] = json!(true);
        let mut grandchild = child.clone(); grandchild["father"] = json!(1);
        let g = Geometry::new(&json!({"META":{"RPEVersion":160},"judgeLineList":[parent,child,grandchild]}), &bpm, false).unwrap();
        let mut cache = HashMap::new();
        for j in 0..12000 { let t = j as f64 / 100.; for i in [2,1,0,2] { assert_eq!(g.world(i,t),g.world_cached(i,t,&mut cache)); assert!(cache.len() <= 32768); } }
    }
    #[test]
    fn constant_detection_keeps_ramps_and_boundary_jumps() {
        let bpm = BpmMap::new(&json!([{"bpm":120,"startTime":[0,0,1]}])).unwrap();
        let constant = Track::new(&json!([event(0.,2.,10.,10.), event(2.,4.,20.,20.)]), &bpm).unwrap();
        assert!(constant.constant_between(0.,1.));
        assert!(!constant.constant_between(0.,2.));
        assert_eq!(constant.value(1.),20.);
        let ramp = Track::new(&json!([event(0.,4.,10.,20.)]), &bpm).unwrap();
        assert!(!ramp.constant_between(0.,1.));
    }
}
