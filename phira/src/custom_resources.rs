//! Menu-only resources. Decode/copy on a worker; upload textures on the render thread.
use anyhow::{ ensure, Context, Result };
use image::{ DynamicImage, ImageFormat };
use macroquad::prelude::*;
use prpr::{ ext::{ SafeTexture, ScaleType, semi_black }, task::Task, ui::Ui };
use sasa::AudioClip;
use serde::{ Deserialize, Serialize };
use std::{
    sync::Arc,
    time::{ Instant, Duration },
    cell::RefCell,
    fs,
    io::Cursor,
    path::{ Path, PathBuf },
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuResources {
    pub music: Option<String>,
    pub music_name: String,
    pub volume: f32,
    pub background: Option<String>,
    pub background_name: String,
    pub brightness: f32,
    pub background_blur: f32,
    pub stripes: bool,
    pub background_scale: f32,
    pub background_x: f32,
    pub background_y: f32,
    pub character: Option<String>,
    pub character_name: String,
    pub character_scale: f32,
    pub character_x: f32,
    pub character_y: f32,
    pub character_rotation: f32,
}
impl Default for MenuResources {
    fn default() -> Self {
        Self {
            music: None,
            music_name: String::new(),
            volume: 1.0,
            background: None,
            background_name: String::new(),
            brightness: 1.0,
            background_blur: 0.0,
            stripes: true,
            background_scale: 100.0,
            background_x: 0.0,
            background_y: 0.0,
            character: None,
            character_name: String::new(),
            character_scale: 100.0,
            character_x: 0.0,
            character_y: 0.0,
            character_rotation: 0.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Music,
    Background,
    Character,
}
impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Music => "自定义主界面音乐",
            Self::Background => "自定义背景",
            Self::Character => "自定义人物立绘",
        }
    }
    pub fn request(self) -> &'static str {
        match self {
            Self::Music => "_menu_music",
            Self::Background => "_menu_background",
            Self::Character => "_menu_character",
        }
    }
    pub fn formats(self) -> &'static str {
        match self {
            Self::Music => "OGG / MP3 / WAV / FLAC · 推荐 OGG",
            Self::Background => "JPG / PNG / WebP · 推荐 JPG",
            Self::Character => "透明 PNG / WebP · 推荐 PNG",
        }
    }
    fn file(self, c: &MenuResources) -> Option<&str> {
        match self {
            Self::Music => c.music.as_deref(),
            Self::Background => c.background.as_deref(),
            Self::Character => c.character.as_deref(),
        }
    }
    pub fn name(self, c: &MenuResources) -> &str {
        match self {
            Self::Music => &c.music_name,
            Self::Background => &c.background_name,
            Self::Character => &c.character_name,
        }
    }
    fn assign(self, c: &mut MenuResources, file: Option<String>, name: String) {
        match self {
            Self::Music => {
                c.music = file;
                c.music_name = name;
            }
            Self::Background => {
                c.background = file;
                c.background_name = name;
            }
            Self::Character => {
                c.character = file;
                c.character_name = name;
            }
        }
    }
}
enum Decoded {
    Music(AudioClip),
    Image(DynamicImage),
}
struct Prepared {
    kind: Kind,
    file: String,
    name: String,
    decoded: Decoded,
    imported: bool,
}
#[derive(Default)]
struct Runtime {
    tasks: Vec<(Kind, Task<Result<Prepared>>)>,
    background: Option<SafeTexture>,
    background_original: Option<Arc<image::RgbaImage>>,
    background_revision: u64,
    blur_task: Option<(u64, f32, Task<Result<image::RgbaImage>>)>,
    blur_applied: f32,
    blur_requested: f32,
    blur_changed: Option<Instant>,
    character: Option<SafeTexture>,
    default_character: Option<SafeTexture>,
    music: Option<AudioClip>,
    reset_music: bool,
}
thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::default();
}
fn directory() -> Result<PathBuf> {
    Ok(PathBuf::from(crate::dir::root()?).join("custom-menu-resources"))
}
fn saved_path(file: &str) -> Result<PathBuf> {
    ensure!(
        !file.is_empty() &&
            Path::new(file)
                .file_name()
                .and_then(|s| s.to_str()) == Some(file) &&
            file != "." &&
            file != "..",
        "无效资源路径"
    );
    Ok(directory()?.join(file))
}
fn decode(kind: Kind, bytes: Vec<u8>) -> Result<Decoded> {
    ensure!(bytes.len() <= 128 * 1024 * 1024, "资源文件不能超过 128 MiB");
    if kind == Kind::Music {
        let clip = AudioClip::new(bytes).context("无法解码音乐，请选择 OGG、MP3、WAV 或 FLAC")?;
        ensure!(clip.length().is_finite() && clip.length() > 0.0, "音乐没有有效音频");
        return Ok(Decoded::Music(clip));
    }
    let format = image::guess_format(&bytes)?;
    ensure!(
        matches!(format, ImageFormat::Png | ImageFormat::WebP) ||
            (kind == Kind::Background && format == ImageFormat::Jpeg),
        "图片格式不支持"
    );
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    ensure!(
        (image.width() as u64) * (image.height() as u64) <= 16_777_216,
        "图片像素不能超过 1677 万"
    );
    if kind == Kind::Character {
        ensure!(image.color().has_alpha(), "立绘请使用带透明通道的 PNG 或 WebP");
    }
    Ok(Decoded::Image(image))
}
fn schedule(kind: Kind, source: String, imported: bool) -> Result<()> {
    ensure!(!busy(), "资源正在导入，请稍候");
    let target = directory()?;
    let task = Task::new(async move { tokio::task
            ::spawn_blocking(
                move || -> Result<Prepared> {
                    ensure!(
                        fs::metadata(&source)?.len() <= 128 * 1024 * 1024,
                        "资源文件不能超过 128 MiB"
                    );
                    let bytes = fs::read(&source)?;
                    let decoded = decode(kind, bytes.clone())?;
                    let name = Path::new(&source)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let file = if imported {
                        fs::create_dir_all(&target)?;
                        let file = format!("{}.resource", uuid::Uuid::new_v4());
                        // Unique names leave the active resource untouched until settings are saved.
                        fs::write(target.join(&file), bytes)?;
                        file
                    } else {
                        name.clone()
                    };
                    Ok(Prepared { kind, file, name, decoded, imported })
                }
            ).await
            .context("资源导入任务失败")? });
    RUNTIME.with(|rt| rt.borrow_mut().tasks.push((kind, task)));
    Ok(())
}
pub fn busy() -> bool {
    RUNTIME.with(|rt| !rt.borrow().tasks.is_empty())
}
pub fn initialize() {
    // Load sequentially; keep startup/default menu functional on corrupt or missing files.
    let c = crate::get_data().menu_resources.clone();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        for kind in [Kind::Music, Kind::Background, Kind::Character] {
            let Some(file) = kind.file(&c).map(str::to_owned) else {
                continue;
            };
            let path = match saved_path(&file) {
                Ok(p) => p,
                Err(error) => {
                    tracing::warn!(%error, "Invalid menu resource");
                    continue;
                }
            };
            let name = kind.name(&c).to_owned();
            rt.tasks.push((
                kind,
                Task::new(async move { tokio::task
                        ::spawn_blocking(
                            move || -> Result<Prepared> {
                                ensure!(
                                    fs::metadata(&path)?.len() <= 128 * 1024 * 1024,
                                    "资源文件过大"
                                );
                                let decoded = decode(kind, fs::read(path)?)?;
                                Ok(Prepared { kind, file, name, decoded, imported: false })
                            }
                        ).await
                        .context("加载资源失败")? }),
            ));
        }
    });
}
pub fn receive(id: &str, source: String) -> bool {
    let kind = match id {
        "_menu_music" => Kind::Music,
        "_menu_background" => Kind::Background,
        "_menu_character" => Kind::Character,
        _ => {
            return false;
        }
    };
    if let Err(error) = schedule(kind, source, true) {
        prpr::scene::show_error(error);
    }
    true
}
pub fn tick() {
    let completed = RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let mut out = Vec::new();
        let mut i = 0;
        while i < rt.tasks.len() {
            if let Some(result) = rt.tasks[i].1.take() {
                out.push(result);
                rt.tasks.remove(i);
            } else {
                i += 1;
            }
        }
        out
    });
    for result in completed {
        match result {
            Err(error) => prpr::scene::show_error(error.context("自定义资源不可用，保留原资源")),
            Ok(p) => {
                if p.imported {
                    let old = crate::get_data().menu_resources.clone();
                    p.kind.assign(
                        &mut crate::get_data_mut().menu_resources,
                        Some(p.file.clone()),
                        p.name
                    );
                    if let Err(error) = crate::save_data() {
                        crate::get_data_mut().menu_resources = old;
                        if let Ok(path) = saved_path(&p.file) {
                            let _ = fs::remove_file(path);
                        }
                        prpr::scene::show_error(error);
                        continue;
                    }
                    // Retain older assets: settings backups may still reference them.
                    prpr::scene::show_message("资源已应用").ok();
                }
                RUNTIME.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    match p.decoded {
                        Decoded::Music(clip) => {
                            rt.music = Some(clip);
                        }
                        Decoded::Image(image) => {
                            if p.kind == Kind::Background {
                                rt.background_original = Some(Arc::new(image.to_rgba8()));
                                rt.background_revision = rt.background_revision.wrapping_add(1);
                                rt.blur_task = None;
                                rt.blur_applied = 0.0;
                                rt.blur_changed = Some(Instant::now());
                                rt.background = Some(image.into());
                            } else {
                                rt.character = Some(image.into());
                            }
                        }
                    }
                });
            }
        }
    }
    update_blur();
}
pub fn reset(kind: Kind) -> Result<()> {
    ensure!(!busy(), "请等待资源导入完成");
    let old = crate::get_data().menu_resources.clone();
    let c = &mut crate::get_data_mut().menu_resources;
    kind.assign(c, None, String::new());
    match kind {
        Kind::Music => {
            c.volume = 1.0;
        }
        Kind::Background => {
            c.brightness = 1.0;
            c.background_blur = 0.0;
            c.stripes = true;
            c.background_scale = 100.0;
            c.background_x = 0.0;
            c.background_y = 0.0;
        }
        Kind::Character => {
            c.character_scale = 100.0;
            c.character_x = 0.0;
            c.character_y = 0.0;
            c.character_rotation = 0.0;
        }
    }
    if let Err(error) = crate::save_data() {
        crate::get_data_mut().menu_resources = old;
        return Err(error);
    }
    // Keep the former resource available to data.json backups.
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        match kind {
            Kind::Music => {
                rt.music = None;
                rt.reset_music = true;
            }
            Kind::Background => {
                rt.background_original = None;
                rt.background_revision = rt.background_revision.wrapping_add(1);
                rt.blur_task = None;
                rt.background = None;
            }
            Kind::Character => {
                rt.character = None;
            }
        }
    });
    Ok(())
}
pub fn take_music() -> (Option<AudioClip>, bool) {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        (rt.music.take(), std::mem::take(&mut rt.reset_music))
    })
}
pub fn character() -> Option<SafeTexture> {
    RUNTIME.with(|rt| rt.borrow().character.clone())
}
pub fn draw_background(ui: &mut Ui, default: &SafeTexture) {
    let custom = RUNTIME.with(|rt| rt.borrow().background.clone());
    let c = &crate::get_data().menu_resources;
    let screen = ui.screen_rect();
    let scale = finite(c.background_scale, 100.0, 5.0, 500.0) / 100.0;
    let rect = Rect::new(
        screen.x * scale + (finite(c.background_x, 0.0, -200.0, 200.0) / 100.0) * screen.w,
        screen.y * scale + (finite(c.background_y, 0.0, -200.0, 200.0) / 100.0) * screen.h,
        screen.w * scale,
        screen.h * scale
    );
    // Draw actual geometry, rather than only moving UVs inside the old screen rectangle.
    ui.fill_rect(rect, (**custom.as_ref().unwrap_or(default), rect, ScaleType::CropCenter));
    gl_use_default_material();
    ui.fill_rect(screen, semi_black(1.0 - finite(c.brightness, 1.0, 0.0, 1.0)));
}
pub fn finite(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { fallback }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_defaults_preserve_menu() {
        let c: MenuResources = serde_json::from_str("{}").unwrap();
        assert!(c.stripes);
        assert_eq!(c.background_scale, 100.0);
        assert_eq!(c.volume, 1.0);
    }
    #[test]
    fn invalid_geometry_is_safe() {
        assert_eq!(finite(f32::NAN, 100.0, 5.0, 500.0), 100.0);
        assert_eq!(finite(-1.0, 100.0, 5.0, 500.0), 5.0);
    }
    #[test]
    fn reject_unrecognized_images() {
        assert!(decode(Kind::Background, vec![0, 1, 2]).is_err());
    }
}

