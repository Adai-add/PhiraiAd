//! Texture/step-event protocol; geometry reuses RPE animations and parents.
use super::Pose;
use crate::core::{JudgeLine, Matrix, Point};
use macroquad::prelude::{vec2, Color, Vec2};

pub fn texture_kind(texture: &str) -> Option<bool> {
    match texture.rsplit(['/', '\\']).next()? {
        "isSubtract0.png" => Some(false),
        "isSubtract1.png" => Some(true),
        _ => None,
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Marker {
    pub time: f64,
    pub state: u8,
}
#[derive(Clone, Debug)]
pub struct RecorderArea {
    pub line: usize,
    pub z_order: i32,
    pub anchor: Vec2,
    pub texture_size: Vec2,
    pub markers: Vec<Marker>,
    pub pose: Pose,
    pub color: Color,
}
impl RecorderArea {
    pub fn marker(&self, t: f64) -> Option<usize> {
        self.markers.partition_point(|m| m.time <= t).checked_sub(1)
    }
    pub fn state(&self, t: f64) -> u8 {
        self.marker(t).map_or(0, |i| self.markers[i].state)
    }
    pub fn ready(&self, t: f64) -> bool {
        let Some(i) = self.marker(t) else { return false };
        if self.markers[i].state != 1 {
            return false;
        }
        self.markers[i + 1..]
            .iter()
            .find(|m| m.state != 1)
            .is_some_and(|m| m.state == 2 && t >= m.time - 0.5)
    }
    pub fn fade(&self, t: f64) -> f32 {
        let Some(i) = self.marker(t) else { return 0. };
        if self.markers[i].state == 1 && !self.ready(t) {
            ((t - self.markers[i].time) / 0.5).clamp(0., 1.) as f32
        } else {
            1.
        }
    }
    pub fn refresh(&mut self, line: &JudgeLine, transform: Matrix, aspect: f32) {
        let scale = line.object.scale.now_with_def(1., 1.);
        let signed = vec2(
            self.texture_size.x * scale.x * 2. / crate::parse::RPE_WIDTH,
            self.texture_size.y * scale.y * 2. / crate::parse::RPE_HEIGHT / aspect,
        );
        let local_center = (Vec2::splat(0.5) - self.anchor) * signed;
        let center = transform.transform_point(&Point::new(local_center.x, local_center.y));
        self.pose = Pose {
            center: vec2(center.x, center.y) * (5. * aspect),
            size: signed.abs() * (5. * aspect),
            angle: transform[(1, 0)].atan2(transform[(0, 0)]),
        };
        self.color = line.color.now_opt().unwrap_or(Color::new(1., 84. / 255., 84. / 255., 1.));
        self.color.a = 1.;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_is_seekable_and_warning_is_in_seconds() {
        let a = RecorderArea {
            line: 0,
            z_order: 0,
            anchor: Vec2::splat(0.5),
            texture_size: Vec2::splat(900.),
            markers: [(1., 1), (3., 2), (4., 4), (5., 2), (6., 3), (7., 0)]
                .map(|(time, state)| Marker { time, state })
                .to_vec(),
            pose: Pose {
                center: Vec2::ZERO,
                size: Vec2::ONE,
                angle: 0.,
            },
            color: Color::new(1., 0., 1., 1.),
        };
        assert_eq!(a.state(0.), 0);
        assert!(!a.ready(2.49));
        assert!(a.ready(2.5));
        assert_eq!(a.state(3.), 2);
        assert_eq!(a.state(4.), 4);
        assert_eq!(a.state(5.5), 2);
        assert_eq!(a.state(6.5), 3);
        assert_eq!(a.state(7.), 0);
        assert_eq!(a.state(3.5), 2);
    }
    #[test]
    fn texture_protocol_uses_exact_basename() {
        assert_eq!(texture_kind("images/isSubtract1.png"), Some(true));
        assert_eq!(texture_kind("images\\isSubtract0.png"), Some(false));
        assert_eq!(texture_kind("not_isSubtract0.png"), None);
    }
}
