//! Validate custom Next resources through the existing noise Program/pass path.
//! Optional argument: the local upstream rpe-next tree (for NoiseField/Ocean).
use macroquad::prelude::*;
use prpr::{
    config::Config,
    core::{ChartExtra, MSRenderTarget, Resource},
    dir::Dir,
    fs::{fs_from_assets, ExternalFileSystem},
    info::ChartInfo,
    noise_area::{render::NoiseRenderer, NoiseAreaState},
    parse::parse_rpe,
};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};
fn conf() -> Conf {
    Conf {
        window_width: 640,
        window_height: 360,
        window_title: "Next skin bindings regression".into(),
        ..Default::default()
    }
}
fn source(skin: &str, parameters: Value, channel: &str) -> Value {
    json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],"judgeLineList":[],
 "noiseAreaLayers":[{"id":"a","name":"A","skin":format!("shader/{skin}"),"parameters":parameters,"channel":channel}],
 "blockAreaList":[{"bottomLeft":{"x":-10000,"y":-10000},"topRight":{"x":10000,"y":10000},"layerId":"a","appearTime":[0,0,1],"disappearTime":[200,0,1],"activeIntervals":[{"startTime":[0,0,1],"endTime":[200,0,1]}]}]})
}
fn write(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let p = root.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}
fn sample(res: &Resource, x: usize, y: usize) -> [u8; 3] {
    let image = res.chart_target.as_ref().unwrap().output().texture.get_texture_data();
    let n = (y * image.width as usize + x) * 3;
    image.bytes[n..n + 3].try_into().unwrap()
}
async fn renderer(root: &Path, name: &str, parameters: Value, channel: &str, low: bool) -> NoiseRenderer {
    let mut fs = ExternalFileSystem(Arc::new(Dir::new(root).unwrap()));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let chart = runtime
        .block_on(parse_rpe(&source(name, parameters, channel).to_string(), &mut fs, ChartExtra::default(), true))
        .unwrap();
    let mut renderer = runtime.block_on(NoiseRenderer::new(low)).unwrap();
    runtime.block_on(renderer.configure_skins(&chart.extra.noise_skins, low)).unwrap();
    renderer
}
async fn draw(renderer: &mut NoiseRenderer, res: &mut Resource, root: &Path, name: &str, parameters: Value, channel: &str, time: f64) -> [u8; 3] {
    let mut fs = ExternalFileSystem(Arc::new(Dir::new(root).unwrap()));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let chart = runtime
        .block_on(parse_rpe(&source(name, parameters, channel).to_string(), &mut fs, ChartExtra::default(), true))
        .unwrap();
    res.time = time;
    res.camera.render_target = Some(res.chart_target.as_ref().unwrap().output());
    set_camera(&res.camera);
    clear_background(BLACK);
    let projection = unsafe { get_internal_gl() }.quad_gl.get_projection_matrix();
    renderer
        .render(res, &chart.extra.block_areas, &NoiseAreaState::default(), (0, 0, 640, 360), projection, false, |_| {})
        .unwrap();
    unsafe { get_internal_gl() }.flush();
    let pixel = sample(res, 320, 180);
    set_default_camera();
    next_frame().await;
    pixel
}
#[macroquad::main(conf)]
async fn main() {
    prpr::core::init_assets();
    let root = tempfile::tempdir().unwrap();
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
    res.camera.zoom = vec2(1., -640. / 360.);
    res.aspect_ratio = 640. / 360.;
    let header = "precision highp float;\nvarying vec2 uv;\nuniform float touchReleaseDuration; // %0.1%\n";
    write(root.path(),"shader/Texture/main.glsl",format!("{header}uniform float strength; // %1.0%\nuniform sampler2D screenTexture;\nuniform sampler2D blockMask;\nuniform sampler2D imageTex; // @texture image.png wrap=repeat filter=nearest colorspace=linear alphaMode=straight\nvoid main(){{gl_FragColor=vec4(texture2D(imageTex,uv).rgb*strength,1.0);}}"));
    let png = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]));
    std::fs::create_dir_all(root.path().join("shader/Texture")).unwrap();
    png.save(root.path().join("shader/Texture/image.png")).unwrap();
    let mut r = renderer(root.path(), "Texture", json!({"strength":0.5}), "normal", false).await;
    let p = draw(&mut r, &mut res, root.path(), "Texture", json!({"strength":0.5}), "normal", 1.).await;
    assert!(p.iter().zip([100u8, 50, 25]).all(|(a, b)| a.abs_diff(b) <= 2), "texture/parameter failed: {p:?}");
    write(
        root.path(),
        "shader/Channels/main.glsl",
        format!("{header}uniform sampler2D combined;void main(){{gl_FragColor=vec4(texture2D(combined,uv).rg,0.0,1.0);}}"),
    );
    write(root.path(), "shader/Channels/red.glsl", "precision highp float;void main(){gl_FragColor=vec4(0.25);}");
    write(root.path(), "shader/Channels/green.glsl", "precision highp float;void main(){gl_FragColor=vec4(0.5);}");
    write(root.path(),"shader/Channels/config.json",json!({"version":1,"padding":0,"targets":{"combined":{"format":"rg8","scaleX":0.5,"scaleY":0.5}},"passes":[{"shader":"red.glsl","output":"combined.r"},{"shader":"green.glsl","output":"combined.g"},{"shader":"main.glsl","output":"$screen","inputs":{"combined":"combined"}}]}).to_string());
    let mut r = renderer(root.path(), "Channels", json!({}), "normal", false).await;
    let p = draw(&mut r, &mut res, root.path(), "Channels", json!({}), "normal", 1.).await;
    assert!(p.iter().zip([64u8, 128, 0]).all(|(a, b)| a.abs_diff(b) <= 2), "component writes failed: {p:?}");
    write(
        root.path(),
        "shader/History/main.glsl",
        format!("{header}uniform sampler2D historyRT;void main(){{gl_FragColor=vec4(texture2D(historyRT,uv).rrr,1.0);}}"),
    );
    write(
        root.path(),
        "shader/History/add.glsl",
        "precision highp float;varying vec2 uv;uniform sampler2D previous;void main(){gl_FragColor=texture2D(previous,uv)+vec4(0.1,0.0,0.0,0.0);}",
    );
    write(root.path(),"shader/History/config.json",json!({"version":1,"padding":0,"targets":{"history":{"noClear":true}},"passes":[{"shader":"add.glsl","output":"history","inputs":{"previous":"history"}},{"shader":"main.glsl","output":"$screen","inputs":{"historyRT":"history"}}]}).to_string());
    let mut r = renderer(root.path(), "History", json!({}), "normal", false).await;
    for (time, want) in [(0., 26u8), (0.5, 52), (0.5, 52), (1., 78), (0., 26)] {
        let p = draw(&mut r, &mut res, root.path(), "History", json!({}), "normal", time).await;
        assert!(p[0].abs_diff(want) <= 3, "history/repaint/seek time={time}: {p:?} wanted {want}");
    }
    let upstream = std::env::args().nth(1).unwrap_or_else(|| "../rpe-next".into());
    let upstream = Path::new(&upstream).join("assets/noise-skins");
    for name in ["NoiseField", "Ocean"] {
        let directory = upstream.join(name);
        assert!(directory.exists(), "Pass the upstream rpe-next path for builtin skin checks");
        for entry in walkdir::WalkDir::new(&directory) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                write(
                    root.path(),
                    &format!("shader/{name}/{}", entry.path().strip_prefix(&directory).unwrap().display()),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
        for channel in ["normal", "low"] {
            let mut r = renderer(root.path(), name, json!({}), channel, false).await;
            for time in [1., 1.25] {
                let p = draw(&mut r, &mut res, root.path(), name, json!({}), channel, time).await;
                assert!(p.iter().any(|c| *c > 5), "builtin skin blank: {name} {channel}: {p:?}");
            }
            println!("BUILTIN PASS: {name} channel={channel}");
        }
    }
    println!("PASS: custom image/parameter, scaled RG component passes, persistent history/repaint/seek, and upstream NoiseField/Ocean normal/low channels");
}
