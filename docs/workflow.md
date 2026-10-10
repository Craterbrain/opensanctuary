# Setting up and running the release workflow

_Last edited: 2026-10-06_

`.github/workflows/release.yml` builds, signs, and publishes a GitHub Release
(`.deb`, Linux tarball, Windows installer, portable zip, `checksums.txt`,
`checksums.txt.minisig`). This doc is the checklist for getting it working and
for cutting a release. The signing key itself is covered in `docs/keys.md`;
how the app verifies updates is in `docs/update.md`.

`ci.yml` is separate: it runs the tests on every push and PR. Only
`release.yml` signs and publishes, and it deliberately does **not** run tests.

## Current status (2026-10-10)

- The workflow has **not yet published a release**. The last dry run (tag
  `v0.2.2-keytest`, run 8) passed the version check and both builds: Linux, and
  Windows after the Wine/Inno-under-xvfb and `llvm` fixes. `sign-and-publish`
  then failed at "Sign checksums.txt" with `Error while loading the secret key
  file`: the `MINISIGN_SECRET_KEY` secret must hold the key file **verbatim, both
  lines** (`untrusted comment: ...` and the base64 line). Reset it with
  `gh secret set MINISIGN_SECRET_KEY < opensanctuary_release.key`, then rerun the
  failed job (`gh run rerun <id> --failed`). The verify and publish steps have not
  run yet. The `MINISIGN_PUBLIC_KEY_B64` variable, by contrast, is the bare
  `RW...` string.
- The workflow fixes are on `main`. The throwaway dry-run tags/versions
  (`v0.2.1-keytest`, `v0.2.2-keytest`) can be deleted; a dry run needs a new tag
  and a matching throwaway `Cargo.toml` version on a branch (see "First run").
- The signing secrets/variable are set. The production public key is embedded
  in `updater.rs` and `install.sh`. `RELEASES_REPO`/`REPO` point at
  `Craterbrain/opensanctuary`.
- Known and still open: see "Known issues" at the bottom.

## One-time setup

### 1. Repo secrets and variable

GitHub repo → **Settings** → **Secrets and variables** → **Actions**.

| Where | Name | Value |
|---|---|---|
| **Secrets** tab | `MINISIGN_SECRET_KEY` | Entire contents of `opensanctuary_release.key` |
| **Secrets** tab | `MINISIGN_PASSWORD` | The key's passphrase |
| **Variables** tab | `MINISIGN_PUBLIC_KEY_B64` | `RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42` |

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
2. Commit and push; confirm `ci.yml` is green on that commit.
3. Run `cargo test` and `cd web && bun run build && bun test` locally if you
   want an earlier signal than CI.

## Cutting a release

```sh
git tag v0.3.0
git push origin v0.3.0
```

Watch it under the repo's **Actions** tab → **Release**. Jobs run in this order:

1. `check-version`: the tag must equal `v<Cargo.toml version>`. Tests are not run
   here; `ci.yml` runs them on every push and PR.
2. `build-linux` and `build-windows` in parallel. Linux builds inside the pinned
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

`ci.yml` runs tests on every push and PR; the release only checks the tag. Two
things currently make CI's web tests unreliable, so fix them before relying on a
green `ci.yml` as a release gate:

- `web/tests/contracts_validation.test.ts` and `web/tests/pairing.test.ts` import
  schemas from `../../../spec-db-rs/contracts/`, a directory outside this repo.
  They cannot pass on a runner. Either vendor the schemas into the repo or skip
  those tests when the directory is missing.
- Many "E2E Live Test" cases showed `(fail)` on the runner; this wasn't
  diagnosed (they likely need a running server or browser setup that CI doesn't
  provide).

Also fixed on `keytest`: `build.rs` runs `bun run build` on every `cargo build`,
so `bun install --frozen-lockfile` must run in `web/` first (added to both
workflows), and the TLS keyring test now skips itself when no OS keyring exists.

## Verifying a published release

```sh
mkdir check && cd check
gh release download v0.3.0 -R <owner>/<repo>
minisign -Vm checksums.txt -P RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42
sha256sum -c checksums.txt --ignore-missing
```

Both must succeed. The first proves the manifest was signed by your key, the
second that each file matches it.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `Tag vX does not match Cargo.toml version Y` | Bump `Cargo.toml` or retag; delete the bad tag with `git push origin :refs/tags/vX` |
| `MINISIGN_SECRET_KEY and/or MINISIGN_PASSWORD ... not set` | Add the secrets (step 1); they must be repo secrets, not environment or org secrets the repo can't see |
| `MINISIGN_PUBLIC_KEY_B64 repo variable is not set` | It's in Secrets, not Variables; add it on the Variables tab |
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

- **TLS key without an OS keyring.** The server stores its TLS private key in the
  OS keyring (`src/network/tls.rs`). On a machine with no Secret Service (headless
  Linux, CI), the key can't be read back and the certificate is regenerated on
  every start, which breaks clients that pinned the old fingerprint (Android TV).
  Not fixed.
- **`ci.yml` web tests** reference files outside the repo (above).
- **Prerelease replay gap.** The updater binds a release to the signed asset
  filename's `major.minor.patch` only; a prerelease replayed as its own final
  version isn't caught. Fully closing it needs the version inside the signed
  manifest.
- **Windows/macOS auto-update** isn't wired; only the Linux `.deb` can be
  applied by the in-app updater.
