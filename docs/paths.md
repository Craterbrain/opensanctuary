# Configurable Default Paths & Storage Architecture

_Last edited: 2026-10-02 20:05_

## Overview & Background

Earlier iterations of the application resolved paths relative to the current working directory (`CWD`), or relative to another CWD-relative path:
- `--db-path` defaulted to `./library.db`
- `--web-dir` defaulted to `./web`
- `--bibles-dir` and `--songs-dir` defaulted to `<db_path's parent>/bibles` and `.../songs`
- Downloaded/searched media (Pexels/Pixabay results, user backgrounds) were written directly into `<web_dir>/media/{images,videos}`

While functional for portable zip distributions where the binary and assets reside together in an unpackaged folder, this model breaks in standard installed desktop environments (such as `.deb` packages or Windows installer setups):
1. **Read-Only Program Content**: Machine-wide installations place binaries and static assets under protected directories (e.g. `/usr/share/opensanctuary/web` or `Program Files`), which regular users cannot write to.
2. **Unpredictable Working Directory**: Launching via desktop shortcuts (`.desktop` entry or Start Menu) sets `CWD` arbitrarily (often `$HOME`), causing the app to litter root user directories with `library.db` and media folders.

To resolve this, OS-Next cleanly separates:
- **Install Directory**: The executable, libraries, and static UI assets (`web/`). Treated as read-only.
- **Config Directory**: Small pointer file (`install.json`) and backend plugins.
- **Data Directory**: The database (`library.db`), Bible translation databases (`bibles/`), song databases (`songs/`), and background media cache (`media/`).

---

## Resolved Paths by Platform

Path defaults are computed using the `dirs` crate (v7.0.0) following standard OS conventions:

| Root | Windows | Linux | macOS |
|---|---|---|---|
| **Install Directory** | `%LOCALAPPDATA%\Programs\OpenSanctuary` (per-user) or `%ProgramFiles%\OpenSanctuary` (machine) | `/usr/bin/opensanctuary` + `/usr/share/opensanctuary/web` | `/Applications/OpenSanctuary.app` |
| **Config Directory** (`install.json`, `plugins/`) | `%APPDATA%\OpenSanctuary` | `~/.config/OpenSanctuary` | `~/Library/Application Support/OpenSanctuary` |
| **Data Directory** (`library.db`, `bibles/`, `songs/`, `media/`) | `%APPDATA%\OpenSanctuary` | `$XDG_DATA_HOME/OpenSanctuary` (default `~/.local/share/OpenSanctuary`) | `~/Library/Application Support/OpenSanctuary` |
| **Media Cache** (`images/`, `videos/`) | `<data dir>\media` | `<data dir>/media` | `<data dir>/media` |

---

## Data Directory Precedence Order

When starting up, `src/storage/paths.rs` (`paths::resolve()`) determines where user data lives according to the following precedence hierarchy:

1. **Explicit CLI Flags (Highest Precedence)**:
   - `--db-path <path>` explicitly pins the database location, and its parent directory becomes the default base for siblings (`bibles/`, `songs/`, `media/`).
   - `--bibles-dir`, `--songs-dir`, and `--media-dir` can individually override their subdirectories on top of `--db-path`.
2. **Relocated Pointer File (`install.json`)**:
   - If present in the platform's **Config Directory** (`config_dir().join("install.json")`), the JSON payload `{ "data_dir": "<custom_path>" }` redirects all runtime data to an alternate directory (e.g., an external drive or shared network mount).
3. **Platform Default Data Directory**:
   - If the platform default directory already contains a `library.db`, or if the current working directory does not appear to be a dev checkout, the platform default data directory is used.
4. **Portable / Dev Mode (Fallback)**:
   - If no `install.json` exists and the current directory contains an existing `library.db` or a `Cargo.toml` file (`cargo run`), the application falls back to CWD-relative paths. This preserves developer ergonomics and portable folder installations.

---

## Web Assets Resolution Order

Static web UI assets are resolved independently in `paths::resolve_web_dir()`:

