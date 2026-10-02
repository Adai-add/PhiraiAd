use crate::{
    challenge::{BadgeTier, ChallengeBadge},
    get_data, get_data_mut, save_data,
};
use anyhow::Result;
use lyon::{math::point, path::Path};
use macroquad::prelude::*;
use prpr::{
    ext::semi_black,
    ui::{DRectButton, Ui},
};

fn polygon(ui: &mut Ui, points: [(f32, f32); 4], color: Color) {
    let mut path = Path::builder();
    path.begin(point(points[0].0, points[0].1));
    for (x, y) in points.into_iter().skip(1) {
        path.line_to(point(x, y));
    }
    path.close();
    ui.fill_path(&path.build(), color);
}

pub fn slanted_path(rect: Rect, skew: f32) -> Path {
    let mut path = Path::builder();
    path.begin(point(rect.x + skew, rect.y));
    path.line_to(point(rect.right(), rect.y));
    path.line_to(point(rect.right() - skew, rect.bottom()));
    path.line_to(point(rect.x, rect.bottom()));
    path.close();
    path.build()
}

/// Both badge options use this same background, keyed only by total score.
pub fn render_badge(ui: &mut Ui, rect: Rect, badge: &ChallengeBadge) {
    render_badge_with_stripe(ui, rect, badge, 1.);
}
pub fn render_display_badge(ui: &mut Ui, rect: Rect, badge: &ChallengeBadge) {
    render_badge_with_stripe(ui, rect, badge, 2.2);
}
fn render_badge_with_stripe(ui: &mut Ui, rect: Rect, badge: &ChallengeBadge, stripe_scale: f32) {
    let colors = match badge.tier {
        BadgeTier::Green => (Color::from_rgba(38, 72, 24, 255), Color::from_rgba(100, 255, 0, 255)),
        BadgeTier::Blue => (Color::from_rgba(27, 56, 70, 255), Color::from_rgba(0, 222, 245, 255)),
        BadgeTier::Orange => (Color::from_rgba(97, 42, 26, 255), Color::from_rgba(255, 101, 0, 255)),
        BadgeTier::Yellow => (Color::from_rgba(126, 119, 25, 255), Color::from_rgba(255, 248, 0, 255)),
        BadgeTier::Rainbow => (Color::from_rgba(45, 100, 68, 255), Color::from_rgba(0, 207, 244, 255)),
    };
    let skew = rect.h * 0.22;
    let poly = |x: f32, w: f32| [(x + skew, rect.y), (x + w, rect.y), (x + w - skew, rect.bottom()), (x, rect.bottom())];
    polygon(ui, poly(rect.x, rect.w), colors.0);
    if badge.tier == BadgeTier::Rainbow {
        let palette = [
            Color::from_rgba(35, 134, 173, 255),
            Color::from_rgba(59, 143, 76, 255),
            Color::from_rgba(125, 136, 61, 255),
            Color::from_rgba(190, 68, 76, 255),
        ];
        for i in 0..20 {
            let p = i as f32 / 19. * 3.;
            let j = (p.floor() as usize).min(2);
            let a = palette[j];
            let b = palette[j + 1];
            let f = (p - j as f32).min(1.);
            let color = Color::new(a.r + (b.r - a.r) * f, a.g + (b.g - a.g) * f, a.b + (b.b - a.b) * f, 1.);
            let x = rect.x + rect.w * i as f32 / 20.;
            let x2 = rect.x + rect.w * (i + 1) as f32 / 20.;
            polygon(
                ui,
                [
                    (x + skew * (1. - i as f32 / 20.), rect.y),
                    (x2 + skew * (1. - (i + 1) as f32 / 20.), rect.y),
                    (x2 - skew * (i + 1) as f32 / 20., rect.bottom()),
                    (x - skew * i as f32 / 20., rect.bottom()),
                ],
                color,
            );
        }
    }
    let stripe = rect.w * 0.025 * stripe_scale;
    polygon(
        ui,
        [
            (rect.x + skew, rect.y),
            (rect.x + skew + stripe, rect.y),
            (rect.x + stripe, rect.bottom()),
            (rect.x, rect.bottom()),
        ],
        colors.1,
    );
    let right_color = if badge.tier == BadgeTier::Rainbow {
        Color::from_rgba(255, 73, 87, 255)
    } else {
        colors.1
    };
    polygon(
        ui,
        [
            (rect.right() - stripe, rect.y),
            (rect.right(), rect.y),
            (rect.right() - skew, rect.bottom()),
            (rect.right() - skew - stripe, rect.bottom()),
        ],
        right_color,
    );
    ui.text(badge.label())
        .pos(rect.center().x, rect.center().y)
        .anchor(0.5, 0.5)
        .no_baseline()
        .size((rect.h * 5.).min(1.8))
        .max_width(rect.w * 0.8)
        .color(WHITE)
        .draw();
}

