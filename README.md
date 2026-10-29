<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="web/logo-banner.svg">
  <img alt="OpenSanctuary" src="docs/logo-banner-light.svg" width="560">
</picture>

**Free, open-source church presentation software. Lyrics, scripture, media and announcements on every screen, driven from a browser.**

[![Latest release](https://img.shields.io/github/v/release/Craterbrain/opensanctuary?label=release&color=FF5722)](https://github.com/Craterbrain/opensanctuary/releases/latest)
[![CI](https://github.com/Craterbrain/opensanctuary/actions/workflows/ci.yml/badge.svg)](https://github.com/Craterbrain/opensanctuary/actions/workflows/ci.yml)
[![CodeQL](https://github.com/Craterbrain/opensanctuary/actions/workflows/codeql.yml/badge.svg)](https://github.com/Craterbrain/opensanctuary/actions/workflows/codeql.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20Windows-lightgrey)

[**Download**](https://github.com/Craterbrain/opensanctuary/releases/latest) ·
[Install](#install) ·
[Features](#features) ·
[Plugins](#plugins) ·
[Documentation](#documentation) ·
[Build from source](#build-from-source)

</div>

<br>

![The OpenSanctuary operator console: schedule, preview, live output and song library](docs/screenshots/console.png)

OpenSanctuary is the software that drives the screens during a church
service. It is written in Rust with a TypeScript web UI, and it runs as one
small server on one machine. Everything else is a web page: the operator
console, the front-of-house output, the stage monitor, and a phone remote.

> **Status: beta.** The latest release is
> [`v0.4.1`](https://github.com/Craterbrain/opensanctuary/releases/latest).
> There is no 1.0 yet. The scheduling engine, multi-display output, most
> importers and plugins are built and used for real. CCLI reporting and
> the local-network HTTPS certificate setup (Caddy) are built but **not yet
> tested**. The Android TV app is a work in progress and the Roku channel is
> not yet implemented. Each document under [`docs/`](docs/) says plainly
> whether what it describes is implemented or still a design.

> **Testing and bug reports are very welcome.** The most useful thing you can
> do right now is use every feature and hunt for edge cases: odd import
> files, unusual display setups, long services, unexpected input. Please
> [open an issue](https://github.com/Craterbrain/opensanctuary/issues) with
> what you did, what you expected and what happened.

## Why OpenSanctuary

Most church presentation tools are Windows-only desktop apps with per-seat
licensing, a weak multi-display story, and file formats that don't talk to
each other. OpenSanctuary starts from different assumptions:

- **A browser is the client.** Output screens and the remote control are web
  pages served over your LAN. Only the server needs installing.
- **Your library comes with you.** Import from EasyWorship, OpenLP,
  FreeShow, PowerPoint and OpenDocument, so a switch doesn't mean
  rebuilding years of songs and slides.
- **Cross-platform.** One Rust codebase for Linux and Windows.
- **No per-seat licensing.** MIT-licensed. Run it on as many machines as
  your church has.

## Install

Grab the latest build from the
[**Releases page**](https://github.com/Craterbrain/opensanctuary/releases/latest).
Every release has these assets:

| Platform | Asset | Notes |
|---|---|---|
| Windows | `opensanctuary-setup-x64.exe` | Installer with Start Menu entry and uninstaller. Your library is kept on uninstall unless you tick the box. |
| Windows | `opensanctuary-windows-x64-<version>.zip` | Portable, no install. |
| Debian, Ubuntu, Mint | `opensanctuary_<version>-1_amd64.deb` | Installs with `apt`. |
| Any Linux | `opensanctuary-<version>-x86_64.flatpak` | `flatpak install --user <file>` |
| Any Linux | `opensanctuary-linux-x64-<version>.tar.gz` | Fetched and installed by [`packaging/install.sh`](packaging/install.sh). |

macOS has no installer yet. You can [build from source](#build-from-source).

On first launch a setup wizard helps you pull in content: public-domain
hymns, public-domain Bibles, or your own existing library. See
[`docs/first-time.md`](docs/first-time.md).

<details>
<summary><b>Verifying a download</b></summary>

Every release ships `checksums.txt` and a detached `checksums.txt.minisig`
signature. The built-in updater and `install.sh` both verify the signature
against an embedded public key before installing anything. To check by hand:

```sh
minisign -Vm checksums.txt -P RWRYSKNecgUiwS2YaMPmshtR2/ZAQA/XnunXaOBQw/SRkZPyMspLwhGw
sha256sum -c --ignore-missing checksums.txt
```

The key ID is `C12205725EA34858`. It is also published separately in
[`docs/release-signing-key.txt`](docs/release-signing-key.txt), so you can
confirm the key in a binary matches the one the project published. The full
story is in [`docs/update.md`](docs/update.md).

</details>

## Features

### Live production

- A scheduling engine for songs, scripture, media and free-form slides. The
  live and staged state (what's on screen versus queued next) stays in sync
  over WebSocket across every connected display.
- A slide editor with text styling, themes, backgrounds, image and video
  elements, and autofit text.
- Blackout, clear-text and logo-override controls for the live output.
- Video and audio playback with loop control.
- CCLI reporting for licensed songs: [`docs/CCLI_REPORTING.md`](docs/CCLI_REPORTING.md).
  **Not yet tested.**

![The slide builder](docs/screenshots/slide-builder.png)

### Displays and remote control

- **Live Output** for front-of-house and **Stage Foldback** for the
  confidence monitor. They are separate pages, each independently themeable.
- **Remote Control**: a mobile page to run the service from a phone or
  tablet, paired by QR code.
- **Android TV app** ([`apps/android-tv`](apps/android-tv)) for permanent
  displays. **Work in progress.** The goal is for the console to install it
  over ADB and make it the TV's home app, so the TV boots straight into
  OpenSanctuary, paired by a phone scanning the QR code on the TV.
- **Roku channel** ([`apps/roku`](apps/roku)). **Not yet implemented.** Only a
  rough outline exists so far.
- **Broadcast:** point OBS or any capture tool at the Live Output page. See
  [`docs/BROADCASTING.md`](docs/BROADCASTING.md).

### Content and import

| Source | Formats |
|---|---|
| EasyWorship | `.ewsx`, and EasyWorship 5 `.ews` schedules |
| OpenLP | `.sqlite`, `.osj`, `.osz` |
| FreeShow | `.show`, `.project` |
| PowerPoint | `.pptx` |
| OpenDocument Presentation | `.odp` (also exports, so a presentation can leave again) |

Importers bring over slides, backgrounds and arrangements, not just text.
Results vary by source file. OpenSanctuary also has its own formats
(`.ossong`, `.ospres`, `.osservice`, `.osbible`) that carry a song,
presentation, service or Bible together with its pictures, videos and music.
See [`docs/file-formats.md`](docs/file-formats.md).

Bible translations you own work out of the box: point the app at your own
`.db` files. Sixty-four public-domain translations in 30 languages are
hosted in [opensanctuary-bibles](https://github.com/Craterbrain/opensanctuary-bibles)
and downloaded through the **Online Bibles** plugin.

### Extensibility and operations

- A permissioned [plugin system](#plugins). Nothing runs until the operator
  approves it.
- A notification center (the **Notices** button) that collects the app's own
  messages and plugin notices, with per-source muting.
- Network diagnostics: mDNS discovery, hostname-conflict detection, DHCP
  Option 12 broadcasting and multi-interface management.
- Trusted HTTPS on the local network, using Caddy and a Let's Encrypt DNS-01
  certificate, so every device on-site gets a valid certificate with no
  browser warning. This is for certificates only: it does not expose the
  server to the internet, and nobody outside your network can connect. See
  [`docs/LOCAL_HTTPS.md`](docs/LOCAL_HTTPS.md). **Not yet tested.**
- A host-token-gated console API. Only the paired console can change the
  library or run commands. Display and remote clients are read-only or
  explicitly paired.
- Signed, verified auto-updates (Ed25519 via minisign) with a background
  check on startup.

## Plugins

Optional features are plugins. The review screen in **Settings → Plugins**
shows exactly what a plugin asks for (which sites it may reach, whether it
may write to your library, which commands it may run), with a risk level for
each. A plugin that changes on disk drops back to "needs review". A
sandboxed plugin never receives the console's own credentials.

Bundled plugins, all sandboxed and off until you turn them on:

| Plugin | What it does | Where it appears |
|---|---|---|
| **Song Lyrics** | Searches LRCLIB and Genius, and imports lyrics as formatted song slides. | Songs library |
| **Online Bibles** | Downloads public-domain translations and looks up passages online. | Scriptures library |
| **Video Downloader** | Downloads video or audio from YouTube and 1000+ sites with `yt-dlp`, which the app installs and updates itself (signature- and checksum-verified). | Media library, video background picker |
| **Stock Images** | Searches Pexels and Pixabay for free backgrounds, using your own free API key. | Media library |

![The Stock Images plugin](docs/screenshots/stock-media-plugin.png)

You can add your own with **Install plugin…** (a `.zip` package that stays off
until you review it). There are two kinds: sandboxed web plugins, the
recommended kind, and console scripts, which run with full console access and
are marked Critical. There are no native (compiled) plugins.

To write one, start with [`docs/PLUGINS.md`](docs/PLUGINS.md) and
[`docs/PLUGIN_PERMISSIONS.md`](docs/PLUGIN_PERMISSIONS.md). The bundled
plugins in [`web/plugins/`](web/plugins/) are worked examples.

## Documentation

| Topic | Read |
|---|---|
| First-run wizard | [`docs/first-time.md`](docs/first-time.md) |
| Import and file formats | [`docs/file-formats.md`](docs/file-formats.md), [`docs/ewpx-import.md`](docs/ewpx-import.md), [`docs/bible-import.md`](docs/bible-import.md) |
| Public-domain songs | [`docs/public-domain-songs.md`](docs/public-domain-songs.md) |
| Where data lives on disk | [`docs/paths.md`](docs/paths.md) |
| Pairing displays and remotes | [`docs/CLIENT_PAIRING.md`](docs/CLIENT_PAIRING.md) |
| Broadcasting to OBS | [`docs/BROADCASTING.md`](docs/BROADCASTING.md) |
| Trusted local HTTPS (Caddy) | [`docs/LOCAL_HTTPS.md`](docs/LOCAL_HTTPS.md) |
| CCLI reporting | [`docs/CCLI_REPORTING.md`](docs/CCLI_REPORTING.md) |
| Writing plugins | [`docs/PLUGINS.md`](docs/PLUGINS.md), [`docs/PLUGIN_PERMISSIONS.md`](docs/PLUGIN_PERMISSIONS.md) |
| Updates and release signing | [`docs/update.md`](docs/update.md), [`docs/SIGNING.md`](docs/SIGNING.md) |
| Installers and packaging | [`docs/installer.md`](docs/installer.md) |
| Cutting a release | [`docs/workflow.md`](docs/workflow.md), [`docs/keys.md`](docs/keys.md) |
| Open tasks | [`docs/TODO.md`](docs/TODO.md) |

## Build from source

See [`BUILDING.md`](BUILDING.md). It covers prerequisites, the headless-only
build, cross-compiling for Windows, and the optional `ffmpeg` and `yt-dlp`
tools used by video import and the Video Downloader plugin.

### Repository layout

| Path | Contents |
|---|---|
| `src/` | Rust backend: show engine, storage, network, media pipeline |
| `web/` | TypeScript operator UI, the live, stage and remote output pages, and `web/plugins/` |
| `apps/` | Companion display apps for Android TV and Roku |
| `packaging/` | `.deb`, Flatpak and Windows installer build scripts, and `install.sh` |
| `tests/` | Rust integration tests and import-format fixtures |
| `docs/` | Architecture, protocol and feature notes |
| `vms/` | Sandboxed VM test automation, see [`vms/vms.md`](vms/vms.md) |

## License

MIT, see [`LICENSE`](LICENSE). This covers OpenSanctuary's own code only.
Third-party dependencies and their licenses are listed in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

Copyrighted content (modern Bible translations, CCLI-licensed songs, stock
media) is intentionally not included. The public-domain hymns and the
public-domain Bible downloads are, and `BUILDING.md` explains the `bibles/`
folder.

---

_Last edited: 2026-10-29_
