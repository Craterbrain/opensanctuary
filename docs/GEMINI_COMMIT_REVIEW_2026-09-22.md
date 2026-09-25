# Review: commits implemented by Gemini, 2026-09-20 → 2026-09-22

## Scope and method

Commit `87c6e3a1` ("fix(dedup): consolidate 6 functions duplicated across
2-3 files") is the last commit reviewed and authored by a Claude session (it
carries a `Claude-Session:` trailer). Every commit after it, through HEAD
(`bdf6bb58`), was implemented by a different AI tool ("Gemini") and had not
been reviewed by anyone before this pass. That range is 10 commits, roughly
21,000 lines, across networking/security, credential storage, mobile remote
control, the editor UI, and storage/test code:

- `326dbfb` feat(editor): slide editor ribbon UI, section tagging, auto-advance timers, formatting tools
- `635cb11` refactor(frontend): modularize UI components, unified clients & presentation sequencing
- `697c9fd` feat(keyring,editor): native keyring with plugin isolation boundary and bulk paste upgrades
- `c224ac8` feat: add mobile remote control webpage with QR barcode connection and robust stage sync
- `b32228b` feat(network): network settings tab, DHCP Option 12 client, conflict detection & cross-platform virtual adapters
- `a085f97` test: add automated visual comparison suite for Linux and Windows release binaries
- `7c8f447` refactor(frontend): dead code elimination, unread imports & CSS cleanup
- `7d5e4bf` test(fix): isolate mock DOM querySelectorAll between test suites
- `a7af6d4` feat(network): dual-plane HTTP/HTTPS listeners, client pairing protocol, and configurable security policies
- `bdf6bb5` feat(storage): expand scripture version parsing and clean Christian keywords array

(Two commits inside this date range, `c19a29c` "feat(theme): rebuild theme
system..." and the merge `ac91717`, were done by a Claude session, not
Gemini, and were excluded from this review.)

Reviewed by splitting across 5 focused passes by risk area, each building
and running the relevant test suite (`cargo build`/`cargo test`,
`bun run build`/`bun test`) and reading full diffs plus current file state,
not just the commit stat. The standard applied matches the previous
Gemini-review commit (`840aa0b`): concrete, verified findings with file/line
and a real trigger scenario, not style nitpicks.

## Verdict

Builds and the full test suite pass throughout (`cargo test`: 81/81 lib+
integration; `bun test`: 330/331, the one failure is a pre-existing-pattern
flake introduced by this same commit range, see below). Functionally, most
of this is solid — the editor/frontend work in particular held up well.
But three findings are critical, verified security gaps in code that is
reachable from any device on the church's LAN, and should block relying on
this for a real service until fixed.

---

## Critical findings

### 1. The HTTPS private key is exposed to any unauthenticated caller

`src/network/tls.rs` stores the server's raw PEM private key under a plain
settings key (`tlsKeyPem`) via `db.set_setting(...)`. `Database::get_settings()`
(`src/storage/db.rs:1784`) returns the *entire* settings table with no key
filtering, and the pre-existing `GET /api/settings` route
(`src/api/routes.rs:1437-1440`, unauthenticated) serializes that whole map
to JSON for any caller.

Verified live: started the server with `--https-port`, then
`curl http://<host>/api/settings` returned the full
`-----BEGIN PRIVATE KEY-----…` PEM. Anyone on the LAN can fetch the
server's real TLS private key with one GET and perfectly impersonate the
"secure" HTTPS listener — this makes the HTTPS feature actively worse than
not having it, since it creates a false sense of security while the key is
one request away.

**Fix direction:** never round-trip the private key through the generic
settings map/route. Store it in a dedicated table or file the generic
settings API doesn't touch, or filter `get_settings` to an explicit
allow-list of client-safe keys.

### 2. The keyring's host-bypass token is handed out over the zero-auth discovery endpoint

`GET /api/server-info` is *intentionally* unauthenticated by design (see
`docs/CLIENT_PAIRING.md` — it's the "what and where is this console" identity
check a client reads before doing anything else). This commit
(`697c9fd`) adds `host_token` to that same response
(`src/api/routes.rs:956-967`). That token grants `KeyringCaller::Host`,
which — unlike plugin-scoped tokens — is **not** subject to any ownership
check in `keyring_get`/`keyring_set`/`keyring_delete`: it can read, delete,
or overwrite every secret in the keyring, including every plugin's
credentials and every provider API key.

