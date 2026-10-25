# Setting up and running the release workflow

_Last edited: 2026-10-22_

`.github/workflows/release.yml` builds, signs, and publishes a GitHub Release
(`.deb`, Linux tarball, Windows installer, portable zip, `checksums.txt`,
`checksums.txt.minisig`). This doc is the checklist for getting it working and
for cutting a release. The signing key itself is covered in `docs/keys.md`;
how the app verifies updates is in `docs/update.md`.

`ci.yml` is separate: it runs the tests on every PR into `main` (skipping
them for documentation-only PRs) and on demand. Only `release.yml` signs and
publishes, and it deliberately does **not** run tests.

## Current status

- The workflow is **working and has published real releases**, most recently `v0.3.5` (2026-10-25). Each release carries the `.deb`, Linux tarball, Flatpak, Windows installer and portable zip, plus `checksums.txt` and `checksums.txt.minisig`.
- Project status: no 1.0 yet; the current line is beta 3.
- `ci.yml` runs on every PR into `main` (the default branch). `main` is protected: a PR must be up to date and green to merge.
- `ci.yml`'s Rust and frontend jobs cache compiled dependencies with `Swatinem/rust-cache` (pinned to a commit, like the other third-party actions). `release.yml` deliberately doesn't: release artifacts are signed, so they're always built from scratch.
- The throwaway dry-run tags `v0.2.1-keytest` and `v0.2.2-keytest` are still on the remote and can be deleted (`git push origin :refs/tags/v0.2.1-keytest`, same for `.2.2`); neither has a GitHub Release.
- The signing secrets and variable are set. The production public key is embedded in `updater.rs` and `install.sh`. `RELEASES_REPO` / `REPO` point at `Craterbrain/opensanctuary`, a **public** repo (the updater's anonymous `/releases/latest` poll needs that).
- Known and still open: see "Known issues" at the bottom.

## One-time setup

### 1. Repo secrets and variable

GitHub repo → **Settings** → **Secrets and variables** → **Actions**.

| Where | Name | Value |
|---|---|---|
| **Secrets** tab | `MINISIGN_SECRET_KEY` | Entire contents of `opensanctuary_release.key` |
| **Secrets** tab | `MINISIGN_PASSWORD` | The key's passphrase |
| **Variables** tab | `MINISIGN_PUBLIC_KEY_B64` | `RWRYSKNecgUiwS2YaMPmshtR2/ZAQA/XnunXaOBQw/SRkZPyMspLwhGw` |

The public key must be a **variable**, not a secret: the workflow reads it as
`vars.MINISIGN_PUBLIC_KEY_B64`, so a secret with that name arrives empty and the
pre-publish verify step fails. If you created it as a secret by mistake, delete
the secret copy once the variable exists.

Check from a terminal:

```sh
gh secret list
gh variable list
```

### 2. Make sure the app trusts the same key

These three must hold the same `RW...` string:

- `RELEASE_PUBLIC_KEY_B64` in `src/network/updater.rs`
- `PUBLIC_KEY` in `packaging/install.sh`
- the `MINISIGN_PUBLIC_KEY_B64` repo variable

If the variable differs from the secret key's real public half, the workflow's
verify step fails and nothing is published. If the two files differ from it,
a release publishes fine but installed apps reject it.

### 3. Point the app at the real release repo

`RELEASES_REPO` in `src/network/updater.rs` and `REPO` in `packaging/install.sh`
both point at `Craterbrain/opensanctuary`. The updater only accepts downloads
from `RELEASES_REPO`, so if the workflow ever publishes elsewhere, change both.

The updater polls `/releases/latest` without credentials, so that repo's
releases must be **public**. Never mark a release as a GitHub "prerelease"
(the workflow doesn't): `/releases/latest` skips them and updates stop.

### 4. Actions permissions

**Settings** → **Actions** → **General**: Actions must be allowed to run. The
workflow requests its own `contents: write` for the publish job only, so
"Workflow permissions" can stay at read-only.

## Before every release

1. Bump `version` in `Cargo.toml` (and the `os-next` entry in `Cargo.lock`, which
   `cargo` updates on the next build). The tag must be exactly `v<that version>`;
   the workflow's first step fails otherwise. This matters: the `.deb` filename
   comes from `Cargo.toml`, and the updater refuses a release whose signed
   filename doesn't carry the tag's version.
2. Decide which plugins ship. Release packages bundle the
   `Craterbrain/opensanctuary-plugins` commit named in `packaging/plugins.ref`
   (fetched by `packaging/fetch-plugins.sh`, which `build-deb.sh` and
   `build-windows-installer.sh` run), not `web/plugins/`. To ship newer plugins,
   point it at that repo's new merge commit and update the dev copies in
   `web/plugins/` to match; `ci.yml`'s `plugins` job fails while they differ.
3. Land it through a PR and confirm `ci.yml` is green on the PR. `main`'s
   branch rule only lets a PR merge once it's up to date with `main` and its
   checks pass, so the merge commit's code is exactly what that run tested —
   CI doesn't run again on the merge itself. To check a specific commit
   anyway: **Actions** → **CI** → **Run workflow** on `main`.
4. Run `cargo test` and `cd web && bun run build && bun test` locally if you
   want an earlier signal than CI.

## Cutting a release

```sh
git tag -a v0.3.5 -m "OpenSanctuary 0.3.5"   # the tag is v<Cargo.toml version>, on the merged release commit
git push origin v0.3.5
```

Watch it under the repo's **Actions** tab → **Release**. Jobs run in this order:

1. `check-version`: the tag must equal `v<Cargo.toml version>`. Tests are not run
   here; `ci.yml` runs them on every PR.
2. `build-linux` and `build-windows` in parallel, then `build-flatpak`, which
   repackages `build-linux`'s tarball as a Flatpak bundle (docs/installer.md
   "Flatpak bundle"). Linux builds inside the pinned
   Docker image (`packaging/Dockerfile.linux-build`); Windows cross-compiles with
   `cargo-xwin` and packages with Inno Setup under Wine, so expect both to be
   slow the first time. Both run with `SKIP_TV_APK=1` (see below). Each runs
   `build.rs`, which typechecks and bundles the web UI, so web dependencies must
   be installed first (the packaging scripts do this).
3. `sign-and-publish`: writes `checksums.txt`, signs it, re-verifies the
   signature against the public-key variable, then creates the Release.

To rebuild and republish an existing tag: **Actions** → **Release** →
**Run workflow**, enter the tag. It checks out that tag, not the default branch.

## First run: do a dry run

Before trusting the workflow with a real version, run it end to end on a
throwaway tag. The tag **must actually exist on the remote**: a manual
`workflow_dispatch` with a tag that was never pushed fails at checkout.

1. On a throwaway branch, set `version = "0.2.1-keytest"` in `Cargo.toml` and
   `Cargo.lock`, and push the branch. (The branch must contain the current
   workflow fixes; a tag runs the workflow file from the tagged commit.)
2. `git tag v0.2.1-keytest && git push origin v0.2.1-keytest`. Tag pushes run
   the workflow regardless of branch.
3. In the run, confirm that `sign-and-publish` finishes green, and that the
   Release has the `.deb`, tarball, installer, zip, `checksums.txt`, and
   `checksums.txt.minisig`.
4. Verify by hand (below), then clean up:
   `gh release delete v0.2.1-keytest --cleanup-tag --yes` (if the run failed
   before publishing there is no Release, so just
   `git push origin :refs/tags/v0.2.1-keytest`), and
   `git push origin --delete keytest`.

To iterate after a failure, push a fix to the branch and move the tag
(`git tag -f v0.2.1-keytest && git push -f origin v0.2.1-keytest`). Read failures
with `gh run view <id> --log-failed`.

## What the release builds (and what they leave out)

Release builds run with `SKIP_TV_APK=1`, so **no Android toolchain is installed
and the Android TV client APK is not bundled** in the `.deb`, Linux tarball, or
Windows installer. The app handles this: ADB provisioning reports "APK not
available" (`src/storage/paths.rs::resolve_tv_apk_path`). To ship the APK again,
remove `SKIP_TV_APK=1` from `release.yml` and restore the Java/Android SDK steps
in the Windows job (see git history for the removed steps).

How the switch works, so you can use it locally too:

- `packaging/build-deb-in-container.sh` / `build-windows-installer.sh` skip
  building and staging the APK.
- `Cargo.toml` has a `no-tv-apk` cargo-deb variant (the asset list without the
  APK). **Keep it in sync with the main `assets` list** when adding files.
- `packaging/opensanctuary.iss` marks the `tv-client` entry
  `skipifsourcedoesntexist`.
- `packaging/build-deb.sh` passes `--build-arg INCLUDE_ANDROID_SDK=0` so the
  Docker image doesn't download the Android SDK.

Locally, `SKIP_TV_APK=1 packaging/build-deb.sh` (podman by default,
`CONTAINER_ENGINE=docker` for docker) reproduces the CI Linux build.

## Why tests aren't in the release workflow

`ci.yml` runs tests on every PR; the release only checks the tag. The web suite passes
locally with no failures (the e2e cases also need Chromium installed for Playwright, and the release binary built): the schemas the contract tests read are vendored in
`web/tests/contracts/`, and tests that need something a runner may not have skip themselves
instead of failing:

- Tests that need a real-world document that can't be committed read it from `$OS_TEST_DOCS`
  and skip without it (see "Real-world test documents" below).
- `e2e_video_picker` makes its own clip with `ffmpeg` and skips without it.

### Real-world test documents

Files produced by the real applications (EasyWorship, LibreOffice) catch what synthetic fixtures
can't -- `test1.ews` showed `.ews` import had never read a real file. They live in the project
owner's folder `~/Documents/OpenSanctuary-test-docs`; point `$OS_TEST_DOCS` elsewhere to use another.
Since 2026-10-22 the owner has allowed using it, and anything in it that's safe to publish is
committed under `testdata/` so CI runs those tests too:

| File | Where | Why |
|---|---|---|
| `test1.ews` | **committed**: `testdata/ews/` | A genuine EasyWorship 5 schedule; one public-domain hymn. |
| `shape-test.odp` | **committed**: `testdata/odp/` | Hand-made in LibreOffice's shape gallery; shapes only, no text or author. |
| `LCspring.ewpx` | folder only (`$OS_TEST_DOCS`) | A real church's announcements deck (event and volunteer slides, an image named after a person) -- not published in this public repo. `test_lcspring_ewpx_presentation_bullets_and_transforms`, the LCspring part of `test_ewsx_schedule_roundtrip` and `e2e_unified_schedule_hub` use it and skip without it. |
| `test1.ewsx` | folder only | This app's own export (it carries our `manifest.json`), so it adds nothing a generated export doesn't: `test_ewsx_export_reimports_through_easyworships_own_files_not_just_our_manifest` builds one. Used by `e2e_unified_schedule_hub`. |

Rust tests find folder-only files with `test_doc(name)` (`tests/core_tests/main.rs`). Before
committing a new real-world file, check it for personal content (names, contact details, a church's
own material): the repo is public.

## Verifying a published release

```sh
mkdir check && cd check
gh release download v0.3.5 -R <owner>/<repo>
minisign -Vm checksums.txt -P RWRYSKNecgUiwS2YaMPmshtR2/ZAQA/XnunXaOBQw/SRkZPyMspLwhGw
sha256sum -c checksums.txt --ignore-missing
```

Both must succeed. The first proves the manifest was signed by your key, the
second that each file matches it.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `Tag vX does not match Cargo.toml version Y` | Bump `Cargo.toml` or retag; delete the bad tag with `git push origin :refs/tags/vX` |
| `MINISIGN_SECRET_KEY and/or MINISIGN_PASSWORD ... not set` | Add the secrets (step 1); they must be repo secrets, not environment or org secrets the repo can't see |
| warning `MINISIGN_PUBLIC_KEY_B64 repo variable is not set` / `contains no valid public key` | The verify step fell back to `docs/release-signing-key.txt`. To use the variable, put it on the Variables tab (not Secrets) with the 56-character `RW...` line |
| `MINISIGN_PUBLIC_KEY_B64 (...) differs from docs/release-signing-key.txt (...)` | The variable and the committed key are different keys (usually a key rotation that missed one). Fix the stale one, plus `RELEASE_PUBLIC_KEY_B64` in `src/network/updater.rs` and `PUBLIC_KEY` in `packaging/install.sh` |
| `minisign -V` fails in the verify step | Wrong passphrase, wrong key pasted, or the variable isn't this key's public half; compare against `docs/release-signing-key.txt` |
| 403 / "Resource not accessible by integration" on publish | Org or repo policy is blocking `contents: write`; allow it for Actions or run from a repo that does |
| Checkout step fails fetching `refs/tags/<tag>` | The tag was never pushed (manual dispatch with a nonexistent tag) |
| `build.rs` / `bun run typecheck` errors like "Cannot find module 'qrcode'" | Web dependencies not installed before `cargo build`; run `bun install --frozen-lockfile` in `web/` first |
| `cargo deb` "Can't resolve asset ... tv-client" | Built without `--variant no-tv-apk` while `SKIP_TV_APK=1`, or the variant asset list drifted from the main one |
| Windows job: "Install Inno Setup under Wine" fails, wine tries to open a window | Hosted runners have no display; wine and the Inno installer need one even when "silent". The workflow runs them under `xvfb-run -a` (install `xvfb`); keep that on both the Inno install step and the installer build step |
| Windows job: `cc-rs: failed to find tool "llvm-lib"` | `llvm` apt package missing on the runner (cargo-xwin needs clang, llvm and lld) |
| App says an update "does not carry release version" | `.deb` name doesn't match the tag; the tag/`Cargo.toml` check should prevent this, so check what produced the filename |
| Installed apps never see the release | `RELEASES_REPO` isn't this repo, the repo/release isn't public, or the release is marked prerelease |

## If the key changes

Update all three places in "Make sure the app trusts the same key", plus
`docs/release-signing-key.txt` and the README. Copies of the app that embed the
old key cannot verify releases signed with the new one; see `docs/update.md`'s
"If the key is ever lost or compromised" for the transition.

## Known issues

- **Prerelease replay gap.** The updater binds a release to the signed asset
  filename's `major.minor.patch` only; a prerelease replayed as its own final
  version isn't caught. Fully closing it needs the version inside the signed
  manifest.
- **Windows/macOS auto-update** isn't wired; only the Linux `.deb` can be
  applied by the in-app updater.
