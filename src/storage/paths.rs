//! Default, user-configurable data paths. See `docs/paths.md` for the full plan
//! this implements.
//!
//! Everything here is CWD-relative today (`library.db`, `web`, `bibles`/`songs`
//! as siblings of the db) because the only distribution so far is "a folder
//! with the exe in it." That breaks the moment there's a real installer: a
//! per-machine install puts the exe under a read-only Program Files-style
//! directory, so writing `library.db` next to it fails outright. This module
//! resolves where user data actually lives, in an order that keeps today's
//! `cargo run`/portable-folder behavior unchanged until an installer or
//! first-run flow opts a machine into the new behavior (see `resolve` below).

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Small pointer file at a fixed platform location, so the app can find a
/// relocated data directory before it has a database to store that fact in.
/// Written by the "Move Data Directory" settings action or by the installer /
/// first-run setup. Its absence is not an error -- it just means "use the
/// platform default."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub data_dir: PathBuf,
}

/// Where an install pointer, and backend plugins, live -- a location that
/// must be findable before `library.db` exists at all, so it can't itself be
/// inside the (possibly relocated) data directory. Fixed per-platform:
/// `%APPDATA%\OpenSanctuary` on Windows, `~/.config/OpenSanctuary` on Linux,
/// `~/Library/Application Support/OpenSanctuary` on macOS. This is exactly
/// where the backend plugin directory already lived before this module
/// existed -- kept as-is so upgrading doesn't strand anyone's plugins.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenSanctuary")
}

/// Backend plugin directory -- unchanged in location, now computed here
/// instead of via an inline `#[cfg(target_os = "windows")]` block in main.rs.
pub fn plugin_dir() -> PathBuf {
    config_dir().join("plugins")
}

fn install_config_path() -> PathBuf {
    config_dir().join("install.json")
}

/// Platform-appropriate data directory when nothing else overrides it:
/// `%APPDATA%\OpenSanctuary` (Windows), `$XDG_DATA_HOME/opensanctuary`
/// (Linux, default `~/.local/share`), `~/Library/Application
/// Support/OpenSanctuary` (macOS).
pub fn platform_default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenSanctuary")
}

pub fn read_install_config() -> Option<InstallConfig> {
    let path = install_config_path();
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<InstallConfig>(&bytes) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!("Failed to parse {:?}, ignoring: {}", path, e);
            None
        }
    }
}

/// Points the app at a relocated data directory. Does not move any files
/// itself -- callers are responsible for copying existing data first.
pub fn write_install_config(data_dir: &Path) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let cfg = InstallConfig { data_dir: data_dir.to_path_buf() };
    let json = serde_json::to_string_pretty(&cfg).unwrap_or_default();
    std::fs::write(install_config_path(), json)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MoveDataDirResult {
    pub source_dir: PathBuf,
    pub target_dir: PathBuf,
    pub files_copied: usize,
    pub sha_verified: bool,
    pub source_deleted: bool,
}

fn compute_file_sha256(path: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Copies `src` to `dst`, hashing the source as it streams through instead
/// of re-reading it afterward -- one read of `src`, one write of `dst`,
/// versus `std::fs::copy` followed by a separate `compute_file_sha256(src)`
/// pass. The destination is still hashed separately by the caller: that's a
/// genuine second read, needed to confirm what actually landed on disk
/// rather than just what was written to the OS.
fn copy_file_with_source_hash(src: &Path, dst: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::{Read, Write};
    let mut src_file = std::fs::File::open(src)?;
    let mut dst_file = std::fs::File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = src_file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        dst_file.write_all(&buffer[..n])?;
    }
    dst_file.flush()?;
    Ok(hasher.finalize().into())
}

fn collect_dir_files(base: &Path, rel_dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let dir = base.join(rel_dir);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let rel_path = rel_dir.join(entry.file_name());
        if ft.is_dir() {
            collect_dir_files(base, &rel_path, files)?;
        } else if ft.is_file() {
            // Skip SQLite temporary shared-memory files which are transient
            if let Some(ext) = rel_path.extension() {
                if ext == "shm" {
                    continue;
                }
            }
            if rel_path.to_string_lossy().ends_with("-shm") {
                continue;
            }
            files.push(rel_path);
        }
    }
    Ok(())
}

