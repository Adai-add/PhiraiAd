//! Practice camera controls; slider limits do not limit typed values.
pub const EXTRA_PANEL_SCALE: f32 = 0.7;

/// Ui::dy appends global translations, so row spacing must be scaled explicitly.
pub fn extra_panel_offset(distance: f32) -> f32 {
    distance * EXTRA_PANEL_SCALE
}

/// Move the original time/timeline/range group together, independent of panel state.
/// UI units are half the chart viewport's width; retain the 10% screen-height shift.
pub fn timeline_position(screen_height: f32, viewport_width: f32) -> f32 {
    0.06 + screen_height * 0.2 / viewport_width.max(1.)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PracticeView {
    pub scale_percent: f32,
    pub center_x: f32,
    pub center_y: f32,
}

impl Default for PracticeView {
    fn default() -> Self {
        Self {
            scale_percent: 100.,
            center_x: 0.,
            center_y: 0.,
        }
    }
}

impl PracticeView {
    pub fn scale(self) -> f32 {
        let scale = self.scale_percent / 100.;
        // A zero-size view must still have an invertible input transform.
        if scale.abs() < 1e-6 {
            1e-6_f32.copysign(scale)
        } else {
            scale
        }
    }

    /// Coordinates use viewport pixels, with positive Y pointing down.
    pub fn center(self, viewport_width: f32) -> (f32, f32) {
        (self.center_x * (2. / viewport_width), self.center_y * (2. / viewport_width))
    }

    pub fn screen_to_chart(self, x: f32, y: f32, viewport_width: f32) -> (f32, f32) {
        let (cx, cy) = self.center(viewport_width);
        (x / self.scale() + cx, y / self.scale() + cy)
    }

    pub fn chart_to_screen(self, x: f32, y: f32, viewport_width: f32) -> (f32, f32) {
        let (cx, cy) = self.center(viewport_width);
        ((x - cx) * self.scale(), (y - cy) * self.scale())
    }
}

#[derive(Clone, Copy)]
pub struct SliderSpec {
    pub min: f32,
    pub max: f32,
    pub midpoint: f32,
    pub step: f32,
    pub suffix: &'static str,
}

pub const SCALE: SliderSpec = SliderSpec {
    min: 5.,
    max: 500.,
    midpoint: 100.,
    step: 1.,
    suffix: "%",
};
pub const CENTER: SliderSpec = SliderSpec {
    min: -2000.,
    max: 2000.,
    midpoint: 0.,
    step: 1.,
    suffix: "",
};

impl SliderSpec {
    pub fn position(self, value: f32) -> f32 {
        let value = value.clamp(self.min, self.max);
        if value <= self.midpoint {
            0.5 * (value - self.min) / (self.midpoint - self.min)
        } else {
            0.5 + 0.5 * (value - self.midpoint) / (self.max - self.midpoint)
        }
    }

    pub fn drag_value(self, position: f32) -> f32 {
        let p = position.clamp(0., 1.);
        let value = if p <= 0.5 {
            self.min + (self.midpoint - self.min) * p * 2.
        } else {
            self.midpoint + (self.max - self.midpoint) * (p - 0.5) * 2.
        };
        ((value / self.step).round() * self.step).clamp(self.min, self.max)
    }
}

pub fn parse(text: &str) -> Option<f32> {
    let value: f32 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_spacing_and_timeline_match_physical_dimensions() {
        let old_rows = [0., 0.08 + 0.055, 0.08 + 0.055 + 0.175, 0.08 + 0.055 + 0.175 * 2.];
        for offset in old_rows {
            assert!((extra_panel_offset(offset) - offset * 0.7).abs() < 1e-6);
        }
        for (screen_height, viewport_width) in [(1080., 1920.), (1080., 2400.), (1440., 2560.)] {
            let pixels = (timeline_position(screen_height, viewport_width) - 0.06) * viewport_width / 2.;
            assert!((pixels - screen_height * 0.1).abs() < 1e-3);
        }
    }

    #[test]
    fn slider_endpoints_and_midpoints() {
        for spec in [SCALE, CENTER] {
            for (p, value) in [(0., spec.min), (0.5, spec.midpoint), (1., spec.max)] {
                assert_eq!(spec.position(value), p);
                assert_eq!(spec.drag_value(p), value);
            }
            assert_eq!(spec.drag_value(-1.), spec.min);
            assert_eq!(spec.drag_value(2.), spec.max);
        }
        assert_eq!(SCALE.drag_value(0.25), 53.);
        assert_eq!(SCALE.drag_value(0.75), 300.);
    }

    #[test]
    fn typed_values_are_not_clamped_or_rounded() {
        for value in [0., -5., 0.123456, 800., -10000., 10000.] {
            assert_eq!(parse(&value.to_string()), Some(value));
        }
        assert_eq!(parse(" 100% "), Some(100.));
        for value in ["", "abc", "NaN", "inf", "-inf"] {
            assert_eq!(parse(value), None);
        }
    }

    #[test]
    fn camera_and_touch_transforms_are_inverse() {
        for scale in [5., 100., 500., 800., -50.] {
            let view = PracticeView {
                scale_percent: scale,
                center_x: 250.,
                center_y: -120.,
            };
            for (x, y) in [(0., 0.), (-0.5, 0.2), (0.75, -0.3)] {
                let chart = view.screen_to_chart(x, y, 1920.);
                let screen = view.chart_to_screen(chart.0, chart.1, 1920.);
                assert!((screen.0 - x).abs() < 1e-5);
                assert!((screen.1 - y).abs() < 1e-5);
            }
        }
        assert!(PracticeView {
            scale_percent: 0.,
            ..Default::default()
        }
        .scale()
        .is_finite());
    }
}
