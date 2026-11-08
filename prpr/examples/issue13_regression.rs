//! Offscreen graphical regression for #13. Run from the repository root:
//! ALSA_CONFIG_PATH=/path/to/null-asound.conf xvfb-run -a cargo run -p prpr --example issue13_regression
use macroquad::prelude::*;
use prpr::{
    config::Config,
    core::{
        Anim, AnimVector, BadNote, BpmList, ChartSettings, CtrlObject, HitSound, JudgeLine, JudgeLineCache, JudgeLineKind, Keyframe, Matrix, Note,
        NoteKind, Object, RenderConfig, Resource, Vector,
    },
    fs::fs_from_assets,
    info::ChartInfo,
    judge::JudgeStatus,
};
use std::cell::RefCell;

fn window_conf() -> Conf {
    Conf {
        window_width: 640,
        window_height: 360,
        window_title: "Issue 13 regression".into(),
        ..Default::default()
    }
}

fn bounds(image: &Image) -> (usize, usize, usize, usize) {
    let mut bounds = (usize::MAX, usize::MAX, 0, 0);
    for (i, rgba) in image.bytes.chunks_exact(4).enumerate() {
        if rgba[..3].iter().any(|c| *c > 20) {
            let (x, y) = (i % image.width as usize, i / image.width as usize);
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
    }
    bounds
}

#[macroquad::main(window_conf)]
async fn main() {
    prpr::core::init_assets();
    let tex = Texture2D::from_rgba8(1, 1, &[0, 0, 0, 255]);
    let mut config = Config::default();
    config.preferred_sample_rate = Some(48000);
    config.audio_buffer_size = Some(512);
    let mut res = Resource::new(
        config,
        ChartInfo {
            music: "click.ogg".into(),
            ..Default::default()
        },
        fs_from_assets("").unwrap(),
        None,
        tex.into(),
        tex.into(),
        true,
    )
    .await
    .unwrap();
    res.time = 1.;
    res.alpha = 1.;
    let mut cases = 0;
    for above in [true, false] {
        for rotation in [0., 37., 90.] {
            for flip in [false, true] {
                for target_pass in [false, true] {
                    res.config.mods.set(prpr::config::Mods::FLIP_X, flip);
                    res.note_flow_speed = if target_pass { 0.6 } else { 1. };
                    let line = JudgeLine {
                        object: Object {
                            translation: AnimVector::fixed(Vector::new(0.18, 0.2)),
                            rotation: Anim::fixed(rotation),
                            ..Default::default()
                        },
                        ctrl_obj: RefCell::new(CtrlObject {
                            // Snapshot must resample the control curve for this note.
                            y: Anim::new(vec![Keyframe::new(0., 1.4, 0), Keyframe::new(1000., 0.7, 0)]),
                            ..Default::default()
                        }),
                        kind: JudgeLineKind::Normal,
                        height: Anim::new(vec![Keyframe::new(0., 0., 0), Keyframe::new(2., 0.2, 0)]),
                        incline: Anim::fixed(12.),
                        notes: vec![],
                        color: Anim::default(),
                        parent: None,
                        rot_with_parent: false,
                        z_index: 0,
                        show_below: true,
                        attach_ui: None,
                        cache: JudgeLineCache::new(&mut []),
                    };
                    let note = Note {
                        object: Object {
                            translation: AnimVector::fixed(Vector::new(0.1, 0.03)),
                            ..Default::default()
                        },
                        kind: NoteKind::Click,
                        hitsound: HitSound::None,
                        time: 1.18,
                        height: 0.25,
                        speed: 1.2,
                        color: WHITE,
                        fx_color: None,
                        judge_area: 1.,
                        above,
                        multiple_hint: false,
                        fake: false,
                        judge: JudgeStatus::NotJudged,
                    };
                    line.ctrl_obj.borrow_mut().set_height(1000.);
                    let transform = line.now_transform(&res, &[]);
                    let bad = BadNote::from_note(&note, &line, transform, &res);
                    let target = render_target(640, 360);
                    let camera = Camera2D {
                        zoom: vec2(if target_pass { 1.2 } else { 1. }, -res.aspect_ratio),
                        render_target: target_pass.then_some(target),
                        ..Default::default()
                    };
                    set_camera(&camera);
                    clear_background(BLACK);
                    let global = Matrix::new_nonuniform_scaling(&Vector::new(if flip { -1. } else { 1. }, -1.));
                    let side = Matrix::new_nonuniform_scaling(&Vector::new(1., if above { 1. } else { -1. }));
                    let mut bpm = BpmList::new(vec![(0., 120.)]);
                    let settings = ChartSettings::default();
                    let mut ctrl = line.ctrl_obj.borrow_mut();
                    let mut config = RenderConfig {
                        settings: &settings,
                        ctrl_obj: &mut ctrl,
                        line_height: line.height.value_at(res.time).unwrap() as f64,
                        height_curve: &line.height,
                        appear_before: f64::INFINITY,
                        draw_below: true,
                        incline_sin: line.incline.now().to_radians().sin(),
                    };
                    res.apply_model_of(&global, |res| {
                        res.with_model(transform * side, |res| note.render(res, &mut config, &mut bpm));
                        res.note_buffer.borrow_mut().draw_all();
                    });
                    unsafe { get_internal_gl() }.flush();
                    let normal = if target_pass {
                        target.texture.get_texture_data()
                    } else {
                        get_screen_data()
                    };
                    set_default_camera();
                    next_frame().await;
                    set_camera(&camera);
                    clear_background(BLACK);
                    BadNote::render_all(&[bad], &mut res);
                    unsafe { get_internal_gl() }.flush();
                    let bad_image = if target_pass {
                        target.texture.get_texture_data()
                    } else {
                        get_screen_data()
                    };
                    let a = bounds(&normal);
                    let b = bounds(&bad_image);
                    assert_ne!(a.0, usize::MAX, "ordinary note was not drawn");
                    assert_ne!(b.0, usize::MAX, "Bad note was not drawn: above={above}, rotation={rotation}, flip={flip}, target={target_pass}");
                    for (x, y) in [a.0, a.1, a.2, a.3].into_iter().zip([b.0, b.1, b.2, b.3]) {
                        assert!(
                            x.abs_diff(y) <= 1,
                            "bounds differ: {a:?} vs {b:?}; above={above}, rotation={rotation}, flip={flip}, target={target_pass}"
                        );
                    }
                    // A subsequent pass must not inherit undrawn Bad geometry.
                    set_default_camera();
                    next_frame().await;
                    set_camera(&camera);
                    clear_background(BLACK);
                    res.note_buffer.borrow_mut().draw_all();
                    unsafe { get_internal_gl() }.flush();
                    let empty = if target_pass {
                        target.texture.get_texture_data()
                    } else {
                        get_screen_data()
                    };
                    assert_eq!(bounds(&empty).0, usize::MAX);
                    cases += 1;
                    set_default_camera();
                    next_frame().await;
                }
            }
        }
    }
    println!("PASS: {cases} Bad/ordinary-note pixel-bound comparisons and empty follow-up passes");
}
