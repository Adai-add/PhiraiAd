//! Phigros block-area (噪域) data and hit testing.
//!
//! The chart field is intentionally kept separate from the global switches so
//! charts without `blockAreaList` stay on the normal rendering path.
use macroquad::prelude::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NoiseAreaConfig {
    pub enabled: bool,
    pub precise_edges: bool,
    pub remove_distortion: bool,
    pub music_unaffected: bool,
}

impl Default for NoiseAreaConfig {
    fn default() -> Self {
        Self { enabled: true, precise_edges: false, remove_distortion: false, music_unaffected: false }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockArea {
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
pub struct Point { pub x: f32, pub y: f32 }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRotateEvent { pub anchor: Point, pub time: f32, pub rotation: f32, #[serde(default)] pub ease_type: i32 }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockMoveEvent { pub end_position: Point, pub time: f32, #[serde(default)] pub ease_type_x: i32, #[serde(default)] pub ease_type_y: i32 }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockScaleEvent { pub anchor: Point, pub time: f32, pub scale: Point, #[serde(default)] pub ease_type_x: i32, #[serde(default)] pub ease_type_y: i32 }

impl BlockArea {
    pub fn normalize(&mut self) {
        for v in [&mut self.appear_time, &mut self.enable_time, &mut self.disable_time, &mut self.disappear_time] {
            if !v.is_finite() { *v = 0.; }
            *v = (*v).max(0.);
        }
        self.enable_time = self.enable_time.max(self.appear_time);
        self.disable_time = self.disable_time.max(self.enable_time);
        self.disappear_time = self.disappear_time.max(self.disable_time);
        self.rotate_events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.move_events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.scale_events.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// Returns whether a normalized screen point is blocked at chart time.
    /// The caller combines normal and subtract regions using XOR parity.
    pub fn contains(&self, point: Vec2, time: f32) -> bool {
        if !(self.enable_time..self.disable_time).contains(&time) { return false; }
        let min = Vec2::new(self.bottom_left_percentage.x.min(self.top_right_percentage.x), self.bottom_left_percentage.y.min(self.top_right_percentage.y));
        let max = Vec2::new(self.bottom_left_percentage.x.max(self.top_right_percentage.x), self.bottom_left_percentage.y.max(self.top_right_percentage.y));
        point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoiseAreaState { pub blocked_ids: Vec<u64> }

impl NoiseAreaState {
    pub fn update(&mut self, areas: &[BlockArea], touches: &[(u64, Vec2)], time: f32, enabled: bool) {
        self.blocked_ids.clear();
        if !enabled { return; }
        for &(id, point) in touches {
            let mut normal = false;
            let mut subtract = false;
            for area in areas {
                if area.contains(point, time) {
                    if area.is_subtract { subtract = !subtract; } else { normal = !normal; }
                }
            }
            if normal || subtract { self.blocked_ids.push(id); }
        }
    }
    pub fn is_blocked(&self, id: u64) -> bool { self.blocked_ids.contains(&id) }
}
