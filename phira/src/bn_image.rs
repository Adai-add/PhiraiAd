//! Fixed-font, local-only Bn recognition. No model, Python, or network dependency.
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use anyhow::{ensure, Context, Result};
use image::{GrayImage, ImageReader, RgbImage};
use regex::Regex;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ChartCandidate {
    pub path: String,
    pub id: Option<i32>,
    pub level: String,
    pub name: String,
    pub constant: f64,
}
#[derive(Clone, Debug)]
pub struct Match {
    pub chart: ChartCandidate,
    pub similarity: f32,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub reliable: bool,
    pub score: Option<i32>,
    pub accuracy: Option<f32>,
    pub constant: Option<f64>,
    pub matches: Vec<Match>,
    pub raw: String,
}
#[derive(Clone)]
struct Mask {
    w: usize,
    h: usize,
    p: Vec<f32>,
}
impl Mask {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, p: vec![0.; w * h] }
    }
    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Self {
        let mut out = Self::new(w, h);
        for j in 0..h {
            out.p[j * w..(j + 1) * w].copy_from_slice(&self.p[(j + y) * self.w + x..(j + y) * self.w + x + w]);
        }
        out
    }
    fn trim(&self) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.p[y * self.w + x] > 0.25 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x0 >= x1 {
            Self::new(1, 1)
        } else {
            self.crop(x0, y0, x1 - x0, y1 - y0)
        }
    }
    fn resized(&self, w: usize, h: usize) -> Self {
        let img = GrayImage::from_raw(self.w as u32, self.h as u32, self.p.iter().map(|v| (v.clamp(0., 1.) * 255.) as u8).collect()).unwrap();
        let img = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle);
        Self {
            w,
            h,
            p: img.into_raw().into_iter().map(|v| v as f32 / 255.).collect(),
        }
    }
    fn columns(&self) -> Vec<(usize, usize)> {
        spans((0..self.w).map(|x| (0..self.h).any(|y| self.p[y * self.w + x] > 0.25)).collect())
    }
}
fn spans(active: Vec<bool>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, v) in active.into_iter().chain(std::iter::once(false)).enumerate() {
        if v && start.is_none() {
            start = Some(i);
        }
        if !v {
            if let Some(s) = start.take() {
                out.push((s, i));
            }
        }
    }
    out
}
fn render(font: &FontArc, text: &str, size: f32) -> Mask {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut x = 0.;
    let mut glyphs = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(g) = font.outline_glyph(id.with_scale_and_position(size, point(x, 0.))) {
            let b = g.px_bounds();
            x0 = x0.min(b.min.x);
            y0 = y0.min(b.min.y);
            x1 = x1.max(b.max.x);
            y1 = y1.max(b.max.y);
            glyphs.push(g);
        }
        // Same advances as the game's glyph_brush layout; do not introduce kerning.
        x += scaled.h_advance(id);
        if x > 4096. {
            break;
        }
    }
    if glyphs.is_empty() {
        return Mask::new(1, 1);
    }
    let mut out = Mask::new((x1 - x0).ceil().max(1.) as usize, (y1 - y0).ceil().max(1.) as usize);
    for g in glyphs {
        let b = g.px_bounds();
        g.draw(|x, y, v| {
            let x = (b.min.x - x0) as usize + x as usize;
            let y = (b.min.y - y0) as usize + y as usize;
            if x < out.w && y < out.h {
                out.p[y * out.w + x] = out.p[y * out.w + x].max(v);
            }
        });
    }
    out.trim()
}
struct Templates {
    glyphs: Vec<(char, Mask, f32)>,
}
impl Templates {
    fn new(font: &FontArc) -> Self {
        Self {
            glyphs: "0123456789.AC%|—"
                .chars()
                .map(|c| {
                    let m = render(font, &c.to_string(), 48.);
                    let ratio = m.w as f32 / m.h as f32;
                    (c, m.resized(24, 36), ratio)
                })
                .collect(),
        }
    }
    fn read(&self, line: &Mask) -> (String, f32) {
        let mut text = String::new();
        let mut quality = 1_f32;
        for (x0, x1) in line.columns() {
            let g = line.crop(x0, 0, x1 - x0, line.h).trim();
            if g.h as f32 <= 3_f32.max(line.h as f32 * 0.24) && g.w as f32 <= 5_f32.max(line.h as f32 * 0.3) {
                text.push('.');
                continue;
            }
            let n = g.resized(24, 36);
            let ratio = g.w as f32 / g.h as f32;
            let mut best = (f32::INFINITY, '?');
            for (c, t, r) in &self.glyphs {
                if *c == '.' {
                    continue;
                }
                let cost = n.p.iter().zip(&t.p).map(|(a, b)| (a - b).abs()).sum::<f32>() / n.p.len() as f32 + 0.10 * (ratio.max(0.02) / r).ln().abs();
                if cost < best.0 {
                    best = (cost, *c);
                }
            }
            text.push(best.1);
            quality = quality.min(1. - best.0);
        }
        (text, quality)
    }
}
// Symmetric distance to foreground, capped at 6 px. Two linear chamfer passes suffice
// and avoids adding a scientific/vision runtime to the app.
fn similarity(a: &Mask, b: &Mask) -> f32 {
    fn distance(a: &Mask, b: &Mask) -> f32 {
        let mut d: Vec<f32> = b.p.iter().map(|v| if *v > 0.35 { 0. } else { 6. }).collect();
        for y in 0..b.h {
            for x in 0..b.w {
                let i = y * b.w + x;
                if x > 0 {
                    d[i] = d[i].min(d[i - 1] + 1.);
                }
                if y > 0 {
                    d[i] = d[i].min(d[i - b.w] + 1.);
                    if x > 0 {
                        d[i] = d[i].min(d[i - b.w - 1] + std::f32::consts::SQRT_2);
                    }
                    if x + 1 < b.w {
                        d[i] = d[i].min(d[i - b.w + 1] + std::f32::consts::SQRT_2);
                    }
                }
            }
        }
        for y in (0..b.h).rev() {
            for x in (0..b.w).rev() {
                let i = y * b.w + x;
                if x + 1 < b.w {
                    d[i] = d[i].min(d[i + 1] + 1.);
                }
                if y + 1 < b.h {
                    d[i] = d[i].min(d[i + b.w] + 1.);
                    if x > 0 {
                        d[i] = d[i].min(d[i + b.w - 1] + std::f32::consts::SQRT_2);
                    }
                    if x + 1 < b.w {
                        d[i] = d[i].min(d[i + b.w + 1] + std::f32::consts::SQRT_2);
                    }
                }
            }
        }
        let mut total = 0.;
        let mut count = 0;
        for (v, d) in a.p.iter().zip(d) {
            if *v > 0.35 {
                total += d;
                count += 1;
            }
        }
        if count == 0 {
            6.
        } else {
            total / count as f32
        }
    }
    (-(distance(a, b) + distance(b, a)) / 2. / 1.25).exp()
}
fn name_similarity(observed: &Mask, name: &str, font: &FontArc) -> f32 {
    let observed = observed.trim();
    let mut best = 0_f32;
    if observed.h < 3 || name.chars().count() > 256 {
        return best;
    }
    let chars: Vec<_> = name.chars().collect();
    for size in (observed.h as f32 * 1.05).floor() as u32..=(observed.h as f32 * 2.1).ceil() as u32 {
        let mut candidates = vec![name.to_owned()];
        let sf = font.as_scaled(PxScale::from(size as f32));
        let mut width = 0.;
        let ellipsis = sf.h_advance(font.glyph_id('…'));
        for i in 1..chars.len() {
            width += sf.h_advance(font.glyph_id(chars[i - 1]));
            if (width + ellipsis - observed.w as f32).abs() < 10_f32.max(observed.w as f32 * 0.1) {
                candidates.push(chars[..i].iter().collect::<String>() + "…");
            }
        }
        for name in candidates {
            let m = render(font, &name, size as f32);
            if (m.w as f32 - observed.w as f32).abs() > 8_f32.max(observed.w as f32 * 0.09)
                || (m.h as f32 - observed.h as f32).abs() > 4_f32.max(observed.h as f32 * 0.2)
            {
                continue;
            }
            best = best.max(similarity(&observed, &m.resized(observed.w, observed.h)));
        }
    }
    best
}
fn strip_level(title: &Mask, font: &FontArc) -> Mask {
    let title = title.trim();
    let cols = title.columns();
    let mut best = (0_f32, 0);
    for n in 1..=3 {
        if cols.len() <= n {
            continue;
        }
        let x = cols[cols.len() - n].0;
        let suffix = title.crop(x, 0, title.w - x, title.h).trim();
        if suffix.h as f32 > title.h as f32 * 0.88 {
            continue;
        }
        for level in ["EZ", "HD", "IN", "AT"] {
            let score = name_similarity(&suffix, level, font);
            if score > best.0 {
                best = (score, x);
            }
        }
    }
    if best.0 > 0.5 {
        title.crop(0, 0, best.1, title.h).trim()
    } else {
        title
    }
}
fn panels(image: &RgbImage) -> Vec<(usize, usize, usize, usize)> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let body = |x: usize, y: usize| {
        let p = image.get_pixel(x as u32, y as u32).0;
        p.iter().zip([34_i32, 44, 60]).all(|(a, b)| (*a as i32 - b).abs() < 12)
    };
    let mut active: Vec<_> = (0..h).map(|y| (0..w).filter(|x| body(*x, y)).count() as f32 > w as f32 * 0.15).collect();
    // Fill text-created holes within a row, retaining the gaps between cards.
    for (s, e) in spans(active.iter().map(|v| !*v).collect()) {
        if s > 0 && e < h && e - s < ((w as f32 * 0.006) as usize).max(3) {
            active[s..e].fill(true);
        }
    }
    let mut out = Vec::new();
    for (y0, y1) in spans(active) {
        if ((y1 - y0) as f32) < w as f32 * 0.025 {
            continue;
        }
        let bottom = y1 - ((y1 - y0) as f32 * 0.06).max(3.) as usize;
        let cols = spans(
            (0..w)
                .map(|x| (bottom..y1).filter(|y| body(x, *y)).count() as f32 / (y1 - bottom) as f32 > 0.7)
                .collect(),
        );
        let cols: Vec<_> = cols.into_iter().filter(|(a, b)| (b - a) as f32 > w as f32 * 0.1).collect();
        if cols.len() == 3 {
            out.extend(cols.into_iter().map(|(x0, x1)| (x0, y0, x1, y1)));
        }
    }
    out
}
fn ink(image: &RgbImage, box_: (usize, usize, usize, usize)) -> Mask {
    let (x0, y0, x1, y1) = box_;
    let mut out = Mask::new(x1 - x0, y1 - y0);
    for y in 0..out.h {
        for x in 0..out.w {
            if x < 3_usize.max((out.h as f32 * 0.035) as usize) {
                continue;
            }
            let p = image.get_pixel((x + x0) as u32, (y + y0) as u32).0;
            let min = *p.iter().min().unwrap();
            let max = *p.iter().max().unwrap();
            if max - min < 45 {
                out.p[y * out.w + x] = ((min as f32 - 60.) / 170.).clamp(0., 1.);
            }
        }
    }
    out
}
pub fn read(path: &Path, font: &FontArc, catalog: &[ChartCandidate]) -> Result<Vec<Row>> {
    ensure!(std::fs::metadata(path)?.len() <= 64 * 1024 * 1024, "图片不能超过64 MiB");
    let dimensions = ImageReader::open(path)?.with_guessed_format()?.into_dimensions()?;
    ensure!(dimensions.0 as u64 * dimensions.1 as u64 <= 32_000_000, "图片像素过多，请拆成较短的图片");
    let image = ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .context("无法读取图片")?
        .to_rgb8();
    recognize(&image, font, catalog)
}
pub fn recognize(image: &RgbImage, font: &FontArc, catalog: &[ChartCandidate]) -> Result<Vec<Row>> {
    let boxes = panels(image);
    ensure!(!boxes.is_empty(), "未找到Bn三列卡片，请选择完整的导出图片");
    ensure!(boxes.len() <= 300, "单次最多识别300张卡片");
    let templates = Templates::new(font);
    let detail = Regex::new(r"^(\d{1,2}\.\d)\|(\d{1,2}\.\d{2})$")?;
    let numeric = Regex::new(r"^(\d{7}|—)ACC(\d{1,3}\.\d{2})%$")?;
    let score_prefix = Regex::new(r"^(\d{7})ACC")?;
    let mut rows = Vec::new();
    for box_ in boxes {
        let m = ink(image, box_);
        let lines: Vec<_> = spans(
            (0..m.h)
                .map(|y| m.p[y * m.w..(y + 1) * m.w].iter().filter(|v| **v > 0.25).count() > 1)
                .collect(),
        )
        .into_iter()
        .filter(|(a, b)| b - a >= 2)
        .collect();
        let mut row = Row {
            reliable: false,
            score: None,
            accuracy: None,
            constant: None,
            matches: Vec::new(),
            raw: "文字行切分失败".into(),
        };
        if lines.len() == 3 {
            let title = strip_level(&m.crop(0, lines[0].0, m.w, lines[0].1 - lines[0].0), font);
            let (values, q1) = templates.read(&m.crop(0, lines[1].0, m.w, lines[1].1 - lines[1].0));
            let (constants, q2) = templates.read(&m.crop(0, lines[2].0, m.w, lines[2].1 - lines[2].0));
            row.reliable = q1 >= 0.65 && q2 >= 0.65;
            row.raw = format!("{values} · {constants} [{q1:.2},{q2:.2}]");
            if q1 >= 0.5 {
                if let Some(c) = numeric.captures(&values) {
                    row.score = c[1].parse().ok();
                    row.accuracy = c[2].parse::<f32>().ok().filter(|v| (0. ..=100.).contains(v)).map(|v| v / 100.);
                } else if let Some(c) = score_prefix.captures(&values) {
                    row.score = c[1].parse().ok();
                }
                row.score = row.score.filter(|v| (0..=1_000_000).contains(v));
            }
            if q2 >= 0.5 {
                row.constant = detail.captures(&constants).and_then(|c| c[1].parse().ok());
            }
            if let Some(constant) = row.constant {
                for chart in catalog.iter().filter(|c| display_constant(c.constant) == display_constant(constant)) {
                    let similarity = name_similarity(&title, &chart.name, font);
                    if similarity >= 0.5 {
                        row.matches.push(Match {
                            chart: chart.clone(),
                            similarity,
                        });
                    }
                }
                row.matches.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
            }
        }
        rows.push(row);
    }
    Ok(rows)
}
pub fn display_constant(v: f64) -> String {
    format!("{v:.1}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_display_constants_still_need_disambiguation() {
        assert_eq!(display_constant(14.90), display_constant(14.91));
        assert_ne!(display_constant(14.9), display_constant(15.1));
    }
    #[test]
    fn uploaded_sample_if_available() {
        let (Ok(font), Ok(image)) = (std::env::var("BN_TEST_FONT"), std::env::var("BN_TEST_IMAGE")) else {
            return;
        };
        let font = FontArc::try_from_vec(std::fs::read(font).unwrap()).unwrap();
        let catalog = [14.9, 15.1, 14.6, 13.0].map(|constant| ChartCandidate {
            path: format!("{constant}"),
            id: None,
            level: "IN".into(),
            name: "RE Aoharu".into(),
            constant,
        });
        let rows = read(Path::new(&image), &font, &catalog).unwrap();
        let constants = [
            13.5, 12.4, 11.7, 15.6, 16.3, 14.9, 15.3, 15.7, 17.1, 14.9, 15.7, 15., 14.6, 15.1, 15.8, 15.1, 15.9, 14.8, 14.4, 14.8, 14.5, 14.6, 14.2,
            15.9, 14.3, 15.2, 14.6, 14.2, 14.5, 14.,
        ];
        assert_eq!(rows.len(), 30);
        for (i, (row, constant)) in rows.iter().zip(constants).enumerate() {
            assert_eq!(row.constant, Some(constant), "card {} {}", i + 1, row.raw);
        }
        let scores = [
            None,
            None,
            Some(1000000),
            Some(941850),
            Some(920448),
            Some(986534),
            Some(964511),
            Some(930309),
            Some(938462),
            Some(955198),
            Some(911789),
            Some(930340),
            Some(997738),
            Some(925759),
            Some(907381),
            Some(922387),
            Some(896716),
            Some(966322),
            Some(954288),
            Some(942819),
            Some(995127),
            Some(930000),
            Some(999160),
            Some(907208),
            Some(993807),
            Some(932345),
            Some(953814),
            Some(994789),
            Some(924850),
            Some(955107),
        ];
        let accuracies = [
            Some(100_f32),
            Some(100.),
            None,
            Some(99.54),
            Some(98.30),
            Some(99.73),
            Some(99.12),
            Some(98.29),
            Some(96.46),
            Some(99.40),
            Some(98.19),
            Some(99.16),
            Some(99.75),
            Some(99.),
            Some(97.95),
            Some(98.88),
            Some(97.69),
            Some(99.23),
            Some(99.72),
            Some(99.03),
            Some(99.46),
            Some(99.30),
            Some(99.91),
            Some(97.42),
            Some(99.70),
            Some(98.29),
            Some(99.09),
            Some(99.42),
            Some(98.93),
            Some(99.31),
        ];
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.score, scores[i], "card {} {}", i + 1, row.raw);
            match (row.accuracy, accuracies[i]) {
                (Some(a), Some(b)) => assert!((a - b / 100.).abs() < 0.000001, "card {} {}", i + 1, row.raw),
                (None, None) => {}
                _ => panic!("card {} ACC mismatch {}", i + 1, row.raw),
            }
        }
        assert_eq!(rows.iter().filter(|r| r.score.is_some()).count(), 28);
        assert_eq!(rows.iter().filter(|r| r.accuracy.is_some()).count(), 29);
        for i in [5, 15, 26] {
            assert_eq!(rows[i].matches.len(), 1, "card {} {:?}", i + 1, rows[i].matches);
            assert_eq!(rows[i].matches[0].chart.constant, constants[i]);
        }
    }
    #[test]
    fn shared_ellipsis_prefix_does_not_resolve_identity() {
        let Ok(font) = std::env::var("BN_TEST_FONT") else {
            return;
        };
        let font = FontArc::try_from_vec(std::fs::read(font).unwrap()).unwrap();
        let title = render(&font, "Connected Sky…", 40.);
        for name in ["Connected Sky - first", "Connected Sky - second"] {
            assert!(name_similarity(&title, name, &font) > 0.8);
        }
    }
}
