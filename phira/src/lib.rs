prpr_l10n::tl_file!("common" ttl crate::);

/// Exported build marker checked by the Android release packager. It exists
/// only when the FFmpeg-backed video feature is compiled into this library.
#[cfg(all(target_os = "android", feature = "video"))]
#[used]
#[no_mangle]
pub static PHIRA_REPLICA_VIDEO_ENABLED: [u8; 38] = *b"PHIRA_REPLICA_VIDEO_ENABLED_FFMPEG_V1\0";

#[rustfmt::skip]
#[cfg(closed)]
mod inner;

mod ai_chart_adapter;
mod ai_model;
mod ai_service;
mod anim;
#[cfg(target_os = "android")]
mod android_refresh;
mod bn_image;
mod bn_import;
mod censor;
mod chart_play_settings;
mod charts_view;
mod client;
/// Packaging rejects old native libraries lacking this layout revision.
#[used]
#[no_mangle]
pub static PHIRAIAD_PRACTICE_LAYOUT_REVISION: [u8; 47] = *b"PHIRAIAD_PRACTICE_LAYOUT_70_SHIFT10_BN_ROWS_V2\0";

pub mod challenge;
mod challenge_ui;
mod custom_font;
mod custom_resources;
pub mod custom_rks;
mod data;
mod data_guard;
pub mod deeplink;
mod icons;
mod images;
mod login;
mod mp;
mod page;
mod play_report_export;
mod popup;
mod rate;
mod resource;
mod scene;
mod tabs;
mod tags;
mod threed;
mod uml;

use anyhow::{Context, Result};
use data::Data;
use macroquad::prelude::*;
use prpr::{
    build_conf,
    core::{init_assets, PGR_FONT},
    ext::SafeTexture,
    log,
    scene::show_error,
    time::TimeManager,
    ui::{cleanup_audio, FontArc, TextPainter},
    Main,
};
use prpr_l10n::set_prefered_locale;
#[cfg(not(feature = "hykb"))]
use prpr_l10n::{GLOBAL, LANGS};
use scene::MainScene;
use std::{
    collections::VecDeque,
    sync::{mpsc, Mutex},
};
use tracing::{error, info};

#[cfg(target_os = "android")]
use jni::{
    objects::{JClass, JObject, JString},
    sys::{jboolean, jfloat, jint},
    EnvUnowned,
};

static MESSAGES_TX: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);
static DATA_PATH: Mutex<Option<String>> = Mutex::new(None);
static CACHE_DIR: Mutex<Option<String>> = Mutex::new(None);
pub static mut DATA: Option<Data> = None;

#[cfg(target_env = "ohos")]
use napi_derive_ohos::napi;

#[cfg(closed)]
pub fn resolve_res_data(bytes: Vec<u8>) -> Result<Vec<u8>> {
    // The closed decoder is external; preserve its API behind this Result wrapper.
    Ok(inner::resolve_data(bytes))
}

#[cfg(not(closed))]
fn decode_resource_blocks(bytes: &mut [u8]) {
    assert_eq!(bytes.len() % 8, 0, "invalid bundled resource length");
    for block in bytes.chunks_mut(8) {
        block.swap(2, 6);
        block.swap(3, 6);

        block[1] = block[1].wrapping_sub(block[0]);
        block[3] = block[3].wrapping_sub(block[2]);
        block[5] = block[5].wrapping_sub(block[4]);
        block[7] = block[7].wrapping_sub(block[6]);
        block[2] = block[2].wrapping_sub(block[0]);
        block[3] = block[3].wrapping_sub(block[1]);
        block[6] = block[6].wrapping_sub(block[4]);
        block[7] = block[7].wrapping_sub(block[5]);
        block[4] = block[4].wrapping_sub(block[0]);
        block[6] = block[6].wrapping_sub(block[2]);
        block[7] = block[7].wrapping_sub(block[3]);
        let old_three = block[3];
        block[3] = block[5].wrapping_sub(block[1]);
        block[5] = old_three;
    }
}

