use super::{NextPage, Page, SharedState};
use crate::{get_data, get_data_mut, save_data};
use anyhow::Result;
use inputbox::InputBox;
use macroquad::prelude::*;
use prpr::{
    custom_judgement::{CustomJudgementConfig, MAX_SCHEMES},
    scene::{request_input, return_input, take_input},
    ui::{Scroll, Ui},
};
use std::borrow::Cow;

pub struct CustomJudgementEditor {
    draft: CustomJudgementConfig,
    scroll: Scroll,
    schemes_scroll: Scroll,
    done: bool,
    error: String,
}
impl CustomJudgementEditor {
    pub fn new() -> Self {
        Self {
            draft: get_data().config.custom_judgement.clone(),
            scroll: Scroll::new(),
            schemes_scroll: Scroll::new(),
            done: false,
            error: String::new(),
        }
    }
    fn field(ui: &mut Ui, id: &str, rect: Rect, text: &mut String) -> bool {
        if ui.button(id, rect, text.clone()) {
            request_input(id, InputBox::new().default_text(text.as_str()));
        }
        if let Some((key, value)) = take_input() {
            if key == id {
                *text = value;
                return true;
            } else {
                return_input(key, value);
            }
        }
        false
    }
    fn persist(&mut self) -> bool {
        if let Err(e) = self.draft.current.validate() {
            self.error = e;
            return false;
        }
        let old = get_data().config.custom_judgement.clone();
        get_data_mut().config.custom_judgement = self.draft.clone();
        if let Err(e) = save_data() {
            get_data_mut().config.custom_judgement = old;
            self.error = format!("保存失败：{e}");
            return false;
        }
        self.error.clear();
        true
    }
}
impl Page for CustomJudgementEditor {
    fn label(&self) -> Cow<'static, str> {
        "自定义判定".into()
    }
    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        self.scroll.update(s.t);
        self.schemes_scroll.update(s.t);
        Ok(())
    }
    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        Ok(self.scroll.touch(touch, s.t) || self.schemes_scroll.touch(touch, s.t))
    }
    fn render(&mut self, ui: &mut Ui, _s: &mut SharedState) -> Result<()> {
        ui.abs_scope(|ui| {
            let top = ui.top;
            ui.fill_rect(Rect::new(-1., -top, 2., top * 2.), Color::from_rgba(19, 29, 39, 255));
            ui.text("自定义判定").pos(-0.94, -top + 0.025).size(0.6).draw();
            if ui.button("custom_apply", Rect::new(0.66, -top + 0.025, 0.28, 0.065), "应用并返回") && self.persist() {
                self.done = true;
            }
            let mut name = self.draft.current.name.clone();
            ui.text("方案名").pos(-0.94, -top + 0.115).size(0.36).draw();
            if Self::field(ui, "custom_name", Rect::new(-0.79, -top + 0.105, 0.35, 0.065), &mut name) {
                if name.trim().is_empty() {
                    self.error = "方案名不能为空".into();
                } else {
                    self.draft.current.name = name.trim().to_owned();
                }
            }
            for (i, n) in [5, 7, 9].iter().enumerate() {
                if ui.button(
                    &format!("custom_count_{n}"),
                    Rect::new(-0.40 + i as f32 * 0.14, -top + 0.105, 0.12, 0.065),
                    format!("{n}档{}", if self.draft.current.bands.len() == *n { " ✓" } else { "" }),
                ) {
                    self.draft.current.change_count(*n);
                    self.scroll.y_scroller.reset();
                    self.error.clear();
                }
            }
            if ui.button("custom_save_new", Rect::new(0.08, -top + 0.105, 0.26, 0.065), "保存新方案") {
                match self.draft.save(true) {
                    Ok(()) => {
                        self.persist();
                    }
                    Err(e) => self.error = e,
                }
            }
            if ui.button("custom_save_existing", Rect::new(0.37, -top + 0.105, 0.27, 0.065), "覆盖/重命名") {
                match self.draft.save(false) {
                    Ok(()) => {
                        self.persist();
                    }
                    Err(e) => self.error = e,
                }
            }
            if ui.button("custom_new_draft", Rect::new(0.67, -top + 0.105, 0.27, 0.065), "新建") {
                self.draft.current = Default::default();
                self.draft.selected = None;
                self.scroll.y_scroller.reset();
                self.error.clear();
            }
            let y = -top + 0.205;
            let height = (2. * top - 0.30).max(0.1);
            ui.scope(|ui| {
                ui.dx(-0.94);
                ui.dy(y);
                ui.text(format!("方案 {}/{}", self.draft.schemes.len(), MAX_SCHEMES)).size(0.34).draw();
                ui.dy(0.055);
                self.schemes_scroll.size((0.40, height - 0.055));
                let mut selected = None;
                let mut deleted = None;
                self.schemes_scroll.render(ui, |ui| {
                    for (i, scheme) in self.draft.schemes.iter().enumerate() {
                        let label = format!("{}{}", if self.draft.selected == Some(i) { "✓ " } else { "" }, scheme.name);
                        // Button text is ellipsized by the existing renderer.
                        if ui.button(&format!("custom_select_{i}"), Rect::new(0., 0., 0.30, 0.06), label) {
                            selected = Some(i);
                        }
                        if ui.button(&format!("custom_delete_{i}"), Rect::new(0.32, 0., 0.065, 0.06), "×") {
                            deleted = Some(i);
                        }
                        ui.dy(0.075);
                    }
                    (0.40, self.draft.schemes.len() as f32 * 0.075)
                });
                if let Some(i) = selected {
                    self.draft.select(i);
                    self.scroll.y_scroller.reset();
                    self.error.clear();
                }
                if let Some(i) = deleted {
                    self.draft.delete(i);
                    self.persist();
                }
            });
            ui.scope(|ui| {
                ui.dx(-0.50);
                ui.dy(y);
                ui.text("边界(ms)       判定档位                 ACC贡献 / 打击特效").size(0.32).draw();
                ui.dy(0.055);
                self.scroll.size((1.44, height - 0.055));
                let draft = &mut self.draft;
                let error = &mut self.error;
                self.scroll.render(ui, |ui| {
                    let count = draft.current.bands.len();
                    for i in 0..count {
                        let band = &mut draft.current.bands[i];
                        ui.fill_rect(Rect::new(0., 0., 1.43, if i == 0 || i == count - 1 { 0.10 } else { 0.20 }), Color::from_rgba(30, 49, 62, 180));
                        let mut boundary = format!("{}", draft.current.boundaries_ms[i]);
                        if Self::field(ui, &format!("custom_boundary_{i}"), Rect::new(0.02, 0.02, 0.24, 0.06), &mut boundary) {
                            match boundary.trim().parse::<f64>() {
                                Ok(v) if v.is_finite() => {
                                    draft.current.boundaries_ms[i] = v;
                                    error.clear();
                                }
                                _ => *error = "请输入有效的毫秒数".into(),
                            }
                        }
                        ui.text(&band.label)
                            .pos(0.30, 0.032)
                            .size(0.38)
                            .color(Color::from_rgba(band.color[0], band.color[1], band.color[2], 255))
                            .draw();
                        let mut contribution = format!("{}", band.contribution);
                        if Self::field(ui, &format!("custom_acc_{i}"), Rect::new(0.86, 0.02, 0.20, 0.06), &mut contribution) {
                            match contribution.trim().parse::<f64>() {
                                Ok(v) if v.is_finite() && v >= 0. => {
                                    band.contribution = v;
                                    error.clear();
                                }
                                _ => *error = "请输入非负的ACC贡献值".into(),
                            }
                        }
                        if i != 0 && i != count - 1 {
                            let mut color = format!("#{:02X}{:02X}{:02X}{:02X}", band.color[0], band.color[1], band.color[2], band.color[3]);
                            ui.text("颜色RGBA").pos(0.02, 0.122).size(0.30).draw();
                            if Self::field(ui, &format!("custom_color_{i}"), Rect::new(0.24, 0.11, 0.29, 0.06), &mut color) {
                                let code = color.trim().trim_start_matches('#');
                                match u32::from_str_radix(code, 16) {
                                    Ok(v) if code.len() == 6 || code.len() == 8 => {
                                        let v = if code.len() == 6 { (v << 8) | 255 } else { v };
                                        band.color = v.to_be_bytes();
                                        error.clear();
                                    }
                                    _ => *error = "颜色请输入6位RGB或8位RGBA十六进制".into(),
                                }
                            }
                            ui.fill_rect(
                                Rect::new(0.56, 0.12, 0.045, 0.04),
                                Color::from_rgba(band.color[0], band.color[1], band.color[2], band.color[3]),
                            );
                            ui.text("大小×").pos(0.65, 0.122).size(0.30).draw();
                            let mut size = format!("{}", band.effect_size);
                            if Self::field(ui, &format!("custom_effect_size_{i}"), Rect::new(0.80, 0.11, 0.18, 0.06), &mut size) {
                                match size.trim().parse::<f32>() {
                                    Ok(v) if v.is_finite() && v > 0. => {
                                        band.effect_size = v;
                                        error.clear();
                                    }
                                    _ => *error = "特效大小必须大于0".into(),
                                }
                            }
                            if ui.button(
                                &format!("custom_effect_enabled_{i}"),
                                Rect::new(1.05, 0.11, 0.35, 0.06),
                                if band.effect_enabled { "特效：开启" } else { "特效：关闭" },
                            ) {
                                band.effect_enabled ^= true;
                            }
                        } else {
                            ui.text("无打击特效").pos(1.09, 0.038).size(0.30).draw();
                        }
                        ui.dy(if i == 0 || i == count - 1 { 0.115 } else { 0.215 });
                    }
                    (1.44, 0.23 + (count - 2) as f32 * 0.215)
                });
            });
            let validation = self.draft.current.validate().err();
            ui.text(if !self.error.is_empty() {
                self.error.as_str()
            } else {
                validation.as_deref().unwrap_or("提前为正，延后为负；边界严格递减。ACC = 总贡献 ÷ 音符数")
            })
            .pos(-0.94, top - 0.07)
            .size(0.30)
            .max_width(1.88)
            .color(if !self.error.is_empty() || validation.is_some() {
                Color::from_rgba(255, 165, 165, 255)
            } else {
                WHITE
            })
            .draw();
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
