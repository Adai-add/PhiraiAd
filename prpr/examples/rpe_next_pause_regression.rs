//! Production game HUD/pause path; the driver clicks each printed CLICK point.
//! This holds the chart at a fixed time, using real rendering/input and pause
//! handling, without starting music or advancing the judgement timeline.
use macroquad::prelude::*;
use prpr::{
    config::Config,
    core::PGR_FONT,
    dir::Dir,
    fs::ExternalFileSystem,
    info::{ChartFormat, ChartInfo},
    judge::Judge,
    scene::{GameMode, GameScene, Main, Scene},
    time::TimeManager,
    ui::{FontArc, TextPainter, Ui},
};
use serde_json::json;
use std::{cell::Cell, io::Write, rc::Rc, sync::Arc};

fn conf() -> Conf {
    Conf {
        window_width: 640,
        window_height: 360,
        window_title: "Next pause regression".into(),
        ..Default::default()
    }
}
struct Probe {
    game: GameScene,
    paused: Rc<Cell<bool>>,
    blocked: Rc<Cell<bool>>,
}
impl Scene for Probe {
    fn enter(&mut self, tm: &mut TimeManager, target: Option<RenderTarget>) -> anyhow::Result<()> {
        <GameScene as Scene>::enter(&mut self.game, tm, target)
    }
    fn update(&mut self, _: &mut TimeManager) -> anyhow::Result<()> {
        let viewport = self.game.res.camera.viewport.unwrap_or((0, 0, 640, 360));
        let aspect = self.game.res.aspect_ratio;
        let points: Vec<_> = Judge::replay_touches(viewport)
            .iter()
            .filter(|t| t.phase == TouchPhase::Started)
            .map(|t| (t.id, vec2(t.position.x * 5. * aspect, -t.position.y * 5. * aspect)))
            .collect();
        self.game
            .judge
            .noise_state
            .update(&self.game.chart.extra.block_areas, &points, 30., aspect, true, 0.016);
        if !points.is_empty() {
            self.blocked.set(!self.game.judge.noise_state.blocked_ids.is_empty());
        }
        Ok(())
    }
    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> anyhow::Result<()> {
        self.game.render_timing_preview(tm, ui)?;
        self.paused.set(tm.paused());
        Ok(())
    }
}

