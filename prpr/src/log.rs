//! Logging utilities.

use colored::Colorize;
use miniquad::{debug, error, info, trace, warn};
use tracing::{field::Visit, Level, Subscriber};
use tracing_subscriber::{prelude::*, EnvFilter, Layer};

struct CustomLayer;

impl<S> Layer<S> for CustomLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        #[derive(Default)]
        struct Visitor {
            message: Option<String>,
            target: Option<String>,
            fields: Vec<(&'static str, String)>,
        }
        impl Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.message = Some(value.to_string());
                } else if !field.name().starts_with("log.") {
                    self.fields.push((field.name(), value.to_string()));
                } else if field.name() == "log.target" {
                    self.target = Some(value.to_string());
                }
            }

            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let val = format!("{value:?}");
                if field.name() == "message" {
                    self.message = Some(val);
                } else if !field.name().starts_with("log.") {
                    self.fields.push((field.name(), val));
                }
            }
        }

        let mut v = Visitor::default();
        event.record(&mut v);

        let meta = event.metadata();
        let target = v.target.as_deref().unwrap_or_else(|| meta.target());
        if target.starts_with("jni::") && meta.level() >= &Level::INFO {
            return;
        }

        #[cfg(not(target_os = "android"))]
        let mut msg = format!("{:.6?} ", chrono::Utc::now()).bright_black().to_string()
            + &match *meta.level() {
                Level::TRACE => "TRACE".bright_black(),
                Level::DEBUG => "DEBUG".magenta(),
                Level::INFO => " INFO".green(),
                Level::WARN => " WARN".yellow(),
                Level::ERROR => "ERROR".red(),
            }
            .to_string()
            + " ";

        #[cfg(target_os = "android")]
        let mut msg = String::new();

        msg += &target.bright_black().to_string();
        if !v.fields.is_empty() {
            msg += &"{".bold().to_string();
            for (name, val) in &v.fields {
                use std::fmt::Write;
                let _ = write!(msg, "{}={val} ", name.italic());
            }
            if !v.fields.is_empty() {
                msg.pop();
            }
            msg += &"}".bold().to_string();
        }
        #[cfg(target_os = "ios")]
        let diagnostic_message = v.message.clone().unwrap_or_default();
        if let Some(message) = v.message {
            msg += ": ";
            msg += &message;
        }

        #[cfg(target_os = "ios")]
        if matches!(*meta.level(), Level::WARN | Level::ERROR) {
            // Keep an uncolored copy for crash annotations and the file log.
            let mut diagnostic = format!("{} {}", meta.level(), target);
            for (name, value) in &v.fields {
                diagnostic += &format!(" {name}={value}");
            }
            // `msg` already owns the message, but includes terminal colors;
            // use the recorded copy below rather than the formatted console line.
            diagnostic += ": ";
            diagnostic += &diagnostic_message;
            ios_diagnostics::record(&diagnostic);
        }

        match *meta.level() {
            Level::TRACE => trace!("{}", msg),
            Level::DEBUG => debug!("{}", msg),
            Level::INFO => info!("{}", msg),
            Level::WARN => warn!("{}", msg),
            Level::ERROR => error!("{}", msg),
        }
    }
}

pub fn register() {
    let filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        EnvFilter::try_new("hyper=info,rustls=info,debug").unwrap()
    };
    tracing_subscriber::registry().with(CustomLayer).with(filter).init();
}

/// Startup breadcrumbs are a no-op outside iOS.
pub fn startup_checkpoint(stage: &'static str) {
    #[cfg(target_os = "ios")]
    ios_diagnostics::record(&format!("STARTUP {stage}"));
    #[cfg(not(target_os = "ios"))]
    let _ = stage;
}

/// Install before Window creation so the first Rust panic is preserved before
/// a C/Objective-C callback turns it into panic_cannot_unwind.
pub fn install_ios_diagnostics() {
    #[cfg(target_os = "ios")]
    ios_diagnostics::install();
}

