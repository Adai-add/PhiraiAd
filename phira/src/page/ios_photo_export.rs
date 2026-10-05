//! iOS Photos bridge. This function runs on the encoding worker, never the UI thread.
use anyhow::{Context, Result};
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    sync::mpsc,
};

type Completion = unsafe extern "C" fn(*mut c_void, c_int, *const c_char);
unsafe extern "C" {
    fn phiraiad_save_png_to_photos(bytes: *const u8, length: usize, filename: *const c_char, completion: Completion, context: *mut c_void);
}

unsafe extern "C" fn completed(context: *mut c_void, success: c_int, error: *const c_char) {
    // Native code calls exactly once. Copy its ephemeral NSString UTF-8 bytes
    // before returning; dropping the Box releases the callback's ownership.
    let sender = unsafe { Box::from_raw(context.cast::<mpsc::Sender<Result<()>>>()) };
    let result = if success != 0 {
        Ok(())
    } else {
        let message = if error.is_null() {
            "相册写入失败".into()
        } else {
            unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned()
        };
        Err(anyhow::anyhow!(message))
    };
    let _ = sender.send(result);
}

pub fn save_png(filename: &str, png: &[u8]) -> Result<String> {
    let name = CString::new(filename).context("图片文件名无效")?;
    let (sender, receiver) = mpsc::channel::<Result<()>>();
    let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
    unsafe { phiraiad_save_png_to_photos(png.as_ptr(), png.len(), name.as_ptr(), completed, context) };
    // Authorization and Photos commit are asynchronous on native queues;
    // waiting here leaves the game/UI thread available for the permission dialog.
    receiver.recv().context("相册导出回调中断")??;
    Ok(format!("系统相册 · {filename}"))
}
