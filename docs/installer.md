# Plan: installers (Windows, Linux)

_Last edited: 2026-10-01 14:19_

## Current state

There is no installer on either platform. Windows distribution today is
`dist/windows-x64/` — a manually-assembled folder (`os-next.exe`,
`WebView2Loader.dll`, `web/`, `library.db`, `.config/`) that only works
because everything is CWD-relative (see `paths.md`) and the user just...
runs the exe from that folder. Linux distribution (the `v0.1.0-alpha`
release) is the same idea as a `.tar.gz`. Both are fine for a developer's
own machine; neither is something to hand a church volunteer.

## Windows — **installer built and verified; app launches cleanly**

`packaging/opensanctuary.iss` + `packaging/build-windows-installer.sh` build
a real `setup.exe` (Inno Setup 6, compiled via `ISCC.exe` under Wine on
Linux — no Windows VM needed for the build itself, only for testing the
result). Verified for real on `vms/win10`, matching the `.deb`'s rigor:

- Silent install (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`) places files
  correctly, registers a Start Menu shortcut (found via Windows Search as a
  real "App", not just a file), and an Add/Remove Programs uninstall entry.
- Running with zero flags resolves `data_dir` to the real
  `%APPDATA%\OpenSanctuary` and serves the real web UI — confirmed via
  `--headless` and via `/api/server-info`.
- Uninstall removes the install dir and leaves `%APPDATA%\OpenSanctuary`
  (the user's library/settings) completely untouched.
- A real, previously-undiscovered `src/storage/paths.rs` bug got found and
  fixed in the process: the CWD-fallback heuristic checked for a
  `web/index.html` sibling to detect "this is a portable/dev checkout" —
  but that's also exactly what a real install's directory looks like
  (`web/` sits next to the exe there too, for `resolve_web_dir` to find
  it). A shell launch that defaults its working directory to the install
  folder (common on Windows) would have made a brand-new install
  misidentify itself as "portable" and write `library.db` straight into
  the install directory — which the uninstaller then deletes. Fixed to
  check for a `Cargo.toml` sibling instead (unambiguously "this is a
  source checkout", never true for either the `.deb` or this installer's
  layout).

**Former blocker, now fixed: the launch crash was a mingw-w64 problem, not an
app bug -- the build now targets `x86_64-pc-windows-msvc` instead.**

The installed app's native GUI window used to reliably crash on launch with
`Exception code: 0xc0000005` (access violation), right after the "Launching
native desktop webview" log line. This was chased across two independent
cross-compile toolchains before being resolved:

- **Cause #1 (zig-as-mingw, fixed by a dependency pin, since superseded)**:
  Event Viewer's fault offset -> `objdump -p` (ImageBase) -> `addr2line`
  against a **debug** cross-build resolved straight to `<f64 as
  zmij::private::Sealed>::write_to_zmij_buffer`. `serde_json` 1.0.147
  switched its float formatter from `ryu` to a new crate, `zmij`, which
  reliably segfaults when cross-compiled to `x86_64-pc-windows-gnu` via
  zig-as-mingw. Fixed at the time by pinning `serde_json = "=1.0.145"` --
  that pin is harmless to keep (see the `Cargo.toml` comment) but is no
  longer load-bearing now that the build isn't on a mingw target at all.
- **Cause #2 (zig-as-mingw, then reproduced independently on real
  mingw-w64-gcc)**: after the zmij fix, the same exception code still fired,
  at a different fault offset, right at native window/WebView2 creation.
  Under zig-as-mingw, disassembling the crash address showed execution
  landing in garbage bytes in the linker-synthesized `jmp *offset(rip);
  int3...` import-thunk block, with exactly one caller in the whole binary:
  `webview2_com_sys::...::CreateCoreWebView2EnvironmentWithOptions`, which
  is `#[link(name = "WebView2Loader.dll")]` on non-MSVC targets -- a real
  dynamic PE import against the bundled `WebView2Loader.dll`, not a system
  DLL. Switching to the *real* `mingw-w64-gcc`/`binutils` package (not
  zig's bundled lld) still crashed, in a different but related way: parsing
  the `MINIDUMP_EXCEPTION_STREAM` out of a real crash dump (by hand, no
  Windows debugger -- `MINIDUMP_HEADER` -> stream directory -> exception
  record, all just `struct.unpack_from` over the raw bytes) showed the
  faulting address was a **bare RVA with no image base added**, landing
  inside `.idata` right by the same DLL's import entries -- i.e. something
  jumped through an import-table-relative address as if it were absolute.
  Two *independent* mingw-w64 toolchains breaking on the same
  `WebView2Loader.dll` dynamic-import path is strong evidence this is a
  structural mingw-w64 + WebView2/COM weak spot, not a one-off linker bug.
- **Fix: switch to `x86_64-pc-windows-msvc` via `cargo-xwin`.** `cargo
  install cargo-xwin` + `rustup target add x86_64-pc-windows-msvc` gives a
  real MSVC-ABI cross-compiler (LLVM `clang-cl`/`lld-link` plus a fetched
  copy of the Windows SDK/CRT -- no Windows machine, no MSVC license, no
  Wine needed for the build itself). On `target_env = "msvc"`,
  `webview2-com-sys` statically links `WebView2LoaderStatic` instead of
  dynamically importing `WebView2Loader.dll` (see
  `webview2-com-sys-0.30.0/src/Microsoft.rs`) -- the entire code path both
  crashes lived in is simply never emitted. Confirmed live on `vms/win10`:
  native window opens, WebView2 renders the real first-run wizard, the
  process stays up and responsive. One fewer runtime file too: no
  `WebView2Loader.dll` to bundle in the installer at all.
  `.cargo/config.toml` sets `-C target-feature=+crt-static` for this
  target, confirmed necessary: without it the exe requires
  `VCRUNTIME140.dll`/`VCRUNTIME140_1.dll` (the VC++ Redistributable), which
  a stock Windows 10 VM doesn't have -- static linking makes the exe fully
  self-contained, same guarantee the old mingw build gave for free.
- **Follow-on bug this fix surfaced, also fixed**: once WebView2 actually
  initialized (which it never had before, on either mingw toolchain), it
  defaulted its user-data folder to `<exe dir>\<exe name>.WebView2` --
  inside the install directory, which the uninstaller doesn't know about
  and doesn't remove, and which a non-writable per-machine install couldn't
  even create. Fixed in `src/webview/mod.rs` by constructing an explicit
  `wry::WebContext` pointed at `data_dir.join("webview2")` (i.e.
  `%APPDATA%\OpenSanctuary\webview2`) and passing it to every
  `WebViewBuilder` on non-Linux (`with_web_context`) -- `launch_desktop_webview_sync`
  now takes the resolved data dir as a parameter. Verified live: a fresh
  install -> launch -> close -> uninstall cycle leaves nothing but the
  self-deleting `unins000.exe` behind in the install dir (standard Inno
  behavior, not a leftover), and `%APPDATA%\OpenSanctuary\webview2` holds
  the WebView2 profile instead.

`--headless` mode was unaffected by either crash the whole time -- server,
all API routes, and the manual update flow always worked; only native
window creation ever reached the broken code path.

### Tooling: Inno Setup

Recommend **Inno Setup** over WiX/MSIX or NSIS:

- Single declarative `.iss` script, free, no license cost, huge install
  base (VS Code, many others use it).
- Produces one `setup.exe` — simplest possible thing to host as a GitHub
  Release asset and point `update.md`'s updater at.
- Native support for both per-user (no UAC) and per-machine (UAC) install
  modes from the same script (`PrivilegesRequiredOverridesAllowed`).
- Silent-install flags (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`) that
  `update.md`'s auto-updater needs to drive it non-interactively.
- Can close a running instance of the app before overwriting its files
  (`CloseApplications`/`AppMutex` directives) — needed since Windows won't
  let you overwrite a running exe.
- Buildable from the existing Windows sandbox VM (`vms/`) via WinRM, same
  as everything else in there — `ISCC.exe` is just another exe to push and
  run. (It also runs under Wine on Linux in principle; start with running
  it on the real VM since that's already working infrastructure.)

WiX/MSIX would be the better choice if this ever needs enterprise
deployment (Group Policy, Intune) — not a near-term need for a church
presentation tool; revisit if that changes.

### Install mode

Default to **per-user** install (`%LOCALAPPDATA%\Programs\OpenSanctuary`,
no admin prompt) — matches modern installer UX (VS Code, Discord, Slack)
and matters for church volunteers who often don't have admin rights on the
machine at the sanctuary. Offer per-machine (`%ProgramFiles%`) as an
explicit option for the admin/IT-managed case. This is exactly the
install-dir split `paths.md` already plans for.

### What the installer does

1. Copies the **runtime-only** subset of files to the install dir: exe,
   `WebView2Loader.dll`, `web/index.html` + `web/*.html` + `web/dist/*` +
   `web/style.css` + `web/*.svg` + `web/plugins/` + `web/songs` (public
   domain) — **not** `web/src`, `web/tests`, `web/build.sh`,
   `web/package.json`, `web/bun.lock`, `test1.ew*` at the repo root (those
   are dev-only; `test1.ewsx` specifically should still ship since the
   in-app "Load Sample Schedule" feature reads it — confirm the exact
   runtime file list against `web/src/app_ui.ts`'s fetch of
   `/test1.ewsx` before finalizing).
2. Does **not** touch the data dir — first run creates it (`first-time.md`).
3. Creates a Start Menu shortcut; Desktop shortcut is a checkbox
   (default off, matching Inno Setup convention).
4. Registers with Add/Remove Programs, including an uninstaller.
5. Version metadata (`VersionInfoVersion`, the `AppId` GUID for
   upgrade-in-place detection) must match the built exe's
   `CARGO_PKG_VERSION` and the git tag driving the release — see
   `update.md` for why this matters for update-checking.

### Bundled ffmpeg + self-managed yt-dlp — **implemented**

Windows has no system package manager to install these optional
dependencies via the way Linux's `.deb` does (`Recommends: ffmpeg, yt-dlp`
in `Cargo.toml`), so the Windows build handles both automatically instead
of leaving it to the operator (`BUILDING.md`'s "Online media import"
section has the full picture across all platforms):

- **ffmpeg/ffprobe are bundled directly** — an LGPLv3 static build from
  [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), pinned to a
  specific dated release tag (`packaging/build-windows-installer.sh`, not
  the rolling "latest" alias, so builds stay reproducible), staged into
  both the installer and the portable zip's `ffmpeg\` folder alongside
  `LICENSE-LGPLv3.txt` and a `SOURCE.txt` pointer to the exact build and
  upstream commit, as LGPLv3 requires. Not gyan.dev's builds — those are
  GPLv3 regardless of variant. LGPL covers everything this app actually
  uses (every decoder is ffmpeg's own; VP9/Opus encoding uses libvpx/
  libopus, both BSD-licensed) since this app never touches a GPL-only
  encoder like libx264/libx265. Resolved at runtime by
  `src/storage/paths.rs`'s `resolve_ffmpeg_command()`/
  `resolve_ffprobe_command()` (bundled copy next to the exe, falling back
  to `PATH` otherwise — the Linux case, unchanged).
- **yt-dlp is fetched and kept current by the app itself, not bundled** —
  `src/network/ytdlp_updater.rs` downloads the latest release from
  `yt-dlp/yt-dlp` on startup and every 12h after, verifying it with GPG
  (yt-dlp signs `SHA2-256SUMS` with RSA-4096, not minisign) against a
  public key pinned in the binary rather than fetched live from yt-dlp's
  own repo at check time — fetching the key from the same place as the
  payload would let a compromised yt-dlp GitHub account swap both
  together. Installed into a self-managed copy under the data directory
  (`ResolvedPaths::tools_dir()`); an existing system-installed `yt-dlp` on
  `PATH` is left alone rather than shadowed. Bundling a frozen copy instead
  wasn't viable: yt-dlp ships new releases far more often than this app
  does, specifically because sites keep changing and breaking extraction,
  so a copy frozen at install time would likely stop working against real
  sites within weeks to months.

**Verified for real**: ran the actual cross-compiled Windows binary under
Wine, both cold (nothing in the videos folder or tools directory yet) and
hot (while already running) — confirmed the bundled ffmpeg/ffprobe are
found via the real `current_exe()`-relative resolution (not just unit
tests against a synthetic directory layout), a real H.264 sample gets
converted to real VP9 through the bundled binary, and a real `yt-dlp.exe`
gets bootstrap-fetched, GPG-verified, and installed into the tools
directory with the correct version marker and executable permissions, all
without any network tooling pre-installed in the Wine prefix beyond what
the app itself ships.

### WebView2 Runtime dependency

The app links `WebView2Loader.dll`, which requires the Evergreen WebView2
Runtime on the target machine. Windows 11 ships it; Windows 10 often
doesn't. **Verify the current detection approach against Microsoft's docs
at implementation time** (registry-key-based detection has shifted before)
— don't hardcode a registry path here without checking. Plan:

- Installer checks for an existing runtime.
- If absent, bundle and silently run Microsoft's small (~2MB) Evergreen
  Bootstrapper as part of setup, rather than just linking to a download
  page and hoping.

### Firewall — **implemented**

Only the HTTPS plane binds `0.0.0.0` (LAN reachability is the point — Live
Output on a second machine, Roku/Android TV discovery, mobile remote
control); the other port is bound to `127.0.0.1` only, for the native
console window itself, and is never reachable from the network regardless
of firewall rules (see `src/main.rs`'s loopback-bind comment).

`packaging/opensanctuary.iss`'s `[Run]` section pre-registers a Windows
Defender Firewall rule (`netsh advfirewall firewall add rule name="OpenSanctuary
HTTPS" dir=in action=allow protocol=TCP localport=8443-8453
program="...\OpenSanctuary.exe"`) covering the whole fallback port range
`src/main.rs` tries at boot (8443 default, up to +10 if that's taken), not
just the default port alone. `[UninstallRun]` removes it again on uninstall.

This only runs when the installer itself is elevated (`Check:
IsAdminInstallMode`) — the default per-user/lowest-privilege install has no
rights to add firewall rules, and Inno doesn't fail the install if the
command errors or is skipped. Defender's own first-launch prompt is still
there as a fallback either way, so a per-user install loses nothing — it
just doesn't get the smoother elevated-install experience.

### Code signing

Unsigned `setup.exe` triggers SmartScreen ("Windows protected your PC").
Acceptable for an alpha; a real certificate costs money and requires
vetting. Worth investigating free code-signing for open-source projects
(e.g. SignPath.io) before any wider release, but explicitly out of scope
here.

### Uninstall

- Must stop a running `os-next.exe` before removing install-dir files.
- Must **not** delete the data dir by default (library, Bible/song content,
  settings) — offer an explicit, unchecked-by-default "Also delete my
  library and settings" checkbox on the uninstall flow, matching the
  convention most desktop apps use.

### Build pipeline — **implemented**

`packaging/build-windows-installer.sh`: builds web assets, cross-compiles
the release exe (`cargo xwin build --target x86_64-pc-windows-msvc`, see
"Former blocker, now fixed" above for why `-msvc` and not a mingw target),
stages the runtime-file subset, and compiles `packaging/opensanctuary.iss`
via `ISCC.exe` — run
under **Wine on Linux**, not the Windows VM (simpler than the originally-
planned WinRM push-and-run: Inno Setup's compiler is a well-behaved native
Win32 app with no GUI dependency, installs cleanly with
`wine <installer> /VERYSILENT`, and needs no network access during the
compile itself). The Windows VM (`vms/win10`) is still what actually
*tests* the result — building and testing don't need to happen on the same
machine. This is what a release workflow (manual today, GitHub Actions
eventually per `update.md`) would invoke to produce the asset attached to
a GitHub Release.

## Linux

Two artifacts, covering different audiences, both built from the same
release (and the same Podman container -- see `update.md`): a **`.deb`**
for Debian/Ubuntu/Mint (the `vms/mint` sandbox VM is exactly this family —
real coverage, not guesswork) where `apt` already solves the dependency
problem Windows needs the WebView2 bootstrapper for; and a **generic
`.tar.gz`** for everyone else (Arch, Fedora, anything not `apt`-based),
since there's no single Linux package format with anywhere near
Windows/macOS's reach. `install.sh`, a friendlier wrapper around fetching
and placing that tarball, is built — see below.

### `.deb` — tooling: cargo-deb — **implemented**

[`cargo-deb`](https://github.com/kornelski/cargo-deb) (`cargo install
cargo-deb`, no sudo needed) builds a `.deb` directly from Cargo project
metadata — no separate packaging script/DSL to maintain in parallel with
`Cargo.toml`. `[package.metadata.deb]` in `Cargo.toml`:

- `depends`: `libwebkit2gtk-4.1-0, libgtk-3-0, libjavascriptcoregtk-4.1-0` —
  `apt install ./opensanctuary.deb` resolves and installs these
  automatically. This is the one part of the whole installer story Windows
  can't do as cleanly (no equivalent of "the package manager just handles
  it" for WebView2 there).
- `recommends`: `ffmpeg, yt-dlp` — both optional (`BUILDING.md`'s "Online
  media import" section), but Debian's `Recommends` is opt-out: `apt
  install` pulls them in by default, `--no-install-recommends` or removing
  either later doesn't break the app — the Media tab's "import from URL"
  just stops working (no `yt-dlp`), or imported H.264 video doesn't get
  converted to VP9 (no `ffmpeg`).
- Layout ended up standard FHS rather than the `/opt/opensanctuary` this
  plan originally sketched: `/usr/bin/opensanctuary` (binary, renamed from
  the crate's `os-next`) + `/usr/share/opensanctuary/web/` (static assets)
  + `/usr/share/applications/opensanctuary.desktop` +
  `/usr/share/icons/hicolor/scalable/apps/opensanctuary.svg`. Same
  runtime-only file subset the Windows section defines (no `web/src`,
  `web/tests`, etc.) — and deliberately *not*
  `songs/public_domain.db`: it would sit under this read-only path, but
  `songs_dir` resolves to the per-user data directory (`paths.md`), so
  SQLite would just create an empty db there instead of ever reading it.
  Bundling it here would silently do nothing; see `Cargo.toml`'s comment.
- `packaging/opensanctuary.desktop` (icon field points at the installed
  hicolor icon above).
- `packaging/build-deb.sh` runs the whole pipeline: `bun run build`
  (web assets) → `cargo build --release` → `cargo deb --no-build`.

This FHS layout is also what made `web_dir`/data-dir resolution's real gap
concrete rather than theoretical: a `.desktop`-launched app's CWD isn't the
install directory the way `cargo run`'s or a portable folder's is. Both
`resolve_web_dir()` and `resolve()`'s CWD-fallback branch in
`src/storage/paths.rs` were fixed as part of building this, not left as
`paths.md`'s original "known gap."

**Verified for real**, not just built: pushed the `.deb` to the `vms/mint`
sandbox VM, `apt-get install` resolved dependencies and installed cleanly,
running it with zero flags from `$HOME` (simulating a real launcher)
correctly served the UI and put data under `~/.local/share/OpenSanctuary`
rather than littering `$HOME`, and `apt-get remove` left that data
directory untouched. Also verified the full public-download path: a
disposable public test repo
([Craterbrain/opensanctuary-release-testing](https://github.com/Craterbrain/opensanctuary-release-testing)),
a real signed release (test key, see `update.md`), anonymous `curl` fetch,
checksum + signature verification, then the exact downloaded `.deb`
installed the same way.

Install/uninstall: `apt install ./opensanctuary_*.deb`,
`apt remove opensanctuary`. Same rule as Windows — uninstall removes the
install dir, never the data dir (`paths.md`'s
`$XDG_DATA_HOME/opensanctuary`) — `apt remove` (as opposed
to `apt purge`) naturally leaves anything outside the package's own file
list alone, which the data dir already is by construction.

### `install.sh` — everyone else — **implemented**

The tarball itself is built now (`opensanctuary-linux-x64-<version>.tar.gz`,
produced by `packaging/build-deb.sh` alongside the `.deb`, from the same
Podman container build -- see `update.md`'s "Linux — built" section).
`packaging/install.sh` is the friendly `curl | sh` wrapper around fetching
and placing it.

A single downloadable POSIX `sh` script (the same shape as rustup's or
Homebrew's installers) for distros without a native package built for them:

1. Detects arch (currently just `x86_64` — flag ARM/aarch64 as future
   work, not a blocker for the alpha) and downloads the matching release
   tarball from `RELEASES_REPO`/`RELEASE_PUBLIC_KEY_B64`'s exact values,
   mirrored as shell variables at the top of the script with a comment to
   swap both together once the real repo goes public.
2. Verifies it against `checksums.txt`/`checksums.txt.minisig`
   (`update.md`'s signing plan) before extracting anything, via the real
   `minisign` CLI against the same embedded Ed25519 public key the Rust
   updater uses — a `curl | sh` script is exactly the kind of thing that
   should be paranoid about what it just downloaded, not just about being
   paranoid about itself (see below). Requires `curl`, `tar`, `sha256sum`,
   `jq`, and `minisign` on `PATH`; missing any of them is a clear error
   naming which one and the install command for Debian/Fedora/Arch, not a
   confusing failure partway through.
3. Installs per-user by default, no `sudo` needed
   (`~/.local/share/opensanctuary-install` for the install dir content,
   `~/.local/bin/opensanctuary` as a symlink to the binary inside it —
   matches the Windows per-user default and the same reasoning: church
   volunteers often don't have root/admin on the machine at the sanctuary).
   **Deliberately not** `~/.local/share/opensanctuary` as originally
   sketched here: the app's own data directory (`paths.md`) is
   `~/.local/share/OpenSanctuary` — different only by case, which a
   reinstall's `rm -rf` on the install dir should never be one filesystem
   case-sensitivity quirk away from deleting. A `--system` flag installs to
   `/opt/opensanctuary` + `/usr/local/bin` instead (no collision risk there
   since the data dir is always under a user's home or `$XDG_DATA_HOME`),
   calling `sudo` itself if not already root.
4. Writes a `.desktop` file to `~/.local/share/applications/` (or
   `/usr/share/applications/` for `--system`), `Icon=` pointing at the
   tarball's own bundled `web/favicon.svg` (absolute path — no separate
   icon assets shipped in the tarball the way the `.deb` bundles the
   hicolor icon set).
5. Idempotent: re-running it is how you'd manually update before
   `update.md`'s Linux auto-apply exists for the tarball case (still
   explicitly out of scope there) — a `VERSION` file dropped in the install
   dir lets a re-run skip straight to "already up to date" without
   redownloading, `--force` overrides. This script becomes the natural
   thing a future Linux updater shells out to, worth keeping in mind in how
   it's structured even though it's not required to do anything
   auto-update-specific yet.

Honest caveat documented in the script's own header, not hidden:
`curl -fsSL .../install.sh | sh` means trusting the script itself sight
unseen, on top of whatever it downloads — the payload verification in
step 2 doesn't cover the script's own contents. Publish the same command
with an explicit "download and inspect first" alternative
(`curl -fsSL ... -o install.sh`, read it, `sh install.sh`) in the docs
rather than only offering the pipe form.

**Verified for real**, not just written: ran it against the actual public
test repo (`Craterbrain/opensanctuary-release-testing`) with a real
`minisign`-compatible verifier on `PATH`, into a scratch `$HOME` — it
fetched `/releases/latest`, downloaded the real tarball plus
`checksums.txt`/`checksums.txt.minisig`, verified both, extracted into
`~/.local/share/opensanctuary-install`, symlinked
`~/.local/bin/opensanctuary`, and wrote the `.desktop` file. Launching the
installed binary through that symlink (`--headless --no-open`) correctly
served `index.html` — confirms `resolve_web_dir()`'s portable-layout
detection (`docs/paths.md`) works through the symlink, since
`current_exe()` resolves to the real file's directory, not the symlink's.
Also verified fail-closed behavior directly: a verifier forced to reject
the signature aborts before writing anything to disk, and a `PATH` missing
`jq`/`minisign` is caught by the preflight check with a clear message
instead of failing confusingly mid-script. **Not yet verified**: the
`--system` path (needs real root; this dev sandbox has no passwordless
`sudo`) — same code path as the per-user install, just untested end to end.

### Where it's hosted

Both `install.sh` and the release tarball/`.deb` need to be fetchable
without auth — same requirement, same blocker, as `update.md`'s "the repo
is private" issue. `install.sh` itself can live at a stable
`raw.githubusercontent.com` URL once the repo is public, or as its own
pinned GitHub Release asset if a stable unversioned URL matters more (the
former is simpler and is what most projects do; revisit only if that
becomes a real problem).