/// Best-effort canonicalization for a path that may not exist yet: walks up
/// to the nearest existing ancestor, canonicalizes *that*, then re-appends
/// the non-existent tail components. Found via a real Windows VM test
/// failure: the previous logic only canonicalized a path that already
/// existed, and otherwise fell back to a raw joined/absolute path with no
/// canonicalization at all. On Linux that near-enough matched a real
/// `canonicalize()` result (no symlinks to resolve, same general shape), so
/// `move_data_dir`'s `starts_with`-based nesting checks worked by
/// coincidence. On Windows, `canonicalize()` prepends a `\\?\` verbatim-path
/// prefix a plain joined path never gets -- so comparing a canonicalized
/// `current_dir` (which almost always already exists) against a
/// not-yet-existing `target_dir` (the common case: you're moving *to* a new
/// location) compared two different representations of the same directory,
/// and `Path::starts_with` silently returned false for a target genuinely
/// nested inside the source.
fn canonicalize_best_effort(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let mut missing_tail: Vec<std::ffi::OsString> = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        if let Ok(canon_ancestor) = std::fs::canonicalize(ancestor) {
            let mut result = canon_ancestor;
            for part in missing_tail.into_iter().rev() {
                result.push(part);
            }
            return result;
        }
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                missing_tail.push(name.to_os_string());
                ancestor = parent;
            }
            // Hit the root (or a bare prefix like "C:\") without finding an
            // existing ancestor -- nothing further to resolve against.
            _ => return absolute,
        }
    }
}

/// Copies existing data files from `current_dir` to `target_dir`, verifies each
/// copied file with SHA-256 checksums, writes `install.json`, and optionally deletes
/// the source directory files if all checksums match.
pub fn move_data_dir(
    current_dir: &Path,
    target_dir: &Path,
    db: Option<&crate::storage::db::Database>,
    delete_source: bool,
) -> std::io::Result<MoveDataDirResult> {
    if target_dir.as_os_str().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Target data directory path cannot be empty",
        ));
    }

    let canon_current = canonicalize_best_effort(current_dir);
    let canon_target = canonicalize_best_effort(target_dir);

    if canon_current == canon_target {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Target directory is identical to the current data directory",
        ));
    }

    if canon_target.starts_with(&canon_current) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Target directory cannot be inside the current data directory",
        ));
    }

    if canon_current.starts_with(&canon_target) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Current data directory cannot be inside the target directory",
        ));
    }

    // Check if target already contains a populated library.db
    let target_db = canon_target.join("library.db");
    if target_db.is_file() {
        let meta = std::fs::metadata(&target_db)?;
        if meta.len() > 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Target directory already contains an existing library.db database",
            ));
        }
    }

    // Collect all files to copy from canon_current
    let mut files_to_copy = Vec::new();
    collect_dir_files(&canon_current, Path::new(""), &mut files_to_copy)?;

    // Ensure target root directory exists
    std::fs::create_dir_all(&canon_target)?;

    let do_copy = || copy_and_verify_files(&canon_current, &canon_target, &files_to_copy);
    match db {
        // Checkpoints, then holds every connection's lock for the whole
        // copy -- so no write can land on a source file after it's already
        // been read into the copy (which would corrupt the snapshot) or,
        // combined with `delete_source` below, after the source is gone
        // (which would silently discard it). See `Database::with_all_connections_locked`.
        Some(database) => database.with_all_connections_locked(do_copy)?,
        None => do_copy()?,
    }

    // Write the new data directory to install.json
    write_install_config(&canon_target)?;

    let mut source_deleted = false;
    if delete_source {
        if db.is_some() {
            // A live database still has these files open, so deleting them
            // now would just unlink the inode out from under it: the
            // running process keeps writing to it successfully, and every
            // one of those writes vanishes forever since they reach neither
            // the deleted original nor the already-copied target. Defer the
            // actual removal to the next process startup instead, once
            // nothing has these files open (see `cleanup_pending_source_dir`).
            write_pending_source_cleanup(&canon_current)?;
        } else {
            delete_source_files_now(&canon_current, &files_to_copy);
        }
        source_deleted = true;
    }

    Ok(MoveDataDirResult {
        source_dir: canon_current,
        target_dir: canon_target,
        files_copied: files_to_copy.len(),
        sha_verified: true,
        source_deleted,
    })
}

/// Copies and SHA-256-verifies every file in `files_to_copy` from
/// `canon_current` to `canon_target`. Split out of `move_data_dir` so it can
/// run either directly or inside `Database::with_all_connections_locked`.
fn copy_and_verify_files(
    canon_current: &Path,
    canon_target: &Path,
    files_to_copy: &[PathBuf],
) -> std::io::Result<()> {
    for rel_path in files_to_copy {
        let src_file = canon_current.join(rel_path);
        let dst_file = canon_target.join(rel_path);

        if let Some(parent) = dst_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let src_hash = copy_file_with_source_hash(&src_file, &dst_file)?;
        let dst_hash = compute_file_sha256(&dst_file)?;

        if src_hash != dst_hash {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "SHA-256 checksum mismatch for {:?}: copy is corrupted",
                    rel_path
                ),
            ));
        }
    }
    Ok(())
}