#[macroquad::main(conf)]
async fn main() {
    prpr::core::init_assets();
    let font = FontArc::try_from_vec(std::fs::read("assets/font.ttf").unwrap()).unwrap();
    let mut painter = TextPainter::new(font, None);
    PGR_FONT.with(|p| *p.borrow_mut() = Some(TextPainter::new(FontArc::try_from_vec(std::fs::read("assets/phigros.ttf").unwrap()).unwrap(), None)));
    let directory = tempfile::tempdir().unwrap();
    std::fs::copy("assets/click.ogg", directory.path().join("click.ogg")).unwrap();
    image::RgbaImage::from_pixel(16, 4, image::Rgba([200, 220, 255, 255]))
        .save(directory.path().join("custom-line.png"))
        .unwrap();
    let event = |start: f64| json!({"startTime":[0,0,1],"endTime":[200,0,1],"start":start,"end":start,"easingType":1});
    let note = json!({"type":1,"above":1,"startTime":[180,0,1],"endTime":[180,0,1],"positionX":0,"yOffset":0,"alpha":255,"size":1,"speed":1,"isFake":0,"visibleTime":999999});
    let line = |texture: &str| json!({"Name":"Regression line","Texture":texture,"isCover":1,"father":-1,"notes":[note.clone()],"eventLayers":[{"alphaEvents":[event(255.)],"moveXEvents":[event(0.)],"moveYEvents":[event(0.)],"rotateEvents":[event(12.)],"speedEvents":[event(11.)]}]});
    let source = json!({"META":{"RPEVersion":250,"offset":0},"BPMList":[{"bpm":120,"startTime":[0,0,1]}],
        "judgeLineList":[line("line.png"),line("custom-line.png")],"blockAreaList":[{"bottomLeft":{"x":-10000,"y":-10000},"topRight":{"x":10000,"y":10000},
        "appearTime":[0,0,1],"disappearTime":[200,0,1],"activeIntervals":[{"startTime":[0,0,1],"endTime":[200,0,1]}]}]});
    let skin_dir = directory.path().join("shader/test");
    std::fs::create_dir_all(&skin_dir).unwrap();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([30, 100, 180, 255]))
        .save(skin_dir.join("decal.png"))
        .unwrap();
    std::fs::write(skin_dir.join("main.glsl"),"precision highp float;varying vec2 uv;\nuniform float touchReleaseDuration; // %0.1%\nuniform sampler2D screenTexture;\nuniform sampler2D blockMask;\nuniform sampler2D decal; // @texture decal.png wrap=repeat filter=nearest\nvoid main(){vec4 original=texture2D(screenTexture,uv);gl_FragColor=texture2D(blockMask,uv).r>0.0?mix(original,texture2D(decal,uv),0.3):original;}").unwrap();
    let mut custom = source.clone();
    custom["noiseAreaLayers"] = json!([{"id":"custom","name":"Custom","skin":"shader/test"}]);
    custom["blockAreaList"][0]["layerId"] = json!("custom");
    for (case, (low, aspect, full, double)) in [
        (true, 4. / 3., (0, 0, 640, 360), false),
        (false, 4. / 3., (0, 0, 640, 360), false),
        (true, 8. / 3., (0, 0, 640, 360), false),
        (false, 1.5, (20, 10, 600, 320), false),
        (true, 4. / 3., (0, 0, 640, 360), true),
        (false, 4. / 3., (0, 0, 640, 360), false),
        (true, 4. / 3., (0, 0, 640, 360), false),
        (false, 4. / 3., (20, 10, 600, 320), false),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(directory.path().join("chart.json"), if case >= 6 { custom.to_string() } else { source.to_string() }).unwrap();
        let mut config = Config::default();
        config.noise_area.low_performance = low;
        config.double_click_to_pause = double;
        config.preferred_sample_rate = Some(48000);
        config.audio_buffer_size = Some(512);
        config.replay_recording_enabled = false;
        if case == 2 {
            config.sample_count = 4;
        }
        let texture = Texture2D::from_rgba8(1, 1, &[0, 0, 0, 255]);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut game = runtime
            .block_on(GameScene::new(
                if case == 5 || case == 7 { GameMode::Exercise } else { GameMode::Normal },
                ChartInfo {
                    chart: "chart.json".into(),
                    format: Some(ChartFormat::Rpe),
                    music: "click.ogg".into(),
                    aspect_ratio: aspect,
                    force_aspect_ratio: true,
                    ..Default::default()
                },
                config,
                Box::new(ExternalFileSystem(Arc::new(Dir::new(directory.path()).unwrap()))),
                None,
                texture.into(),
                texture.into(),
                None,
                None,
                None,
                None,
            ))
            .unwrap();
        if case == 5 || case == 7 {
            game.res.practice_view.scale_percent = 50.;
        }
        game.res.update_size(full);
        let vp = game.res.camera.viewport.unwrap();
        let center = vec2(-0.94, -1. / aspect + 0.07 / aspect + 0.024);
        let click = vec2(
            vp.0 as f32 + (center.x + 1.) * vp.2 as f32 / 2.,
            screen_height() - (vp.1 + vp.3) as f32 + (center.y + 1. / aspect) * vp.2 as f32 / 2.,
        );
        let paused = Rc::new(Cell::new(false));
        let blocked = Rc::new(Cell::new(false));
        let probe = Probe {
            game,
            paused: paused.clone(),
            blocked: blocked.clone(),
        };
        let mut main = Main::new(Box::new(probe), TimeManager::default(), None::<RenderTarget>).await.unwrap();
        main.viewport = Some(full);
        main.top_level = true;
        for _ in 0..3 {
            main.update().unwrap();
            main.render(&mut painter).unwrap();
            next_frame().await;
        }
        main.update().unwrap();
        main.render(&mut painter).unwrap();
        let image = get_screen_data();
        let x = (click.x - 0.015 * vp.2 as f32 / 2.).round() as usize;
        let y = image.height as usize - 1 - click.y.round() as usize;
        let color = &image.bytes[(y * image.width as usize + x) * 4..][..3];
        assert!(color.iter().all(|v| *v > 190), "pause bars rendered at the wrong position/scale: case={case}, color={color:?}");
        println!("CLICK {} {} {}", click.x, click.y, if double { 2 } else { 1 });
        std::io::stdout().flush().unwrap();
        let start = get_time();
        while !paused.get() && get_time() - start < 2. {
            main.update().unwrap();
            main.render(&mut painter).unwrap();
            next_frame().await;
        }
        assert!(blocked.get(), "fixture did not cover the pause contact with active noise");
        assert!(paused.get(), "pause did not respond: case={case}, low={low}, aspect={aspect}, viewport={full:?}, double={double}");
        println!("PAUSED case={case}, low={low}, aspect={aspect}, viewport={full:?}, double={double}");
        // Let the driver release its last physical click before the next scene.
        let until = get_time() + 0.2;
        while get_time() < until {
            next_frame().await;
        }
        set_default_camera();
    }
    println!(
        "PASS: 8 production HUD pixels and pause button cases under active Next noise, including letterboxing, offset viewport and double click"
    );
}
