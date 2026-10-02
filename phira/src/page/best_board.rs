//! Three-column view of the exact candidates contributing to custom RKS.
use super::{Illustration, SharedState};
use crate::{custom_rks::RankedEntry, get_data};
use macroquad::prelude::*;
use prpr::ui::{DRectButton, Scroll, Ui};

const UNIT: f32 = 1.88 / 1200.;
const FONT_SCALE: f32 = 0.83;
const FONT_UNIT: f32 = UNIT * 1.121 / 0.08;
const GAP: f32 = 12. * UNIT;
const CARD_WIDTH: f32 = 394. * 0.9 * 0.95 * UNIT;
const CARD_HEIGHT: f32 = 85. * UNIT;
const BOARD_WIDTH: f32 = CARD_WIDTH * 3. + GAP * 2.;

use crate::challenge::difficulty_label;

struct Card {
    entry: RankedEntry,
    illustration: Option<Illustration>,
    score: Option<i32>,
    level: &'static str,
}

pub struct BestBoard {
    pub open: bool,
    pub entry_active: bool,
    entry_button: DRectButton,
    close_button: DRectButton,
    export_button: DRectButton,
    pub export_requested: bool,
    scroll: Scroll,
    cards: Vec<Card>,
    player: String,
    rks: f64,
    course_badge: Option<crate::challenge::ChallengeBadge>,
}

impl BestBoard {
    pub fn new() -> Self {
        Self {
            open: false,
            entry_active: false,
            entry_button: DRectButton::new(),
            close_button: DRectButton::new(),
            export_button: DRectButton::new(),
            export_requested: false,
            scroll: Scroll::new(),
            cards: Vec::new(),
            player: String::new(),
            rks: 0.,
            course_badge: None,
        }
    }

    pub fn show(&mut self, entries: Vec<RankedEntry>, rks: f64, state: &SharedState) {
        let data = get_data();
        self.player = data.me.as_ref().map(|user| user.name.clone()).unwrap_or_else(|| "离线玩家".into());
        self.rks = rks;
        self.course_badge = data.challenge_badges.display().cloned();
        self.cards = entries
            .into_iter()
            .map(|entry| {
                let chart = entry
                    .path
                    .as_ref()
                    .and_then(|path| state.charts_local.iter().find(|chart| chart.local_path.as_ref() == Some(path)));
                let score = entry.path.as_ref().and_then(|path| {
                    data.charts
                        .iter()
                        .filter(|chart| &chart.local_path == path)
                        .filter_map(|chart| chart.record.as_ref().map(|record| record.score))
                        .chain(data.local_records.get(path).and_then(|record| record.as_ref()).map(|record| record.score))
                        .chain(
                            chart
                                .and_then(|chart| chart.info.id)
                                .and_then(|id| data.replica_local_records.get(&id))
                                .map(|record| record.score),
                        )
                        .max()
                });
                let level = difficulty_label(chart.map(|chart| chart.info.level.as_str()).unwrap_or(""), entry.difficulty);
                Card {
                    level,
                    illustration: chart.map(|chart| chart.illu.clone()),
                    entry,
                    score,
                }
            })
            .collect();
        self.scroll = Scroll::new();
        self.entry_button.inner.cancel();
        self.close_button.inner.cancel();
        self.export_button.inner.cancel();
        self.export_requested = false;
        self.open = true;
    }

    pub fn close(&mut self) -> bool {
        let was_open = self.open;
        self.open = false;
        self.entry_button.inner.cancel();
        self.close_button.inner.cancel();
        self.export_button.inner.cancel();
        self.export_requested = false;
        self.cards.clear();
        was_open
    }

    pub fn entry_touch(&mut self, touch: &Touch, t: f32) -> bool {
        self.entry_active && self.entry_button.touch(touch, t)
    }

    pub fn touch(&mut self, touch: &Touch, t: f32) {
        if self.close_button.touch(touch, t) {
            self.close();
            return;
        }
        if self.export_button.touch(touch, t) {
            self.export_requested = true;
            return;
        }
        self.scroll.touch(touch, t);
    }

    pub fn update(&mut self, t: f32) {
        if self.open {
            self.scroll.update(t);
            for card in &mut self.cards {
                if let Some(illustration) = &mut card.illustration {
                    illustration.settle(t);
                }
            }
        }
    }

    pub fn render_entry(&mut self, ui: &mut Ui, t: f32, rect: Rect, n: usize) {
        self.entry_active = true;
        self.entry_button.render_text(ui, rect, t, format!("查看 B{n}"), 0.4, false);
    }

