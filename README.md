# OpenSanctuary

_Last edited: 2026-10-02 08:28_

A next-generation church presentation engine — the software that drives the
screens during a church service (lyrics, scripture, announcements, video)
— written in Rust with a TypeScript web-based control UI.

## Why this exists

Existing church presentation tools are mostly older Windows-only desktop
apps (EasyWorship, ProPresenter) with per-seat licensing, no real
multi-display story, and formats that don't talk to each other. OpenSanctuary is built around a different set of assumptions:

- **A browser is the client.** The operator console, the Live/Stage/Remote
  output screens, and the mobile remote control are all just web pages
  served over the LAN. No per-machine install for anything but the one
  server.
- **Your existing library should still work.** Real import support for
  EasyWorship (`.ewsx`), OpenLP (`.sqlite`/`.osj`/`.osz`), FreeShow
  (`.show`/`.project`), and PowerPoint (`.pptx`) — moving to OpenSanctuary
  shouldn't mean rebuilding years of songs and slides by hand.
- **Cross-platform, not Windows-only.** One Rust binary; Linux, Windows, and (maybe eventually) macOS from the same codebase.
- **No per-seat licensing.** MIT-licensed. Run it on as many machines as
  your church has.

Status: **alpha**. The core engine, multi-display output, and most import
paths work and are used for real; some documented features
(`docs/CCLI_REPORTING.md`, `docs/TUNNELS.md`) are design docs, not built
yet — each doc under `docs/` says plainly whether it's implemented.

## Features

**Live production**
- A scheduling engine for songs, scripture, media, and free-form slides,
  with live/staged state (what's on screen vs. queued up next) kept in
  sync over WebSocket across every connected display.
- A slide editor with text styling, backgrounds, image/video elements,
  autofit text, and themes.
- Blackout, clear-text, and logo-override controls for the live output.
- Video/audio playback with loop control, and online media search
  (Pexels/Pixabay) for backgrounds.

**Displays & remote control**
- **Live Output** (front-of-house) and **Stage Foldback** (confidence
  monitor) as separate pages, each independently themeable.
- **Remote Control** — a mobile-friendly page to drive the service from a
  phone or tablet, paired via QR code.
- **Android TV client** (`apps/android-tv`) for permanent displays, with
  ADB-driven sideload-and-pair provisioning straight from the console —
  no Play Store listing needed, no typing an IP address on a remote.
  (Roku support is designed but not built yet.)
- Any screen capture tool (OBS Browser Source, etc.) can broadcast the
  Live Output page directly — see `docs/BROADCASTING.md`.

**Content & import**
- Real importers for EasyWorship, OpenLP, FreeShow, and PowerPoint — not
  just "read the text," but slides, backgrounds, and arrangements. YMMV
- Bible lookup against a pluggable provider registry (public-domain
  translations fetchable in-app; point at your own `.db` files for
  anything copyrighted).
- Song lyrics lookup via Genius.
- Online video import via `yt-dlp`, with automatic H.264→VP9 conversion
  for native hardware-decoded playback on the target TV hardware
  (`src/storage/transcode.rs`) — see `BUILDING.md`.

**Extensibility & operations**
- Two independent plugin systems — backend (Rust, `libloading`) and
  frontend (TypeScript) — see `docs/PLUGINS.md`.
- Network diagnostics: mDNS discovery, hostname-conflict detection, DHCP
  Option 12 broadcasting, multi-interface management.
- Host-token-gated console API — only the paired console can mutate the
  library or trigger commands; display/remote clients are read-only or
  explicitly paired.
- Signed, verified auto-updates (Ed25519/minisign) with a background
  check on startup — see `docs/update.md`.

## Releases

Not yet published — see **Status** above. Once this repo is public,
releases will be published at
[github.com/Craterbrain/opensanctuary/releases](https://github.com/Craterbrain/opensanctuary/releases)
as three artifacts per version, all signed:

- **`.deb`** — Debian/Ubuntu/Mint, installs via `apt`.
- **Generic Linux tarball** — Fedora, Arch, anything else; fetched and
  installed by `packaging/install.sh`.
- **Windows installer** (and a portable, no-install zip).

Every release ships a `checksums.txt` and a detached `checksums.txt.minisig`
signature; the app's own auto-updater (`src/network/updater.rs`) verifies
both against an embedded public key before ever applying an update, and
`packaging/install.sh` does the same for a fresh install. See
`docs/update.md` for the full verification story, and
`docs/release-signing-key.txt` (once published) for the production public
key, independent of any single binary.

Release signing public key (minisign, key ID `80DBAA0356AE9867`):

```
RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42
```

Verify a download manually with
`minisign -Vm checksums.txt -P RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42`,
then check the installer's SHA-256 against `checksums.txt`.

## Building from source

See [BUILDING.md](BUILDING.md) — covers prerequisites, the headless-only
build, cross-compiling for Windows, and the optional `yt-dlp`/`ffmpeg`
dependency for online media import.

## License

MIT — see [LICENSE](LICENSE). This covers OpenSanctuary's own code only.

- Third-party dependencies and their licenses are listed in
  [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
- Bundled Bible translations (`bibles/`, not included in this repo — see
  `BUILDING.md`) are public-domain texts (KJV, ASV, World English Bible);
  bundled songs (`songs/public_domain.db`) are public-domain hymns.
  Copyrighted content (modern Bible translations, CCLI-licensed songs,
  stock media) is intentionally not included.

## Repository layout

- `src/` — Rust backend (show engine, storage, network, media pipeline)
- `web/` — TypeScript/HTML operator UI and live/stage/remote output pages
- `apps/` — companion apps (Android TV, Roku) for remote display output
- `packaging/` — `.deb`/Windows installer build scripts, `install.sh`
- `tests/` — Rust integration tests and import-format fixtures
- `docs/` — architecture, protocol, and feature design notes — each one
  states plainly whether what it describes is built yet
- `vms/` — sandboxed VM test automation (see `vms/vms.md`)
