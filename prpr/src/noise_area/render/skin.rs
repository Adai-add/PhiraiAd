//! Bind RPE-Next skin pass descriptions to the existing Program/pass/mesh path.
//! There is no separate Next renderer, visual algorithm or GPU mask counter.
use super::*;
use crate::noise_area::skin::{Channel, Definition, Target};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Frame {
    size: (u32, u32),
    logical: Vec2,
    pad: Vec2,
}
impl Frame {
    fn new(logical: Vec2, padding: Vec2) -> Result<Self> {
        let pad = padding.ceil();
        let size = logical + pad * 2.;
        ensure!(logical.min_element() >= 1. && size.max_element() <= 8192., "Skin target dimensions exceed GLES texture limit");
        Ok(Self {
            size: (size.x as u32, size.y as u32),
            logical,
            pad,
        })
    }
    fn rect(self) -> Vec4 {
        let size = vec2(self.size.0 as f32, self.size.1 as f32);
        let p = self.pad / size;
        let s = self.logical / size;
        vec4(p.x, p.y, s.x, s.y)
    }
    fn uv(self, p: Vec2) -> Vec2 {
        (p * self.logical + self.pad) / vec2(self.size.0 as f32, self.size.1 as f32)
    }
}
struct Buffer {
    frame: Frame,
    slots: [RenderTarget; 2],
    before: Option<RenderTarget>,
    front: usize,
    format: String,
}
impl Buffer {
    fn new(frame: Frame, spec: &Target) -> Self {
        let allocate = || {
            let t = render_target(frame.size.0, frame.size.1);
            t.texture.set_filter(if spec.filter == "linear" {
                FilterMode::Linear
            } else {
                FilterMode::Nearest
            });
            t
        };
        Self {
            frame,
            slots: [allocate(), allocate()],
            before: spec.no_clear.then(allocate),
            front: 0,
            format: spec.format.clone(),
        }
    }
    fn current(&self) -> RenderTarget {
        self.slots[self.front]
    }
    fn next(&self) -> RenderTarget {
        self.slots[self.front ^ 1]
    }
    fn clear(&self) {
        EffectRenderer::camera(self.current());
        clear_background(if self.format == "rgba8" { BLANK } else { BLACK });
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { get_internal_gl() }.flush();
        for t in self.slots.iter().copied().chain(self.before) {
            t.delete();
        }
    }
}
struct BoundPass {
    description: crate::noise_area::skin::Pass,
    program: Program,
    images: Vec<(String, Texture2D)>,
}
struct Resources {
    dimensions: (u32, u32),
    screen: Buffer,
    mask: Buffer,
    targets: BTreeMap<String, Buffer>,
    last_time: Option<f64>,
}
pub(super) struct Bindings {
    pub layer: usize,
    definition: Definition,
    channel: Channel,
    passes: Vec<BoundPass>,
    resources: Option<Resources>,
}
impl Bindings {
    pub fn new(definition: &Definition, low: bool) -> Result<Self> {
        let channel = if definition.channel_low || low {
            definition.low.as_ref().unwrap_or(&definition.normal)
        } else {
            &definition.normal
        }
        .clone();
        let mut passes = Vec::new();
        for pass in &channel.passes {
            let source = adapt_source(&definition.sources[&pass.shader])?;
            let (output, component) = pass.output.split_once('.').map_or((pass.output.as_str(), None), |(a, b)| (a, Some(b)));
            let format = channel.targets.get(output).map(|t| t.format.as_str()).unwrap_or("rgba8");
            let available = match format {
                "r8" => "r",
                "rg8" => "rg",
                _ => "rgba",
            };
            let write = std::array::from_fn::<_, 4, _>(|i| {
                available.contains("rgba".chars().nth(i).unwrap()) && component.is_none_or(|c| c == &"rgba"[i..i + 1])
            });
            let write = (write[0], write[1], write[2], write[3]);
            let program = Program::with_color_write(VERT, &source, None, write).with_context(|| format!("{} / {}", definition.root, pass.shader))?;
            let mut images = Vec::new();
            for image in &definition.images[&pass.shader] {
                let texture = Texture2D::from_rgba8(image.width as u16, image.height as u16, &image.rgba);
                texture.set_filter(if image.filter == "nearest" {
                    FilterMode::Nearest
                } else {
                    FilterMode::Linear
                });
                texture.raw_miniquad_texture_handle().set_wrap(
                    unsafe { get_internal_gl() }.quad_context,
                    match image.wrap.as_str() {
                        "repeat" => miniquad::TextureWrap::Repeat,
                        "mirror" => miniquad::TextureWrap::Mirror,
                        _ => miniquad::TextureWrap::Clamp,
                    },
                );
                images.push((image.name.clone(), texture));
            }
            passes.push(BoundPass {
                description: pass.clone(),
                program,
                images,
            });
        }
        Ok(Self {
            layer: definition.layer,
            definition: definition.clone(),
            channel,
            passes,
            resources: None,
        })
    }
    pub fn prepare(&mut self, size: (u32, u32)) -> Result<()> {
        if self.resources.as_ref().is_some_and(|r| r.dimensions == size) {
            return Ok(());
        }
        let logical = vec2(size.0 as f32, size.1 as f32);
        let pad = crate::noise_area::skin::padding(self.channel.padding.as_ref(), size.0, size.1)?;
        let pad = vec2(pad.0 as f32, pad.1 as f32);
        let frame = |spec: &Target, pad: Vec2| {
            Frame::new((logical * vec2(spec.scale_x as f32, spec.scale_y as f32)).floor(), pad * vec2(spec.scale_x as f32, spec.scale_y as f32))
        };
        let screen_spec = Target {
            filter: "linear".into(),
            ..Default::default()
        };
        let screen_pad = if self.channel.padding.as_ref().and_then(|v| v.get("screen")).and_then(Value::as_bool) == Some(false) {
            Vec2::ZERO
        } else {
            pad
        };
        let screen = Buffer::new(frame(&screen_spec, screen_pad)?, &screen_spec);
        let mask_spec = Target {
            format: "r8".into(),
            ..self.channel.block_mask.clone()
        };
        let mask = Buffer::new(frame(&mask_spec, pad)?, &mask_spec);
        let mut targets = BTreeMap::new();
        for (name, spec) in &self.channel.targets {
            targets.insert(name.clone(), Buffer::new(frame(spec, pad)?, spec));
        }
        for buffer in targets.values().chain([&screen, &mask]) {
            buffer.clear();
        }
        self.resources = Some(Resources {
            dimensions: size,
            screen,
            mask,
            targets,
            last_time: None,
        });
        Ok(())
    }
}
impl Drop for Bindings {
    fn drop(&mut self) {
        unsafe { get_internal_gl() }.flush();
        for pass in &self.passes {
            for (_, texture) in &pass.images {
                texture.delete();
            }
        }
    }
}

