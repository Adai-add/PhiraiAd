//! Best-effort frame-rate preference on the actual game Surface (Android 11+).
//! No DEX/shell modifications, window mutation, or per-frame JNI work.
use anyhow::{ensure, Context, Result};
use jni::sys::{jclass, jmethodID, jobject, jobjectArray, jvalue, JNIEnv};
use std::{ffi::CString, ptr, time::{Duration, Instant}};

pub struct HighRefresh {
    remaining: u8,
    next: Instant,
}
impl HighRefresh {
    pub fn new() -> Self {
        Self { remaining: 5, next: Instant::now() }
    }
    pub fn resume(&mut self) {
        self.remaining = 5;
        self.next = Instant::now();
    }
    pub fn tick(&mut self) {
        if self.remaining == 0 || Instant::now() < self.next {
            return;
        }
        self.remaining -= 1;
        self.next = Instant::now() + Duration::from_secs(1);
        match request() {
            Ok(Some((requested, current))) => {
                tracing::info!(requested_hz = requested, before_request_hz = current,
                    "Android high-refresh preference submitted; system decides actual rate");
                self.remaining = 0;
            }
            Ok(None) => {
                tracing::info!("Android high-refresh Surface API unavailable before Android 11");
                self.remaining = 0;
            }
            Err(error) if self.remaining == 0 => {
                tracing::warn!(?error, "Android high-refresh request skipped; continuing normally");
            }
            Err(_) => {} // Surface may not be ready yet; retry a bounded number of times.
        }
    }
}

