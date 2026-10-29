# Updater (verified releases from GitHub)

_Last edited: 2026-10-29_

The app checks `Craterbrain/opensanctuary` (a public repo, so the anonymous Releases API works) for a newer
release, downloads the installer for its platform, and verifies it before offering it to the operator.
`crate::network::updater::RELEASES_REPO` and `packaging/install.sh`'s `REPO` point at that repo, and
`RELEASE_PUBLIC_KEY_B64` / `PUBLIC_KEY` hold the production signing key (`docs/release-signing-key.txt`,
`docs/keys.md`). See `docs/workflow.md`'s "Current status" for the release history.

## Check flow

1. Trigger: on startup (once, backgrounded — never block boot on a
   network call) **and** a manual "Check for Updates" action in Settings →
   About, next to the existing `appVersion` readonly row. Throttle the
   startup check to at most once per 6–24h regardless of how often the app
   restarts — GitHub's anonymous rate limit is 60 req/hour/IP, and there's
   no reason to check more often than that anyway.
2. `GET /repos/Craterbrain/opensanctuary/releases/latest`, parse
   `tag_name` (e.g. `v0.1.0-alpha`).
   **Depends on releases never being marked GitHub-"prerelease"** — that
   flag excludes a release from this endpoint entirely (confirmed against
   a real public test release: marking it prerelease made this 404, not
   just filter). Keep every real release, alpha included, as a plain
   release; put the actual stability signal in the version string itself
   (`-alpha`, `-beta`) rather than GitHub's separate flag. Simpler than
   switching to the list endpoint, and the one used for testing this
   really did get switched back and re-verified working end to end.
3. Compare against the running build's own version
   (`env!("CARGO_PKG_VERSION")`, already exposed via `/api/status`'s
   `"version"` field, `src/api/routes.rs`) using the `semver` crate rather
   than string comparison — tags carry prerelease suffixes
   (`-alpha`/`-beta`) that need real semver ordering, not lexical.
4. If newer: surface it inline in Settings → About (extend the existing
   `appVersion` row with "Update available: vX.Y.Z" + a Download/Install
   action) — **not** an interrupting modal. This app runs live during
   services; it must never surprise an operator mid-presentation.

## What "verified" means, concretely

Tier 2 (below) has been validated for real, not just planned: a throwaway
test keypair (`.test-signing-key/`, gitignored — see "Release signing key")
signed a real `checksums.txt` for a real `.deb`, both published on a
disposable public test repo
([Craterbrain/opensanctuary-release-testing](https://github.com/Craterbrain/opensanctuary-release-testing)).
Anonymous `curl` (no auth, matching exactly what an end user's updater
does) fetched `/releases/latest`, the `.deb`, `checksums.txt`, and
`checksums.txt.minisig`; the checksum matched, `verify_signature()`
(`src/network/updater.rs`) accepted the real signature, and the
downloaded `.deb` installed cleanly with `apt`. The check-flow note above
(never mark a real release GitHub-prerelease) came directly out of this
test: the release was briefly marked prerelease to mirror the real repo's
own `v0.1.0-alpha`, `/releases/latest` 404'd, and un-marking it (a plain
release, version string still says `-test`/`-alpha`) fixed it — a test
repo has no reason to carry that complication, and neither does a real one.
Since then, tier 2 has also verified real production releases
(`v0.1.0-alpha`, `v0.2.0`) signed with the real key, not just this dry run.

Three tiers, in order of how much infrastructure they need:

1. **Transport security** (already free): `reqwest`'s default TLS
   certificate validation on the `api.github.com`/release-asset download.
   This proves the bytes weren't tampered with in transit, not that
   they're legitimate in the first place.
2. **Signed checksum manifest** (do this for the very next release — see
   "Release signing key" below for the full plan): publish a
   `checksums.txt` (SHA-256 of every release asset) plus a detached
   Ed25519 signature over it, `checksums.txt.minisig`. The updater
   verifies the signature against a public key *embedded in the binary*
   before trusting anything in the file, then verifies the downloaded
   installer's hash against it. This is the tier that actually answers "is
   this really a release I signed," independent of GitHub account
   security — a stolen GitHub token lets someone upload a fake release,
   but they can't produce a valid signature without the private key.
3. **Build provenance** (additional, optional, more setup): GitHub
   Artifact Attestations (Sigstore-backed, `gh attestation verify`) tie a
   release asset to "built by this specific GitHub Actions workflow run in
   this repo" — no key management needed for this one specifically, but it
   only proves *which CI run* produced the bytes, not the same thing as
   tier 2's "the project maintainer personally vouches for this."
   **Prerequisite**: release builds need to happen in a GitHub Actions
   workflow, not ad-hoc on a local machine or the `vms/` sandbox VMs as
   they are now — that's a real scope item (CI needs the same cross-compile
   + `installer.md`'s Inno Setup step, reproduced in Actions, likely
   Windows-hosted runners) — flag it as a dependency, not assume it exists.