fn set_value(program: &Program, name: &str, value: &Value) {
    if let Some(n) = value.as_f64() {
        program.f(name, n as f32);
    } else if let Some(v) = value.as_array() {
        let n = |i: usize| v[i].as_f64().unwrap_or(0.) as f32;
        if v.len() == 2 {
            program.v2(name, vec2(n(0), n(1)));
        } else if v.len() == 4 {
            program.v4(name, vec4(n(0), n(1), n(2), n(3)));
        }
    }
}
fn bind(program: &Program, name: &str, texture: Texture2D, frame: Frame) {
    // Self::pass validates caller bindings; the protocol has dynamic names, so
    // bind these through the exact same material API before that shared pass.
    if program.textures.contains(name) {
        program.material.set_texture(name, texture);
    }
    program.v2(&format!("{name}Size"), frame.logical);
    program.v4(&format!("phiraRect_{name}"), frame.rect());
}
fn copy(renderer: &EffectRenderer, source: RenderTarget, source_rect: Vec4, target: RenderTarget, output_frame: Frame) -> Result<()> {
    EffectRenderer::camera(target);
    clear_background(BLANK);
    gl_use_material(renderer.copy_sprite.material);
    // Reuse the existing mesh/texture path. UVs are specified explicitly to
    // preserve framebuffer orientation and crop/pad without a copy shader.
    let rect = output_frame.rect();
    let p0 = rect.truncate().truncate();
    let span = vec2(rect.z, rect.w);
    let points = [p0, p0 + vec2(span.x, 0.), p0 + span, p0 + vec2(0., span.y)];
    let u = vec2(source_rect.x, source_rect.y);
    let uv = vec2(source_rect.z, source_rect.w);
    let coords = [u, u + vec2(uv.x, 0.), u + uv, u + vec2(0., uv.y)];
    let vertices = std::array::from_fn::<_, 4, _>(|i| macroquad::models::Vertex {
        position: vec3(points[i].x * 2. - 1., points[i].y * 2. - 1., 0.),
        uv: coords[i],
        color: WHITE,
    });
    let gl = unsafe { get_internal_gl() }.quad_gl;
    gl.texture(Some(source.texture));
    gl.draw_mode(DrawMode::Triangles);
    gl.geometry(&vertices, &[0, 1, 2, 0, 2, 3]);
    unsafe { get_internal_gl() }.flush();
    let _ = renderer;
    Ok(())
}

