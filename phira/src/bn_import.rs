//! Review and merge Bn screenshot records into local storage only.
use crate::bn_image::{self, ChartCandidate, Row};
use anyhow::{ensure, Context, Result};
use inputbox::InputBox;
use macroquad::prelude::*;
use prpr::{
    scene::SimpleRecord,
    scene::{request_file, request_input, show_error, show_message},
    ui::{Scroll, Ui},
};
use std::{
    collections::HashSet,
    path::Path,
    sync::{mpsc, Mutex},
};

static FILE: Mutex<Option<String>> = Mutex::new(None);
pub fn needs_catalog() -> bool {
    FILE.lock().unwrap().is_some()
}
pub fn receive_file(path: String) {
    *FILE.lock().unwrap() = Some(path);
}
struct Selection {
    choices: Vec<ChartCandidate>,
    chosen: Option<usize>,
    selected: bool,
}
#[derive(Default)]
pub struct Panel {
    pub open: bool,
    worker: Option<mpsc::Receiver<Result<Vec<Row>>>>,
    rows: Vec<Row>,
    catalog: Vec<ChartCandidate>,
    selections: Vec<Selection>,
    scroll: Scroll,
    serial: u64,
    pub changed: bool,
    confirming: bool,
}
fn merge(record: &mut Option<SimpleRecord>, new: &SimpleRecord) {
    if let Some(old) = record {
        old.update(new);
    } else {
        *record = Some(new.clone());
    }
}
impl Panel {
    pub fn choose(&mut self) {
        request_file("_bn_import_image");
    }
    pub fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.worker = None;
        self.confirming = false;
        self.serial = self.serial.wrapping_add(1);
        true
    }
    pub fn touch(&mut self, touch: &Touch, t: f32) {
        if self.worker.is_none() && !self.confirming {
            self.scroll.touch(touch, t);
        }
    }
    pub fn update(&mut self, catalog: Vec<ChartCandidate>, t: f32) {
        self.scroll.update(t);
        if let Some(path) = FILE.lock().unwrap().take() {
            self.catalog = catalog;
            self.rows.clear();
            self.selections.clear();
            self.confirming = false;
            self.scroll = Scroll::new();
            self.open = true;
            self.serial = self.serial.wrapping_add(1);
            if let Some(font) = crate::custom_font::bn_recognition_font() {
                let (tx, rx) = mpsc::channel();
                let worker_catalog = self.catalog.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(bn_image::read(Path::new(&path), &font, &worker_catalog));
                });
                self.worker = Some(rx);
            } else {
                show_message("字体尚未加载，无法识别图片").error();
                self.open = false;
            }
        }
        let result = self.worker.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("图片识别任务中断"))),
        });
        if let Some(result) = result {
            self.worker = None;
            match result {
                Ok(rows) => {
                    self.selections = rows
                        .iter()
                        .map(|row| {
                            let mut choices: Vec<_> = row.matches.iter().map(|m| m.chart.clone()).collect();
                            if let Some(constant) = row.constant {
                                for c in &self.catalog {
                                    if bn_image::display_constant(c.constant) == bn_image::display_constant(constant)
                                        && !choices.iter().any(|p| p.path == c.path)
                                    {
                                        choices.push(c.clone());
                                    }
                                }
                            }
                            let unique = row.matches.len() == 1 && row.matches[0].similarity >= 0.72;
                            Selection {
                                choices,
                                chosen: unique.then_some(0),
                                selected: unique && row.reliable && row.score.is_some() && row.accuracy.is_some(),
                            }
                        })
                        .collect();
                    self.rows = rows;
                }
                Err(err) => {
                    show_error(err);
                    self.open = false;
                }
            }
        }
    }
    pub fn input(&mut self, id: &str, text: String) -> Result<()> {
        let parts: Vec<_> = id.split(':').collect();
        if parts.len() != 4 || parts[1].parse::<u64>().ok() != Some(self.serial) || !self.open {
            return Ok(());
        }
        let i = parts[2].parse::<usize>()?;
        let row = self.rows.get_mut(i).context("成绩条目不存在")?;
        match parts[3] {
            "score" => {
                let value = text.trim().parse::<i32>().context("请输入整数分数")?;
                ensure!((0..=1_000_000).contains(&value), "分数范围为0到1000000");
                row.score = Some(value);
            }
            "acc" => {
                let value = text.trim().trim_end_matches('%').parse::<f32>().context("请输入ACC百分比")?;
                ensure!(value.is_finite() && (0. ..=100.).contains(&value), "ACC范围为0%到100%");
                row.accuracy = Some(value / 100.);
            }
            _ => {}
        }
        Ok(())
    }
    fn commit(&mut self) -> Result<usize> {
        // Stage the entire serializable data, write with the double-backup saver,
        // then publish record fields only. Preserve the live collection cache.
        let mut staged: crate::data::Data = serde_json::from_slice(&serde_json::to_vec(crate::get_data())?)?;
        let mut seen = HashSet::new();
        let mut count = 0;
        for (row, selection) in self.rows.iter().zip(&self.selections) {
            if !selection.selected {
                continue;
            }
            let chart = selection.chosen.and_then(|i| selection.choices.get(i)).context("请选择匹配的谱面")?;
            ensure!(
                row.constant
                    .is_some_and(|c| bn_image::display_constant(c) == bn_image::display_constant(chart.constant)),
                "定数不一致，禁止导入"
            );
            let new = SimpleRecord {
                score: row.score.context("分数缺失，请补填")?,
                accuracy: row.accuracy.context("ACC缺失，请补填")?,
                full_combo: false,
            };
            ensure!((0..=1_000_000).contains(&new.score) && new.accuracy.is_finite() && (0. ..=1.).contains(&new.accuracy), "成绩数值无效");
            // A Bn screenshot cannot prove full combo; do not invent that flag.
            // Multiple rows mapped to one chart are safely merged by maxima.
            let local = staged.charts.iter_mut().find(|c| c.local_path == chart.path);
            if let Some(local) = local {
                merge(&mut local.record, &new);
            }
            if let Some(id) = chart.id {
                if let Some(record) = staged.replica_local_records.get_mut(&id) {
                    record.update(&new);
                } else {
                    staged.replica_local_records.insert(id, new.clone());
                }
            }
            merge(staged.local_records.entry(chart.path.clone()).or_insert(None), &new);
            if seen.insert(chart.path.clone()) {
                count += 1;
            }
        }
        ensure!(count > 0, "没有可导入的已选成绩");
        crate::data_guard::save(Path::new(&crate::dir::root()?), &staged)?;
        let live = crate::get_data_mut();
        live.local_records = staged.local_records;
        live.replica_local_records = staged.replica_local_records;
        for (live, saved) in live.charts.iter_mut().zip(staged.charts) {
            live.record = saved.record;
        }
        Ok(count)
    }
    pub fn render(&mut self, ui: &mut Ui, _t: f32) {
        if !self.open {
            return;
        }
        ui.fill_rect(ui.screen_rect(), Color::from_rgba(17, 23, 34, 255));
        let top = -ui.top + 0.03;
        ui.text("Bn图片导入 · 核对后仅合并本地成绩").pos(-0.94, top).size(0.45).draw();
        if ui.button("bn-import-close", Rect::new(0.76, top, 0.18, 0.07), "取消") {
            self.close();
            return;
        }
        if self.worker.is_some() {
            ui.text("正在识别图片…").anchor(0.5, 0.5).size(0.5).draw();
            return;
        }
        if self.confirming {
            ui.text("确认导入已选成绩？\n合并分数和ACC的最高值，不上传在线成绩。\n原图未显示的数值必须手动补填。")
                .pos(0., -0.15)
                .anchor(0.5, 0.5)
                .size(0.5)
                .multiline()
                .draw();
            if ui.button("bn-import-return", Rect::new(-0.45, 0.15, 0.4, 0.08), "返回核对") {
                self.confirming = false;
            }
            if ui.button("bn-import-confirm", Rect::new(0.05, 0.15, 0.4, 0.08), "确认导入") {
                match self.commit() {
                    Ok(n) => {
                        self.changed = true;
                        self.close();
                        show_message(format!("已合并{n}张谱面的本地成绩")).ok();
                    }
                    Err(err) => {
                        self.confirming = false;
                        show_error(err);
                    }
                }
            }
            return;
        }
        let viewport = (ui.top * 2. - 0.30).max(0.05);
        let offset = self.scroll.y_scroller.offset;
        self.scroll.size((1.88, viewport));
        ui.scope(|ui| {
            ui.dx(-0.94);
            ui.dy(top + 0.11);
            self.scroll.render(ui, |ui| {
                for (i, (row, selection)) in self.rows.iter_mut().zip(self.selections.iter_mut()).enumerate() {
                    let y = i as f32 * 0.22;
                    if y + 0.22 < offset || y > offset + viewport {
                        continue;
                    }
                    ui.fill_rect(Rect::new(0., y, 1.88, 0.205), Color::from_rgba(34, 44, 60, 255));
                    let name = selection
                        .chosen
                        .and_then(|i| selection.choices.get(i))
                        .map(|c| c.name.as_str())
                        .unwrap_or("未匹配 / 请选谱面");
                    ui.text(format!("{} · {} · 定数 {}", i + 1, name, row.constant.map(|c| format!("{c:.1}")).unwrap_or_else(|| "?".into())))
                        .pos(0.02, y + 0.01)
                        .size(0.36)
                        .max_width(1.47)
                        .draw();
                    let ready = selection.chosen.is_some() && row.score.is_some() && row.accuracy.is_some();
                    if ui.button(&format!("bn-select-{i}"), Rect::new(1.55, y + 0.01, 0.3, 0.06), if selection.selected { "✓ 已选" } else { "选择" })
                    {
                        if ready {
                            selection.selected = !selection.selected;
                        } else {
                            show_message("先选择谱面并补填分数、ACC").error();
                        }
                    }
                    if !selection.choices.is_empty() {
                        for (suffix, x, delta) in [("prev", 0.02, -1_i32), ("next", 0.17, 1)] {
                            if ui.button(&format!("bn-{suffix}-{i}"), Rect::new(x, y + 0.065, 0.13, 0.06), if delta < 0 { "上个" } else { "下个" })
                            {
                                let n = selection.choices.len() as i32;
                                selection.chosen = Some(selection.chosen.map_or(0, |v| (v as i32 + delta).rem_euclid(n) as usize));
                                selection.selected = false;
                            }
                        }
                    }
                    let score = row.score.map(|v| format!("{v:07}")).unwrap_or_else(|| "补填分数".into());
                    if ui.button(&format!("bn-score-{i}"), Rect::new(0.34, y + 0.065, 0.50, 0.06), score) {
                        request_input(
                            &format!("bn-import:{}:{i}:score", self.serial),
                            InputBox::new().default_text(row.score.map(|v| v.to_string()).unwrap_or_default()),
                        );
                    }
                    let acc = row.accuracy.map(|v| format!("ACC {:.2}%", v * 100.)).unwrap_or_else(|| "补填ACC".into());
                    if ui.button(&format!("bn-acc-{i}"), Rect::new(0.88, y + 0.065, 0.5, 0.06), acc) {
                        request_input(
                            &format!("bn-import:{}:{i}:acc", self.serial),
                            InputBox::new().default_text(row.accuracy.map(|v| format!("{:.2}", v * 100.)).unwrap_or_default()),
                        );
                    }
                    let status = if selection.choices.is_empty() {
                        "无同定数匹配，跳过"
                    } else if row.matches.len() == 1 && row.matches[0].similarity >= 0.72 {
                        "唯一模板候选，仍需核对"
                    } else {
                        "需要手动选谱面；仅列同定数候选"
                    };
                    let identity = selection
                        .chosen
                        .and_then(|i| {
                            selection
                                .choices
                                .get(i)
                                .map(|c| format!("候选{}/{} · {} · {:.3} · {}", i + 1, selection.choices.len(), c.level, c.constant, c.path))
                        })
                        .unwrap_or_default();
                    ui.text(format!("{status} · {identity} · {}", row.raw))
                        .pos(0.02, y + 0.145)
                        .size(0.28)
                        .max_width(1.82)
                        .draw();
                }
                (1.88, self.rows.len() as f32 * 0.22)
            });
        });
        let n = self.selections.iter().filter(|s| s.selected).count();
        if ui.button("bn-import-review", Rect::new(0.40, ui.top - 0.09, 0.54, 0.07), format!("导入已选 {n} 项")) {
            self.confirming = true;
        }
    }
}
