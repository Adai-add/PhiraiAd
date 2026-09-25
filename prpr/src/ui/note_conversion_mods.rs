//! Per-chart note conversion controls in the song Mods panel.
use super::{DRectButton, Ui};
use crate::chart_play::NoteConversion;
use macroquad::prelude::*;

#[derive(Default)]
pub struct NoteConversionMods {
    buttons: [DRectButton; 3],
}
impl NoteConversionMods {
    const MODES: [NoteConversion; 3] = [NoteConversion::Tap, NoteConversion::Drag, NoteConversion::Flick];
    pub fn touch(&mut self, touch: &Touch, time: f32, current: NoteConversion) -> Option<NoteConversion> {
        for (button, mode) in self.buttons.iter_mut().zip(Self::MODES) {
            if button.touch(touch, time) {
                return Some(if current == mode { NoteConversion::Original } else { mode });
            }
        }
        None
    }
    pub fn render(&mut self, ui: &mut Ui, time: f32, width: f32, current: NoteConversion) -> f32 {
        ui.text("按键转换").pos(0.03, 0.025).size(0.6).no_baseline().draw();
        ui.text("Hold仅保留头部。")
            .pos(0.03, 0.09)
            .size(0.3)
            .max_width(width - 0.06)
            .no_baseline()
            .draw();
        for (i, (mode, label)) in Self::MODES.into_iter().zip(["全 Tap", "全 Drag", "全 Flick"]).enumerate() {
            let y = 0.15 + i as f32 * 0.15;
            ui.text(label).pos(0.03, y + 0.075).anchor(0., 0.5).size(0.6).no_baseline().draw();
            let rect = Rect::new(width - 0.24, y + 0.03, 0.2, 0.09);
            let on = current == mode;
            self.buttons[i].build(ui, time, rect, |ui, path| {
                ui.fill_path(&path, if on { WHITE } else { ui.background() });
                let center = rect.center();
                ui.text(if on { "开启" } else { "关闭" })
                    .pos(center.x, center.y)
                    .anchor(0.5, 0.5)
                    .no_baseline()
                    .size(0.5)
                    .color(if on { BLACK } else { WHITE })
                    .draw();
            });
        }
        0.60
    }
}
