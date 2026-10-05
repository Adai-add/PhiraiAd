use super::{Page, SFader, SharedState};
use anyhow::Result;
use inputbox::InputBox;
use macroquad::prelude::*;
use prpr::{
    ext::{poll_future, LocalTask},
    replay::{self, ChartRecord, Manifest},
    scene::{request_input, return_input, show_error, take_input, NextScene, ReplayScene},
    ui::{Dialog, Scroll, Ui},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

enum DeleteRequest {
    Replays(Vec<String>),
    Charts(Vec<u64>),
}
pub struct ReplayLibraryPage {
    entries: Vec<Manifest>,
    charts: Vec<ChartRecord>,
    chart_tab: bool,
    chart_filter: Option<u64>,
    chart_counts: BTreeMap<u64, usize>,
    revision: u64,
    usage: replay::StorageUsage,
    scroll: Scroll,
    selected: BTreeSet<String>,
    multi: bool,
    confirmed: Arc<Mutex<Option<DeleteRequest>>>,
    renaming: Option<String>,
    loading: LocalTask<Result<ReplayScene>>,
    sf: SFader,
}
impl ReplayLibraryPage {
    pub fn new() -> Result<Self> {
        let snapshot = replay::library_snapshot()?;
        let revision = snapshot.revision;
        let entries = snapshot.entries;
        let usage = snapshot.usage;
        let mut chart_counts = BTreeMap::new();
        for entry in &entries {
            *chart_counts.entry(entry.chart_id).or_default() += 1;
        }
        Ok(Self {
            revision,
            entries,
            charts: snapshot.charts,
            chart_tab: false,
            chart_filter: None,
            chart_counts,
            usage,
            scroll: Scroll::new(),
            selected: BTreeSet::new(),
            multi: false,
            confirmed: Arc::default(),
            renaming: None,
            loading: None,
            sf: SFader::new(),
        })
    }
    fn refresh(&mut self) -> Result<()> {
        let snapshot = replay::library_snapshot()?;
        self.revision = snapshot.revision;
        self.entries = snapshot.entries;
        self.charts = snapshot.charts;
        self.usage = snapshot.usage;
        self.chart_counts.clear();
        for entry in &self.entries {
            *self.chart_counts.entry(entry.chart_id).or_default() += 1;
        }
        Ok(())
    }
    fn confirm_delete(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let description = if ids.len() == 1 {
            format!("是否确认删除“{}”？", self.entries.iter().find(|e| e.id == ids[0]).map_or("", |e| e.name.as_str()))
        } else {
            format!("是否确认删除这 {} 个回放？", ids.len())
        };
        let confirmed = self.confirmed.clone();
        Dialog::plain("删除回放", description)
            .buttons(vec!["取消".into(), "删除".into()])
            .listener(move |_, button| {
                if button == 1 {
                    *confirmed.lock().unwrap() = Some(DeleteRequest::Replays(ids.clone()));
                }
                false
            })
            .show();
    }
    fn confirm_delete_charts(&self, ids: Vec<u64>) {
        if ids.is_empty() {
            return;
        }
        let count = self.entries.iter().filter(|e| ids.contains(&e.chart_id)).count();
        let name = if ids.len() == 1 {
            self.charts.iter().find(|r| r.id == ids[0]).map(|r| r.display_name()).unwrap_or_default()
        } else {
            format!("这 {} 份谱面记录", ids.len())
        };
        let confirmed = self.confirmed.clone();
        Dialog::plain("删除谱面记录", format!("是否确认删除“{name}”？这将一并删除所有对应录像，共 {count} 个。"))
            .buttons(vec!["取消".into(), "删除".into()])
            .listener(move |_, button| {
                if button == 1 {
                    *confirmed.lock().unwrap() = Some(DeleteRequest::Charts(ids.clone()));
                }
                false
            })
            .show();
    }
    fn change_tab(&mut self, chart_tab: bool) {
        self.chart_tab = chart_tab;
        self.chart_filter = None;
        self.multi = false;
        self.selected.clear();
        self.scroll = Scroll::new();
    }
}
impl Page for ReplayLibraryPage {
    fn label(&self) -> Cow<'static, str> {
        "回放库".into()
    }
    fn can_play_bgm(&self) -> bool {
        self.loading.is_none()
    }
    fn enter(&mut self, s: &mut SharedState) -> Result<()> {
        self.revision = replay::revision();
        self.refresh()?;
        self.sf.enter(s.t);
        Ok(())
    }
    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        replay::poll_saves();
        if self.revision != replay::revision() {
            self.revision = replay::revision();
            self.refresh()?;
        }
        let deleted = self.confirmed.lock().unwrap().take();
        if let Some(request) = deleted {
            let result = match request {
                DeleteRequest::Replays(ids) => replay::delete(&ids),
                DeleteRequest::Charts(ids) => replay::delete_charts(&ids),
            };
            match result {
                Ok(()) => {
                    self.refresh()?;
                    self.selected.clear();
                }
                Err(err) => show_error(err),
            }
        }
        if let Some((id, value)) = take_input() {
            if id == "replay_rename" {
                if let Some(replay_id) = self.renaming.take() {
                    match replay::rename(&replay_id, &value) {
                        Ok(()) => self.refresh()?,
                        Err(err) => show_error(err),
                    }
                }
            } else {
                return_input(id, value);
            }
        }
        if let Some(task) = &mut self.loading {
            if let Some(result) = poll_future(task.as_mut()) {
                self.loading = None;
                match result {
                    Ok(scene) => self.sf.goto(s.t, scene),
                    Err(err) => show_error(err),
                }
            }
        }
        Ok(())
    }
    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        if self.loading.is_some() || self.sf.transiting() {
            return Ok(true);
        }
        Ok(self.scroll.touch(touch, s.t))
    }
    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        s.render_fader(ui, |ui| {
            let top = ui.top;
            if ui.button("replay_tab", Rect::new(-0.94, -top + 0.095, 0.28, 0.065), if self.chart_tab { "录像" } else { "✓ 录像" }) {
                self.change_tab(false);
            }
            if ui.button("chart_record_tab", Rect::new(-0.64, -top + 0.095, 0.34, 0.065), if self.chart_tab { "✓ 谱面记录" } else { "谱面记录" })
            {
                self.change_tab(true);
            }
            ui.text(format!("总占用 {}", replay::format_size(self.usage.total_bytes)))
                .pos(-0.28, -top + 0.115)
                .size(0.30)
                .max_width(0.54)
                .draw();
            if ui.button("replay_multi", Rect::new(0.30, -top + 0.10, 0.25, 0.065), if self.multi { "取消多选" } else { "多选删除" }) {
                self.multi = !self.multi;
                self.selected.clear();
            }
            if self.multi {
                if ui.button("replay_all", Rect::new(0.57, -top + 0.10, 0.16, 0.065), "全选") {
                    self.selected = if self.chart_tab {
                        self.charts.iter().map(|e| e.id.to_string()).collect()
                    } else {
                        self.entries
                            .iter()
                            .filter(|e| self.chart_filter.is_none_or(|id| e.chart_id == id))
                            .map(|e| e.id.clone())
                            .collect()
                    };
                }
                if ui.button("replay_delete_selected", Rect::new(0.75, -top + 0.10, 0.20, 0.065), format!("删除({})", self.selected.len())) {
                    if self.chart_tab {
                        self.confirm_delete_charts(self.selected.iter().filter_map(|s| s.parse().ok()).collect());
                    } else {
                        self.confirm_delete(self.selected.iter().cloned().collect());
                    }
                }
            }
            let filtered: Vec<_> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| self.chart_filter.is_none_or(|id| e.chart_id == id))
                .map(|(i, _)| i)
                .collect();
            if let Some(id) = self.chart_filter {
                ui.text(format!("谱面记录 #{id} · {} 个录像", filtered.len()))
                    .pos(-0.94, -top + 0.185)
                    .size(0.30)
                    .draw();
                if ui.button("replay_clear_filter", Rect::new(0.72, -top + 0.175, 0.23, 0.06), "显示全部") {
                    self.chart_filter = None;
                    self.scroll = Scroll::new();
                }
            }
            let empty = if self.chart_tab { self.charts.is_empty() } else { filtered.is_empty() };
            if empty {
                ui.text(if self.chart_tab {
                    "还没有谱面记录"
                } else {
                    "还没有保存的录像"
                })
                .anchor(0.5, 0.5)
                .size(0.6)
                .draw();
                return;
            }
            let list_top = if self.chart_filter.is_some() { 0.255 } else { 0.20 };
            ui.dx(-0.94);
            ui.dy(-top + list_top);
            self.scroll.size((1.88, (top * 2. - list_top - 0.07).max(0.1)));
            let mut view = None;
            let mut rename = None;
            let mut delete = None;
            let mut delete_chart = None;
            let mut filter = None;
            self.scroll.render(ui, |ui| {
                if self.chart_tab {
                    for (i, entry) in self.charts.iter().enumerate() {
                        let y = i as f32 * 0.17;
                        let count = self.chart_counts.get(&entry.id).copied().unwrap_or(0);
                        ui.fill_rect(Rect::new(0., y, 1.86, 0.157), ui.background());
                        ui.text(&entry.info.name).pos(0.025, y + 0.015).size(0.41).max_width(1.14).draw();
                        ui.text(format!("#{} · {}", entry.id, entry.created.get(..10).unwrap_or(&entry.created)))
                            .pos(0.025, y + 0.075)
                            .size(0.30)
                            .max_width(1.14)
                            .color(Color::new(1., 1., 1., 0.65))
                            .draw();
                        ui.text(format!(
                            "{} · {} 个录像 · {} {:.1}",
                            replay::format_size(self.usage.per_chart.get(&entry.id).copied().unwrap_or(0)),
                            count,
                            entry.info.level,
                            entry.info.difficulty
                        ))
                        .pos(0.025, y + 0.116)
                        .size(0.27)
                        .max_width(1.14)
                        .color(Color::new(1., 1., 1., 0.55))
                        .draw();
                        let key = entry.id.to_string();
                        if self.multi {
                            if ui.button(
                                &format!("chart_select_{}", entry.id),
                                Rect::new(1.60, y + 0.045, 0.23, 0.064),
                                if self.selected.contains(&key) { "✓ 已选" } else { "选择" },
                            ) {
                                if !self.selected.remove(&key) {
                                    self.selected.insert(key);
                                }
                            }
                        } else {
                            if ui.button(&format!("chart_info_{}", entry.id), Rect::new(1.20, y + 0.045, 0.18, 0.064), "查看") {
                                Dialog::plain(
                                    "谱面记录",
                                    format!(
                                        "{}\n首次记录：{}\n难度：{} {:.1}\n对应录像：{} 个\nSHA-256：{}",
                                        entry.display_name(),
                                        entry.created,
                                        entry.info.level,
                                        entry.info.difficulty,
                                        count,
                                        entry.hash
                                    ),
                                )
                                .buttons(vec!["关闭".into()])
                                .show();
                            }
                            if ui.button(&format!("chart_replays_{}", entry.id), Rect::new(1.40, y + 0.045, 0.21, 0.064), "录像") {
                                filter = Some(entry.id);
                            }
                            if ui.button(&format!("chart_delete_{}", entry.id), Rect::new(1.63, y + 0.045, 0.20, 0.064), "删除") {
                                delete_chart = Some(entry.id);
                            }
                        }
                    }
                    (1.88, self.charts.len() as f32 * 0.17)
                } else {
                    for (i, &index) in filtered.iter().enumerate() {
                        let entry = &self.entries[index];
                        let y = i as f32 * 0.17;
                        ui.fill_rect(Rect::new(0., y, 1.86, 0.157), ui.background());
                        ui.text(&entry.name).pos(0.025, y + 0.015).size(0.44).max_width(1.13).draw();
                        ui.text(format!(
                            "{} · 谱面 #{} · {:.1} 秒 · {}",
                            replay::format_size(self.usage.per_replay.get(&entry.id).copied().unwrap_or(0)),
                            entry.chart_id,
                            entry.duration,
                            if entry.practice { "练习" } else { "整曲" }
                        ))
                        .pos(0.025, y + 0.077)
                        .size(0.30)
                        .max_width(1.13)
                        .color(Color::new(1., 1., 1., 0.55))
                        .draw();
                        if self.multi {
                            if ui.button(
                                &format!("replay_select_{}", entry.id),
                                Rect::new(1.60, y + 0.045, 0.23, 0.064),
                                if self.selected.contains(&entry.id) { "✓ 已选" } else { "选择" },
                            ) {
                                if !self.selected.remove(&entry.id) {
                                    self.selected.insert(entry.id.clone());
                                }
                            }
                        } else {
                            if ui.button(&format!("replay_view_{}", entry.id), Rect::new(1.20, y + 0.045, 0.18, 0.064), "查看") {
                                view = Some(entry.id.clone());
                            }
                            if ui.button(&format!("replay_rename_{}", entry.id), Rect::new(1.40, y + 0.045, 0.21, 0.064), "重命名") {
                                rename = Some((entry.id.clone(), entry.name.clone()));
                            }
                            if ui.button(&format!("replay_delete_{}", entry.id), Rect::new(1.63, y + 0.045, 0.20, 0.064), "删除") {
                                delete = Some(entry.id.clone());
                            }
                        }
                    }
                    (1.88, filtered.len() as f32 * 0.17)
                }
            });
            if let Some(id) = view {
                if self.loading.is_none() {
                    self.loading = Some(Box::pin(ReplayScene::load(id)));
                }
            }
            if let Some((id, name)) = rename {
                self.renaming = Some(id);
                request_input("replay_rename", InputBox::new().default_text(&name));
            }
            if let Some(id) = delete {
                self.confirm_delete(vec![id]);
            }
            if let Some(id) = delete_chart {
                self.confirm_delete_charts(vec![id]);
            }
            if let Some(id) = filter {
                self.change_tab(false);
                self.chart_filter = Some(id);
            }
        });
        if self.loading.is_some() {
            ui.fill_rect(ui.screen_rect(), Color::new(0., 0., 0., 0.7));
            ui.text("正在载入回放…").anchor(0.5, 0.5).size(0.65).draw();
        }
        self.sf.render(ui, s.t);
        Ok(())
    }
    fn next_scene(&mut self, s: &mut SharedState) -> NextScene {
        self.sf.next_scene(s.t).unwrap_or_default()
    }
}