/// Deletes the already-copied source files (and any now-empty directories)
/// immediately. Only safe when nothing still has them open -- callers with a
/// live `Database` must use `write_pending_source_cleanup` instead.
fn delete_source_files_now(canon_current: &Path, files_to_copy: &[PathBuf]) {
    for rel_path in files_to_copy {
        let src_file = canon_current.join(rel_path);
        let _ = std::fs::remove_file(src_file);
    }
    fn remove_empty_subdirs(dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    remove_empty_subdirs(&entry.path());
                    let _ = std::fs::remove_dir(entry.path());
                }
            }
        }
    }
    remove_empty_subdirs(canon_current);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingSourceCleanup {
    dir: PathBuf,
}

fn pending_source_cleanup_path() -> PathBuf {
    config_dir().join("pending_data_dir_cleanup.json")
}

/// Records that `dir` should be deleted once it's safe to -- i.e. at the
/// start of a fresh process, before anything has reopened files under it.
/// See `move_data_dir`'s `delete_source` handling and `cleanup_pending_source_dir`.
fn write_pending_source_cleanup(dir: &Path) -> std::io::Result<()> {
    let cfg_dir = config_dir();
    std::fs::create_dir_all(&cfg_dir)?;
    let payload = PendingSourceCleanup { dir: dir.to_path_buf() };
    let json = serde_json::to_string_pretty(&payload).unwrap_or_default();
    std::fs::write(pending_source_cleanup_path(), json)
}

/// Finishes a data-directory move's deferred cleanup, if one is pending.
/// Meant to be called once, early at process startup (before anything else
/// touches `current_data_dir`) -- by then the previous process has exited
/// and closed every handle it held, so it's finally safe to actually remove
/// the old directory `move_data_dir` copied everything out of.
pub fn cleanup_pending_source_dir(current_data_dir: &Path) {
    let marker = pending_source_cleanup_path();
    let Some(pending) = std::fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PendingSourceCleanup>(&bytes).ok())
    else {
        return;
    };

    // Never delete the directory we're actually about to run from -- a
    // no-op in the intended flow (the moved-away-from directory can't also
    // be where install.json now points), kept as cheap insurance.
    let canon_current = std::fs::canonicalize(current_data_dir).unwrap_or_else(|_| current_data_dir.to_path_buf());
    let canon_pending = std::fs::canonicalize(&pending.dir).unwrap_or_else(|_| pending.dir.clone());
    if canon_pending == canon_current {
        tracing::warn!(
            "Pending data-dir cleanup target {:?} matches the active data directory; skipping and clearing the marker.",
            pending.dir
        );
        let _ = std::fs::remove_file(&marker);
        return;
    }

    // The process that called `move_data_dir` kept running (and writing)
    // against `pending.dir` for however long it took the operator to
    // actually restart -- its copy at `canon_current` is only a snapshot
    // from the moment of the move. Re-sync every file one more time before
    // deleting anything, so whatever changed in the meantime (schedule
    // edits, settings, event-log rows) survives instead of being silently
    // discarded along with the old directory.
    if let Err(e) = sync_dir_into(&pending.dir, &canon_current) {
        tracing::warn!(
            "Failed to sync pending changes from {:?} into {:?}: {} (skipping cleanup this startup, will retry)",
            pending.dir,
            canon_current,
            e
        );
        return;
    }

    match std::fs::remove_dir_all(&pending.dir) {
        Ok(()) => {
            tracing::info!("Cleaned up old data directory {:?} after a prior move.", pending.dir);
            let _ = std::fs::remove_file(&marker);
        }
        Err(e) => {
            tracing::warn!(
                "Failed to clean up old data directory {:?}: {} (will retry next startup)",
                pending.dir,
                e
            );
        }
    }
}

