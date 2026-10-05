//! Persist a validated font and apply it to every UI painter on the render thread.
use anyhow::{ensure, Context, Result};
use prpr::{
    core::{BOLD_FONT, PGR_FONT},
    ui::{FontArc, TextPainter},
};
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
struct Fonts {
    regular: Option<FontArc>,
    bold: Option<FontArc>,
    pgr: Option<FontArc>,
    custom: Option<FontArc>,
    name: String,
    dirty: bool,
}
thread_local! { static FONTS: RefCell<Fonts> = RefCell::default(); }
fn path() -> Result<PathBuf> {
    Ok(PathBuf::from(crate::dir::root()?).join("custom-font.ttf"))
}
fn name_path() -> Result<PathBuf> {
    Ok(PathBuf::from(crate::dir::root()?).join("custom-font-name.txt"))
}
fn parse(bytes: Vec<u8>) -> Result<FontArc> {
    ensure!(bytes.len() <= 32 * 1024 * 1024, "字体文件不能超过 32 MiB");
    FontArc::try_from_vec(bytes).context("无法读取字体，请选择有效的 TTF 或 OTF 字体")
}
pub fn initialize(regular: FontArc, pgr: FontArc) {
    let loaded = (|| -> Result<Option<FontArc>> {
        let path = path()?;
        if !path.exists() {
            return Ok(None);
        }
        ensure!(fs::metadata(&path)?.len() <= 32 * 1024 * 1024, "自定义字体文件过大");
        Ok(Some(parse(fs::read(path)?)?))
    })();
    let custom = match loaded {
        Ok(font) => font,
        Err(error) => {
            tracing::warn!(%error, "Custom font unavailable; using default fonts");
            None
        }
    };
    let name = name_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_else(|| "自定义字体".into());
    FONTS.with(|f| {
        *f.borrow_mut() = Fonts {
            regular: Some(regular),
            pgr: Some(pgr),
            custom,
            name,
            dirty: true,
            ..Default::default()
        }
    });
}
/// Keep the default bold font current without allowing server updates to override the user's choice.
pub fn set_default_bold(font: FontArc) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.bold = Some(font);
        f.dirty = true;
    });
}
pub fn import(source: &str) -> Result<()> {
    ensure!(fs::metadata(source)?.len() <= 32 * 1024 * 1024, "字体文件不能超过 32 MiB");
    let bytes = fs::read(source).context("无法读取选择的字体文件")?;
    let font = parse(bytes.clone())?; // Validate before changing saved data or active painters.
    let name = Path::new(source)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "自定义字体".into());
    fs::write(path()?, bytes).context("无法保存自定义字体")?;
    // The name is cosmetic; failure must not invalidate an already saved font.
    if let Err(error) = fs::write(name_path()?, &name) {
        tracing::warn!(%error, "Cannot save custom font display name");
    }
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.custom = Some(font);
        f.name = name;
        f.dirty = true;
    });
    Ok(())
}
pub fn reset() -> Result<()> {
    for p in [path()?, name_path()?] {
        match fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        f.custom = None;
        f.dirty = true;
    });
    Ok(())
}
pub fn display_name() -> String {
    FONTS.with(|f| {
        let f = f.borrow();
        if f.custom.is_some() {
            f.name.clone()
        } else {
            "默认字体 · 支持 TTF / OTF".into()
        }
    })
}
pub fn apply(painter: &mut TextPainter) {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if !f.dirty {
            return;
        }
        let Some(regular) = f.regular.clone() else {
            return;
        };
        let chosen = f.custom.clone();
        *painter = TextPainter::new(chosen.clone().unwrap_or_else(|| regular.clone()), chosen.as_ref().map(|_| regular.clone()));
        if let Some(bold) = f.bold.clone() {
            BOLD_FONT.with(|p| *p.borrow_mut() = Some(TextPainter::new(chosen.clone().unwrap_or_else(|| bold.clone()), Some(regular.clone()))));
        }
        if let Some(pgr) = f.pgr.clone() {
            PGR_FONT.with(|p| *p.borrow_mut() = Some(TextPainter::new(chosen.clone().unwrap_or_else(|| pgr.clone()), chosen.as_ref().map(|_| pgr))));
        }
        f.dirty = false;
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_import_rejects_invalid_bytes_and_accepts_packaged_font() {
        assert!(parse(vec![0, 1, 2]).is_err());
        assert!(parse(include_bytes!("../../assets/font.ttf").to_vec()).is_ok());
        assert!(parse(include_bytes!("../../assets/bold.ttf").to_vec()).is_ok());
    }
}

/// Use the bundled font for imported Bn screenshots, independent of today's UI override.
pub fn bn_recognition_font() -> Option<FontArc> {
    FONTS.with(|f| f.borrow().regular.clone())
}
