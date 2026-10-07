# Plan: updater (verified releases from GitHub)

_Last edited: 2026-10-01 13:38_

## Blocker to resolve first: the repo is private

`Craterbrain/opensanctuary` is currently a **private** repo. GitHub's
Releases API requires authentication for a private repo — there's no safe
way to embed a credential for that in a binary handed out to end users.
This has to be decided before any of the below is buildable:

- **(a) Make the repo public** before shipping this to real users. Then
  `GET https://api.github.com/repos/Craterbrain/opensanctuary/releases/latest`
  works unauthenticated — the normal, intended use of that endpoint.
  Recommended: simplest, no extra infrastructure, and is what the API is
  for.
- **(b) Keep source private, mirror releases publicly.** A second, public
  repo containing only release artifacts (not source), which the updater
  points at instead. More moving parts, only worth it if source needs to
  stay private for longer than releases should.

Everything below assumes (a). If the decision goes the other way, only the
"where to check" endpoint changes — the verification and apply steps are
the same either way.

**Current status: worked around via (b), not resolved.** The automated
check/download/apply flow described below is built and does run against a
real repo — but that repo is
[`Craterbrain/opensanctuary-release-testing`](https://github.com/Craterbrain/opensanctuary-release-testing),
a disposable public test repo, not the real `Craterbrain/opensanctuary`
(still private). `crate::network::updater::RELEASES_REPO` is the one
constant pointing at it; swapping it for the real repo (once public) or a
dedicated release-mirror repo is the entire migration. `RELEASE_PUBLIC_KEY_B64`
is similarly still the throwaway test key (`.test-signing-key/test.pub`),
not a real production key — see "Release signing key" below, which hasn't
happened yet either.

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

**Actually setting this up? Use `docs/keys.md`** — a step-by-step
checklist (generate, back up, add the GitHub secrets, publish the public
key) that follows the plan below. This section is the design rationale
behind those steps, not the steps themselves.

The private key doesn't exist yet. This is the plan for generating it and,
the harder part, keeping it safe *and* not losing it — those are two
different failure modes (theft vs. "my laptop died") and the plan needs to
cover both without trading one off against the other.

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
not a confusing partial release. **Not yet exercised against a real
GitHub Actions run** — see that workflow file's own header comment.

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
auto-updates** (not needed for the alpha): a second, offline-only
"root" keypair, generated once and never used for day-to-day release
signing, whose only job is countersigning a future key-rotation
announcement for the everyday release-signing key. This mirrors [The
Update Framework](https://theupdateframework.io/)'s root/targets key
separation at a much smaller scale — it means the frequently-used release
key being compromised doesn't fully break trust, since the rarely-touched
root key can still vouch for the rotation. Skip it for now; the two-key
model adds real operational overhead that isn't worth it until there's
something at stake beyond an alpha's own installer.

## Download & apply (Windows) — built, interactively rather than silently

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
elevated privileges itself. It doesn't know how to auto-apply the generic
tarball (no universal "install" action for a bare tarball the way `apt`/
Explorer provide for a `.deb`/`.exe`) — that one's for `install.sh` (a
manual re-run, not wired into the app's own update check) or manual
extraction. **Both Linux release binaries are built inside
a pinned Podman container** (`packaging/Dockerfile.linux-build`, run via
`packaging/build-deb.sh`, which now produces both artifacts from one build),
not directly on whatever machine happens to run the build — see that
Dockerfile's own comment for why its base image (Ubuntu 22.04) is the real
floor once both glibc *and* `libwebkit2gtk-4.1-dev` availability are
accounted for, not just glibc alone.

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
- **Points at a test repo, not the real one** — see "Current status" under
  "Blocker to resolve first" above. The test repo's current release
  (`v0.2.0-test3` at the time of writing -- the tag itself is disposable
  and gets rebuilt/replaced under a fresh tag rather than edited in place,
  since clobbering an existing release's same-named assets hit real GitHub
  CDN staleness during testing) carries all three platform artifacts signed
  under one `checksums.txt`.

## Explicitly out of scope for the alpha

- Fully silent/no-confirmation updates.
- Auto-apply on macOS (Linux now has it; see above).
- Delta/binary-diff updates (full installer download each time is fine at
  this size and update frequency).
