//! Draw the same hint row used by ChartsView and verify its actual glyphs/layout.
use ab_glyph::{Font, FontArc};
use macroquad::prelude::*;
use phira::{custom_rks::RksGainHint, rks_hint};
use prpr::ui::{TextPainter, Ui};

fn conf() -> Conf {
    Conf {
        window_title: "RKS gain hint regression".into(),
        window_width: 960,
        window_height: 540,
        ..Default::default()
    }
}
#[macroquad::main(conf)]
async fn main() {
    prpr::core::init_assets();
    let font = FontArc::try_from_vec(std::fs::read("assets/font.ttf").unwrap()).unwrap();
    for ch in ['φ', '√', '↑'] {
        assert_ne!(font.glyph_id(ch).0, 0, "missing {ch}");
    }
    let mut painter = TextPainter::new(font, None);
    let mut cases = 0;
    for columns in [3, 4, 6, 8] {
        for hint in [
            RksGainHint::Phi,
            RksGainHint::NoGain,
            RksGainHint::Ready,
            RksGainHint::TargetAccuracy(9901),
            RksGainHint::TargetAccuracy(7000),
            RksGainHint::TargetAccuracy(10000),
            RksGainHint::Pending,
        ] {
            let mut ui = Ui::new(&mut painter, None);
            set_camera(&ui.camera());
            clear_background(BLACK);
            let cw = 2. / columns as f32;
            let card = Rect::new(-0.7, -0.25, cw - 0.026, 0.274);
            ui.fill_rect(card, Color::from_rgba(20, 25, 30, 255));
            let size = 0.52 * card.w / cw;
            let x = card.right() - 0.016;
            let difficulty = ui.text("IN Lv.16.5").pos(x, card.y + 0.016).anchor(1., 0.).size(size).draw();
            let accuracy = ui
                .text("99.50%")
                .pos(x, difficulty.bottom() + 0.017)
                .anchor(1., 0.)
                .size(size * 0.78)
                .draw();
            let row = rks_hint::render(&mut ui, hint, card, accuracy, size * 0.78, WHITE);
            let title = ui
                .text("Test chart")
                .pos(card.x + 0.01, card.bottom() - 0.02)
                .anchor(0., 1.)
                .size(0.6 * card.w / cw)
                .draw();
            assert!(row.y > accuracy.bottom());
            assert!(row.bottom() < title.y, "hint overlaps chart name, columns={columns}");
            assert!(row.x - 0.006 >= card.x && row.right() + 0.006 <= card.right());
            let top = ui.top;
            let image = get_screen_data();
            let x0 = (((row.x + 1.) * 0.5 * image.width as f32).floor() as i32 - 2).max(0) as usize;
            let x1 = (((row.right() + 1.) * 0.5 * image.width as f32).ceil() as usize + 2).min(image.width as usize);
            let y0 = (((1. - row.bottom() / top) * 0.5 * image.height as f32).floor() as i32 - 3).max(0) as usize;
            let y1 = (((1. - row.y / top) * 0.5 * image.height as f32).ceil() as usize + 3).min(image.height as usize);
            let mut visible = 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &image.bytes[(y * image.width as usize + x) * 4..][..3];
                    let matches = match hint {
                        RksGainHint::Phi => p[0] > 150 && p[1] > 120 && p[2] < 100,
                        RksGainHint::NoGain => p[1] > 150 && p[0] < 140 && p[2] < 160,
                        _ => p.iter().all(|c| *c > 140),
                    };
                    visible += usize::from(matches);
                }
            }
            assert!(visible > 0, "hint glyph/color invisible: {hint:?}, columns={columns}");
            cases += 1;
            next_frame().await;
        }
    }
    println!(
        "PASS: {cases} production hint rows; gold φ, green √, arrows, two decimals, narrow cards and no overlaps; version v{}",
        env!("CARGO_PKG_VERSION")
    );
}
