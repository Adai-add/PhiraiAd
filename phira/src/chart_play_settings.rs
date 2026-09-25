//! Atomic per-chart persistence shared by the preview and the song Mods panel.
use anyhow::{Context, Result};
use prpr::{chart_play::ChartPlaySettings, info::ChartInfo};
use std::{io::Write, path::Path};

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing chart parent directory")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn save(local_path: &str, settings: &ChartPlaySettings) -> Result<()> {
    let charts = prpr::dir::Dir::new(crate::dir::charts()?)?;
    let root = charts.join(local_path.replace(':', "_"))?;
    std::fs::create_dir_all(&root)?;
    let dir = prpr::dir::Dir::new(root)?;
    let mut settings = settings.clone();
    settings.normalize();
    if local_path.starts_with(':') {
        atomic_write(&dir.join("replica-play.json")?, &serde_json::to_vec(&settings)?)?;
        if let Some(info) = crate::scene::ASSET_CHART_INFO.lock().unwrap().as_mut() {
            info.replica_play = settings;
        }
        return Ok(());
    }
    let mut info: ChartInfo = serde_yaml::from_slice(&dir.read("info.yml")?)?;
    let (chart_name, bytes) = read_chart(&dir, &info)?;
    if let Some(bytes) = settings.embed_in_chart(&bytes) {
        atomic_write(&dir.join(&chart_name)?, &bytes)?;
    } else {
        // Text/binary formats have no safe arbitrary JSON extension point.
        info.replica_play = settings;
        atomic_write(&dir.join("info.yml")?, serde_yaml::to_string(&info)?.as_bytes())?;
    }
    Ok(())
}

fn read_chart(dir: &prpr::dir::Dir, info: &ChartInfo) -> Result<(String, Vec<u8>)> {
    let mut chart_name = info.chart.clone();
    if !dir.exists(&chart_name)? {
        if let Some(name) = chart_name.strip_suffix(".pec") {
            let json = format!("{name}.json");
            if dir.exists(&json)? {
                chart_name = json;
            }
        }
    }
    let bytes = dir.read(&chart_name)?;
    Ok((chart_name, bytes))
}

pub fn load(local_path: &str) -> Result<ChartPlaySettings> {
    let charts = prpr::dir::Dir::new(crate::dir::charts()?)?;
    let root = charts.join(local_path.replace(':', "_"))?;
    if local_path.starts_with(':') {
        let path = root.join("replica-play.json");
        let mut settings: ChartPlaySettings = if path.exists() {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            // ASSET_CHART_INFO can still belong to the previously played song.
            ChartPlaySettings::default()
        };
        settings.normalize();
        return Ok(settings);
    }
    let dir = prpr::dir::Dir::new(root)?;
    let info: ChartInfo = serde_yaml::from_slice(&dir.read("info.yml")?)?;
    let (_, bytes) = read_chart(&dir, &info)?;
    let mut settings = ChartPlaySettings::from_chart_bytes(&bytes).unwrap_or(info.replica_play);
    settings.normalize();
    Ok(settings)
}
