# Client discovery & console pairing

_Last edited: 2026-10-29_

This covers two related things for **permanent display clients** — the
Roku and Android TV apps, as opposed to someone opening `live.html` in a
random browser tab: how they *find* an OS-Next console on the LAN, and how
they *stay* pointed at the right one once found. Both halves are
implemented for **Android TV** (`apps/android-tv`): discovery via mDNS/manual
IP, and the two-way pairing bridge below via `PairingManager.kt` -- either
the QR flow this doc describes. Installing over ADB (see "ADB-driven setup"
below) makes the app the TV's home app and pins the console's certificate,
but never pairs: the phone's scan is the only way in. **Roku is still just a design** -- no Roku app exists yet; read this
before building it.

---

## What's already implemented (discovery)

- The server broadcasts itself via mDNS as **`open-sanctuary.local`** (see
  `src/discovery.rs`), under the custom service type
  `_opensanctuary._tcp.local.` rather than plain `_http._tcp` — so a client
  can browse specifically for OS-Next consoles instead of every HTTP server
  on the network. TXT records carry `id` (the console's stable instance ID),
  `name` (the configured church name), and `version`.
- mDNS only resolves the `.local` TLD (RFC 6762) — **`.lan` is not a real
  mDNS domain**, nothing on the network will answer for it. If a `.lan`
  name is wanted for other reasons (e.g. a router's local DNS), that's a
  separate, router-level DNS entry, unrelated to and not a substitute for
  this mDNS advertisement.
- Every console has a stable **instance ID** (`server_instance_id` in the
  `settings` table, a UUID generated once on first boot and persisted
  forever — `get_or_create_instance_id()` in `src/discovery.rs`). This is
  the value a client should pin to, not the IP address or hostname, since
  IPs get reassigned by DHCP and two consoles could theoretically both
  answer to `open-sanctuary.local` on different LANs (or briefly on the same
  one, if a demo unit and the real console are both running).
- `GET /api/server-info` returns `{ instance_id, name, version,
  protocol_version }` — the same identity as the mDNS TXT record, fetchable
  over plain HTTP once a client has an IP (from mDNS or manual entry) but
  before it does anything else.

None of this requires a client to authenticate or register — it's pure
"what and where is this console," equivalent to reading a name tag. The
security property below is what pairing adds on top.

## The problem pairing solves

A permanent display (Android TV mounted on a sanctuary wall, a Roku plugged
into a lobby TV) is supposed to always show *one specific console's* output.
Without any pairing, that assumption only holds by accident: if a second
OS-Next instance ever starts up on the same network (a demo laptop, a
neighboring ministry sharing the building's WiFi, someone's dev build) and
happens to be reachable at the same IP/hostname the display last used —
whether through a naming collision, a stale cached IP, or the display simply
being pointed at it during setup — the display will render whatever *that*
console sends, with no indication anything changed. That's the "hijacking"
this is meant to prevent: not a sophisticated attack, just an ordinary LAN
mishap that a lyrics display has no way to notice on its own today.

## The pairing model: Two-Way QR-Bridge

Pairing for dedicated display apps (Android TV, Roku) uses a **two-way QR bridge**
to authorize new hardware securely without requiring typing passwords on a TV remote:

```
[Console Remote Dialog]
      │
      │ 1. Operator opens "Pair TV App"
      │    Console fetches POST /api/pairing/session
      ▼
[Console Screen] ── displays QR: https://<ip>:8443/pairing.html?key=<session_token>
      │
      │ 2. Technician scans with mobile phone camera
      ▼
[Mobile Phone: pairing.html]
      │ Requests camera permission & activates QR reticle
      │
      │ 3. Technician aims phone camera at TV Screen
      ▼
[TV Client App]
      │ Displays pairing QR: {"device_id": "...", "name": "...", "platform": "..."}
      │ Polls GET /api/pairing/status?device_id=...
      ▼
[Mobile Phone: pairing.html]
      │ Scans TV QR & dispatches POST /api/pairing/authorize
      │ Payload: { session_token, device_id, name, platform }
      ▼
[OS-Next Server Engine]
      │ Validates session_token (5 min TTL)
      │ Issues persistent device_token (dev_tok_...)
      │ Writes record to `paired_devices` SQLite table
      │ Responds { success: true } to phone
      │ Returns { paired: true, token: "..." } to TV status poll
      ▼
[TV App] ── Transitions from pairing splash to active presentation view!
[Settings Dialog] ── Real-time display under "📺 Paired Devices" tab
```

Minting a session (`POST /api/pairing/session`), listing paired devices
(`GET /api/pairing/devices`), and revoking one
(`DELETE /api/pairing/devices/:id`) are console-only — they require the
host session token (see below), so a LAN client can't self-pair a rogue
device or read every device's permanent bearer token. `authorize` (called
by the *phone*, not the console) and `status` (polled by a device that has
no token yet) stay open by necessity, gated in practice by needing a
session token only a host-authenticated console could have minted.

A session stays valid for its full 5-minute TTL across *multiple*
authorizations rather than being consumed after the first — a technician
pairs every TV in the building from one console-minted session instead of
`pairing.html` needing to mint follow-up sessions itself (which would mean
giving the phone console-level trust it was never meant to have).

### ADB-driven setup (installs the app and makes it the TV's home app)

The Android TV app isn't published to the Play Store while its future is
uncertain, so it's sideloaded. The console has a direct ADB channel to the TV
while it does that, and uses it to turn the TV into a dedicated display: the
app becomes the TV's **home app** and the stock launcher is switched off, so
the TV boots straight into OpenSanctuary and the Home button returns to it.

ADB does **not** pair the TV. It carries no session token and no credential;
the only thing it tells the TV about trust is the console's certificate
fingerprint (a pin). A TV only ever gets its token from the QR bridge above,
from an operator looking at both screens.

```
[Console] Settings > Paired Devices > "Install via ADB"
      │ Operator enables Network debugging in the TV's Developer Options,
      │ types its IP into the console.
      ▼
POST /api/tv-provision/start -- src/network/adb.rs::provision:
      │ 1. `adb connect <ip>:5555`
      │ 2. `adb install -r <bundled APK>`
      │ 3. Asks the TV which apps answer to the HOME action
      │    (`cmd package query-activities ... android.intent.category.HOME`);
      │    refuses to go on unless OpenSanctuary is one of them. Then
      │    `pm disable-user --user 0 <launcher>` for every other launcher
      │    (never the app itself, never a first-boot setup wizard).
      │ 4. `adb shell am start -n org.opensanctuary.tv/.MainActivity
      │     --es server_ip ... --ei server_port ... --es cert_fingerprint ...
      │     --es console_name ...`
      ▼
[TV App] ── Remembers the console address, pins the certificate, and shows
            its pairing QR. The console jumps to the Pair TV App tab.
      ▼
[Operator's phone] ── scans the TV's QR: the two-way bridge above.
```

The app declares `MAIN` + `HOME` + `DEFAULT` for its activity, which is what
makes it eligible. With the stock launcher disabled it is the only home app,
so Android uses it without asking.

**Undoing it:** the Install-via-ADB tab has *Restore the TV's home screen*
(`POST /api/tv-provision/restore-home`): it re-enables the launcher packages
that are disabled on the TV (`pm list packages -d`, filtered to launchers) and
the TV offers its own home screen again. The app stays installed. If the TV
can't be reached, `adb shell pm enable --user 0 com.google.android.tvlauncher`
(Android TV) or `com.google.android.apps.tv.launcherx` (Google TV) does it by
hand.

A TV installed any other way (no console ADB access) pairs exactly the same
way: it renders its own pairing QR and polls `GET /api/pairing/status`.

Either path also now enforces the actual hijack-prevention this whole
design is for, which the app never did before: every connect compares
`GET /api/server-info`'s `instance_id` against the one stored at pairing
time, and shows a distinct "different console" screen on mismatch instead
of silently rendering whatever answered.

The TV app also pins the console's **TLS certificate fingerprint**, not just
`instance_id`: `PairingManager.kt`'s `pinningTrustManager` hashes the
presented cert the same way `cert_fingerprint_sha256`
(`src/network/tls.rs`) does, trusts it on first connection
(trust-on-first-use, same model as SSH host keys), and rejects a mismatch on
every connection after. Because the console's self-signed cert is
regenerated whenever its private key can't be read back (see
`get_or_create_tls_certificate` in `src/network/tls.rs`), a console with no
OS keyring used to mint a new cert — and a new fingerprint — on every
restart, breaking every already-pinned TV (it would need re-pairing after
every restart). Fixed by `tls.rs`'s owner-only fallback key file, which keeps
the private key (and therefore the cert and its fingerprint) stable across
restarts even with no keyring; see `docs/workflow.md`'s "Known issues" for
that fix's own history.

