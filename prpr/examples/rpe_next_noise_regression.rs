//! Exercise the native Next importer and real NoiseRenderer on an offscreen framebuffer.
//! cargo run --locked -p prpr --example rpe_next_noise_regression
use macroquad::prelude::*;
use prpr::{
    config::Config,
    core::{ChartExtra, MSRenderTarget, Resource},
    fs::fs_from_assets,
    info::ChartInfo,
    noise_area::{render::NoiseRenderer, NoiseAreaState},
    parse::parse_rpe,
};
use serde_json::{json, Value};

fn conf() -> Conf {
    Conf {
        window_width: 640,
        window_height: 360,
        window_title: "RPE-Next noise regression".into(),
        ..Default::default()
    }
}
fn area(invert: bool, layer: &str) -> Value {
    json!({"bottomLeft":{"x":-300,"y":-200},"topRight":{"x":300,"y":200},
        "appearTime":[0,0,1],"disappearTime":[16,0,1],"isInvert":invert,"layerId":layer,
        "activeIntervals":[{"startTime":[4,0,1],"endTime":[8,0,1],"readyFlash":1.25}]})
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
    res.chart_target = Some(MSRenderTarget::new((640, 360), 1));
    res.camera.viewport = Some((0, 0, 640, 360));
    res.last_vp = (0, 0, 640, 360);
    let mut count = 0;
    for low in [true, false] {
        res.config.noise_area.low_performance = low;
        // Production scene loading uses a wake-capable executor. The renderer
        // intentionally yields while compiling shaders; macroquad's own frame
        // executor rejects wake_by_ref, so drive this setup on the same thread.
        let executor = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut renderer = executor.block_on(NoiseRenderer::new(low)).unwrap();
        // Compare the default Next look against the existing renderer, not a
        // separately invented Next color/compositing formula.
        let mut references = std::collections::HashMap::new();
        let mut reference_renderer = executor.block_on(NoiseRenderer::new(low)).unwrap();
        for time in [0f64, 0.75, 0.75 + 1.25 / 6., 2.5, 2.6, 4.1, 8.1] {
            res.time = time;
            res.camera.render_target = Some(res.chart_target.as_ref().unwrap().output());
            set_camera(&res.camera);
            clear_background(BLACK);
            let active = time >= 2. && time < 4.;
            let ready = time >= 0.75 && time < 2.;
            let enable = if active {
                -1.
            } else if ready {
                time as f32 + 0.25
            } else {
                time as f32 + 10.
            };
            let area = prpr::noise_area::BlockArea {
                bottom_left_percentage: prpr::noise_area::Point { x: 0., y: 0. },
                top_right_percentage: prpr::noise_area::Point { x: 100., y: 100. },
                appear_time: -100.,
                enable_time: enable,
                disable_time: 100.,
                disappear_time: 200.,
                ..Default::default()
            };
            let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
            reference_renderer
                .render(&mut res, &[area], &NoiseAreaState::default(), (0, 0, 640, 360), projection, false, |_| {})
                .unwrap();
            unsafe { get_internal_gl() }.flush();
            let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
            let offset = (180 * image.width as usize + 320) * 3;
            references.insert(time.to_bits(), <[u8; 3]>::try_from(&image.bytes[offset..offset + 3]).unwrap());
        }
        let expected = |time: f64, layers: usize| if layers == 0 { [0u8; 3] } else { references[&time.to_bits()] };

        for inverses in (0..=4).chain([255, 256, 257]) {
            for separate in [false, true] {
                let mut areas = vec![area(false, "a")];
                areas.extend((0..inverses).map(|_| area(true, if separate { "b" } else { "a" })));
                let source = json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],
                    "judgeLineList":[],"blockAreaList":areas,"noiseAreaLayers":[{"id":"a","name":"A"},{"id":"b","name":"B"}]});
                let mut fs = fs_from_assets("").unwrap();
                let chart = parse_rpe(&source.to_string(), &mut *fs, ChartExtra::default(), true).await.unwrap();
                for time in [0., 0.75, 0.75 + 1.25 / 6., 2.5, 4.1, 8.1, 2.5] {
                    res.time = time;
                    let output = res.chart_target.as_ref().unwrap().output();
                    res.camera.render_target = Some(output);
                    set_camera(&res.camera);
                    clear_background(BLACK);
                    let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
                    renderer
                        .render(&mut res, &chart.extra.block_areas, &NoiseAreaState::default(), (0, 0, 640, 360), projection, false, |_| {})
                        .unwrap();
                    unsafe { get_internal_gl() }.flush();
                    let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
                    // MSRenderTarget stores RGB8. This fork's read_pixels packs
                    // three bytes even though macroquad Image allocates RGBA.
                    let offset = (180 * image.width as usize + 320) * 3;
                    let pixel = &image.bytes[offset..offset + 3];
                    let layers = if time >= 8. {
                        0
                    } else if separate {
                        1 + usize::from(inverses == 1)
                    } else {
                        usize::from(inverses != 1)
                    };
                    let wanted = expected(time, layers);
                    for (a, b) in pixel.iter().zip(wanted) {
                        assert!(a.abs_diff(b) <= 5, "low={low}, inverses={inverses}, separate={separate}, time={time}: {pixel:?} != {wanted:?}");
                    }
                    count += 1;
                    set_default_camera();
                    next_frame().await;
                }
            }
        }
        let side = if low { 16 } else { 100 };
        let dense: Vec<_> = (0..side * side)
            .map(|i| {
                let mut a = area(false, "a");
                let (x, y) = ((i % side) * 1200 / side - 600, (i / side) * 700 / side - 350);
                a["bottomLeft"] = json!({"x":x,"y":y});
                a["topRight"] = json!({"x":x+1200/side,"y":y+700/side});
                a
            })
            .collect();
        let source = json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],
            "judgeLineList":[],"blockAreaList":dense,"noiseAreaLayers":[{"id":"a","name":"A"}]});
        let mut fs = fs_from_assets("").unwrap();
        let chart = parse_rpe(&source.to_string(), &mut *fs, ChartExtra::default(), true).await.unwrap();
        for time in [2.5, 2.6] {
            let start = std::time::Instant::now();
            res.time = time;
            res.camera.render_target = Some(res.chart_target.as_ref().unwrap().output());
            set_camera(&res.camera);
            clear_background(BLACK);
            let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
            renderer
                .render(&mut res, &chart.extra.block_areas, &NoiseAreaState::default(), (0, 0, 640, 360), projection, false, |_| {})
                .unwrap();
            unsafe { get_internal_gl() }.flush();
            let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
            let offset = (180 * image.width as usize + 320) * 3;
            for (a, b) in image.bytes[offset..offset + 3].iter().zip(expected(time, 1)) {
                assert!(a.abs_diff(b) <= 5, "dense noise mask has a gap, seam, or repeated alpha blend");
            }
            println!(
                "DENSE: low={low}, {} areas, cache={}, render+readback_ms={:.3}",
                side * side,
                time == 2.6,
                start.elapsed().as_secs_f64() * 1000.
            );
            count += 1;
            set_default_camera();
            next_frame().await;
        }
        let source = json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],
            "judgeLineList":[],"blockAreaList":[area(false,"default")]});
        let mut fs = fs_from_assets("").unwrap();
        let chart = parse_rpe(&source.to_string(), &mut *fs, ChartExtra::default(), true).await.unwrap();
        res.practice_view.scale_percent = 50.;
        res.time = 2.5;
        res.camera.render_target = Some(res.chart_target.as_ref().unwrap().output());
        set_camera(&res.camera);
        clear_background(BLACK);
        let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
        renderer
            .render(&mut res, &chart.extra.block_areas, &NoiseAreaState::default(), (0, 0, 640, 360), projection, true, |_| clear_background(BLACK))
            .unwrap();
        unsafe { get_internal_gl() }.flush();
        let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
        let offset = (180 * image.width as usize + 320) * 3;
        assert!(image.bytes[offset..offset + 3].iter().any(|v| *v > 10), "native expanded-view noise mask is missing");
        res.practice_view = Default::default();
        count += 1;
        set_default_camera();
        next_frame().await;
        let mut off_centre = area(false, "default");
        off_centre["bottomLeft"] = json!({"x":200,"y":-300});
        off_centre["topRight"] = json!({"x":450,"y":-150});
        off_centre["rotateEvents"] = json!([{"startTime":[0,0,1],"endTime":[4,0,1],"start":0,"end":37,"anchor":{"x":325,"y":-225}}]);
        let source =
            json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],"judgeLineList":[],"blockAreaList":[off_centre]});
        let mut fs = fs_from_assets("").unwrap();
        let chart = parse_rpe(&source.to_string(), &mut *fs, ChartExtra::default(), true).await.unwrap();
        for flip in [false, true] {
            for expanded in [false, true] {
                res.config.mods.set(prpr::config::Mods::FLIP_X, flip);
                res.practice_view.scale_percent = if expanded { 50. } else { 100. };
                res.time = 2.5;
                res.camera.render_target = Some(res.chart_target.as_ref().unwrap().output());
                set_camera(&res.camera);
                clear_background(BLACK);
                let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
                renderer
                    .render(&mut res, &chart.extra.block_areas, &NoiseAreaState::default(), (0, 0, 640, 360), projection, expanded, |_| {
                        clear_background(BLACK)
                    })
                    .unwrap();
                unsafe { get_internal_gl() }.flush();
                let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
                let scale = if expanded { 0.5 } else { 1. };
                let x = (320. + if flip { -1. } else { 1. } * scale * 325. / 1350. * 640.) as usize;
                let y = (180. - scale * 225. / 900. * 360.) as usize;
                let offset = (y * image.width as usize + x) * 3;
                // The shared procedural material varies across screen UVs;
                // geometry checks must sample visibility, not a constant tint.
                assert!(image.bytes[offset..offset + 3].iter().any(|v| *v > 5), "off-centre/mirror noise mask is missing");
                count += 1;
                set_default_camera();
                next_frame().await;
            }
        }
        res.practice_view = Default::default();
        res.config.mods.set(prpr::config::Mods::FLIP_X, false);
    }
    println!("PASS: {count} imported native-noise framebuffer samples; both rendering modes, 0–4 and 255–257 inversions, independent layers, warning/active/hidden/seek states");
}
