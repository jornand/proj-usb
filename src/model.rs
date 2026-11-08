use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "mp4"
                    | "m4v"
                    | "mkv"
                    | "avi"
                    | "mov"
                    | "wmv"
                    | "flv"
                    | "webm"
                    | "mpg"
                    | "mpeg"
                    | "ts"
                    | "mts"
                    | "m2ts"
                    | "vob"
                    | "3gp"
                    | "ogv"
                    | "asf"
                    | "mxf"
            )
        })
        .unwrap_or(false)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub dir: bool,
    pub relative: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub root: PathBuf,
    pub name: String,
    pub free: Option<u64>,
    pub removable: bool,
    pub network: bool,
}
impl Drive {
    pub fn label(&self) -> String {
        let kind = if self.network {
            "сеть"
        } else if self.removable {
            "USB"
        } else {
            "диск"
        };
        let size = self
            .free
            .map(|v| format!(" · {} free", human_size(v)))
            .unwrap_or_default();
        format!("{} [{}]{}", self.name, kind, size)
    }
}
pub fn human_size(n: u64) -> String {
    let (div, unit) = if n >= 1 << 40 {
        (1_u64 << 40, "ТБ")
    } else if n >= 1 << 30 {
        (1_u64 << 30, "ГБ")
    } else {
        (1_u64 << 20, "МБ")
    };
    format!("{:.1} {}", n as f64 / div as f64, unit)
}
pub fn clock(secs: f64) -> String {
    let n = secs.max(0.0).ceil() as u64;
    format!("{:02}:{:02}:{:02}", n / 3600, n / 60 % 60, n % 60)
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AudioTracks {
    #[default]
    First,
    All,
}
impl AudioTracks {
    pub fn label(self) -> &'static str {
        match self {
            Self::First => "первая",
            Self::All => "все",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Codec {
    Copy,
    #[default]
    Aac,
    Mp3,
}
impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Copy => "без конвертации",
            Self::Aac => "AAC 256 kbit/s",
            Self::Mp3 => "MP3 256 kbit/s",
        }
    }
}
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub source: PathBuf,
    pub output: PathBuf,
    pub tracks: AudioTracks,
    pub codec: Codec,
}
impl JobSpec {
    pub fn args(&self) -> Vec<std::ffi::OsString> {
        let mut a: Vec<std::ffi::OsString> = ["-hide_banner", "-nostdin", "-y", "-i"]
            .into_iter()
            .map(Into::into)
            .collect();
        a.push(self.source.as_os_str().to_owned());
        a.extend(
            [
                "-map",
                "0:v?",
                "-map",
                match self.tracks {
                    AudioTracks::First => "0:a:0?",
                    AudioTracks::All => "0:a?",
                },
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-c:v",
                "copy",
                "-c:a",
            ]
            .into_iter()
            .map(Into::into),
        );
        a.push(
            match self.codec {
                Codec::Copy => "copy",
                Codec::Aac => "aac",
                Codec::Mp3 => "libmp3lame",
            }
            .into(),
        );
        if self.codec != Codec::Copy {
            a.extend(["-b:a", "256k"].into_iter().map(Into::into));
        }
        a.extend(
            ["-progress", "pipe:1", "-nostats"]
                .into_iter()
                .map(Into::into),
        );
        a.push(self.output.as_os_str().to_owned());
        a
    }
    pub fn command_text(&self) -> String {
        std::iter::once("ffmpeg".to_owned())
            .chain(self.args().iter().map(|s| quote(&s.to_string_lossy())))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn quote(s: &str) -> String {
    #[cfg(windows)]
    {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}
pub fn output_path(folder: &Path, name: &str, source: &Path) -> Result<PathBuf, String> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err("Недопустимое имя файла (без пути и расширения).".into());
    }
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Зарезервированное имя Windows.".into());
    }
    let mut file = std::ffi::OsString::from(name);
    if let Some(ext) = source.extension() {
        file.push(".");
        file.push(ext);
    }
    Ok(folder.join(file))
}
pub struct Job {
    pub id: u64,
    pub spec: JobSpec,
    pub duration: Option<f64>,
    pub processed: f64,
    pub cancel: Arc<AtomicBool>,
    pub stopping: bool,
}
impl Job {
    pub fn remaining(&self) -> Option<f64> {
        self.duration.map(|d| (d - self.processed).max(0.0))
    }
}
#[derive(Debug)]
pub enum Event {
    Snapshot {
        side: usize,
        generation: u64,
        result: Result<Vec<Entry>, String>,
    },
    Found {
        generation: u64,
        entries: Vec<Entry>,
    },
    SearchDone {
        generation: u64,
        error: Option<String>,
    },
    Drives(Result<Vec<Drive>, String>),
    Info {
        path: PathBuf,
        result: Result<String, String>,
    },
    Duration {
        id: u64,
        seconds: Option<f64>,
    },
    Progress {
        id: u64,
        seconds: f64,
    },
    Finished {
        id: u64,
        result: Result<(), String>,
        cancelled: bool,
    },
    Deleted(Result<(), String>),
    Log(String),
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_filter() {
        assert!(is_video(Path::new("Film.MKV")));
        assert!(!is_video(Path::new("readme.txt")));
    }
    #[test]
    fn arguments_preserve_video_and_container() {
        let s = JobSpec {
            source: "a b.mkv".into(),
            output: "c d.mkv".into(),
            tracks: AudioTracks::All,
            codec: Codec::Mp3,
        };
        let a: Vec<String> = s
            .args()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(a.windows(2).any(|x| x == ["-c:v", "copy"]));
        assert!(a.windows(2).any(|x| x == ["-map", "0:a?"]));
        assert!(a.contains(&"-y".into()));
        assert_eq!(a.last().unwrap(), "c d.mkv");
    }
    #[test]
    fn filenames() {
        assert!(output_path(Path::new("/tmp"), "../bad", Path::new("x.mp4")).is_err());
        assert!(output_path(Path::new("/tmp"), "CON", Path::new("x.mp4")).is_err());
        assert_eq!(
            output_path(Path::new("/tmp"), "Фильм 1", Path::new("x.MKV"))
                .unwrap()
                .file_name()
                .unwrap(),
            "Фильм 1.MKV"
        );
    }
    #[test]
    fn clock_rounds_up() {
        assert_eq!(clock(3601.1), "01:00:02");
    }
}
