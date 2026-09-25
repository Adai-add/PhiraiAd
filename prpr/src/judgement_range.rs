//! Read-only judgement-range visualization used by the debug overlay.
//!
//! This module never mutates note or judge state.  Timing windows come from
//! `Judge::judgement_range_profile`, while position, width and speed are
//! evaluated through the same chart animations used by normal note drawing.

use crate::{
    config::JudgementRangeAnchor,
    core::{AnimFloat, CtrlObject, JudgeLine, Note, NoteKind, Resource, NOTE_WIDTH_RATIO_BASE},
    judge::{JudgeStatus, JudgementRangeProfile, JudgementTimeWindow},
    parse::RPE_HEIGHT,
};
use macroquad::prelude::*;

const OUTLINE_WIDTH: f32 = 0.004;
const MIN_RECT_SIZE: f32 = 0.0015;

const PERFECT_COLOR: Color = Color::new(0.30, 0.88, 1.00, 1.0);
const GOOD_COLOR: Color = Color::new(0.42, 0.92, 0.43, 1.0);
const BAD_COLOR: Color = Color::new(1.00, 0.62, 0.18, 1.0);
const MISS_COLOR: Color = Color::new(1.00, 0.22, 0.28, 1.0);
const DRAG_COLOR: Color = Color::new(0.98, 0.88, 0.25, 1.0);
const FLICK_COLOR: Color = Color::new(0.95, 0.28, 0.72, 1.0);
const HOLD_TAIL_COLOR: Color = Color::new(0.67, 0.39, 1.00, 1.0);

#[inline]
fn sample(anim: &AnimFloat, time: f64, default: f32) -> f32 {
    let mut anim = anim.clone();
    anim.set_time(time);
    anim.now_opt().unwrap_or(default)
}

#[inline]
fn sample_ctrl(anim: &AnimFloat, height: f64, default: f32) -> f32 {
    sample(anim, height, default)
}

/// Reproduce the line-local y position used by `Note::render` at an arbitrary
/// chart time. `point_height` is the note head height, or the Hold tail height
/// when the tail window is being drawn.
fn point_base_at(note: &Note, line: &JudgeLine, ctrl: &CtrlObject, res: &Resource, point_height: f64, time: f64) -> f32 {
    let line_height = sample(&line.height, time, 0.) as f64;
    let y_offset = sample(&note.object.translation.1, time, 0.);
    let ctrl_height = if note.speed.abs() <= f64::EPSILON {
        note.height - line_height
    } else {
        note.height - line_height + y_offset as f64 / note.speed
    } * RPE_HEIGHT as f64
        / 2.;
    let speed = note.speed * sample_ctrl(&ctrl.y, ctrl_height, 1.) as f64;
    (((point_height - line_height) / res.aspect_ratio as f64 * speed) * res.note_flow_speed as f64 + y_offset as f64 / res.aspect_ratio as f64) as f32
}

fn current_note_half_width(note: &Note, line: &JudgeLine, ctrl: &CtrlObject, res: &Resource, x_limit: f64) -> f32 {
    let line_height = line.height.now() as f64;
    let ctrl_height = if note.speed.abs() <= f64::EPSILON {
        note.height - line_height
    } else {
        note.height - line_height + note.object.translation.1.now() as f64 / note.speed
    } * RPE_HEIGHT as f64
        / 2.;
    let ctrl_size = sample_ctrl(&ctrl.size, ctrl_height, 1.).abs();
    let note_size = note.object.scale.now_with_def(1., 1.).x.abs();
    let multiple_scale = if res.config.double_hint && note.multiple_hint {
        res.res_pack.note_style_mh.click.width() / res.res_pack.note_style.click.width()
    } else {
        1.
    };

    // Default-sized notes reproduce the exact judge-space limit.  Per-chart
    // note size, sizeControl, global note scale and multiple-note texture width
    // then scale the displayed range exactly as the engine scales the sprite.
    let rendered_half_width = res.note_width * multiple_scale * note_size * ctrl_size;
    (rendered_half_width * (x_limit as f32 / NOTE_WIDTH_RATIO_BASE as f32) * note.judge_area.abs()).max(MIN_RECT_SIZE)
}

fn current_note_x(note: &Note, line: &JudgeLine, ctrl: &CtrlObject, res: &Resource, point_base: f32, on_note: bool) -> f32 {
    let x = note.object.translation.0.now();
    if !on_note || matches!(note.kind, NoteKind::Hold { .. }) {
        return x;
    }
    let line_height = line.height.now() as f64;
    let y_offset = note.object.translation.1.now();
    let ctrl_height = if note.speed.abs() <= f64::EPSILON {
        note.height - line_height
    } else {
        note.height - line_height + y_offset as f64 / note.speed
    } * RPE_HEIGHT as f64
        / 2.;
    let pos = sample_ctrl(&ctrl.pos, ctrl_height, 1.);
    let incline_sin = line.incline.now_opt().map(|it| it.to_radians().sin()).unwrap_or_default();
    // `point_base` already contains yOffset/aspect, exactly like the final
    // line-local translation in Note::now_transform. Do not add yOffset a
    // second time when reproducing the incline-dependent x displacement.
    let incline = 1. - incline_sin * point_base * res.aspect_ratio * RPE_HEIGHT / 2. / 360.;
    x * incline * pos
}

