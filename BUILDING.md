# Building OpenSanctuary (os-next)

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

Opens the operator window automatically and starts the HTTP server on
`:8080` (HTTPS admin/pairing on `:8443`). Useful flags:

- `--headless --no-open` — server only, no desktop window (for a machine
  acting purely as the presentation server, controlled from another device)
- `--port <N>` / `--https-port <N>`
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

## Tests

```sh
cargo test                 # Rust unit + integration tests
cd web && bun test         # frontend tests
```

Platform-specific code (`src/network/windows.rs`, Linux keyring/DBus paths)
isn't exercised by the above on every OS — see `vms/vms.md` for running
those against real Windows/Linux sandbox VMs.

## Cross-compiling for Windows from Linux

Not required for normal development, but if you need it: the MSVC-free path
is `zig cc` standing in for a mingw-w64 toolchain (real Windows SEH
unwinding, unlike a minimal libgcc_eh stub, which test builds need). See
`vms/windows-test.sh` for a complete working example (env vars, linker
flags, and how `panic = "unwind"` test builds are made to link).