fn duplicate(renderer: &EffectRenderer, source: RenderTarget, target: RenderTarget) -> Result<()> {
    copy(renderer, source, vec4(0., 0., 1., 1.), target, Frame::new(vec2(target.texture.width(), target.texture.height()), Vec2::ZERO)?)
}

impl EffectRenderer {
    pub(super) fn skin_passes(
        &mut self,
        bindings: &mut Bindings,
        res: &mut Resource,
        areas: &[BlockArea],
        state: &NoiseAreaState,
        full_vp: (i32, i32, i32, i32),
        projection: Mat4,
    ) -> Result<()> {
        let Some(target) = res.chart_target.as_ref() else {
            return Ok(());
        };
        let source = target.output();
        let output = self.overlay_output.unwrap_or(source);
        let cv = res.camera.viewport.unwrap_or(full_vp);
        let vp = (cv.0 - full_vp.0, cv.1 - full_vp.1, cv.2, cv.3);
        let size = (vp.2.max(1) as u32, vp.3.max(1) as u32);
        let saved_viewport = unsafe { get_internal_gl() }.quad_gl.get_viewport();
        push_camera_state();
        let result = (|| -> Result<()> {
            bindings.prepare(size)?;
            let resources = bindings.resources.as_mut().unwrap();
            let rect = vec4(
                vp.0 as f32 / source.texture.width(),
                vp.1 as f32 / source.texture.height(),
                vp.2 as f32 / source.texture.width(),
                vp.3 as f32 / source.texture.height(),
            );
            copy(self, source, rect, resources.screen.current(), resources.screen.frame)?;
            let repeated = resources.last_time == Some(res.time);
            let seek = resources.last_time.is_none_or(|t| res.time < t || (res.time - t).abs() > 1.);
            for (name, buffer) in &resources.targets {
                if let Some(before) = buffer.before {
                    if seek {
                        buffer.clear();
                    } else if repeated {
                        duplicate(self, before, buffer.current())?;
                    }
                    if !repeated {
                        duplicate(self, buffer.current(), before)?;
                    }
                } else if !bindings.channel.targets[name].no_clear {
                    buffer.clear();
                }
            }
            EffectRenderer::camera(resources.mask.current());
            clear_background(BLACK);
            gl_use_default_material();
            let project = |p: Vec2| {
                let uv = Self::screen_uv(p, res, vp, source.texture.width(), source.texture.height(), projection);
                let pixel = uv * vec2(source.texture.width(), source.texture.height()) - vec2(vp.0 as f32, vp.1 as f32);
                resources.mask.frame.uv(pixel / vec2(size.0 as f32, size.1 as f32))
            };
            for (polygon, word) in mask_words(areas, res.time as f32, res.aspect_ratio, project) {
                for i in 1..polygon.len() - 1 {
                    Self::mesh([polygon[0], polygon[i], polygon[i + 1], polygon[i + 1]], Color::new(word as f32 / 255., 0., 0., 1.), None);
                }
            }
            unsafe { get_internal_gl() }.flush();
            for pass in &bindings.passes {
                let (name, _) = pass
                    .description
                    .output
                    .split_once('.')
                    .map_or((pass.description.output.as_str(), None), |(a, b)| (a, Some(b)));
                let buffer = if name == "$screen" {
                    &resources.screen
                } else {
                    &resources.targets[name]
                };
                let target = buffer.next();
                let frame = buffer.frame;
                // Preserve channels when a pass writes target.r/target.g.
                duplicate(self, buffer.current(), target)?;
                bind(&pass.program, "screenTexture", resources.screen.current().texture, resources.screen.frame);
                bind(&pass.program, "blockMask", resources.mask.current().texture, resources.mask.frame);
                for (uniform, input) in &pass.description.inputs {
                    let input = if input == "$screen" {
                        &resources.screen
                    } else {
                        &resources.targets[input]
                    };
                    bind(&pass.program, uniform, input.current().texture, input.frame);
                }
                for (name, image) in &pass.images {
                    let frame = Frame::new(vec2(image.width(), image.height()), Vec2::ZERO)?;
                    bind(&pass.program, name, *image, frame);
                }
                pass.program.v2("screenSize", vec2(size.0 as f32, size.1 as f32));
                pass.program.v2("targetSize", frame.logical);
                pass.program.v2("targetTexelSize", Vec2::ONE / frame.logical);
                pass.program.v2("phiraOrigin", frame.pad);
                pass.program.v2("phiraLogicalSize", frame.logical);
                pass.program.f("time", res.time as f32);
                for (name, value) in &bindings.definition.parameters {
                    set_value(&pass.program, name, value);
                }
                for (name, value) in &pass.description.constants {
                    set_value(&pass.program, name, value);
                }
                let mut count = 0i32;
                for (i, hover) in state.hovers.iter().filter(|h| h.scale > 0.).take(32).enumerate() {
                    let uv = project(hover.position);
                    let value = vec4(uv.x, uv.y, (hover.scale / 11.5).clamp(0., 1.), 1.);
                    pass.program.v4(&format!("phiraTouch{i}"), value);
                    count += 1;
                }
                for i in count..32 {
                    pass.program.v4(&format!("phiraTouch{i}"), Vec4::ZERO);
                }
                if pass.program.uniforms.contains_key("touchCount") {
                    pass.program.material.set_uniform("touchCount", count);
                }
                Self::pass(&pass.program, target, &[], false, &mut self.profiler, 11, None)?;
                if name == "$screen" {
                    resources.screen.front ^= 1;
                } else {
                    resources.targets.get_mut(name).unwrap().front ^= 1;
                }
            }
            // Copy the configured skin's final logical image back into the
            // chart viewport through the same textured mesh primitive.
            Self::camera(output);
            gl_use_material(self.copy_sprite.material);
            let frame = resources.screen.frame;
            let texture = resources.screen.current().texture;
            let full = vec2(output.texture.width(), output.texture.height());
            let origin = vec2(vp.0 as f32, vp.1 as f32) / full;
            let span = vec2(vp.2 as f32, vp.3 as f32) / full;
            let points = [origin, origin + vec2(span.x, 0.), origin + span, origin + vec2(0., span.y)];
            let rect = frame.rect();
            let uv = vec2(rect.x, rect.y);
            let duv = vec2(rect.z, rect.w);
            let coords = [uv, uv + vec2(duv.x, 0.), uv + duv, uv + vec2(0., duv.y)];
            let vertices = std::array::from_fn::<_, 4, _>(|i| macroquad::models::Vertex {
                position: vec3(points[i].x * 2. - 1., points[i].y * 2. - 1., 0.),
                uv: coords[i],
                color: WHITE,
            });
            let gl = unsafe { get_internal_gl() }.quad_gl;
            gl.texture(Some(texture));
            gl.draw_mode(DrawMode::Triangles);
            gl.geometry(&vertices, &[0, 1, 2, 0, 2, 3]);
            unsafe { get_internal_gl() }.flush();
            resources.last_time = Some(res.time);
            Ok(())
        })();
        gl_use_default_material();
        pop_camera_state();
        let gl = unsafe { get_internal_gl() };
        gl.quad_gl.render_pass(Some(output.render_pass));
        gl.quad_gl.viewport(saved_viewport);
        result
    }
}