Because the server binds `0.0.0.0` and LAN origins are explicitly trusted
(`is_allowed_sanctuary_origin`), any device on the same network can GET one
endpoint, read `host_token`, and dump the entire keyring with one more
request.

Separately and just as bad on its own: `POST /api/keyring/plugin/token`
(`src/api/routes.rs:1504-1524`) mints a valid token for *any* `plugin_name`
supplied in the request body, with no check that the caller actually is
that plugin. Any HTTP client can request a token claiming to be a specific
plugin and read that plugin's stored secret. The client-side "isolation"
(`web/src/core/plugins.ts`) is a JS property-masking convention, not a real
boundary — plugins run in the same realm as the host and can read
`window.OS.keyring` directly, bypassing the context object they were
handed.

**Fix direction:** `host_token` must not be reachable from an unauthenticated
discovery route — split discovery identity from any credential. Plugin
tokens need to be bound to something the server can actually verify (e.g.
issued once at plugin registration/load time and never re-mintable on
request), and `Host`-level keyring access needs the same ownership
discipline as plugin-scoped access, not a blanket bypass.

### 3. Client-pairing management endpoints have no server-side authentication — **this is a real implementation gap, not intended behavior**

> **Correction to the original review pass:** the reviewer characterized
> this as "deliberate-as-tested," citing that the commit's own test
> asserts the open read succeeds. That's a misreading of intent. Per
> `docs/CLIENT_PAIRING.md`'s own "Critical Invariant" section, the
> **zero-auth design applies specifically to output-display surfaces** —
> `live.html`, `stage.html` (foldback), and the mobile remote — so a
> volunteer can open a browser tab or plug in a TV without typing a
> password. It was never meant to apply to the **pairing management API
> itself** (`/api/pairing/session`, `/status`, `/devices`, `/devices/:id`).
> Per the user: *"I want live.html and foldback to be available
> unauthenticated. But I want the apps to authenticate with the server."*
> The pairing lifecycle exists precisely to let a dedicated TV/Roku app
> authenticate itself to the server — Gemini's implementation built the
> QR-bridge mechanics correctly but left every endpoint in that mechanism
> unauthenticated, which defeats the point of building it. The commit's
> test asserting open access just reflects what got built, not what was
> asked for.

Concretely, today:
- `POST /api/pairing/session` — any network client can mint a session
  token directly, bypassing "operator clicks Pair TV App in the console."
- `GET /api/pairing/status?device_id=X` returns that device's permanent
  bearer token to anyone who supplies (or guesses) its ID.
- `GET /api/pairing/devices` dumps every paired device's permanent token
  to any unauthenticated caller.
- `DELETE /api/pairing/devices/:id` lets anyone revoke any paired display —
  a trivial denial-of-service against a live sanctuary display mid-service.

Additionally, "dual-plane HTTP/HTTPS" doesn't actually separate the
sensitive pairing plane from the plaintext plane: `src/main.rs` mounts the
*same* router, with the same absence of auth, on both listeners, so pairing
tokens can transit in cleartext regardless of which URL a client uses.

**Fix direction:** the pairing *session/authorize* handshake (steps
happening on the console + technician's phone, which don't yet have a
device identity to check) can reasonably stay open, matching the QR-bridge
design — but once a device holds a `dev_tok_...`, every subsequent call
that device or an admin makes (`status` polling, listing, revoking) should
require presenting a valid credential, not just a guessable ID in the query
string. `GET /api/pairing/devices` and `DELETE /api/pairing/devices/:id` in
particular look like they should require an authenticated operator/console
session, not be open to any LAN caller.

### Follow-up (resolved): mobile remote is now paired, not zero-auth

`docs/CLIENT_PAIRING.md`'s "Critical Invariant" section originally listed
the **mobile remote (`/remote`)** alongside `live.html`/`stage.html` as
intentionally zero-auth, and the `c224ac8` commit's auth stub
(`remote_auth_stub`, accepted a PIN field and never checked it;
`RemoteAuthStub.isAuthenticated()` hardcoded `true` client-side) matched
that. The user decided `/remote` should require pairing too, same as the
TV/Roku flow, so this has been implemented:

- `remote_auth_stub` is gone; `POST /api/pairing/remote-session` mints a
  real device token via the same `paired_devices` store TV/Roku pairing
  uses (single-step — one QR, no second device to bridge to).
- The console's "Mobile Remote" QR (Pairing menu) now embeds that token in
  the URL fragment; `remote.html` shows a "Not Paired" gate and never
  connects or renders show data until `POST /api/pairing/verify` confirms
  the token.
