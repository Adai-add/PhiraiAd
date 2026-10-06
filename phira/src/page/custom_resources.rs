use super::{ NextPage, Page, SharedState };
use crate::{ custom_resources::{ self, Kind, MenuResources }, get_data, get_data_mut, save_data };
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    ext::RectExt,
    scene::{ request_file, show_error, show_message },
    ui::{ DRectButton, Scroll, Slider, Ui },
};
use std::borrow::Cow;

pub struct ResourceEditor {
    kind: Kind,
    scroll: Scroll,
    import: DRectButton,
    reset: DRectButton,
    sliders: Vec<Slider>,
    dirty_at: Option<f32>,
}
impl ResourceEditor {
    pub fn new(kind: Kind) -> Self {
        let sliders = Self::specs(kind)
            .into_iter()
            .map(|(_, min, max, step)| Slider::new(min..max, step).with_relative_drag(0.01))
            .collect();
        Self {
            kind,
            scroll: Scroll::new(),
            import: DRectButton::new(),
            reset: DRectButton::new(),
            sliders,
            dirty_at: None,
        }
    }
    fn specs(kind: Kind) -> Vec<(&'static str, f32, f32, f32)> {
        match kind {
            Kind::Music => vec![("音乐音量 %", 0.0, 200.0, 1.0)],
            Kind::Background =>
                vec![
                    ("背景亮度 %", 0.0, 100.0, 1.0),
                    ("背景缩放 %", 5.0, 500.0, 1.0),
                    ("背景横坐标 %", -200.0, 200.0, 1.0),
                    ("背景纵坐标 %", -200.0, 200.0, 1.0),
                    ("背景模糊度", 0.0, 200.0, 1.0)
                ],
            Kind::Character =>
                vec![
                    ("立绘大小 %", 5.0, 500.0, 1.0),
                    ("立绘横坐标 %", -200.0, 200.0, 1.0),
                    ("立绘纵坐标 %", -200.0, 200.0, 1.0),
                    ("立绘旋转 °", -180.0, 180.0, 1.0)
                ],
        }
    }
    fn value(kind: Kind, i: usize, c: &MenuResources) -> f32 {
        match kind {
            Kind::Music => c.volume * 100.0,
            Kind::Background =>
                [
                    c.brightness * 100.0,
                    c.background_scale,
                    c.background_x,
                    c.background_y,
                    c.background_blur,
                ][i],
            Kind::Character =>
                [c.character_scale, c.character_x, c.character_y, c.character_rotation][i],
        }
    }
    fn set(kind: Kind, i: usize, c: &mut MenuResources, value: f32) {
        match kind {
            Kind::Music => {
                c.volume = value / 100.0;
            }
            Kind::Background =>
                match i {
                    0 => {
                        c.brightness = value / 100.0;
                    }
                    1 => {
                        c.background_scale = value;
                    }
                    2 => {
                        c.background_x = value;
                    }
                    3 => {
                        c.background_y = value;
                    }
                    _ => {
                        c.background_blur = value;
                    }
                }
            Kind::Character =>
                match i {
                    0 => {
                        c.character_scale = value;
                    }
                    1 => {
                        c.character_x = value;
                    }
                    2 => {
                        c.character_y = value;
                    }
                    _ => {
                        c.character_rotation = value;
                    }
                }
        }
    }
}
impl Page for ResourceEditor {
    fn label(&self) -> Cow<'static, str> {
        self.kind.title().into()
    }
    fn exit(&mut self) -> Result<()> {
        save_data()
    }
    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        self.scroll.update(s.t);
        if self.dirty_at.is_some_and(|t| s.t - t > 0.5) {
            save_data()?;
            self.dirty_at = None;
        }
        Ok(())
    }
    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        if self.scroll.touch(touch, s.t) {
            return Ok(true);
        }
        if self.import.touch(touch, s.t) {
            if !custom_resources::busy() {
                request_file(self.kind.request());
            } else {
                show_message("正在加载资源，请稍候").ok();
            }
            return Ok(true);
        }
        if self.reset.touch(touch, s.t) {
            if let Err(error) = custom_resources::reset(self.kind) {
                show_error(error);
            } else {
                show_message("已恢复默认").ok();
                self.dirty_at = None;
            }
            return Ok(true);
        }
        for (i, slider) in self.sliders.iter_mut().enumerate() {
            let mut value = Self::value(self.kind, i, &get_data().menu_resources);
            if let Some(changed) = slider.touch(touch, s.t, &mut value) {
                if changed {
                    Self::set(self.kind, i, &mut get_data_mut().menu_resources, value);
                    self.dirty_at = Some(s.t);
                }
                self.scroll.y_scroller.halt();
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        s.render_fader(ui, |ui| {
            let mut r = ui.content_rect().feather(-0.025);
            if self.kind == Kind::Character {
                custom_resources::draw_character_preview(ui);
                // Right-side controls leave the home-page illustration visible on the left.
                r.x = 0.15;
                r.w = 0.82;
                ui.fill_rect(r, Color::from_rgba(20, 28, 38, 210));
            }
            self.scroll.size((r.w, r.h));
            ui.scope(|ui| {
                ui.dx(r.x);
                ui.dy(r.y);
                self.scroll.render(ui, |ui| {
                    let w = r.w;
                    let name = self.kind.name(&get_data().menu_resources);
                    ui.text(if name.is_empty() { "默认资源" } else { name })
                        .size(0.48)
                        .max_width(w - 0.04)
                        .draw();
                    ui.dy(0.065);
                    ui.text(self.kind.formats())
                        .size(0.35)
                        .max_width(w - 0.04)
                        .draw();
                    ui.dy(0.08);
                    self.import.render_text(
                        ui,
                        Rect::new(0.0, 0.0, 0.32, 0.075),
                        t,
                        if custom_resources::busy() {
                            "正在加载…"
                        } else {
                            "选择文件"
                        },
                        0.45,
                        false
                    );
                    self.reset.render_text(
                        ui,
                        Rect::new(0.36, 0.0, 0.32, 0.075),
                        t,
                        "恢复默认",
                        0.45,
                        false
                    );
                    ui.dy(0.11);
                    let specs = Self::specs(self.kind);
                    for (i, (label, min, max, _)) in specs.iter().copied().enumerate() {
                        let blur_row = self.kind == Kind::Background && i == 4;
                        let two_lines = self.kind == Kind::Character || blur_row;
                        ui.fill_path(
                            &Rect::new(0.0, 0.0, w, 0.138).rounded(0.012),
                            Color::from_rgba(30, 49, 62, 180)
                        );
                        let title = ui
                            .text(label)
                            .pos(0.025, if two_lines { 0.018 } else { 0.05 })
                            .size(0.45)
                            .draw();
                        if blur_row {
                            ui.text("（调整之后需要稍等几秒）")
                                .pos(title.right() + 0.01, 0.021)
                                .size(0.33)
                                .draw();
                        }
                        let value = Self::value(self.kind, i, &get_data().menu_resources);
                        self.sliders[i].render(
                            ui,
                            Rect::new(
                                w - 0.54,
                                if two_lines {
                                    0.072
                                } else {
                                    0.046
                                },
                                if self.kind == Kind::Character {
                                    0.2
                                } else {
                                    0.28
                                },
                                0.05
                            ),
                            t,
                            value,
                            String::new()
                        );
                        ui.scope(|ui| {
                            ui.dx(w - 0.22);
                            ui.dy(if two_lines { 0.072 } else { 0.046 });
                            let mut text = format!("{value:.1}");
                            let mut changed = false;
                            let id = format!("menu_resource_{:?}_{i}", self.kind);
                            if
                                ui.button_with_size(
                                    &id,
                                    Rect::new(0.0, 0.0, 0.18, 0.065),
                                    &text,
                                    0.4
                                )
                            {
                                prpr::scene::request_input(
                                    &id,
                                    inputbox::InputBox::new().default_text(text.as_str())
                                );
                            }
                            if let Some((input_id, input)) = prpr::scene::take_input() {
                                if input_id == id {
                                    text = input;
                                    changed = true;
                                } else {
                                    prpr::scene::return_input(input_id, input);
                                }
                            }
                            if changed {
                                if let Ok(v) = text.trim().parse::<f32>() {
                                    if v.is_finite() {
                                        Self::set(
                                            self.kind,
                                            i,
                                            &mut get_data_mut().menu_resources,
                                            v.clamp(min, max)
                                        );
                                        self.dirty_at = Some(t);
                                    }
                                }
                            }
                        });
                        ui.dy(0.15);
                    }
                    if self.kind == Kind::Background {
                        ui.fill_path(
                            &Rect::new(0.0, 0.0, w, 0.138).rounded(0.012),
                            Color::from_rgba(30, 49, 62, 180)
                        );
                        ui.text("背景条纹").pos(0.025, 0.05).size(0.45).draw();
                        let on = get_data().menu_resources.stripes;
                        if
                            ui.button(
                                "menu_resource_stripes",
                                Rect::new(w - 0.28, 0.034, 0.24, 0.07),
                                if on {
                                    "开启"
                                } else {
                                    "关闭"
                                }
                            )
                        {
                            get_data_mut().menu_resources.stripes ^= true;
                            self.dirty_at = Some(t);
                        }
                        ui.dy(0.15);
                    }
                    let rows = specs.len() + usize::from(self.kind == Kind::Background);
                    (w, 0.255 + (rows as f32) * 0.15)
                });
            });
            Ok(())
        })
    }
    fn next_page(&mut self) -> NextPage {
        NextPage::None
    }
}
