//! "Restart to finish updating" (Linux).
//!
//! Installing a new `.deb` replaces `/usr/bin/opensanctuary` on disk, but the copy already running keeps the old program in memory
//! until it is restarted (the Windows installer closes and reopens the app itself; the Linux package manager does not). The kernel
//! marks the running program's own path with " (deleted)" once the file under it has been replaced, which is how this tells.
//! A restart is never automatic: the operator chooses the moment (Settings > About), because the app runs live during services.

use std::path::{Path, PathBuf};

const REPLACED_SUFFIX: &str = " (deleted)";

/// Does this `/proc/self/exe` link target say the file it points to was replaced or removed?
pub fn link_marks_replaced(link: &Path) -> bool {
    link.to_string_lossy().ends_with(REPLACED_SUFFIX)
}

/// The path the running program was started from, with the " (deleted)" marker removed. `None` unless that path exists again
/// (the installed replacement) and is a file.
pub fn replacement_path(link: &Path) -> Option<PathBuf> {
    let s = link.to_string_lossy();
    let path = PathBuf::from(s.strip_suffix(REPLACED_SUFFIX)?);
    path.is_file().then_some(path)
}

/// True when the program file this process was started from has been replaced on disk by an update and the replacement is there.
/// Always false off Linux, and inside a Flatpak (its updates replace the whole sandbox deployment, not this path).
pub fn running_binary_replaced() -> bool {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("FLATPAK_ID").is_some() {
            return false;
        }
        std::fs::read_link("/proc/self/exe").ok().and_then(|l| replacement_path(&l)).is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Starts the installed program again, detached, with this process's own arguments, after a short pause so this process has let go of
/// its ports first. The caller exits right after.
#[cfg(target_os = "linux")]
pub fn spawn_replacement() -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let link = std::fs::read_link("/proc/self/exe").map_err(|e| format!("can't tell which program is running: {e}"))?;
    let exe = replacement_path(&link).ok_or_else(|| "the installed program isn't there to start".to_string())?;
    std::process::Command::new("sh")
        .arg("-c")
        // "$0" is the program, "$@" its arguments: nothing is spliced into the shell text, so odd characters in arguments are safe.
        .arg("sleep 2; exec \"$0\" \"$@\"")
        .arg(&exe)
        .args(std::env::args_os().skip(1))
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start the new version: {e}"))
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_replacement() -> Result<(), String> {
    Err("restarting after an update is only needed on Linux".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_link_is_recognised() {
        assert!(link_marks_replaced(Path::new("/usr/bin/opensanctuary (deleted)")));
        assert!(!link_marks_replaced(Path::new("/usr/bin/opensanctuary")));
        assert!(!link_marks_replaced(Path::new("/home/a (deleted) b/opensanctuary")));
    }

    #[test]
    fn the_replacement_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("opensanctuary");
        let link = PathBuf::from(format!("{} (deleted)", exe.display()));
        assert_eq!(replacement_path(&link), None, "nothing installed there yet");
        std::fs::write(&exe, b"x").unwrap();
        assert_eq!(replacement_path(&link), Some(exe.clone()));
        assert_eq!(replacement_path(&exe), None, "a link without the marker isn't a replaced one");
    }

    #[test]
    fn this_test_binary_is_not_reported_as_replaced() {
        assert!(!running_binary_replaced());
    }
}