- Every command the remote sends (HTTP `/api/command` and the shared `/ws`)
  carries the token and is checked server-side against `paired_devices`
  before executing; commands with no token attached (the operator console)
  are unaffected.
- Revoking a remote session works exactly like revoking a TV pairing —
  Settings > Paired Devices.
- See `docs/CLIENT_PAIRING.md` for the updated invariant and full flow, and
  `web/tests/e2e_remote_mobile.test.ts` for a live end-to-end test proving
  the gate blocks both no-token and invalid-token access.

One known limitation, disclosed rather than silently left implicit: this
closes the *documented* entry point (the `/remote` page and the QR that
reaches it), but `/api/command` and `/ws` still accept commands with no
token at all — that's the pre-existing "console has no auth" gap noted in
finding #3 above, and it's a separate, larger decision (would mean adding
some form of login/session to the operator console itself) that wasn't
part of this fix.

---

## Secondary findings (real, lower severity)

**`b32228b` — network settings / DHCP Option 12 / virtual adapters**
- Conflict detection is advisory-only and fails open: `check_hostname_conflict`/
  `check_port_conflict` are only ever called from the read-only "check"
  endpoints. None of the actual write paths (`post_network_interfaces_handler`,
  `toggle_dedicated_mac_handler`, the startup DHCP broadcast) call them before
  applying a setting — the server will happily save/broadcast a hostname it
  already reported as colliding.
- The new privileged endpoints (adapter toggle, DHCP broadcast) have no
  auth, consistent with the rest of the API but newly exposing OS-elevation
  prompts (pkexec/UAC) and raw broadcast packet sends to any LAN client.
- The Windows adapter branch is `#[cfg(target_os = "windows")]`-gated and
  entirely unverified in this (Linux) environment — the commit's claim of
  "117 tests passing on Windows" could not be reproduced or confirmed here.
- What's solid: command-injection defenses are good — hostnames and
  interface names are validated (`is_valid_interface_name`,
  `sanitize_rfc1035_hostname`) before being spliced into shell/PowerShell
  scripts.

**`a085f97` — Linux/Windows visual comparison test suite**
- Doesn't actually compare anything: it captures paired Linux/Windows
  screenshots at 14 points but only asserts two unrelated pixel checks — a
  real rendering divergence between platforms would pass silently. The
  commit message's "automated visual comparison" claim isn't backed by the
  code.
- Hardcoded artifact path (`/home/jasonb/.gemini/antigravity/brain/...`) is
  a session directory belonging to another AI tool, not part of this repo —
  will fail on any other machine.
- Undocumented hard preconditions (pre-built Linux + cross-compiled Windows
  release binaries, Wine installed) fail with a raw, unhandled `ENOENT`
  stack trace rather than any actionable message when run cold.
- Does genuinely run and pass end-to-end when its environment is present
  (verified: 5/5 passing here) — not dead scaffolding, just missing the
  actual comparison logic and fragile setup.

**`326dbfb` / `635cb11` / `7c8f447` / `7d5e4bf` — editor ribbon UI & frontend refactors**
- One real timer-teardown gap: the yt-dlp download-progress poller in the
  Video Picker (`web/src/ui/video_picker.ts`, a 400ms `setInterval`) has no
  reset hook tied to the modal closing, unlike 7 other modals that
  correctly register one via `dialogManager.registerResetHook`. Closing the
  picker mid-download leaves it polling every 400ms until the download
  finishes or 5 consecutive failures occur. Bounded, not an infinite leak,
  but a real instance of the pattern.
- The `e2e_compare_windows_linux.test.ts` failure currently seen in
  `bun test` (1/331) is a TOCTOU race in a fix this same commit range
  introduced (`isClosed()` checked, then an `await` that can still race the
  page closing) — test-only impact, no production code involved.
- Auto-advance timers, section tagging, dead-code removal, and the
  websocket reconnect/backoff logic were all reviewed in depth and found
  solid — no double-fire handlers, no orphaned imports, correct
  clear-before-set timer discipline throughout.
- The test-isolation fix (`7d5e4bf`) genuinely closes one real cross-file
  mock leak between two specific test files, but the underlying root cause
  (no `afterAll` teardown of the shared `globalThis.document` mock) remains
  — it currently causes no visible failures only because both files'
  mock implementations happen to be equivalent, which is a coincidence, not
  a structural fix.