/// Copies every file under `src_root` into the same relative path under
/// `dst_root`, overwriting whatever is already there. Unconditional (no
/// mtime/size comparison) -- this only ever runs once, at startup, when a
/// pending data-dir cleanup marker exists, so the extra I/O is a small,
/// one-time cost worth paying for certainty over the alternative of a
/// missed write.
fn sync_dir_into(src_root: &Path, dst_root: &Path) -> std::io::Result<()> {
    let mut files = Vec::new();
    collect_dir_files(src_root, Path::new(""), &mut files)?;
    for rel_path in &files {
        let src_file = src_root.join(rel_path);
        let dst_file = dst_root.join(rel_path);
        if let Some(parent) = dst_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&src_file, &dst_file)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// `--db-path` (or another explicit CLI path flag) was given directly.
    Cli,
    /// `install.json` pointed at a relocated data directory.
    InstallConfig,
    /// Either the platform-default data directory already has a
    /// `library.db` in it (an existing installed copy continuing to run),
    /// or nothing pointed anywhere else *and* CWD doesn't look like a real
    /// portable/dev checkout either -- a fresh real install (e.g. launched
    /// from a `.desktop` entry, CWD often just `$HOME`) defaults here rather
    /// than littering CWD with `library.db`/`bibles/`/`songs/`.
    PlatformDefault,
    /// CWD looks like a portable/dev checkout (has `library.db` already, or
    /// a `web/index.html` sibling) and nothing overrides it. Covers `cargo
    /// run` and the existing portable-folder distribution unchanged.
    Cwd,
}

#[derive(Debug, Clone)]
pub struct ResolvedPaths {
    pub db_path: PathBuf,
    pub bibles_dir: PathBuf,
    pub songs_dir: PathBuf,
    pub media_dir: PathBuf,
    pub source: DataDirSource,
}

impl ResolvedPaths {
    /// The directory `library.db` lives in -- what Settings shows as "Data
    /// Directory."
    pub fn data_dir(&self) -> PathBuf {
        self.db_path
            .parent()
            .map(|p| if p.as_os_str().is_empty() { PathBuf::from(".") } else { p.to_path_buf() })
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Where `src/network/ytdlp_updater.rs` keeps its self-managed `yt-dlp`
    /// copy (Windows has no system package manager to install/update it
    /// via, unlike `ffmpeg`'s `.deb`/`pacman`/`dnf` `Recommends`). Under the
    /// data directory, not the install directory: this is mutable state the
    /// app writes and updates itself, not a read-only installed asset --
    /// same reasoning `media_dir`/`bibles_dir`/`songs_dir` already follow.
    pub fn tools_dir(&self) -> PathBuf {
        self.data_dir().join("tools")
    }
}

/// Resolves `db_path`/`bibles_dir`/`songs_dir`/`media_dir`, in precedence
/// order: an explicit CLI flag always wins outright (for the whole group,
/// via `db_path`'s parent, same as today) with `bibles_dir`/`songs_dir`/
/// `media_dir` individually overridable on top of that; otherwise
/// `install.json`; otherwise an already-populated platform-default
/// directory; otherwise today's CWD-relative default.
///
/// This only resolves *where data should live* -- it doesn't create any of
/// it. `Database::new_with_dirs` (called with the result) is what actually
/// creates `bibles_dir`/`songs_dir` if missing; `library.db` is created by
/// SQLite on first open.
pub fn resolve(
    cli_db_path: Option<&str>,
    cli_bibles_dir: Option<&str>,
    cli_songs_dir: Option<&str>,
    cli_media_dir: Option<&str>,
) -> ResolvedPaths {
    let (base_data_dir, source, db_path) = if let Some(p) = cli_db_path {
        let db_path = PathBuf::from(p);
        let base = db_path
            .parent()
            .map(|p| if p.as_os_str().is_empty() { PathBuf::from(".") } else { p.to_path_buf() })
            .unwrap_or_else(|| PathBuf::from("."));
        (base, DataDirSource::Cli, db_path)
    } else if let Some(cfg) = read_install_config() {
        let db_path = cfg.data_dir.join("library.db");
        (cfg.data_dir, DataDirSource::InstallConfig, db_path)
    } else {
        let platform_default = platform_default_data_dir();
        let cwd = PathBuf::from(".");
        // CWD is only trusted when it actually looks like a portable/dev
        // checkout -- an existing library.db (continuing to use wherever
        // data already lives), or a Cargo.toml sibling (a real source
        // checkout; `cargo run`). NOT a `web/index.html` sibling: that
        // layout is *also* exactly what a real install looks like (`web/`
        // sits next to the exe there too, for `resolve_web_dir` to find
        // it), so it doesn't distinguish "portable/dev" from "installed"
        // at all -- and a shell launch defaulting CWD to the install
        // directory (common on Windows) would otherwise make a fresh
        // install look "portable" and write data straight into the
        // (soon to be uninstalled) install directory. A repo checkout
        // never ships Cargo.toml/src, so this doesn't misfire for it.
        let looks_like_portable_checkout =
            cwd.join("library.db").exists() || cwd.join("Cargo.toml").is_file();
        if platform_default.join("library.db").exists() || !looks_like_portable_checkout {
            let db_path = platform_default.join("library.db");
            (platform_default, DataDirSource::PlatformDefault, db_path)
        } else {
            (cwd.clone(), DataDirSource::Cwd, cwd.join("library.db"))
        }
    };

    let bibles_dir = cli_bibles_dir.map(PathBuf::from).unwrap_or_else(|| base_data_dir.join("bibles"));
    let songs_dir = cli_songs_dir.map(PathBuf::from).unwrap_or_else(|| base_data_dir.join("songs"));
    let media_dir = cli_media_dir.map(PathBuf::from).unwrap_or_else(|| base_data_dir.join("media"));

    ResolvedPaths { db_path, bibles_dir, songs_dir, media_dir, source }
}

/// Detects an older portable/dev-style install's `library.db` sitting next
/// to the currently running executable, or in the process's working
/// directory, that this run *isn't* already using -- surfaced once in the
/// first-time setup wizard's "Data directory" step (docs/first-time.md step
/// 2), so migrating from a portable build to a real install doesn't
/// silently orphan existing data behind a brand-new, empty database.
///
/// Returns the containing directory (what `write_install_config` needs),
/// not the db file itself. A zero-byte `library.db` is treated as "nothing
/// here" rather than a legacy install -- that's what a just-created, never
/// opened database looks like, not real content worth offering to adopt.
pub fn detect_legacy_portable_library(active_data_dir: &Path) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.to_path_buf()));
    detect_legacy_portable_library_in(active_data_dir, &PathBuf::from("."), exe_dir.as_deref())
}