pub struct BadgePicker {
    pub open: bool,
    saving: Option<ChallengeBadge>,
    buttons: [DRectButton; 5],
    close: DRectButton,
}
impl Default for BadgePicker {
    fn default() -> Self {
        Self {
            open: false,
            saving: None,
            buttons: std::array::from_fn(|_| DRectButton::new()),
            close: DRectButton::new(),
        }
    }
}
impl BadgePicker {
    pub fn save(&mut self, badge: ChallengeBadge) {
        self.saving = Some(badge);
        self.open = true;
    }
    pub fn display(&mut self) {
        self.saving = None;
        self.open = true;
    }
    pub fn touch(&mut self, touch: &Touch, t: f32) -> Result<bool> {
        if !self.open {
            return Ok(false);
        }
        if self.close.touch(touch, t) {
            self.open = false;
            return Ok(true);
        }
        for (i, button) in self.buttons.iter_mut().enumerate() {
            if button.touch(touch, t) {
                let mut slots = get_data().challenge_badges.clone();
                let changed = if let Some(badge) = &self.saving {
                    slots.save(i, badge.clone())
                } else {
                    slots.select(i)
                };
                if changed {
                    let old = std::mem::replace(&mut get_data_mut().challenge_badges, slots);
                    if let Err(error) = save_data() {
                        get_data_mut().challenge_badges = old;
                        return Err(error);
                    }
                    self.open = false;
                }
                return Ok(true);
            }
        }
        Ok(true)
    }
    pub fn render(&mut self, ui: &mut Ui, t: f32) {
        if !self.open {
            return;
        }
        ui.fill_rect(ui.screen_rect(), semi_black(0.8));
        ui.fill_rect(Rect::new(-0.91, -0.22, 1.82, 0.46), Color::from_rgba(25, 31, 43, 255));
        ui.text(if self.saving.is_some() {
            "保存到槽位"
        } else {
            "选择展示槽位"
        })
        .pos(0., -0.19)
        .anchor(0.5, 0.)
        .size(0.55)
        .draw();
        for i in 0..5 {
            let rect = Rect::new(-0.85 + i as f32 * 0.345, -0.085, 0.32, 0.20);
            let data = get_data();
            let badge = data.challenge_badges.slots[i].as_ref();
            self.buttons[i].render_text(ui, rect, t, "", 0.4, data.challenge_badges.displayed == Some(i));
            ui.text(format!("槽位 {}", i + 1))
                .pos(rect.center().x, rect.y + 0.015)
                .anchor(0.5, 0.)
                .size(0.35)
                .draw();
            if let Some(badge) = badge {
                render_badge(ui, Rect::new(rect.x + 0.018, rect.y + 0.072, rect.w - 0.036, 0.095), badge);
            } else {
                ui.text("空").pos(rect.center().x, rect.y + 0.09).anchor(0.5, 0.).size(0.4).draw();
            }
        }
        self.close.render_text(ui, Rect::new(-0.12, 0.15, 0.24, 0.065), t, "返回", 0.4, false);
    }
}
