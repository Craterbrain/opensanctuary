# Building OpenSanctuary (os-next)

_Last edited: 2026-10-15_

## Prerequisites

- Rust (stable, 2021 edition) — https://rustup.rs
- [Bun](https://bun.sh) — builds the web UI (TypeScript → bundled JS)
- Linux only, for the native desktop window (`desktop-webview` feature,
  on by default): GTK3 + WebKitGTK 4.1 dev headers.
  - Debian/Ubuntu: `sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev`
  - Arch/CachyOS: `sudo pacman -S gtk3 webkit2gtk-4.1`
  - Not needed if you build headless-only (see below).

## Build

```sh
cd web && bun install && bun run build && cd ..
cargo build --release
```

The binary is `target/release/os-next`. It serves the UI from `web/` — the
web build step above must run first (it writes `web/dist/*.js`).

## Run

```sh
./target/release/os-next
```

Opens the operator window automatically. Two listeners, not interchangeable:
`:8080` is loopback-only (`127.0.0.1`) and exists purely for the native
console window itself -- never reachable from the network, regardless of
firewall rules. `:8443` is HTTPS (self-signed by default) and is the only
plane any other device (a second machine's browser, Remote Control phones,
Android TV, a Stage Foldback monitor) can ever reach this app through --
HTTPS can't be disabled. See `src/main.rs`'s loopback-bind comment, or
`docs/TUNNELS.md` for fronting the HTTPS plane with Caddy for a real
public cert. Useful flags:

- `--headless --no-open` — server only, no desktop window (for a machine
  acting purely as the presentation server, controlled from another device)
- `--port <N>` — the loopback-only console port
- `--https-port <N>` — the network-facing HTTPS port
- `--db-path <path>` — SQLite library DB (default `library.db`, created on
  first run)
- `--bibles-dir <dir>` / `--songs-dir <dir>` — see below

## Headless-only build (no GTK/WebKit dependency)

```sh
cargo build --release --no-default-features --features wgpu-render
```

Or drop `wgpu-render` too for the smallest build (`--no-default-features`)
if you don't need the GPU-accelerated render path either.

## Bible & song content

`bibles/` and `songs/*.db` are **not** included in this repo — most Bible
translations and virtually all contemporary worship songs are copyrighted,
and we don't have redistribution rights. Two options:

1. Point `--bibles-dir`/`--songs-dir` at your own SQLite content (schema:
   see `src/storage/bible_providers.rs` and `src/storage/db.rs`).
2. Use in-app search (Settings → Bibles/Songs) to pull free/public-domain
   content from the configured providers at runtime.

`songs/public_domain.db` (public-domain hymns) *is* included and loads by
default.

## Video tools (ffmpeg + yt-dlp)

Neither is needed to build or run the app. `ffmpeg`/`ffprobe` convert imported H.264 video to
VP9 for native playback; `yt-dlp` is used only by the **Video Downloader** plugin
(`web/plugins/ytdlp/`, turned on in Settings → Plugins). How they're obtained differs by
platform:

- **Windows installer/portable build**: `ffmpeg`/`ffprobe` are bundled directly into the
  installer/portable zip (`packaging/build-windows-installer.sh`'s "Bundled ffmpeg/ffprobe"
  step) — an LGPLv3 static build from
  [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), pinned to a specific dated
  release, not gyan.dev's builds (those are GPLv3). Resolved at runtime by
  `src/storage/paths.rs`'s `resolve_ffmpeg_command()`/`resolve_ffprobe_command()`.
- **`yt-dlp` is never bundled** on any platform: it ships new releases far more often than this
  app does (sites keep breaking extraction), so a frozen copy would go stale within weeks. The
  Video Downloader plugin declares where to get it (the official GitHub release, the per-OS
  asset, the checksums file, a detached GPG signature and the pinned public key
  `web/plugins/ytdlp/yt-dlp-key.asc`), and the host (`src/api/plugin_tools.rs`) installs and
  updates it into `<data dir>/tools/` on request, verifying signature then checksum first. A
  `yt-dlp` already on your `PATH` is used as-is and never replaced. See `docs/PLUGINS.md`.
- **Linux `.deb`**: both are listed as `Recommends` in `Cargo.toml`'s
  `[package.metadata.deb]`, so `apt install ./opensanctuary.deb` pulls them in from the distro's
  own repos.
- **Linux generic tarball / building from source / macOS**: install `ffmpeg` yourself (and
  `yt-dlp` if you'd rather not let the plugin manage its own copy) —
  - Debian/Ubuntu: `sudo apt install ffmpeg yt-dlp`
  - Arch/CachyOS: `sudo pacman -S ffmpeg yt-dlp`
  - Fedora: `sudo dnf install ffmpeg yt-dlp`
  - The distro packages above include libvpx (VP9) and libopus encoder support; a minimal
    custom ffmpeg build might not.

Missing `ffmpeg`/`ffprobe` degrades silently: H.264 video just stays H.264 (larger file, but still
plays fine). Without `yt-dlp` the Video Downloader installs its own copy the first time you use it.

## Tests

```sh
cargo test                 # Rust unit + integration tests
cd web && bun test         # frontend tests
```

Platform-specific code (`src/network/windows.rs`, Linux keyring/DBus paths)
isn't exercised by the above on every OS — see `vms/vms.md` for running
those against real Windows/Linux sandbox VMs.

## Packaging a release

This is for producing the actual `.deb`/tarball/Windows-installer release
artifacts, not for day-to-day development:

- `packaging/build-deb.sh` — Linux `.deb` + generic tarball, both from one
  reproducible Podman container build (`packaging/Dockerfile.linux-build`).
  See `docs/installer.md`'s Linux section.
- `packaging/build-windows-installer.sh` — Windows installer (Inno Setup,
  via Wine) + portable zip, including the bundled ffmpeg step above. See
  `docs/installer.md`'s Windows section.
- `packaging/install.sh` — the end-user-facing installer script for the
  generic Linux tarball (`curl | sh`), not something you run as part of
  building.
- Both scripts also build and bundle the Android TV client APK
  (`apps/android-tv/build_apk.sh`) by default, which needs the Android SDK
  (`packaging/Dockerfile.linux-build` installs it into the container; a
  local Windows-installer build needs it on your own machine). Set
  `SKIP_TV_APK=1` to skip building/bundling it entirely — no Android
  toolchain required, and ADB provisioning then reports "APK not available"
  (`src/storage/paths.rs::resolve_tv_apk_path`). The real `release.yml`
  workflow always builds with `SKIP_TV_APK=1` (see `docs/workflow.md`), so a
  local build with the APK included is a strictly bigger build than a tagged
  release, not a smaller one.

## Cross-compiling for Windows from Linux

Not required for normal development, but if you need it:

```sh
cargo install cargo-xwin
rustup target add x86_64-pc-windows-msvc
cargo xwin build --release --target x86_64-pc-windows-msvc \
  --no-default-features --features desktop-webview
```

[`cargo-xwin`](https://github.com/rust-cross/cargo-xwin) cross-compiles to
real MSVC-ABI Windows via `clang-cl`/`lld-link`, fetching the Windows
SDK/CRT itself — no Windows machine or MSVC license needed. This is the
*only* supported cross-compile path: an earlier mingw-w64-based approach
(including `zig cc` standing in for mingw) is gone for good reason — two
independent mingw toolchains both produced a real access-violation crash
specifically around `WebView2Loader.dll`'s import thunk. The MSVC target
sidesteps it entirely (`webview2-com-sys` statically links
`WebView2LoaderStatic` on MSVC instead of dynamically importing
`WebView2Loader.dll`), see `.cargo/config.toml`'s comment and
`docs/installer.md` for the full story. `packaging/build-windows-installer.sh`
runs this same cross-compile as part of building the real installer
(Inno Setup, under Wine) and portable zip; `vms/windows-test.sh` does the
equivalent for cross-compiled *tests* (`cargo xwin test`), run against a
real Windows sandbox VM — see `vms/vms.md`.