/// Testable core of `detect_legacy_portable_library`, with the two
/// candidate directories (CWD, the exe's own directory) passed in explicitly
/// instead of read from process-global state -- same reasoning as
/// `resolve_web_dir`/`resolve_web_dir_near_exe`'s split above.
fn detect_legacy_portable_library_in(active_data_dir: &Path, cwd: &Path, exe_dir: Option<&Path>) -> Option<PathBuf> {
    let canon_active_db = canonicalize_best_effort(&active_data_dir.join("library.db"));

    let mut candidates: Vec<PathBuf> = vec![cwd.to_path_buf()];
    if let Some(dir) = exe_dir {
        candidates.push(dir.to_path_buf());
    }

    for candidate in candidates {
        let db_file = candidate.join("library.db");
        let Ok(meta) = std::fs::metadata(&db_file) else { continue };
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        let canon_candidate_db = canonicalize_best_effort(&db_file);
        if canon_candidate_db == canon_active_db {
            continue;
        }
        return Some(canonicalize_best_effort(&candidate));
    }
    None
}

/// Package name used for the FHS-standard installed location this resolves
/// against (`<prefix>/share/opensanctuary/web`) -- matches the `.deb`'s own
/// asset placement (see `docs/installer.md`).
const FHS_SHARE_DIR_NAME: &str = "opensanctuary";

/// Resolves where the static web UI assets live: an explicit CLI flag always
/// wins; otherwise a `web/` directory next to the running executable (the
/// portable-folder layout, e.g. `dist/windows-x64/web`); otherwise the
/// FHS-standard installed location relative to the executable
/// (`<prefix>/bin/opensanctuary` -> `<prefix>/share/opensanctuary/web`, e.g.
/// `/usr/share/opensanctuary/web` for a `.deb` install); otherwise today's
/// bare `"web"`, resolved against the current working directory (`cargo
/// run`'s dev workflow, unchanged).
///
/// This matters for real installs specifically because a `.desktop`-launched
/// app's working directory is *not* reliably the install directory (unlike
/// `cargo run` or double-clicking the exe in its own portable folder) -- so
/// CWD-relative `"web"` alone silently breaks the moment there's a real
/// installed shortcut, the same category of bug `resolve()` above fixes for
/// the database.
pub fn resolve_web_dir(cli_web_dir: Option<&str>) -> PathBuf {
    if let Some(w) = cli_web_dir {
        return PathBuf::from(w);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = resolve_web_dir_near_exe(&exe) {
            return dir;
        }
    }
    PathBuf::from("web")
}

fn resolve_web_dir_near_exe(exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;

    let adjacent = exe_dir.join("web");
    if adjacent.join("index.html").is_file() {
        return Some(adjacent);
    }

    let bin_parent = exe_dir.parent()?;
    let fhs = bin_parent.join("share").join(FHS_SHARE_DIR_NAME).join("web");
    if fhs.join("index.html").is_file() {
        return Some(fhs);
    }

    None
}

