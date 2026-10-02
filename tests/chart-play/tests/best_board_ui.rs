//! Exercise the production board with recording graphics adapters.
#![allow(dead_code)]
extern crate self as macroquad;
extern crate self as prpr;
#[path = "../../../phira/src/page/best_board.rs"]
mod board;
#[path = "../../../phira/src/custom_rks.rs"]
pub mod custom_rks;
use std::{cell::RefCell, collections::HashMap};
#[derive(Clone, Copy, Debug)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn center(self) -> Vec2 {
        Vec2 {
            x: self.x + self.w / 2.,
            y: self.y + self.h / 2.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(u8);
impl Color {
    pub fn from_rgba(r: u8, _: u8, _: u8, _: u8) -> Self {
        Self(r)
    }
}
pub const YELLOW: Color = Color(255);
#[derive(Clone, Copy)]
pub struct Touch {
    pub position: Vec2,
}
pub mod prelude {
    pub use crate::{Color, Rect, Touch, YELLOW};
}
#[derive(Clone)]
pub struct Illustration;
impl Illustration {
    pub fn alpha(&self, _: f32) -> f32 {
        1.
    }
    pub fn aspect_ratio(&self) -> f32 {
        16. / 9.
    }
    pub fn settle(&mut self, _: f32) {}
    pub fn notify(&self) {}
    pub fn shading(&self, _: Rect, _: f32) -> Color {
        Color(70)
    }
}
pub struct SharedState {
    pub charts_local: Vec<Chart>,
}
pub struct Chart {
    pub local_path: Option<String>,
    pub info: Info,
    pub illu: Illustration,
}
pub struct Info {
    pub id: Option<i32>,
    pub level: String,
}
#[derive(Default, Clone)]
pub struct Data {
    pub me: Option<User>,
    pub charts: Vec<Local>,
    pub local_records: HashMap<String, Option<Record>>,
    pub replica_local_records: HashMap<i32, Record>,
}
#[derive(Clone)]
pub struct User {
    pub name: String,
}
#[derive(Clone)]
pub struct Local {
    pub local_path: String,
    pub record: Option<Record>,
}
#[derive(Clone)]
pub struct Record {
    pub score: i32,
}
thread_local! {static DATA:RefCell<Data>=RefCell::new(Data::default());}
pub fn get_data() -> Data {
    DATA.with(|v| v.borrow().clone())
}
pub mod ui {
    use super::*;
    #[derive(Default)]
    pub struct Ui {
        pub top: f32,
        pub x: f32,
        pub y: f32,
        pub texts: Vec<(String, f32, f32, Color)>,
        pub rects: Vec<Rect>,
        pub buttons: Vec<(String, Rect)>,
    }
    impl Ui {
        pub fn screen_rect(&self) -> Rect {
            Rect::new(-1., -self.top, 2., 2. * self.top)
        }
        pub fn fill_rect(&mut self, r: Rect, _: Color) {
            self.rects.push(Rect::new(r.x + self.x, r.y + self.y, r.w, r.h));
        }
        pub fn text(&mut self, text: impl Into<String>) -> Text<'_> {
            Text {
                ui: self,
                text: text.into(),
                x: 0.,
                y: 0.,
                color: Color(0),
            }
        }
        pub fn scope(&mut self, f: impl FnOnce(&mut Self)) {
            let old = (self.x, self.y);
            f(self);
            (self.x, self.y) = old;
        }
        pub fn dx(&mut self, x: f32) {
            self.x += x;
        }
        pub fn dy(&mut self, y: f32) {
            self.y += y;
        }
    }
    pub struct Text<'a> {
        ui: &'a mut Ui,
        text: String,
        x: f32,
        y: f32,
        color: Color,
    }
    impl Text<'_> {
        pub fn pos(mut self, x: f32, y: f32) -> Self {
            self.x = x;
            self.y = y;
            self
        }
        pub fn size(self, _: f32) -> Self {
            self
        }
        pub fn anchor(self, _: f32, _: f32) -> Self {
            self
        }
        pub fn max_width(self, w: f32) -> Self {
            assert!(w > 0.);
            self
        }
        pub fn color(mut self, c: Color) -> Self {
            self.color = c;
            self
        }
        pub fn draw(self) -> Rect {
            // Approximate glyph advances; production measures with the actual font.
            let width = self.text.len() as f32 * 0.01;
            let rect = Rect::new(self.x + self.ui.x, self.y + self.ui.y, width, 0.02);
            self.ui.texts.push((self.text, rect.x, rect.y, self.color));
            rect
        }
        pub fn measure(&self) -> Rect {
            Rect::new(self.x, self.y, self.text.len() as f32 * 0.01, 0.02)
        }
    }
    pub struct RectButton;
    impl RectButton {
        pub fn cancel(&mut self) {}
    }
    pub struct DRectButton {
        pub inner: RectButton,
    }
    impl DRectButton {
        pub fn new() -> Self {
            Self { inner: RectButton }
        }
        pub fn render_text(&mut self, u: &mut Ui, r: Rect, _: f32, label: impl Into<String>, _: f32, _: bool) {
            u.buttons.push((label.into(), r));
        }
        pub fn touch(&mut self, _: &Touch, _: f32) -> bool {
            false
        }
    }
    pub struct Scroller {
        pub offset: f32,
    }
    pub struct Scroll {
        pub y_scroller: Scroller,
    }
    impl Scroll {
        pub fn new() -> Self {
            Self {
                y_scroller: Scroller { offset: 0. },
            }
        }
        pub fn touch(&mut self, _: &Touch, _: f32) {}
        pub fn update(&mut self, _: f32) {}
        pub fn size(&mut self, _: (f32, f32)) {}
        pub fn render(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Ui) -> (f32, f32)) {
            f(ui);
        }
    }
}
fn setup(n: usize) -> (board::BestBoard, SharedState) {
    let locals: Vec<_> = (0..n)
        .map(|i| custom_rks::LocalInput {
            path: format!("chart{i}"),
            name: format!("Song{i}"),
            level: "IN".into(),
            difficulty: 16. - i as f64 / 10.,
            ai_difficulty: None,
            accuracy: Some(if i == 0 { 100. } else { 99. }),
        })
        .collect();
    let settings = custom_rks::Settings {
        best_count: n,
        ap_count: 1,
        ..Default::default()
    };
    let state = SharedState {
        charts_local: (0..n)
            .map(|i| Chart {
                local_path: Some(format!("chart{i}")),
                info: Info {
                    id: Some(i as i32),
                    level: "IN Lv.15".into(),
                },
                illu: Illustration,
            })
            .collect(),
    };
    DATA.with(|v| {
        *v.borrow_mut() = Data {
            me: Some(User { name: "Player".into() }),
            charts: vec![Local {
                local_path: "chart0".into(),
                record: Some(Record { score: 1_000_000 }),
            }],
            ..Default::default()
        }
    });
    let mut board = board::BestBoard::new();
    board.show(custom_rks::ranked_entries(&settings, &locals), custom_rks::calculate(&settings, &locals).1.rks, &state);
    (board, state)
}
#[test]
fn renders_three_columns_identity_details_and_yellow_ap() {
    let (mut board, _) = setup(4);
    let mut ui = ui::Ui {
        top: 0.5625,
        ..Default::default()
    };
    board.render(&mut ui, 1.);
    assert!(ui
        .texts
        .iter()
        .any(|(s, _, _, _)| s.contains("Player") && s.contains("RKS") && s.contains("B5")));
    let titles: Vec<_> = ui.texts.iter().filter(|(s, _, _, _)| s.starts_with("Song")).collect();
    assert_eq!(titles.len(), 5);
    assert_eq!(titles[0].2, titles[1].2);
    assert_eq!(titles[1].2, titles[2].2);
    assert!(titles[0].1 < titles[1].1 && titles[1].1 < titles[2].1);
    assert!(titles[3].2 > titles[0].2);
    assert!(ui.texts.iter().any(|(s, _, _, c)| s == "AP" && *c == YELLOW));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "1000000"));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "ACC 100.00%"));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "PhiraiAd"));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "16.0 | 16.00"));
    let score_y = ui.texts.iter().find(|(s, _, _, _)| s == "1000000").unwrap().2;
    let acc_y = ui.texts.iter().find(|(s, _, _, _)| s == "ACC 100.00%").unwrap().2;
    assert_eq!(score_y, acc_y);
    assert!(!ui
        .texts
        .iter()
        .any(|(s, _, _, _)| s.contains("RKS贡献") || s.contains("定数") || s.contains("等效RKS")));
    let score_x = ui.texts.iter().find(|(s, _, _, _)| s == "1000000").unwrap().1;
    let acc_x = ui.texts.iter().find(|(s, _, _, _)| s == "ACC 100.00%").unwrap().1;
    assert!(acc_x > score_x);
    let covers: Vec<_> = ui.rects.iter().filter(|rect| rect.w > 0.26 && rect.w < 0.27).collect();
    assert!(!covers.is_empty());
    for cover in covers {
        assert!((cover.w / cover.h - 2.).abs() < 1e-5);
        let card = ui
            .rects
            .iter()
            .find(|card| card.w > 0.52 && card.w < 0.54 && (card.x - cover.x).abs() < 1e-5 && (card.y - cover.y).abs() < 1e-5)
            .unwrap();
        assert!((card.h - cover.h).abs() < 1e-5);
        assert!(card.x >= -1. && card.right() <= 1.);
    }

    for (_, r) in &ui.buttons {
        assert!(r.x >= -1. && r.right() <= 1. && r.y >= -ui.top && r.y + r.h <= ui.top);
    }
}
#[test]
fn empty_board_and_close_reset_state() {
    let (mut board, _) = setup(0);
    let mut ui = ui::Ui {
        top: 0.45,
        ..Default::default()
    };
    board.render(&mut ui, 1.);
    assert!(ui.texts.iter().any(|(s, _, _, _)| s.contains("B0")));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "暂无可计入的成绩"));
    assert!(board.close());
    assert!(!board.open);
    assert!(!board.close());
    let mut after = ui::Ui::default();
    board.render(&mut after, 1.);
    assert!(after.texts.is_empty());
}
#[test]
fn all_real_score_sources_are_compared() {
    let (mut board, state) = setup(1);
    DATA.with(|v| {
        let mut data = v.borrow_mut();
        data.charts[0].record = Some(Record { score: 900_000 });
        data.local_records.insert("chart0".into(), Some(Record { score: 950_000 }));
        data.replica_local_records.insert(0, Record { score: 999_000 });
    });
    let local = custom_rks::LocalInput {
        path: "chart0".into(),
        name: "Song0".into(),
        level: "IN".into(),
        difficulty: 16.,
        ai_difficulty: None,
        accuracy: Some(99.),
    };
    board.show(custom_rks::ranked_entries(&Default::default(), &[local]), 15., &state);
    let mut ui = ui::Ui {
        top: 0.5625,
        ..Default::default()
    };
    board.render(&mut ui, 1.);
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "0999000"));
}

#[test]
fn long_image_includes_every_row_and_excludes_action_buttons() {
    let (mut board, _) = setup(30);
    let height = board.export_height();
    let mut ui = ui::Ui {
        top: height / 2.,
        ..Default::default()
    };
    board.render_export(&mut ui, 1., 0.);
    // AP and best groups independently contain the AP chart: B31.
    assert_eq!(ui.texts.iter().filter(|(s, _, _, _)| s.starts_with("Song")).count(), 31);
    assert!(ui.texts.iter().any(|(s, _, _, _)| s.contains("B31")));
    assert!(ui.texts.iter().any(|(s, _, _, _)| s == "PhiraiAd"));
    assert!(ui.buttons.is_empty());
    for (_, _, y, _) in &ui.texts {
        assert!(*y >= -ui.top && *y <= ui.top);
    }
    // The middle tile retains rows intersecting its upper and lower edges.
    let mut tile = ui::Ui {
        top: 0.10,
        ..Default::default()
    };
    board.render_export(&mut tile, 1., 0.35);
    let titles: Vec<_> = tile.texts.iter().filter(|(s, _, _, _)| s.starts_with("Song")).collect();
    assert!(titles.len() >= 3 && titles.len() < 31);
}
