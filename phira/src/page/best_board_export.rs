//! Capture the entire board in small GPU tiles, then encode one PNG off-thread.
use super::best_board::BestBoard;
use anyhow::{Context, Result};
use image::ImageEncoder;
use macroquad::prelude::*;
use prpr::{
    scene::{show_error, show_message},
    ui::Ui,
};
use std::{io::Write, sync::mpsc};

#[derive(Default)]
pub struct BestBoardExport {
    pending: bool,
    capture: Option<Capture>,
    saving: Option<mpsc::Receiver<Result<String>>>,
}

struct Capture {
    width: u32,
    logical_width: f32,
    height: u32,
    next_row: u32,
    pixels: Vec<u8>,
    target: RenderTarget,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.target.delete();
    }
}
impl BestBoardExport {
    pub fn busy(&self) -> bool {
        self.pending || self.capture.is_some() || self.saving.is_some()
    }
    pub fn cancel(&mut self) -> bool {
        if self.pending || self.capture.is_some() {
            self.pending = false;
            self.capture = None;
            return true;
        }
        self.saving.is_some()
    }
    pub fn update(&mut self) {
        if let Some(task) = &self.saving {
            let result = match task.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("图片导出任务中断"))),
            };
            if let Some(result) = result {
                self.saving = None;
                match result {
                    Ok(path) => {
                        show_message(format!("图片已导出：{path}")).duration(6.).ok();
                    }
                    Err(error) => show_error(error),
                }
            }
        }
    }
    pub fn render(&mut self, board: &mut BestBoard, ui: &mut Ui, t: f32) {
        if std::mem::take(&mut board.export_requested) && !self.busy() {
            self.pending = true;
        }
        if self.pending && board.illustrations_ready(t) {
            self.pending = false;
            match Capture::new(board.export_width(), board.export_height()) {
                Ok(capture) => self.capture = Some(capture),
                Err(error) => show_error(error),
            }
        }
        if let Some(capture) = &mut self.capture {
            capture.draw_tile(board, ui, t);
            if capture.next_row == capture.height {
                let mut capture = self.capture.take().unwrap();
                let pixels = std::mem::take(&mut capture.pixels);
                let (width, height) = (capture.width, capture.height);
                let (sender, receiver) = mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let _ = sender.send(save_image(pixels, width, height));
                });
                self.saving = Some(receiver);
            }
        }
        if self.busy() {
            ui.full_loading("正在导出整张长图…（返回可取消截取）", t);
        }
    }
}
impl Capture {
    fn new(logical_width: f32, logical_height: f32) -> Result<Self> {
        use miniquad::gl::{glGetIntegerv, GL_MAX_TEXTURE_SIZE};
        let mut maximum = 0;
        unsafe {
            glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut maximum);
        }
        anyhow::ensure!(maximum > 0 && logical_height.is_finite() && logical_height > 0., "无法创建导出画布");
        // Keep CPU capture memory below 128 MiB, and GPU memory below one tile.
        let memory_width = (128. * 1024. * 1024. / (logical_height * 4. / logical_width)).sqrt() as u32;
        let width = 1920_u32.min(maximum as u32).min(memory_width);
        anyhow::ensure!(width >= 256, "谱面数量过多，无法生成整张图片");
        let height = (logical_height * width as f32 / logical_width).ceil() as u32;
        let length = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .context("图片尺寸过大")?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).context("图片导出内存不足")?;
        pixels.resize(length, 0);
        let target = render_target(width, height.min(1024).min(maximum as u32));
        Ok(Self {
            width,
            logical_width,
            height,
            next_row: 0,
            pixels,
            target,
        })
    }
    fn draw_tile(&mut self, board: &mut BestBoard, ui: &mut Ui, t: f32) {
        let tile_height = self.target.texture.height() as u32;
        let rows = tile_height.min(self.height - self.next_row);
        let old_viewport = ui.viewport;
        let old_top = ui.top;
        let old_alpha = ui.alpha;
        let old_gl_viewport = unsafe { get_internal_gl() }.quad_gl.get_viewport();
        unsafe { get_internal_gl() }.flush();
        push_camera_state();
        ui.viewport = (0, 0, self.width as i32, tile_height as i32);
        ui.top = tile_height as f32 / self.width as f32 * self.logical_width / 2.;
        ui.alpha = 1.;
        let mut camera = ui.camera();
        camera.render_target = Some(self.target);
        camera.zoom *= 2. / self.logical_width;
        set_camera(&camera);
        clear_background(BLACK);
        ui.abs_scope(|ui| board.render_export(ui, t, self.next_row as f32 * self.logical_width / self.width as f32));
        unsafe { get_internal_gl() }.flush();
        let tile = self.target.texture.get_texture_data();
        pop_camera_state();
        unsafe { get_internal_gl() }.quad_gl.viewport(old_gl_viewport);
        ui.viewport = old_viewport;
        ui.top = old_top;
        ui.alpha = old_alpha;
        copy_tile(&mut self.pixels, &tile.bytes, self.width, tile_height, self.next_row, rows)
            .expect("capture dimensions are fixed by the allocated tile");
        self.next_row += rows;
    }
}

/// OpenGL rows are bottom-up; copy only the remaining top rows of the last tile.
fn copy_tile(output: &mut [u8], tile: &[u8], width: u32, tile_height: u32, first_row: u32, rows: u32) -> Result<()> {
    let stride = width as usize * 4;
    anyhow::ensure!(stride > 0 && rows <= tile_height && tile.len() == stride * tile_height as usize, "无效截图块");
    anyhow::ensure!((first_row as usize + rows as usize) <= output.len() / stride, "截图块超出画布");
    for row in 0..rows as usize {
        let source = (tile_height as usize - 1 - row) * stride;
        let destination = (first_row as usize + row) * stride;
        output[destination..destination + stride].copy_from_slice(&tile[source..source + stride]);
    }
    Ok(())
}

fn save_image(pixels: Vec<u8>, width: u32, height: u32) -> Result<String> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)?;
    let filename = format!("PhiraiAd-Bn-{}-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"), uuid::Uuid::new_v4());
    #[cfg(target_os = "android")]
    match crate::play_report_export::export_android_downloads(&filename, &png, "image/png", "Pictures/PhiraiAd-Bn") {
        Ok(path) => return Ok(path),
        Err(error) => return Err(error.context("相册写入失败")),
    }
    let directory = std::path::PathBuf::from(crate::dir::root()?).join("best-boards");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(filename);
    let temporary = path.with_extension("tmp");
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&png)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(path.to_string_lossy().into_owned())
}