**`bdf6bb5` — scripture version parsing / keyword cleanup**
- Solid; no real bugs. No colliding aliases introduced, parsing already
  fails gracefully on malformed input (no `unwrap()` on untrusted data), no
  dangling references to any removed keyword string anywhere in the
  codebase.

---

## What's solid across the board

- Every commit in this range builds cleanly and the existing test suites
  stay green (no compile regressions anywhere).
- Command-injection defenses in the new network/adapter code are correctly
  implemented.
- The keyring's underlying OS-level wrapper (`src/storage/keyring.rs`) is a
  clean, real integration with Secret Service/Credential Manager — no
  plaintext-file fallback, no secret values ever logged. The problem is
  entirely in the *authentication around it*, not the storage primitive
  itself.
- XSS handling in the new mobile remote page is genuinely clean — all
  show-derived strings are escaped before rendering.
- The bulk-paste editor upgrades, scripture parsing, section tagging, and
  the bulk of the frontend modularization work are correct and
  well-isolated; reviewers found essentially nothing wrong beyond the two
  bounded issues noted above.

---

## Suggested fix order

1. Stop leaking the TLS private key via `/api/settings` (#1) — one-line
   blast radius, trivial and highest severity.
2. Stop leaking `host_token` via `/api/server-info`, and close the
   plugin-token-minting hole (#2).
3. Require an authenticated caller for pairing device status/list/revoke,
   matching the documented "apps authenticate with the server" intent (#3).
4. ~~Resolve the open question on `/remote`'s auth stub~~ — done, see
   "Follow-up (resolved)" above. `/api/command` and `/ws` still accept
   commands with no token at all, though (the pre-existing console-auth
   gap) — worth its own decision later.
5. ~~Wire conflict detection into the network write paths~~ — done, see
   "#5 fixed" below.

---

## Status: 2026-09-23

**All five items are now fixed.**

### #5 fixed: network conflict detection now enforced, not just advisory

`POST /api/settings` and `POST /api/network/dedicated-mac/toggle` used to
save/apply `networkHostname`/`networkPort` unconditionally, even though the
read-only `GET /api/network/check-hostname`/`check-port` endpoints already
computed whether the value collided with something else on the network —
the UI's warning badge had nothing enforcing it server-side.

- `post_settings` now holds back `networkHostname`/`networkPort` when
  flagged, reporting it in a new `_conflicts` field on the response instead
  of saving silently — the rest of the same request (e.g. bundled CSP/CORS
  settings) still saves normally. The frontend can resubmit with
  `_confirmOverrides` (comma-separated key names) once the operator
  explicitly confirms a warning they were already shown.
- `toggle_dedicated_mac_handler` runs the same hostname check before ever
  touching a real interface; a conflict returns `{success: false, conflict:
  true, ...}` without invoking `pkexec`/UAC. A `confirm_conflict: true` flag
  lets the operator proceed anyway after an explicit confirm dialog.
- The startup DHCP Option 12 broadcast (`src/main.rs`) can't fail-closed the
  same way (no operator to redirect at boot) but now at least logs a
  warning if the hostname it's about to broadcast conflicts with something.

**Verified live** (not just in tests): found a real device already on this
LAN via `avahi-browse` (an Epson printer answering as `EPSON5490D4.local` /
192.168.1.118) and hit all three paths with that exact hostname —
`/api/network/check-hostname`, `/api/network/dedicated-mac/toggle`, and
`/api/settings` all correctly reported the conflict, and the adapter-toggle
call did not invoke `pkexec`. Added
`test_conflicting_network_settings_are_held_back_unless_confirmed`
(deterministic, using a real bound TCP port rather than a network-dependent
hostname, since the hostname-conflict branch has no portable way to
reproduce automatically — same limitation the codebase's own pre-existing
`check_hostname_conflict` unit tests already had).

**A real regression this fix introduced, caught before it shipped**: the
first version of the startup DHCP-broadcast check ran
`check_hostname_conflict` — which does blocking `std::net` DNS/mDNS
resolution — directly inside a `tokio::spawn`'d async task with no
`spawn_blocking`. That stalled whatever Tokio worker thread it landed on
for as long as the resolution took, which was long enough to make
`e2e_remote_mobile.test.ts`'s blackout-toggle assertion consistently miss
its 5-second deadline (reproduced 3/3 times in isolation — not
resource-contention flakiness). Fixed by wrapping the check in
`tokio::task::spawn_blocking`; the same test suite run went from ~295s to
~103s afterward, consistent with the stall actually being the cause.

Full suite after all five fixes: 123/123 Rust tests (40 lib + 83
integration), 333/333 bun tests.

### What changed, and the architecture decision behind it

All three turned out to share one root cause: the app had no concept of
"this request is genuinely from the console" at all — every credential
(`host_token`, plugin tokens, pairing session tokens) was mintable or
readable by any LAN caller because there was nothing to check a caller
*against*. Patching each leak individually would've left the same hole open
for the next thing built on top of it, so — per the go-ahead to change the
network/security architecture since nothing is heavily built on it yet —
this introduces one real concept instead: **host-level trust means "you
have actual access to this machine," established without ever putting the
credential on the network**, not a token any LAN client can eventually
obtain.

- **The native desktop webview** gets `host_session_token` injected
  directly into its own JS context at window creation
  (`window.__OS_HOST_TOKEN__`, via wry's `with_initialization_script` —
  `src/webview/mod.rs`). It is never sent over HTTP or WebSocket.
- **A browser tab on the same machine** (127.0.0.1/localhost — the same
  address the console's own startup banner already tells operators to use)
  can fetch it from the new `GET /api/internal/host-token`, which only
  answers a request whose own TCP connection is loopback
  (`ConnectInfo<SocketAddr>`, checked server-side — not spoofable by a
  network client).
- A LAN client, even a paired remote-control device, is never loopback and
  never gets this token, structurally, not by convention.
- `web/src/core/host_session.ts` is the one place the frontend resolves it;
  `keyring.ts` and `api_client.ts`'s `pairing.*` methods use it.

**#1 — TLS private key leak (`/api/settings`).** The private key no longer
touches the app's own database at all — moved to the OS keyring
(`KeyringService`, same crate already used for plugin/provider secrets),
service `OpenSanctuary:System`, so a bug in a generic "dump all settings"
route can't leak it again structurally. The certificate (public by nature)
stays in `settings`. A one-time migration moves an existing deployment's
legacy plaintext key into the keyring on next read instead of silently
generating (and invalidating every already-pinned client's trust in) a
fresh cert. `src/network/tls.rs`.

**#2 — keyring host-token leak + forgeable plugin tokens.**
`GET /api/server-info` no longer includes `host_token` (it's still
deliberately zero-auth for everything else it returns — discovery identity,
not a credential). `POST /api/keyring/plugin/token` now checks the asserted
`plugin_name` against files actually present under `<web_dir>/plugins/`
before minting anything, closing the "mint a token for a plugin that was
never installed" exploit. Disclosed, not silently claimed fixed: this does
NOT stop a malicious script from claiming to be a *real* installed plugin —
plugins run in the same JS realm as the host with no Worker/iframe
boundary, so there's no way to verify which script actually made the
request without real realm isolation, which is a larger change than this
pass covers.

**#3 — pairing management endpoints had no authenticated caller.**
`POST /api/pairing/session`, `GET /api/pairing/devices`,
`DELETE /api/pairing/devices/:id`, and `POST /api/pairing/remote-session`
now require the host token. `POST /api/pairing/authorize` (called by the
technician's *phone*, never the console) and `GET /api/pairing/status`
(polled by an unpaired device that has no token yet) stay open by
necessity — they're gated in practice by needing a session token only a
host-authenticated console could have minted. One related behavior change:
pairing sessions are no longer single-use-then-discarded — they stay valid
for their existing 5-minute TTL across multiple device authorizations, so a
technician can pair every TV in the building from one console-minted
session instead of the phone needing to mint its own follow-up sessions
(which would've required giving the phone console-level trust it was never
meant to have). `pairing.html`'s "Pair Another Display" button now just
reuses the existing session instead of calling the (now console-only)
session-minting endpoint itself.

**Side effect worth noting**: fixing this uncovered that `app_ui.ts` never
actually imported `api` from `api_client.ts` — `api.pairing.createSession()`
in the console's own "Pair TV App" QR flow has been silently throwing and
getting swallowed by a `catch` since it was written, meaning that flow's
session token was never actually set. Now imported and working.

**Verified live** against the built release binary (not just tests): TLS
key absent from `/api/settings`, `host_token` absent from
`/api/server-info`, `/api/internal/host-token` succeeds from loopback,
`/api/pairing/session` and `/api/pairing/devices` 401 without the host
token and 200 with it, `/api/keyring/plugin/token` 404s for a made-up
plugin name and 200s for the real installed one. Full suite: 122/122 Rust
tests (cargo test), 333/333 bun tests.
