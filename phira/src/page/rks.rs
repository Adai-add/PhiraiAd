//! RKS editor hosted by the existing library page. Drawers overlay the original
//! grid, and all configuration is isolated in custom-rks.json.
use crate::{custom_rks::*, dir, get_data, page::SharedState};
use anyhow::{anyhow, Result};
use inputbox::InputBox;
use macroquad::prelude::*;
use prpr::{
    ext::RectExt,
    scene::request_input,
    ui::{button_hit, DRectButton, RectButton, Ui},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub enum RksAction {
    Toggle(String),
    Edit(String),
}
#[derive(Clone, Debug)]
enum Target {
    Local(String),
    Extra(u64),
}
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Local,
    Extra,
    Rules,
}

pub struct RksPanel {
    pub editing: bool,
    pub resolved: Arc<HashMap<String, Resolved>>,
    settings: Settings,
    path: PathBuf,
    locals: Vec<LocalInput>,
    summary: Summary,
    tab: Tab,
    target: Option<Target>,
    extra_page: usize,
    buttons: HashMap<String, DRectButton>,
    active: Vec<String>,
    block: RectButton,
    block_active: bool,
    pending: Option<(String, Option<Target>)>,
    input_serial: u64,
    ai_revision: u64,
    ai_poll: std::time::Instant,
}
impl RksPanel {
    pub fn new() -> Result<Self> {
        let path = PathBuf::from(dir::root()?).join("custom-rks.json");
        let settings = Settings::load(&path)?;
        Ok(Self {
            editing: false,
            resolved: Arc::default(),
            settings,
            path,
            locals: Vec::new(),
            summary: Summary::default(),
            tab: Tab::Local,
            target: None,
            extra_page: 0,
            buttons: HashMap::new(),
            active: Vec::new(),
            block: RectButton::new(),
            block_active: false,
            pending: None,
            input_serial: 0,
            ai_revision: 0,
            ai_poll: std::time::Instant::now(),
        })
    }
    pub fn poll_ai(&mut self) {
        if self.ai_poll.elapsed() < std::time::Duration::from_millis(250) {
            return;
        }
        self.ai_poll = std::time::Instant::now();
        let revision = crate::ai_service::revision();
        if revision == self.ai_revision {
            return;
        }
        self.ai_revision = revision;
        for input in &mut self.locals {
            input.ai_difficulty = crate::ai_service::value(&input.path);
        }
        self.recalculate();
    }
    fn sync_ai_queue(&self) {
        crate::ai_service::catalog(self.locals.iter().map(|c| c.path.clone()));
        crate::ai_service::priorities(
            self.settings
                .charts
                .iter()
                .filter(|(_, c)| c.difficulty_source == DifficultySource::Ai)
                .map(|(p, _)| p.clone()),
        );
    }
    pub fn local_editing(&self) -> bool {
        self.editing && self.tab == Tab::Local
    }
    pub fn refresh(&mut self, state: &SharedState) {
        let data = get_data();
        let records: HashMap<_, _> = data.charts.iter().map(|c| (c.local_path.as_str(), c.record.as_ref())).collect();
        self.locals = state
            .charts_local
            .iter()
            .filter_map(|c| {
                let path = c.local_path.as_ref()?;
                let ordinary = crate::custom_rks::highest_record_accuracy(
                    records.get(path.as_str()).copied().flatten().map(|r| r.accuracy as f64 * 100.),
                    data.local_records.get(path).and_then(|r| r.as_ref()).map(|r| r.accuracy as f64 * 100.),
                );
                Some(LocalInput {
                    path: path.clone(),
                    name: c.info.name.clone(),
                    level: c.info.level.clone(),
                    difficulty: c.info.difficulty as f64,
                    ai_difficulty: crate::ai_service::value(path),
                    accuracy: crate::custom_rks::highest_record_accuracy(
                        ordinary,
                        c.info
                            .id
                            .and_then(|id| data.replica_local_records.get(&id))
                            .map(|r| r.accuracy as f64 * 100.),
                    ),
                })
            })
            .collect();
        self.sync_ai_queue();
        self.recalculate();
    }
    fn recalculate(&mut self) {
        let (resolved, summary) = calculate(&self.settings, &self.locals);
        self.resolved = Arc::new(resolved);
        self.summary = summary;
    }
    fn commit(&mut self, next: Settings) -> Result<()> {
        next.save(&self.path)?;
        self.settings = next;
        self.sync_ai_queue();
        self.recalculate();
        Ok(())
    }
    pub fn exit(&mut self) {
        self.editing = false;
        self.target = None;
        self.tab = Tab::Local;
        self.cancel_buttons();
    }
    pub fn back(&mut self) -> bool {
        if !self.editing {
            return false;
        }
        if self.target.take().is_none() {
            self.exit();
        } else {
            self.cancel_buttons();
        }
        true
    }
    fn cancel_buttons(&mut self) {
        for b in self.buttons.values_mut() {
            b.inner.cancel();
        }
        self.active.clear();
    }
    pub fn action(&mut self, action: RksAction) -> Result<()> {
        match action {
            RksAction::Toggle(path) => {
                if !self.locals.iter().any(|c| c.path == path) {
                    return Ok(());
                }
                let mut next = self.settings.clone();
                let chart = next.charts.entry(path).or_default();
                chart.included = !chart.included;
                self.commit(next)?;
            }
            RksAction::Edit(path) => {
                self.target = Some(Target::Local(path));
                self.cancel_buttons();
            }
        }
        Ok(())
    }
    fn edit_input(&mut self, field: &str, value: String) {
        self.input_serial += 1;
        let id = format!("rks:{}:{field}", self.input_serial);
        self.pending = Some((id.clone(), self.target.clone()));
        request_input(&id, InputBox::new().default_text(value));
    }
    pub fn input(&mut self, id: &str, text: String) -> Result<()> {
        let Some((expected, target)) = self.pending.take() else {
            return Ok(());
        };
        if id != expected {
            self.pending = Some((expected, target));
            return Ok(());
        }
        let field = id.rsplit(':').next().unwrap_or("");
        let text = text.trim();
        let mut next = self.settings.clone();
        if field == "name" {
            if text.is_empty() || text.chars().count() > 150 {
                return Err(anyhow!("歌曲名需为 1～150 个字符"));
            }
            if let Some(Target::Extra(id)) = target {
                if let Some(e) = next.extras.iter_mut().find(|e| e.id == id) {
                    e.name = text.to_owned();
                }
            }
        } else if field == "best" || field == "ap" {
            let n: usize = text.parse().map_err(|_| anyhow!("请输入 0～1000 的整数"))?;
            if n > 1000 {
                return Err(anyhow!("数量范围为 0～1000"));
            }
            if field == "best" {
                next.best_count = n;
            } else {
                next.ap_count = n;
            }
            if next.best_count + next.ap_count == 0 {
                return Err(anyhow!("两种成绩数量不能同时为零"));
            }
        } else {
            let raw: f64 = text.parse().map_err(|_| anyhow!("请输入有效数字"))?;
            let value = if field == "difficulty" {
                difficulty_value(raw).ok_or_else(|| anyhow!("定数必须是有效的非负数"))?
            } else {
                let a = accuracy_value(raw).ok_or_else(|| anyhow!("准确率范围为 0～100%"))?;
                (a * 100.).round() / 100.
            };
            match target {
                Some(Target::Local(path)) => {
                    let c = next.charts.entry(path).or_default();
                    if field == "difficulty" {
                        c.custom_difficulty = Some(value);
                        c.difficulty_source = DifficultySource::Custom;
                    } else {
                        c.custom_accuracy = Some(value);
                        c.accuracy_source = AccuracySource::Custom;
                    }
                }
                Some(Target::Extra(id)) => {
                    if let Some(e) = next.extras.iter_mut().find(|e| e.id == id) {
                        if field == "difficulty" {
                            e.difficulty = value;
                        } else {
                            e.accuracy = value;
                        }
                    }
                }
                None => return Ok(()),
            }
        }
        self.commit(next)
    }
    fn command(&mut self, id: &str) -> Result<()> {
        button_hit();
        match id {
            "entry" => {
                self.editing = true;
                self.tab = Tab::Local;
            }
            "done" => self.exit(),
            "local" => {
                self.tab = Tab::Local;
                self.target = None;
            }
            "extras" => {
                self.tab = Tab::Extra;
                self.target = None;
            }
            "rules" => {
                self.tab = Tab::Rules;
                self.target = None;
            }
            "search" => request_input("search", InputBox::new()),
            "select-all" | "clear-all" => {
                let mut next = self.settings.clone();
                let included = id == "select-all";
                match self.tab {
                    Tab::Local => {
                        for chart in &self.locals {
                            next.charts.entry(chart.path.clone()).or_default().included = included;
                        }
                    }
                    Tab::Extra => {
                        for entry in &mut next.extras {
                            entry.included = included;
                        }
                    }
                    Tab::Rules => return Ok(()),
                }
                self.commit(next)?;
            }
            "close" => self.target = None,
            "prev" => self.extra_page = self.extra_page.saturating_sub(1),
            "next" => self.extra_page = (self.extra_page + 1).min(self.settings.extras.len().saturating_sub(1) / 5),
            "add" => {
                let mut next = self.settings.clone();
                let id = next.next_id();
                next.extras.push(ExtraEntry {
                    id,
                    name: "自定义歌曲".into(),
                    difficulty: 0.,
                    accuracy: 100.,
                    included: true,
                });
                self.commit(next)?;
                self.extra_page = self.settings.extras.len().saturating_sub(1) / 5;
                self.target = Some(Target::Extra(id));
            }
            "best" => self.edit_input("best", self.settings.best_count.to_string()),
            "ap" => self.edit_input("ap", self.settings.ap_count.to_string()),
            "ai" | "file" | "custom-d" | "record" | "custom-a" | "difficulty" | "accuracy" | "name" | "included" | "delete" => {
                let mut next = self.settings.clone();
                match self.target.clone() {
                    Some(Target::Local(path)) => {
                        let Some(input) = self.locals.iter().find(|i| i.path == path).cloned() else {
                            self.target = None;
                            return Ok(());
                        };
                        let c = next.charts.entry(path).or_default();
                        match id {
                            "ai" => c.difficulty_source = DifficultySource::Ai,
                            "file" => c.difficulty_source = DifficultySource::File,
                            "custom-d" => {
                                c.difficulty_source = DifficultySource::Custom;
                                if c.custom_difficulty.is_none() {
                                    c.custom_difficulty = difficulty_value(input.difficulty);
                                }
                            }
                            "record" => c.accuracy_source = AccuracySource::Record,
                            "custom-a" => {
                                c.accuracy_source = AccuracySource::Custom;
                                if c.custom_accuracy.is_none() {
                                    c.custom_accuracy = input.accuracy.map(|a| (a * 100.).round() / 100.);
                                }
                            }
                            "included" => c.included = !c.included,
                            "difficulty" => {
                                self.edit_input("difficulty", c.custom_difficulty.map(format_difficulty).unwrap_or_default());
                                return Ok(());
                            }
                            "accuracy" => {
                                self.edit_input("accuracy", c.custom_accuracy.map(|a| format!("{a:.2}")).unwrap_or_default());
                                return Ok(());
                            }
                            _ => return Ok(()),
                        }
                    }
                    Some(Target::Extra(key)) => {
                        let Some(e) = next.extras.iter_mut().find(|e| e.id == key) else {
                            return Ok(());
                        };
                        match id {
                            "included" => e.included = !e.included,
                            "name" => {
                                self.edit_input("name", e.name.clone());
                                return Ok(());
                            }
                            "difficulty" => {
                                self.edit_input("difficulty", format_difficulty(e.difficulty));
                                return Ok(());
                            }
                            "accuracy" => {
                                self.edit_input("accuracy", format!("{:.2}", e.accuracy));
                                return Ok(());
                            }
                            "delete" => {
                                next.extras.retain(|e| e.id != key);
                                self.target = None;
                            }
                            _ => return Ok(()),
                        }
                    }
                    None => return Ok(()),
                }
                self.commit(next)?;
            }
            _ => {
                if let Some(n) = id.strip_prefix("extra-").and_then(|s| s.parse::<u64>().ok()) {
                    self.target = Some(Target::Extra(n));
                }
            }
        }
        self.cancel_buttons();
        Ok(())
    }
    pub fn touch(&mut self, touch: &Touch, _t: f32) -> Result<bool> {
        let mut clicked = None;
        let mut captured = false;
        for id in &self.active {
            let b = self.buttons.get_mut(id).unwrap();
            captured |= b.inner.contains(touch.position) || b.inner.touching();
            if b.touch(touch, _t) {
                clicked = Some(id.clone());
            }
        }
        if let Some(id) = clicked {
            self.command(&id)?;
            return Ok(true);
        }
        Ok(captured || (self.editing && self.block_active && self.block.contains(touch.position)))
    }
    pub fn begin_render(&mut self) {
        self.active.clear();
        self.block_active = false;
    }
    fn button(&mut self, ui: &mut Ui, t: f32, id: &str, text: impl Into<String>, r: Rect, selected: bool) {
        self.active.push(id.to_owned());
        self.buttons
            .entry(id.to_owned())
            .or_insert_with(DRectButton::new)
            .render_text(ui, r, t, text.into(), 0.40, selected);
    }
    pub fn render_entry(&mut self, ui: &mut Ui, t: f32, r: Rect) {
        self.button(ui, t, "entry", format!("RKS: {:.2}", self.summary.rks), r, false);
    }
    pub fn render_header(&mut self, ui: &mut Ui, t: f32) {
        let y = -ui.top + 0.04;
        let h = ui.content_rect().y + ui.top - 0.06;
        let mut x = -0.55;
        for (id, text, width, selected) in [
            ("local", "本地谱面".to_owned(), 0.19, self.tab == Tab::Local),
            ("extras", "额外条目".into(), 0.19, self.tab == Tab::Extra),
            ("rules", "计算规则".into(), 0.19, self.tab == Tab::Rules),
            ("result", format!("RKS: {:.2}", self.summary.rks), 0.26, false),
            ("done", "完成".into(), 0.12, false),
            ("search", "搜索".into(), 0.12, false),
        ] {
            self.button(ui, t, id, text, Rect::new(x, y, width, h), selected);
            x += width + 0.015;
        }
        if self.tab != Tab::Rules {
            self.button(ui, t, "select-all", "全选", Rect::new(x, y, 0.14, h), false);
            self.button(ui, t, "clear-all", "全取消", Rect::new(x + 0.155, y, 0.16, h), false);
        }
    }
    pub fn render_overlay(&mut self, ui: &mut Ui, t: f32) {
        if !self.editing {
            return;
        }
        let area = ui.content_rect();
        let panel_w = 0.67;
        if self.tab != Tab::Local {
            ui.fill_path(&area.rounded(0.008), Color::from_rgba(36, 46, 60, 250));
            self.block.set(ui, area);
            self.block_active = true;
        }
        if self.tab == Tab::Rules {
            let x = area.x + 0.045;
            let w = area.w - 0.09;
            let y = area.y + 0.035;
            ui.text("自定义 RKS · 汇总规则").pos(x, y).size(0.55).draw();
            self.button(ui, t, "best", format!("最佳成绩数量：{}", self.settings.best_count), Rect::new(x, y + 0.07, w.min(0.65), 0.065), false);
            self.button(ui, t, "ap", format!("AP 成绩数量：{}", self.settings.ap_count), Rect::new(x, y + 0.15, w.min(0.65), 0.065), false);
            for (i, line) in [
                "单曲：准确率低于 70% 为 0",
                "否则：定数 × ((准确率 − 55) / 45)²",
                "两组独立取最高值，AP 可以同时进入最佳成绩",
                "两组求和 ÷ 总数量；缺少的位置按 0 计",
                "AP 必须实际达到 100%，不按显示值判断",
            ]
            .iter()
            .enumerate()
            {
                ui.text(*line).pos(x, y + 0.25 + i as f32 * 0.047).size(0.40).max_width(w).draw();
            }
            return;
        }
        if self.tab == Tab::Extra {
            let x = area.x + 0.025;
            let y = area.y + 0.025;
            let w = if self.target.is_some() { area.w - panel_w - 0.06 } else { area.w - 0.05 };
            self.button(ui, t, "add", "＋ 添加条目", Rect::new(x, y, w, 0.06), false);
            self.extra_page = self.extra_page.min(self.settings.extras.len().saturating_sub(1) / 5);
            let start = self.extra_page * 5;
            let rows: Vec<_> = self.settings.extras.iter().skip(start).take(5).cloned().collect();
            for (i, e) in rows.iter().enumerate() {
                self.button(
                    ui,
                    t,
                    &format!("extra-{}", e.id),
                    format!("{} {} · {} · {:.2}%", if e.included { "✓" } else { "○" }, e.name, format_difficulty(e.difficulty), e.accuracy),
                    Rect::new(x, y + 0.078 + i as f32 * 0.082, w, 0.067),
                    matches!(self.target,Some(Target::Extra(id)) if id==e.id),
                );
            }
            let py = area.bottom() - 0.078;
            self.button(ui, t, "prev", "上一页", Rect::new(x, py, w * 0.35, 0.055), false);
            self.button(ui, t, "next", "下一页", Rect::new(x + w * 0.65, py, w * 0.35, 0.055), false);
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        let r = Rect::new(area.right() - panel_w, area.y, panel_w, area.h);
        ui.fill_path(&r.rounded(0.008), Color::from_rgba(30, 42, 55, 255));
        if self.tab == Tab::Local {
            self.block.set(ui, r);
            self.block_active = true;
        }
        let x = r.x + 0.025;
        let w = r.w - 0.05;
        let mut y = r.y + 0.025;
        self.button(ui, t, "close", "×", Rect::new(r.right() - 0.065, y, 0.045, 0.045), false);
        match target {
            Target::Local(path) => {
                let Some(input) = self.locals.iter().find(|i| i.path == path).cloned() else {
                    self.target = None;
                    return;
                };
                let c = self.settings.chart(&path);
                let resolved = resolve(&input, &c);
                ui.text(&input.name).pos(x, y).size(0.46).max_width(w - 0.06).draw();
                y += 0.055;
                self.button(ui, t, "included", if c.included { "✓ 已计入" } else { "○ 未计入" }, Rect::new(x, y, w, 0.052), c.included);
                y += 0.072;
                ui.text("定数来源").pos(x, y).size(0.38).draw();
                y += 0.035;
                let cw = (w - 0.014) / 3.;
                self.button(ui, t, "file", "跟随文件", Rect::new(x, y, cw, 0.05), c.difficulty_source == DifficultySource::File);
                self.button(ui, t, "custom-d", "自定义", Rect::new(x + cw + 0.007, y, cw, 0.05), c.difficulty_source == DifficultySource::Custom);
                self.button(ui, t, "ai", "AI 计算", Rect::new(x + 2. * (cw + 0.007), y, cw, 0.05), c.difficulty_source == DifficultySource::Ai);
                y += 0.065;
                if c.difficulty_source == DifficultySource::Custom {
                    self.button(
                        ui,
                        t,
                        "difficulty",
                        format!("定数：{}", c.custom_difficulty.map(format_difficulty).unwrap_or_else(|| "点击填写".into())),
                        Rect::new(x, y, w, 0.052),
                        false,
                    );
                } else {
                    ui.text(format!("定数：{}", resolved.difficulty.map(format_difficulty).unwrap_or_else(|| "待补全".into())))
                        .pos(x, y + 0.012)
                        .size(0.42)
                        .draw();
                }
                y += 0.075;
                if c.difficulty_source == DifficultySource::Ai {
                    ui.text(crate::ai_service::status(&path)).pos(x, y).size(0.30).max_width(w).draw();
                    y += 0.04;
                }
                ui.text("准确率来源").pos(x, y).size(0.38).draw();
                y += 0.035;
                self.button(ui, t, "record", "跟随记录", Rect::new(x, y, w / 2. - 0.004, 0.05), c.accuracy_source == AccuracySource::Record);
                self.button(
                    ui,
                    t,
                    "custom-a",
                    "自定义",
                    Rect::new(x + w / 2. + 0.004, y, w / 2. - 0.004, 0.05),
                    c.accuracy_source == AccuracySource::Custom,
                );
                y += 0.065;
                if c.accuracy_source == AccuracySource::Custom {
                    self.button(
                        ui,
                        t,
                        "accuracy",
                        format!("准确率：{}", c.custom_accuracy.map(|a| format!("{a:.2}%")).unwrap_or_else(|| "点击填写".into())),
                        Rect::new(x, y, w, 0.052),
                        false,
                    );
                } else {
                    ui.text(format!("准确率：{}", resolved.accuracy.map(|a| format!("{a:.2}%")).unwrap_or_else(|| "暂无记录".into())))
                        .pos(x, y + 0.012)
                        .size(0.42)
                        .draw();
                }
                y += 0.075;
                ui.text(
                    resolved
                        .single_rks
                        .map(|v| format!("单曲 RKS：{v:.2}"))
                        .unwrap_or_else(|| "数据不全，暂不计入结果".into()),
                )
                .pos(x, y)
                .size(0.40)
                .max_width(w)
                .draw();
            }
            Target::Extra(id) => {
                let Some(e) = self.settings.extras.iter().find(|e| e.id == id).cloned() else {
                    self.target = None;
                    return;
                };
                ui.text("额外条目").pos(x, y).size(0.46).draw();
                y += 0.065;
                for (key, label) in [
                    ("included", if e.included { "✓ 已计入".into() } else { "○ 未计入".into() }),
                    ("name", format!("歌曲名：{}", e.name)),
                    ("difficulty", format!("定数：{}", format_difficulty(e.difficulty))),
                    ("accuracy", format!("准确率：{:.2}%", e.accuracy)),
                    ("delete", "删除此条目".into()),
                ] {
                    self.button(ui, t, key, label, Rect::new(x, y, w, 0.06), key == "included" && e.included);
                    y += 0.083;
                }
                ui.text("仅用于 RKS，不生成谱面").pos(x, y).size(0.38).max_width(w).draw();
            }
        }
    }
}
