//! Official block-area and Recorder carrier (噪域) data and hit testing.
//!
//! Both formats share gameplay switches, touch latches and effect rendering;
//! Recorder carriers add seekable lifecycle markers and RPE animation snapshots.
use macroquad::prelude::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoiseAreaConfig {
    pub enabled: bool,
    pub precise_edges: bool,
    pub remove_distortion: bool,
    pub music_unaffected: bool,
    pub low_performance: bool,
}

impl Default for NoiseAreaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            precise_edges: false,
            remove_distortion: false,
            music_unaffected: false,
            low_performance: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockArea {
    #[serde(skip)]
    pub recorder: Option<Box<recorder::RecorderArea>>,
    pub top_right_percentage: Point,
    pub bottom_left_percentage: Point,
    pub appear_time: f32,
    pub enable_time: f32,
    pub disable_time: f32,
    pub disappear_time: f32,
    pub is_subtract: bool,
    pub rotate_events: Vec<BlockRotateEvent>,
    pub move_events: Vec<BlockMoveEvent>,
    pub scale_events: Vec<BlockScaleEvent>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRotateEvent {
    pub anchor: Point,
    pub time: f32,
    pub rotation: f32,
    #[serde(default)]
    pub ease_type: i32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockMoveEvent {
    pub end_position: Point,
    pub time: f32,
    #[serde(default)]
    pub ease_type_x: i32,
    #[serde(default)]
    pub ease_type_y: i32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockScaleEvent {
    pub anchor: Point,
    pub time: f32,
    pub scale: Point,
    #[serde(default)]
    pub ease_type_x: i32,
    #[serde(default)]
    pub ease_type_y: i32,
}

pub mod audio;
mod perf;
pub mod recorder;
pub mod render;

fn ease_raw(n: i32, t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    use std::f32::consts::PI;
    match n {
        1 => 1. - (t * PI / 2.).cos(),
        2 => (t * PI / 2.).sin(),
        3 => (1. - (PI * t).cos()) / 2.,
        4 => t * t,
        5 => 1. - (1. - t).powi(2),
        6 => {
            if t < 0.5 {
                2. * t * t
            } else {
                1. - (-2. * t + 2.).powi(2) / 2.
            }
        }
        7 => t.powi(3),
        8 => 1. - (1. - t).powi(3),
        9 => {
            if t < 0.5 {
                4. * t.powi(3)
            } else {
                1. - (-2. * t + 2.).powi(3) / 2.
            }
        }
        10 => t.powi(4),
        11 => 1. - (1. - t).powi(4),
        12 => {
            if t < 0.5 {
                8. * t.powi(4)
            } else {
                1. - (-2. * t + 2.).powi(4) / 2.
            }
        }
        13 => 0.,
        14 => 1.,
        _ => t,
    }
}
fn ease(n: i32, t: f32) -> f32 {
    let p = t.clamp(0., 1.) * 100.;
    let i = p.floor();
    let a = ease_raw(n, i / 100.);
    a + (ease_raw(n, (i + 1.).min(100.) / 100.) - a) * (p - i)
}
fn ratio(t: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        1.
    } else {
        ((t - a) / (b - a)).clamp(0., 1.)
    }
}
fn world(p: Point, aspect: f32) -> Vec2 {
    Vec2::new((p.x - 0.5) * 10. * aspect, (p.y - 0.5) * 10.)
}
fn rotate(p: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    Vec2::new(c * p.x - s * p.y, s * p.x + c * p.y)
}
fn safe_div(a: f32, b: f32) -> f32 {
    if b.abs() < f32::from_bits(8) {
        1.
    } else {
        a / b
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub center: Vec2,
    pub size: Vec2,
    pub angle: f32,
}
impl Pose {
    pub fn contains(self, p: Vec2, adjusted: bool, subtract: bool) -> bool {
        if self.size.x.abs() < 0.0001 || self.size.y.abs() < 0.0001 {
            return false;
        }
        let local = rotate(p - self.center, -self.angle) / self.size;
        let sign = if subtract { 1. } else { -1. };
        let half = if adjusted {
            Vec2::splat(0.5) + Vec2::new((0.3 / self.size.x.abs()).min(0.25), (0.3 / self.size.y.abs()).min(0.25)) * sign
        } else {
            Vec2::splat(0.5)
        };
        local.x.abs() <= half.x && local.y.abs() <= half.y
    }
    pub fn corners(self) -> [Vec2; 4] {
        [Vec2::new(-0.5, -0.5), Vec2::new(0.5, -0.5), Vec2::new(0.5, 0.5), Vec2::new(-0.5, 0.5)]
            .map(|p| self.center + rotate(p * self.size, self.angle))
    }
}
impl BlockArea {
    pub fn normalize(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            [
                self.appear_time,
                self.enable_time,
                self.disable_time,
                self.disappear_time,
                self.top_right_percentage.x,
                self.top_right_percentage.y,
                self.bottom_left_percentage.x,
                self.bottom_left_percentage.y
            ]
            .iter()
            .all(|v| v.is_finite()),
            "噪域坐标/时间必须是有限数值"
        );
        self.enable_time = self.enable_time.max(self.appear_time);
        self.disable_time = self.disable_time.max(self.enable_time);
        self.disappear_time = self.disappear_time.max(self.disable_time);
        for e in &self.move_events {
            anyhow::ensure!(
                [e.time, e.end_position.x, e.end_position.y].iter().all(|v| v.is_finite())
                    && (0..=14).contains(&e.ease_type_x)
                    && (0..=14).contains(&e.ease_type_y),
                "噪域移动事件非法或缺少自定义 AnimationCurve 数据"
            );
        }
        for e in &self.scale_events {
            anyhow::ensure!(
                [e.time, e.anchor.x, e.anchor.y, e.scale.x, e.scale.y].iter().all(|v| v.is_finite())
                    && (0..=14).contains(&e.ease_type_x)
                    && (0..=14).contains(&e.ease_type_y),
                "噪域缩放事件非法或缺少自定义 AnimationCurve 数据"
            );
        }
        for e in &self.rotate_events {
            anyhow::ensure!(
                [e.time, e.anchor.x, e.anchor.y, e.rotation].iter().all(|v| v.is_finite()) && (0..=14).contains(&e.ease_type),
                "噪域旋转事件非法或缺少自定义 AnimationCurve 数据"
            );
        }
        self.move_events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.scale_events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.rotate_events.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(())
    }
    pub fn active(&self, t: f32) -> bool {
        if let Some(r) = &self.recorder {
            return r.state(t as f64) == 2;
        }
        t >= self.enable_time && t < self.disable_time
    }
    pub fn visible(&self, t: f32) -> bool {
        if let Some(r) = &self.recorder {
            return (1..=3).contains(&r.state(t as f64));
        }
        t >= self.appear_time && t < self.disappear_time
    }
    pub fn ready(&self, t: f32) -> bool {
        if let Some(r) = &self.recorder {
            return r.ready(t as f64);
        }
        t >= self.enable_time - 0.5 && t < self.enable_time
    }
    pub fn fade(&self, t: f32) -> f32 {
        if let Some(r) = &self.recorder {
            return r.fade(t as f64);
        }
        if !self.active(t) && !self.ready(t) && t < self.enable_time {
            ((t - self.appear_time) / 0.5).clamp(0., 1.)
        } else {
            1.
        }
    }
    pub fn pose(&self, t: f32, aspect: f32) -> Pose {
        if let Some(r) = &self.recorder {
            return r.pose;
        }
        let lo = world(self.bottom_left_percentage, aspect);
        let hi = world(self.top_right_percentage, aspect);
        let base = (lo + hi) * 0.5;
        let mut center = base;
        let mut scale = Vec2::ONE;
        if let Some(first) = self.scale_events.first() {
            scale = Vec2::new(first.scale.x, first.scale.y);
            for (i, e) in self.scale_events.iter().enumerate() {
                let Some(next) = self.scale_events.get(i + 1) else { break };
                let p = ratio(t, e.time, next.time);
                let completed = t >= next.time;
                let v = Vec2::new(
                    e.scale.x + (next.scale.x - e.scale.x) * if completed { 1. } else { ease(e.ease_type_x, p) },
                    e.scale.y + (next.scale.y - e.scale.y) * if completed { 1. } else { ease(e.ease_type_y, p) },
                );
                let anchor = world(e.anchor, aspect);
                let rel = Vec2::new(safe_div(v.x, e.scale.x), safe_div(v.y, e.scale.y));
                center = anchor + (center - anchor) * rel;
                scale = v;
                if t < next.time {
                    break;
                }
            }
        }
        let mut angle = 0.;
        if let Some(first) = self.rotate_events.first() {
            angle = first.rotation;
            for (i, e) in self.rotate_events.iter().enumerate() {
                let Some(next) = self.rotate_events.get(i + 1) else { break };
                let v = e.rotation
                    + (next.rotation - e.rotation)
                        * if t >= next.time {
                            1.
                        } else {
                            ease(e.ease_type, ratio(t, e.time, next.time))
                        };
                let anchor = world(e.anchor, aspect);
                center = anchor + rotate(center - anchor, (v - e.rotation).to_radians());
                angle = v;
                if t < next.time {
                    break;
                }
            }
        }
        if let Some(first) = self.move_events.first() {
            let mut v = first.end_position;
            for (i, e) in self.move_events.iter().enumerate() {
                let Some(next) = self.move_events.get(i + 1) else { break };
                let p = ratio(t, e.time, next.time);
                let completed = t >= next.time;
                v = Point {
                    x: e.end_position.x + (next.end_position.x - e.end_position.x) * if completed { 1. } else { ease(e.ease_type_x, p) },
                    y: e.end_position.y + (next.end_position.y - e.end_position.y) * if completed { 1. } else { ease(e.ease_type_y, p) },
                };
                if t < next.time {
                    break;
                }
            }
            center += world(v, aspect) - base;
        }
        Pose {
            center,
            size: ((hi - lo) * scale).abs(),
            angle: angle.to_radians(),
        }
    }
}
pub(crate) fn blocked(areas: &[BlockArea], p: Vec2, t: f32, aspect: f32) -> bool {
    let (mut a, mut b, mut c, mut d) = (false, false, false, false);
    let (mut rn, mut rs, mut ran, mut ras) = (false, 0usize, false, 0usize);
    for area in areas.iter().filter(|a| a.active(t)) {
        let pose = area.pose(t, aspect);
        if area.recorder.is_some() {
            if pose.contains(p, false, area.is_subtract) {
                if area.is_subtract {
                    rs += 1
                } else {
                    rn = true
                }
            }
            if pose.contains(p, true, area.is_subtract) {
                if area.is_subtract {
                    ras += 1
                } else {
                    ran = true
                }
            }
            continue;
        }
        if pose.contains(p, false, area.is_subtract) {
            if area.is_subtract {
                b ^= true
            } else {
                a = true
            }
        }
        if pose.contains(p, true, area.is_subtract) {
            if area.is_subtract {
                d ^= true
            } else {
                c = true
            }
        }
    }
    ((a ^ b) && (c ^ d)) || ((rn ^ (rs == 1)) && (ran ^ (ras == 1)))
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Hover {
    pub finger: Option<u64>,
    pub position: Vec2,
    pub scale: f32,
    start: f32,
    target: f32,
    elapsed: f32,
    moving: bool,
}
impl Hover {
    fn animate(&mut self, target: f32, show: bool) {
        self.start = if show { 0. } else { self.scale };
        self.scale = self.start;
        self.target = target;
        self.elapsed = 0.;
        self.moving = true;
    }
    fn tick(&mut self, dt: f32) {
        if self.moving {
            let p = (self.elapsed / 0.1).clamp(0., 1.);
            self.scale = self.start + (self.target - self.start) * p;
            self.elapsed += dt.max(0.);
            if p >= 1. {
                self.moving = false
            }
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct NoiseAreaState {
    pub blocked_ids: std::collections::HashSet<u64>,
    pub hovers: [Hover; 10],
    pub positions: Vec<Vec2>,
}
impl NoiseAreaState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn update(&mut self, areas: &[BlockArea], touches: &[(u64, Vec2)], t: f32, aspect: f32, enabled: bool, dt: f32) {
        if !enabled {
            self.clear();
            return;
        }
        self.blocked_ids.retain(|id| touches.iter().any(|(i, _)| i == id));
        self.positions.clear();
        let mut seen = [false; 10];
        for &(id, p) in touches {
            if self.blocked_ids.contains(&id) || blocked(areas, p, t, aspect) {
                self.blocked_ids.insert(id);
                self.positions.push(p);
                let slot = self
                    .hovers
                    .iter()
                    .position(|h| h.finger == Some(id))
                    .or_else(|| self.hovers.iter().position(|h| h.finger.is_none()));
                if let Some(i) = slot {
                    let h = &mut self.hovers[i];
                    if h.finger != Some(id) {
                        h.finger = Some(id);
                        h.animate(11.5, true)
                    }
                    h.position = p;
                    seen[i] = true;
                }
            }
        }
        for (i, h) in self.hovers.iter_mut().enumerate() {
            if h.finger.is_some() && !seen[i] {
                h.finger = None;
                h.animate(0., false)
            }
            h.tick(dt)
        }
    }
    pub fn is_blocked(&self, id: u64) -> bool {
        self.blocked_ids.contains(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec2;
    fn area(sub: bool) -> BlockArea {
        BlockArea {
            bottom_left_percentage: Point { x: 0.2, y: 0.2 },
            top_right_percentage: Point { x: 0.8, y: 0.8 },
            enable_time: 1.,
            disable_time: 3.,
            disappear_time: 4.,
            is_subtract: sub,
            ..Default::default()
        }
    }
    #[test]
    fn union_parity_and_edge_protection() {
        let a = area(false);
        assert!(blocked(&[a.clone(), a.clone()], Vec2::ZERO, 2., 1.));
        assert!(!blocked(&[a.clone(), area(true)], Vec2::ZERO, 2., 1.));
        assert!(blocked(&[a.clone(), area(true), area(true)], Vec2::ZERO, 2., 1.));
        assert!(!blocked(&[a], vec2(2.9, 0.), 2., 1.));
        assert!(blocked(&[area(true)], Vec2::ZERO, 2., 1.));
    }
    #[test]
    fn persists_outside_until_release() {
        let mut s = NoiseAreaState::default();
        s.update(&[area(false)], &[(7, Vec2::ZERO)], 2., 1., true, 0.016);
        assert!(s.is_blocked(7));
        s.update(&[area(false)], &[(7, vec2(20., 20.))], 5., 1., true, 0.016);
        assert!(s.is_blocked(7));
        assert_eq!(s.hovers[0].position, vec2(20., 20.));
        s.update(&[], &[], 5., 1., true, 0.016);
        assert!(!s.is_blocked(7));
        assert!(s.hovers[0].scale > 0.);
        for _ in 0..10 {
            s.update(&[], &[], 5., 1., true, 0.016);
        }
        assert_eq!(s.hovers[0].scale, 0.);
    }
    #[test]
    fn ten_visual_slots_and_reuse() {
        let mut s = NoiseAreaState::default();
        let touches = (0..11).map(|i| (i, Vec2::ZERO)).collect::<Vec<_>>();
        s.update(&[area(false)], &touches, 2., 1., true, 0.05);
        assert_eq!(s.blocked_ids.len(), 11);
        assert_eq!(s.hovers.iter().filter(|h| h.finger.is_some()).count(), 10);
        s.update(&[], &[], 2., 1., true, 0.05);
        s.update(&[area(false)], &[(50, Vec2::ZERO)], 2., 1., true, 0.05);
        assert_eq!(s.hovers[0].finger, Some(50));
        assert_eq!(s.hovers[0].scale, 0.);
        s.update(&[], &[(50, Vec2::ZERO)], 2., 1., false, 0.05);
        assert!(s.blocked_ids.is_empty());
    }
    #[test]
    fn animation_and_custom_curve_validation() {
        let mut a = area(false);
        a.move_events = vec![
            BlockMoveEvent {
                time: 1.,
                end_position: Point { x: 0.5, y: 0.5 },
                ..Default::default()
            },
            BlockMoveEvent {
                time: 2.,
                end_position: Point { x: 0.8, y: 0.5 },
                ..Default::default()
            },
        ];
        a.normalize().unwrap();
        assert!((a.pose(1.5, 1.).center.x - 1.5).abs() < 0.0001);
        assert_eq!(ease(13, 1.), 0.);
        assert_eq!(ease(14, 0.), 1.);
        a.move_events[0].ease_type_x = 15;
        assert!(a.normalize().is_err());
    }
    #[test]
    fn missing_config_gets_default_and_switches_independent() {
        let cfg: crate::config::Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.noise_area.enabled);
        assert!(!cfg.noise_area.precise_edges);
        assert!(!cfg.noise_area.low_performance);
        let mut n = cfg.noise_area;
        n.music_unaffected = true;
        n.low_performance = true;
        assert!(!n.remove_distortion);
        assert!(!n.precise_edges);
        let json = serde_json::to_string(&n).unwrap();
        let back: NoiseAreaConfig = serde_json::from_str(&json).unwrap();
        assert!(back.music_unaffected);
        assert!(back.low_performance);
    }
}
