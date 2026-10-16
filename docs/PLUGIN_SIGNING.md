# Plugin signing

_Status: **phases 1 to 4 are implemented** (section 10); phase 5 (the catalog) is design only, deferred until there's demand. Section 0 describes what the code does now. Written 2026-10-14; updated 2026-10-15 to fold in phases 2 and 3, then again the same day to fold in phase 4._

## 0. Today (after phases 1-4)

What exists, and what it does and doesn't prove:

- A plugin folder may hold `plugin.json.minisig`, a detached [minisign](https://jedisct1.github.io/minisign/) signature over the **bytes of `plugin.json`**, and `plugin.json` may name a `publisher_key` (a minisign public key). Discovery verifies the signature (`verify_signer` in `src/core/plugin_registry.rs`, using the same `verify_signature` as the app updater). A signature that is present but invalid, or present without a `publisher_key`, makes the plugin fail to load.
- A `files` map in `plugin.json` lists every other file's SHA-256, so one signature over the manifest covers the whole package (*fully signed*); without it, a signature covers only the manifest (*manifest signed*) and the review screen says exactly that, instead of overstating what it proved.
- The verified key is recorded as the plugin's `observed_signer`, pinned as `approved_signer` when the operator approves, and a different key (or a signed plugin that becomes unsigned) shows "signer changed" on the review screen — unless a valid rotation statement (section 5) explains the change.
- A publisher trust store (`plugin_publishers` table, section 4) tracks every key the operator has seen, trusted or revoked; the console shows a trust-level badge (Built-in / Trusted / Known / New / Unsigned / Invalid / Revoked) rather than a bare "signed" fact.
- A signed, release-key-distributed revocation list (`revoked_keys.json`, section 5) stops every plugin signed by a revoked key and forces re-confirmation before approving another from it.
- A console-wide `pluginSignaturePolicy` setting (`allow` / `warn` / `require`, section 6) controls how much an unsigned plugin is allowed to get away with, and a remembered per-publisher version floor (section 7) flags a downgrade even after a revoke/re-approve or an uninstall/reinstall.
- Separately, every plugin is **digest-pinned**: a hash over every file in its folder is recorded at approval, and any difference stops the plugin until it is reviewed again. This is independent of signing.
- The tools a plugin may run (`process.run`) have their own, stronger scheme: the tool's release is verified by a signature and a checksum against a key shipped in the plugin folder.

**Gap 1 (closed, phase 1): the signature covered the manifest, not the plugin.** `plugin.json` didn't list the other files' hashes, so a validly signed manifest next to a swapped `app.js` still verified. Closed by the `files` map above. (An older line in `docs/PLUGIN_PERMISSIONS.md` claimed the manifest carries the artifact hashes; that was true of native libraries and is no longer true. It has been corrected.)

**Gap 2 (closed, phases 2-3): a key meant nothing without something to say whose it is.** Closed by the trust store (section 4) and the release-key-signed revocation list plus rotation statements (section 5): the app now has a notion of a publisher it already knows, a key that was compromised (revoked), and a key that was rotated.

**Gap 3 (by design, not a bug to fix): nothing stops "signed" being mistaken for "safe".** A signature says *who*, never *whether it is harmless*. Permissions, not signatures, are what limit damage, and the design keeps it that way — see "Non-goals" below.

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
| **Known publisher** | Signed; the operator has approved a plugin from this key before, but never explicitly trusted it | Today's TOFU behavior. |
| **New publisher** | Signed by a key the app has never seen approved | Added in phase 2: the first sight of a key, which deserves more care than a known one. |
| **Revoked publisher** | Signed by a key the operator revoked | Phase 2. |
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

## 6. Policy -- implemented

A setting in Settings → Plugins, `pluginSignaturePolicy` (`src/core/plugin_policy.rs`), console-only, read through the same generic `settings` table every other console-only setting uses:

| Value | Effect |
|---|---|
| `allow` (default) | Unsigned and signed plugins are reviewed the same way. |
| `warn` | Unsigned plugins show a prominent warning on the review screen and must be approved with a confirmation. |
| `require` | Only built-in and signed plugins load. For churches that install from one trusted source only. An unsigned, non-built-in plugin can't even be approved while this is set (`approval_gate` returns `Refused`, not `NeedsConfirmation` -- no confirmation unlocks it), and an already-approved one that was approved while a looser policy was in effect stops loading (shown as the `policy_blocked` state) without losing its approval -- loosen the policy and it resumes without being re-approved.

Independent of policy: a plugin asking for a **Critical** permission (`process.run`, `keyring.service`, `ui.dom`) that is *unsigned* gets the confirmation step, and an **invalid** signature is always refused. The policy never loosens anything for built-ins (`plugin_policy::permits_load`/`approval_gate` both short-circuit to "allowed" for a built-in or a signed plugin before even looking at the policy value).

## 7. Rollback and freshness -- implemented

- The highest `version` ever approved per `(plugin id, publisher key)` is recorded in a `plugin_version_floor` table (`""` for unsigned), raised on every approval (`plugin_registry::remember_approved_version`) but never lowered. A lower signed version is shown as a **downgrade** (`is_downgrade_from_floor` in the plugin's API view) whether or not it's even been approved yet, so this holds across revoke/re-approve -- and, deliberately, across an uninstall/reinstall too: the floor is never deleted when a plugin is removed, which is exactly the loophole an attacker would otherwise use to erase the memory and quietly reintroduce an old, vulnerable version.
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
2. **Trust store and display** (section 4, 9) — **done**: `plugin_publishers` table; trust levels Built-in / Trusted / Known / New / Unsigned / Invalid / Revoked, shown as a badge with the publisher's name; "Trust everything this publisher signs" at approval (and a Trust publisher button later); a Publishers list with Stop trusting and Revoke; revoking stops every plugin that key signed and makes approving another need a confirmation; all of it console-only and audited (`publisher_trusted`, `publisher_untrusted`, `publisher_revoked`). Trust is display and policy only: it never skips the review screen or approves anything (approving an update was already one click). **Deviation from section 2:** *Built-in* means "the folder ships with the app", decided by location, not by a signature verifying against the release key. Signing the bundled plugins with the release key needs the release pipeline's secret and comes with phase 3's revocation list, which uses the same key.
3. **Release key, revocation list and rotation** (section 5) — **done**, tested with throwaway keys (`testdata/signing/`, started with the hidden `--test-release-key`):
   - *Release key.* A plugin signed by the release key is the app's own: trusted wherever it lives, its publisher name is vouched for as "OpenSanctuary" (so no one else can pose as it), and it can't be revoked by a list or by hand. The bundled plugins aren't signed in this repository; the release process signs them with `cargo run --release --example sign_release -- sign-plugin <release key> <folder>` (hashes the files, sets `publisher_key`, writes `plugin.json.minisig`). **Still to wire:** that step in the release workflow, which needs the real secret and is the one thing not exercised here.
   - *Revocation list.* `revoked_keys.json` + `.minisig`, signed by the release key, read from the app's `web/` folder and from the data directory. Only a list whose signature verifies counts; the highest `sequence` wins and a lower one is never applied after a higher one (an old list can't un-revoke); each key is revoked and every plugin it signed stops (audited as `revoked` by `revocation list`); entries naming the release key or junk are skipped. It is applied at startup and whenever the plugin list is read. Status at `GET /api/plugins/revocations`, shown under Publishers; a list that couldn't be used is shown too. Fetching a newer list over the network is not built (open question 3); dropping the file in the data directory works now.
   - *Rotation.* A publisher moves to a new key by shipping `plugin.rotation.json` (`{"old_key", "new_key"}`) with `plugin.rotation.json.minisig` signed by the OLD key. If it verifies, names the plugin's signing key as `new_key`, and the old key isn't revoked, there is no "different publisher" warning, the old key's standing (trusted or known) carries over, and approving moves your trust to the new key (audited as `key_rotated`). A statement signed by the new key, naming another key, or from a revoked old key is ignored and the plugin shows the ordinary warning.
4. **Policy setting** (section 6) and downgrade memory (section 7) — **done**: `pluginSignaturePolicy` (`allow`/`warn`/`require`) in `src/core/plugin_policy.rs`, enforced everywhere a plugin is actually loaded or served (`plugin_registry::is_usable`, used by the WS command path, the keyring token route, the sandboxed frame and web-plugin-script serving, token minting and the sandboxed command/tool runner) as well as at approval time (`approval_gate`, wired into `POST /api/plugins/:id/approve`'s new `confirm_unsigned` field). The version floor is a small `plugin_version_floor` table, updated on every approval and exposed per plugin as `version_floor`/`is_downgrade_from_floor`. Settings → Plugins has the policy selector at the top of the page; each plugin card shows the floor-based downgrade warning independent of whether it's ever been approved. **Caught in review:** `is_builtin` reads a folder that can't even canonicalize (e.g. a stale `plugin_grants` row whose folder vanished outside the normal uninstall route) as built-in -- a deliberately safe reading for the unrelated `removable`-badge check it's shared with, but a dangerous one once the same answer also decides whether `require` applies. `is_usable` now also requires the folder to still exist, independent of what it would otherwise have been classified as.
5. **Catalog** (section 8), only if there is demand and a place to host it.

Each phase ships on its own and leaves the previous behavior intact.

## 11. Tests to write with it

- Signature good, bad, wrong key, signed-then-edited, extra unlisted file, missing listed file, `files` hash mismatch, path escape in `files`.
- Manifest-only signed vs fully signed vs unsigned labels.
- Trusted key + same permissions = one click; trusted key + wider permissions = full review; revoked key = distrusted and stopped.
- Revocation list: higher sequence replaces, lower is ignored, bad signature is ignored.
- Rotation: valid statement accepted silently, forged statement rejected, rotation from a revoked key rejected.
- ~~Policy modes, including that `require` never blocks a built-in.~~ `core::plugin_policy::tests` (unit) + `test_plugin_signature_policy_and_version_floor` (integration, `tests/core_tests.rs`).
- ~~Downgrade across revoke and re-approve.~~ `plugin_registry::tests::version_floor_only_ever_rises_and_is_scoped_per_signer`/`downgrade_from_floor_is_only_reported_for_two_real_comparable_versions` (unit) + the same integration test above.
- Hostile: a signed package whose signature is valid but whose `publisher_key` field was swapped to the attacker's own key (the signature then verifies, so the *key identity* is what the operator is shown and what is pinned; confirm the card makes this visible).

## 12. Open questions

1. ~~Does the manifest parser tolerate unknown fields?~~ Yes at the top level; `files` needs no version bump.
2. Should a *built-in* plugin be required to verify against the release key, or is "it's in the install folder" enough (an attacker who can write the install folder already owns the machine)? Leaning: verify anyway, for the display consistency.
3. Where would a revocation list be fetched from, given the app has no service of its own? Likely the same release host as the updater.
4. How does an author get a first key and publish it so people recognize it? Probably a "publisher card" in the plugin's README, plus the key id shown in the app; there is no way to prevent impersonation of a *name*, only of a *key*.
