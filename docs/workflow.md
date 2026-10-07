# Setting up and running the release workflow

_Last edited: 2026-10-06_

`.github/workflows/release.yml` builds, signs, and publishes a GitHub Release
(`.deb`, Linux tarball, Windows installer, portable zip, `checksums.txt`,
`checksums.txt.minisig`). This doc is the checklist for getting it working and
for cutting a release. The signing key itself is covered in `docs/keys.md`;
how the app verifies updates is in `docs/update.md`.

`ci.yml` is separate: it runs the same tests on every push and PR. Only
`release.yml` signs and publishes.

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

1. Bump `version` in `Cargo.toml`. The tag must be exactly `v<that version>`;
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

1. `test`: tag/version check, `cargo test`, web typecheck/build/test.
2. `build-linux` and `build-windows` in parallel (the Windows job installs the
   Android SDK, Wine, and Inno Setup, so expect it to be slow, especially the
   first time).
3. `sign-and-publish`: writes `checksums.txt`, signs it, re-verifies the
   signature against the public-key variable, then creates the Release.

To rebuild and republish an existing tag: **Actions** → **Release** →
**Run workflow**, enter the tag. It checks out that tag, not the default branch.

## First run: do a dry run

This workflow has not run on GitHub yet. Before trusting it with a real version:

1. On a throwaway branch, set `version = "0.2.1-keytest"` in `Cargo.toml` and
   push the branch.
2. `git tag v0.2.1-keytest && git push origin v0.2.1-keytest`. Tag pushes run
   the workflow regardless of branch.
3. In the run, confirm that `sign-and-publish` finishes green, and that the
   Release has the `.deb`, tarball, installer, zip, `checksums.txt`, and
   `checksums.txt.minisig`.
4. Verify by hand (below), then delete the Release and the tag:
   `gh release delete v0.2.1-keytest --cleanup-tag --yes`.

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
| App says an update "does not carry release version" | `.deb` name doesn't match the tag; the tag/`Cargo.toml` check should prevent this, so check what produced the filename |
| Installed apps never see the release | `RELEASES_REPO` isn't this repo, the repo/release isn't public, or the release is marked prerelease |

## If the key changes

Update all three places in "Make sure the app trusts the same key", plus
`docs/release-signing-key.txt` and the README. Copies of the app that embed the
old key cannot verify releases signed with the new one; see `docs/update.md`'s
"If the key is ever lost or compromised" for the transition.
