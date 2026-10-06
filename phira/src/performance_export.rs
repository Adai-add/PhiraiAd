//! Public Downloads export, off the game thread.
use anyhow::{ Context, Result };
use prpr::{ performance, scene::show_message };
use std::{ path::PathBuf, sync::{ mpsc, Mutex } };
static RESULTS: Mutex<Option<mpsc::Receiver<Result<String>>>> = Mutex::new(None);
pub fn start(name: &str) -> bool {
    if RESULTS.lock().unwrap().is_some() {
        return false;
    }
    let (tx, rx) = mpsc::channel();
    if
        !performance::start(
            serde_json::json!({"chart_name":name,"app_version":"v1.3.0","git_revision":env!("GIT_HASH"),"base_revision":"bcdecf3","profiler_revision":"performance-r1","os":std::env::consts::OS,"arch":std::env::consts::ARCH}),
            Box::new(move |report| {
                std::thread::spawn(move || {
                    let _ = tx.send(export(&report));
                });
            })
        )
    {
        return false;
    }
    *RESULTS.lock().unwrap() = Some(rx);
    true
}
pub fn poll() {
    let mut result = RESULTS.lock().unwrap();
    let res = result.as_ref().and_then(|r| {
        match r.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) =>
                Some(Err(anyhow::anyhow!("性能报告导出线程已退出"))),
        }
    });
    if let Some(res) = res {
        *result = None;
        drop(result);
        match res {
            Ok(path) => {
                show_message(format!("性能报告已保存：{path}")).ok();
            }
            Err(e) => {
                tracing::error!(?e,"performance report export failed");
                show_message(format!("性能报告导出失败：{e}")).error();
            }
        }
    }
}
fn export(report: &performance::Report) -> Result<String> {
    let stem = format!("PhiraiAd-performance-{}-{}", report.started_unix_ms, uuid::Uuid::new_v4());
    let json = serde_json::to_vec_pretty(report)?;
    let mut text = format!(
        "PhiraiAd 谱面性能报告\n结束原因：{}\n总时间：{:.3}s\n\n",
        report.reason,
        report.elapsed_seconds
    );
    for (phase, f) in &report.phases {
        text += &format!(
            "{phase}: {}帧，平均FPS {:.2}，P50/P95/P99 {:.2}/{:.2}/{:.2}ms，最慢 {:.2}ms；>16.67/33.33/50/100ms：{}/{}/{}/{}\n",
            f.frames,
            if f.total_ms > 0.0 {
                ((f.frames as f64) * 1000.0) / f.total_ms
            } else {
                0.0
            },
            f.p50_ms,
            f.p95_ms,
            f.p99_ms,
            f.max_ms,
            f.above_16_67_ms,
            f.above_33_33_ms,
            f.above_50_ms,
            f.above_100_ms
        );
    }
    text += &format!(
        "\n修正音频回调：{}次，总计 {:.3}ms，最大 {:.3}ms，超过缓冲时长 {}次（非硬件欠载计数）\n",
        report.audio.corrected_callback_calls,
        report.audio.corrected_callback_total_ms,
        report.audio.corrected_callback_max_ms,
        report.audio.corrected_callbacks_over_buffer_duration
    );
    text += "\nCPU 分阶段（包含子阶段，请勿相加）：\n";
    for (label, s) in &report.cpu_inclusive {
        text += &format!(
            "{label}: {}次，总计 {:.3}ms，平均 {:.3}ms，最大 {:.3}ms\n",
            s.calls,
            s.total_ms,
            s.total_ms / (s.calls.max(1) as f64),
            s.max_ms
        );
    }
    text += &format!("\nGPU 可用：{:?}；GPU 仅含异步采样的噪域通道\n", report.gpu_supported);
    for (label, s) in &report.gpu_sampled {
        text += &format!(
            "{label}: {}样本，平均 {:.3}ms，最大 {:.3}ms\n",
            s.calls,
            s.total_ms / (s.calls.max(1) as f64),
            s.max_ms
        );
    }
    text += &format!(
        "\n采样RSS峰值 {:?} bytes；进程CPU {:?}s\n",
        report.rss_peak_sampled_bytes,
        report.process_cpu_seconds
    );
    for note in &report.notes {
        text += &format!("说明：{note}\n");
    }
    #[cfg(target_os = "android")]
    {
        let jp = crate::play_report_export::export_android_downloads(
            &format!("{stem}.json"),
            &json,
            "application/json",
            "Download/PhiraiAd-Performance"
        );
        let tp = crate::play_report_export::export_android_downloads(
            &format!("{stem}.txt"),
            text.as_bytes(),
            "text/plain",
            "Download/PhiraiAd-Performance"
        );
        if let (Ok(j), Ok(_)) = (&jp, &tp) {
            return Ok(j.clone());
        }
        // Preserve both files even when public export fails; clearly disclose the fallback.
        tracing::warn!(?jp,?tp,"public performance export failed; preserving in app storage");
        let dir = PathBuf::from(crate::dir::play_reports()?).join("Performance");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{stem}.json")), json)?;
        std::fs::write(dir.join(format!("{stem}.txt")), text)?;
        return Ok(format!("公共Downloads写入失败；已保留在 {}", dir.display()));
    }
    #[cfg(not(target_os = "android"))]
    {
        let dir = downloads()?.join("PhiraiAd-Performance");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{stem}.json")), json)?;
        std::fs::write(dir.join(format!("{stem}.txt")), text)?;
        Ok(dir.to_string_lossy().into_owned())
    }
}
#[cfg(not(target_os = "android"))]
fn downloads() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return Ok(
            PathBuf::from(std::env::var_os("USERPROFILE").context("USERPROFILE unavailable")?).join(
                "Downloads"
            )
        );
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?);
        let config = std::env
            ::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        if let Ok(s) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
            for l in s.lines() {
                if let Some(v) = l.strip_prefix("XDG_DOWNLOAD_DIR=") {
                    let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
                    let p = PathBuf::from(v);
                    if p.is_absolute() {
                        return Ok(p);
                    }
                }
            }
        }
        Ok(home.join("Downloads"))
    }
}