#[inline]
fn visible(time: f64, window: JudgementTimeWindow, now: f64, horizon: f64, speed: f64) -> bool {
    let start = time - window.early * speed;
    let end = time + window.late * speed;
    end >= now && start <= now + horizon * speed
}

#[derive(Clone, Copy)]
enum Anchor {
    Line,
    Note,
}

struct DrawContext<'a> {
    note: &'a Note,
    line: &'a JudgeLine,
    ctrl: &'a CtrlObject,
    res: &'a Resource,
    point_height: f64,
    point_time: f64,
    half_width: f32,
    anchor: Anchor,
}

impl DrawContext<'_> {
    fn rect_for_times(&self, start: f64, end: f64) -> Rect {
        let side = if self.note.above { 1. } else { -1. };
        let at_start = point_base_at(self.note, self.line, self.ctrl, self.res, self.point_height, start);
        let at_end = point_base_at(self.note, self.line, self.ctrl, self.res, self.point_height, end);
        let current = point_base_at(self.note, self.line, self.ctrl, self.res, self.point_height, self.res.time);
        let (y1, y2, on_note) = match self.anchor {
            Anchor::Line => (side * at_start, side * at_end, false),
            Anchor::Note => (side * (current - at_start), side * (current - at_end), true),
        };
        let x = current_note_x(self.note, self.line, self.ctrl, self.res, current, on_note);
        Rect::new(x - self.half_width, y1.min(y2), self.half_width * 2., (y1 - y2).abs().max(MIN_RECT_SIZE))
    }

    fn draw_segment(&self, start: f64, end: f64, color: Color, alpha: f32) {
        if end <= start {
            return;
        }
        let rect = self.rect_for_times(start, end);
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, Color { a: alpha, ..color });
    }

    fn draw_window_fill(&self, window: JudgementTimeWindow, color: Color, alpha: f32, speed: f64) {
        self.draw_segment(self.point_time - window.early * speed, self.point_time + window.late * speed, color, alpha);
    }

    /// Draw `outer - inner` as two disjoint timing bands.  This is what keeps
    /// Tap and Hold-head transparency constant where P/G/B windows nest.
    fn draw_difference_fill(&self, outer: JudgementTimeWindow, inner: JudgementTimeWindow, color: Color, alpha: f32, speed: f64) {
        self.draw_segment(self.point_time - outer.early * speed, self.point_time - inner.early * speed, color, alpha);
        self.draw_segment(self.point_time + inner.late * speed, self.point_time + outer.late * speed, color, alpha);
    }

    fn draw_outline(&self, window: JudgementTimeWindow, color: Color, speed: f64) {
        let rect = self.rect_for_times(self.point_time - window.early * speed, self.point_time + window.late * speed);
        draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, OUTLINE_WIDTH, color);
    }
}

