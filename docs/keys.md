# Setting up the release signing key

_Last edited: 2026-10-01 13:38_

This is the one key OpenSanctuary needs right now: the Ed25519 keypair
(via `minisign`) that signs every release's `checksums.txt`, which
`src/network/updater.rs` (the app's own auto-updater) and
`packaging/install.sh` (the generic-Linux installer) both verify before
ever installing anything. The design rationale and threat model are in
`docs/update.md`'s "Release signing key" section — this doc is just the
checklist for actually doing it, once, for real.

Follow these in order. Steps 1–4 you do on your own machine, outside any
CI system. Nothing in steps 1–4 ever touches GitHub.

## 0. Install minisign

- Debian/Ubuntu: `sudo apt install minisign`
- Arch/CachyOS: `sudo pacman -S minisign`
- Fedora: `sudo dnf install minisign`
- macOS: `brew install minisign`
- Windows: [prebuilt binaries here](https://jedisct1.github.io/minisign/)

## 1. Generate the keypair

Pick a directory **outside this repo checkout** — your home directory, or
wherever you normally keep credentials. (If you generate it inside the
repo anyway: `.release-signing-key/` is already gitignored as a safety
net — see `.gitignore` — but outside the repo is the real safeguard, not
the gitignore entry.)

```sh
mkdir -p ~/opensanctuary-signing-key && cd ~/opensanctuary-signing-key
minisign -G -p opensanctuary_release.pub -s opensanctuary_release.key
```

It'll ask you to set a password. **Always set one** — an empty password
means the key file alone is the entire secret, which defeats the point of
all of this. Use a long, random, multi-word passphrase (5–6 random words —
a password manager's "generate passphrase" feature is fine). You'll need
this passphrase again in step 5.

This produces two files:

- `opensanctuary_release.pub` — two lines: an `untrusted comment` with the
  key ID, then the public key itself. **Not secret.**
- `opensanctuary_release.key` — the encrypted private key. **This is the
  one thing in this whole project that cannot be regenerated or
  recovered if lost.** Everything below is about not losing it.

## 2. Save the passphrase in your password manager

As its **own, separate entry** — not as a note attached to a copy of the
key file, not in the same password-manager item. The reason this matters:
if somewhere down the line one copy of the key file leaks (a misconfigured
backup, a compromised machine), the passphrase being stored somewhere
completely separate is what keeps that leak from being immediately
game-over.

Name the entry something you'll recognize in five years, e.g.
"OpenSanctuary release signing key passphrase."

## 3. Back up the encrypted key file itself

The key file is already passphrase-encrypted, so copying it around freely
is safe — the only remaining risk is "do I still have a copy," not "did a
copy leak." Put `opensanctuary_release.key` in **at least three places**,
each a genuinely different failure domain, so no single bad day (a dead
hard drive, a stolen laptop, a house fire) takes out every copy at once:

1. **Your normal dev machine** — wherever you generated it (step 1).
2. **A USB drive kept somewhere physically separate** — a different room
   or building than the dev machine. This is what survives "the dev
   machine's disk died" or "the dev machine was stolen."
3. **Your password manager, as a file attachment** — a *separate entry*
   from the passphrase entry in step 2 (or an encrypted cloud folder you
   already trust). Safe specifically because the file is already
   encrypted on its own.
4. **Optional, cheap extra durability**: print `opensanctuary_release.key`
   (it's short plain text) and put the paper in a fireproof safe or
   document box. Immune to drive failure and file-format rot in a way
   nothing digital-only is.

Do **not** put `opensanctuary_release.pub` through this same ceremony —
that one's supposed to be public, see step 6.

## 4. Double-check nothing got committed

```sh
cd ~/OS-Next  # wherever your checkout is
git status
```

Should show nothing related to the key. If you generated the key inside
the repo and `git status` shows it anyway, it means you generated it
somewhere `.gitignore` doesn't cover — move both files outside the repo
entirely before going further, and if either file was ever `git add`ed,
treat that key as compromised and start over at step 1.

## 5. Add the GitHub repo secrets (for `.github/workflows/release.yml`)

On GitHub: your repo → **Settings** → **Secrets and variables** →
**Actions** → **Secrets** tab → **New repository secret**, twice:

| Secret name | Value |
|---|---|
| `MINISIGN_SECRET_KEY` | The **entire contents** of `opensanctuary_release.key` — open it in a text editor and paste everything, both lines. |
| `MINISIGN_PASSWORD` | The passphrase from step 1 (the one saved in your password manager in step 2). |

Then the **Variables** tab (same Settings page, next to Secrets) →
**New repository variable** — this one is *not* a secret, it's meant to be
public:

| Variable name | Value |
|---|---|
| `MINISIGN_PUBLIC_KEY_B64` | The second line of `opensanctuary_release.pub` only (the `RW...` string — not the `untrusted comment:` line above it). |

The release workflow fails closed with a clear error if any of these
three are missing — it won't silently publish an unsigned release.

## 6. Publish the public key, and point the app at it

The public key wants the opposite treatment from the private key — as
visible and independently cross-checkable as possible, so the key baked
into a binary isn't the sole source of truth for its own trust:

1. **Embed it in the app** _(done)_: `RELEASE_PUBLIC_KEY_B64` in
   `src/network/updater.rs` and `PUBLIC_KEY` in `packaging/install.sh` hold
   the same `RW...` string as step 5's variable. This is what every
   installed copy of OpenSanctuary actually checks updates against.
2. **Swap the repo too**: same file, `RELEASES_REPO` currently points at
   `Craterbrain/opensanctuary-release-testing` (the disposable test repo).
   Change it to the real repo once it's public.
3. **Publish it independently** _(done: `docs/release-signing-key.txt`, README "Releases")_: create `docs/release-signing-key.txt`
   containing the full contents of `opensanctuary_release.pub` (both
   lines) plus a one-line note of when/how it was generated. Add a short
   "Verifying releases" mention in `README.md` pointing at it. The point
   of this step is that someone can confirm the key in a binary they
   downloaded matches the key the project itself published, independent
   of trusting that binary.

## 7. Dry-run before trusting it for a real release

Before tagging a real version:

1. Push a throwaway tag (e.g. `v0.0.0-keytest`) or use **Actions** →
   **Release** → **Run workflow** (`workflow_dispatch`) against an
   existing tag.
2. Watch the `sign-and-publish` job specifically — confirm it reports a
   successful signature, not one of the two fail-closed checks it has for
   missing secrets, and that the resulting GitHub Release actually has
   `checksums.txt.minisig` attached.
3. Delete the throwaway tag/release once confirmed.

`.github/workflows/release.yml`'s own header comment has the full list of
what each job does if anything here doesn't match what you see.

## If the key is ever lost or compromised

See `docs/update.md`'s "If the key is ever lost or compromised" section —
short version: generate a new keypair (back to step 1), ship exactly one
release the old-fashioned way (manual download) whose only purpose is
switching every installed copy to the new public key, and announce the
change prominently outside GitHub too. This is why steps 2–3 above exist —
the goal is to never actually need this.