Ship tier 2 for the very next release; treat tier 3 as a later addition
layered on top, not a replacement for it. Don't claim more verification in
user-facing copy than what's actually implemented.

## Release signing key

**Already done — see `docs/keys.md`** for the real key's status (generated
2026-10-11, rotated once already after an earlier key's encrypted secret was
lost before anything was signed with it) and `docs/release-signing-key.txt`
for the public half. This section remains as the design rationale this was
built from, not a still-open task — read it for *why* the setup works this
way, not as a "this hasn't happened yet" status.

Keeping the private key safe *and* not losing it are two different failure
modes (theft vs. "my laptop died"), and the plan below covers both without
trading one off against the other.

### Tooling: minisign (Ed25519)

Recommend [minisign](https://jedisct1.github.io/minisign/) over raw
GPG: purpose-built for exactly this (sign a file, verify with an embedded
public key), tiny trusted-computing base, and the private key file is
*natively* passphrase-encrypted (scrypt-derived) — so "the key file leaked"
and "the key is usable" are two separate failures, not one. For the Rust
side, [`minisign-verify`](https://crates.io/crates/minisign-verify) is a
small, zero-dependency, pure-Rust crate (by minisign's own author) that
verifies signatures — no need to shell out to the `minisign` binary or add
a heavier crypto dependency just for this.

### Generating it

```sh
minisign -G -p opensanctuary_release.pub -s opensanctuary_release.key
```

Do this on a machine you already trust day-to-day (the normal dev
machine is fine for a project this size — a dedicated air-gapped machine
is the stronger option if this ever protects something higher-stakes, not
a requirement to start). When prompted for a password:

- **Always set one.** An empty passphrase means the key file alone is the
  entire secret — exactly the single-point-of-failure this plan exists to
  avoid.
- Use a long, random multi-word passphrase (Diceware-style, 5–6 words) —
  long enough to resist offline brute-forcing of minisign's scrypt KDF,
  short enough to actually type or reliably store.
- Store the passphrase in your password manager **as its own entry** —
  deliberately *not* alongside or attached to any copy of the key file
  itself (see below). The passphrase's entire value is that it's not
  co-located with the thing it protects.

### Backing up the (encrypted) key file — the "don't lose it" half

Because the key file is already passphrase-encrypted, backing it up
generously is *safe* — the remaining risk is purely "do I still have a
copy of it," not "did a copy leak." Keep it in at least three places, each
in a different failure domain, so no single event (disk failure, house
fire, forgotten-where-that-USB-drive-is) takes out every copy at once:

1. **Primary** — on the dev machine, wherever keys normally live for you.
2. **Physical, offline copy** — the key file on a USB drive, kept
   somewhere physically separate from the dev machine (different room or
   building). Protects against the dev machine's disk failing or the
   machine itself being lost/stolen/destroyed.
3. **Password manager / encrypted cloud copy** — the key file as a secure
   attachment in the same password manager holding the passphrase (as a
   *separate* entry from the passphrase itself), or an encrypted cloud
   folder. Reasonably safe specifically *because* the file is already
   encrypted — this is defense in depth, not the only layer.
4. **Optional, high-durability extra**: print the key file's contents (it's
   short plain text) and store the paper in a fireproof safe or document
   box. Immune to bit-rot, drive failure, and format obsolescence in a way
   digital-only backups aren't — cheap insurance for something that's
   otherwise unrecoverable if every digital copy is gone.

### Publishing the public key

Unlike the private key, the public key wants to be *as visible and
cross-checkable as possible* — the goal is the opposite of secrecy:

- Embedded in the binary (`src/network/updater.rs`, a `const`) — this is
  what actually verifies updates.
- **Also** published in the repo itself, clearly labeled: a README
  "Verifying releases" section plus a dedicated
  `docs/release-signing-key.txt` containing the exact public key and its
  minisign key ID. This gives an independent way to confirm the key baked
  into any given binary is the real one, rather than the binary being the
  sole source of truth for its own trust anchor.

### Release workflow integration

After building `checksums.txt` (the tier-2 manifest above):

```sh
minisign -S -s opensanctuary_release.key -m checksums.txt
```

produces `checksums.txt.minisig`. Upload both as release assets alongside
the installer. The updater (`src/network/updater.rs`) downloads both,
verifies `checksums.txt.minisig` against the embedded public key
*before* trusting anything in `checksums.txt`, then checks the downloaded
installer's own hash against that now-trusted manifest. Fail closed on any
verification failure — never fall back to "install anyway."

**Automated**: `.github/workflows/release.yml` does exactly the steps
above (build Linux `.deb`+tarball, build the Windows installer+portable
zip, compute `checksums.txt`, sign it) on every `v*.*.*` tag push, using
[`thomasdesr/minisign-action`](https://github.com/thomasdesr/minisign-action)
instead of shelling out to `minisign` directly. Needs two repo *secrets*
(`MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD` — the key file contents and its
passphrase from the "Generating it" section above) and one repo *variable*
(`MINISIGN_PUBLIC_KEY_B64`, not a secret — see "Publishing the public key"
below) set under Settings → Secrets and variables → Actions before it can
publish anything; it fails closed with a clear error if either is missing,
not a confusing partial release. Every release so far has gone through
this workflow; see `docs/workflow.md`'s "Current status" and "Known issues".

### If the key is ever lost or compromised

Either case has the same resolution, because a hardcoded public key can't
silently trust a replacement without breaking the whole point of pinning
it:

1. Generate a new keypair (same procedure above).
2. Ship one release the old-fashioned way — the updater will (correctly)
   refuse to auto-verify it, so this one requires a manual download —
   whose sole purpose is switching to the new public key, announced
   prominently (release notes, README, ideally somewhere outside GitHub
   too) so the change is independently visible rather than a silent binary
   swap.
3. If compromise (not just loss) is suspected, also treat every release
   signed since the suspected compromise date as untrusted, and consider
   pulling those GitHub releases.

This is inherently a little painful by design — it's the cost of the key
not being recoverable by anyone else, which is the same property that
makes it trustworthy in the first place. It's also exactly what the backup
plan above is for: the failure mode to actually avoid is needing this
procedure at all because every copy of the key was lost.

**Optional stronger option, worth revisiting once real users depend on
auto-updates** (not needed pre-1.0): a second, offline-only
"root" keypair, generated once and never used for day-to-day release
signing, whose only job is countersigning a future key-rotation
announcement for the everyday release-signing key. This mirrors [The
Update Framework](https://theupdateframework.io/)'s root/targets key
separation at a much smaller scale — it means the frequently-used release
key being compromised doesn't fully break trust, since the rarely-touched
root key can still vouch for the rotation. Skip it for now; the two-key
model adds real operational overhead that isn't worth it until there's
something at stake beyond a pre-1.0 beta's own installer.

## Download & apply (Windows) — built, interactively rather than silently

_Works from the first release after 0.3.5. Earlier builds refused it: the installer's file name,
`opensanctuary-setup-x64.exe`, has no version in it, and the updater only accepted assets whose signed
name carried the release version, so a Windows install on 0.2.0–0.3.5 must download the installer by hand
once._ The version is now bound another way: the signed `checksums.txt` must also list a file named for the
release's `major.minor.patch` (the portable zip, the `.deb`), so an older signed manifest replayed under a
newer tag is rejected (`manifest_binds_version` in `src/network/updater.rs`).

`POST /api/updates/download-and-install` covers Windows too now, same as
Linux: downloads the matching `opensanctuary-setup-x64.exe` asset, verifies
it against `checksums.txt`/`checksums.txt.minisig` (refusing on any
failure), then hands it to the OS's default handler for a `.exe`, which
just runs it. That launches Inno Setup's own installer UI
interactively — **not** the fully-silent `/VERYSILENT /SUPPRESSMSGBOXES
/NORESTART` flow originally sketched below. Deliberate, not a shortcut: it
matches this app's existing "never a silent swap while a service could be
running, always an explicit choice" posture, already true of the manual
`apply-local` path this reuses the last step of. Inno Setup's
`CloseApplications`/`AppMutex` directives (`installer.md`) still handle
closing the running app and relaunching after once the operator clicks
through; the install dir gets overwritten in place, the data dir is
untouched by construction (`paths.md`).

The originally-sketched fully-silent flow, kept here for reference in case
that tradeoff is revisited later:

1. Download the matching asset, plus `checksums.txt` and
   `checksums.txt.minisig`, to a temp path.
2. Verify both the same way as above.
3. Prompt the user — "Update ready: vX.Y.Z. Restart now to install?" — an
   explicit choice, never a silent swap while a service could be running.
4. On confirmation, launch the downloaded installer with
   `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` instead of just opening it.

## Linux — built; Windows — built; macOS — still no installer

Linux has both a `.deb` (`cargo-deb`) and a generic `opensanctuary-linux-x64-<version>.tar.gz`
tarball for non-apt distros (Fedora, Arch, etc. — the portable-folder
layout `resolve_web_dir()`/`paths::resolve()` already know how to find, no
installer needed; `packaging/install.sh`, the friendly wrapper script
`installer.md` describes for fetching and placing that tarball, is built
and verified -- see `installer.md`'s "`install.sh` — everyone else"
section). `POST /api/updates/download-and-install` implements the
automated apply step for the `.deb` specifically: downloads it, verifies it
the same way the manual path does, and hands it to the desktop's own
`.deb`-install handler (`xdg-open`/`apt`) — never runs anything with
elevated privileges itself. A copy installed with `install.sh` (recognised by the `VERSION` file and `web/` folder beside the
program) is updated from the same signed release instead: the check picks the `.tar.gz`, and the download is verified the
same way, unpacked next to the install, and each top-level entry (the program, `web/`, ...) replaces its namesake in place (`apply_tarball`). Files
the tarball doesn't carry are left alone, and the running program is unlinked rather than overwritten, so it keeps running until the
"restart to finish updating" prompt. A per-user install needs no root; an install the user can't write to (`/opt/opensanctuary`) gets a message with the
`install.sh --system` command instead. Verified on the Mint VM 2026-10-27 (0.3.9 build -> signed 0.3.11 tarball -> restart). **Both Linux release binaries are built inside
a pinned Podman container** (`packaging/Dockerfile.linux-build`, run via
`packaging/build-deb.sh`, which now produces both artifacts from one build),
not directly on whatever machine happens to run the build — see that
Dockerfile's own comment for why its base image (Ubuntu 22.04) is the real
floor once both glibc *and* `libwebkit2gtk-4.1-dev` availability are
accounted for, not just glibc alone.

### Restart to finish updating (Linux)

Installing the `.deb` replaces `/usr/bin/opensanctuary` on disk, but the copy already running keeps the old program in memory. (The Windows
installer closes and reopens the app by itself; the Linux package manager does not.) So after an update the operator is still on the old version
until a restart, and nothing used to say so.

* **Detection** (`src/network/relaunch.rs`): the kernel marks a running program's own path with ` (deleted)` once the file under it has been replaced,
  so `readlink /proc/self/exe` ending in that marker, with the file present again, means "an update was installed". Not used inside a Flatpak (its
  updates replace the whole sandbox, not this path) or off Linux.
* **A pop-up** (updates are always started by the operator, so asking is not a surprise): a background check every 30 s posts one sticky warning to the
  Notices bell ("Restart to finish updating"), and the console turns that notice into a dialog, "The update is installed. Restart OpenSanctuary now?"
  (*Restart now* / *Cancel*). Choose Cancel if a service is running: the same question is under Settings > About. A console opened after the update
  finished asks once when it loads. Asked once per page load.
* **Settings > About > Restart to finish updating** appears only while a restart is needed (`restart_needed` in `GET /api/updates/status`), and the
  version row says so. *Restart now…* asks first (the console and the output windows go away for a few seconds), then calls
  `POST /api/system/restart`.
* **The restart** (`post_restart`): refused with 409 unless the program really was replaced, so it can't bounce a healthy app. It starts the installed program
  again, detached, with the same arguments (`sh -c 'sleep 2; exec "$0" "$@"'`, nothing spliced into shell text), answers, then ends the native window loop
  (`DisplayEvent::Quit`) or exits when there is none. The two-second wait lets this copy release its ports first. A browser console polls until the new copy
  answers and reloads itself.

Tested: unit tests for the detection (`network::relaunch`), HTTP tests for the status flag and the refusal (`tests/core_tests/restart.rs`), the Settings
row and the pop-up trigger (`web/tests/restart_after_update.test.ts`), and by hand on the Mint VM (install over a running copy, notice, Restart now).

No installer exists yet for macOS. Update-*checking* still works the same
way there (it's just an HTTP call, and `check_latest_release` already
resolves a `.dmg`-suffixed asset name for display purposes), but *apply* is
out of scope until there's a packaging story (a signed `.app`/notarization)
— `post_update_download_and_install` explicitly refuses on macOS with a
clear error rather than pretending to auto-apply something that isn't
built. (No macOS UI path actually surfaces this yet — the frontend's "Check
for Updates"/"Download & Install" flow was built and tested on Linux;
wiring a macOS-appropriate fallback message is unstarted.)

## Where implemented

- `src/network/updater.rs` — the manual path (`check_local_file_against_sibling_checksums`,
  `open_with_system_handler`) plus the automated one
  (`check_latest_release`, `download_and_verify_release`), same shape as
  `src/network/tls.rs`. Depends on `minisign-verify` (signature
  verification) and `semver` (version comparison); embeds the release
  public key as a `const` (`RELEASE_PUBLIC_KEY_B64`).
- Routes: `POST /api/updates/apply-local` (manual), `GET /api/updates/status`,
  `POST /api/updates/check`, `POST /api/updates/download-and-install`
  (automated -- Linux and Windows; macOS refuses cleanly, see above). All
  require the host token.
- Background: `src/main.rs` runs one check on startup and every 12h after,
  caching the result in `AppState::update_status`.
- Frontend: extends the existing `about` settings category
  (`web/src/core/settings_schema.ts`, `web/src/ui/settings_dialog.ts`) —
  "Check for Updates" (shows the result, offers to download-and-install if
  newer) sits alongside the pre-existing "Install Update from File" — no
  new top-level category needed.
- **Points at the real repo** — `RELEASES_REPO`/`REPO` are
  `Craterbrain/opensanctuary` (public), not the old test mirror; see
  "Resolved: the repo is public" above and `docs/workflow.md`'s "Current
  status" for the real release history this now checks against.

## Explicitly out of scope pre-1.0

- Fully silent/no-confirmation updates.
- Auto-apply on macOS (Linux now has it; see above).
- Delta/binary-diff updates (full installer download each time is fine at
  this size and update frequency).