1. **Explicit CLI Flag**: `--web-dir <path>` always wins.
2. **Adjacent to Binary**: `<exe_dir>/web` (portable bundle layout, e.g. `dist/windows-x64/web/index.html`).
3. **FHS Standard Packaging**: `<bin_parent>/share/opensanctuary/web` (Debian/Linux packages installing binary to `/usr/bin` and static assets to `/usr/share/opensanctuary/web`).
4. **Dev Fallback**: `./web` relative to current working directory.

---

## Media Cache Decoupling

The media cache is completely decoupled from the static web UI assets:
- **Writable Location**: User-downloaded backgrounds (Pexels, Pixabay, imported PPTX/FreeShow assets) and cached video loops are saved to `<data dir>/media/{images,videos}`.
- **Asset Graph**: `AssetGraph::new(&media_dir)` operates against this data directory.
- **HTTP Routing**: Axum routes serve the media directory directly at `/media` via `ServeDir::new(media_dir)`.

---

## Settings UI & Overrides — **implemented**

The settings schema (`web/src/core/settings_schema.ts`) includes a dedicated `storage` category:

- **Data Directory (`dataDirectory`)**: Read-only display showing the currently active resolved data directory path.
- **Move Data Directory (`moveDataDirectory`)**: Settings action that copies the whole data directory (library, Bibles, songs, media) to a selected destination with SHA-256 integrity verification per file, calls `write_install_config` on success, and optionally deletes the original once verified. Uses the native folder picker below when available, falling back to `window.prompt` otherwise. `src/storage/paths.rs::move_data_dir`, `POST /api/system/move-data-dir` (`src/api/routes/system.rs`).
- **Reveal in File Manager (`revealDataDirectory`)**: Settings action that opens the resolved data directory in Explorer/Finder/the Linux file manager, via the same `open::that` primitive the update-installer flow already uses. `POST /api/system/reveal-data-dir`.
- **Bibles Directory (`biblesDirectory`)**, **Songs Directory (`songsDirectory`)**, **Media Cache Directory (`mediaCacheDirectory`)**: Text settings to point each at an external folder (e.g. shared NAS drive). Each gets a native folder-picker "Browse…" button (`DIRECTORY_PICKER_KEYS` in `settings_dialog.ts`, backed by `POST /api/system/pick-folder`), falling back to typing the path directly when there's no native window (browser-tab/headless). Requires app restart to take effect.

Both the native pickers and the reveal action degrade cleanly outside the native desktop webview (a plain browser tab, or `--headless`): the pick-folder/pick-file routes report `{ available: false }` and the frontend falls back to a text prompt; reveal-in-file-manager doesn't depend on `display_manager` at all (it just shells out via the `open` crate), so it works the same way regardless of how the page is being viewed — it opens a window on whichever machine the *server* is running on.

---

## Legacy Import Detection — **implemented**

`paths::detect_legacy_portable_library(active_data_dir)` checks two
locations -- the process's working directory, and the directory the running
executable itself lives in -- for a `library.db` that (a) is non-empty (a
zero-byte file is just a freshly-created, never-opened database, not real
content) and (b) isn't the one this run is already using. This covers the
two realistic ways someone ends up with an orphaned portable-style install:
double-clicking the exe inside its own folder (CWD == exe's directory), or
launching a fresh installed copy from a shortcut whose "Start in" directory
happens to still be the old portable folder.

The result is surfaced as `legacy_library_detected` in `GET /api/server-info`,
and shown in the first-time setup wizard's "Data directory" step
(`docs/first-time.md`) as "Found an existing library at `<path>` — use it
instead of starting fresh?" Accepting calls `POST
/api/system/adopt-legacy-library`, which re-validates the path still has a
real `library.db` server-side, then calls `write_install_config` to point
`install.json` at it -- no file copy happens; the fresh (and still
essentially empty) database this run already created at the platform-default
location is simply left in place, unused, and the legacy directory becomes
the active one after a restart.

## Future Work & Roadmap

No items currently tracked here -- the four capabilities originally listed
(Move Data Directory wizard, native folder pickers, reveal-in-file-manager,
legacy import detection) are all now implemented; see "Settings UI &
Overrides" and "Legacy Import Detection" above.
