//! Replay viewer: restores a tape into the real chart renderer, never Judge::update.
use super::{GameMode, GameScene, LoadingScene, NextScene, Scene};
use crate::{
    replay::{self, Manifest, Tape},
    time::TimeManager,
    ui::Ui,
};
use anyhow::{bail, Result};
use macroquad::prelude::*;
use std::sync::Arc;

const CONSOLE_HEIGHT: f32 = 0.25;

fn chart_viewport(full: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let reserved = (CONSOLE_HEIGHT * full.2 as f32 / 2.).ceil() as i32;
    let reserved = reserved.clamp(0, (full.3 - 1).max(0));
    (full.0, full.1 + reserved, full.2, full.3 - reserved)
}

pub struct ReplayScene {
    game: GameScene,
    manifest: Manifest,
    tape: Arc<Tape>,
    clock: TimeManager,
    position: f64,
    rate: f32,
    playing: bool,
    current: Option<usize>,
    last_real: f64,
    seek: bool,
    fx_position: f64,
    offsets: bool,
    ranges: bool,
    fingers: bool,
    dragging: Option<u64>,
    done: bool,
    timeline: Rect,
    frame_seek: Option<usize>,
    annotations_index: Vec<(usize, usize)>,
    audio_error_reported: bool,
}
impl ReplayScene {
    pub async fn load(id: String) -> Result<Self> {
        let (manifest, tape) = tokio::task::spawn_blocking(move || replay::load(&id)).await??;
        let mut config = manifest.config.clone();
        config.replay_recording_enabled = false;
        // Recorded speed already includes Nightcore. Restore the visual mod flag
        // after construction, so the loader cannot multiply playback rate twice.
        config.mods.remove(crate::config::Mods::NIGHTCORE);
        config.judgement_range_debug.enabled = false;
        let mut loader = LoadingScene::new(
            GameMode::View,
            manifest.info.clone(),
            config,
            Box::new(replay::SnapshotFs(manifest.assets.clone())),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let mut game = loader.load_task.take().unwrap().await?;
        if game.chart.lines.iter().map(|l| l.notes.len()).collect::<Vec<_>>() != manifest.note_counts {
            bail!("谱面快照与录像音符不一致");
        }
        game.res.config.mods = manifest.config.mods;
        game.res.info.replica_play = manifest.info.replica_play.clone();
        game.prepare_replay();
        let annotations_index = tape
            .frames
            .iter()
            .enumerate()
            .flat_map(|(fi, frame)| {
                frame
                    .outcomes
                    .iter()
                    .enumerate()
                    .filter_map(|(ei, event)| {
                        let hold = game
                            .chart
                            .lines
                            .get(event.line as usize)
                            .and_then(|l| l.notes.get(event.note as usize))
                            .is_some_and(|n| matches!(n.kind, crate::core::NoteKind::Hold { .. }));
                        // Hold head results are Err(bool); successful tail results duplicate them.
                        (!(hold && matches!(event.result, Ok(crate::judge::Judgement::Perfect | crate::judge::Judgement::Good)))).then_some((fi, ei))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        Ok(Self {
            game,
            manifest,
            tape: Arc::new(tape),
            clock: TimeManager::default(),
            position: 0.,
            rate: 1.,
            playing: false,
            current: None,
            last_real: 0.,
            seek: true,
            fx_position: 0.,
            offsets: true,
            ranges: false,
            fingers: true,
            dragging: None,
            done: false,
            timeline: Rect::default(),
            frame_seek: None,
            annotations_index,
            audio_error_reported: false,
        })
    }
    fn restore(&mut self, end: usize) {
        let begin = self.tape.frames[..=end].iter().rposition(|f| f.checkpoint).unwrap_or(0);
        self.game.clear_replay_effects();
        self.game.chart.reset();
        for frame in &self.tape.frames[begin..=end] {
            self.game.apply_replay_notes(frame);
        }
        // Rebuild effects at their recorded age, without replaying historical sounds.
        let start = self.tape.frames.partition_point(|f| f.tape < self.position - 0.6).min(end);
        let mut clock = self.tape.frames[start].tape;
        let mut rotating = false;
        for frame in &self.tape.frames[start..=end] {
            if !rotating {
                self.game.advance_replay_effects((frame.tape - clock).max(0.) as f32);
            }
            self.game.emit_replay_effects(frame, false);
            clock = frame.tape;
            rotating = frame.rotating;
        }
        if !rotating {
            self.game.advance_replay_effects((self.position - clock).max(0.) as f32);
        }
        self.fx_position = self.position;
    }
    fn controls(&mut self, ui: &mut Ui) {
        ui.abs_scope(|ui| {
            let top = ui.top;
            ui.fill_rect(Rect::new(-1., top - CONSOLE_HEIGHT, 2., CONSOLE_HEIGHT), Color::new(0., 0., 0., 0.7));
            for (x, label, value) in [
                (-0.97, "按键偏移", &mut self.offsets),
                (-0.69, "判定范围", &mut self.ranges),
                (-0.41, "触控点", &mut self.fingers),
            ] {
                ui.scope(|ui| {
                    ui.dx(x);
                    ui.dy(top - 0.221);
                    ui.with(crate::core::Matrix::new_scaling(0.8), |ui| {
                        ui.checkbox(label, value);
                    });
                });
            }
            for (i, (delta, label)) in [(-10, "−10"), (-1, "−1"), (1, "+1"), (10, "+10")].iter().enumerate() {
                if ui.button(&format!("replay_frame_{i}"), Rect::new(-0.11 + i as f32 * 0.135, top - 0.226, 0.12, 0.06), *label) {
                    let index = replay::step_frame(self.frame_seek.or(self.current).unwrap_or(0), self.tape.frames.len(), *delta);
                    self.position = self.tape.frames[index].tape;
                    self.frame_seek = Some(index);
                    self.playing = false;
                    self.seek = true;
                }
            }
            ui.text(format!("{} / {} 帧", self.frame_seek.or(self.current).unwrap_or(0) + 1, self.tape.frames.len()))
                .pos(0.46, top - 0.196)
                .anchor(0., 0.5)
                .size(0.30)
                .max_width(0.50)
                .draw();
            self.timeline = Rect::new(-0.86, top - 0.145, 1.72, 0.045);
            let p = (self.position / self.duration().max(0.001)) as f32;
            ui.fill_rect(Rect::new(-0.86, top - 0.128, 1.72, 0.008), Color::new(1., 1., 1., 0.25));
            ui.fill_rect(Rect::new(-0.86, top - 0.128, 1.72 * p, 0.008), WHITE);
            ui.fill_circle(-0.86 + 1.72 * p, top - 0.124, 0.014, WHITE);
            if ui.button("replay_play", Rect::new(-0.86, top - 0.085, 0.19, 0.06), if self.playing { "暂停" } else { "播放" }) {
                if self.position >= self.duration() {
                    self.position = 0.;
                    self.seek = true;
                }
                self.playing = !self.playing;
                self.seek = true;
            }
            ui.text(format!("{:.1} / {:.1}s", self.position, self.duration()))
                .pos(-0.62, top - 0.055)
                .anchor(0., 0.5)
                .size(0.38)
                .draw();
            for (i, rate) in [0.1, 0.25, 0.5, 1., 1.5, 2., 3.].iter().enumerate() {
                let label = format!("{}{rate}×", if self.rate == *rate { "✓" } else { "" });
                if ui.button(&format!("replay_rate_{i}"), Rect::new(-0.10 + i as f32 * 0.14, top - 0.085, 0.13, 0.06), label) {
                    self.rate = *rate;
                    if !self.playing {
                        self.frame_seek = self.current;
                    }
                    self.seek = true;
                }
            }
            if ui.button("replay_back", Rect::new(-0.96, -top + 0.02, 0.16, 0.06), "返回") {
                self.done = true;
            }
        });
    }
    fn duration(&self) -> f64 {
        self.tape.frames.last().unwrap().tape
    }
    fn screen_point(&self, p: Vec2, ui: &Ui) -> Vec2 {
        let vp = self.game.res.camera.viewport.unwrap_or(ui.viewport);
        let scale = vp.2 as f32 / ui.viewport.2.max(1) as f32;
        vec2(
            p.x * scale + ((vp.0 - ui.viewport.0) as f32 * 2. + vp.2 as f32 - ui.viewport.2 as f32) / ui.viewport.2.max(1) as f32,
            p.y * scale + ((ui.viewport.1 - vp.1) as f32 * 2. + ui.viewport.3 as f32 - vp.3 as f32) / ui.viewport.2.max(1) as f32,
        )
    }
    fn rotate_point(p: Vec2, angle: f32) -> Vec2 {
        vec2(p.x * angle.cos() - p.y * angle.sin(), p.x * angle.sin() + p.y * angle.cos())
    }
    fn projected_point(&self, center: [f32; 2], ui: &Ui, angle: f32) -> Vec2 {
        self.rotate_canvas_point(self.screen_point(vec2(center[0], -center[1] / self.game.res.aspect_ratio), ui), ui, angle)
    }
    fn rotate_canvas_point(&self, p: Vec2, ui: &Ui, angle: f32) -> Vec2 {
        let vp = chart_viewport(ui.viewport);
        let center = vec2(0., ((ui.viewport.1 - vp.1) as f32 * 2. + ui.viewport.3 as f32 - vp.3 as f32) / ui.viewport.2.max(1) as f32);
        Self::rotate_point(p - center, angle) + center
    }
    fn annotations(&mut self, ui: &mut Ui) {
        let frame = &self.tape.frames[self.current.unwrap_or(0)];
        // Chart is already drawn; contacts precede the offset labels.
        if self.fingers {
            for finger in &frame.contacts {
                let p = self.screen_point(vec2(finger.position[0], finger.position[1]), ui);
                ui.fill_circle(p.x, p.y, 0.0325, Color::new(0.61, 0.08, 0.08, 1.));
                ui.stroke_circle(p.x, p.y, 0.0325, 0.004, Color::new(1., 0.55, 0., 1.));
            }
        }
        if !self.offsets {
            return;
        }
        let begin = self
            .annotations_index
            .partition_point(|&(fi, _)| self.tape.frames[fi].tape <= self.position - 0.3);
        let end = self
            .annotations_index
            .partition_point(|&(fi, _)| self.tape.frames[fi].tape <= self.position + 1.);
        for &(fi, ei) in &self.annotations_index[begin..end] {
            let trigger = &self.tape.frames[fi];
            let event = &trigger.outcomes[ei];
            let age = self.position - trigger.tape;
            let alpha = replay::annotation_alpha(age);
            if alpha <= 0. {
                continue;
            }
            let p = if age < 0. || (age == 0. && fi > self.current.unwrap_or(0)) {
                // Follow the actual renderer's line, scroll, incline, control,
                // note rotation and practice camera transforms before the hit.
                let Some(center) = self.game.res.replay_note_centers.get(&(event.line, event.note)).copied() else {
                    continue;
                };
                self.projected_point(center, ui, frame.angle)
            } else if let Some(center) = event.rendered_center {
                // Freeze in the trigger frame's screen space, including its rotation.
                self.projected_point(center, ui, trigger.angle)
            } else {
                // Compatibility with r1 tapes, which only saved the judgement center.
                let mut p = vec2(event.center[0], event.center[1]);
                if self.manifest.config.flip_x() {
                    p.x = -p.x;
                }
                let view = crate::practice_view::PracticeView {
                    scale_percent: trigger.view[0],
                    center_x: trigger.view[1],
                    center_y: trigger.view[2],
                };
                let (x, y) = view.chart_to_screen(p.x, p.y, trigger.viewport_width);
                self.rotate_canvas_point(self.screen_point(vec2(x, y), ui), ui, trigger.angle)
            };
            if p.x.abs() > 1. || p.y.abs() > ui.top {
                continue;
            }
            let text = format!("{:.0}", -event.difference * 1000.);
            for (dx, dy) in [(-0.003, 0.), (0.003, 0.), (0., -0.003), (0., 0.003), (-0.002, -0.002), (0.002, 0.002)] {
                ui.text(&text)
                    .pos(p.x + dx, p.y + dy)
                    .anchor(0.5, 0.5)
                    .size(0.45)
                    .color(Color { a: alpha, ..BLACK })
                    .draw();
            }
            let mut color = replay::color(event.color);
            color.a *= alpha;
            ui.text(&text).pos(p.x, p.y).anchor(0.5, 0.5).size(0.45).color(color).draw();
        }
    }
}
impl Scene for ReplayScene {
    fn screen_touch_coordinates(&self) -> bool {
        true
    }
    fn enter(&mut self, tm: &mut TimeManager, target: Option<RenderTarget>) -> Result<()> {
        self.game.enter(&mut self.clock, target)?;
        self.game.music.pause()?;
        self.clock.adjust_time = false;
        self.last_real = tm.real_time();
        Ok(())
    }
    fn pause(&mut self, _tm: &mut TimeManager) -> Result<()> {
        self.playing = false;
        self.game
            .sync_replay_audio(&self.tape.frames[self.current.unwrap_or(0)], self.rate, false, false)?;
        self.seek = true;
        Ok(())
    }
    fn resume(&mut self, tm: &mut TimeManager) -> Result<()> {
        self.last_real = tm.real_time();
        Ok(())
    }
    fn touch(&mut self, _tm: &mut TimeManager, touch: &Touch) -> Result<bool> {
        if touch.phase == TouchPhase::Started && self.timeline.contains(touch.position) {
            self.dragging = Some(touch.id);
        }
        if self.dragging == Some(touch.id) {
            self.position = ((touch.position.x - self.timeline.x) / self.timeline.w).clamp(0., 1.) as f64 * self.duration();
            self.seek = true;
            self.frame_seek = None;
            if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.dragging = None;
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn update(&mut self, tm: &mut TimeManager) -> Result<()> {
        replay::poll_saves();
        self.game.res.audio.recover_if_needed()?;
        let real = tm.real_time();
        let dt = (real - self.last_real).max(0.);
        self.last_real = real;
        if self.playing && self.dragging.is_none() {
            self.position = (self.position + dt * self.rate as f64).min(self.duration());
        }
        let end = if let Some(index) = self.frame_seek.take() {
            index
        } else if !self.playing && !self.seek {
            self.current.unwrap_or(0)
        } else {
            replay::seek_range(&self.tape, self.position).1
        };
        if self.seek || self.current.is_none_or(|old| end < old) {
            self.restore(end);
        } else if let Some(old) = self.current {
            let mut rotating = self.tape.frames[old].rotating;
            for frame in &self.tape.frames[old + 1..=end] {
                if !rotating {
                    self.game.advance_replay_effects((frame.tape - self.fx_position).max(0.) as f32);
                }
                self.fx_position = frame.tape;
                self.game.apply_replay_notes(frame);
                self.game.emit_replay_effects(frame, self.playing && self.dragging.is_none());
                rotating = frame.rotating;
            }
            if !rotating {
                self.game.advance_replay_effects((self.position - self.fx_position).max(0.) as f32);
            }
            self.fx_position = self.position;
        }
        let frame = &self.tape.frames[end];
        let discontinuity = self
            .current
            .is_some_and(|old| (frame.song - self.tape.frames[old].song).abs() > dt * self.rate as f64 * frame.speed as f64 + 0.15);
        if self.position >= self.duration() {
            self.playing = false;
        }
        if let Err(err) = self
            .game
            .sync_replay_audio(frame, self.rate, self.seek || discontinuity, self.playing && self.dragging.is_none())
        {
            // A full or unavailable audio queue must not stop the chart/console.
            if !self.audio_error_reported {
                tracing::warn!("replay audio synchronization failed; visuals remain active: {err:#}");
                self.audio_error_reported = true;
            }
        }
        self.current = Some(end);
        self.seek = false;
        Ok(())
    }
    fn render(&mut self, _tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.game.res.replay_note_targets.clear();
        if self.offsets {
            let begin = self
                .annotations_index
                .partition_point(|&(fi, _)| self.tape.frames[fi].tape < self.position);
            let end = self
                .annotations_index
                .partition_point(|&(fi, _)| self.tape.frames[fi].tape <= self.position + 1.);
            for &(fi, ei) in &self.annotations_index[begin..end] {
                let event = &self.tape.frames[fi].outcomes[ei];
                self.game.res.replay_note_targets.insert((event.line, event.note));
            }
        }
        let frame = &self.tape.frames[self.current.unwrap_or(0)];
        self.game.res.config.judgement_range_debug.enabled = self.ranges;
        // Particle time follows the viewing speed and freezes with playback.
        self.game.last_update_time = self.clock.real_time();
        // Render the entire chart/HUD into a separate viewport above the console.
        // Keep the recording's aspect ratio; Resource letterboxes it as needed.
        let full_viewport = ui.viewport;
        let full_top = ui.top;
        ui.viewport = chart_viewport(full_viewport);
        ui.top = ui.viewport.3 as f32 / ui.viewport.2.max(1) as f32;
        // Save before the chart render: noise-area postprocessing swaps chart
        // targets and GameScene can finish with its offscreen target active.
        // Saving only before drawing controls would preserve that leaked target.
        let previous_viewport = unsafe { get_internal_gl() }.quad_gl.get_viewport();
        push_camera_state();
        let result = self.game.render_replay_frame(frame, &mut self.clock, ui);
        ui.viewport = full_viewport;
        ui.top = full_top;
        pop_camera_state();
        // Macroquad's camera stack restores render pass/matrix/depth, not viewport.
        unsafe { get_internal_gl() }.quad_gl.viewport(previous_viewport);
        result?;
        push_camera_state();
        let mut camera = ui.camera();
        camera.render_target = self.game.res.camera.render_target;
        set_camera(&camera);
        ui.abs_scope(|ui| {
            let vp = chart_viewport(ui.viewport);
            let chart_height = vp.3 as f32 * 2. / ui.viewport.2.max(1) as f32;
            let area = Rect::new(-1., -ui.top, 2., chart_height);
            ui.scissor(area, |ui| {
                if let Some((number, alpha)) = self.tape.frames[self.current.unwrap_or(0)].countdown {
                    ui.fill_rect(area, Color::new(0., 0., 0., alpha));
                    ui.text(number.to_string()).pos(0., area.y + area.h / 2.).anchor(0.5, 0.5).size(1.).draw();
                }
                self.annotations(ui);
            });
            self.controls(ui);
        });
        pop_camera_state();
        unsafe { get_internal_gl() }.quad_gl.viewport(previous_viewport);
        Ok(())
    }
    fn next_scene(&mut self, _tm: &mut TimeManager) -> NextScene {
        if self.done {
            let _ = self.game.music.pause();
            NextScene::Pop
        } else {
            NextScene::None
        }
    }
}

#[cfg(test)]
mod layout_tests {
    #[test]
    fn noise_target_height_does_not_shift_console_touch() {
        let raw = macroquad::prelude::vec2(210.89012, 1012.0629);
        let full = (0, 0, 1920, 1080);
        let point = crate::judge::Judge::screen_touch_position(raw, full, 1080.);
        let play = macroquad::prelude::Rect::new(-0.86, 1080. / 1920. - 0.085, 0.19, 0.06);
        assert!(play.contains(point));
        let offscreen = crate::judge::Judge::screen_touch_position(raw, (0, 0, 1920, 840), 1080.);
        assert!(!play.contains(offscreen));
        assert!((point.y - offscreen.y - 0.125).abs() < 1e-6);
    }

    #[test]
    fn console_touch_matches_display_with_inset_viewport() {
        let vp = (20, 30, 2400, 1080);
        let top = vp.3 as f32 / vp.2 as f32;
        // Round-trip centers of play, speed, frame-step, timeline and back controls.
        for expected in [
            (-0.765, top - 0.055),
            (0.1, top - 0.055),
            (0.08, top - 0.196),
            (0., top - 0.124),
            (-0.88, -top + 0.05),
        ] {
            let raw = macroquad::prelude::vec2(
                vp.0 as f32 + (expected.0 + 1.) * vp.2 as f32 / 2.,
                1200. - (vp.1 + vp.3) as f32 + (expected.1 + top) * vp.2 as f32 / 2.,
            );
            let point = crate::judge::Judge::screen_touch_position(raw, vp, 1200.);
            assert!((point.x - expected.0).abs() < 1e-6);
            assert!((point.y - expected.1).abs() < 1e-6);
        }
    }

    #[test]
    fn chart_stays_above_console_in_offset_viewports() {
        for full in [(0, 0, 1920, 1080), (20, 30, 2400, 1080), (0, 0, 1024, 768)] {
            let chart = super::chart_viewport(full);
            assert_eq!(chart.0, full.0);
            assert_eq!(chart.2, full.2);
            assert_eq!(chart.1 + chart.3, full.1 + full.3);
            assert!(chart.3 > 0);
            assert!((chart.1 - full.1) as f32 >= super::CONSOLE_HEIGHT * full.2 as f32 / 2.);
        }
    }
}
