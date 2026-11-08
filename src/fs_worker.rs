use crate::model::{is_video, Entry, Event};
use globset::GlobBuilder;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub fn snapshot(path: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    // ReadDir is consumed and dropped in this worker. No handle is stored in UI state.
    for item in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let item = item.map_err(|e| e.to_string())?;
        let kind = item.file_type().map_err(|e| e.to_string())?;
        let dir = if kind.is_symlink() {
            item.path().is_dir()
        } else {
            kind.is_dir()
        };
        if dir || is_video(&item.path()) {
            entries.push(Entry {
                path: item.path(),
                name: item.file_name().to_string_lossy().into_owned(),
                dir,
                relative: String::new(),
            });
        }
    }
    sort(&mut entries);
    Ok(entries)
}
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
}
pub fn load(side: usize, generation: u64, path: PathBuf, tx: Sender<Event>) {
    thread::spawn(move || {
        let result = snapshot(&path);
        let _ = tx.send(Event::Snapshot {
            side,
            generation,
            result,
        });
    });
}
pub fn search(
    generation: u64,
    root: PathBuf,
    mask: String,
    cancel: Arc<AtomicBool>,
    tx: Sender<Event>,
) {
    thread::spawn(move || {
        let matcher = match GlobBuilder::new(&mask)
            .case_insensitive(true)
            .literal_separator(false)
            .build()
        {
            Ok(g) => g.compile_matcher(),
            Err(e) => {
                let _ = tx.send(Event::SearchDone {
                    generation,
                    error: Some(e.to_string()),
                });
                return;
            }
        };
        let mut pending = vec![root.clone()];
        let mut batch = Vec::new();
        let mut last = Instant::now();
        let mut denied = 0;
        while let Some(folder) = pending.pop() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            // Collect a directory before traversal: no ReadDir handle survives a directory iteration.
            let children: Vec<_> = match fs::read_dir(&folder) {
                Ok(rd) => rd.collect(),
                Err(_) => {
                    denied += 1;
                    continue;
                }
            };
            for child in children {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let child = match child {
                    Ok(c) => c,
                    Err(_) => {
                        denied += 1;
                        continue;
                    }
                };
                let kind = match child.file_type() {
                    Ok(k) => k,
                    Err(_) => {
                        denied += 1;
                        continue;
                    }
                };
                // Never recurse through symlinks: avoids cycles and escapes from the search root.
                if kind.is_dir() {
                    pending.push(child.path());
                } else if !kind.is_symlink()
                    && is_video(&child.path())
                    && matcher.is_match(child.file_name())
                {
                    let path = child.path();
                    let relative = path
                        .parent()
                        .and_then(|p| p.strip_prefix(&root).ok())
                        .unwrap_or(Path::new("."));
                    let relative = if relative.as_os_str().is_empty() {
                        format!(".{}", std::path::MAIN_SEPARATOR)
                    } else {
                        format!("{}{}", relative.display(), std::path::MAIN_SEPARATOR)
                    };
                    batch.push(Entry {
                        name: child.file_name().to_string_lossy().into_owned(),
                        path,
                        dir: false,
                        relative,
                    });
                }
                if batch.len() >= 64 || last.elapsed() >= Duration::from_millis(80) {
                    if !batch.is_empty() {
                        let _ = tx.send(Event::Found {
                            generation,
                            entries: std::mem::take(&mut batch),
                        });
                    }
                    last = Instant::now();
                }
            }
        }
        if !batch.is_empty() {
            let _ = tx.send(Event::Found {
                generation,
                entries: batch,
            });
        }
        let error =
            (denied > 0).then(|| format!("Поиск завершён; недоступных элементов: {denied}"));
        let _ = tx.send(Event::SearchDone { generation, error });
    });
}
pub fn delete(path: PathBuf, tx: Sender<Event>) {
    thread::spawn(move || {
        let result = fs::symlink_metadata(&path)
            .and_then(|m| {
                if m.is_dir() && !m.file_type().is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                }
            })
            .map_err(|e| format!("Удаление {}: {e}", path.display()));
        let _ = tx.send(Event::Deleted(result));
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_filter_and_sort() {
        let p = std::env::temp_dir().join(format!("projector-test-{}", std::process::id()));
        fs::create_dir_all(p.join("folder")).unwrap();
        fs::write(p.join("a.MP4"), b"").unwrap();
        fs::write(p.join("notes.txt"), b"").unwrap();
        let list = snapshot(&p).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].dir);
        // Directory removal proves the snapshot retains no directory descriptors.
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn glob_case_insensitive() {
        let m = GlobBuilder::new("*movie*.mkv")
            .case_insensitive(true)
            .build()
            .unwrap()
            .compile_matcher();
        assert!(m.is_match("MY_MOVIE.MKV"));
    }
}
