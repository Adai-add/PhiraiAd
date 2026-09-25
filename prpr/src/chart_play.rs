//! Per-chart auto-flip intervals and shared visual Hold geometry helpers.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoFlipInterval {
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteConversion {
    #[default]
    Original,
    Tap,
    Drag,
    Flick,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChartPlaySettings {
    pub auto_flip_intervals: Vec<AutoFlipInterval>,
    pub note_conversion: NoteConversion,
}
impl ChartPlaySettings {
    pub fn normalize(&mut self) {
        self.auto_flip_intervals.retain_mut(|r| {
            if !r.start.is_finite() || !r.end.is_finite() {
                return false;
            }
            r.start = r.start.max(0.);
            r.end > r.start
        });
        self.auto_flip_intervals.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut merged: Vec<AutoFlipInterval> = Vec::new();
        for r in self.auto_flip_intervals.drain(..) {
            if let Some(last) = merged.last_mut() {
                if r.start <= last.end {
                    last.end = last.end.max(r.end);
                    continue;
                }
            }
            merged.push(r);
        }
        self.auto_flip_intervals = merged;
    }
    pub fn flipped_at(&self, time: f64) -> bool {
        if !time.is_finite() {
            return false;
        }
        let i = self.auto_flip_intervals.partition_point(|r| r.start <= time);
        i > 0 && time < self.auto_flip_intervals[i - 1].end
    }
    pub fn blocks_score_upload(&self) -> bool {
        !self.auto_flip_intervals.is_empty() || self.note_conversion != NoteConversion::Original
    }
    pub fn add(&mut self, start: f64, end: f64) -> bool {
        if !start.is_finite() || !end.is_finite() || end <= start.max(0.) {
            return false;
        }
        self.auto_flip_intervals.push(AutoFlipInterval { start, end });
        self.normalize();
        true
    }
    pub fn from_chart_bytes(bytes: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let mut settings: Self = serde_json::from_value(value.get("phiraReplica")?.clone()).ok()?;
        settings.normalize();
        Some(settings)
    }
    /// Preserve every unrelated JSON field. PEC/PBC use the chart's info.yml instead.
    pub fn embed_in_chart(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let extra = value.as_object_mut()?.entry("phiraReplica").or_insert_with(|| serde_json::json!({}));
        let object = extra.as_object_mut()?;
        object.remove("shortenHolds"); // r16 legacy setting no longer controls gameplay.
        let mut s = self.clone();
        s.normalize();
        let fields = serde_json::to_value(s).ok()?;
        for (k, v) in fields.as_object()? {
            object.insert(k.clone(), v.clone());
        }
        serde_json::to_vec(&value).ok()
    }
}
/// Shared reflection factors for chart geometry, inverse touch mapping and shader compensation.
pub fn reflection_axes(flip_x: bool, flip_y: bool) -> (f32, f32) {
    (if flip_x { -1. } else { 1. }, if flip_y { -1. } else { 1. })
}

/// Minimum full shortened body in drawing coordinates (not chart seconds).
/// The original renderer is free to consume the remaining body below this size.
pub const MIN_SHORTENED_HOLD_BODY: f64 = 0.01;
/// Prepare the endpoint of the FULL hold before submitting it to the renderer.
/// No current line height/time enters this calculation: the endpoint must not
/// follow the judgement line and pin the remaining body to a minimum each frame.
/// Zero drawing scale cannot display a body; avoid division by zero in that case.
pub fn hold_render_end_height(head: f64, sampled_end: f64, drawing_scale: f64) -> f64 {
    if !drawing_scale.is_finite() || drawing_scale.abs() < 1e-12 {
        return head;
    }
    head + ((sampled_end - head) * drawing_scale).max(MIN_SHORTENED_HOLD_BODY) / drawing_scale
}

/// Convert real-time tail forgiveness to chart time, never before the head.
pub fn hold_visual_end(start: f64, end: f64, speed: f64, tail: f64) -> f64 {
    (end - speed.max(0.) * tail).max(start).min(end.max(start))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intervals_merge_and_boundaries() {
        let mut s = ChartPlaySettings::default();
        assert!(s.add(3., 5.));
        assert!(s.add(1., 3.));
        assert!(s.add(2., 4.));
        assert!(!s.add(f64::NAN, 8.));
        assert!(!s.add(9., 9.));
        assert!(s.add(-2., 0.5));
        assert_eq!(s.auto_flip_intervals, vec![AutoFlipInterval { start: 0., end: 0.5 }, AutoFlipInterval { start: 1., end: 5. }]);
        assert!(!s.flipped_at(0.5));
        assert!(s.flipped_at(1.));
        assert!(!s.flipped_at(5.));
        assert!(s.flipped_at(2.));
    }
    #[test]
    fn short_hold_and_playback_speed() {
        assert!((hold_visual_end(1., 2., 1., 0.22) - 1.78).abs() < 1e-10);
        assert!((hold_visual_end(1., 2., 0.5, 0.22) - 1.89).abs() < 1e-10);
        assert_eq!(hold_visual_end(1., 1.1, 1., 0.22), 1.);
        assert_eq!(hold_visual_end(1., 2., 10., 0.22), 1.);
        assert_eq!(hold_visual_end(1., 1., 1., 0.22), 1.);
    }
    #[test]
    fn embedded_roundtrip_and_clear_preserve_chart() {
        let bytes = br#"{"META":{"name":"test"},"judgeLineList":[{"notes":[1,2,3]}],"phiraReplica":{"futureField":42}}"#;
        let mut s = ChartPlaySettings::default();
        s.add(2., 4.);
        let embedded = s.embed_in_chart(bytes).unwrap();
        assert_eq!(ChartPlaySettings::from_chart_bytes(&embedded).unwrap(), s);
        let v: serde_json::Value = serde_json::from_slice(&embedded).unwrap();
        assert_eq!(v["phiraReplica"]["futureField"], 42);
        assert_eq!(v["judgeLineList"][0]["notes"], serde_json::json!([1, 2, 3]));
        s.auto_flip_intervals.clear();
        assert!(!ChartPlaySettings::from_chart_bytes(&s.embed_in_chart(&embedded).unwrap())
            .unwrap()
            .blocks_score_upload());
        assert!(s.embed_in_chart(b"pec text").is_none());
    }
}
