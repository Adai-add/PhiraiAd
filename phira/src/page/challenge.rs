use super::ChartItem;
use crate::{challenge_ui::BadgePicker, get_data, get_data_mut, save_data};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::ui::{DRectButton, Ui};

pub const SELECTOR_EXTRA_HEIGHT: f32 = SELECTOR_HEIGHT - 0.12;
const SELECTOR_HEIGHT: f32 = 0.38 * 2. / 3.;
pub const ENTRY_WIDTH: f32 = 0.24 + 0.018 + 0.14;

pub struct ChallengePanel {
    pub active: bool,
    pub selected: crate::challenge::ChallengeSelection<ChartItem>,
    pub start: Option<[ChartItem; 3]>,
    entry: DRectButton,
    badge: DRectButton,
    cancel: DRectButton,
    clear: DRectButton,
    save_selection: DRectButton,
    positions: [DRectButton; 3],
    remove: [DRectButton; 3],
    begin: DRectButton,
    entry_ready: bool,
    area: Rect,
    picker: BadgePicker,
    restoring: Vec<(usize, i32, prpr::task::Task<Result<std::sync::Arc<crate::client::Chart>>>)>,
}
impl ChallengePanel {
    pub fn new() -> Self {
        Self {
            active: false,
            selected: Default::default(),
            start: None,
            entry: DRectButton::new(),
            badge: DRectButton::new(),
            cancel: DRectButton::new(),
            clear: DRectButton::new(),
            save_selection: DRectButton::new(),
            positions: std::array::from_fn(|_| DRectButton::new()),
            remove: std::array::from_fn(|_| DRectButton::new()),
            begin: DRectButton::new(),
            entry_ready: false,
            area: Rect::new(0., 0., 0., 0.),
            picker: BadgePicker::default(),
            restoring: Vec::new(),
        }
    }
    pub fn ready(&self) -> bool {
        self.selected.ready()
    }
    pub fn select(&mut self, mut chart: ChartItem) {
        if let Some(value) = chart.local_path.as_ref().and_then(|p| resolve_difficulty(p, &chart.info)) {
            chart.info.difficulty = value as f32;
        }
        if !self.selected.select_unique(chart, |a, b| {
            a.info.id.zip(b.info.id).is_some_and(|(a, b)| a == b) || a.local_path.as_ref().zip(b.local_path.as_ref()).is_some_and(|(a, b)| a == b)
        }) {
            prpr::scene::show_message("不能重复选择同一个谱面").warn();
        }
    }
    pub fn close(&mut self) -> bool {
        if self.picker.open {
            self.picker.open = false;
            return true;
        }
        if !self.active {
            return false;
        }
        self.active = false;
        self.restoring.clear();
        self.selected = Default::default();
        self.start = None;
        true
    }
    pub fn update(&mut self, t: f32) {
        self.restoring.retain_mut(|(slot, id, task)| {
            if let Some(result) = task.take() {
                if let Ok(chart) = result {
                    if let Some(selected) = &mut self.selected.slots[*slot] {
                        if selected.info.id == Some(*id) {
                            selected.illu = super::Illustration::from_file_thumbnail(chart.illustration.clone());
                        }
                    }
                }
                false
            } else {
                true
            }
        });
        for chart in self.selected.slots.iter_mut().flatten() {
            chart.illu.settle(t);
        }
    }
    pub fn begin_render(&mut self) {
        self.entry_ready = false;
    }
    /// Lay out both controls in the library toolbar, using its height and baseline.
    pub fn render_entry(&mut self, ui: &mut Ui, t: f32, rect: Rect) {
        self.entry_ready = true;
        self.entry
            .render_text(ui, Rect::new(rect.x, rect.y, 0.24, rect.h), t, "课题模式", 0.35, false);
        let badge_rect = Rect::new(rect.x + 0.24 + 0.018, rect.y, 0.14, rect.h);
        self.badge.render_text(ui, badge_rect, t, "", 0.4, true);
        if let Some(badge) = get_data().challenge_badges.display() {
            crate::challenge_ui::render_display_badge(
                ui,
                Rect::new(badge_rect.x + 0.004, badge_rect.y + 0.004, badge_rect.w - 0.008, badge_rect.h - 0.008),
                badge,
            );
        }
    }
    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<bool> {
        if self.picker.touch(touch, t)? {
            return Ok(true);
        }
        if !self.active {
            if self.entry_ready && self.entry.touch(touch, t) {
                self.active = true;
                self.selected = Default::default();
                self.restoring.clear();
                for (i, saved) in get_data().challenge_selection.iter().enumerate() {
                    if let Some(saved) = saved {
                        let exists = saved
                            .local_path
                            .as_ref()
                            .is_some_and(|p| std::path::Path::new(&format!("{}/{p}", crate::dir::charts().unwrap_or_default())).exists());
                        if !exists && saved.info.id.is_none() {
                            continue;
                        }
                        let local_path = if exists { saved.local_path.clone() } else { None };
                        let illu = local_path
                            .as_ref()
                            .map(|p| super::local_illustration(p.clone(), prpr::ext::BLACK_TEXTURE.clone(), false))
                            .unwrap_or_else(|| super::Illustration::from_done(prpr::ext::BLACK_TEXTURE.clone()));
                        if self.selected.slots.iter().flatten().any(|old| {
                            old.info.id.zip(saved.info.id).is_some_and(|(a, b)| a == b)
                                || old.local_path.as_ref().zip(local_path.as_ref()).is_some_and(|(a, b)| a == b)
                        }) {
                            continue;
                        }
                        if !exists {
                            if let Some(id) = saved.info.id {
                                self.restoring
                                    .push((i, id, prpr::task::Task::new(crate::client::Client::load::<crate::client::Chart>(id))));
                            }
                        }
                        self.selected.slots[i] = Some(ChartItem {
                            info: saved.info.clone(),
                            local_path,
                            illu,
                            chart_type: super::ChartType::Imported,
                        });
                    }
                }
                self.selected.focused = self.selected.slots.iter().position(Option::is_none).unwrap_or(0);
                return Ok(true);
            }
            if self.entry_ready && self.badge.touch(touch, t) {
                self.picker.display();
                return Ok(true);
            }
            return Ok(false);
        }
        if self.cancel.touch(touch, t) {
            self.close();
            return Ok(true);
        }
        if self.clear.touch(touch, t) {
            self.restoring.clear();
            self.selected = Default::default();
            return Ok(true);
        }
        if self.save_selection.touch(touch, t) {
            let saved = std::array::from_fn(|i| {
                self.selected.slots[i].as_ref().map(|chart| crate::data::SavedChallengeChart {
                    info: chart.info.clone(),
                    local_path: chart.local_path.clone(),
                })
            });
            let old = std::mem::replace(&mut get_data_mut().challenge_selection, saved);
            if let Err(error) = save_data() {
                get_data_mut().challenge_selection = old;
                return Err(error);
            }
            prpr::scene::show_message("课题选择已保存").ok();
            return Ok(true);
        }
        for i in 0..3 {
            if self.positions[i].touch(touch, t) {
                self.selected.focused = i;
                return Ok(true);
            }
            if self.selected.slots[i].is_some() && self.remove[i].touch(touch, t) {
                self.selected.remove(i);
                return Ok(true);
            }
        }
        if self.ready() && self.begin.touch(touch, t) {
            self.start = self.selected.course();
            return Ok(true);
        }
        Ok(self.area.contains(touch.position))
    }
    pub fn render(&mut self, ui: &mut Ui, t: f32, enabled: bool) {
        self.entry_ready &= enabled && !self.active;
        let y = -ui.top + 0.04;
        if self.active {
            self.area = Rect::new(-0.7, y, 1.67, SELECTOR_HEIGHT);
            ui.fill_rect(self.area, Color::from_rgba(22, 29, 41, 255));
            self.cancel.render_text(ui, Rect::new(-0.7, y, 0.22, 0.065), t, "取消", 0.4, false);
            self.clear
                .render_text(ui, Rect::new(-0.7, y + 0.085, 0.22, 0.065), t, "清空", 0.35, false);
            self.save_selection
                .render_text(ui, Rect::new(-0.7, y + 0.17, 0.22, 0.065), t, "保存选择", 0.32, false);
            for i in 0..3 {
                let x = -0.46 + i as f32 * 0.40;
                let label = ["1st", "2nd", "3rd"][i];
                self.positions[i].render_text(ui, Rect::new(x, y, 0.38, 0.065), t, label, 0.4, self.selected.focused == i);
                if let Some(chart) = &self.selected.slots[i] {
                    let cover = Rect::new(x + 0.10, y + 0.075, 0.18, 0.12);
                    chart.illu.notify();
                    ui.fill_rect(cover, chart.illu.shading(cover, t));
                    self.remove[i].render_text(ui, Rect::new(x + 0.04, cover.y, 0.05, 0.065), t, "×", 0.5, false);
                    ui.text(&chart.info.name)
                        .pos(x + 0.19, cover.bottom() + 0.004)
                        .anchor(0.5, 0.)
                        .max_width(0.35)
                        .size(0.3)
                        .draw();
                    ui.text(format!(
                        "{}  {:.1}",
                        crate::challenge::difficulty_label(&chart.info.level, chart.info.difficulty as f64),
                        chart.info.difficulty
                    ))
                    .pos(x + 0.19, cover.bottom() + 0.031)
                    .anchor(0.5, 0.)
                    .max_width(0.35)
                    .size(0.3)
                    .draw();
                }
            }
            let rect = Rect::new(0.76, y, 0.21, 0.065);
            if self.ready() {
                self.begin.render_text(ui, rect, t, "开始", 0.4, false);
            } else {
                ui.fill_rect(rect, Color::from_rgba(52, 58, 69, 255));
                ui.text("开始")
                    .pos(rect.center().x, rect.center().y)
                    .anchor(0.5, 0.5)
                    .no_baseline()
                    .size(0.4)
                    .color(GRAY)
                    .draw();
            }
        }
        self.picker.render(ui, t);
    }
}

/// Use the same resolved constant shown by the library, independently of score inclusion.
fn resolve_difficulty(path: &str, info: &crate::data::BriefChartInfo) -> Option<f64> {
    let settings = crate::custom_rks::Settings::load(&std::path::PathBuf::from(crate::dir::root().ok()?).join("custom-rks.json")).ok()?;
    let input = crate::custom_rks::LocalInput {
        path: path.into(),
        name: info.name.clone(),
        level: info.level.clone(),
        difficulty: info.difficulty as f64,
        ai_difficulty: crate::ai_service::value(path),
        accuracy: None,
    };
    crate::custom_rks::resolve(&input, settings.charts.get(path).unwrap_or(&Default::default())).difficulty
}
