//! Piecewise-linear practice speed controls. Both halves share the 1x midpoint.
use std::ops::Range;

pub fn position(value: f32, range: &Range<f32>) -> f32 {
    let value = value.clamp(range.start, range.end);
    if value <= 1. {
        0.5 * (value - range.start) / (1. - range.start)
    } else {
        0.5 + 0.5 * (value - 1.) / (range.end - 1.)
    }
}

pub fn value(position: f32, range: &Range<f32>) -> f32 {
    let p = position.clamp(0., 1.);
    if p <= 0.5 {
        range.start + (1. - range.start) * p * 2.
    } else {
        1. + (range.end - 1.) * (p - 0.5) * 2.
    }
}

pub fn drag_value(p: f32, range: &Range<f32>, step: f32) -> f32 {
    ((value(p, range) / step).round() * step).clamp(range.start, range.end)
}

/// Reject invalid/non-finite/out-of-range entries. Manual values retain millesimal precision.
pub fn parse(text: &str, range: &Range<f32>) -> Option<f32> {
    let value: f32 = text.trim().parse().ok()?;
    if !value.is_finite() || value < range.start || value > range.end {
        return None;
    }
    Some(((value * 1000.).round() / 1000.).clamp(range.start, range.end))
}

/// At least two decimal places, with a third digit when needed (e.g. a locked reciprocal).
pub fn display(value: f32) -> String {
    let mut text = format!("{value:.3}");
    if text.ends_with('0') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_center_and_each_linear_half() {
        for range in [0.05..10., 0.1..20.] {
            assert_eq!(position(range.start, &range), 0.);
            assert_eq!(position(1., &range), 0.5);
            assert_eq!(position(range.end, &range), 1.);
            assert!((value(0.25, &range) - (range.start + 1.) / 2.).abs() < 1e-6);
            assert!((value(0.75, &range) - (range.end + 1.) / 2.).abs() < 1e-6);
            for i in 0..=100 {
                let p = i as f32 / 100.;
                assert!((position(value(p, &range), &range) - p).abs() < 1e-6);
            }
            assert_eq!(drag_value(0.5, &range, 0.05), 1.);
            assert_eq!(drag_value(-1., &range, 0.05), range.start);
            assert_eq!(drag_value(2., &range, 0.05), range.end);
        }
    }
    #[test]
    fn numeric_input_preserves_precision_and_rejects_invalid_values() {
        for bad in ["", "abc", "NaN", "inf", "-inf", "-1", "0", "10.001"] {
            assert_eq!(parse(bad, &(0.05..10.)), None);
        }
        assert_eq!(parse(" 0.075 ", &(0.05..10.)), Some(0.075));
        assert_eq!(parse("3.3333", &(0.1..20.)), Some(3.333));
        assert_eq!(parse("0.05", &(0.05..10.)), Some(0.05));
        assert_eq!(parse("10", &(0.05..10.)), Some(10.));
        assert_eq!(parse("0.05", &(0.1..20.)), None);
        assert_eq!(parse("20", &(0.1..20.)), Some(20.));
        assert_eq!(display(1.), "1.00");
        assert_eq!(display(3.333), "3.333");
    }
}