#[cfg(not(closed))]
pub fn resolve_res_data(mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    anyhow::ensure!(!bytes.is_empty(), "empty bundled resource");
    anyhow::ensure!(bytes.len() % 8 == 0, "invalid bundled resource length: {}", bytes.len());
    decode_resource_blocks(&mut bytes);
    let remainder = bytes[bytes.len() - 1] as usize;
    anyhow::ensure!(remainder < 8, "invalid bundled resource padding: {remainder}");
    let padding = 8 - remainder;
    anyhow::ensure!(
        bytes.len() >= padding && bytes[bytes.len() - padding..].iter().all(|&it| it as usize == remainder),
        "invalid bundled resource padding bytes"
    );
    bytes.truncate(bytes.len() - padding);
    zstd::decode_all(bytes.as_slice()).context("failed to decompress bundled resource")
}

pub async fn load_res(name: &str) -> Result<Vec<u8>> {
    let result = async {
        let bytes = load_file(name)
            .await
            .with_context(|| format!("failed to read bundled resource: {name}"))?;
        resolve_res_data(bytes).with_context(|| format!("failed to decode bundled resource: {name}"))
    }
    .await;
    if let Err(err) = &result {
        error!(resource = name, ?err, "bundled resource loading failed");
    }
    result
}

#[allow(unused)]
pub async fn load_res_tex(name: &str) -> Result<SafeTexture> {
    let bytes = load_res(name).await?;
    let image = image::load_from_memory(&bytes)
        .with_context(|| format!("failed to decode bundled image: {name}"))
        .map_err(|err| {
            error!(resource = name, ?err, "bundled image loading failed");
            err
        })?;
    Ok(image.into())
}

#[cfg(all(test, not(closed)))]
mod bundled_resource_tests {
    use super::{decode_resource_blocks, resolve_res_data};

    #[test]
    fn decodes_known_resource_block() {
        let mut encoded = vec![0x28, 0xdd, 0x64, 0xd8, 0x28, 0xda, 0x57, 0x86];
        decode_resource_blocks(&mut encoded);
        assert_eq!(encoded, [0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x58, 0x81, 0xa4]);
    }

