use crate::model::{clock, Event, JobSpec};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub fn command(name: &str) -> Command {
    let c = Command::new(name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = c;
        c.creation_flags(0x08000000);
        c
    }
    #[cfg(not(windows))]
    {
        c
    }
}
fn wait(
    child: &mut Child,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
) -> Result<std::process::ExitStatus, String> {
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || timeout.is_some_and(|t| started.elapsed() > t) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) {
                "Отменено"
            } else {
                "Превышено время ожидания процесса"
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(s)) => return Ok(s),
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
}
fn probe(path: &Path, cancel: &AtomicBool) -> Result<Value, String> {
    let mut child = command("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось запустить ffprobe из PATH: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_thread = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let err_thread = thread::spawn(move || {
        let mut b = String::new();
        let _ = err.read_to_string(&mut b);
        b
    });
    let status = wait(&mut child, cancel, Some(Duration::from_secs(60)));
    let bytes = out_thread.join().unwrap_or_default();
    let error = err_thread.join().unwrap_or_default();
    if !status?.success() {
        return Err(format!("ffprobe: {}", error.trim()));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Ответ ffprobe: {e}"))
}
fn number(v: &Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_f64())
        .filter(|n| n.is_finite() && *n >= 0.0)
}
pub fn duration(v: &Value) -> Option<f64> {
    number(&v["format"]["duration"]).or_else(|| {
        v["streams"].as_array().and_then(|a| {
            a.iter()
                .filter_map(|s| number(&s["duration"]))
                .max_by(f64::total_cmp)
        })
    })
}
fn bitrate(v: &Value) -> String {
    number(v)
        .map(|n| format!("{:.0} kbit/s", n / 1000.0))
        .unwrap_or_else(|| "не указан".into())
}
pub fn describe(v: &Value) -> String {
    let mut s = format!(
        "Контейнер: {}\nДлительность: {}\nБитрейт контейнера: {}\n\nДорожки:\n",
        v["format"]["format_long_name"]
            .as_str()
            .or(v["format"]["format_name"].as_str())
            .unwrap_or("неизвестен"),
        duration(v)
            .map(clock)
            .unwrap_or_else(|| "неизвестна".into()),
        bitrate(&v["format"]["bit_rate"])
    );
    if let Some(streams) = v["streams"].as_array() {
        for track in streams {
            s.push_str(&format!(
                "#{} · {} · {} · {}{}\n",
                track["index"],
                track["codec_type"].as_str().unwrap_or("?"),
                track["codec_name"].as_str().unwrap_or("?"),
                bitrate(&track["bit_rate"]),
                track["tags"]["language"]
                    .as_str()
                    .map(|l| format!(" · {l}"))
                    .unwrap_or_default()
            ));
        }
    }
    s
}
pub fn info(path: std::path::PathBuf, tx: Sender<Event>) {
    thread::spawn(move || {
        let result = probe(&path, &AtomicBool::new(false)).map(|v| describe(&v));
        let _ = tx.send(Event::Info { path, result });
    });
}
pub fn run(id: u64, spec: JobSpec, cancel: Arc<AtomicBool>, tx: Sender<Event>) {
    thread::spawn(move || {
        let mut reserved = false;
        let result = (|| -> Result<(), String> {
            let v = probe(&spec.source, &cancel)?;
            if !v["streams"]
                .as_array()
                .is_some_and(|s| s.iter().any(|s| s["codec_type"] == "video"))
            {
                return Err("В исходном файле нет видеодорожки".into());
            }
            let _ = tx.send(Event::Duration {
                id,
                seconds: duration(&v),
            });
            if cancel.load(Ordering::Relaxed) {
                return Err("Отменено".into());
            }
            // Exclusive reservation protects existing files, including source == destination.
            let reservation = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&spec.output)
                .map_err(|e| format!("Нельзя создать {}: {e}", spec.output.display()))?;
            reserved = true;
            drop(reservation);
            let mut child = command("ffmpeg")
                .args(spec.args())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("Не удалось запустить ffmpeg из PATH: {e}"))?;
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            let progress_tx = tx.clone();
            let progress = thread::spawn(move || {
                let mut last = Instant::now() - Duration::from_secs(1);
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(us) = line
                        .strip_prefix("out_time_us=")
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        if last.elapsed() >= Duration::from_millis(100) {
                            let _ = progress_tx.send(Event::Progress {
                                id,
                                seconds: (us / 1_000_000.0).max(0.0),
                            });
                            last = Instant::now();
                        }
                    }
                }
            });
            let (tail_tx, tail_rx) = mpsc::channel();
            let log_tx = tx.clone();
            let stderr_thread = thread::spawn(move || {
                let mut tail = std::collections::VecDeque::new();
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        let _ = log_tx.send(Event::Log(format!("#{id} {line}")));
                    }
                    if tail.len() == 8 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
                let _ = tail_tx.send(tail.into_iter().collect::<Vec<_>>().join("\n"));
            });
            let status = wait(&mut child, &cancel, None);
            let _ = progress.join();
            let _ = stderr_thread.join();
            let status = status?;
            if !status.success() {
                return Err(format!(
                    "ffmpeg: {status}\n{}",
                    tail_rx.recv().unwrap_or_default()
                ));
            }
            Ok(())
        })();
        let cancelled = cancel.load(Ordering::Relaxed);
        let mut result = result;
        if (cancelled || result.is_err()) && reserved {
            if let Err(e) = std::fs::remove_file(&spec.output) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    result = Err(format!(
                        "{}; не удалось удалить незавершённый файл: {e}",
                        result.err().unwrap_or_else(|| "Отменено".into())
                    ));
                }
            }
        }
        let _ = tx.send(Event::Finished {
            id,
            result,
            cancelled,
        });
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_format() {
        let v = serde_json::json!({"format":{"duration":"60.5","format_name":"matroska","bit_rate":"256000"},
            "streams":[{"index":0,"codec_type":"audio","codec_name":"aac","bit_rate":"256000"}]});
        assert_eq!(duration(&v), Some(60.5));
        assert!(describe(&v).contains("aac"));
        assert!(describe(&v).contains("256 kbit/s"));
    }
    #[test]
    fn unknown_duration() {
        assert_eq!(duration(&serde_json::json!({})), None);
    }
}
