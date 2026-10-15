# Plugin signing: design (not implemented)

_Status: **phase 1 is implemented** (section 10); phases 2 to 5 are design only. Section 0 describes what the code does now. Written 2026-10-14, updated with phase 1._

## 0. Today (after phase 1)

Phase 1 closed gap 1 below: a plugin can list every file's hash in `files`, so one signature covers the whole package, and the review screen says exactly what a signature covers (*signed, covers every file* / *manifest signed, other files aren't covered* / *unsigned*). What follows is the situation phase 1 started from, kept because gaps 2 and 3 still stand.

What exists, and what it does and doesn't prove:

- A plugin folder may hold `plugin.json.minisig`, a detached [minisign](https://jedisct1.github.io/minisign/) signature over the **bytes of `plugin.json`**, and `plugin.json` may name a `publisher_key` (a minisign public key). Discovery verifies the signature (`verify_signer` in `src/core/plugin_registry.rs`, using the same `verify_signature` as the app updater). A signature that is present but invalid, or present without a `publisher_key`, makes the plugin fail to load.
- The verified key is recorded as the plugin's `observed_signer`, pinned as `approved_signer` when the operator approves, and a different key (or a signed plugin that becomes unsigned) shows "signer changed" on the review screen.
- Separately, every plugin is **digest-pinned**: a hash over every file in its folder is recorded at approval, and any difference stops the plugin until it is reviewed again. This is independent of signing.
- The tools a plugin may run (`process.run`) have their own, stronger scheme: the tool's release is verified by a signature and a checksum against a key shipped in the plugin folder.

**Gap 1: the signature covers the manifest, not the plugin.** `plugin.json` does not list the other files' hashes, so a validly signed manifest next to a swapped `app.js` still verifies. The review screen says "signed (publisher key …)", which today means "the manifest was signed", not "this code is what the publisher released". Digest pinning still catches changes *after* approval, but it can't tell the operator that the code they are approving at first install is the publisher's. (An older line in `docs/PLUGIN_PERMISSIONS.md` claimed the manifest carries the artifact hashes; that was true of native libraries and is no longer true. It has been corrected.)

**Gap 2: a key means nothing without something to say whose it is.** Trust on first use pins a key the first time it is approved; the app has no notion of a publisher it already knows, a key that was compromised, or a key that was rotated.

**Gap 3: nothing stops "signed" being mistaken for "safe".** A signature says *who*, never *whether it is harmless*. Permissions, not signatures, are what limit damage, and the design below keeps it that way.

## 1. Goals and non-goals

Goals
1. A signature covers the **whole package**, so "signed" means what it sounds like.
2. The operator can tell **built-in**, **a publisher they have chosen to trust**, **a publisher seen before**, and **unsigned** apart at a glance, and each has a different consequence.
3. A compromised or retired key can be **revoked** without shipping a new app version for every plugin.
4. **Rollback is detectable**: an old, validly signed, vulnerable version can't be quietly put back.
5. It works **offline** and with **no service the project must run** (a catalog is optional, later).

Non-goals
- Signing never grants permissions, never skips the review screen, and never lets a plugin run before approval.
- No certificate authority, no accounts, no required registry. Minisign keys only (already a dependency; the updater uses them).
- Unsigned plugins stay allowed by default (people write their own for one church).

## 2. Trust levels

| Level | Meaning | How it is established |
|---|---|---|
| **Built-in** | Ships with the app | The folder is in the install's `web/plugins/` **and** its package signature verifies against the app's release key (the key the updater already trusts). |
| **Trusted publisher** | A publisher the operator chose to trust | The signing key is in the operator's trust list (section 4). |
| **Known publisher** | Signed; key pinned on first approval of some plugin, never explicitly trusted | Today's TOFU behavior. |
| **Unsigned** | No signature | Allowed unless policy forbids it (section 6). |
| **Invalid** | A signature is present and fails (bad, wrong key, files don't match) | Refused to load, with the reason. Never downgraded to "unsigned". |

Trust level is **display and policy only**. It never widens what a plugin may do.

## 3. Package signature (closing gap 1)

Add a `files` map to `plugin.json`: every file in the package except `plugin.json` and the signature, as `"<relative path>": "<sha256 hex>"`. The signature over `plugin.json` then covers the whole package, because the manifest names every file's hash.

Verification, in this order, and any failure is "invalid":
1. The signature verifies over the exact bytes of `plugin.json` against `publisher_key`.
2. Every file in the folder has a `files` entry whose hash matches, and every `files` entry exists. **No extra files** (an unlisted file next to a signed manifest is how a signature gets laundered).
3. `files` paths pass the same containment rules as today (relative, no `..`, no links).

A plugin whose manifest has no `files` and no signature is simply unsigned, as now. A plugin with a signature but no `files` is treated as **manifest-only signed** and labelled exactly that (so the display stops overstating it), until the publisher re-releases it with `files`.

Tooling (implemented): `os-next --hash-plugin <folder>` writes `files` into `plugin.json`; the author then sets `publisher_key` and signs with the standard `minisign -Sm plugin.json`; `os-next --check-plugin <folder>` reports *unsigned*, *manifest only* or *fully signed* and exits non-zero if the package doesn't verify. The binary deliberately has no signing command: that would mean shipping minisign's signing code and handling authors' secret keys, and `minisign` already exists for it.

Compatibility: `manifest_version` stays 1. `PluginManifest` does not reject unknown fields (only a few nested structs do, such as toolbar items), so `files` is an optional field that older hosts simply ignore.

## 4. Publisher trust store

New table `plugin_publishers(key TEXT PRIMARY KEY, name TEXT, trusted INTEGER, revoked INTEGER, added_at INTEGER, added_by TEXT, note TEXT)`.

- **Trust publisher**: on the review screen of a signed plugin, a checkbox "Trust everything this publisher signs" (off by default; it is a convenience, not a requirement to approve). Console-only, audited, like every grant decision.
- **What trusting does**: an update signed by a trusted key whose permissions are *the same or narrower* can be approved with one click; it never skips the screen when it asks for **more**. It does not auto-approve.
- **Revoking a publisher** (operator action, or a key on the shipped revocation list, section 5): every plugin from that key moves to a `distrusted` state: stopped, shown with the reason, re-approvable only after the operator confirms. The same machinery as `changed`.

## 5. Revocation and rotation

- **Revocation list**: a small signed file, `revoked_keys.json`, `{ "revoked": [{ "key": "...", "reason": "...", "since": "..." }], "sequence": 12 }`, signed by the app release key, shipped inside app releases and also fetchable by the existing update check. Applied at startup and whenever it updates. A higher `sequence` always replaces a lower one; an older file is ignored (so it can't be used to un-revoke).
- **Rotation**: a publisher moves to a new key with a *rotation statement* (`old key signs: "new key X replaces me from <date>"`), shipped in the plugin's folder. The host accepts a key change from A to B without a scary re-prompt only if A's signature over B verifies and A isn't revoked. Without it, a key change is today's "signer changed" warning.
- **Compromised release key**: the app's own key rotates through the same updater mechanism it already has; out of scope here.

## 6. Policy

A setting in Settings → Plugins, `plugin_signature_policy`, console-only:

| Value | Effect |
|---|---|
| `allow` (default) | Unsigned and signed plugins are reviewed the same way. |
| `warn` | Unsigned plugins show a prominent warning on the review screen and must be approved with a confirmation. |
| `require` | Only built-in and signed plugins load. For churches that install from one trusted source only. |

Independent of policy: a plugin asking for a **Critical** permission (`process.run`, `keyring.service`, `ui.dom`) that is *unsigned* gets the confirmation step, and an **invalid** signature is always refused. The policy never loosens anything for built-ins.

## 7. Rollback and freshness

- Record the highest `version` ever approved per `(plugin id, publisher key)`. A lower signed version is shown as a **downgrade** (the review screen already flags this from the version numbers; this makes it hold across revoke/re-approve).
- With a catalog (section 8) each entry carries `min_version`; the host refuses to install anything below it. Without one, downgrades are flagged, not blocked: the operator may have good reason.

## 8. Catalog (later, optional)

A catalog is a signed index of packages: `{ "plugins": [{ "id", "version", "min_version", "package_url", "package_sha256", "publisher_key", "summary" }], "sequence": N, "timestamp": "..." }`, signed by a catalog key. It gives discovery and updates, not trust: the package signature still has to verify, a catalog entry can't grant permissions, and a stale or replayed catalog (lower `sequence`, old `timestamp`) is ignored. Installing from a catalog is the existing install path (`POST /api/plugins/install`) with the package fetched by the host, so every install check already applies. Nothing in sections 3 to 7 depends on a catalog existing.

## 9. Console UI

- Plugin card: a trust badge (Built-in / Trusted publisher / Known publisher / Unsigned / Invalid), and for signed ones the publisher name and short key id.
- Honest wording: *Fully signed* vs *Manifest signed (files not covered)* vs *Unsigned*.
- Review screen of a signed plugin: "Trust this publisher" checkbox; "This update is signed by the same publisher you trusted" line next to the existing "what changed" block.
- Settings → Plugins → **Publishers**: the trust list with remove/revoke, and the revocation list's sequence and date.
- Audit entries: `publisher_trusted`, `publisher_revoked`, `signature_invalid`, `key_rotated`.

## 10. Implementation phases

1. **Honest labels and `files` verification** (section 3) — **done**: `files` in the manifest; discovery and install refuse a package whose files differ, are missing, or aren't listed (signed or not); `signature_scope` is recorded (`unsigned` / `manifest` / `full`) and shown; a plugin approved as fully signed that later isn't gets a warning; `--hash-plugin` and `--check-plugin` for authors. Built-in plugins are not signed yet (that belongs with the release key in phase 2).
2. **Trust store and display** (section 4, 9): table, badges, trust checkbox, Publishers list.
3. **Revocation list and rotation** (section 5).
4. **Policy setting** (section 6) and downgrade memory (section 7).
5. **Catalog** (section 8), only if there is demand and a place to host it.

Each phase ships on its own and leaves the previous behavior intact.

## 11. Tests to write with it

- Signature good, bad, wrong key, signed-then-edited, extra unlisted file, missing listed file, `files` hash mismatch, path escape in `files`.
- Manifest-only signed vs fully signed vs unsigned labels.
- Trusted key + same permissions = one click; trusted key + wider permissions = full review; revoked key = distrusted and stopped.
- Revocation list: higher sequence replaces, lower is ignored, bad signature is ignored.
- Rotation: valid statement accepted silently, forged statement rejected, rotation from a revoked key rejected.
- Policy modes, including that `require` never blocks a built-in.
- Downgrade across revoke and re-approve.
- Hostile: a signed package whose signature is valid but whose `publisher_key` field was swapped to the attacker's own key (the signature then verifies, so the *key identity* is what the operator is shown and what is pinned; confirm the card makes this visible).

## 12. Open questions

1. ~~Does the manifest parser tolerate unknown fields?~~ Yes at the top level; `files` needs no version bump.
2. Should a *built-in* plugin be required to verify against the release key, or is "it's in the install folder" enough (an attacker who can write the install folder already owns the machine)? Leaning: verify anyway, for the display consistency.
3. Where would a revocation list be fetched from, given the app has no service of its own? Likely the same release host as the updater.
4. How does an author get a first key and publish it so people recognize it? Probably a "publisher card" in the plugin's README, plus the key id shown in the app; there is no way to prevent impersonation of a *name*, only of a *key*.
