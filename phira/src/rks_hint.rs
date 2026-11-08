//! Shared production drawing for the library's RKS improvement row.
use crate::custom_rks::RksGainHint;
use macroquad::prelude::*;
use prpr::{ext::RectExt, ui::Ui};

pub fn render(ui: &mut Ui, hint: RksGainHint, card: Rect, accuracy: Rect, mut size: f32, inherited: Color) -> Rect {
    let color = Color {
        a: inherited.a,
        ..match hint {
            RksGainHint::Phi => Color::from_rgba(255, 215, 0, 255),
            RksGainHint::NoGain => Color::from_rgba(70, 230, 100, 255),
            _ => inherited,
        }
    };
    let label = hint.text();
    let width = ui.text(&label).size(size).measure().w;
    let available = (card.w - 0.032).max(0.001);
    if width > available {
        size *= available / width;
    }
    let mut text = ui
        .text(label)
        .pos(card.right() - 0.016, accuracy.bottom() + 0.012)
        .anchor(1., 0.)
        .size(size)
        .color(color);
    let bounds = text.measure();
    text.ui.fill_path(
        &bounds.feather(0.006).rounded(0.007),
        Color {
            a: inherited.a * 0.7,
            ..text.ui.background()
        },
    );
    text.draw();
    bounds
}