fn mask_words(areas: &[BlockArea], t: f32, aspect: f32, project: impl Fn(Vec2) -> Vec2) -> Vec<(Vec<Vec2>, u8)> {
    let regions: Vec<_> = areas
        .iter()
        .filter(|a| a.visible(t))
        .map(|a| {
            let p = a.pose(t, aspect).corners().map(&project);
            let phase = if a.active(t) {
                0
            } else if a.ready(t) {
                2
            } else {
                1
            };
            (p, phase, a.is_subtract)
        })
        .filter(|(p, _, _)| p.iter().all(|p| p.is_finite()) && polygon_area(p).abs() > 1e-10)
        .collect();
    // Opaque mask writes already form a union for same-phase normal areas;
    // reuse those polygons directly instead of subdividing a dense grid.
    if let Some((_, phase, _)) = regions.first() {
        if regions.iter().all(|(_, p, invert)| p == phase && !*invert) {
            let word = match phase {
                0 => 1,
                2 => 20,
                _ => 4,
            };
            return regions.iter().map(|(p, _, _)| (p.to_vec(), word)).collect();
        }
    }
    let screen = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)];
    let mut cells = vec![screen.to_vec()];
    for (p, _, _) in &regions {
        cells = cells
            .into_iter()
            .flat_map(|cell| {
                let inner = intersection(cell.clone(), p);
                if inner.is_empty() {
                    vec![cell]
                } else {
                    let mut parts = subtract_polygon(cell, p);
                    parts.push(inner);
                    parts
                }
            })
            .filter(|p| p.len() >= 3 && polygon_area(p).abs() > 1e-10)
            .collect();
    }
    cells
        .into_iter()
        .filter_map(|cell| {
            let center = cell.iter().copied().sum::<Vec2>() / cell.len() as f32;
            let mut normal = [false; 3];
            let mut inverse = [0usize; 3];
            for (p, phase, invert) in &regions {
                if inside_convex(center, p) {
                    let mut apply = |group: usize| if *invert { inverse[group] += 1 } else { normal[group] = true };
                    if *phase == 0 {
                        apply(0)
                    } else {
                        apply(1);
                        if *phase == 2 {
                            apply(2);
                        }
                    }
                }
            }
            let word = (0..3).fold(0u8, |word, i| word | ((normal[i] as u8) << (i * 2)) | (((inverse[i] == 1) as u8) << (i * 2 + 1)));
            (word != 0).then_some((cell, word))
        })
        .collect()
}