struct LocalFrame(*mut JNIEnv);
impl Drop for LocalFrame {
    fn drop(&mut self) {
        unsafe {
            if ((**self.0).v1_6.ExceptionCheck)(self.0) {
                ((**self.0).v1_6.ExceptionClear)(self.0);
            }
            ((**self.0).v1_6.PopLocalFrame)(self.0, ptr::null_mut());
        }
    }
}
unsafe fn check(env: *mut JNIEnv) -> Result<()> {
    if ((**env).v1_6.ExceptionCheck)(env) {
        ((**env).v1_6.ExceptionClear)(env);
        anyhow::bail!("Android refresh JNI call raised an exception");
    }
    Ok(())
}
unsafe fn class(env: *mut JNIEnv, name: &str) -> Result<jclass> {
    let name = CString::new(name)?;
    let result = ((**env).v1_6.FindClass)(env, name.as_ptr());
    check(env)?;
    ensure!(!result.is_null(), "Android refresh class unavailable");
    Ok(result)
}
unsafe fn method(env: *mut JNIEnv, object: jobject, name: &str, signature: &str) -> Result<jmethodID> {
    ensure!(!object.is_null(), "Android refresh object unavailable");
    let cls = ((**env).v1_6.GetObjectClass)(env, object);
    check(env)?;
    ensure!(!cls.is_null(), "Android refresh object class unavailable");
    let name = CString::new(name)?;
    let signature = CString::new(signature)?;
    let result = ((**env).v1_6.GetMethodID)(env, cls, name.as_ptr(), signature.as_ptr());
    ((**env).v1_6.DeleteLocalRef)(env, cls);
    check(env)?;
    ensure!(!result.is_null(), "Android refresh method unavailable");
    Ok(result)
}
unsafe fn object(env: *mut JNIEnv, obj: jobject, name: &str, signature: &str, args: &[jvalue]) -> Result<jobject> {
    let id = method(env, obj, name, signature)?;
    let value = ((**env).v1_6.CallObjectMethodA)(env, obj, id, args.as_ptr());
    check(env)?;
    ensure!(!value.is_null(), "Android refresh {name} returned null");
    Ok(value)
}
unsafe fn int(env: *mut JNIEnv, obj: jobject, name: &str) -> Result<i32> {
    let id = method(env, obj, name, "()I")?;
    let value = ((**env).v1_6.CallIntMethodA)(env, obj, id, ptr::null());
    check(env)?;
    Ok(value)
}
unsafe fn rate(env: *mut JNIEnv, mode: jobject) -> Result<f32> {
    let id = method(env, mode, "getRefreshRate", "()F")?;
    let value = ((**env).v1_6.CallFloatMethodA)(env, mode, id, ptr::null());
    check(env)?;
    Ok(value)
}
// Bounded traversal of the framework view tree. The existing shell's content
// is a SurfaceView; do not rely on its obfuscated class name or child index.
unsafe fn surface_view(env: *mut JNIEnv, view: jobject, surface_class: jclass, group_class: jclass,
    depth: u8, budget: &mut usize) -> Result<Option<jobject>> {
    if view.is_null() || depth == 0 || *budget == 0 {
        return Ok(None);
    }
    *budget -= 1;
    if ((**env).v1_6.IsInstanceOf)(env, view, surface_class) {
        return Ok(Some(view));
    }
    if !((**env).v1_6.IsInstanceOf)(env, view, group_class) {
        return Ok(None);
    }
    let count = int(env, view, "getChildCount")?.clamp(0, 64);
    let get_child = method(env, view, "getChildAt", "(I)Landroid/view/View;")?;
    for i in 0..count {
        let args = [jvalue { i }];
        let child = ((**env).v1_6.CallObjectMethodA)(env, view, get_child, args.as_ptr());
        check(env)?;
        if let Some(found) = surface_view(env, child, surface_class, group_class, depth - 1, budget)? {
            return Ok(Some(found));
        }
        if !child.is_null() {
            ((**env).v1_6.DeleteLocalRef)(env, child);
        }
        if *budget == 0 { break; }
    }
    Ok(None)
}
fn request() -> Result<Option<(f32, f32)>> {
    let env = unsafe { miniquad::native::attach_jni_env() } as *mut JNIEnv;
    ensure!(!env.is_null(), "Android refresh JNI environment unavailable");
    unsafe {
        let pushed = ((**env).v1_6.PushLocalFrame)(env, 128);
        check(env)?;
        ensure!(pushed >= 0, "Android refresh JNI local frame unavailable");
        let _frame = LocalFrame(env);
        let version = class(env, "android/os/Build$VERSION")?;
        let name = CString::new("SDK_INT")?;
        let signature = CString::new("I")?;
        let field = ((**env).v1_6.GetStaticFieldID)(env, version, name.as_ptr(), signature.as_ptr());
        check(env)?;
        ensure!(!field.is_null(), "Android SDK field unavailable");
        let sdk = ((**env).v1_6.GetStaticIntField)(env, version, field);
        check(env)?;
        if sdk < 30 { return Ok(None); }

        let activity = ndk_context::android_context().context() as jobject;
        let window = object(env, activity, "getWindow", "()Landroid/view/Window;", &[])?;
        let decor = object(env, window, "getDecorView", "()Landroid/view/View;", &[])?;
        let sc = class(env, "android/view/SurfaceView")?;
        let gc = class(env, "android/view/ViewGroup")?;
        let mut budget = 64;
        let view = surface_view(env, decor, sc, gc, 8, &mut budget)?.context("Game SurfaceView not ready")?;
        let display = object(env, view, "getDisplay", "()Landroid/view/Display;", &[])?;
        let current_mode = object(env, display, "getMode", "()Landroid/view/Display$Mode;", &[])?;
        let width = int(env, current_mode, "getPhysicalWidth")?;
        let height = int(env, current_mode, "getPhysicalHeight")?;
        let observed = rate(env, current_mode)?;
        ensure!(observed.is_finite() && observed > 0., "Invalid current refresh rate");
        let mut requested = observed;
        let modes = object(env, display, "getSupportedModes", "()[Landroid/view/Display$Mode;", &[])? as jobjectArray;
        let count = ((**env).v1_6.GetArrayLength)(env, modes);
        check(env)?;
        for i in 0..count.min(256) {
            let mode = ((**env).v1_6.GetObjectArrayElement)(env, modes, i);
            check(env)?;
            if !mode.is_null() {
                // Request a rate, never a different resolution/mode ID.
                if int(env, mode, "getPhysicalWidth")? == width && int(env, mode, "getPhysicalHeight")? == height {
                    let hz = rate(env, mode)?;
                    if hz.is_finite() && hz > requested && hz <= 1000. { requested = hz; }
                }
                ((**env).v1_6.DeleteLocalRef)(env, mode);
            }
        }
        let holder = object(env, view, "getHolder", "()Landroid/view/SurfaceHolder;", &[])?;
        let surface = object(env, holder, "getSurface", "()Landroid/view/Surface;", &[])?;
        let valid_method = method(env, surface, "isValid", "()Z")?;
        let valid = ((**env).v1_6.CallBooleanMethodA)(env, surface, valid_method, ptr::null());
        check(env)?;
        ensure!(valid, "Game Surface not valid yet");
        let set_rate = method(env, surface, "setFrameRate", "(FI)V")?;
        // COMPATIBILITY_DEFAULT (0): game can render at the chosen display rate.
        // Two-argument API changes only seamlessly, avoiding a disruptive switch.
        let args = [jvalue { f: requested }, jvalue { i: 0 }];
        ((**env).v1_6.CallVoidMethodA)(env, surface, set_rate, args.as_ptr());
        check(env)?;
        Ok(Some((requested, observed)))
    }
}
