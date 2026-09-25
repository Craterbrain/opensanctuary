# Client discovery & console pairing (design doc, not yet built)

This covers two related things for **permanent display clients** — the
Roku and Android TV apps, as opposed to someone opening `live.html` in a
random browser tab: how they *find* an OS-Next console on the LAN, and how
they *stay* pointed at the right one once found. The first half (discovery)
is implemented today. The second half (pairing) is a design only — it needs
a real client to be meaningful, and neither the Roku app nor a rewritten
Android TV app exists yet. Read this before building either.

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
[Console Screen] ── displays QR: http://<ip>:8080/pairing.html?key=<session_token>
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
token, embeds it in the QR as `http://<ip>:<port>/remote#token=<token>` (a
URL fragment, so it's never sent to the server as part of the page request
or logged via Referer), and the phone's camera does the rest. `remote.html`
reads the token from the fragment, verifies it against
`POST /api/pairing/verify` before rendering anything, and attaches it to
every command it sends afterward (`x-device-token` header over HTTP,
`{cmd, device_token}` envelope over the shared `/ws`) — see
`web/src/remote_client.ts`. No token, or one that's been revoked from
Settings > Paired Devices, and the page shows a "Not Paired" screen instead
of connecting. Unlike TV/Roku devices, a remote session is meant to be
short-lived per volunteer; operators revoke it the same way from the Paired
Devices tab once it's no longer needed.

---

## Console (host) authentication

Every "console-only" action above — minting a pairing session, listing or
revoking paired devices, minting a remote-control session — is gated behind
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
- Any other caller — including an already-paired remote-control device or
  TV — is never loopback and never obtains this token.
- `web/src/core/host_session.ts` is the frontend's single resolution point;
  `keyring.ts` and `api_client.ts`'s `pairing.*` methods use it.

See `docs/GEMINI_COMMIT_REVIEW_2026-09-22.md` for the incidents (TLS
private key and host-token leaks, forgeable plugin tokens, unauthenticated
pairing management) this replaced.

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