    pub fn render(&mut self, ui: &mut Ui, t: f32) {
        if !self.open {
            return;
        }
        ui.fill_rect(ui.screen_rect(), Color::from_rgba(17, 23, 34, 255));
        let width = BOARD_WIDTH;
        let top = -ui.top + 0.035;
        self.render_header(ui, top, -width / 2., width / 2. - 0.39);
        self.export_button
            .render_text(ui, Rect::new(width / 2. - 0.37, top, 0.18, 0.07), t, "导出图片", 0.34, false);
        self.close_button
            .render_text(ui, Rect::new(width / 2. - 0.17, top, 0.17, 0.07), t, "关闭", 0.4, false);
        let viewport_height = (ui.top * 2. - 0.24).max(0.05);
        let content_height = self.cards_height().max(viewport_height);
        let offset = self.scroll.y_scroller.offset;
        self.scroll.size((width, viewport_height));
        ui.scope(|ui| {
            ui.dx(-width / 2.);
            ui.dy(top + 0.1);
            self.scroll.render(ui, |ui| {
                Self::render_cards(&mut self.cards, ui, t, offset, viewport_height);
                (width, content_height)
            });
        });
        let footer_y = ui.top - 0.055;
        ui.text("PhiraiAd").pos(0., footer_y).anchor(0.5, 0.5).size(0.46).draw();
    }
    fn cards_height(&self) -> f32 {
        if self.cards.is_empty() {
            0.20
        } else {
            self.cards.len().div_ceil(3) as f32 * (CARD_HEIGHT + GAP) - GAP
        }
    }
    /// Center the whole row; shift left before shortening only the player name.
    fn render_header(&self, ui: &mut Ui, y: f32, left: f32, right: f32) {
        let size = 0.6;
        let measure = |ui: &mut Ui, text: &str| ui.text(text).size(size).measure().w;
        let middle = format!(" · RKS {:.4}{}", self.rks, if self.course_badge.is_some() { " · " } else { "" });
        let end = format!(" · B{}", self.cards.len());
        let middle_width = measure(ui, &middle);
        let end_width = measure(ui, &end);
        let badge_width = self.course_badge.as_ref().map(|badge| (measure(ui, &badge.label()) + 0.04).max(0.10)).unwrap_or(0.);
        let fixed_width = middle_width + badge_width + end_width;
        let name = fit_header_name(ui, &self.player, size, (right - left - fixed_width).max(0.001));
        let name_width = measure(ui, &name);
        let total = name_width + fixed_width;
        let mut x = (-total / 2.).min(right - total).max(left);
        let center_y = y + 0.035;
        ui.text(&name).pos(x, center_y).anchor(0., 0.5).no_baseline().size(size).draw();
        x += name_width;
        ui.text(&middle).pos(x, center_y).anchor(0., 0.5).no_baseline().size(size).draw();
        x += middle_width;
        if let Some(badge) = &self.course_badge {
            crate::challenge_ui::render_display_badge(ui, Rect::new(x, center_y - 0.0275, badge_width, 0.055), badge);
            x += badge_width;
        }
        ui.text(&end).pos(x, center_y).anchor(0., 0.5).no_baseline().size(size).draw();
    }
    pub fn export_width(&self) -> f32 { BOARD_WIDTH }
    pub fn export_height(&self) -> f32 {
        0.135 + self.cards_height() + 0.11
    }
    pub fn illustrations_ready(&self, t: f32) -> bool {
        let mut ready = true;
        for illu in self.cards.iter().filter_map(|card| card.illustration.as_ref()) {
            illu.notify();
            ready &= illu.alpha(t) >= 1.;
        }
        ready
    }
    /// Full board coordinates, sliced by the caller without a scroll container.
    pub fn render_export(&mut self, ui: &mut Ui, t: f32, offset: f32) {
        ui.fill_rect(ui.screen_rect(), Color::from_rgba(17, 23, 34, 255));
        let y = -ui.top - offset;
        self.render_header(ui, y + 0.035, -BOARD_WIDTH / 2., BOARD_WIDTH / 2.);
        let viewport_height = ui.top * 2.;
        ui.scope(|ui| {
            ui.dx(-BOARD_WIDTH / 2.);
            ui.dy(y + 0.135);
            Self::render_cards(&mut self.cards, ui, t, offset - 0.135, viewport_height);
        });
        ui.text("PhiraiAd")
            .pos(0., y + self.export_height() - 0.055)
            .anchor(0.5, 0.5)
            .size(0.46)
            .draw();
    }
    fn render_cards(cards: &mut [Card], ui: &mut Ui, t: f32, offset: f32, viewport_height: f32) {
        let gap = GAP;
        let card_width = CARD_WIDTH;
        let card_height = CARD_HEIGHT;
        let width = BOARD_WIDTH;
        let title_size = 18. * FONT_SCALE * FONT_UNIT;
        let score_size = 20. * FONT_SCALE * FONT_UNIT;
        let detail_size = 16. * FONT_SCALE * FONT_UNIT;
        for (index, card) in cards.iter_mut().enumerate() {
            let x = (index % 3) as f32 * (card_width + gap);
            let y = (index / 3) as f32 * (card_height + gap);
            if y + card_height < offset || y > offset + viewport_height {
                continue;
            }
            let rect = Rect::new(x, y, card_width, card_height);
            ui.fill_rect(rect, Color::from_rgba(34, 44, 60, 255));
            let cover = Rect::new(x, y, card_height * 2., card_height);
            if let Some(illustration) = &card.illustration {
                illustration.notify();
                ui.fill_rect(cover, illustration.shading(cover, t));
            } else {
                ui.fill_rect(cover, Color::from_rgba(53, 65, 83, 255));
                ui.text("♪").pos(cover.center().x, cover.center().y).anchor(0.5, 0.5).size(0.8).draw();
            }
            let tx = cover.right() + 10. * UNIT;
            let text_width = rect.right() - tx - 8. * UNIT;
            let level_size = detail_size * 0.85;
            let level_width = ui.text(card.level).size(level_size).measure().w;
            let ap_width = if card.entry.ap { 34. * FONT_SCALE * UNIT } else { 0. };
            let title = ui
                .text(&card.entry.name)
                .pos(tx, y + 24. * UNIT)
                .anchor(0., 1.)
                .size(title_size)
                .max_width((text_width - ap_width - level_width - 5. * UNIT).max(0.001))
                .draw();
            ui.text(card.level)
                .pos(tx + title.w + 5. * UNIT, y + 24. * UNIT)
                .anchor(0., 1.)
                .size(level_size)
                .draw();
            if card.entry.ap {
                ui.text("AP")
                    .pos(rect.right() - 8. * UNIT, y + 24. * UNIT)
                    .anchor(1., 1.)
                    .size(detail_size)
                    .color(YELLOW)
                    .draw();
            }
            let score = card.score.map_or_else(|| "—".into(), |score| format!("{score:07}"));
            let score_rect = ui
                .text(score)
                .pos(tx, y + 50. * UNIT)
                .anchor(0., 1.)
                .size(score_size)
                .max_width(text_width)
                .draw();
            let acc_x = tx + score_rect.w + 7. * UNIT;
            ui.text(format!("ACC {:.2}%", card.entry.accuracy))
                .pos(acc_x, y + 50. * UNIT)
                .anchor(0., 1.)
                .size(detail_size)
                .max_width((rect.right() - 8. * UNIT - acc_x).max(0.001))
                .draw();
            ui.text(format!("{:.1} | {:.2}", card.entry.difficulty, card.entry.rks))
                .pos(tx, y + 74. * UNIT)
                .anchor(0., 1.)
                .size(detail_size)
                .max_width(text_width)
                .draw();
        }
        if cards.is_empty() {
            ui.text("暂无可计入的成绩").pos(width / 2., 0.1).anchor(0.5, 0.).size(0.5).draw();
        }
    }
}

