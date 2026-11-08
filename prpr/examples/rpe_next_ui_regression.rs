//! Regression for Next noise leaking its full-target viewport into the HUD.
//! Run from the repository root with an OpenGL display.
use macroquad::prelude::*;
use prpr::{
    config::Config,
    core::{ChartExtra, MSRenderTarget, Resource},
    ext::get_viewport,
    fs::fs_from_assets,
    info::ChartInfo,
    noise_area::{render::NoiseRenderer, NoiseAreaState},
    parse::parse_rpe,
};
use serde_json::json;

fn conf() -> Conf {
    Conf {
        window_width: 640,
        window_height: 360,
        window_title: "Next noise HUD regression".into(),
        ..Default::default()
    }
}

#[macroquad::main(conf)]
async fn main() {
    prpr::core::init_assets();
    let mut config = Config::default();
    config.preferred_sample_rate = Some(48000);
    config.audio_buffer_size = Some(512);
    let texture = Texture2D::from_rgba8(1, 1, &[0, 0, 0, 255]);
    let mut res = Resource::new(
        config,
        ChartInfo {
            music: "click.ogg".into(),
            ..Default::default()
        },
        fs_from_assets("").unwrap(),
        None,
        texture.into(),
        texture.into(),
        true,
    )
    .await
    .unwrap();
    let source = json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],
        "judgeLineList":[],"blockAreaList":[{"bottomLeft":{"x":-675,"y":-450},"topRight":{"x":675,"y":450},
        "appearTime":[0,0,1],"disappearTime":[16,0,1],"activeIntervals":[{"startTime":[4,0,1],"endTime":[8,0,1],"readyFlash":1.25}]}]});
    let chart = parse_rpe(&source.to_string(), &mut *fs_from_assets("").unwrap(), ChartExtra::default(), true)
        .await
        .unwrap();
    let mut cases = 0;
    for low in [true, false] {
        res.config.noise_area.low_performance = low;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut renderer = runtime.block_on(NoiseRenderer::new(low)).unwrap();
        for (full, local) in [
            ((0, 0, 640, 360), (0, 0, 640, 360)),
            ((0, 0, 640, 360), (80, 0, 480, 360)),
            ((0, 0, 640, 360), (0, 60, 640, 240)),
            ((20, 10, 600, 320), (60, 0, 480, 320)),
        ] {
            res.chart_target = Some(MSRenderTarget::new((full.2 as u32, full.3 as u32), 1));
            res.camera.viewport = Some((full.0 + local.0, full.1 + local.1, local.2, local.3));
            res.aspect_ratio = local.2 as f32 / local.3 as f32;
            res.camera.zoom = vec2(1., -res.aspect_ratio);
            for expanded in [false, true] {
                res.practice_view.scale_percent = if expanded { 50. } else { 100. };
                for time in [0., 0.75, 2.5, 2.6, 8.1, 2.5] {
                    let output = res.chart_target.as_ref().unwrap().output();
                    res.camera.render_target = Some(output);
                    res.time = time;
                    set_camera(&Camera2D {
                        viewport: Some(local),
                        ..res.camera
                    });
                    clear_background(BLACK);
                    let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
                    renderer
                        .render(&mut res, &chart.extra.block_areas, &NoiseAreaState::default(), full, projection, expanded, |_| {
                            clear_background(BLACK)
                        })
                        .unwrap();
                    assert_eq!(get_viewport(), local, "Next noise changed HUD viewport: low={low}, expanded={expanded}, time={time}");
                    // Draw a HUD marker using the restored chart projection. Check
                    // the actual pixels at its letterboxed position after composition.
                    let target = res.chart_target.as_ref().unwrap().output();
                    let center = vec2(-0.94, -1. / res.aspect_ratio + 0.06);
                    draw_rectangle(center.x - 0.02, center.y - 0.02, 0.04, 0.04, Color::new(0., 1., 0., 1.));
                    unsafe { get_internal_gl() }.flush();
                    let image = target.texture.get_texture_data();
                    let px = local.0 as f32 + (center.x + 1.) * local.2 as f32 / 2.;
                    let py = local.1 as f32 + (1. - center.y * res.aspect_ratio) * local.3 as f32 / 2.;
                    let offset = (py as usize * image.width as usize + px as usize) * 3;
                    let p = &image.bytes[offset..offset + 3];
                    assert!(p[1] > 200 && p[0] < 30 && p[2] < 30, "HUD marker moved: {p:?}");
                    cases += 1;
                    set_default_camera();
                    next_frame().await;
                }
            }
        }
    }
    println!("PASS: {cases} Next noise HUD viewport/pixel checks, including letterboxing, offset viewports, cached frames and expanded view");
}