const TV_APK_FILENAME: &str = "opensanctuary-tv.apk";

/// Resolves the bundled Android TV client APK (see the ADB-provisioning
/// plan) using the exact same precedence as `resolve_web_dir` above: a
/// `tv-client/` directory next to the executable (portable-folder layout),
/// then the `.deb`'s FHS layout
/// (`<prefix>/share/opensanctuary/tv-client/opensanctuary-tv.apk`), then
/// `None` -- unlike the web assets, there's no bare-CWD dev fallback,
/// because there's nothing to sensibly default to in a dev checkout that
/// hasn't built the Android app; callers treat `None` as a normal
/// "APK not available" state (`AdbAvailability::apk_available: false` in
/// `src/network/adb.rs`), not an error.
pub fn resolve_tv_apk_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    resolve_tv_apk_path_near_exe(&exe)
}

fn resolve_tv_apk_path_near_exe(exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;

    let adjacent = exe_dir.join("tv-client").join(TV_APK_FILENAME);
    if adjacent.is_file() {
        return Some(adjacent);
    }

    let bin_parent = exe_dir.parent()?;
    let fhs = bin_parent.join("share").join(FHS_SHARE_DIR_NAME).join("tv-client").join(TV_APK_FILENAME);
    if fhs.is_file() {
        return Some(fhs);
    }

    None
}

/// Resolves the `ffmpeg` command to run: a bundled `ffmpeg/ffmpeg.exe` next
/// to the executable (the Windows installer's staged LGPL build --
/// `packaging/build-windows-installer.sh` -- there's no system package
/// manager on Windows to install it via, unlike the `.deb`'s `Recommends`),
/// falling back to the bare command name for `Command::new` to resolve via
/// `PATH` otherwise (the Linux/macOS case, system-installed, unchanged).
/// Never returns `None`: an unresolved bundled copy just means "rely on
/// PATH," same as this codebase's behavior before bundling existed.
pub fn resolve_ffmpeg_command() -> PathBuf {
    let bare = PathBuf::from("ffmpeg");
    match std::env::current_exe() {
        Ok(exe) => resolve_bundled_tool_or_path_name_near_exe(&exe, "ffmpeg"),
        Err(_) => bare,
    }
}

/// Same as `resolve_ffmpeg_command`, for `ffprobe`.
pub fn resolve_ffprobe_command() -> PathBuf {
    match std::env::current_exe() {
        Ok(exe) => resolve_bundled_tool_or_path_name_near_exe(&exe, "ffprobe"),
        Err(_) => PathBuf::from("ffprobe"),
    }
}

