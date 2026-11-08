use crate::model::{AudioTracks, Codec};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub tracks: AudioTracks,
    pub codec: Codec,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source: crate::drives::home(),
            destination: crate::drives::home(),
            tracks: AudioTracks::First,
            codec: Codec::Aac,
        }
    }
}
fn file() -> Result<PathBuf, String> {
    directories::ProjectDirs::from("ai", "ProjectorTools", "ProjectorUSB")
        .map(|d| d.config_dir().join("settings.json"))
        .ok_or_else(|| "Не найден каталог профиля пользователя".into())
}
pub fn load() -> Settings {
    file()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save(settings: &Settings) -> Result<(), String> {
    let file = file()?;
    let folder = file.parent().unwrap();
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if file.exists() {
        std::fs::remove_file(&file).map_err(|e| e.to_string())?;
    }
    std::fs::rename(tmp, file).map_err(|e| e.to_string())
}
