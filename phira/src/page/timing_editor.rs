//! Empty-chart HUD preview with the same renderer used during play.
use super::{NextPage, Page, SharedState};
use crate::{get_data, get_data_mut, save_data};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    config::TimingBarConfig,
    core::{BpmList, Chart, ChartExtra, ChartSettings, Matrix, NoteKind},
    ext::{poll_future, LocalTask},
    judge::{Judge, JudgeReportEvent, Judgement},
    scene::{show_error, GameScene},
    time::TimeManager,
    timing_bar::{self, TimingBar},
    ui::Ui,
};
use std::{borrow::Cow, collections::HashMap};

pub struct TimingEditor {
    draft: TimingBarConfig,
    preview: TimingBar,
    judge: Judge,
    done: bool,
    controls: bool,
    panel_rect: Option<Rect>,
    dragging: Option<(u64, Vec2, Vec2)>,
    last_sample: f32,
    sample_index: usize,
    chart_scene: Option<GameScene>,
    chart_task: LocalTask<Result<GameScene>>,
    chart_error: Option<String>,
    chart_time: TimeManager,
}
impl TimingEditor {
    pub fn new() -> Self {
        let chart = Chart::new(0., Vec::new(), BpmList::default(), ChartSettings::default(), ChartExtra::default(), HashMap::new());
        Self {
            draft: get_data().config.timing_bar.clone(),
            preview: TimingBar::default(),
            judge: Judge::new(&chart),
            done: false,
            controls: true,
            panel_rect: None,
            dragging: None,
            last_sample: -1.,
            sample_index: 0,
            chart_scene: None,
            chart_task: Some(Box::pin(prpr::timing_preview::load(get_data().config.clone()))),
            chart_error: None,
            chart_time: TimeManager::default(),
        }
    }
    fn numeric(ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32) {
        let mut text = format!("{:.1}", *value);
        let mut changed = false;
        ui.input(
            label,
            &mut text,
            prpr::ui::InputParams {
                length: 0.18,
                changed: Some(&mut changed),
                ..().into()
            },
        );
        if changed {
            if let Ok(v) = text.trim().parse::<f32>() {
                if v.is_finite() {
                    *value = v.clamp(min, max);
                }
            }
        }
    }
}
impl Page for TimingEditor {
    fn label(&self) -> Cow<'static, str> {
        "准度条编辑".into()
    }
    fn can_play_bgm(&self) -> bool {
        false
    }
    fn exit(&mut self) -> Result<()> {
        get_data_mut().config.timing_bar = self.draft.clone();
        save_data()
    }
    fn touch(&mut self, touch: &Touch, _s: &mut SharedState) -> Result<bool> {
        let half_height = screen_height() / screen_width();
        if let Some((id, origin, anchor)) = self.dragging {
            if touch.id == id {
                let delta = touch.position - origin;
                self.draft.x = (anchor.x + delta.x).clamp(-1., 1.);
                self.draft.y = (anchor.y + delta.y / half_height).clamp(-1., 1.);
                if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.dragging = None;
                }
                return Ok(true);
            }
        }
        if touch.phase == TouchPhase::Started
            && touch.position.y > -half_height + 0.15
            && touch.position.y < half_height - 0.10
            && !self.panel_rect.is_some_and(|rect| rect.contains(touch.position))
        {
            let scale = self.draft.size.clamp(5., 500.) / 100.;
            let anchor = vec2(self.draft.x, self.draft.y * half_height);
            let r = Rect::new(
                anchor.x - 0.40 * scale,
                anchor.y - if self.draft.curved { 0.25 * scale } else { 0.07 * scale },
                0.80 * scale,
                if self.draft.curved { 0.34 * scale } else { 0.15 * scale },
            );
            if r.contains(touch.position) {
                self.dragging = Some((touch.id, touch.position, vec2(self.draft.x, self.draft.y)));
            }
        }
        Ok(false)
    }
    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        if let Some(task) = &mut self.chart_task {
            if let Some(result) = poll_future(task.as_mut()) {
                self.chart_task = None;
                match result {
                    Ok(scene) => self.chart_scene = Some(scene),
                    Err(error) => {
                        self.chart_error = Some(format!("{error:#}"));
                        show_error(error);
                    }
                }
            }
        }
        let interval = if self.sample_index != 0 && self.sample_index % 10 == 0 {
            2.2
        } else {
            0.6
        };
        if s.t - self.last_sample > interval {
            self.last_sample = s.t;
            let differences = [-0.18, -0.025, 0.02, 0.11, -0.06, 0.04, 0.005, -0.09, 0.13, -0.01];
            let mut diff: f64 = differences[self.sample_index % differences.len()];
            self.sample_index += 1;
            let profile = self.judge.judgement_range_profile(&get_data().config);
            let miss = self.sample_index % 10 == 0;
            if miss {
                diff = profile.tap_outer.late + 0.001;
            }
            let perfect = if diff < 0. {
                profile.tap_perfect.early
            } else {
                profile.tap_perfect.late
            };
            let good = if diff < 0. { profile.tap_good.early } else { profile.tap_good.late };
            let custom =
                (get_data().config.judgement_mode == prpr::config::JudgementMode::Custom).then(|| get_data().config.custom_judgement.effective());
            let j = if miss {
                Judgement::Miss
            } else if let Some(scheme) = custom {
                scheme.classify(-diff * 1000.).map_or(Judgement::Bad, |stage| scheme.outcome(stage))
            } else if diff.abs() <= perfect {
                Judgement::Perfect
            } else if diff.abs() <= good {
                Judgement::Good
            } else {
                Judgement::Bad
            };
            self.preview.record_scaled(
                &JudgeReportEvent {
                    time: s.t as f64,
                    line_id: 0,
                    note_id: 0,
                    judgement: Ok(j),
                    difference: diff,
                },
                &NoteKind::Click,
                &self.draft,
                s.rt as f64,
                timing_bar::extent(custom),
            );
        }
        self.preview.animate(s.rt as f64, &self.draft);
        Ok(())
    }
    fn render(&mut self, ui: &mut Ui, _s: &mut SharedState) -> Result<()> {
        ui.abs_scope(|ui| {
            let top = ui.top;
            if let Some(scene) = &mut self.chart_scene {
                let gl = unsafe { get_internal_gl() };
                let viewport = gl.quad_gl.get_viewport();
                push_camera_state();
                let result = scene.render_timing_preview(&mut self.chart_time, ui);
                pop_camera_state();
                gl.quad_gl.viewport(viewport);
                result?;
            } else {
                ui.fill_rect(Rect::new(-1., -top, 2., top * 2.), Color::from_rgba(18, 22, 30, 255));
                ui.text(self.chart_error.as_deref().unwrap_or("正在加载空谱面…"))
                    .pos(-0.85, 0.)
                    .size(0.4)
                    .max_width(1.1)
                    .draw();
            }
            let profile = self.judge.judgement_range_profile(&get_data().config);
            // Editing must show the bar even when its gameplay switch is off.
            let mut preview_config = self.draft.clone();
            preview_config.enabled = true;
            timing_bar::render(
                ui,
                &preview_config,
                &self.preview,
                &profile,
                (get_data().config.judgement_mode == prpr::config::JudgementMode::Custom).then(|| get_data().config.custom_judgement.effective()),
            );
            if ui.button("timing_editor_controls", Rect::new(-0.95, top - 0.075, 0.25, 0.06), if self.controls { "隐藏面板" } else { "显示面板" })
            {
                self.controls ^= true;
            }
            if ui.button("timing_editor_save", Rect::new(0.74, top - 0.075, 0.21, 0.06), "保存") {
                self.done = true;
            }
            self.panel_rect = None;
            if self.controls {
                let scale = ((top * 2. - 0.28) / 0.64).clamp(0.1, 1.);
                let panel = Rect::new(0.98 - 0.57 * scale, -top + 0.18, 0.57 * scale, 0.64 * scale);
                self.panel_rect = Some(panel);
                ui.fill_rect(panel, Color::from_rgba(28, 40, 54, 240));
                ui.scope(|ui| {
                    ui.dx(panel.x);
                    ui.dy(panel.y);
                    ui.with(Matrix::new_scaling(scale), |ui| {
                        ui.dx(0.02);
                        ui.dy(0.02);
                        if ui.button(
                            "timing_editor_style",
                            Rect::new(0., 0., 0.50, 0.06),
                            if self.draft.curved { "样式：弧形" } else { "样式：直线" },
                        ) {
                            self.draft.curved ^= true;
                        }
                        ui.dy(0.085);
                        ui.scope(|ui| {
                            ui.dx(0.26);
                            Self::numeric(ui, "记录时间s", &mut self.draft.record_seconds, 0.5, 10.);
                        });
                        ui.dy(0.075);
                        ui.slider("记录时间", 0.5..10.0, 0.1, &mut self.draft.record_seconds, Some(0.34));
                        ui.dy(0.12);
                        ui.scope(|ui| {
                            ui.dx(0.26);
                            let mut x = self.draft.x * 100.;
                            Self::numeric(ui, "横坐标%", &mut x, -100., 100.);
                            self.draft.x = x / 100.;
                        });
                        ui.dy(0.075);
                        ui.scope(|ui| {
                            ui.dx(0.26);
                            let mut y = self.draft.y * 100.;
                            Self::numeric(ui, "纵坐标%", &mut y, -100., 100.);
                            self.draft.y = y / 100.;
                        });
                        ui.dy(0.075);
                        ui.scope(|ui| {
                            ui.dx(0.26);
                            Self::numeric(ui, "大小%", &mut self.draft.size, 5., 500.);
                        });
                        ui.dy(0.075);
                        ui.slider("大小", 5.0..500.0, 1., &mut self.draft.size, Some(0.34));
                    });
                });
            }
            Ok(())
        })
    }
    fn next_page(&mut self) -> NextPage {
        if self.done {
            self.done = false;
            NextPage::Pop
        } else {
            NextPage::None
        }
    }
}