fn render_note_ranges(note: &Note, line: &JudgeLine, res: &Resource, profile: &JudgementRangeProfile, anchor: Anchor) {
    if note.fake || matches!(note.judge, JudgeStatus::Judged | JudgeStatus::PreJudge) {
        return;
    }
    let cfg = &res.config.judgement_range_debug;
    let speed = res.config.speed.max(f32::EPSILON) as f64;
    let now = res.time;
    let horizon = cfg.horizon.max(0.) as f64;
    let alpha = cfg.fill_alpha.clamp(0., 1.);
    let ctrl = line.ctrl_obj.borrow();

    let draw_head = !matches!(note.judge, JudgeStatus::Hold(..));
    let (window, x_limit) = match note.kind {
        NoteKind::Click => (profile.tap_outer, profile.tap_x),
        NoteKind::Hold { .. } => (profile.hold_outer, profile.hold_x),
        NoteKind::Drag => (profile.drag_outer, profile.drag_x),
        NoteKind::Flick => (profile.flick_outer, profile.flick_x),
    };

    if draw_head && visible(note.time, window, now, horizon, speed) {
        let ctx = DrawContext {
            note,
            line,
            ctrl: &ctrl,
            res,
            point_height: note.height,
            point_time: note.time,
            half_width: current_note_half_width(note, line, &ctrl, res, x_limit),
            anchor,
        };
        match note.kind {
            NoteKind::Click if cfg.tap.enabled => {
                if cfg.tap.perfect {
                    ctx.draw_window_fill(profile.tap_perfect, PERFECT_COLOR, alpha, speed);
                }
                if cfg.tap.good {
                    ctx.draw_difference_fill(profile.tap_good, profile.tap_perfect, GOOD_COLOR, alpha, speed);
                }
                if cfg.tap.bad {
                    ctx.draw_difference_fill(profile.tap_outer, profile.tap_good, BAD_COLOR, alpha, speed);
                }
                if cfg.tap.perfect {
                    ctx.draw_outline(profile.tap_perfect, PERFECT_COLOR, speed);
                }
                if cfg.tap.good {
                    ctx.draw_outline(profile.tap_good, GOOD_COLOR, speed);
                }
                if cfg.tap.miss {
                    ctx.draw_outline(profile.tap_outer, MISS_COLOR, speed);
                } else if cfg.tap.bad {
                    ctx.draw_outline(profile.tap_outer, BAD_COLOR, speed);
                }
            }
            NoteKind::Hold { .. } if cfg.hold.enabled => {
                if cfg.hold.head_perfect {
                    ctx.draw_window_fill(profile.hold_perfect, PERFECT_COLOR, alpha, speed);
                }
                if cfg.hold.head_good {
                    ctx.draw_difference_fill(profile.hold_outer, profile.hold_perfect, GOOD_COLOR, alpha, speed);
                }
                if cfg.hold.head_perfect {
                    ctx.draw_outline(profile.hold_perfect, PERFECT_COLOR, speed);
                }
                if cfg.hold.head_miss {
                    ctx.draw_outline(profile.hold_outer, MISS_COLOR, speed);
                } else if cfg.hold.head_good {
                    ctx.draw_outline(profile.hold_outer, GOOD_COLOR, speed);
                }
            }
            NoteKind::Drag if cfg.drag => {
                ctx.draw_window_fill(profile.drag_outer, DRAG_COLOR, alpha, speed);
                ctx.draw_outline(profile.drag_outer, DRAG_COLOR, speed);
            }
            NoteKind::Flick if cfg.flick => {
                ctx.draw_window_fill(profile.flick_outer, FLICK_COLOR, alpha, speed);
                ctx.draw_outline(profile.flick_outer, FLICK_COLOR, speed);
            }
            _ => {}
        }
    }

    if let NoteKind::Hold { end_time, end_height } = note.kind {
        if cfg.hold.enabled
            && cfg.hold.tail
            && !matches!(note.judge, JudgeStatus::NotJudged) // Tail is relevant only after the head has been caught.
            && visible(
                end_time,
                JudgementTimeWindow {
                    early: profile.hold_tail,
                    late: 0.,
                },
                now,
                horizon,
                speed,
            )
        {
            let ctx = DrawContext {
                note,
                line,
                ctrl: &ctrl,
                res,
                point_height: end_height,
                point_time: end_time,
                half_width: current_note_half_width(note, line, &ctrl, res, profile.hold_x),
                anchor,
            };
            let tail = JudgementTimeWindow {
                early: profile.hold_tail,
                late: 0.,
            };
            ctx.draw_window_fill(tail, HOLD_TAIL_COLOR, alpha, speed);
            ctx.draw_outline(tail, HOLD_TAIL_COLOR, speed);
        }
    }
}

fn note_outer_window(note: &Note, profile: &JudgementRangeProfile) -> JudgementTimeWindow {
    match note.kind {
        NoteKind::Click => profile.tap_outer,
        NoteKind::Hold { .. } => profile.hold_outer,
        NoteKind::Drag => profile.drag_outer,
        NoteKind::Flick => profile.flick_outer,
    }
}

pub fn render_judgement_ranges(lines: &[JudgeLine], note_indices: &[(Vec<u32>, usize)], res: &mut Resource, profile: &JudgementRangeProfile) {
    let cfg = &res.config.judgement_range_debug;
    if !cfg.enabled {
        return;
    }
    let draw_line = matches!(cfg.anchor, JudgementRangeAnchor::JudgeLine | JudgementRangeAnchor::Both);
    let draw_note = matches!(cfg.anchor, JudgementRangeAnchor::Note | JudgementRangeAnchor::Both);

    let speed = res.config.speed.max(f32::EPSILON) as f64;
    let end = res.time + cfg.horizon.max(0.) as f64 * speed;
    for (line, (indices, start)) in lines.iter().zip(note_indices) {
        let transform = line.now_transform(res, lines);
        res.apply_model_of(&transform, |res| {
            // Judge already owns a stable time-sorted index for every line.
            // Reuse it so a small look-ahead does not scan the whole chart.
            for note_id in &indices[*start..] {
                let note = &line.notes[*note_id as usize];
                if note.time - note_outer_window(note, profile).early * speed > end {
                    break;
                }
                if draw_line {
                    render_note_ranges(note, line, res, profile, Anchor::Line);
                }
                if draw_note {
                    render_note_ranges(note, line, res, profile, Anchor::Note);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_uses_real_seconds_at_current_playback_speed() {
        let w = JudgementTimeWindow { early: 0.2, late: 0.3 };
        // At 0.5x, a 2-real-second horizon spans one chart second.
        assert!(visible(11.09, w, 10., 2., 0.5));
        assert!(!visible(11.2, w, 10., 2., 0.5));
    }

    #[test]
    fn disjoint_difference_bands_do_not_overlap_the_inner_window() {
        let inner = JudgementTimeWindow { early: 0.08, late: 0.15 };
        let outer = JudgementTimeWindow { early: 0.22, late: 0.29 };
        assert!(outer.early >= inner.early);
        assert!(outer.late >= inner.late);
        assert!(inner.early <= outer.early && inner.late <= outer.late);
    }
}