// Three box passes approximate a Gaussian; linear-time rolling sums, no per-frame blur.
fn gaussian_three_steps(image: &image::RgbaImage, sigma: f32) -> image::RgbaImage {
    let (w, h) = image.dimensions();
    if sigma <= 0.0 || w == 0 || h == 0 {
        return image.clone();
    }
    let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let m = (
        (12.0 * sigma * sigma - 3.0 * ((lower * lower) as f32) - 12.0 * (lower as f32) - 9.0) /
        (-4.0 * (lower as f32) - 4.0)
    )
        .round()
        .clamp(0.0, 3.0) as usize;
    let mut pixels: Vec<[u8; 4]> = image
        .pixels()
        .map(|p| {
            let a = p[3] as u32;
            [
                (((p[0] as u32) * a + 127) / 255) as u8,
                (((p[1] as u32) * a + 127) / 255) as u8,
                (((p[2] as u32) * a + 127) / 255) as u8,
                p[3],
            ]
        })
        .collect();
    let mut scratch = vec![[0u8; 4]; pixels.len()];
    for step in 0..3 {
        let radius = ((if step < m { lower } else { upper }) - 1) / 2;
        for horizontal in [true, false] {
            let (lines, length) = if horizontal {
                (h as usize, w as usize)
            } else {
                (w as usize, h as usize)
            };
            for line in 0..lines {
                let index = |position: i32| {
                    let p = position.clamp(0, (length as i32) - 1) as usize;
                    if horizontal {
                        line * (w as usize) + p
                    } else {
                        p * (w as usize) + line
                    }
                };
                let mut sum = [0u32; 4];
                for k in -radius..=radius {
                    let pixel = pixels[index(k)];
                    for c in 0..4 {
                        sum[c] += pixel[c] as u32;
                    }
                }
                let count = (2 * radius + 1) as u32;
                for pos in 0..length {
                    let dst = index(pos as i32);
                    for c in 0..4 {
                        scratch[dst][c] = ((sum[c] + count / 2) / count) as u8;
                    }
                    let left = pixels[index((pos as i32) - radius)];
                    let right = pixels[index((pos as i32) + radius + 1)];
                    for c in 0..4 {
                        sum[c] = sum[c] - (left[c] as u32) + (right[c] as u32);
                    }
                }
            }
            std::mem::swap(&mut pixels, &mut scratch);
        }
    }
    let mut out = image::RgbaImage::new(w, h);
    for (dst, p) in out.pixels_mut().zip(pixels) {
        let a = p[3] as u32;
        *dst = if a == 0 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([
                (((p[0] as u32) * 255 + a / 2) / a).min(255) as u8,
                (((p[1] as u32) * 255 + a / 2) / a).min(255) as u8,
                (((p[2] as u32) * 255 + a / 2) / a).min(255) as u8,
                p[3],
            ])
        };
    }
    out
}
fn update_blur() {
    let desired = finite(crate::get_data().menu_resources.background_blur, 0.0, 0.0, 200.0);
    RUNTIME.with(|state| {
        let mut rt = state.borrow_mut();
        if desired != rt.blur_requested {
            rt.blur_requested = desired;
            rt.blur_changed = Some(Instant::now());
        }
        if let Some((revision, sigma, task)) = &mut rt.blur_task {
            if let Some(result) = task.take() {
                let (revision, sigma) = (*revision, *sigma);
                rt.blur_task = None;
                if revision == rt.background_revision && sigma == desired {
                    match result {
                        Ok(image) => {
                            rt.background = Some(DynamicImage::ImageRgba8(image).into());
                            rt.blur_applied = sigma;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Background blur failed");
                            rt.blur_applied = sigma;
                        }
                    }
                }
            }
        }
        if
            rt.blur_task.is_none() &&
            rt.blur_applied != desired &&
            rt.blur_changed.is_some_and(|time| time.elapsed() >= Duration::from_millis(150))
        {
            if let Some(original) = rt.background_original.clone() {
                if desired == 0.0 {
                    rt.background = Some(DynamicImage::ImageRgba8((*original).clone()).into());
                    rt.blur_applied = 0.0;
                } else {
                    let revision = rt.background_revision;
                    let task = Task::new(async move {
                        tokio::task
                            ::spawn_blocking(move || gaussian_three_steps(&original, desired)).await
                            .context("背景模糊处理失败")
                    });
                    rt.blur_task = Some((revision, desired, task));
                }
            }
        }
    });
}
/// Shared by the actual home page and its editor, in the same screen coordinates.
pub fn draw_character(ui: &mut Ui, offset: f32) -> bool {
    let Some(illu) = character() else {
        return false;
    };
    draw_character_texture(ui, offset, illu);
    true
}
pub fn set_default_character(texture: SafeTexture) {
    RUNTIME.with(|rt| {
        rt.borrow_mut().default_character = Some(texture);
    });
}
pub fn draw_character_preview(ui: &mut Ui) {
    let texture = RUNTIME.with(|rt| {
        let rt = rt.borrow();
        rt.character.clone().or_else(|| rt.default_character.clone())
    });
    if let Some(texture) = texture {
        draw_character_texture(ui, 0.0, texture);
    }
}
fn draw_character_texture(ui: &mut Ui, offset: f32, illu: SafeTexture) {
    let c = &crate::get_data().menu_resources;
    let r = Rect::new(-1.0 + offset, -ui.top + 0.12, 1.0, 1.7);
    let scale = finite(c.character_scale, 100.0, 5.0, 500.0) / 100.0;
    let x = r.center().x + (finite(c.character_x, 0.0, -200.0, 200.0) / 100.0) * 2.0;
    let y = r.center().y + (finite(c.character_y, 0.0, -200.0, 200.0) / 100.0) * ui.top * 2.0;
    let angle = finite(c.character_rotation, 0.0, -180.0, 180.0).to_radians();
    let transform =
        nalgebra::Translation2::new(x, y).to_homogeneous() *
        nalgebra::Rotation2::new(angle).to_homogeneous() *
        prpr::core::Matrix::new_scaling(scale);
    let aspect = illu.width() / illu.height();
    let width = r.w.min(r.h * aspect);
    let height = width / aspect;
    let rect = Rect::new(-width / 2.0, -height / 2.0, width, height);
    ui.with(transform, |ui| ui.fill_rect(rect, (*illu, rect, ScaleType::Fit)));
}
#[cfg(test)]
mod blur_tests {
    use super::*;
    #[test]
    fn zero_blur_preserves_bytes() {
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 100]));
        assert_eq!(gaussian_three_steps(&image, 0.0), image);
    }
    #[test]
    fn constant_and_transparency_stay_constant() {
        for p in [
            [255, 0, 0, 255],
            [0, 0, 0, 0],
        ] {
            let image = image::RgbaImage::from_pixel(2, 1, image::Rgba(p));
            assert_eq!(gaussian_three_steps(&image, 100.0), image);
        }
    }
}
