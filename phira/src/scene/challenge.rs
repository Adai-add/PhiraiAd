//! Course owner. Only one loading/game child exists at a time, so restarting
//! and leaving cannot accidentally resume the previous chart.
use crate::{
    challenge::{BadgeNumber, ChallengeProgress, ChallengeResult, ChallengeSong},
    challenge_ui::{render_badge, slanted_path, BadgePicker},
    client::{Chart, Client},
    dir, get_data, get_data_mut,
    page::ChartItem,
    save_data,
    scene::{fs_from_path, Downloading, SongScene},
};
use anyhow::{anyhow, Result};
use macroquad::prelude::*;
use prpr::{
    ext::{poll_future, semi_black, LocalTask},
    fs,
    scene::{show_error, ChallengeEvent, GameMode, LoadingScene, NextScene, Scene, SimpleRecord},
    task::Task,
    time::TimeManager,
    ui::{DRectButton, Ui},
};
use std::{any::Any, path::Path, sync::Arc};

#[derive(PartialEq)]
enum Phase {
    Playing,
    Results,
    Badges,
}

pub struct ChallengeScene {
    charts: [ChartItem; 3],
    progress: ChallengeProgress,
    phase: Phase,
    waiting: bool,
    loading: LocalTask<Result<LoadingScene>>,
    fetch: Option<Task<Result<Arc<Chart>>>>,
    download: Option<Downloading>,
    next: Option<NextScene>,
    back: DRectButton,
    retry: DRectButton,
    proceed: DRectButton,
    done: DRectButton,
    ticks: [DRectButton; 2],
    picker: BadgePicker,
    target: Option<RenderTarget>,
}
impl ChallengeScene {
    pub fn new(charts: [ChartItem; 3]) -> Self {
        Self {
            charts,
            progress: ChallengeProgress::default(),
            phase: Phase::Playing,
            waiting: false,
            loading: None,
            fetch: None,
            download: None,
            next: None,
            back: DRectButton::new(),
            retry: DRectButton::new(),
            proceed: DRectButton::new(),
            done: DRectButton::new(),
            ticks: std::array::from_fn(|_| DRectButton::new()),
            picker: BadgePicker::default(),
            target: None,
        }
    }
    fn restart(&mut self) {
        self.progress.restart();
        self.phase = Phase::Playing;
        self.waiting = false;
        self.loading = None;
        self.fetch = None;
        self.download = None;
        self.next = None;
        self.picker.open = false;
        self.back.inner.cancel();
        self.retry.inner.cancel();
        self.proceed.inner.cancel();
        self.done.inner.cancel();
        for tick in &mut self.ticks {
            tick.inner.cancel();
        }
    }
    fn fail(&mut self, error: anyhow::Error) {
        show_error(error);
        self.next = Some(NextScene::Pop);
    }
    fn prepare(&mut self) -> Result<()> {
        let index = self.progress.index();
        let chart = &mut self.charts[index];
        if chart.local_path.is_none() {
            let id = chart.info.id.ok_or_else(|| anyhow!("谱面没有可用的文件路径"))?;
            let path = format!("download/{id}");
            if Path::new(&format!("{}/{path}", dir::charts()?)).exists() {
                chart.local_path = Some(path);
            } else {
                if self.fetch.is_none() && self.download.is_none() {
                    self.fetch = Some(Task::new(async move { Client::load::<Chart>(id).await }));
                }
                return Ok(());
            }
        }
        let path = chart.local_path.clone().unwrap();
        let mut config = get_data().config.clone();
        config.mods = get_data()
            .charts
            .iter()
            .find(|c| c.local_path == path)
            .map(|c| c.mods)
            .unwrap_or(config.mods);
        config.enable_challenge();
        config.player_name = get_data().me.as_ref().map(|u| u.name.clone()).unwrap_or_else(|| "离线玩家".into());
        let data = get_data();
        config.res_pack_path = if data.respack_id == 0 {
            None
        } else {
            data.respacks
                .get(data.respack_id - 1)
                .map(|name| dir::respacks().map(|root| format!("{root}/{name}")))
                .transpose()?
        };
        let id = chart.info.id;
        self.loading = Some(Box::pin(async move {
            let mut fs = fs_from_path(&path)?;
            let mut info = fs::load_info(fs.as_mut()).await?;
            info.id = id;
            LoadingScene::new(GameMode::Normal, info, config, fs, None, None, None, None, Some(crate::play_report_export::report_fn()), None).await
        }));
        Ok(())
    }
    fn save_local_result(&self, index: usize, result: &prpr::judge::PlayResult) -> Result<()> {
        // Match ordinary strict-play local bests; course results never upload.
        let chart = &self.charts[index];
        let record = SimpleRecord {
            score: result.score as i32,
            accuracy: result.accuracy as f32,
            full_combo: result.max_combo == result.num_of_notes,
        };
        let data = get_data_mut();
        if let Some(id) = chart.info.id {
            if let Some(old) = data.replica_local_records.get_mut(&id) {
                old.update(&record);
            } else {
                data.replica_local_records.insert(id, record);
            }
        } else if let Some(path) = &chart.local_path {
            let old = if let Some(local) = data.charts.iter_mut().find(|c| c.local_path == *path) {
                &mut local.record
            } else {
                data.local_records.entry(path.clone()).or_default()
            };
            if let Some(old) = old {
                old.update(&record);
            } else {
                *old = Some(record);
            }
        }
        crate::charts_view::NEED_UPDATE.store(true, std::sync::atomic::Ordering::Relaxed);
        save_data()
    }
    fn render_results(&mut self, ui: &mut Ui, t: f32) {
        let top = ui.top;
        let screen = ui.screen_rect();
        crate::scene::TEX_BACKGROUND.with(|bg| {
            if let Some(bg) = bg.borrow().as_ref() { ui.fill_rect(screen, (**bg, screen)); }
            else { ui.fill_rect(screen, Color::from_rgba(31, 39, 51, 255)); }
        });
        ui.fill_rect(screen, semi_black(0.25));
        const U: f32 = 2. / 2048.;
        let font = |pixels: f32| pixels * U * 1.121 / 0.08;
        let left = -750. * U;
        let width = 1500. * U;
        self.back.render_text(ui, Rect::new(left, -top + 45. * U, 118. * U, 58. * U), t, "返回", font(26.), false);
        self.retry.render_text(ui, Rect::new(-614. * U, -top + 45. * U, 210. * U, 58. * U), t, "重新开始", font(26.), false);
        ui.text("课题结算").pos(-360. * U, -top + 86. * U).anchor(0., 1.).size(font(38.)).draw();
        ui.text(get_data().me.as_ref().map(|u| u.name.as_str()).unwrap_or("离线玩家"))
            .pos(734. * U, -top + 83. * U).anchor(1., 1.).size(font(30.)).max_width(0.5).draw();
        let gap = 20. * U;
        let row_h = (172. * U).min(((top * 2. - 158. * U - 0.19 - gap * 2.) / 3.).max(0.10));
        let row_scale = row_h / (172. * U);
        let title_size = font(34.);
        let title_width = (ui.text("abcdefghijklmnopqrstuvwxyz").size(title_size).measure().w / 26. * 22.).min(410. * U);
        for (i, result) in self.progress.results.iter().enumerate() {
            let y = -top + 158. * U + i as f32 * (row_h + gap);
            let rect = Rect::new(left, y, width, row_h);
            ui.fill_path(&slanted_path(rect, 22. * U), Color::from_rgba(36, 47, 62, 242));
            let cover = Rect::new(left, y, 328. * U, row_h);
            ui.fill_path(&slanted_path(cover, 22. * U), self.charts[i].illu.shading(cover, t));
            ui.text(["1st", "2nd", "3rd"][i]).pos(left + 32. * U, y + 29. * U * row_scale).anchor(0., 1.).size(font(20.)).draw();
            let tx = -394. * U;
            ui.text(&result.song.name).pos(tx, y + 68. * U * row_scale).anchor(0., 1.).size(title_size).max_width(title_width).draw();
            let level = crate::challenge::difficulty_label(&result.song.level, result.song.difficulty);
            let badge = Rect::new(tx, y + 100. * U * row_scale, 76. * U, 42. * U * row_scale);
            ui.fill_path(&slanted_path(badge, 7. * U), Color::from_rgba(76, 92, 113, 255));
            ui.text(level).pos(badge.center().x, badge.center().y).anchor(0.5, 0.5).no_baseline().size(font(23.)).draw();
            ui.text(format!("{:.1}", result.song.difficulty)).pos(tx + 100. * U, y + 130. * U * row_scale).anchor(0., 1.).size(font(28.)).draw();
            ui.text(format!("{:07}", result.score)).pos(46. * U, y + 74. * U * row_scale).anchor(0., 1.).size(font(65.)).draw();
            for (j, label) in ["Perfect", "Good", "Bad", "Miss"].iter().enumerate() {
                let x = (74. + j as f32 * 90.) * U;
                ui.text(result.counts[j].to_string()).pos(x, y + 122. * U * row_scale).anchor(0.5, 1.).size(font(28.)).draw();
                ui.text(*label).pos(x, y + 149. * U * row_scale).anchor(0.5, 1.).size(font(17.)).draw();
            }
            for (label, count, dy) in [("Early", result.early, 81.), ("Late", result.late, 124.)] {
                ui.text(label).pos(460. * U, y + dy * U * row_scale).anchor(0., 1.).size(font(23.)).draw();
                ui.text(count.to_string()).pos(694. * U, y + dy * U * row_scale).anchor(1., 1.).size(font(28.)).draw();
            }
        }
        ui.fill_path(&slanted_path(Rect::new(left, top - 0.17, width, 0.13), 22. * U), Color::from_rgba(27, 36, 50, 235));
        ui.text("总分").pos(0., top - 0.13).anchor(0.5, 1.).size(font(23.)).draw();
        ui.text(format!("{:07}", self.progress.total())).pos(0., top - 0.065).anchor(0.5, 1.).size(font(61.)).draw();
        if self.progress.total() == 3_000_000 { ui.text("φ").pos(0.19, top - 0.065).anchor(0., 1.).size(font(55.)).color(YELLOW).draw(); }
        ui.text("PhiraiAd").pos(left + 52. * U, top - 0.09).anchor(0., 1.).size(font(26.)).draw();
        self.proceed.render_text(ui, Rect::new(486. * U, top - 0.14, 212. * U, 58. * U), t, "继续", font(26.), true);
    }
    fn render_badges(&mut self, ui: &mut Ui, t: f32) {
        let top = ui.top;
        ui.fill_rect(ui.screen_rect(), BLACK);
        ui.text(format!("总分 {}", self.progress.total()))
            .pos(0., -top + 0.055)
            .anchor(0.5, 0.)
            .size(0.65)
            .draw();
        self.back
            .render_text(ui, Rect::new(-0.93, -top + 0.04, 0.18, 0.065), t, "返回", 0.4, false);
        for (i, number) in [BadgeNumber::Integer, BadgeNumber::Precise].into_iter().enumerate() {
            if let Some(badge) = self.progress.badge(number) {
                let x = if i == 0 { -0.87 } else { 0.12 };
                let h = (top * 0.65).clamp(0.10, 0.28);
                render_badge(ui, Rect::new(x + 0.075, -h * 0.4, 0.60, h * 0.8), &badge);
                self.ticks[i].render_text(ui, Rect::new(x + 0.26, h / 2. + 0.05, 0.23, 0.08), t, "✓", 0.65, false);
            }
        }
        self.done.render_text(ui, Rect::new(0.72, top - 0.11, 0.24, 0.07), t, "完成", 0.4, false);
    }
}
impl Scene for ChallengeScene {
    fn enter(&mut self, _tm: &mut TimeManager, target: Option<RenderTarget>) -> Result<()> {
        self.target = target;
        crate::ai_service::pause();
        Ok(())
    }
    fn on_result(&mut self, _tm: &mut TimeManager, result: Box<dyn Any>) -> Result<()> {
        self.waiting = false;
        let result = match result.downcast::<ChallengeEvent>() {
            Ok(event) => {
                match *event {
                    ChallengeEvent::Completed(result) => {
                        let i = self.progress.index();
                        if i >= 3 {
                            return Ok(());
                        }
                        if let Err(error) = self.save_local_result(i, &result) {
                            show_error(error);
                        }
                        let chart = &self.charts[i];
                        self.progress.complete(ChallengeResult {
                            song: ChallengeSong {
                                name: chart.info.name.clone(),
                                level: chart.info.level.clone(),
                                difficulty: chart.info.difficulty as f64,
                            },
                            score: result.score,
                            accuracy: result.accuracy,
                            counts: result.counts,
                            early: result.early_kind[1],
                            late: result.late_kind[1],
                        });
                        if self.progress.index() == 3 {
                            self.phase = Phase::Results;
                        }
                    }
                    ChallengeEvent::Restart => self.restart(),
                    ChallengeEvent::Aborted => {
                        self.next = Some(NextScene::Pop);
                    }
                }
                return Ok(());
            }
            Err(result) => result,
        };
        if let Ok(error) = result.downcast::<anyhow::Error>() {
            self.fail(*error);
        } else {
            self.fail(anyhow!("课题游玩未返回有效结果"));
        }
        Ok(())
    }
    fn touch(&mut self, tm: &mut TimeManager, touch: &Touch) -> Result<bool> {
        let t = tm.now() as f32;
        if self.picker.touch(touch, t)? {
            return Ok(true);
        }
        if let Some(download) = &mut self.download {
            if download.touch(touch, t) {
                self.next = Some(NextScene::Pop);
            }
            return Ok(true);
        }
        if self.back.touch(touch, t) {
            if self.phase == Phase::Badges {
                self.phase = Phase::Results;
            } else {
                self.next = Some(NextScene::Pop);
            }
            return Ok(true);
        }
        match self.phase {
            Phase::Results => {
                if self.retry.touch(touch, t) {
                    self.restart();
                } else if self.proceed.touch(touch, t) {
                    if self.progress.badge(BadgeNumber::Integer).is_some() {
                        self.phase = Phase::Badges;
                    } else {
                        self.next = Some(NextScene::Pop);
                    }
                }
            }
            Phase::Badges => {
                for i in 0..2 {
                    if self.ticks[i].touch(touch, t) {
                        if let Some(badge) = self.progress.badge(if i == 0 { BadgeNumber::Integer } else { BadgeNumber::Precise }) {
                            self.picker.save(badge);
                        }
                    }
                }
                if self.done.touch(touch, t) {
                    self.next = Some(NextScene::Pop);
                }
            }
            Phase::Playing => {}
        }
        Ok(true)
    }
    fn update(&mut self, _tm: &mut TimeManager) -> Result<()> {
        for chart in &mut self.charts {
            chart.illu.settle(_tm.now() as f32);
        }
        if self.next.is_some() || self.phase != Phase::Playing || self.waiting {
            return Ok(());
        }
        if let Some(fetch) = &mut self.fetch {
            if let Some(result) = fetch.take() {
                self.fetch = None;
                match result {
                    Ok(entity) => match SongScene::global_start_download(self.charts[self.progress.index()].info.clone(), (*entity).clone(), None) {
                        Ok(download) => self.download = Some(download),
                        Err(error) => self.fail(error),
                    },
                    Err(error) => self.fail(error),
                }
            }
            return Ok(());
        }
        if let Some(download) = &mut self.download {
            match download.check()? {
                Some(Some(_)) => {
                    let i = self.progress.index();
                    self.charts[i].local_path = Some(format!("download/{}", self.charts[i].info.id.unwrap()));
                    self.download = None;
                }
                Some(None) => {
                    self.download = None;
                    self.next = Some(NextScene::Pop);
                }
                None => {}
            }
            return Ok(());
        }
        if let Some(loading) = &mut self.loading {
            if let Some(result) = poll_future(loading.as_mut()) {
                self.loading = None;
                match result {
                    Ok(scene) => {
                        self.waiting = true;
                        self.next = Some(NextScene::Overlay(Box::new(scene)));
                    }
                    Err(error) => self.fail(error),
                }
            }
        } else if let Err(error) = self.prepare() {
            self.fail(error);
        }
        Ok(())
    }
    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        let mut camera = ui.camera();
        camera.render_target = self.target;
        set_camera(&camera);
        let t = tm.now() as f32;
        match self.phase {
            Phase::Playing => {
                ui.fill_rect(ui.screen_rect(), Color::from_rgba(17, 23, 34, 255));
                ui.full_loading(format!("课题模式 · {} / 3", self.progress.index() + 1), t);
                self.back
                    .render_text(ui, Rect::new(-0.93, -ui.top + 0.04, 0.18, 0.065), t, "取消", 0.4, false);
            }
            Phase::Results => self.render_results(ui, t),
            Phase::Badges => self.render_badges(ui, t),
        }
        if let Some(download) = &mut self.download {
            download.render(ui, t);
        }
        self.picker.render(ui, t);
        Ok(())
    }
    fn next_scene(&mut self, _tm: &mut TimeManager) -> NextScene {
        self.next.take().unwrap_or_default()
    }
}