#[cfg(target_os = "ios")]
mod ios_diagnostics {
    use std::{
        collections::VecDeque,
        ffi::CString,
        fs::OpenOptions,
        io::Write,
        path::PathBuf,
        sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Mutex, Once, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    // Best-effort CrashReporter annotation ABI, also used by Apple's Swift
    // runtime (include/swift/Runtime/Debug.h). This is not a public iOS API:
    // the OS may omit it. No private symbols/frameworks are imported.
    #[repr(C)]
    struct CrashAnnotations {
        version: u64,
        message: AtomicU64,
        signature_string: u64,
        backtrace: u64,
        message2: AtomicU64,
        thread: u64,
        dialog_mode: u64,
        abort_cause: u64,
    }

    #[used]
    #[no_mangle]
    #[link_section = "__DATA,__crash_info"]
    static PHIRAIAD_CRASH_ANNOTATIONS: CrashAnnotations = CrashAnnotations {
        version: 5,
        message: AtomicU64::new(0),
        signature_string: 0,
        backtrace: 0,
        message2: AtomicU64::new(0),
        thread: 0,
        dialog_mode: 0,
        abort_cause: 0,
    };

    struct History {
        lines: VecDeque<String>,
        // Fixed address for process lifetime; the system reads this C string
        // after suspending the crashed process. It must never be freed.
        buffer: [u8; 8192],
    }
    static HISTORY: Mutex<History> = Mutex::new(History {
        lines: VecDeque::new(),
        buffer: [0; 8192],
    });
    static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
    static FILE_LOCK: Mutex<()> = Mutex::new(());
    static INSTALL: Once = Once::new();
    static FIRST_MAIN_PANIC: AtomicBool = AtomicBool::new(false);
    static FIRST_BACKGROUND_PANIC: AtomicBool = AtomicBool::new(false);

    extern "C" {
        fn write(fd: i32, buffer: *const std::ffi::c_void, count: usize) -> isize;
        fn pthread_main_np() -> i32;
    }

    fn stderr(message: &str) {
        // Do not use eprintln!/tracing in the panic hook: failed formatting or
        // a poisoned subscriber lock must not cause a second panic.
        unsafe {
            let _ = write(2, message.as_ptr().cast(), message.len());
            let _ = write(2, b"\n".as_ptr().cast(), 1);
        }
    }

    fn append_file(message: &str) {
        let Some(path) = LOG_PATH.get() else { return };
        let Ok(_guard) = FILE_LOCK.try_lock() else { return };
        // Bound disk usage; retain the preceding log as a sibling file.
        if std::fs::metadata(path).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(path, path.with_extension("previous.log"));
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{message}");
            let _ = file.sync_data();
        }
    }

    pub(super) fn record(message: &str) {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_millis()).unwrap_or(0);
        let mut message = format!("{timestamp} {message}").replace('\0', "?");
        // Bound individual errors without splitting UTF-8.
        if message.len() > 2048 {
            let mut end = 2048;
            while !message.is_char_boundary(end) { end -= 1; }
            message.truncate(end);
        }
        if let Ok(mut history) = HISTORY.try_lock() {
            history.lines.push_back(message.clone());
            while history.lines.len() > 24 || history.lines.iter().map(|s| s.len() + 1).sum::<usize>() >= 8192 {
                history.lines.pop_front();
            }
            let snapshot = history.lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
            history.buffer[..snapshot.len()].copy_from_slice(snapshot.as_bytes());
            history.buffer[snapshot.len()] = 0;
            PHIRAIAD_CRASH_ANNOTATIONS.message2.store(history.buffer.as_ptr() as u64, Ordering::Release);
        }
        append_file(&message);
    }

    pub(super) fn install() {
        INSTALL.call_once(|| {
            let path = std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library").join("PhiraiAd-ios-diagnostic.log"))
                .unwrap_or_else(|| std::env::temp_dir().join("PhiraiAd-ios-diagnostic.log"));
            if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
            let _ = LOG_PATH.set(path);
            // iOS uses our hook instead of the default thread-name/backtrace
            // hook, which can re-enter thread-local state during FFI cleanup.
            std::panic::set_hook(Box::new(|info| {
                // AI workers use catch_unwind. A recovered worker panic must
                // not hide a later fatal main-thread panic.
                let is_main = unsafe { pthread_main_np() } != 0;
                let first = if is_main { &FIRST_MAIN_PANIC } else { &FIRST_BACKGROUND_PANIC };
                if !first.swap(true, Ordering::AcqRel) {
                    let reason = info.payload().downcast_ref::<String>().map(String::as_str)
                        .or_else(|| info.payload().downcast_ref::<&str>().copied())
                        .unwrap_or("non-string panic payload");
                    let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                        .unwrap_or_else(|| "unknown location".to_owned());
                    let thread = if is_main { "main" } else { "background" };
                    let message = format!("PHIRAIAD_FIRST_RUST_PANIC thread={thread} at {location}: {reason}").replace('\0', "?");
                    // Retain at most one message for each thread category.
                    // They remain valid after the hook returns and abort begins.
                    if let Ok(c_string) = CString::new(message.as_bytes()) {
                        let pointer = c_string.into_raw() as u64;
                        if is_main {
                            PHIRAIAD_CRASH_ANNOTATIONS.message.store(pointer, Ordering::Release);
                        } else {
                            let _ = PHIRAIAD_CRASH_ANNOTATIONS.message.compare_exchange(0, pointer, Ordering::AcqRel, Ordering::Acquire);
                        }
                    }
                    stderr(&message);
                    record(&message);
                } else {
                    // Do not overwrite the original panic with the FFI wrapper.
                    stderr("PHIRAIAD_SECONDARY_PANIC: original panic retained in __crash_info");
                }
            }));
            record("SESSION_START iOS diagnostics installed");
        });
    }
}
