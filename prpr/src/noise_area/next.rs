//! RPE-Next native noise-area data (format 220+, animation curves 250+).
//! Independent implementation of the file semantics. No editor code/assets are embedded.
use super::Pose;
use anyhow::{ensure, Context, Result};
use macroquad::prelude::vec2;
use nalgebra::{Matrix3, Point2};
use serde::Deserialize;

const EPS: f64 = 1e-9;
type Matrix = Matrix3<f64>;

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}
impl Position {
    fn validate(self) -> Result<()> {
        ensure!(self.x.is_finite() && self.y.is_finite(), "RPE-Next 噪域坐标必须为有限数值");
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct Beat([i64; 3]);
impl Beat {
    fn value(self) -> Result<f64> {
        ensure!(self.0[2] > 0 && self.0.iter().all(|v| v.unsigned_abs() <= 9_007_199_254_740_991), "RPE-Next 噪域拍数非法");
        Ok(self.0[0] as f64 + self.0[1] as f64 / self.0[2] as f64)
    }
}

#[derive(Debug)]
pub struct Tempo(Vec<(f64, f64, f64)>);
impl Tempo {
    pub fn new(points: impl IntoIterator<Item = (f64, f64)>) -> Result<Self> {
        let mut points: Vec<_> = points.into_iter().collect();
        ensure!(!points.is_empty(), "RPE-Next BPMList 不能为空");
        ensure!(points.iter().all(|(beat, bpm)| beat.is_finite() && bpm.is_finite() && *bpm > 0.), "RPE-Next BPM 非法");
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut time = 0.;
        let mut previous = 0.;
        let mut rate = 60. / points[0].1;
        let points = points
            .into_iter()
            .map(|(beat, bpm)| {
                time += (beat - previous) * rate;
                previous = beat;
                rate = 60. / bpm;
                (beat, time, rate)
            })
            .collect();
        Ok(Self(points))
    }
    fn time(&self, beat: Beat) -> Result<f64> {
        let beat = beat.value()?;
        let i = self.0.partition_point(|p| p.0 <= beat).saturating_sub(1);
        let (start, time, rate) = self.0[i];
        Ok(time + (beat - start) * rate)
    }
}

fn one() -> f64 {
    1.
}
fn easing_one() -> u8 {
    1
}
fn bezier_default() -> [f64; 4] {
    [0., 0., 1., 1.]
}
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Bool(v) => Ok(v),
        serde_json::Value::Number(v) if v.as_u64() == Some(0) => Ok(false),
        serde_json::Value::Number(v) if v.as_u64() == Some(1) => Ok(true),
        _ => Err(serde::de::Error::custom("RPE-Next 标志必须为 bool 或 0/1")),
    }
}
fn null_vec<'de, D, T>(d: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum Flash {
    Seconds(f64),
    Enabled(bool),
}
impl Flash {
    fn seconds(value: &Option<Self>) -> Result<f64> {
        Ok(match value {
            None => 0.5,
            Some(Self::Enabled(v)) => {
                if *v {
                    0.5
                } else {
                    0.
                }
            }
            Some(Self::Seconds(v)) => {
                ensure!(v.is_finite(), "RPE-Next readyFlash 非法");
                (v.max(0.) * 1000.).round() / 1000.
            }
        })
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Interval {
    start_time: Beat,
    end_time: Beat,
    ready_flash: Option<Flash>,
}
#[derive(Clone, Debug)]
struct Activation {
    start: f64,
    end: f64,
    flash: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct CurveNode {
    x: f64,
    y: f64,
    mode: Option<String>,
    #[serde(rename = "in")]
    incoming: Option<[f64; 2]>,
    out: Option<[f64; 2]>,
}
#[derive(Clone, Debug, Deserialize)]
struct Curve {
    version: u8,
    nodes: Vec<CurveNode>,
}
impl Curve {
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 1 && (2..=256).contains(&self.nodes.len()), "RPE-Next 自定义曲线版本/节点数非法");
        let first = &self.nodes[0];
        let last = self.nodes.last().unwrap();
        ensure!(first.x == 0. && first.y == 0. && last.x == 1. && last.y == 1., "RPE-Next 自定义曲线端点非法");
        for (i, n) in self.nodes.iter().enumerate() {
            ensure!(n.x.is_finite() && n.y.is_finite() && n.y.abs() <= 10000., "RPE-Next 自定义曲线数值非法");
            ensure!(i == 0 || n.x > self.nodes[i - 1].x, "RPE-Next 自定义曲线时间必须递增");
            ensure!(["auto", "smooth", "corner", "linear", "hold"].contains(&n.mode.as_deref().unwrap_or("corner")), "RPE-Next 自定义曲线模式非法");
            for (handle, lo, hi) in [
                (n.incoming, self.nodes[i.saturating_sub(1)].x, n.x),
                (n.out, n.x, self.nodes[(i + 1).min(self.nodes.len() - 1)].x),
            ] {
                if let Some([x, y]) = handle {
                    ensure!(x.is_finite() && y.is_finite() && y.abs() <= 10000. && x >= lo && x <= hi, "RPE-Next 曲线控制柄非法");
                }
            }
        }
        Ok(())
    }
    fn value(&self, x: f64) -> f64 {
        let x = x.clamp(0., 1.);
        if x == 0. || x == 1. {
            return x;
        }
        let i = self.nodes.partition_point(|n| n.x <= x).saturating_sub(1).min(self.nodes.len() - 2);
        let a = &self.nodes[i];
        let b = &self.nodes[i + 1];
        if x == a.x || a.mode.as_deref() == Some("hold") {
            return a.y;
        }
        if a.mode.as_deref() == Some("linear") {
            return a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x);
        }
        let handle = |index: usize, outgoing: bool| {
            let n = &self.nodes[index];
            if n.mode.as_deref() != Some("auto") {
                if let Some(p) = if outgoing { n.out } else { n.incoming } {
                    return p;
                }
            }
            let left = &self.nodes[index.saturating_sub(1)];
            let right = &self.nodes[(index + 1).min(self.nodes.len() - 1)];
            let dx = (b.x - a.x) / 3. * if outgoing { 1. } else { -1. };
            [n.x + dx, n.y + dx * (right.y - left.y) / (right.x - left.x)]
        };
        let c = handle(i, true);
        let d = handle(i + 1, false);
        let u = invert_cubic([a.x, c[0], d[0], b.x], x);
        cubic([a.y, c[1], d[1], b.y], u)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomEasing {
    curve: Curve,
    domain: Option<[f64; 2]>,
    source_start: Option<f64>,
    source_end: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    start_time: Beat,
    end_time: Beat,
    start: f64,
    end: f64,
    #[serde(default = "easing_one")]
    easing_type: u8,
    #[serde(default)]
    easing_left: f64,
    #[serde(default = "one")]
    easing_right: f64,
    #[serde(default, deserialize_with = "flag")]
    bezier: bool,
    #[serde(default = "bezier_default")]
    bezier_points: [f64; 4],
    #[serde(default, deserialize_with = "flag")]
    inst: bool,
    anchor: Option<Position>,
    transform_start: Option<f64>,
    transform_end: Option<f64>,
    #[serde(default)]
    transform_step: bool,
    custom_easing: Option<CustomEasing>,
    trajectory: Option<serde_json::Value>,
    #[serde(skip)]
    time: f64,
    #[serde(skip)]
    until: f64,
    #[serde(skip)]
    prefix: Matrix,
}
impl Event {
    fn value(&self, t: f64) -> f64 {
        let progress = if self.until <= self.time {
            1.
        } else {
            ((t - self.time) / (self.until - self.time)).clamp(0., 1.)
        };
        let (mut start, mut end) = (self.start, self.end);
        let amount = if let Some(custom) = &self.custom_easing {
            if let (Some(a), Some(b)) = (custom.source_start, custom.source_end) {
                start = a;
                end = b;
            }
            let [a, b] = custom.domain.unwrap_or([0., 1.]);
            custom.curve.value(a + (b - a) * progress)
        } else if self.bezier {
            if progress == 0. || progress == 1. {
                progress
            } else {
                let [x1, y1, x2, y2] = self.bezier_points;
                cubic([0., y1, y2, 1.], invert_cubic([0., x1, x2, 1.], progress))
            }
        } else {
            let a = standard_easing(self.easing_type, self.easing_left);
            let b = standard_easing(self.easing_type, self.easing_right);
            if (b - a).abs() < 1e-12 {
                progress
            } else {
                (standard_easing(self.easing_type, self.easing_left + (self.easing_right - self.easing_left) * progress) - a) / (b - a)
            }
        };
        start + (end - start) * amount
    }
}

fn cubic(p: [f64; 4], t: f64) -> f64 {
    let u = 1. - t;
    p[0] * u * u * u + 3. * p[1] * u * u * t + 3. * p[2] * u * t * t + p[3] * t * t * t
}
fn invert_cubic(p: [f64; 4], x: f64) -> f64 {
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..48 {
        let mid = (lo + hi) * 0.5;
        if cubic(p, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}
fn standard_easing(kind: u8, x: f64) -> f64 {
    use std::f64::consts::PI;
    let bounce = |x: f64| {
        let (x, add) = if x < 1. / 2.75 {
            (x, 0.)
        } else if x < 2. / 2.75 {
            (x - 1.5 / 2.75, 0.75)
        } else if x < 2.5 / 2.75 {
            (x - 2.25 / 2.75, 0.9375)
        } else {
            (x - 2.625 / 2.75, 0.984375)
        };
        7.5625 * x * x + add
    };
    let in_out = |power: i32| {
        if x < 0.5 {
            (2. * x).powi(power) * 0.5
        } else {
            1. - (2. - 2. * x).powi(power) * 0.5
        }
    };
    match kind {
        2 => (x * PI * 0.5).sin(),
        3 => 1. - (x * PI * 0.5).cos(),
        4 => 1. - (1. - x).powi(2),
        5 => x * x,
        6 => (1. - (PI * x).cos()) * 0.5,
        7 => in_out(2),
        8 => 1. - (1. - x).powi(3),
        9 => x.powi(3),
        10 => 1. - (1. - x).powi(4),
        11 => x.powi(4),
        12 => in_out(3),
        13 => in_out(4),
        14 => 1. - (1. - x).powi(5),
        15 => x.powi(5),
        16 => {
            if x == 1. {
                1.
            } else {
                1. - 2f64.powf(-10. * x)
            }
        }
        17 => {
            if x == 0. {
                0.
            } else {
                2f64.powf(10. * x - 10.)
            }
        }
        18 => (1. - (x - 1.).powi(2)).sqrt(),
        19 => 1. - (1. - x * x).abs().sqrt(),
        20 => 1. + 2.70158 * (x - 1.).powi(3) + 1.70158 * (x - 1.).powi(2),
        21 => 2.70158 * x.powi(3) - 1.70158 * x.powi(2),
        22 => {
            if x < 0.5 {
                (1. - (1. - (2. * x).powi(2)).sqrt()) * 0.5
            } else {
                ((1. - (2. - 2. * x).powi(2)).sqrt() + 1.) * 0.5
            }
        }
        23 => {
            let c = 1.70158 * 1.525;
            if x < 0.5 {
                (2. * x).powi(2) * ((c + 1.) * 2. * x - c) * 0.5
            } else {
                ((2. * x - 2.).powi(2) * ((c + 1.) * (2. * x - 2.) + c) + 2.) * 0.5
            }
        }
        24 | 25 | 29 if x == 0. || x == 1. => x,
        24 => 2f64.powf(-10. * x) * ((10. * x - 0.75) * 2. * PI / 3.).sin() + 1.,
        25 => -2f64.powf(10. * x - 10.) * ((10. * x - 10.75) * 2. * PI / 3.).sin(),
        26 => bounce(x),
        27 => 1. - bounce(1. - x),
        28 => {
            if x < 0.5 {
                (1. - bounce(1. - 2. * x)) * 0.5
            } else {
                (1. + bounce(2. * x - 1.)) * 0.5
            }
        }
        29 => {
            let wave = ((20. * x - 11.125) * 2. * PI / 4.5).sin();
            if x < 0.5 {
                -2f64.powf(20. * x - 10.) * wave * 0.5
            } else {
                2f64.powf(-20. * x + 10.) * wave * 0.5 + 1.
            }
        }
        _ => x,
    }
}

fn anchored(kind: usize, start: f64, value: f64, p: Position) -> Matrix {
    let (a, b, c, d) = if kind == 2 {
        let (s, c) = (value - start).to_radians().sin_cos();
        (c, -s, s, c)
    } else {
        let ratio = if start == 0. { 1. } else { value / start };
        let ratio = if ratio.is_finite() { ratio } else { 1. };
        if kind == 3 {
            (ratio, 0., 0., 1.)
        } else {
            (1., 0., 0., ratio)
        }
    };
    Matrix::new(a, c, p.x - a * p.x - c * p.y, b, d, p.y - b * p.x - d * p.y, 0., 0., 1.)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Definition {
    bottom_left: Position,
    top_right: Position,
    appear_time: Beat,
    disappear_time: Beat,
    #[serde(default, deserialize_with = "null_vec")]
    active_intervals: Vec<Interval>,
    #[serde(default)]
    is_invert: bool,
    #[serde(default = "default_layer")]
    layer_id: String,
    #[serde(default, deserialize_with = "null_vec")]
    move_x_events: Vec<Event>,
    #[serde(default, deserialize_with = "null_vec")]
    move_y_events: Vec<Event>,
    #[serde(default, deserialize_with = "null_vec")]
    rotate_events: Vec<Event>,
    #[serde(default, deserialize_with = "null_vec")]
    scale_x_events: Vec<Event>,
    #[serde(default, deserialize_with = "null_vec")]
    scale_y_events: Vec<Event>,
}
pub fn default_layer() -> String {
    "default".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    HiddenBefore,
    Disabled,
    Ready,
    Active,
    HiddenAfter,
}
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub state: State,
    pub mix: f32,
    pub pose: Pose,
}
#[derive(Clone, Debug)]
pub struct NextArea {
    pub layer: usize,
    pub invert: bool,
    pub appear: f64,
    pub disappear: f64,
    center: Position,
    size: Position,
    intervals: Vec<Activation>,
    activations: Vec<Activation>,
    tracks: [Vec<Event>; 5],
}
impl NextArea {
    pub fn parse(value: serde_json::Value, tempo: &Tempo, layers: &[String]) -> Result<Self> {
        let d: Definition = serde_json::from_value(value).context("RPE-Next 噪域格式错误")?;
        d.bottom_left.validate()?;
        d.top_right.validate()?;
        let layer = layers.iter().position(|v| *v == d.layer_id).context("RPE-Next 噪域引用了不存在的层")?;
        let appear = tempo.time(d.appear_time)?;
        let disappear = tempo.time(d.disappear_time)?;
        ensure!(appear <= disappear, "RPE-Next 噪域生命周期反向");
        let mut intervals = Vec::<Activation>::new();
        let mut original_intervals = Vec::new();
        for v in d.active_intervals {
            let a = tempo.time(v.start_time)?;
            let b = tempo.time(v.end_time)?;
            ensure!(a <= b, "RPE-Next 噪域激活区间反向");
            if b > a {
                original_intervals.push((a, b));
            }
            let start = appear.max(a);
            let end = disappear.min(b);
            let flash = Flash::seconds(&v.ready_flash)?;
            if end > start || a == b && start == end {
                intervals.push(Activation { start, end, flash });
            }
        }
        original_intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure!(original_intervals.windows(2).all(|p| p[0].1 <= p[1].0), "RPE-Next 激活区间重叠");
        intervals.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut merged = Vec::<Activation>::new();
        for v in intervals {
            if let Some(last) = merged.last_mut().filter(|last| last.start == v.start) {
                last.end = last.end.max(v.end);
                last.flash = last.flash.max(v.flash);
            } else {
                merged.push(v);
            }
        }
        let activations: Vec<_> = merged.iter().filter(|v| v.end > v.start).cloned().collect();
        ensure!(activations.windows(2).all(|p| p[0].end <= p[1].start), "RPE-Next 激活区间重叠");
        let center = Position {
            x: (d.bottom_left.x + d.top_right.x) * 0.5,
            y: (d.bottom_left.y + d.top_right.y) * 0.5,
        };
        let size = Position {
            x: (d.top_right.x - d.bottom_left.x).abs(),
            y: (d.top_right.y - d.bottom_left.y).abs(),
        };
        let mut tracks = [d.move_x_events, d.move_y_events, d.rotate_events, d.scale_x_events, d.scale_y_events];
        for (kind, track) in tracks.iter_mut().enumerate() {
            for e in track.iter_mut() {
                e.time = tempo.time(e.start_time)?;
                e.until = tempo.time(e.end_time)?;
                ensure!(e.time <= e.until && e.start.is_finite() && e.end.is_finite(), "RPE-Next 数值事件非法");
                ensure!((1..=29).contains(&e.easing_type), "RPE-Next easingType 非法");
                ensure!(
                    e.easing_left.is_finite()
                        && e.easing_right.is_finite()
                        && e.easing_left >= 0.
                        && e.easing_right <= 1.
                        && e.easing_left < e.easing_right,
                    "RPE-Next 缓动裁切非法"
                );
                ensure!(!e.inst || e.start == e.end, "RPE-Next 钩定事件端值必须一致");
                ensure!(e.trajectory.is_none(), "RPE-Next 噪域中的空间轨迹扩展尚不支持");
                if e.bezier {
                    ensure!(
                        e.bezier_points.iter().all(|v| v.is_finite())
                            && (0. ..=1.).contains(&e.bezier_points[0])
                            && (0. ..=1.).contains(&e.bezier_points[2]),
                        "RPE-Next Bezier 非法"
                    );
                }
                if kind >= 2 {
                    e.anchor.context("RPE-Next 旋转/缩放缺少锚点")?.validate()?;
                }
                ensure!(!e.transform_step || kind >= 2 && e.time == e.until, "RPE-Next transformStep 非法");
                for v in [e.transform_start, e.transform_end].into_iter().flatten() {
                    ensure!(kind >= 2 && v.is_finite(), "RPE-Next 变换基准非法");
                }
                if let Some(c) = &e.custom_easing {
                    c.curve.validate()?;
                    let [a, b] = c.domain.unwrap_or([0., 1.]);
                    ensure!(a.is_finite() && b.is_finite() && 0. <= a && a < b && b <= 1., "RPE-Next 自定义曲线 domain 非法");
                    ensure!(
                        c.source_start.is_some() == c.source_end.is_some() && c.source_start.iter().chain(c.source_end.iter()).all(|v| v.is_finite()),
                        "RPE-Next 自定义曲线源端值非法"
                    );
                }
            }
            track.sort_by(|a, b| a.time.total_cmp(&b.time));
            for pair in track.windows(2) {
                ensure!(
                    pair[0].until <= pair[1].time
                        && (pair[0].time != pair[1].time || kind >= 2 && pair[0].transform_step && pair[0].until == pair[0].time),
                    "RPE-Next 数值事件重叠"
                );
            }
            let mut prefix = Matrix::identity();
            for i in 0..track.len() {
                if i > 0 && kind >= 2 {
                    let p = &track[i - 1];
                    let anchor = p.anchor.unwrap_or(center);
                    let endpoint = p.transform_end.unwrap_or(p.end);
                    prefix = anchored(kind, p.transform_start.unwrap_or(p.start), endpoint, anchor) * prefix;
                    prefix = anchored(kind, endpoint, track[i].transform_start.unwrap_or(track[i].start), anchor) * prefix;
                }
                track[i].prefix = prefix;
            }
        }
        Ok(Self {
            layer,
            invert: d.is_invert,
            appear,
            disappear,
            center,
            size,
            intervals: merged,
            activations,
            tracks,
        })
    }
    pub fn phase(&self, t: f64) -> (State, f32) {
        let current = self
            .activations
            .partition_point(|v| v.start <= t + EPS)
            .checked_sub(1)
            .map(|i| &self.activations[i]);
        let next = self.intervals.get(self.intervals.partition_point(|v| v.start <= t + EPS));
        let state = if t + EPS < self.appear {
            State::HiddenBefore
        } else if t + EPS >= self.disappear {
            State::HiddenAfter
        } else if current.is_some_and(|v| t + EPS < v.end) {
            State::Active
        } else if next.is_some_and(|v| v.flash > 0. && t + EPS >= v.start - v.flash) {
            State::Ready
        } else {
            State::Disabled
        };
        let mix = if state == State::Active {
            1.
        } else if state == State::Ready {
            let v = next.unwrap();
            let p = ((t - v.start + v.flash) / v.flash).clamp(0., 1.);
            ((1. + (p * std::f64::consts::PI * 6.).cos()) * 0.5) as f32
        } else {
            0.
        };
        (state, mix)
    }
    pub fn sample(&self, t: f64, aspect: f32) -> Sample {
        let (state, mix) = self.phase(t);
        let defaults = [self.center.x, self.center.y, 0., 1., 1.];
        let mut values = defaults;
        let mut center = Point2::new(self.center.x, self.center.y);
        let mut entries: [Option<&Event>; 5] = [None; 5];
        for (kind, track) in self.tracks.iter().enumerate() {
            if let Some(i) = track.partition_point(|e| e.time <= t + EPS).checked_sub(1) {
                let e = &track[i];
                values[kind] = e.value(t);
                entries[kind] = Some(e);
            }
        }
        for kind in [3, 4, 2] {
            if let Some(e) = entries[kind] {
                center = (anchored(kind, e.transform_start.unwrap_or(e.start), values[kind], e.anchor.unwrap_or(self.center)) * e.prefix)
                    .transform_point(&center);
            }
        }
        center.x += values[0] - self.center.x;
        center.y += values[1] - self.center.y;
        // RPE's fixed 1350x900 chart plane, converted to the game's noise units.
        let unit_x = 10. * aspect as f64 / crate::parse::RPE_WIDTH as f64;
        let unit_y = 10. / crate::parse::RPE_HEIGHT as f64;
        Sample {
            state,
            mix,
            pose: Pose {
                center: vec2((center.x * unit_x) as f32, (center.y * unit_y) as f32),
                size: vec2((self.size.x * values[3] * unit_x).abs() as f32, (self.size.y * values[4] * unit_y).abs() as f32),
                angle: -values[2].to_radians() as f32,
            },
        }
    }
}

pub fn is_native(value: &serde_json::Value) -> bool {
    ["bottomLeft", "topRight", "activeIntervals", "isInvert", "layerId"]
        .iter()
        .any(|key| value.get(key).is_some())
}

pub fn parse_layers(value: Option<&serde_json::Value>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(vec![default_layer()]);
    };
    let layers = value.as_array().context("RPE-Next noiseAreaLayers 必须是数组")?;
    ensure!(!layers.is_empty(), "RPE-Next noiseAreaLayers 不能为空");
    let mut ids = Vec::new();
    for layer in layers {
        let id = layer
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .context("RPE-Next 噪域层缺少 id")?;
        ensure!(!ids.iter().any(|v| v == id), "RPE-Next 噪域层 id 重复");
        ids.push(id.to_owned());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    pub fn fixture() -> Value {
        json!({"bottomLeft":{"x":-250,"y":-150},"topRight":{"x":250,"y":150},
            "appearTime":[0,0,1],"disappearTime":[16,0,1],
            "activeIntervals":[{"startTime":[4,0,1],"endTime":[8,0,1]}]})
    }
    #[test]
    fn matches_unmodified_rpe_next_reference_values() {
        let reference: Value = serde_json::from_str(include_str!("../../tests/fixtures/rpe_next_noise_reference.json")).unwrap();
        let mut count = 0;
        for case in reference["cases"].as_array().unwrap() {
            let tempo = Tempo::new(case["bpm"].as_array().unwrap().iter().map(|v| {
                let b: Beat = serde_json::from_value(v["startTime"].clone()).unwrap();
                (b.value().unwrap(), v["bpm"].as_f64().unwrap())
            }))
            .unwrap();
            let area = NextArea::parse(case["area"].clone(), &tempo, &[default_layer()]).unwrap();
            for expected in case["samples"].as_array().unwrap() {
                let t = expected["time"].as_f64().unwrap();
                let aspect = expected["aspect"].as_f64().unwrap() as f32;
                let sample = area.sample(t, aspect);
                let state = match sample.state {
                    State::HiddenBefore => "hiddenBefore",
                    State::HiddenAfter => "hiddenAfter",
                    State::Active => "active",
                    State::Ready => "ready",
                    State::Disabled => "disabled",
                };
                assert_eq!(state, expected["state"], "{} at {t}", case["name"]);
                assert!((sample.mix as f64 - expected["mix"].as_f64().unwrap()).abs() < 1e-6, "{} mix at {t}", case["name"]);
                let expected_point = |p: &Value| vec2(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32);
                assert!(
                    (sample.pose.center - expected_point(&expected["center"])).length() < 3e-5,
                    "{} center at {t}: {:?} vs {}",
                    case["name"],
                    sample.pose.center,
                    expected["center"]
                );
                for point in sample.pose.corners() {
                    let error = expected["points"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| (point - expected_point(p)).length())
                        .fold(f32::INFINITY, f32::min);
                    assert!(error < 3e-5, "{} corners at {t}, aspect={aspect}, error={error}", case["name"]);
                }
                count += 1;
            }
        }
        assert_eq!(count, 2984);
    }
    #[test]
    fn rejects_invalid_beats_layers_and_animations() {
        let tempo = Tempo::new([(0., 120.)]).unwrap();
        for path in ["appearTime", "disappearTime"] {
            let mut bad = fixture();
            bad[path] = json!([0, 0, 0]);
            assert!(NextArea::parse(bad, &tempo, &[default_layer()]).is_err());
        }
        let mut bad = fixture();
        bad["layerId"] = json!("missing");
        assert!(NextArea::parse(bad, &tempo, &[default_layer()]).is_err());
        let mut bad = fixture();
        bad["rotateEvents"] = json!([{"startTime":[0,0,1],"endTime":[2,0,1],"start":0,"end":90}]);
        assert!(NextArea::parse(bad, &tempo, &[default_layer()]).is_err());
        for value in [json!([]), json!([{"id":"x"},{"id":"x"}]), json!(true)] {
            assert!(parse_layers(Some(&value)).is_err());
        }
        assert!(Tempo::new([(0., 0.)]).is_err());
        let mut overlap = fixture();
        overlap["activeIntervals"] = json!([
            {"startTime":[1,0,1],"endTime":[5,0,1]},
            {"startTime":[1,0,1],"endTime":[3,0,1]}]);
        assert!(NextArea::parse(overlap, &tempo, &[default_layer()]).is_err());
    }
    #[test]
    fn optional_null_tracks_and_nonzero_first_bpm_match_next_defaults() {
        let tempo = Tempo::new([(2., 120.), (4., 60.)]).unwrap();
        assert_eq!(tempo.time(Beat([0, 0, 1])).unwrap(), 0.);
        assert_eq!(tempo.time(Beat([6, 0, 1])).unwrap(), 4.);
        let mut value = fixture();
        for field in [
            "activeIntervals",
            "moveXEvents",
            "moveYEvents",
            "rotateEvents",
            "scaleXEvents",
            "scaleYEvents",
        ] {
            value[field] = Value::Null;
        }
        let next = NextArea::parse(value, &tempo, &[default_layer()]).unwrap();
        assert_eq!(next.phase(1.).0, State::Disabled);
        assert_eq!(next.sample(1., 1.5).pose.center, vec2(0., 0.));
    }
    #[test]
    fn native_masks_are_independent_per_layer_and_keep_touch_latches() {
        use super::super::{blocked, BlockArea, NoiseAreaState};
        let tempo = Tempo::new([(0., 120.)]).unwrap();
        let make = |invert, layer| {
            let mut definition = fixture();
            definition["isInvert"] = json!(invert);
            definition["layerId"] = json!(if layer == 0 { "a" } else { "b" });
            let area = NextArea::parse(definition, &tempo, &["a".into(), "b".into()]).unwrap();
            BlockArea {
                is_subtract: invert,
                next: Some(Box::new(area)),
                ..Default::default()
            }
        };
        let p = vec2(0., 0.);
        for inverses in 0..=4 {
            let mut areas = vec![make(false, 0)];
            areas.extend((0..inverses).map(|_| make(true, 0)));
            assert_eq!(blocked(&areas, p, 2.5, 1.5), inverses != 1);
        }
        let areas = [make(false, 0), make(true, 1)];
        assert!(blocked(&areas, p, 2.5, 1.5));
        assert!(!blocked(&areas, p, 1., 1.5));
        let mut state = NoiseAreaState::default();
        state.update(&areas, &[(7, p)], 2.5, 1.5, true, 0.016);
        assert!(state.is_blocked(7));
        state.update(&areas, &[(7, vec2(50., 50.))], 4.1, 1.5, true, 0.016);
        assert!(state.is_blocked(7));
        state.release_finger(7);
        assert!(!state.is_blocked(7));
    }
}
