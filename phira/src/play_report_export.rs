//! Platform export sink for gameplay reports.

use crate::dir;
use anyhow::{Context, Result};
use prpr::{play_report::PlayReport, scene::ReportFn};
use sanitize_filename::sanitize;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
#[cfg(target_os = "android")]
use tracing::warn;
use uuid::Uuid;

pub fn report_fn() -> ReportFn {
    Arc::new(export_report)
}

fn export_report(report: &PlayReport) -> Result<String> {
    let mut filename = sanitize(report.suggested_filename());
    if filename.trim_matches('.').is_empty() {
        filename = format!("play-report-{}.json", report.report_id);
    }
    let bytes = serde_json::to_vec_pretty(report).context("failed to serialize play report")?;

    #[cfg(target_os = "android")]
    {
        match export_android_downloads(&filename, &bytes) {
            Ok(path) => return Ok(path),
            Err(err) => warn!(?err, "failed to export play report through Android MediaStore; using app storage"),
        }
    }

    export_app_storage(&filename, &bytes)
}

fn export_app_storage(filename: &str, bytes: &[u8]) -> Result<String> {
    let directory = PathBuf::from(dir::play_reports()?);
    let target = unique_path(&directory, filename);
    let temporary = directory.join(format!(".{}.{}.tmp", filename, Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = File::create(&temporary).with_context(|| format!("failed to create {}", temporary.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(target.to_string_lossy().into_owned())
}

fn unique_path(directory: &Path, filename: &str) -> PathBuf {
    let initial = directory.join(filename);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().and_then(|it| it.to_str()).unwrap_or("play-report");
    let extension = path.extension().and_then(|it| it.to_str()).unwrap_or("json");
    for suffix in 2..=9999 {
        let candidate = directory.join(format!("{stem}-{suffix:02}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}.{extension}", Uuid::new_v4()))
}

#[cfg(target_os = "android")]
fn export_android_downloads(filename: &str, bytes: &[u8]) -> Result<String> {
    use jni::sys::{jclass, jint, jobject, jstring, JNIEnv};
    use std::{ffi::CString, os::fd::FromRawFd};

    struct PendingExceptionGuard(*mut JNIEnv);

    impl Drop for PendingExceptionGuard {
        fn drop(&mut self) {
            unsafe {
                if ((**self.0).v1_6.ExceptionCheck)(self.0) {
                    ((**self.0).v1_6.ExceptionClear)(self.0);
                }
            }
        }
    }

    struct LocalFrameGuard(*mut JNIEnv);

    impl Drop for LocalFrameGuard {
        fn drop(&mut self) {
            unsafe {
                ((**self.0).v1_6.PopLocalFrame)(self.0, std::ptr::null_mut());
            }
        }
    }

    unsafe fn java_string(env: *mut JNIEnv, value: &str) -> Result<jstring> {
        let value = CString::new(value).context("Java string contains NUL")?;
        let string = ((**env).v1_6.NewStringUTF)(env, value.as_ptr());
        anyhow::ensure!(!string.is_null(), "JNI NewStringUTF returned null");
        Ok(string)
    }

    unsafe fn find_class(env: *mut JNIEnv, name: &str) -> Result<jclass> {
        let name = CString::new(name)?;
        let class = ((**env).v1_6.FindClass)(env, name.as_ptr());
        anyhow::ensure!(!class.is_null(), "Android class not found");
        Ok(class)
    }

    unsafe fn method(env: *mut JNIEnv, class: jclass, name: &str, signature: &str) -> Result<jni::sys::jmethodID> {
        let name = CString::new(name)?;
        let signature = CString::new(signature)?;
        let method = ((**env).v1_6.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr());
        anyhow::ensure!(!method.is_null(), "Android method not found");
        Ok(method)
    }

    unsafe fn put_string(env: *mut JNIEnv, values: jobject, values_class: jclass, key: &str, value: &str) -> Result<()> {
        let put = method(env, values_class, "put", "(Ljava/lang/String;Ljava/lang/String;)V")?;
        let key = java_string(env, key)?;
        let value = java_string(env, value)?;
        ((**env).v1_6.CallVoidMethod)(env, values, put, key, value);
        if ((**env).v1_6.ExceptionCheck)(env) {
            ((**env).v1_6.ExceptionClear)(env);
            anyhow::bail!("Android ContentValues.put failed");
        }
        Ok(())
    }

    // miniquad exposes the NDK's generated JNI types while `jni` exposes
    // jni-sys types. They describe the same VM ABI but are distinct Rust
    // declarations, so cross the boundary once at the raw pointer here.
    let env = unsafe { miniquad::native::attach_jni_env() } as *mut JNIEnv;
    let _exception_guard = PendingExceptionGuard(env);
    anyhow::ensure!(unsafe { ((**env).v1_6.PushLocalFrame)(env, 32) } >= 0, "failed to allocate a JNI local frame");
    let _local_frame = LocalFrameGuard(env);
    let sdk = unsafe {
        let version_class = find_class(env, "android/os/Build$VERSION")?;
        let field_name = CString::new("SDK_INT")?;
        let field_signature = CString::new("I")?;
        let field = ((**env).v1_6.GetStaticFieldID)(env, version_class, field_name.as_ptr(), field_signature.as_ptr());
        anyhow::ensure!(!field.is_null(), "Android SDK_INT field not found");
        ((**env).v1_6.GetStaticIntField)(env, version_class, field)
    };
    anyhow::ensure!(sdk >= 29, "public automatic report export requires Android 10 or newer");

    let fd = unsafe {
        let context = ndk_context::android_context().context() as jobject;
        let context_class = ((**env).v1_6.GetObjectClass)(env, context);
        let get_resolver = method(env, context_class, "getContentResolver", "()Landroid/content/ContentResolver;")?;
        let resolver = ((**env).v1_6.CallObjectMethod)(env, context, get_resolver);
        anyhow::ensure!(!resolver.is_null(), "Android ContentResolver is unavailable");

        let values_class = find_class(env, "android/content/ContentValues")?;
        let constructor = method(env, values_class, "<init>", "()V")?;
        let values = ((**env).v1_6.NewObject)(env, values_class, constructor);
        anyhow::ensure!(!values.is_null(), "failed to create Android ContentValues");
        put_string(env, values, values_class, "_display_name", filename)?;
        put_string(env, values, values_class, "mime_type", "application/json")?;
        put_string(env, values, values_class, "relative_path", "Download/Phira-Reports")?;

        let downloads_class = find_class(env, "android/provider/MediaStore$Downloads")?;
        let field_name = CString::new("EXTERNAL_CONTENT_URI")?;
        let field_signature = CString::new("Landroid/net/Uri;")?;
        let uri_field = ((**env).v1_6.GetStaticFieldID)(env, downloads_class, field_name.as_ptr(), field_signature.as_ptr());
        anyhow::ensure!(!uri_field.is_null(), "Android downloads URI field not found");
        let downloads_uri = ((**env).v1_6.GetStaticObjectField)(env, downloads_class, uri_field);
        anyhow::ensure!(!downloads_uri.is_null(), "Android downloads URI is unavailable");

        let resolver_class = ((**env).v1_6.GetObjectClass)(env, resolver);
        let insert = method(env, resolver_class, "insert", "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;")?;
        let uri = ((**env).v1_6.CallObjectMethod)(env, resolver, insert, downloads_uri, values);
        anyhow::ensure!(!uri.is_null(), "Android MediaStore refused the report");

        let open = method(env, resolver_class, "openFileDescriptor", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;")?;
        let mode = java_string(env, "w")?;
        let parcel = ((**env).v1_6.CallObjectMethod)(env, resolver, open, uri, mode);
        anyhow::ensure!(!parcel.is_null(), "failed to open Android report file");
        let parcel_class = ((**env).v1_6.GetObjectClass)(env, parcel);
        let detach = method(env, parcel_class, "detachFd", "()I")?;
        let fd: jint = ((**env).v1_6.CallIntMethod)(env, parcel, detach);
        anyhow::ensure!(fd >= 0, "Android returned an invalid report file descriptor");
        fd
    };

    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(format!("Download/Phira-Reports/{filename}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_report_names_receive_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("song.json"), b"one").unwrap();
        assert_eq!(unique_path(dir.path(), "song.json").file_name().unwrap(), "song-02.json");
    }
}