fn resolve_bundled_tool_or_path_name_near_exe(exe: &Path, name: &str) -> PathBuf {
    let bare = PathBuf::from(name);
    let Some(exe_dir) = exe.parent() else { return bare };

    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let bundled = exe_dir.join("ffmpeg").join(&filename);
    if bundled.is_file() {
        bundled
    } else {
        bare
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_cli_db_path_wins_and_derives_siblings() {
        let r = resolve(Some("/tmp/somewhere/mylib.db"), None, None, None);
        assert_eq!(r.source, DataDirSource::Cli);
        assert_eq!(r.db_path, PathBuf::from("/tmp/somewhere/mylib.db"));
        assert_eq!(r.bibles_dir, PathBuf::from("/tmp/somewhere/bibles"));
        assert_eq!(r.songs_dir, PathBuf::from("/tmp/somewhere/songs"));
        assert_eq!(r.media_dir, PathBuf::from("/tmp/somewhere/media"));
        assert_eq!(r.data_dir(), PathBuf::from("/tmp/somewhere"));
    }

    #[test]
    fn individual_overrides_apply_on_top_of_cli_db_path() {
        let r = resolve(Some("/tmp/x/lib.db"), Some("/custom/bibles"), None, None);
        assert_eq!(r.bibles_dir, PathBuf::from("/custom/bibles"));
        assert_eq!(r.songs_dir, PathBuf::from("/tmp/x/songs"));
    }

    #[test]
    fn bare_filename_db_path_resolves_siblings_relative_to_cwd() {
        let r = resolve(Some("library.db"), None, None, None);
        assert_eq!(r.db_path, PathBuf::from("library.db"));
        assert_eq!(r.bibles_dir, PathBuf::from("./bibles"));
    }

    #[test]
    fn explicit_cli_web_dir_always_wins() {
        assert_eq!(resolve_web_dir(Some("/custom/web")), PathBuf::from("/custom/web"));
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "os-next-paths-test-{}-{}-{:?}",
            label,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn web_dir_resolves_to_portable_layout_next_to_exe() {
        let root = scratch_dir("portable");
        let exe_dir = root.join("bin");
        std::fs::create_dir_all(exe_dir.join("web")).unwrap();
        std::fs::write(exe_dir.join("web").join("index.html"), "<html></html>").unwrap();

        let found = resolve_web_dir_near_exe(&exe_dir.join("os-next"));
        assert_eq!(found, Some(exe_dir.join("web")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn web_dir_resolves_to_fhs_layout_when_no_adjacent_web_dir() {
        let root = scratch_dir("fhs");
        let bin_dir = root.join("usr").join("bin");
        let web_dir = root.join("usr").join("share").join(FHS_SHARE_DIR_NAME).join("web");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&web_dir).unwrap();
        std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();

        let found = resolve_web_dir_near_exe(&bin_dir.join("opensanctuary"));
        assert_eq!(found, Some(web_dir));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn web_dir_resolution_finds_nothing_when_neither_layout_exists() {
        let root = scratch_dir("none");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let found = resolve_web_dir_near_exe(&bin_dir.join("os-next"));
        assert_eq!(found, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tv_apk_resolves_to_portable_layout_next_to_exe() {
        let root = scratch_dir("tv-apk-portable");
        let exe_dir = root.join("bin");
        std::fs::create_dir_all(exe_dir.join("tv-client")).unwrap();
        std::fs::write(exe_dir.join("tv-client").join(TV_APK_FILENAME), b"fake apk bytes").unwrap();

        let found = resolve_tv_apk_path_near_exe(&exe_dir.join("os-next"));
        assert_eq!(found, Some(exe_dir.join("tv-client").join(TV_APK_FILENAME)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tv_apk_resolves_to_fhs_layout_when_no_adjacent_tv_client_dir() {
        let root = scratch_dir("tv-apk-fhs");
        let bin_dir = root.join("usr").join("bin");
        let tv_dir = root.join("usr").join("share").join(FHS_SHARE_DIR_NAME).join("tv-client");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&tv_dir).unwrap();
        std::fs::write(tv_dir.join(TV_APK_FILENAME), b"fake apk bytes").unwrap();

        let found = resolve_tv_apk_path_near_exe(&bin_dir.join("opensanctuary"));
        assert_eq!(found, Some(tv_dir.join(TV_APK_FILENAME)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tv_apk_resolution_finds_nothing_when_neither_layout_exists() {
        let root = scratch_dir("tv-apk-none");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let found = resolve_tv_apk_path_near_exe(&bin_dir.join("os-next"));
        assert_eq!(found, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detects_a_legacy_library_in_cwd_distinct_from_the_active_one() {
        let root = scratch_dir("legacy-cwd");
        let active_dir = root.join("active");
        let legacy_dir = root.join("legacy");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("library.db"), b"not actually empty").unwrap();

        let found = detect_legacy_portable_library_in(&active_dir, &legacy_dir, None);
        assert_eq!(found, Some(canonicalize_best_effort(&legacy_dir)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detects_a_legacy_library_next_to_the_exe() {
        let root = scratch_dir("legacy-exe");
        let active_dir = root.join("active");
        let exe_dir = root.join("old-portable-build");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("library.db"), b"not actually empty").unwrap();

        // CWD (an unrelated empty dir) has nothing -- only the exe-adjacent one does.
        let unrelated_cwd = root.join("somewhere-else");
        std::fs::create_dir_all(&unrelated_cwd).unwrap();
        let found = detect_legacy_portable_library_in(&active_dir, &unrelated_cwd, Some(&exe_dir));
        assert_eq!(found, Some(canonicalize_best_effort(&exe_dir)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ignores_the_currently_active_library_itself() {
        let root = scratch_dir("legacy-active-is-self");
        let active_dir = root.join("active");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::write(active_dir.join("library.db"), b"the real one in use").unwrap();

        // CWD happens to be the same directory the app is already using.
        let found = detect_legacy_portable_library_in(&active_dir, &active_dir, None);
        assert_eq!(found, None, "must not flag the database this run is already using");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ignores_a_zero_byte_library_db() {
        let root = scratch_dir("legacy-zero-byte");
        let active_dir = root.join("active");
        let legacy_dir = root.join("legacy");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("library.db"), b"").unwrap();

        let found = detect_legacy_portable_library_in(&active_dir, &legacy_dir, None);
        assert_eq!(found, None, "a zero-byte library.db is a just-created empty db, not real content");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_nothing_when_no_candidate_has_a_library_db() {
        let root = scratch_dir("legacy-none");
        let active_dir = root.join("active");
        let cwd = root.join("cwd");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        let found = detect_legacy_portable_library_in(&active_dir, &cwd, None);
        assert_eq!(found, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ffmpeg_command_resolves_to_bundled_copy_next_to_exe() {
        let root = scratch_dir("ffmpeg-bundled");
        let bin_dir = root.join("bin");
        let ffmpeg_dir = bin_dir.join("ffmpeg");
        std::fs::create_dir_all(&ffmpeg_dir).unwrap();
        let filename = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(ffmpeg_dir.join(&filename), b"fake ffmpeg binary").unwrap();

        let found = resolve_bundled_tool_or_path_name_near_exe(&bin_dir.join("opensanctuary"), "ffmpeg");
        assert_eq!(found, ffmpeg_dir.join(&filename));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ffmpeg_command_falls_back_to_bare_name_when_not_bundled() {
        let root = scratch_dir("ffmpeg-not-bundled");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let found = resolve_bundled_tool_or_path_name_near_exe(&bin_dir.join("opensanctuary"), "ffprobe");
        assert_eq!(found, PathBuf::from("ffprobe"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tools_dir_is_a_sibling_of_the_data_dir_contents() {
        let r = resolve(Some("/tmp/somewhere/mylib.db"), None, None, None);
        assert_eq!(r.tools_dir(), PathBuf::from("/tmp/somewhere/tools"));
    }

    #[test]
    fn move_data_dir_copies_and_verifies_sha256() {
        let root = scratch_dir("move-copy");
        let src = root.join("source_data");
        let dst = root.join("target_data");
        std::fs::create_dir_all(src.join("bibles")).unwrap();
        std::fs::create_dir_all(src.join("media").join("images")).unwrap();

        std::fs::write(src.join("library.db"), b"SQLITE-SAMPLE-DATABASE-CONTENT").unwrap();
        std::fs::write(src.join("bibles").join("kjv.db"), b"KJV-BIBLE-CONTENT").unwrap();
        std::fs::write(src.join("media").join("images").join("bg.jpg"), b"IMAGE-JPEG-BYTES").unwrap();

        let res = move_data_dir(&src, &dst, None, false).unwrap();
        assert_eq!(res.files_copied, 3);
        assert!(res.sha_verified);
        assert!(!res.source_deleted);

        assert_eq!(std::fs::read(dst.join("library.db")).unwrap(), b"SQLITE-SAMPLE-DATABASE-CONTENT");
        assert_eq!(std::fs::read(dst.join("bibles").join("kjv.db")).unwrap(), b"KJV-BIBLE-CONTENT");
        assert_eq!(std::fs::read(dst.join("media").join("images").join("bg.jpg")).unwrap(), b"IMAGE-JPEG-BYTES");

        // Source files must still exist when delete_source = false
        assert!(src.join("library.db").exists());
        assert!(src.join("bibles").join("kjv.db").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_data_dir_deletes_source_after_sha_verification() {
        let root = scratch_dir("move-delete");
        let src = root.join("source_data");
        let dst = root.join("target_data");
        std::fs::create_dir_all(src.join("songs")).unwrap();

        std::fs::write(src.join("library.db"), b"DATABASE-TO-MOVE").unwrap();
        std::fs::write(src.join("songs").join("public_domain.db"), b"SONGS-TO-MOVE").unwrap();

        let res = move_data_dir(&src, &dst, None, true).unwrap();
        assert_eq!(res.files_copied, 2);
        assert!(res.sha_verified);
        assert!(res.source_deleted);

        assert_eq!(std::fs::read(dst.join("library.db")).unwrap(), b"DATABASE-TO-MOVE");
        assert_eq!(std::fs::read(dst.join("songs").join("public_domain.db")).unwrap(), b"SONGS-TO-MOVE");

        // Source files must be deleted when delete_source = true
        assert!(!src.join("library.db").exists());
        assert!(!src.join("songs").join("public_domain.db").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_data_dir_rejects_identical_and_nested_targets() {
        let root = scratch_dir("move-guards");
        let src = root.join("source_data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("library.db"), b"TEST").unwrap();

        // Identical target
        let err1 = move_data_dir(&src, &src, None, false);
        assert!(err1.is_err());

        // Target inside source
        let nested = src.join("nested_target");
        let err2 = move_data_dir(&src, &nested, None, false);
        assert!(err2.is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
