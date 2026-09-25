//! Live chart preview controls. The caller owns music seeking and explicit persistence.
use super::Ui;
use crate::chart_play::ChartPlaySettings;
use macroquad::prelude::*;

pub enum ChartPlayEditorAction {
    Seek(f64),
    TogglePause,
    Save,
    Cancel,
}
#[derive(Default)]
pub struct ChartPlayEditor {
    start: f32,
    end: f32,
    selected: Option<usize>,
    page: usize,
    message: String,
}

impl ChartPlayEditor {
    /// Independent of the active chart camera, its aspect ratio and render target.
    pub fn camera(ui: &Ui, target: Option<RenderTarget>) -> Camera2D {
        Camera2D {
            render_target: target,
            ..ui.camera()
        }
    }

    pub fn render(&mut self, ui: &mut Ui, settings: &mut ChartPlaySettings, time: f64, duration: f64, paused: bool) -> Option<ChartPlayEditorAction> {
        let mut action = None;
        let before = (self.start, self.end);
        let editing = self.selected;
        let duration = duration.max(0.001);
        let current = time.clamp(0., duration) as f32;
        let scale = ((ui.top * 2. - 0.19) / 0.83).min(1.).max(0.45);
        ui.scope(|ui| {
            ui.dx(0.25);
            ui.dy(-ui.top + 0.025);
            ui.with(crate::core::Matrix::new_scaling(scale), |ui| {
                ui.fill_rect(Rect::new(0., 0., 0.72, 0.83), Color::new(0.07, 0.08, 0.10, 0.94));
                ui.text("自动倒打").pos(0.025, 0.02).size(0.52).no_baseline().draw();
                ui.dy(-0.06);
                if ui.button("flip-preview-play", Rect::new(0.025, 0.15, 0.20, 0.06), if paused { "播放" } else { "暂停" }) {
                    action = Some(ChartPlayEditorAction::TogglePause);
                }
                if ui.button("flip-preview-back", Rect::new(0.24, 0.15, 0.20, 0.06), "后退 1 秒") {
                    action = Some(ChartPlayEditorAction::Seek((time - 1.).max(0.)));
                }
                if ui.button("flip-preview-forward", Rect::new(0.455, 0.15, 0.24, 0.06), "前进 1 秒") {
                    action = Some(ChartPlayEditorAction::Seek((time + 1.).min(duration)));
                }
                ui.text(format!("预览 {:.3} 秒  {}", current, if settings.flipped_at(time) { "倒打中" } else { "正常" }))
                    .pos(0.025, 0.225)
                    .size(0.36)
                    .no_baseline()
                    .draw();
                if ui.button("flip-mark-start", Rect::new(0.025, 0.275, 0.325, 0.065), format!("开始 {:.3} ← 当前", self.start)) {
                    self.start = current;
                    self.message.clear();
                }
                if ui.button("flip-mark-end", Rect::new(0.37, 0.275, 0.325, 0.065), format!("结束 {:.3} ← 当前", self.end)) {
                    self.end = current;
                    self.message.clear();
                }
                for (index, delta, label) in [
                    (0, -0.01, "开始 -0.01"),
                    (1, 0.01, "开始 +0.01"),
                    (2, -0.01, "结束 -0.01"),
                    (3, 0.01, "结束 +0.01"),
                ] {
                    if ui.button(&format!("flip-fine-{index}"), Rect::new(0.025 + index as f32 * 0.17, 0.35, 0.16, 0.055), label) {
                        let value = if index < 2 { &mut self.start } else { &mut self.end };
                        *value = (*value + delta).clamp(0., duration as f32);
                    }
                }
                if editing == self.selected && (self.start, self.end) != before && self.end > self.start {
                    if let Some(i) = self.selected {
                        if i < settings.auto_flip_intervals.len() {
                            settings.auto_flip_intervals.remove(i);
                            settings.add(self.start as f64, self.end as f64);
                            self.selected = settings
                                .auto_flip_intervals
                                .iter()
                                .position(|r| r.start <= self.start as f64 && r.end >= self.end as f64);
                            if let Some(i) = self.selected {
                                self.start = settings.auto_flip_intervals[i].start as f32;
                                self.end = settings.auto_flip_intervals[i].end as f32;
                            }
                            self.message = "已实时更新预览".into();
                        }
                    }
                }
                if ui.button("flip-add", Rect::new(0.025, 0.42, 0.325, 0.06), if self.selected.is_some() { "完成编辑" } else { "添加区间" }) {
                    if self.end > self.start {
                        if let Some(i) = self.selected.take() {
                            if i < settings.auto_flip_intervals.len() {
                                settings.auto_flip_intervals.remove(i);
                            }
                        }
                        settings.add(self.start as f64, self.end as f64);
                        self.message = "已添加".into();
                    } else {
                        self.message = "结束必须晚于开始".into();
                    }
                }
                if ui.button("flip-new", Rect::new(0.37, 0.42, 0.325, 0.06), "新建 / 取消选中") {
                    self.selected = None;
                    self.start = current;
                    self.end = (current + 1.).min(duration as f32);
                    self.message.clear();
                }
                self.page = self.page.min(settings.auto_flip_intervals.len().saturating_sub(1) / 3);
                let mut remove = None;
                for (row, i) in (self.page * 3..settings.auto_flip_intervals.len().min(self.page * 3 + 3)).enumerate() {
                    let r = settings.auto_flip_intervals[i];
                    let y = 0.50 + row as f32 * 0.06;
                    if ui.button(
                        &format!("flip-select-{i}"),
                        Rect::new(0.025, y, 0.54, 0.055),
                        format!("{}{}  {:.3} — {:.3}", if self.selected == Some(i) { "✓ " } else { "" }, i + 1, r.start, r.end),
                    ) {
                        self.selected = Some(i);
                        self.start = r.start as f32;
                        self.end = r.end as f32;
                        action = Some(ChartPlayEditorAction::Seek(r.start));
                    }
                    if ui.button(&format!("flip-remove-{i}"), Rect::new(0.58, y, 0.115, 0.055), "删除") {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    settings.auto_flip_intervals.remove(i);
                    self.selected = None;
                }
                if ui.button("flip-prev", Rect::new(0.025, 0.685, 0.16, 0.05), "上一页") {
                    self.page = self.page.saturating_sub(1);
                }
                if ui.button("flip-next", Rect::new(0.20, 0.685, 0.16, 0.05), "下一页") && (self.page + 1) * 3 < settings.auto_flip_intervals.len()
                {
                    self.page += 1;
                }
                ui.text(format!("共 {} 段", settings.auto_flip_intervals.len()))
                    .pos(0.38, 0.70)
                    .size(0.29)
                    .no_baseline()
                    .draw();
                ui.text(&self.message).pos(0.025, 0.75).size(0.28).no_baseline().max_width(0.67).draw();
                if ui.button("flip-cancel", Rect::new(0.025, 0.80, 0.325, 0.06), "取消") {
                    action = Some(ChartPlayEditorAction::Cancel);
                }
                if ui.button("flip-save", Rect::new(0.37, 0.80, 0.325, 0.06), "保存并返回") {
                    settings.normalize();
                    action = Some(ChartPlayEditorAction::Save);
                }
            });
        });
        ui.scope(|ui| {
            ui.dx(-0.95);
            ui.dy(ui.top - 0.14);
            ui.fill_rect(Rect::new(-0.015, -0.01, 1.93, 0.13), Color::new(0.03, 0.04, 0.06, 0.9));
            let mut cursor = current;
            ui.slider("谱面进度（秒）", 0.0..duration as f32, 0.001, &mut cursor, Some(1.90));
            // Match Ui::slider's track Y (label height + 0.03).
            let cy = ui.text(format!("谱面进度（秒）: {current:.3}")).size(0.4).measure().h + 0.03;
            draw_flip_markers(ui, settings, 0., cy, 1.9, 0., duration);
            if (cursor - current).abs() > 0.0005 {
                action = Some(ChartPlayEditorAction::Seek(cursor as f64));
            }
        });
        action
    }
}

/// start is green, end is red; times outside the visible track are clipped.
pub fn draw_flip_markers(ui: &mut Ui, settings: &ChartPlaySettings, x: f32, y: f32, width: f32, offset: f64, duration: f64) {
    if duration <= 0. {
        return;
    }
    for r in &settings.auto_flip_intervals {
        for (time, color) in [(r.start, GREEN), (r.end, RED)] {
            let t = (time + offset) / duration;
            if (0.0..=1.0).contains(&t) {
                ui.fill_rect(Rect::new(x + t as f32 * width - 0.0025, y - 0.014, 0.005, 0.028), color);
            }
        }
    }
}
