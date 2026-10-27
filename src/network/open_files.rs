//! Opening OpenSanctuary files (`.ossong`, `.ospres`, `.osservice`, `.osbible`) from the desktop: double-click, "Open with", or the command line.
//!
//! The file manager starts the program with the file's path. If OpenSanctuary is already running, that second process must not open a second
//! console on the same library: it hands the path to the running one and exits. Every instance holds a lock file in its data folder for as long
//! as it runs; a process that finds the lock taken writes its paths into `open-requests/` in that folder (a private folder of the user's own
//! data directory, so only that user's processes can ask) and quits. The running instance picks them up within a second.
//!
//! A path never opens anything by itself. It joins a list of pending files that the console takes one by one through the same inspect, copyright
//! pop-up, import steps as the Import dialog (so a copyrighted song still shows its advisory, and a bad file is refused with a reason).

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// The file extensions OpenSanctuary opens this way.
pub const EXTENSIONS: [&str; 4] = ["ossong", "ospres", "osservice", "osbible"];

const LOCK_FILE: &str = "instance.lock";
const REQUEST_DIR: &str = "open-requests";
/// A request older than this is stale (written by a launch whose target never picked it up) and is ignored.
const REQUEST_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(120);

/// Is this a file we open: one of our extensions, and a regular file that exists.
pub fn is_openable(path: &Path) -> bool {
    let ext_ok = path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)));
    ext_ok && path.is_file()
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingFile {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
}

static PENDING: Mutex<Vec<PendingFile>> = Mutex::new(Vec::new());

/// Adds `path` to the files waiting for the console. `None` if it isn't one we open (or is already waiting).
pub fn queue(path: &Path) -> Option<PendingFile> {
    let path = path.canonicalize().ok()?;
    if !is_openable(&path) {
        return None;
    }
    let mut list = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if list.iter().any(|p| p.path == path) {
        return None;
    }
    let file = PendingFile { id: uuid::Uuid::new_v4().to_string(), name: path.file_name()?.to_string_lossy().into_owned(), path };
    list.push(file.clone());
    Some(file)
}

pub fn pending() -> Vec<PendingFile> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Removes a pending file from the list and returns it (the console reads it once).
pub fn take(id: &str) -> Option<PendingFile> {
    let mut list = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let i = list.iter().position(|p| p.id == id)?;
    Some(list.remove(i))
}

/// Takes this data folder's instance lock. `Some(lock)` means this process is the instance (keep the value alive for the whole run);
/// `None` means another OpenSanctuary already runs on this library.
pub fn acquire_instance_lock(data_dir: &Path) -> Option<File> {
    let _ = fs::create_dir_all(data_dir);
    let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join(LOCK_FILE)).ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(std::fs::TryLockError::WouldBlock) => None,
        // A file system that can't lock (some network shares) must not stop the program from starting.
        Err(_) => Some(file),
    }
}

/// Asks the running instance to open these files. Returns how many requests were written.
pub fn hand_off(data_dir: &Path, paths: &[PathBuf]) -> std::io::Result<usize> {
    let dir = data_dir.join(REQUEST_DIR);
    fs::create_dir_all(&dir)?;
    let mut n = 0;
    for p in paths {
        let Ok(abs) = p.canonicalize() else { continue };
        if !is_openable(&abs) {
            continue;
        }
        let tmp = dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&tmp, abs.to_string_lossy().as_bytes())?;
        fs::rename(&tmp, dir.join(format!("{}.req", uuid::Uuid::new_v4())))?;
        n += 1;
    }
    Ok(n)
}

/// Reads and removes the hand-off requests waiting in this data folder, queues the files, and returns them.
pub fn drain_requests(data_dir: &Path) -> Vec<PendingFile> {
    let dir = data_dir.join(REQUEST_DIR);
    let Ok(rd) = fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("req") {
            continue;
        }
        let fresh = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < REQUEST_MAX_AGE);
        let target = fs::read_to_string(&path).ok();
        let _ = fs::remove_file(&path);
        if let (true, Some(target)) = (fresh, target) {
            if let Some(f) = queue(Path::new(target.trim())) {
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("os-open-{}-{}", name, uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_our_files_that_exist_are_openable() {
        let d = temp("ext");
        for (name, ok) in [("a.ossong", true), ("b.OSPRES", true), ("c.osservice", true), ("d.osbible", true), ("e.zip", false), ("f.ossong.txt", false), ("noext", false)] {
            let p = d.join(name);
            fs::write(&p, b"x").unwrap();
            assert_eq!(is_openable(&p), ok, "{name}");
        }
        assert!(!is_openable(&d.join("missing.ossong")));
        let dir_named_like_a_file = d.join("folder.ossong");
        fs::create_dir(&dir_named_like_a_file).unwrap();
        assert!(!is_openable(&dir_named_like_a_file), "a folder is not a file");
    }

    #[test]
    fn a_second_launch_hands_its_file_to_the_running_instance() {
        let data = temp("data");
        let files = temp("files");
        let song = files.join("Old Hymn.ossong");
        fs::write(&song, b"PK").unwrap();
        fs::write(files.join("notes.txt"), b"x").unwrap();

        let first = acquire_instance_lock(&data);
        assert!(first.is_some(), "the first process is the instance");
        assert!(acquire_instance_lock(&data).is_none(), "the second finds the lock taken");

        assert_eq!(hand_off(&data, &[song.clone(), files.join("notes.txt"), files.join("gone.ossong")]).unwrap(), 1, "only the real, openable file is passed on");
        let got = drain_requests(&data);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "Old Hymn.ossong");
        assert!(drain_requests(&data).is_empty(), "a request is read once");

        // The console takes it once.
        let id = got[0].id.clone();
        assert!(pending().iter().any(|p| p.id == id));
        assert_eq!(take(&id).unwrap().name, "Old Hymn.ossong");
        assert!(take(&id).is_none());

        drop(first);
        // Free again once the instance quits. (A child process forked by another test in the same instant briefly holds a copy of the
        // descriptor, and with it the lock, until it execs; so allow a moment.)
        let freed = (0..50).any(|_| {
            let got = acquire_instance_lock(&data).is_some();
            if !got {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            got
        });
        assert!(freed, "the lock is free again when the instance quits");
    }

    #[test]
    fn a_request_naming_something_else_or_left_behind_opens_nothing() {
        let data = temp("req");
        let dir = data.join(REQUEST_DIR);
        fs::create_dir_all(&dir).unwrap();
        // Someone with write access to the folder can only name files of our kinds; anything else is dropped.
        fs::write(dir.join("x.req"), "/etc/passwd").unwrap();
        let evil = temp("evil").join("script.sh");
        fs::write(&evil, b"x").unwrap();
        fs::write(dir.join("y.req"), evil.to_string_lossy().as_bytes()).unwrap();
        assert!(drain_requests(&data).is_empty());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "requests are removed whether or not they were used");
    }
}