fn adapt_source(source: &str) -> Result<String> {
    let mut source = regex::Regex::new(r"(?m)^\s*#version[^\n]*")?.replace_all(source, "").into_owned();
    source = regex::Regex::new(r"\buniform\s+vec4\s+touchPoints\s*\[\s*32\s*\]\s*;")?
        .replace_all(&source, {
            let mut text = (0..32).map(|i| format!("uniform vec4 phiraTouch{i};\n")).collect::<String>();
            text.push_str("vec4 phiraTouchAt(int i){");
            for i in 0..32 {
                text.push_str(&format!("if(i=={i})return phiraTouch{i};"));
            }
            text.push_str("return vec4(0.0);}");
            text
        })
        .into_owned();
    source = regex::Regex::new(r"\btouchPoints\s*\[([^\]]+)\]")?
        .replace_all(&source, "phiraTouchAt($1)")
        .into_owned();
    let samplers: Vec<_> = regex::Regex::new(r"uniform\s+(?:(?:highp|mediump|lowp)\s+)?sampler2D\s+(\w+)\s*;")?
        .captures_iter(&source)
        .map(|c| c[1].to_owned())
        .collect();
    let mut declarations=String::from("precision highp float;\nuniform vec2 phiraOrigin;uniform vec2 phiraLogicalSize;\nvec4 phiraSample(sampler2D s,vec4 r,vec2 p){return texture2D(s,r.xy+p*r.zw);}\n");
    for name in samplers {
        declarations.push_str(&format!("uniform vec4 phiraRect_{name};\n"));
        let re = regex::Regex::new(&format!(r"\btexture2D\s*\(\s*{}\s*,", regex::escape(&name)))?;
        source = re.replace_all(&source, format!("phiraSample({name},phiraRect_{name},")).into_owned();
    }
    let uv = regex::Regex::new(r"\bvarying\s+(?:(?:lowp|mediump|highp)\s+)?vec2\s+uv\s*;")?;
    if !uv.is_match(&source) {
        declarations.push_str("vec2 uv;\n");
    }
    source = uv.replace(&source, "vec2 uv;").into_owned();
    source = regex::Regex::new(r"\bvoid\s+main\s*\(\s*(?:void\s*)?\)")?
        .replace(&source, "void phiraSkinMain()")
        .into_owned();
    Ok(format!("#version 100\n{declarations}{source}\nvoid main(){{uv=(gl_FragCoord.xy-phiraOrigin)/phiraLogicalSize;phiraSkinMain();}}"))
}