/// Shorten at Unicode character boundaries using actual glyph advances.
fn fit_header_name(ui: &mut Ui, name: &str, size: f32, width: f32) -> String {
    if ui.text(name).size(size).measure().w <= width {
        return name.to_owned();
    }
    if ui.text("…").size(size).measure().w > width {
        return String::new();
    }
    let ends: Vec<_> = name.char_indices().map(|(i, _)| i).chain(std::iter::once(name.len())).collect();
    let (mut low, mut high) = (0, ends.len() - 1);
    while low < high {
        let mid = (low + high + 1) / 2;
        if ui.text(format!("{}…", &name[..ends[mid]])).size(size).measure().w <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!("{}…", &name[..ends[low]])
}

#[cfg(test)]
mod tests {
    use super::difficulty_label;
    #[test]
    fn standard_chart_labels_take_priority_over_the_constant() {
        for (level, expected) in [("EZ Lv.15", "EZ"), (" hd Lv.2 ", "HD"), ("IN", "IN"), ("AT Lv.6", "AT")] {
            assert_eq!(difficulty_label(level, 10.), expected);
        }
    }
    #[test]
    fn unknown_labels_use_the_exact_requested_boundaries() {
        for (constant, expected) in [
            (0., "EZ"),
            (7.9, "EZ"),
            (8., "HD"),
            (12.9, "HD"),
            (13., "IN"),
            (16.4, "IN"),
            (16.5, "AT"),
            (20., "AT"),
        ] {
            assert_eq!(difficulty_label("INS Lv.15", constant), expected);
            assert_eq!(difficulty_label("", constant), expected);
        }
    }
}
