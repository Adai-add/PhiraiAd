//! GLES2 adaptation of the extracted BlockRender passes.
use super::{BlockArea, NoiseAreaState};
use crate::core::Resource;
use anyhow::{ensure, Result};
use macroquad::prelude::*;
use miniquad::{BlendFactor, BlendState, BlendValue, Equation, PipelineParams, UniformType};
use std::collections::{HashMap, HashSet};
const VERT: &str = r#"#version 100
attribute vec3 position; attribute vec2 texcoord; attribute vec4 color0;
uniform mat4 Model; uniform mat4 Projection;
varying vec2 uv; varying vec4 col;
void main(){gl_Position=Projection*Model*vec4(position,1.);uv=texcoord;col=color0/255.;}"#;
const SPRITE: &str = r#"#version 100
precision mediump float; varying vec2 uv; varying vec4 col; uniform sampler2D Texture;
void main(){gl_FragColor=texture2D(Texture,uv)*col;}"#;
const PACK_MASK: &str = r#"#version 100
precision highp float; varying vec2 uv; uniform sampler2D Active; uniform sampler2D Ready; uniform sampler2D Normal; uniform sampler2D Subtract;
void main(){gl_FragColor=vec4(texture2D(Active,uv).r,texture2D(Ready,uv).r,texture2D(Normal,uv).r,texture2D(Subtract,uv).g);}"#;
const PACK_EFFECT: &str = r#"#version 100
precision highp float; varying vec2 uv; uniform sampler2D Edge; uniform sampler2D Glow; uniform sampler2D Hover;
void main(){gl_FragColor=vec4(texture2D(Edge,uv).r,texture2D(Glow,uv).g,texture2D(Hover,uv).r,1.);}"#;
const PRECISE: &str = r#"#version 100
precision highp float; varying vec2 uv; uniform sampler2D Normal; uniform sampler2D Subtract; uniform vec2 SourceSize; uniform vec2 TargetSize;
void main(){float v=0.; for(int y=0;y<4;y++){for(int x=0;x<4;x++){vec2 p=uv+(vec2(float(x),float(y))/4.+0.125-0.5)/TargetSize;float a=texture2D(Normal,p).r;float b=texture2D(Subtract,p).r;v+=abs(a-(step(0.09,b)-step(0.12,b)));}}gl_FragColor=vec4(v/16.,0.,0.,1.);}"#;
// Ready/disabled composition has no displacement sampler. Keep its
// binding list distinct from the active composition shader.
fn compose_textures<T: Copy>(normal: T, subtract: T, displacement: Option<T>) -> ([(&'static str, T); 3], usize) {
    if let Some(displacement) = displacement {
        ([("_DisplaceMap", displacement), ("_NormalBlockRT", normal), ("_SubtractBlockRT", subtract)], 3)
    } else {
        ([("_DisabledNormalBlockRT", normal), ("_DisabledSubtractBlockRT", subtract), ("", normal)], 2)
    }
}
#[derive(Clone, Copy)]
struct ScreenQuad {
    points: [Vec2; 4],
    color: Color,
}
const TOUCH_UNIFORMS: [&str; 10] = [
    "_TouchPos0",
    "_TouchPos1",
    "_TouchPos2",
    "_TouchPos3",
    "_TouchPos4",
    "_TouchPos5",
    "_TouchPos6",
    "_TouchPos7",
    "_TouchPos8",
    "_TouchPos9",
];
struct Program {
    material: Material,
    uniforms: HashMap<String, UniformType>,
    textures: HashSet<String>,
}
impl Program {
    fn new(vert: &str, frag: &str, blend: Option<BlendState>) -> Result<Self> {
        let regex = regex::Regex::new(r"uniform\s+(?:(?:highp|mediump|lowp)\s+)?(float|int|vec2|vec3|vec4|sampler2D)\s+(\w+)\s*;")?;
        let mut uniforms = HashMap::new();
        let mut textures = HashSet::new();
        for caps in regex.captures_iter(&format!("{vert}\n{frag}")) {
            let n = caps[2].to_owned();
            if &caps[1] == "sampler2D" {
                if n != "Texture" {
                    textures.insert(n);
                }
                continue;
            }
            let ty = match &caps[1] {
                "float" => UniformType::Float1,
                "int" => UniformType::Int1,
                "vec2" => UniformType::Float2,
                "vec3" => UniformType::Float3,
                _ => UniformType::Float4,
            };
            uniforms.insert(n, ty);
        }
        let label = frag
            .lines()
            .find(|line| line.contains("uniform") && line.contains("sampler2D"))
            .unwrap_or("sprite");
        let started = std::time::Instant::now();
        tracing::info!(stage = "shader_begin", shader = label, "noise-area initialization");
        let material = load_material(
            vert,
            frag,
            MaterialParams {
                uniforms: uniforms.iter().map(|(n, t)| (n.clone(), *t)).collect(),
                textures: textures.iter().cloned().collect(),
                pipeline_params: PipelineParams {
                    color_blend: blend,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        tracing::info!(stage = "shader_ready", shader = label, elapsed_ms = started.elapsed().as_millis(), "noise-area initialization");
        Ok(Self {
            material,
            uniforms,
            textures,
        })
    }
    fn f(&self, n: &str, v: f32) {
        if self.uniforms.contains_key(n) {
            self.material.set_uniform(n, v)
        }
    }
    fn v2(&self, n: &str, v: Vec2) {
        if self.uniforms.contains_key(n) {
            self.material.set_uniform(n, v)
        }
    }
    fn v4(&self, n: &str, v: Vec4) {
        if self.uniforms.contains_key(n) {
            self.material.set_uniform(n, v)
        }
    }
    fn props(&self, values: &serde_json::Value) {
        if let Some(map) = values.as_object() {
            for (n, v) in map {
                if let Some(t) = self.uniforms.get(n) {
                    if let Some(f) = v.as_f64() {
                        self.f(n, f as f32)
                    } else if let Some(a) = v.as_array() {
                        let a: Vec<_> = a.iter().map(|v| v.as_f64().unwrap_or(0.) as f32).collect();
                        match t {
                            UniformType::Float2 => self.v2(n, vec2(a[0], a[1])),
                            UniformType::Float3 => self.material.set_uniform(n, vec3(a[0], a[1], a[2])),
                            UniformType::Float4 => self.v4(n, vec4(a[0], a[1], a[2], a[3])),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
impl Drop for Program {
    fn drop(&mut self) {
        self.material.delete();
    }
}
struct Targets {
    w: u32,
    h: u32,
    targets: Vec<RenderTarget>,
}
impl Targets {
    fn new(w: u32, h: u32, precise: bool) -> Self {
        let (bw, bh) = ((w / 8).max(1), (h / 8).max(1));
        let (ew, eh) = if precise { (w, h) } else { (bw * 2, bh * 2) };
        let ss = if w.max(h) * 4 <= 4096 && (w as u64 * h as u64) * 16 <= 8_388_608 {
            4
        } else if w.max(h) * 2 <= 4096 && (w as u64 * h as u64) * 4 <= 8_388_608 {
            2
        } else {
            1
        };
        // 0/1 active masks, 2/3 ready, 4/5 disabled, 6 subtract active,
        // 7 subtract ready, 8 compose active, 9 compose ready, 10 disabled,
        // 11 edge, 12/13 glow ping-pong, 14 hover, 15 packed masks,
        // 16 packed effects, 17 scene color, 18/19 supersample geometry.
        let sizes = [
            (bw, bh),
            (bw, bh),
            (bw, bh),
            (bw, bh),
            (bw, bh),
            (bw, bh),
            (bw, bh),
            (bw, bh),
            if precise { (w, h) } else { (bw, bh) },
            (bw, bh),
            (bw, bh),
            (ew, eh),
            (ew, eh),
            (ew, eh),
            (bw, bh),
            if precise { (w, h) } else { (bw, bh) },
            (ew, eh),
            ((w / 6).max(1), (h / 6).max(1)),
            if precise { (w * ss, h * ss) } else { (1, 1) },
            if precise { (w * ss, h * ss) } else { (1, 1) },
            (bw, bh),
        ];
        let targets = sizes
            .into_iter()
            .enumerate()
            .map(|(i, (x, y))| {
                let r = render_target(x, y);
                r.texture.set_filter(if i == 16 { FilterMode::Linear } else { FilterMode::Nearest });
                r
            })
            .collect();
        Self { w, h, targets }
    }
}
impl Drop for Targets {
    fn drop(&mut self) {
        let mut gl = unsafe { get_internal_gl() };
        gl.flush();
        for t in &self.targets {
            t.render_pass.delete(gl.quad_context);
        }
    }
}
pub struct NoiseRenderer {
    programs: Vec<Program>,
    sprite: Program,
    hover_sprite: Program,
    mask: Program,
    effect: Program,
    precise: Program,
    targets: Option<Targets>,
    precise_enabled: bool,
    noise: Texture2D,
    spark: Texture2D,
    grain: Texture2D,
    touch: Texture2D,
    blank: Texture2D,
    geometry: [Vec<ScreenQuad>; 6],
}
impl NoiseRenderer {
    pub async fn new() -> Result<Self> {
        let blend = Some(BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::OneMinusValue(BlendValue::SourceAlpha)));
        macro_rules! program {
            ($n:literal) => {{
                // LoadingScene polls once per visible frame; do not compile every
                // effect in one poll and leave the loading image unresponsive.
                let mut yielded = false;
                std::future::poll_fn(|cx| {
                    if std::mem::replace(&mut yielded, true) {
                        std::task::Poll::Ready(())
                    } else {
                        cx.waker().wake_by_ref();
                        std::task::Poll::Pending
                    }
                })
                .await;
                tracing::info!(stage = "program_begin", program = $n, "noise-area initialization");
                Program::new(include_str!(concat!("shaders/", $n, ".vert")), include_str!(concat!("shaders/", $n, ".glsl")), blend)?
            }};
        }
        tracing::info!(stage = "textures_begin", "noise-area initialization");
        let noise = Texture2D::from_file_with_format(include_bytes!("assets/texture_30.png"), Some(ImageFormat::Png));
        let spark = Texture2D::from_file_with_format(include_bytes!("assets/texture_17.png"), Some(ImageFormat::Png));
        let grain = Texture2D::from_file_with_format(include_bytes!("assets/texture_15.png"), Some(ImageFormat::Png));
        let touch = Texture2D::from_file_with_format(include_bytes!("assets/touch.png"), Some(ImageFormat::Png));
        for tex in [noise, spark, grain] {
            tex.set_filter(FilterMode::Linear);
            tex.raw_miniquad_texture_handle()
                .set_wrap(unsafe { get_internal_gl() }.quad_context, miniquad::TextureWrap::Repeat);
        }
        touch.set_filter(FilterMode::Linear);
        tracing::info!(stage = "textures_ready", "noise-area initialization");
        let renderer = Self {
            programs: vec![
                program!("SubtractBlockBlender_program_0"),
                program!("SubtractBlockBlender_program_1"),
                program!("BlockCompose_program_0"),
                program!("BlockCompose_program_1"),
                program!("EdgeMask_program_1"),
                program!("GlowMask_program_0"),
                // Unity DisabledBlock uses additive blending (One, One).
                // Premultiplied-over would erase the chart because its alpha is 1.
                Program::new(
                    include_str!("shaders/DisabledBlock_program_0.vert"),
                    include_str!("shaders/DisabledBlock_program_0.glsl"),
                    Some(BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::One)),
                )?,
                program!("ActiveBlock_program_0"),
            ],
            sprite: Program::new(VERT, SPRITE, Some(BlendState::new(Equation::Add, BlendFactor::Value(BlendValue::SourceAlpha), BlendFactor::One)))?,
            hover_sprite: Program::new(VERT, &SPRITE.replace("texture2D(Texture,uv)*col", "vec4(texture2D(Texture,uv).a) * col"), blend)?,
            mask: Program::new(VERT, PACK_MASK, None)?,
            effect: Program::new(VERT, PACK_EFFECT, None)?,
            precise: Program::new(VERT, PRECISE, None)?,
            targets: None,
            precise_enabled: false,
            noise,
            spark,
            grain,
            touch,
            blank: Texture2D::from_rgba8(1, 1, &[0, 0, 0, 0]),
            geometry: std::array::from_fn(|_| Vec::new()),
        };
        // Material properties do not vary during a play session.
        let props: serde_json::Value = serde_json::from_str(include_str!("assets/materials.json"))?;
        renderer.programs[2].props(&props["6"]);
        renderer.programs[3].props(&props["6"]);
        renderer.programs[6].props(&props["8"]);
        renderer.programs[7].props(&props["5"]);
        Ok(renderer)
    }
    fn camera(target: RenderTarget) {
        set_camera(&Camera2D {
            zoom: vec2(1., 1.),
            render_target: Some(target),
            ..Default::default()
        });
    }
    fn quad() {
        Self::mesh([vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)], WHITE, None);
    }
    fn pass(program: &Program, target: RenderTarget, textures: &[(&str, Texture2D)], clear: bool) -> Result<()> {
        for (name, _) in textures {
            ensure!(program.textures.contains(*name), "Noise-area shader has no texture sampler {name}");
        }
        Self::camera(target);
        if clear {
            clear_background(BLANK);
        }
        for (n, t) in textures {
            program.material.set_texture(n, *t)
        }
        gl_use_material(program.material);
        Self::quad();
        gl_use_default_material();
        unsafe { get_internal_gl() }.flush();
        Ok(())
    }
    fn screen_uv(p: Vec2, res: &Resource, vp: (i32, i32, i32, i32), w: f32, h: f32) -> Vec2 {
        let x = p.x / (5. * res.aspect_ratio) * if res.config.flip_x() { -1. } else { 1. };
        let y = -p.y / (5. * res.aspect_ratio);
        let (x, y) = res.practice_view.chart_to_screen(x, y, vp.2 as f32);
        vec2((vp.0 as f32 + (x + 1.) * 0.5 * vp.2 as f32) / w, (vp.1 as f32 + (1. - y * res.aspect_ratio) * 0.5 * vp.3 as f32) / h)
    }
    fn mesh(points: [Vec2; 4], color: Color, tex: Option<Texture2D>) {
        let uv = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)];
        let vertices = std::array::from_fn::<_, 4, _>(|i| macroquad::models::Vertex {
            position: vec3(points[i].x * 2. - 1., points[i].y * 2. - 1., 0.),
            uv: uv[i],
            color,
        });
        let gl = unsafe { get_internal_gl() }.quad_gl;
        gl.texture(tex);
        gl.draw_mode(DrawMode::Triangles);
        gl.geometry(&vertices, &[0, 1, 2, 0, 2, 3]);
    }

    pub fn render(&mut self, res: &mut Resource, areas: &[BlockArea], state: &NoiseAreaState, full_vp: (i32, i32, i32, i32)) -> Result<()> {
        if !res.config.noise_area.enabled {
            return Ok(());
        }
        if !areas.iter().any(|area| area.visible(res.time as f32)) && !state.hovers.iter().any(|hover| hover.scale > 0.) {
            return Ok(());
        }
        let Some(target) = res.chart_target.as_mut() else { return Ok(()) };
        unsafe { get_internal_gl() }.flush();
        target.swap();
        let source = target.old().texture;
        let output = target.output();
        let (w, h) = (source.width() as u32, source.height() as u32);
        let precise = res.config.noise_area.precise_edges;
        if self.targets.as_ref().is_none_or(|t| t.w != w || t.h != h) || self.precise_enabled != precise {
            tracing::info!(stage = "targets_begin", width = w, height = h, precise, "noise-area initialization");
            self.targets = Some(Targets::new(w, h, precise));
            tracing::info!(stage = "targets_ready", "noise-area initialization");
            self.precise_enabled = precise;
        }
        let rt = &self.targets.as_ref().unwrap().targets;
        let t = res.time as f32;
        let cv = res.camera.viewport.unwrap_or(full_vp);
        let vp = (cv.0 - full_vp.0, cv.1 - full_vp.1, cv.2, cv.3);
        // Resolve each visible area's event chains and screen transform once,
        // then reuse the same vertices for low-resolution / precise masks.
        for group in &mut self.geometry {
            group.clear();
        }
        for area in areas.iter().filter(|area| area.visible(t)) {
            let phase = if area.active(t) {
                0
            } else if area.ready(t) {
                2
            } else {
                4
            };
            let fade = if phase == 4 && t < area.enable_time {
                ((t - area.appear_time) / 0.5).clamp(0., 1.)
            } else {
                1.
            };
            self.geometry[phase + usize::from(area.is_subtract)].push(ScreenQuad {
                points: area
                    .pose(t, res.aspect_ratio)
                    .corners()
                    .map(|p| Self::screen_uv(p, res, vp, w as f32, h as f32)),
                color: Color::new(1., if area.is_subtract { fade } else { 1. }, 1., if area.is_subtract { 0.1 } else { fade }),
            });
        }
        let active = !self.geometry[0].is_empty() || !self.geometry[1].is_empty();
        let ready = !self.geometry[2].is_empty() || !self.geometry[3].is_empty();
        let disabled = !self.geometry[4].is_empty() || !self.geometry[5].is_empty();
        let hover_visible = state.hovers.iter().any(|hover| hover.scale > 0.);
        // Empty channels always sample a persistent zero texture. Never read an
        // old target after skipping its pass (phase changes, seeks, restarts).
        let mut tex = [self.blank; 21];
        push_camera_state();
        let result = (|| -> Result<()> {
            for i in [0, 1, 2, 3, 4, 5, 14, 18, 19] {
                let group = if i >= 18 { i - 18 } else { i };
                if i == 14 {
                    if !hover_visible {
                        continue;
                    }
                } else if (i >= 18 && !precise) || (precise && i < 2) || self.geometry[group].is_empty() {
                    continue;
                }
                tex[i] = rt[i].texture;
                Self::camera(rt[i]);
                clear_background(BLANK);
                gl_use_material(if i == 14 { self.hover_sprite.material } else { self.sprite.material });
                if i == 14 {
                    for hover in &state.hovers {
                        if hover.scale <= 0. {
                            continue;
                        }
                        let center = hover.position;
                        let local: [Vec2; 6] = [
                            vec2(-0.14000000059604645, 0.2199999988079071),
                            vec2(0.2199999988079071, -0.2199999988079071),
                            vec2(0.2199999988079071, 0.2199999988079071),
                            vec2(-0.1599999964237213, -0.2199999988079071),
                            vec2(-0.2199999988079071, 0.15000000596046448),
                            vec2(-0.2199999988079071, -0.10999999940395355),
                        ];
                        let vertices = std::array::from_fn::<_, 6, _>(|i| {
                            let p = local[i];
                            let pos = Self::screen_uv(center + p * hover.scale, res, vp, w as f32, h as f32);
                            macroquad::models::Vertex {
                                position: vec3(pos.x * 2. - 1., pos.y * 2. - 1., 0.),
                                uv: p / 0.44 + Vec2::splat(0.5),
                                color: WHITE,
                            }
                        });
                        let gl = unsafe { get_internal_gl() }.quad_gl;
                        gl.texture(Some(self.touch));
                        gl.draw_mode(DrawMode::Triangles);
                        gl.geometry(&vertices, &[5, 4, 3, 0, 3, 4, 1, 3, 0, 2, 1, 0]);
                    }
                } else {
                    for area in &self.geometry[group] {
                        Self::mesh(area.points, area.color, None);
                    }
                }
                gl_use_default_material();
                unsafe { get_internal_gl() }.flush();
            }
            for (p, target, input) in [(0, 6, 1), (1, 7, 3), (1, 10, 5)] {
                if self.geometry[input].is_empty() || (precise && target == 6) {
                    continue;
                }
                tex[target] = rt[target].texture;
                self.programs[p].f("_ClampThresholdLow", 0.09);
                self.programs[p].f("_ClampThresholdHigh", 0.12);
                Self::pass(&self.programs[p], rt[target], &[("_MainTex", tex[input])], true)?;
            }
            for (p, target, n, s) in [(2, 8, 0, 6), (3, 9, 2, 7)] {
                if (p == 2 && !active) || (p == 3 && !ready) || (p == 2 && precise) {
                    continue;
                }
                tex[target] = rt[target].texture;
                let pr = &self.programs[p];
                pr.v4("_BlockTime", vec4(t / 20., t, t * 2., t * 3.));
                let (bindings, len) = compose_textures(tex[n], tex[s], (p == 2).then_some(self.noise));
                Self::pass(pr, rt[target], &bindings[..len], true)?;
            }
            if precise && active {
                tex[8] = rt[8].texture;
                self.precise.v2("SourceSize", vec2(rt[18].texture.width(), rt[18].texture.height()));
                self.precise.v2("TargetSize", vec2(w as f32, h as f32));
                Self::pass(&self.precise, rt[8], &[("Normal", tex[18]), ("Subtract", tex[19])], true)?;
            }
            let ew = rt[11].texture.width();
            let eh = rt[11].texture.height();
            let dilation = if precise { 4. } else { 1. };
            let texel = vec4(dilation / ew, dilation / eh, ew, eh);
            let mut glow = 8;
            if active {
                tex[11] = rt[11].texture;
                self.programs[4].v4("_DilateTexelSize", texel);
                Self::pass(&self.programs[4], rt[11], &[("_MainTex", tex[8]), ("_ComposeRT", tex[8])], true)?;
                for (i, weight) in [0.45856798, 0.28286123, 0.15658922, 0.07305908, 0.02494781].into_iter().enumerate() {
                    let out = 12 + i % 2;
                    tex[out] = rt[out].texture;
                    let pr = &self.programs[5];
                    pr.v4("_DilateTexelSize", texel);
                    pr.f("_PassWeight", weight);
                    pr.f("_GlowFirstPass", if i == 0 { 1. } else { 0. });
                    Self::pass(pr, rt[out], &[("_MainTex", tex[glow]), ("_ComposeRT", tex[8])], true)?;
                    glow = out;
                }
            }
            if active || ready || hover_visible {
                Self::pass(&self.mask, rt[15], &[("Active", tex[8]), ("Ready", tex[9]), ("Normal", tex[2]), ("Subtract", tex[7])], true)?;
                Self::pass(&self.effect, rt[16], &[("Edge", tex[11]), ("Glow", tex[glow]), ("Hover", tex[14])], true)?;
            }
            if disabled {
                tex[20] = rt[20].texture;
                Self::pass(&self.programs[3], rt[20], &[("_DisabledNormalBlockRT", tex[4]), ("_DisabledSubtractBlockRT", tex[10])], true)?;
            }
            if (active || hover_visible) && !res.config.noise_area.remove_distortion {
                Self::camera(rt[17]);
                clear_background(BLANK);
                Self::mesh([vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)], WHITE, Some(source));
                unsafe { get_internal_gl() }.flush();
            }
            // Preserve the original scene outside the block overlay.
            Self::camera(output);
            clear_background(BLANK);
            Self::mesh([vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)], WHITE, Some(source));
            unsafe { get_internal_gl() }.flush();
            if disabled {
                let dp = &self.programs[6];
                dp.v4("_BlockTime", vec4(t / 20., t, t * 2., t * 3.));
                Self::pass(dp, output, &[("_ComposeRT", tex[20]), ("_DisplaceMap", self.noise), ("_SparkMap", self.spark)], false)?;
            }
            if active || ready || hover_visible {
                let pr = &self.programs[7];
                pr.v4("_BlockTime", vec4(t / 20., t, t * 2., t * 3.));
                pr.v4("_ScreenParams", vec4(w as f32, h as f32, 1. + 1. / w as f32, 1. + 1. / h as f32));
                pr.v4("_ProjectionParams", vec4(1., 0.1, 100., 0.01));
                pr.v4("_EffectRT_TexelSize", vec4(1. / ew, 1. / eh, ew, eh));
                pr.v2("_HoverSize", vec2(rt[14].texture.width(), rt[14].texture.height()));
                pr.f("_RemoveDistortion", if res.config.noise_area.remove_distortion { 1. } else { 0. });
                pr.f("_TouchPosShine", 1.);
                pr.material.set_uniform("_TouchPosCount", state.positions.len().min(10) as i32);
                for i in 0..10 {
                    let p = state
                        .positions
                        .get(i)
                        .map(|p| Self::screen_uv(*p, res, vp, w as f32, h as f32))
                        .unwrap_or(Vec2::ZERO);
                    pr.v2(TOUCH_UNIFORMS[i], vec2(p.x * w as f32 / h as f32, p.y));
                }
                Self::pass(
                    pr,
                    output,
                    &[
                        ("_MaskRT", rt[15].texture),
                        ("_EffectRT", rt[16].texture),
                        ("_DisplaceMap", self.noise),
                        ("_SparkMap", self.spark),
                        ("_NoiseMap", self.grain),
                        (
                            "_SceneColor",
                            if res.config.noise_area.remove_distortion || !(active || hover_visible) {
                                source
                            } else {
                                rt[17].texture
                            },
                        ),
                    ],
                    false,
                )?;
            }
            Ok(())
        })();
        pop_camera_state();
        let gl = unsafe { get_internal_gl() };
        gl.quad_gl.render_pass(Some(output.render_pass));
        gl.quad_gl.viewport(Some(vp));
        result
    }
}
impl Drop for NoiseRenderer {
    fn drop(&mut self) {
        for tex in [self.noise, self.spark, self.grain, self.touch, self.blank] {
            tex.delete();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compose_bindings_match_actual_shader_sampler_declarations() {
        let regex = regex::Regex::new(r"uniform\s+(?:(?:highp|mediump|lowp)\s+)?sampler2D\s+(\w+)\s*;").unwrap();
        for (shader, displacement) in [
            (include_str!("shaders/BlockCompose_program_0.glsl"), Some(3)),
            (include_str!("shaders/BlockCompose_program_1.glsl"), None),
        ] {
            let declared: HashSet<String> = regex.captures_iter(shader).map(|cap| cap[1].to_owned()).collect();
            let (bindings, len) = compose_textures(1, 2, displacement);
            let bound: HashSet<String> = bindings[..len].iter().map(|(name, _)| name.to_string()).collect();
            assert_eq!(bound, declared);
            assert!(displacement.is_some() || !bound.contains("_DisplaceMap"));
        }
    }
}
