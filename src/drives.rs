use crate::model::{Drive, Event};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
    time::Duration,
};

pub fn home() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_owned())
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}
pub fn current_drive<'a>(path: &Path, drives: &'a [Drive]) -> Option<&'a Drive> {
    drives
        .iter()
        .filter(|d| path.starts_with(&d.root))
        .max_by_key(|d| d.root.components().count())
}
pub fn root(path: &Path, drives: &[Drive]) -> PathBuf {
    if let Some(d) = current_drive(path, drives) {
        return d.root.clone();
    }
    let mut root = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                root.push(c.as_os_str())
            }
            _ => break,
        }
    }
    if root.as_os_str().is_empty() {
        home()
    } else {
        root
    }
}
pub fn watch(tx: Sender<Event>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let _ = tx.send(Event::Drives(enumerate()));
            for _ in 0..20 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
}
#[cfg(windows)]
pub fn enumerate() -> Result<Vec<Drive>, String> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    };
    let bits = unsafe { GetLogicalDrives() };
    if bits == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut out = Vec::new();
    for i in 0..26 {
        if bits & (1 << i) == 0 {
            continue;
        }
        let name = format!("{}:\\", (b'A' + i) as char);
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        // DRIVE_REMOTE=4; querying its free space can block for disconnected shares.
        let network = kind == 4;
        let mut free = 0;
        let size = if !network
            && unsafe {
                GetDiskFreeSpaceExW(
                    wide.as_ptr(),
                    &mut free,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } != 0
        {
            Some(free)
        } else {
            None
        };
        out.push(Drive {
            root: name.clone().into(),
            name,
            free: size,
            network,
            removable: kind == 2,
        });
    }
    Ok(out)
}
#[cfg(target_os = "macos")]
pub fn enumerate() -> Result<Vec<Drive>, String> {
    use std::ffi::CStr;
    let mut ptr: *mut libc::statfs = std::ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut ptr, libc::MNT_NOWAIT) };
    if count <= 0 || ptr.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut out = Vec::new();
    for mount in unsafe { std::slice::from_raw_parts(ptr, count as usize) } {
        let name = unsafe { CStr::from_ptr(mount.f_mntonname.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        if name != "/" && !name.starts_with("/Volumes/") {
            continue;
        }
        let network = mount.f_flags & libc::MNT_LOCAL as u32 == 0;
        let free = if network {
            None
        } else {
            Some((mount.f_bavail as u64).saturating_mul(mount.f_bsize as u64))
        };
        out.push(Drive {
            root: name.clone().into(),
            removable: name.starts_with("/Volumes/") && !network,
            name,
            free,
            network,
        });
    }
    Ok(out)
}
// Linux is supported for development/testing, not a required release target.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn enumerate() -> Result<Vec<Drive>, String> {
    let text = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in text.lines() {
        let p: Vec<_> = line.split_whitespace().collect();
        if p.len() < 3 {
            continue;
        }
        let name = p[1].replace("\\040", " ");
        if name != "/"
            && !name.starts_with("/media/")
            && !name.starts_with("/mnt/")
            && !name.starts_with("/run/media/")
        {
            continue;
        }
        let network = matches!(p[2], "cifs" | "nfs" | "nfs4" | "smbfs" | "fuse.sshfs");
        let free = if network {
            None
        } else {
            std::ffi::CString::new(name.clone()).ok().and_then(|c| {
                let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
                if unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } == 0 {
                    let s = unsafe { s.assume_init() };
                    Some(s.f_bavail.saturating_mul(s.f_frsize))
                } else {
                    None
                }
            })
        };
        out.push(Drive {
            root: name.clone().into(),
            removable: name != "/" && !network,
            name,
            free,
            network,
        });
    }
    Ok(out)
}