The key is also kept **per install**. It used to sit in one keyring entry
(`OpenSanctuary:System` / `tls_private_key`) shared by every install on the
machine, so a second install (a portable copy beside an installed one, a test
server) found a key that didn't match its certificate, minted a new pair over
it, and the first install did the same on its next start, breaking pinned TVs
each time. Now each install has a random `tlsKeyId` in its own database (so it
moves with the data folder) and its key lives in `tls_private_key:<id>`; with
no keyring, the owner-only fallback file is `tls_private_key.pem` next to that
install's database. On upgrade an install adopts the old shared key once, only
if it still matches its own certificate, and leaves the shared entry alone for
older builds. Test servers set `OS_NEXT_TLS_KEY_STORE=file`, which keeps the
key in that file and never touches the OS keyring.

### Critical Invariant: Zero-Auth Open Access for Web *Displays*

`live.html` and `stage.html` remain **100% unauthenticated and open on the
LAN** — they're read-only output surfaces (a projector tab, a volunteer's
foldback monitor), so requiring a pairing token before a browser can render
them would be pure friction with no one it's protecting. Pairing is applied
to any surface that can *control* the live show: permanent TV hardware
clients (`apps/android-tv`, `apps/roku`) via the two-way QR-bridge above,
and the mobile remote (`/remote`) via the same underlying `paired_devices`
store, one QR scan (see below).