    #[test]
    fn rejects_empty_and_incomplete_resources() {
        assert!(resolve_res_data(Vec::new()).unwrap_err().to_string().contains("empty"));
        assert!(resolve_res_data(vec![0; 7]).unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn rejects_invalid_padding_and_compressed_data() {
        // Encoded blocks decode to eight 8s (invalid remainder), invalid padding,
        // and eight 1s (valid padding but invalid compressed payload), respectively.
        assert!(resolve_res_data(vec![8, 16, 32, 32, 16, 32, 16, 64])
            .unwrap_err()
            .to_string()
            .contains("padding"));
        assert!(resolve_res_data(vec![0, 0, 0, 2, 0, 0, 0, 3]).is_err());
        assert!(resolve_res_data(vec![1, 2, 4, 4, 2, 4, 2, 8])
            .unwrap_err()
            .to_string()
            .contains("decompress"));
    }

    #[test]
    fn resolves_bundled_character_image() {
        let decoded = resolve_res_data(include_bytes!("../../assets/res/shee.char").to_vec()).unwrap();
        assert_eq!(&decoded[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(decoded.len(), 119_952);
    }
}

pub fn sync_data() {
    if get_data().language.is_none() {
        #[cfg(feature = "hykb")]
        let default_lang = "zh-CN".to_owned();
        #[cfg(not(feature = "hykb"))]
        let default_lang = LANGS[GLOBAL.order.lock().unwrap()[0]].to_owned();
        get_data_mut().language = Some(default_lang);
    }
    set_prefered_locale(get_data().language.as_ref().and_then(|it| it.parse().ok()));
    let _ = client::set_access_token_sync(get_data().tokens.as_ref().map(|it| &*it.0));
}

pub fn set_data(data: Data) {
    unsafe {
        DATA = Some(data);
    }
}

#[allow(static_mut_refs)]
pub fn get_data() -> &'static Data {
    unsafe { DATA.as_ref().unwrap() }
}

#[allow(static_mut_refs)]
pub fn get_data_mut() -> &'static mut Data {
    unsafe { DATA.as_mut().unwrap() }
}

pub fn save_data() -> Result<()> {
    data_guard::save(std::path::Path::new(&dir::root()?), get_data())?;
    Ok(())
}

mod dir {
    use anyhow::Result;

    use crate::{CACHE_DIR, DATA_PATH};

    fn ensure(s: &str) -> Result<String> {
        let s = format!("{}/{}", DATA_PATH.lock().unwrap().as_ref().map(|it| it.as_str()).unwrap_or("."), s);
        let path = std::path::Path::new(&s);
        if !path.exists() {
            std::fs::create_dir_all(path)?;
        }
        Ok(s)
    }

    pub fn cache() -> Result<String> {
        if let Some(cache) = &*CACHE_DIR.lock().unwrap() {
            ensure(cache)
        } else {
            ensure("cache")
        }
    }

    pub fn bold_font_path() -> Result<String> {
        Ok(format!("{}/bold.ttf", root()?))
    }

    pub fn cache_image_local() -> Result<String> {
        ensure(&format!("{}/image", cache()?))
    }

    pub fn root() -> Result<String> {
        ensure("data")
    }

    pub fn charts() -> Result<String> {
        ensure("data/charts")
    }

    pub fn collections() -> Result<String> {
        ensure("data/collections")
    }

    pub fn custom_charts() -> Result<String> {
        ensure("data/charts/custom")
    }

    pub fn downloaded_charts() -> Result<String> {
        ensure("data/charts/download")
    }

    pub fn respacks() -> Result<String> {
        ensure("data/respack")
    }

    pub fn play_reports() -> Result<String> {
        ensure("data/play-reports")
    }
}

async fn the_main() -> Result<()> {
    log::startup_checkpoint("01 logging");
    log::register();
    #[cfg(target_env = "ohos")]
    {
        *DATA_PATH.lock().unwrap() = Some("/data/storage/el2/base".to_owned());
        *CACHE_DIR.lock().unwrap() = Some("/data/storage/el2/base/cache".to_owned());
        prpr::core::DPI_VALUE.store(250, std::sync::atomic::Ordering::Relaxed);
    };

    log::startup_checkpoint("02 assets directory");
    init_assets();
    log::startup_checkpoint("03 Tokio runtime");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    log::startup_checkpoint("04 platform data directory");
    #[cfg(target_os = "ios")]
    {
        use objc2_foundation::{NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains};

        let directories = NSSearchPathForDirectoriesInDomains(NSSearchPathDirectory::LibraryDirectory, NSSearchPathDomainMask::UserDomainMask, true);
        let path = directories.firstObject().unwrap().to_string();
        *DATA_PATH.lock().unwrap() = Some(path);
        *CACHE_DIR.lock().unwrap() = Some("Caches".to_owned());
    }

    log::startup_checkpoint("05 saved data initialization");
    let dir = dir::root()?;
    prpr::replay::set_root(&dir);
    let mut data: Data = data_guard::load(std::path::Path::new(&dir))?;
    data.init().await?;
    set_data(data);
    let _ = prpr::practice_audio::PREFERENCE_SAVER.set(|enabled| {
        let old = get_data().config.practice_preserve_pitch;
        get_data_mut().config.practice_preserve_pitch = enabled;
        if let Err(error) = save_data() {
            get_data_mut().config.practice_preserve_pitch = old;
            return Err(error);
        }
        Ok(())
    });
    sync_data();
    save_data()?;

    // Warm up the offline banned-word automaton so local edits can check
    // synchronously. No-op without the `aa` feature.
    tokio::spawn(censor::preload());

    let rx = {
        let (tx, rx) = mpsc::channel();
        *MESSAGES_TX.lock().unwrap() = Some(tx);
        rx
    };

    unsafe { get_internal_gl() }
        .quad_context
        .display_mut()
        .set_pause_resume_listener(on_pause_resume);

    log::startup_checkpoint("06 fonts");
    let pgr_font = FontArc::try_from_vec(load_file("phigros.ttf").await?)?;
    PGR_FONT.with(|it| *it.borrow_mut() = Some(TextPainter::new(pgr_font.clone(), None)));

    let font = FontArc::try_from_vec(load_file("font.ttf").await?)?;
    crate::custom_font::initialize(font.clone(), pgr_font);
    let mut painter = TextPainter::new(font.clone(), None);

    log::startup_checkpoint("07 MainScene construction");
    let mut main = Main::new(Box::new(MainScene::new(font).await?), TimeManager::default(), None).await?;

    #[cfg(target_os = "android")]
    let mut high_refresh = android_refresh::HighRefresh::new();
    let tm = TimeManager::default();
    let mut fps_time = -1;

    const FPS_BUF_SIZE: usize = 60;
    let mut fps_times = VecDeque::<f32>::with_capacity(FPS_BUF_SIZE);
    let mut last_frame_start = f32::NAN;
    let mut fps_time_sum = 0.;

    let mut first_frame = true;
    let mut diagnostic_frame = 0usize;
    'app: loop {
        if main.paused() {
            match rx.recv() {
                Ok(false) => {
                    log::diagnostic_event("LIFECYCLE resume from paused wait begin");
                    main.resume()?;
                    #[cfg(target_os = "android")]
                    high_refresh.resume();
                    log::diagnostic_event("LIFECYCLE resume from paused wait complete");
                }
                Ok(true) => {}
                Err(_) => break 'app,
            }
        }

        #[cfg(target_os = "android")]
        high_refresh.tick();
        log::frame_checkpoint(diagnostic_frame, "update begin");
        let frame_start = tm.real_time();
        if !last_frame_start.is_nan() {
            if fps_times.len() == FPS_BUF_SIZE {
                fps_time_sum -= fps_times.pop_front().unwrap();
            }
            let frame_time = frame_start as f32 - last_frame_start;
            fps_times.push_back(frame_time);
            fps_time_sum += frame_time;
        }
        last_frame_start = frame_start as f32;
        let res = || -> Result<()> {
            if first_frame {
                log::startup_checkpoint("12 first frame update");
            }
            main.update()?;
            log::frame_checkpoint(diagnostic_frame, "update complete; render begin");
            if first_frame {
                log::startup_checkpoint("13 first frame render");
            }
            crate::custom_font::apply(&mut painter);
            main.render(&mut painter)?;
            log::frame_checkpoint(diagnostic_frame, "render commands queued");
            if first_frame {
                log::startup_checkpoint("14 first frame rendered");
            }
            first_frame = false;
            if let Ok(paused) = rx.try_recv() {
                if paused {
                    log::diagnostic_event("LIFECYCLE pause apply begin");
                    main.pause()?;
                    log::diagnostic_event("LIFECYCLE pause apply complete");
                } else {
                    log::diagnostic_event("LIFECYCLE resume apply begin");
                    main.resume()?;
                    #[cfg(target_os = "android")]
                    high_refresh.resume();
                    log::diagnostic_event("LIFECYCLE resume apply complete");
                }
            }
            log::frame_checkpoint(diagnostic_frame, "texture cleanup begin");
            prpr::ext::flush_pending_texture_deletions();
            log::frame_checkpoint(diagnostic_frame, "texture cleanup complete");
            Ok(())
        }();
        if let Err(err) = res {
            error!("uncaught error: {err:?}");
            show_error(err);
        }
        if main.should_exit() {
            break 'app;
        }

        let t = tm.real_time();

        let fps_now = t as i32;
        if fps_now != fps_time {
            fps_time = fps_now;
            if fps_times.len() == FPS_BUF_SIZE {
                let actual_fps = 1. / (fps_time_sum / FPS_BUF_SIZE as f32);
                let current_fps = 1. / (t - frame_start);
                info!("FPS {} (capped at {})", current_fps as u32, actual_fps as u32);
            }
        }

        // While backgrounded the scene is paused; the blocking `recv_timeout`
        // above already parks this thread, so nothing extra is needed here.
        log::frame_checkpoint(diagnostic_frame, "next_frame begin");
        next_frame().await;
        log::frame_checkpoint(diagnostic_frame, "next_frame resumed; prior frame submitted");
        diagnostic_frame = diagnostic_frame.saturating_add(1);
    }
    Ok(())
}

fn build_global_window_conf() -> Conf {
    let mut conf = build_conf();
    conf.window_title = "Phira".to_owned();
    conf.icon = Some(miniquad::conf::Icon {
        small: *include_bytes!("../icon/small"),
        medium: *include_bytes!("../icon/medium"),
        big: *include_bytes!("../icon/big"),
    });

    #[cfg(target_os = "windows")]
    {
        conf.fullscreen = dir::root()
            .ok()
            .and_then(|r| std::fs::read_to_string(std::path::Path::new(&r).join("data.json")).ok())
            .and_then(|s| serde_json::from_str::<Data>(&s).ok())
            .is_some_and(|d| d.config.fullscreen_mode);
    }

    conf
}

#[no_mangle]
pub extern "C" fn quad_main() {
    log::install_ios_diagnostics();
    log::startup_checkpoint("00 window creation");
    macroquad::Window::from_config(build_global_window_conf(), async {
        if let Err(err) = the_main().await {
            error!(?err, "global error");
        }
    });
    cleanup_audio();
}

fn on_pause_resume(pause: bool) {
    log::diagnostic_event(if pause {
        "LIFECYCLE native pause received"
    } else {
        "LIFECYCLE native resume received"
    });
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(pause);
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_initializeEnvironment(env: EnvUnowned, _class: JClass) {
    unsafe {
        inputbox::backend::Android::initialize_raw(env.as_raw()).unwrap();
    }
}

/// Compatibility hook required by the Android 0.8.2 host shell.
///
/// The public Rust source does not contain the closed-source input preprocessor
/// shipped in the official Android binary. The host still invokes this method
/// before forwarding every event through `surfaceOnTouch`, so an absent symbol
/// terminates the process with `UnsatisfiedLinkError`. Keeping this hook as a
/// no-op preserves the normal miniquad input path while satisfying the shell's
/// JNI contract.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_quad_1native_QuadNative_preprocessInput(
    _env: EnvUnowned,
    _class: JClass,
    _event: JObject,
    _x: jfloat,
    _y: jfloat,
    _is_external: jboolean,
    _is_virtual: jboolean,
) {
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnPause(_env: EnvUnowned, _class: JClass) {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(true);
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnResume(_env: EnvUnowned, _class: JClass) {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(false);
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnDestroy(_env: EnvUnowned, _class: JClass) {
    std::process::exit(0);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDataPath(_env: EnvUnowned, _class: JClass, path: JString) {
    *DATA_PATH.lock().unwrap() = Some(path.to_string());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setTempDir(_env: EnvUnowned, _class: JClass, path: JString) {
    let path = path.to_string();
    std::env::set_var("TMPDIR", path.clone());
    *CACHE_DIR.lock().unwrap() = Some(path);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDpi(_env: EnvUnowned, _class: JClass, dpi: jint) {
    prpr::core::DPI_VALUE.store(dpi as _, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setChosenFile(_env: EnvUnowned, _class: JClass, file: JString) {
    use prpr::scene::CHOSEN_FILE;
    CHOSEN_FILE.lock().unwrap().1 = Some(file.to_string());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDeepLink(_env: EnvUnowned, _class: JClass, url: JString) {
    deeplink::set_deeplink(url.to_string());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_markImport(_env: EnvUnowned, _class: JClass) {
    use prpr::scene::CHOSEN_FILE;

    CHOSEN_FILE.lock().unwrap().0 = Some("_import".to_owned());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_markImportRespack(_env: EnvUnowned, _class: JClass) {
    use prpr::scene::CHOSEN_FILE;

    CHOSEN_FILE.lock().unwrap().0 = Some("_import_respack".to_owned());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setInputText(_env: EnvUnowned, _class: JClass, text: JString) {
    use prpr::scene::INPUT_TEXT;
    INPUT_TEXT.lock().unwrap().1 = Some(text.to_string());
}

/// Credentials obtained from the native HYKB (好游快爆) login SDK.
pub struct HykbCredential {
    /// SDK result code: 0 on success, otherwise an error / user cancellation.
    pub code: i32,
    pub uid: i64,
    pub nick: String,
    pub access_token: String,
}

impl HykbCredential {
    /// Map the SDK result code to an error, or yield the credential on success.
    /// Centralizes the code → user-facing message translation shared by every
    /// HYKB login/bind entry point.
    #[cfg(feature = "hykb")]
    pub fn ok_or_err(self) -> Result<Self> {
        if self.code == 0 {
            Ok(self)
        } else {
            // A non-zero code is any failure the HYKB SDK reports: 2001 auth
            // failed, 2002 login failed, 2003 cancelled, 2004 exception, 2005
            // developer-requested exit / account logout. A HYKB build mandates a
            // valid, matching HYKB session, so every one of these must tear the
            // in-game session down — otherwise cancelling the HYKB prompt during
            // a silent re-verify would leave the player signed in and bypass the
            // gate entirely.
            force_logout();
            anyhow::bail!("{}", crate::ttl!("hykb-login-cancelled"))
        }
    }
}

/// Slot for the pending HYKB login result. The native callback fulfills it.
static HYKB_TX: Mutex<Option<tokio::sync::oneshot::Sender<HykbCredential>>> = Mutex::new(None);

/// Call a no-arg `void` method on the Android host activity (the HYKB shell).
#[cfg(all(target_os = "android", feature = "hykb"))]
fn call_activity_void(method: &'static jni::strings::JNIStr) {
    use jni::{jni_sig, objects::JObject, vm::JavaVM};

    JavaVM::singleton()
        .unwrap()
        .attach_current_thread(|env| -> jni::errors::Result<()> {
            let ctx = unsafe { JObject::from_raw(env, ndk_context::android_context().context() as _) };
            env.call_method(ctx, method, jni_sig!("()V"), &[])?;
            Ok(())
        })
        .unwrap();
}

/// Ask the Android shell to pop the HYKB account picker (`MainActivity.hykbSwitchAccount`).
/// Used by the explicit login / switch-account flow.
#[cfg(all(target_os = "android", feature = "hykb"))]
fn request_hykb_login() {
    call_activity_void(jni::jni_str!("hykbSwitchAccount"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
fn request_hykb_login() {}

/// Ask the Android shell to sign in using the cached HYKB account without
/// popping the picker (`MainActivity.hykbLogin`). The credentials the SDK
/// reports flow back through `HYKB_TX`, so the caller can verify them against
/// the restored Phira session. Used by the silent startup restore.
#[cfg(all(target_os = "android", feature = "hykb"))]
fn request_hykb_login_silent() {
    call_activity_void(jni::jni_str!("hykbLogin"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
fn request_hykb_login_silent() {}

/// Tell the native HYKB SDK to sign out (`MainActivity.hykbLogout`). Called when the
/// player logs out from their profile.
#[cfg(all(target_os = "android", feature = "hykb"))]
pub fn hykb_logout() {
    call_activity_void(jni::jni_str!("hykbLogout"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
pub fn hykb_logout() {}

/// Tear down the local session: sign out of the native HYKB SDK, clear the
/// stored account and tokens, then re-sync. Shared by every path that must
/// reject a login — a failed/cancelled HYKB verification, a uid mismatch, or
/// the player logging out from their profile.
pub fn force_logout() {
    hykb_logout();
    get_data_mut().me = None;
    get_data_mut().tokens = None;
    let _ = save_data();
    sync_data();
}

/// Trigger the native HYKB login and await its credentials.
#[allow(unused)]
pub async fn obtain_hykb_credential() -> Result<HykbCredential> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *HYKB_TX.lock().unwrap() = Some(tx);
    request_hykb_login();
    let cred = rx.await.map_err(|_| anyhow::anyhow!("hykb login cancelled"))?;
    Ok(cred)
}

/// Silently restore the HYKB session from the cached account and await its
/// credentials. Unlike [`obtain_hykb_credential`], this does not pop the account
/// picker; used by the blocking startup check to verify the restored session.
#[allow(unused)]
pub async fn obtain_hykb_credential_silent() -> Result<HykbCredential> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *HYKB_TX.lock().unwrap() = Some(tx);
    request_hykb_login_silent();
    let cred = rx.await.map_err(|_| anyhow::anyhow!("hykb login cancelled"))?;
    Ok(cred)
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_hykbLoginCallback(
    _env: EnvUnowned,
    _class: JClass,
    code: jint,
    uid: jni::sys::jlong,
    nick: JString,
    access_token: JString,
) {
    let nick = if nick.is_null() { String::new() } else { nick.to_string() };
    let access_token = if access_token.is_null() {
        String::new()
    } else {
        access_token.to_string()
    };
    if let Some(tx) = HYKB_TX.lock().unwrap().take() {
        let _ = tx.send(HykbCredential {
            code: code as i32,
            uid: uid as i64,
            nick,
            access_token,
        });
    } else if code == 2005 {
        // No login is in flight, so this is the SDK's asynchronous
        // anti-addiction "exit game" action: the player hit a play-time limit
        // and chose to quit from the SDK's own dialog. Honor it by exiting.
        // Other async codes (e.g. 2008 "continue playing") are handled inside
        // the SDK and need no response here. A request-less success (code 0, the
        // SDK switching accounts on its own) is likewise ignored: any signed-in
        // HYKB account is accepted, so a switch no longer tears the session down.
    }
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn set_input_text(text: String) {
    use prpr::scene::INPUT_TEXT;
    INPUT_TEXT.lock().unwrap().1 = Some(text);
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn set_chosen_file(file: String) {
    use prpr::scene::CHOSEN_FILE;
    CHOSEN_FILE.lock().unwrap().1 = Some(file);
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn mark_auto_import() {
    use prpr::scene::CHOSEN_FILE;
    CHOSEN_FILE.lock().unwrap().0 = Some("_import_auto".to_owned());
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn on_foreground() {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(false);
    }
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn on_background() {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(true);
    }
}
