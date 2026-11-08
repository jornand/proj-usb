//! Run with cargo test -- --include-ignored (requires ffmpeg and ffprobe in PATH).
use crate::{model::*, process};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
fn fixture(name: &str, seconds: &str) -> (PathBuf, PathBuf) {
    let folder = std::env::temp_dir().join(format!("projector-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let source = folder.join("Исходный фильм с пробелами.mkv");
    let status = process::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=s=64x64:r=5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880",
            "-t",
            seconds,
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-c:v",
            "mpeg4",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    (folder, source)
}
fn inspect(path: &Path) -> serde_json::Value {
    let o = process::command("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(o.status.success());
    serde_json::from_slice(&o.stdout).unwrap()
}
fn await_job(rx: &mpsc::Receiver<Event>) -> (Result<(), String>, bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "Job timed out");
        if let Event::Finished {
            result, cancelled, ..
        } = rx.recv_timeout(Duration::from_secs(10)).unwrap()
        {
            return (result, cancelled);
        }
    }
}
#[test]
#[ignore = "Requires ffmpeg and ffprobe"]
fn real_conversion_matrix() {
    let (folder, source) = fixture("matrix", "2");
    for (index, (tracks, codec)) in [
        (AudioTracks::First, Codec::Aac),
        (AudioTracks::All, Codec::Aac),
        (AudioTracks::First, Codec::Mp3),
        (AudioTracks::All, Codec::Mp3),
        (AudioTracks::First, Codec::Copy),
        (AudioTracks::All, Codec::Copy),
    ]
    .into_iter()
    .enumerate()
    {
        let output = folder.join(format!("Результат {index}.mkv"));
        let (tx, rx) = mpsc::channel();
        process::run(
            index as u64,
            JobSpec {
                source: source.clone(),
                output: output.clone(),
                tracks,
                codec,
            },
            Arc::new(AtomicBool::new(false)),
            tx,
        );
        let (r, c) = await_job(&rx);
        assert!(!c);
        r.unwrap();
        let v = inspect(&output);
        let streams = v["streams"].as_array().unwrap();
        assert_eq!(
            streams.iter().find(|s| s["codec_type"] == "video").unwrap()["codec_name"],
            "mpeg4"
        );
        let audio: Vec<_> = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .collect();
        assert_eq!(audio.len(), if tracks == AudioTracks::All { 2 } else { 1 });
        let expected = match codec {
            Codec::Aac => "aac",
            Codec::Mp3 => "mp3",
            Codec::Copy => "pcm_s16le",
        };
        assert!(audio.iter().all(|s| s["codec_name"] == expected));
    }
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
#[ignore = "Requires ffmpeg and ffprobe"]
fn existing_output_is_not_overwritten() {
    let (folder, source) = fixture("overwrite", "1");
    let output = folder.join("existing.mkv");
    std::fs::write(&output, b"DO NOT OVERWRITE").unwrap();
    let (tx, rx) = mpsc::channel();
    process::run(
        1,
        JobSpec {
            source,
            output: output.clone(),
            tracks: AudioTracks::First,
            codec: Codec::Aac,
        },
        Arc::new(AtomicBool::new(false)),
        tx,
    );
    assert!(await_job(&rx).0.is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"DO NOT OVERWRITE");
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
#[ignore = "Requires ffmpeg and ffprobe"]
fn cancellation_removes_partial_output() {
    let (folder, source) = fixture("cancel", "240");
    let output = folder.join("cancelled.mkv");
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    process::run(
        1,
        JobSpec {
            source,
            output: output.clone(),
            tracks: AudioTracks::All,
            codec: Codec::Aac,
        },
        cancel.clone(),
        tx,
    );
    let start = Instant::now();
    while !output.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(output.exists());
    cancel.store(true, Ordering::Relaxed);
    let (_, cancelled) = await_job(&rx);
    assert!(cancelled);
    assert!(!output.exists());
    std::fs::remove_dir_all(folder).unwrap();
}