### Mobile remote pairing (single-QR, reuses the same device store)

`/remote` requires a paired device token — unlike the TV/Roku flow, there's
no second device to bridge to, so it's one step: the console's Pairing menu
("Mobile Remote" tab) calls `POST /api/pairing/remote-session` to mint a
token, embeds it in the QR as `https://<ip>:<port>/remote#token=<token>` (a
URL fragment, so it's never sent to the server as part of the page request
or logged via Referer), and the phone's camera does the rest. `remote.html`
reads the token from the fragment, verifies it against
`POST /api/pairing/verify` before rendering anything, and attaches it to
every command it sends afterward (`x-device-token` header over HTTPS,
`{cmd, device_token}` envelope over the shared `/ws`) — see
`web/src/remote_client.ts`. No token, or one that's been revoked from
Settings > Paired Devices, and the page shows a "Not Paired" screen instead
of connecting. Unlike TV/Roku devices, a remote session is meant to be
short-lived per volunteer; operators revoke it the same way from the Paired
Devices tab once it's no longer needed.

---

## Console (host) authentication

Every "console-only" action — minting a pairing session, listing or revoking
paired devices, minting a remote-control session, **issuing a show-control
command over `POST /api/command` or the shared `/ws`, and loading/replacing
the schedule over `POST /api/schedule/open`** — is gated behind
`host_session_token`, a random value generated fresh each time the server
starts (`src/main.rs`). The design goal: host-level trust means "you have
real access to this machine," established without the credential ever
touching the network, not a token a LAN client could eventually intercept
or guess.

- **Native desktop webview**: the token is injected directly into the
  window's own JS context at creation (`window.__OS_HOST_TOKEN__`), via
  wry's `with_initialization_script` hook (`src/webview/mod.rs`). It's
  never sent over HTTP or WebSocket — the native window is the only thing
  that ever holds it this way.
- **A browser tab on the same machine**: `GET /api/internal/host-token`
  answers only when the request's own TCP connection is loopback
  (127.0.0.1/localhost — the same address the startup banner already
  points operators to), checked server-side via the real peer address, not
  a value the client sends.
- **A console on a different machine** (see "Remote console access" below):
  the token is handed out-of-band (a printed link/QR, or a manually pasted
  value) and confirmed via `POST /api/internal/verify-host-token`.
- Any other caller — including an already-paired remote-control device or
  TV — is never loopback and never knows a bare out-of-band token, so none
  of this changes who *can* obtain it.
- `web/src/core/host_session.ts` is the frontend's single resolution point;
  `keyring.ts` and `api_client.ts`'s `pairing.*`/`sendCommand` methods, plus
  `web/src/core/ws_client.ts`'s `EngineWebSocketClient` (the operator
  console's WS connection) and `web/src/core/schedule_drop.ts`, all use it.

### Remote console access

**Persistent console credential.** The `#host_token=…` link only lives as long as the
server process (a fresh token is generated at every start). The first time a browser
opens that link it exchanges it (`POST /api/pairing/console-credential`) for a
long-lived `console_tok_…` credential kept in that browser's `localStorage`. The
server accepts it anywhere it accepts the host token, so the console keeps working
across restarts with no re-linking. Only a SHA-256 of the credential is stored (as a
`console` device in `paired_devices`); it appears in Settings > Paired Devices, and
removing it there deauthenticates that browser immediately (lost or stolen laptop).
Only the current per-run host token can mint credentials, so adding another browser
means opening the current terminal link, and a stolen credential can't copy itself.

The console doesn't have to run on the same machine as the server — e.g.
the server runs headless on a small box wired to the projector, and the
operator actually drives the show from their own laptop on the same LAN.
Since `GET /api/internal/host-token` only ever answers loopback callers,
such a console needs the token handed to it another way:

- The startup banner (`src/main.rs`) prints a ready-to-open link —
  `https://<lan-ip>:<https-port>/#host_token=<token>` (HTTP, with a visible
  "not encrypted" note, if HTTPS isn't up) — **and** the same URL rendered
  as a scannable QR code directly in the terminal
  (`os_next::network::render_terminal_qr`), so a phone or a laptop's own
  camera app can open it with no typing at all. Whoever can read the
  server's terminal or logs is the only one who gets it — same "host-level
  trust means real access to this machine" design as loopback, just
  extended to bootstrap a remote session instead of only a local one.
- The token travels in the URL fragment (never sent to the server as part
  of the page request, same reasoning as `/remote`'s paired-device token),
  is verified via `POST /api/internal/verify-host-token`, then cached in
  `sessionStorage` (so a reload doesn't need the link again) and stripped
  from the visible address bar immediately.
- If the console can't resolve a token at all (opened from a LAN address
  with no link, no cache), it still renders and shows live state (already
  zero-auth for reading) but shows a small banner to paste in a token read
  aloud or copied from the server operator — see `submitManualHostToken` in
  `host_session.ts` and the banner in `app_core.ts`.

### One console at a time

Only one console may issue commands at once — **first one connected wins**.
The frontend generates a random `console_session_id` once per browser tab
(persisted in `sessionStorage`, so a reload of the same tab keeps its
identity rather than contending with its own just-abandoned connection),
sent alongside the host token on every command (`{cmd, host_token,
console_session_id}` over `/ws`, `x-console-session-id` over HTTP).
`AppState::try_claim_console` (`src/api/ws.rs`) claims the lock for whichever
session presents it first; a different session gets a distinct
`console_locked` rejection instead of a silent drop, surfaced to the
operator as a toast (`onCommandRejected` in `ws_client.ts`). The lock
releases when that console's WS connection disconnects — via a `Drop` guard
so it releases correctly even on an abrupt disconnect, not just a clean
close (see `ConsoleLockGuard` in `handle_socket`). A console that only ever
uses the HTTP fallback (its WS never connects) has no socket to drop, so two
other things release its lock: the page tells the host when it closes
(`POST /api/console/release` from a `pagehide` handler, `releaseConsoleLockOnClose` in
`host_session.ts`), and a lock nobody has touched for 60 seconds (`CONSOLE_HTTP_IDLE`) goes to
the next console that asks. A lock backed by a live WebSocket is only ever released by that
socket's own disconnect.

`/api/command` and `/ws` accept *either* credential, since both a paired
remote and the operator console send real commands through them: a paired
device presents `x-device-token` / `{cmd, device_token}` (checked against
`paired_devices`, see "Mobile remote pairing" above); every other caller
must instead present `x-host-token` / `{cmd, host_token}` matching
`host_session_token`. `/api/schedule/open` is console-only outright (no
paired device has a reason to replace the whole schedule from a file), so it
always requires the host token, with no device-token alternative.

This design replaced several incidents: TLS private key and host-token
leaks, forgeable plugin tokens, unauthenticated pairing management, and a
console-auth gap on `/api/command`/`/ws`.

---

## Server-Side Awareness: The Paired Devices Tab

The server tracks all active display hardware in `library.db` within the `paired_devices`
table:

| Column | Type | Description |
|---|---|---|
| `id` | `TEXT PRIMARY KEY` | Hardware device UUID |
| `name` | `TEXT NOT NULL` | Human-readable name (e.g. "Sanctuary Main TV") |
| `platform` | `TEXT NOT NULL` | Client platform (`android-tv`, `roku`) |
| `paired_at` | `INTEGER NOT NULL` | Epoch timestamp of initial authorization |
| `last_seen` | `INTEGER NOT NULL` | Epoch timestamp of last poll or connection |
| `token` | `TEXT NOT NULL` | Cryptographic client authorization token |

Operators manage paired hardware via **Settings > Paired Devices** (`#options-modal`):
- **Inspect Status**: View active TV clients, platforms, and last seen timestamps.
- **Unpair / Revoke**: Immediately invalidate a device token via `DELETE /api/pairing/devices/:id`.
  The revoked TV is booted back to the pairing screen on its next poll.
- **Pair New Device**: One-click shortcut opening the console's "Pair TV App" QR modal.

---

## Resetting a pairing

Consoles get reinstalled (new `library.db` means a new `instance_id`), and
displays get repurposed to a different room or campus. Either case needs an
explicit, deliberate "forget this console" action in the client — never
automatic, since silently accepting a different `instance_id` is exactly
the failure mode this whole design exists to prevent.

- Displays have a **"Reset Pairing"** button in their settings dialog.
- The console operator can revoke pairing at any time from **Settings > Paired Devices**.
- A display reset on its own side keeps its `device_id`, and the console still lists it. The console hands a token
  out once, so a scan can't pair it again (`409`, "Remove it from the console first"). The Android TV app notices
  this (its status poll says `paired` with no token) and shows **"Remove this device from the console to pair
  again"** with the steps: Settings > Paired Devices, remove it, scan again.

