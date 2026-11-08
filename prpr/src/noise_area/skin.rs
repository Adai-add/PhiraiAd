//! RPE-Next skin resource/config adapter. No GPU rendering lives here.
use crate::fs::FileSystem;
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Target {
    pub scale_x: f64,
    pub scale_y: f64,
    pub format: String,
    pub filter: String,
    pub no_clear: bool,
}
impl Default for Target {
    fn default() -> Self {
        Self {
            scale_x: 1.,
            scale_y: 1.,
            format: "rgba8".into(),
            filter: "nearest".into(),
            no_clear: false,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Pass {
    pub shader: String,
    pub output: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub constants: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Channel {
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    pub passes: Vec<Pass>,
    #[serde(default, rename = "blockMask")]
    pub block_mask: Target,
    #[serde(default)]
    pub padding: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct Image {
    pub name: String,
    pub path: String,
    pub wrap: String,
    pub filter: String,
    pub srgb: bool,
    pub premult: bool,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct Definition {
    pub layer: usize,
    pub root: String,
    pub normal: Channel,
    pub low: Option<Channel>,
    pub channel_low: bool,
    pub parameters: BTreeMap<String, Value>,
    pub sources: BTreeMap<String, String>,
    pub images: BTreeMap<String, Vec<Image>>,
}
pub fn safe_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.chars().any(|c| c < ' ' || "\\:#?".contains(c))
            && path.split('/').all(|p| !p.is_empty() && p != "." && p != ".."),
        "Invalid RPE-Next resource path: {path}"
    );
    Ok(())
}
fn parameter_type(value: &Value) -> Option<&'static str> {
    if value.as_f64().is_some_and(f64::is_finite) {
        return Some("float");
    }
    let values = value.as_array()?;
    if !values.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) {
        return None;
    }
    match values.len() {
        2 => Some("vec2"),
        4 => Some("vec4"),
        _ => None,
    }
}
fn validate_target(t: &Target) -> Result<()> {
    ensure!(t.scale_x.is_finite() && t.scale_y.is_finite() && t.scale_x > 0. && t.scale_y > 0., "Invalid skin target scale");
    ensure!(["r8", "rg8", "rgba8"].contains(&t.format.as_str()), "Invalid skin target format");
    ensure!(["nearest", "linear"].contains(&t.filter.as_str()), "Invalid skin target filter");
    Ok(())
}
pub fn padding(spec: Option<&Value>, w: u32, h: u32) -> Result<(u32, u32)> {
    let fallback = json!(32);
    let v = spec.unwrap_or(&fallback);
    let (pixels, x, y) = if let Some(n) = v.as_f64() {
        (n, 0., 0.)
    } else {
        ensure!(v.is_object(), "Invalid skin padding");
        for k in v.as_object().unwrap().keys() {
            ensure!(["pixels", "screenWidth", "screenHeight", "screen", "geometryOnly"].contains(&k.as_str()), "Unknown skin padding key");
        }
        (
            v.get("pixels").and_then(Value::as_f64).unwrap_or(0.),
            v.get("screenWidth").and_then(Value::as_f64).unwrap_or(0.),
            v.get("screenHeight").and_then(Value::as_f64).unwrap_or(0.),
        )
    };
    ensure!(
        pixels.is_finite() && (0. ..=2048.).contains(&pixels) && x.is_finite() && (0. ..=1.).contains(&x) && y.is_finite() && (0. ..=1.).contains(&y),
        "Invalid skin padding bounds"
    );
    Ok(((pixels + w as f64 * x).ceil() as u32, (pixels + h as f64 * y).ceil() as u32))
}
impl Definition {
    pub async fn load(layer: usize, settings: &Value, fs: &mut dyn FileSystem) -> Result<Self> {
        let root = settings["skin"].as_str().context("Missing skin root")?.to_owned();
        safe_path(&root)?;
        ensure!(root.starts_with("shader/") && root.split('/').count() == 2, "Skin root must be shader/NAME");
        let main = String::from_utf8(fs.load_file(&format!("{root}/main.glsl")).await?).context("Skin main.glsl is not UTF-8")?;
        let config = if fs.exists(&format!("{root}/config.json")).await? {
            serde_json::from_slice::<Value>(&fs.load_file(&format!("{root}/config.json")).await?)?
        } else {
            json!({"version":1,"targets":{},"passes":[{"shader":"main.glsl","output":"$screen"}]})
        };
        ensure!(config["version"] == 1, "Unsupported RPE-Next skin config version");
        let mut normal: Channel = serde_json::from_value(config.clone())?;
        let mut low = config
            .get("lowPerformance")
            .map(|v| serde_json::from_value::<Channel>(v.clone()))
            .transpose()?;
        if normal.padding.is_none() {
            normal.padding = config.get("padding").cloned();
        }
        if let Some(c) = &mut low {
            if c.padding.is_none() {
                c.padding = normal.padding.clone();
            }
        }
        let mut sources = BTreeMap::from([("main.glsl".into(), main.clone())]);
        let defaults = regex::Regex::new(r"uniform\s+(float|vec2|vec4)\s+(\w+)\s*;[^\r\n]*?//\s*%([^%\r\n]+)%")?;
        let mut parameters = BTreeMap::new();
        for c in defaults.captures_iter(&main) {
            let values = c[3]
                .split(',')
                .map(|n| n.trim().parse::<f64>())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let value = if &c[1] == "float" {
                ensure!(values.len() == 1, "Invalid scalar skin parameter");
                json!(values[0])
            } else {
                json!(values)
            };
            ensure!(parameter_type(&value) == Some(&c[1]), "Invalid skin parameter {}", &c[2]);
            ensure!(parameters.insert(c[2].into(), value).is_none(), "Duplicate skin parameter");
        }
        ensure!(
            parameters.get("touchReleaseDuration").and_then(Value::as_f64).is_some_and(|v| v >= 0.),
            "Skin must declare nonnegative touchReleaseDuration"
        );
        if let Some(overrides) = settings.get("parameters") {
            for (name, value) in overrides.as_object().context("Invalid skin parameters")? {
                let default = parameters.get(name).context("Unknown skin parameter override")?;
                ensure!(parameter_type(default) == parameter_type(value), "Skin parameter type mismatch: {name}");
                ensure!(name != "touchReleaseDuration" || value.as_f64().is_some_and(|v| v >= 0.), "Negative touch release duration");
                parameters.insert(name.clone(), value.clone());
            }
        }
        let image_re = regex::Regex::new(r"uniform\s+sampler2D\s+(\w+)\s*;\s*//\s*@texture\s+(\S+)([^\r\n]*)")?;
        let uniform_re = regex::Regex::new(r"uniform\s+(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(\w+)")?;
        let mut images = BTreeMap::new();
        for channel in std::iter::once(&normal).chain(low.iter()) {
            ensure!(!channel.passes.is_empty(), "Empty skin pass list");
            validate_target(&channel.block_mask)?;
            padding(channel.padding.as_ref(), 640, 360)?;
            for (name, target) in &channel.targets {
                ensure!(name != "$screen" && !name.contains('.'), "Invalid skin target name");
                validate_target(target)?;
            }
            let mut written = BTreeSet::new();
            for pass in &channel.passes {
                safe_path(&pass.shader)?;
                ensure!(pass.shader.ends_with(".glsl"), "Skin pass must be GLSL");
                if !sources.contains_key(&pass.shader) {
                    sources.insert(pass.shader.clone(), String::from_utf8(fs.load_file(&format!("{root}/{}", pass.shader)).await?)?);
                }
                let source = &sources[&pass.shader];
                let uniforms: BTreeMap<_, _> = uniform_re.captures_iter(source).map(|c| (c[2].to_owned(), c[1].to_owned())).collect();
                if !images.contains_key(&pass.shader) {
                    let mut loaded = Vec::new();
                    for c in image_re.captures_iter(source) {
                        safe_path(&c[2])?;
                        let directory = pass.shader.rsplit_once('/').map_or("", |(v, _)| v);
                        let path = if directory.is_empty() {
                            c[2].to_owned()
                        } else {
                            format!("{directory}/{}", &c[2])
                        };
                        safe_path(&path)?;
                        let mut image = Image {
                            name: c[1].into(),
                            path: path.clone(),
                            wrap: "clamp".into(),
                            filter: "linear".into(),
                            srgb: false,
                            premult: false,
                            rgba: Vec::new(),
                            width: 0,
                            height: 0,
                        };
                        for option in c[3].split_whitespace() {
                            let (key, value) = option.split_once('=').context("Invalid skin texture option")?;
                            match key {
                                "wrap" => {
                                    ensure!(["clamp", "repeat", "mirror"].contains(&value), "Invalid wrap");
                                    image.wrap = value.into();
                                }
                                "filter" => {
                                    ensure!(["linear", "nearest"].contains(&value), "Invalid filter");
                                    image.filter = value.into();
                                }
                                "colorspace" => {
                                    ensure!(["linear", "srgb"].contains(&value), "Invalid colorspace");
                                    image.srgb = value == "srgb";
                                }
                                "alphaMode" => {
                                    ensure!(["straight", "premult"].contains(&value), "Invalid alpha mode");
                                    image.premult = value == "premult";
                                }
                                _ => bail!("Unknown texture option: {option}"),
                            }
                        }
                        let bytes = fs
                            .load_file(&format!("{root}/{path}"))
                            .await
                            .with_context(|| format!("Missing skin texture {root}/{path}"))?;
                        let mut pixels = image::load_from_memory(&bytes)?.flipv().to_rgba8();
                        image.width = pixels.width();
                        image.height = pixels.height();
                        for pixel in pixels.pixels_mut() {
                            for c in &mut pixel.0[..3] {
                                let x = *c as f64 / 255.;
                                let x = if image.srgb {
                                    if x <= 0.04045 {
                                        x / 12.92
                                    } else {
                                        ((x + 0.055) / 1.055).powf(2.4)
                                    }
                                } else {
                                    x
                                };
                                *c = (x * 255.).round() as u8;
                            }
                            if image.premult {
                                for i in 0..3 {
                                    pixel.0[i] = (pixel.0[i] as u32 * pixel.0[3] as u32 / 255) as u8;
                                }
                            }
                        }
                        image.rgba = pixels.into_raw();
                        loaded.push(image);
                    }
                    images.insert(pass.shader.clone(), loaded);
                }
                let (output, component) = pass.output.split_once('.').map_or((pass.output.as_str(), None), |(a, b)| (a, Some(b)));
                ensure!(output == "$screen" || channel.targets.contains_key(output), "Unknown skin output {output}");
                if let Some(component) = component {
                    let target = channel.targets.get(output).context("Screen channel writes are invalid")?;
                    let channels = match target.format.as_str() {
                        "r8" => "r",
                        "rg8" => "rg",
                        _ => "rgba",
                    };
                    ensure!(component.len() == 1 && channels.contains(component), "Invalid skin output component");
                }
                for (name, input) in &pass.inputs {
                    ensure!(uniforms.get(name).map(String::as_str) == Some("sampler2D"), "Skin input is not a sampler: {name}");
                    if input != "$screen" {
                        let target = channel.targets.get(input).context("Unknown skin input target")?;
                        ensure!(
                            target.no_clear || (written.contains(input) && input != output),
                            "Skin target read before write or feedback: {input}"
                        );
                    }
                }
                for (name, value) in &pass.constants {
                    ensure!(parameter_type(value) == uniforms.get(name).map(String::as_str), "Invalid skin constant {name}");
                }
                for (name, ty) in &uniforms {
                    if ty == "sampler2D" {
                        ensure!(
                            name == "screenTexture"
                                || name == "blockMask"
                                || pass.inputs.contains_key(name)
                                || images[&pass.shader].iter().any(|i| i.name == *name),
                            "Unbound skin sampler: {name}"
                        );
                    }
                }
                written.insert(output.to_owned());
            }
        }
        let channel = settings.get("channel").and_then(Value::as_str).unwrap_or("normal");
        ensure!(["normal", "low"].contains(&channel), "Invalid skin channel");
        Ok(Self {
            layer,
            root,
            normal,
            low,
            channel_low: channel == "low",
            parameters,
            sources,
            images,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dir::Dir, fs::ExternalFileSystem};
    use std::sync::Arc;
    fn load(root: &std::path::Path, settings: Value) -> Result<Definition> {
        let mut fs = ExternalFileSystem(Arc::new(Dir::new(root)?));
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(Definition::load(0, &settings, &mut fs))
    }
    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("shader/test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.glsl"),"precision highp float;\nvarying vec2 uv;\nuniform float touchReleaseDuration; // %0.1%\nuniform vec4 tint; // %1,0.5,0.25,1%\nuniform sampler2D picture; // @texture pixel.png wrap=mirror filter=nearest colorspace=srgb alphaMode=premult\nvoid main(){gl_FragColor=texture2D(picture,uv)*tint;}").unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 64, 32, 128]))
            .save(dir.join("pixel.png"))
            .unwrap();
        root
    }
    #[test]
    fn loads_real_image_options_defaults_and_overrides() {
        let root = fixture();
        let def = load(root.path(), json!({"skin":"shader/test","parameters":{"tint":[0,1,0,1]}})).unwrap();
        assert_eq!(def.parameters["tint"], json!([0, 1, 0, 1]));
        assert_eq!(def.parameters["touchReleaseDuration"], json!(0.1));
        let image = &def.images["main.glsl"][0];
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!((&image.wrap, &image.filter), (&"mirror".to_owned(), &"nearest".to_owned()));
        assert_eq!(image.rgba, vec![27, 6, 2, 128]);
        assert_eq!(def.normal.passes[0].output, "$screen");
    }
    #[test]
    fn rejects_missing_images_unknown_parameters_and_invalid_types() {
        let root = fixture();
        assert!(load(root.path(), json!({"skin":"shader/test","parameters":{"tint":1}})).is_err());
        assert!(load(root.path(), json!({"skin":"shader/test","parameters":{"unknown":1}})).is_err());
        std::fs::remove_file(root.path().join("shader/test/pixel.png")).unwrap();
        assert!(load(root.path(), json!({"skin":"shader/test"})).is_err());
    }
    #[test]
    fn rejects_traversal_channels_and_uninitialized_targets() {
        for name in ["../x", "shader/../secret", "/root", "shader\\a", "shader/a?x", "shader//a"] {
            assert!(safe_path(name).is_err());
        }
        let root = fixture();
        assert!(load(root.path(), json!({"skin":"shader/test","channel":"missing"})).is_err());
        std::fs::write(
            root.path().join("shader/test/config.json"),
            json!({"version":1,"targets":{"cache":{}},"passes":[{"shader":"main.glsl","output":"$screen","inputs":{"picture":"cache"}}]}).to_string(),
        )
        .unwrap();
        assert!(load(root.path(), json!({"skin":"shader/test"})).is_err());
    }
    #[test]
    fn guard_padding_matches_logical_frame_units() {
        assert_eq!(padding(Some(&json!({"pixels":32,"screenWidth":0.071,"screenHeight":0.071})), 640, 360).unwrap(), (78, 58));
        assert_eq!(padding(Some(&json!(0)), 640, 360).unwrap(), (0, 0));
        assert!(padding(Some(&json!(-1)), 640, 360).is_err());
    }
}
