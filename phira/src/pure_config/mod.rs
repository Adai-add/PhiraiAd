//! Song-menu conversion UI and atomic local-chart publication.
mod core;
use crate::data::LocalChart;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, Utc};
pub use core::{Options, Progress};
use macroquad::prelude::*;
use prpr::{
    config::Mods,
    info::{ChartFormat, ChartInfo},
    scene::{show_error, NextScene, Scene},
    time::TimeManager,
    ui::Ui,
};
use serde_json::json;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, mpsc},
};

/// Unregistered output is removed if the user leaves/cancels before publication.
pub struct Generated {
    pub chart: Option<LocalChart>,
    pub source_path: String,
    directory: PathBuf,
    published: bool,
}
impl Generated {
    pub fn register(mut self) -> Result<()> {
        let data = crate::get_data_mut();
        let index = data
            .charts
            .iter()
            .position(|c| c.local_path == self.source_path)
            .map(|i| i + 1)
            .unwrap_or(data.charts.len());
        data.charts.insert(index, self.chart.take().unwrap());
        if let Err(e) = crate::save_data() {
            crate::get_data_mut().charts.remove(index);
            return Err(e.context("保存谱面库失败"));
        }
        self.published = true;
        crate::charts_view::NEED_UPDATE.store(true, Ordering::Relaxed);
        Ok(())
    }
}
impl Drop for Generated {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

fn generate(charts_root: &Path, source_path: String, options: Options, p: &Progress) -> Result<Generated> {
    let charts = prpr::dir::Dir::new(charts_root)?;
    let source = charts.join(&source_path)?;
    ensure!(source.is_dir(), "请先将谱面下载到本地");
    let dir = prpr::dir::Dir::new(&source)?;
    let mut info: ChartInfo = serde_yaml::from_slice(&dir.read("info.yml")?)?;
    let mut chart_name = info.chart.clone();
    if !dir.exists(&chart_name)? {
        if let Some(base) = chart_name.strip_suffix(".pec") {
            let replacement = format!("{base}.json");
            if dir.exists(&replacement)? {
                chart_name = replacement;
            }
        }
    }
    let resources = core::io::CopyPlan::new(&source, p)?;
    let bytes = dir.read(&chart_name)?;
    let (mut out, report) = core::convert(&bytes, &options, info.use_rpe_170_speed.unwrap_or(false), p)?;
    info.name = format!("纯配置{}", info.name);
    info.id = None;
    info.uploader = None;
    info.phira_ai_difficulty = None;
    // Default library order is the stored vector. Registration inserts after the
    // source; timestamp metadata also carries the explicitly requested +1 second.
    let fallback = std::fs::metadata(dir.join(&chart_name)?)?
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let base = info.created.or(info.updated).unwrap_or(fallback);
    info.created = Some(base + Duration::seconds(1));
    info.updated = Some(info.updated.unwrap_or(base) + Duration::seconds(1));
    info.chart_updated = Some(info.chart_updated.unwrap_or(base) + Duration::seconds(1));
    info.chart = "chart.json".into();
    info.format = Some(ChartFormat::Rpe);
    out["META"]["name"] = json!(info.name);
    out["META"]["song"] = json!(info.music);
    out["META"]["background"] = json!(info.illustration);
    out["META"]["composer"] = json!(info.composer);
    out["META"]["charter"] = json!(info.charter);
    out["META"]["level"] = json!(info.level);
    out["META"]["id"] = json!(uuid::Uuid::new_v4().to_string());
    // Pure gameplay should not accidentally inherit original performance caches.
    if let Some(obj) = out.as_object_mut() {
        obj.remove("phiraAiDifficulty");
        obj.remove("phira_ai_difficulty");
    }
    let custom = charts.join("custom")?;
    std::fs::create_dir_all(&custom)?;
    let staging = tempfile::Builder::new().prefix(".pure-config-").tempdir_in(&custom)?;
    resources.execute(staging.path(), p)?;
    p.check()?;
    core::io::write_json(&staging.path().join("chart.json"), &out, p)?;
    {
        let mut file = File::create(staging.path().join("info.yml"))?;
        file.write_all(serde_yaml::to_string(&info)?.as_bytes())?;
        file.flush()?;
    }
    std::fs::write(staging.path().join("pure-config-report.json"), serde_json::to_vec(&report)?)?;
    p.check()?;
    let id = uuid::Uuid::new_v4().to_string();
    let directory = custom.join(&id);
    p.update(core::Stage::Commit, 0, 1);
    std::fs::rename(staging.path(), &directory).context("提交纯配置谱面目录失败")?;
    let chart = LocalChart {
        info: info.into(),
        local_path: format!("custom/{id}"),
        record: None,
        mods: Mods::default(),
        played_unlock: false,
    };
    p.committed();
    Ok(Generated {
        chart: Some(chart),
        source_path,
        directory,
        published: false,
    })
}

pub struct ConverterScene {
    source_path: String,
    charts_root: PathBuf,
    options: Options,
    progress: Progress,
    task: Option<mpsc::Receiver<Result<Generated>>>,
    next: NextScene,
}
impl ConverterScene {
    pub fn new(path: String) -> Result<Self> {
        Ok(Self {
            source_path: path,
            charts_root: PathBuf::from(crate::dir::charts()?),
            options: Default::default(),
            progress: Default::default(),
            task: None,
            next: NextScene::None,
        })
    }
    fn start(&mut self) {
        let root = self.charts_root.clone();
        let path = self.source_path.clone();
        let options = self.options.clone();
        self.progress = Progress::default();
        let progress = self.progress.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate(&root, path, options, &progress)))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("转谱线程异常退出")));
            let _ = tx.send(result);
        });
        self.task = Some(rx);
    }
}
impl Drop for ConverterScene {
    fn drop(&mut self) {
        self.progress.cancelled.store(true, Ordering::Relaxed);
    }
}
impl Scene for ConverterScene {
    fn screen_touch_coordinates(&self) -> bool {
        true
    }
    fn update(&mut self, _tm: &mut TimeManager) -> Result<()> {
        let value = self.task.as_ref().map(|rx| rx.try_recv());
        match value {
            Some(Ok(Ok(g))) => {
                self.task = None;
                if self.progress.cancelled.load(Ordering::Relaxed) {
                    drop(g);
                    self.next = NextScene::Pop;
                } else {
                    self.next = NextScene::PopWithResult(Box::new(g));
                }
            }
            Some(Ok(Err(e))) => {
                self.task = None;
                if self.progress.cancelled.load(Ordering::Relaxed) {
                    self.next = NextScene::Pop;
                } else {
                    show_error(e);
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.task = None;
                show_error(anyhow::anyhow!("转谱线程已停止"));
            }
            _ => {}
        }
        Ok(())
    }
    fn render(&mut self, _tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        ui.fill_rect(Rect::new(-1., -ui.top, 2., ui.top * 2.), Color::new(0.06, 0.08, 0.11, 1.));
        let scale = ((ui.top * 2. - 0.08) / 0.95).min(1.);
        ui.scope(|ui| {
            ui.with(prpr::core::Matrix::new_scaling(scale), |ui| {
                ui.dx(-0.86);
                ui.dy(-0.43);
                ui.text("生成纯配置谱面").size(0.7).no_baseline().draw();
                ui.dy(0.09);
                if self.task.is_some() {
                    let progress = self.progress.snapshot();
                    ui.text(format!("总进度（估算）{:.1}%", progress.overall * 100.)).size(0.5).no_baseline().draw();
                    ui.dy(0.07);
                    let bar = Rect::new(0., 0., 1.63, 0.025);
                    ui.fill_rect(bar, Color::new(0.2, 0.23, 0.28, 1.));
                    ui.fill_rect(Rect::new(bar.x, bar.y, bar.w * progress.overall, bar.h), Color::new(0.25, 0.72, 0.95, 1.));
                    ui.dy(0.07);
                    let detail = if progress.total == 0 { "准备中".to_owned() } else { format!("本阶段 {:.1}%", progress.fraction * 100.) };
                    ui.text(format!("{} · {}", progress.stage, detail)).size(0.4).no_baseline().draw();
                    ui.dy(0.07);
                    ui.fill_rect(bar, Color::new(0.2, 0.23, 0.28, 1.));
                    if progress.total != 0 { ui.fill_rect(Rect::new(bar.x, bar.y, bar.w * progress.fraction, bar.h), Color::new(0.4, 0.82, 0.58, 1.)); }
                    ui.dy(0.07);
                    let detail = if matches!(progress.stage, "复制资源" | "保存谱面") && progress.total > 0 {
                        format!("{:.1} / {:.1} MB · ", progress.done as f64 / 1_000_000., progress.total as f64 / 1_000_000.)
                    } else { String::new() };
                    ui.text(format!("{}已耗时 {:.1} 秒", detail, progress.elapsed)).size(0.4).no_baseline().draw();
                    if ui.button("pure-cancel-task", Rect::new(0., 0.50, 0.6, 0.08), "取消生成") {
                        self.progress.cancelled.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                ui.slider("下落速度", 0.1..50., 0.1, &mut self.options.speed, Some(1.65));
                ui.dy(0.14);
                ui.slider("假原音符下隐时间（秒）", 0. ..10., 0.1, &mut self.options.hide_seconds, Some(1.65));
                ui.dy(0.14);
                ui.checkbox_with_id("pure-fade", "靠近纯配置判定线渐隐", &mut self.options.proximity_fade);
                ui.dy(0.10);
                if self.options.delete_lines {
                    self.options.delete_notes = true;
                    ui.text("删除假原音符：是（删除原判定线时必选）").size(0.45).no_baseline().draw();
                } else {
                    ui.checkbox_with_id("pure-notes", "删除假原音符", &mut self.options.delete_notes);
                }
                ui.dy(0.10);
                ui.checkbox_with_id("pure-lines", "删除原判定线", &mut self.options.delete_lines);
                if self.options.delete_lines {
                    self.options.delete_notes = true;
                }
                ui.dy(0.10);
                ui.text("生成独立谱面，名称前加“纯配置”").size(0.4).no_baseline().draw();
                ui.dy(0.09);
                if ui.button("pure-back", Rect::new(0., 0., 0.75, 0.08), "返回") {
                    self.next = NextScene::Pop;
                }
                if ui.button("pure-generate", Rect::new(0.88, 0., 0.75, 0.08), "开始生成") {
                    self.start();
                }
            });
        });
        Ok(())
    }
    fn next_scene(&mut self, _tm: &mut TimeManager) -> NextScene {
        std::mem::take(&mut self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_copy_has_exact_prefix_time_offset_and_preserved_resources() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("custom/source");
        std::fs::create_dir_all(&src).unwrap();
        let timestamp = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let info = ChartInfo {
            name: "Message".into(),
            id: Some(42),
            created: Some(timestamp),
            updated: Some(timestamp),
            chart_updated: Some(timestamp),
            ..Default::default()
        };
        std::fs::write(src.join("info.yml"), serde_yaml::to_string(&info).unwrap()).unwrap();
        let chart = json!({"META":{"RPEVersion":160},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],"judgeLineList":[core::static_line(vec![json!({"type":1,"positionX":0,"startTime":[2,0,1],"endTime":[2,0,1]})],11.,8.)],"blockAreaList":[{}],"unrelatedField":{"keep":true}});
        std::fs::write(src.join("chart.json"), serde_json::to_vec(&chart).unwrap()).unwrap();
        std::fs::write(src.join("song.mp3"), b"unchanged music").unwrap();
        std::fs::write(src.join("extra.json"), b"{\"effects\":[]}").unwrap();
        let generated = generate(root.path(), "custom/source".into(), Default::default(), &Progress::default()).unwrap();
        let output = &generated.directory;
        let new_info: ChartInfo = serde_yaml::from_slice(&std::fs::read(output.join("info.yml")).unwrap()).unwrap();
        assert_eq!(new_info.name, "纯配置Message");
        assert_eq!(new_info.created, Some(timestamp + Duration::seconds(1)));
        assert_eq!(new_info.updated, Some(timestamp + Duration::seconds(1)));
        assert_eq!(new_info.id, None);
        assert_eq!(std::fs::read(output.join("song.mp3")).unwrap(), b"unchanged music");
        assert_eq!(std::fs::read(output.join("extra.json")).unwrap(), std::fs::read(src.join("extra.json")).unwrap());
        let out: serde_json::Value = serde_json::from_slice(&std::fs::read(output.join("chart.json")).unwrap()).unwrap();
        assert!(out.get("blockAreaList").is_none());
        assert_eq!(out["META"]["name"], "纯配置Message");
        assert_eq!(out["unrelatedField"], chart["unrelatedField"]);
        assert_eq!(out["judgeLineList"][1]["notes"][0]["startTime"], chart["judgeLineList"][0]["notes"][0]["startTime"]);
        assert!(generated.chart.as_ref().unwrap().record.is_none());
        let output = output.clone();
        drop(generated);
        assert!(!output.exists());
        assert_eq!(std::fs::read(src.join("chart.json")).unwrap(), serde_json::to_vec(&chart).unwrap());
    }
}
