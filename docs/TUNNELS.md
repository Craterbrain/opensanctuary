# Trusted, offline-capable HTTPS via Caddy + Let's Encrypt DNS-01 (design + setup doc)

_Last edited: 2026-10-02 17:24_

## What this gets you, and why it matters for OS-Next

Every HTTPS surface OS-Next has today — the "Secure Console," `pairing.html`
(camera access requires a secure context), and this session's Remote Console
Access feature — uses a **self-signed** certificate
(`get_or_create_tls_certificate`, `src/network/tls.rs`). That's fine for
reaching a console over loopback, but every non-loopback client (a
technician's phone scanning a pairing QR, an operator's laptop hitting the
printed remote-console link) sees a real "this connection is not private"
browser warning, or in some embedded/native contexts, a hard failure instead
of a click-through.

This doc describes running **[Caddy](https://caddyserver.com)** in front of
OS-Next as a reverse proxy, using the
[`caddy-dns/cloudflare`](https://github.com/caddy-dns/cloudflare) module to
get a real Let's Encrypt certificate via DNS-01 challenge instead of the
usual HTTP-01 challenge. The difference matters specifically for a church
network:

- **HTTP-01** (the default) requires port 80 reachable from the public
  internet at issuance/renewal time — i.e. a real port-forward and a
  connection that's up right then.
- **DNS-01** only requires the ability to create one TXT record via the
  Cloudflare API, which needs internet access but **not** an open inbound
  port and **not** a connection that happens to be up at exactly the right
  moment. Once issued, the certificate is cached on disk and Caddy keeps
  serving it — entirely offline — until it's within 30 days of expiring
  (Let's Encrypt certs are valid 90 days; Caddy renews automatically at that
  point). If the ISP connection is down when renewal is attempted, Caddy
  just keeps serving the still-valid cached cert and retries later — a
  church whose internet drops for a service doesn't lose HTTPS.

The result: `connect.yourchurch.org` resolves (via a local DNS override,
below) to the server's LAN IP, and every device on-site gets a real,
browser-trusted certificate with zero warnings — whether or not the
building's internet is currently working.

## How this fits into OS-Next's architecture

**Since this doc was first written, OS-Next's own network model changed**
(see `docs/update.md`'s history, or `src/main.rs`'s loopback-bind comment):
the cleartext plane is no longer an optional "disable it if you don't need
it" thing reachable from the LAN -- it's now *always* bound to `127.0.0.1`
only, and HTTPS (self-signed by default) is the only plane ever reachable
from the network. This actually makes the Caddy setup below simpler, not
harder: the cleartext plane was already something only a same-machine
process should ever talk to, and now that's enforced at the socket level
instead of by convention.

Caddy sits **in front of** OS-Next and reverse-proxies to its loopback-only
cleartext plane, exactly as before -- this still works unchanged because
Caddy runs on the *same machine*, and `127.0.0.1` is reachable by any local
process regardless of whether it's also reachable from the network:

```
Internet (only for cert issuance/renewal, every ~60-90 days)
      |
[Caddy :80/:443] --- real Let's Encrypt cert, terminates TLS here
      |
      | reverse_proxy 127.0.0.1:<server_port>   (OS-Next's loopback-only plane)
      v
[OS-Next :8080, bound to 127.0.0.1 only]   --- same app, same routes, unmodified
```

- Point Caddy's `reverse_proxy` at OS-Next's loopback plane (`8080` by
  default, `--port`/Settings-configured), not its HTTPS port (`8443`).
  Caddy is already doing the one real TLS termination; there's no reason to
  also terminate TLS a second time inside OS-Next for traffic that already
  arrived decrypted-then-re-encrypted-by-Caddy.
- OS-Next's own HTTPS plane (`:8443`, self-signed) can no longer be
  disabled, and doesn't need to be -- it's LAN-reachable but never
  port-forwarded, so it coexists peacefully as a fallback (a self-signed
  cert warning, same as any device on the LAN gets without Caddy) alongside
  Caddy's real-cert public URL. There's nothing to turn off.
- **Only forward ports 80/443** on the church's router, to whichever machine
  runs Caddy. OS-Next's own ports (`8080`, `8443`) should stay LAN-internal —
  never port-forwarded directly (`8080` can't be reached from the network at
  all now regardless). Caddy is the only thing that needs to be
  internet-reachable, and only for the DNS-01 challenge's outbound API call
  to Cloudflare (inbound port 80 isn't even required for DNS-01, unlike
  HTTP-01).

### The reverse-proxy gap this setup exposed — fixed

`GET /api/internal/host-token` (`src/api/routes.rs`, the route that hands a
non-native console its `host_session_token` — see `docs/CLIENT_PAIRING.md`
"Console (host) authentication") decides whether to answer by checking the
request's real origin. It used to check only the request's *own TCP
connection* (`ConnectInfo<SocketAddr>` → `addr.ip().is_loopback()`) — the
only place in the backend that made a security decision this way (verified
by grepping the whole `src/` tree for `is_loopback`).

**Running Caddy in front of OS-Next on the same machine** (`reverse_proxy
127.0.0.1:8080`) means every request Caddy forwards arrives at OS-Next
*from* `127.0.0.1` — because that's genuinely who opened the TCP connection
to OS-Next's listener; Caddy did, on the app's own loopback interface. Left
unfixed, **every visitor in the world would look like a loopback caller**,
because Caddy sits between OS-Next and the network. That would have
silently defeated the entire point of the loopback check: an attacker on
the internet could have hit `https://connect.yourchurch.org/api/internal/host-token`
and gotten the real host token handed back, no different from a real
operator sitting at the machine.

**Fixed** in `src/network/mod.rs::resolve_real_client_ip` — the standard
"trusted proxy" pattern (the same idea as nginx's
`set_real_ip_from`/`real_ip_header`, or Rails'
`config.action_dispatch.trusted_proxies`): only trust an `X-Forwarded-For`
header's client-IP value when the *immediate* TCP peer is itself loopback
(i.e. the request genuinely arrived via something running on this same
machine), and in that case treat *that* address, not `127.0.0.1`, as the
real caller for the loopback check. A request whose immediate peer is
**not** loopback gets no such override — nobody off-machine can spoof this
by just adding their own `X-Forwarded-For` header, because their own TCP
connection was never loopback to begin with. `get_internal_host_token` now
calls this before its loopback check; no config flag needed — a bare TCP
loopback connection with no forwarding header (native webview, or a browser
tab hitting `127.0.0.1` directly, with no proxy involved) behaves exactly as
it did before.

Verified three ways: a pure-function unit test
(`network::tests::test_resolve_real_client_ip_trusts_forwarded_header_only_from_loopback`)
covering the forged-header/multi-hop/garbage-header cases; a real-server
integration test extending `test_client_pairing_lifecycle_and_zero_auth_displays`
that hits the actual route over a real loopback TCP connection with a
spoofed `X-Forwarded-For: 203.0.113.7` and confirms `403 Forbidden`, then
confirms `X-Forwarded-For: 127.0.0.1` still succeeds; and a live `curl`
check against the real release binary reproducing the exact scenario this
section describes:

```
$ curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:PORT/api/internal/host-token
200   # direct loopback, no proxy -- unaffected
$ curl -o /dev/null -w '%{http_code}\n' -H 'X-Forwarded-For: 203.0.113.7' http://127.0.0.1:PORT/api/internal/host-token
403   # a proxy reporting a real remote client -- correctly rejected
$ curl -o /dev/null -w '%{http_code}\n' -H 'X-Forwarded-For: 127.0.0.1' http://127.0.0.1:PORT/api/internal/host-token
200   # a proxy reporting a loopback original client -- still allowed
```

Caddy in front of OS-Next is now safe to run as described in this doc — no
extra firewalling of `/api/internal/host-token` beyond the normal "only
forward 80/443" guidance above is required.

### Public HTTPS URL — preferring the trusted domain everywhere

Once a church has `connect.yourchurch.org` set up, **Settings > Network >
Public HTTPS URL** lets every place in the app that would otherwise fall
back to the self-signed cert (and warn the user about it) prefer the
trusted public domain instead:

- The **Remote Console Access** banner/QR printed at startup (`src/main.rs`)
  prints `https://connect.yourchurch.org/#host_token=...` as the primary
  line/QR once the setting is non-empty, with the self-signed
  `https://<lan-ip>:<https-port>/#host_token=...` URL still printed
  underneath as a fallback — it's never silently dropped, since the public
  URL depends on external DNS/Caddy actually being up, which the banner has
  no way to verify at boot.
- `presentation_helpers.ts`'s `buildPairingUrl`/`buildRemoteUrl` (the TV/Roku
  pairing bridge and Mobile Remote QR) prefer the configured public URL over
  the self-signed `https_port` plane — for `pairing.html` specifically, a
  trusted cert removes any ambiguity around camera-API secure-context
  requirements on stricter mobile browsers.
- `GET /api/network/info`'s `pairing_url`/`remote_url` prefer it too, and
  the response carries the raw value back as `public_https_url` for any
  client that wants it directly.

**Setting it**: paste the `https://` URL into Settings > Network > Public
HTTPS URL. A live badge next to the field verifies it end-to-end as you
type (debounced) — it's not just "looks like a URL," it actually calls `GET
/api/network/check-public-url` server-side, which fetches
`{url}/api/server-info` and confirms the `instance_id` that comes back
matches *this* server, not just that some HTTPS server answered. That
catches the easy mistakes (DNS not propagated yet, Caddy not running, or
the domain accidentally pointing at a different machine) before you rely on
it. Saving an invalid value (wrong scheme, no host) is rejected server-side
too, same as the hostname/port conflict checks above it.

## Critical prerequisite: offline local DNS

When the building's internet is down, phones and laptops can't query public
DNS (`1.1.1.1`, `8.8.8.8`) to resolve `connect.yourchurch.org` at all —
they need to get that answer from something on the LAN:

- In the church's router (or an internal DNS server like Pi-hole/AdGuard),
  add a local DNS override: `connect.yourchurch.org` → the server's LAN IP
  (e.g. `192.168.1.50`).
- If the router has **DNS rebinding protection** enabled (many consumer
  routers do, by default, as an anti-malware measure), it will otherwise
  refuse to let a public domain name resolve to a private IP — add
  `yourchurch.org` to its allow-list.

Without this, Caddy's certificate is real and trusted, but nothing on-site
can find the server by name once the internet is out — the local DNS
override is what makes this actually offline-capable, not just "has a real
cert."

## The Caddyfile

Standard Caddy builds don't include the Cloudflare DNS module — you need a
build compiled with `github.com/caddy-dns/cloudflare` (see the
platform-specific install steps below). Point it at OS-Next's cleartext
port:

```caddyfile
connect.yourchurch.org {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }

    reverse_proxy localhost:8080
}
```

Prefer an environment variable (`{env.CLOUDFLARE_API_TOKEN}`, set in the
service definition below) over pasting the token directly into the
Caddyfile — the token only needs the Cloudflare **Zone.DNS: Edit** permission
scoped to this one zone, not full account access; create it as its own
scoped API token in the Cloudflare dashboard, not your global API key.

## Setup for Windows (Home Edition)

Windows Home has no built-in way to run a standalone `.exe` as a persistent
background service — [NSSM](https://nssm.cc) (Non-Sucking Service Manager)
fills that gap.

### Quickest path: run the script

`packaging/setup-tunnel-windows.ps1` automates everything below — Caddy
download, Caddyfile, the NSSM service, and the firewall rules — in one
command, from an **Administrator PowerShell**:

```powershell
.\packaging\setup-tunnel-windows.ps1 -Domain connect.yourchurch.org
```

It prompts for the Cloudflare API token securely (never a command-line
argument, so it never lands in PowerShell history). Pass `-DryRun` first if
you want to see exactly what it would do without changing anything — every
command it would run gets printed either way. It's safe to re-run: an
existing install just gets its Caddyfile/token updated and the service
restarted, instead of failing.

It also checks 80/443 for a port collision before doing anything else
(the most common reason this kind of setup fails — IIS, a stray previous
Caddy, etc.) and, if its own `New-NetFirewallRule` step fails for any reason,
prints the exact manual firewall commands from step 2 below directly in its
output — you're never stuck without an answer.

What it does **not** automate: the local DNS override ("Critical
prerequisite" above) and setting Public HTTPS URL in OS-Next itself once
Caddy is up — both still require a manual step.

The manual steps it automates are documented next, for anyone who'd rather
do this by hand, review exactly what the script does before running it, or
fix a step the script couldn't finish on its own.

### 1. Download Caddy with the Cloudflare plugin

Go to the [Caddy download page](https://caddyserver.com/download), add
`github.com/caddy-dns/cloudflare` under "Plugins," and download the Windows
(amd64) build.

- Place `caddy.exe` in a permanent directory, e.g. `C:\Caddy\caddy.exe`.
- Place the Caddyfile alongside it: `C:\Caddy\Caddyfile` (use
  `root * C:/ChurchSite` / Windows-style paths if serving static files
  instead of/alongside `reverse_proxy`).

### 2. Open the firewall for HTTP/HTTPS

In an Administrator PowerShell:

```powershell
New-NetFirewallRule -DisplayName "Caddy-HTTP" -Direction Inbound -LocalPort 80 -Protocol TCP -Action Allow
New-NetFirewallRule -DisplayName "Caddy-HTTPS" -Direction Inbound -LocalPort 443 -Protocol TCP -Action Allow
```

### 3. Install and run Caddy as a background service

```powershell
# Install NSSM (or download manually from nssm.cc if winget can't find it)
winget install -e --id NSSM.NSSM --accept-source-agreements --accept-package-agreements

# Register Caddy as a service, with the Cloudflare token as an env var
nssm install Caddy "C:\Caddy\caddy.exe" "run --config C:\Caddy\Caddyfile"
nssm set Caddy AppEnvironmentExtra "CLOUDFLARE_API_TOKEN=your-scoped-token-here"
nssm set Caddy Start SERVICE_AUTO_START
nssm start Caddy
```

**Verification:** `Get-Service Caddy` should report `Running`. Caddy now
starts silently on boot, even with no user logged in. Check
`C:\Caddy\Caddyfile`-relative logs, or run `nssm status Caddy`, if the
service doesn't come up — the most common cause is the Cloudflare token
lacking DNS-edit permission on the zone.

## Setup for Linux (Debian, Ubuntu, Arch, RHEL)

On Linux, Caddy runs as a native `systemd` unit with automatic permission
dropping.

### Quickest path: run the script

`packaging/setup-tunnel-linux.sh` automates everything below — Caddy
download, Caddyfile, the systemd service, and the firewall rules — in one
command (needs root for binding 80/443, systemd, and the firewall):

```bash
sudo packaging/setup-tunnel-linux.sh --domain connect.yourchurch.org
```

It prompts for the Cloudflare API token securely (never a command-line
argument, so it never lands in shell history or `ps`). Pass `--dry-run`
first if you want to see exactly what it would do without changing
anything — every command it would run gets printed either way. It's safe
to re-run: an existing install just gets its Caddyfile/token updated and
reloaded, instead of failing.

It also checks 80/443 for a port collision before doing anything else
(the most common reason this kind of setup fails — Apache, a stray
previous Caddy instance, etc.) and, if its own `ufw`/`firewall-cmd` step
fails or neither tool is found, prints the exact manual firewall commands
from step 2 below directly in its output — you're never stuck without an
answer.

What it does **not** automate: the local DNS override ("Critical
prerequisite" above) and setting Public HTTPS URL in OS-Next itself once
Caddy is up — both still require a manual step.

The manual steps it automates are documented next, for anyone who'd rather
do this by hand, review exactly what the script does before running it, or
fix a step the script couldn't finish on its own.

### 1. Get a Caddy build with the Cloudflare plugin

Simplest: download a prebuilt custom binary from the
[Caddy download page](https://caddyserver.com/download) the same way as the
Windows steps above, with `github.com/caddy-dns/cloudflare` added, then:

```bash
sudo mv caddy_custom_linux_amd64 /usr/bin/caddy
sudo chmod +x /usr/bin/caddy
```

Alternative, if you'd rather build it yourself with `xcaddy` (Debian/Ubuntu):

```bash
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/xcaddy/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-xcaddy-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/xcaddy/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-xcaddy.list
sudo apt update && sudo apt install -y xcaddy

sudo xcaddy build --with github.com/caddy-dns/cloudflare --output /usr/bin/caddy
```

(On Arch/RHEL without that apt repo, install `go` and run `go install
github.com/caddyserver/xcaddy/cmd/xcaddy@latest` instead, then the same
`xcaddy build` command.)

### 2. Open the firewall

```bash
# UFW (Debian/Ubuntu)
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp

# firewalld (RHEL/Fedora)
sudo firewall-cmd --permanent --add-service=http
sudo firewall-cmd --permanent --add-service=https
sudo firewall-cmd --reload
```

### 3. Configuration and service

```bash
sudo mkdir -p /etc/caddy
sudo nano /etc/caddy/Caddyfile   # paste the Caddyfile above
```

```bash
sudo useradd --system --shell /bin/false --create-home caddy || true
sudo curl -sSL https://raw.githubusercontent.com/caddyserver/dist/master/init/caddy.service -o /etc/systemd/system/caddy.service

# The Cloudflare token needs to reach Caddy's environment -- add an
# override rather than editing the fetched unit file directly, so a future
# re-fetch of caddy.service doesn't silently drop it:
sudo mkdir -p /etc/systemd/system/caddy.service.d
printf '[Service]\nEnvironment=CLOUDFLARE_API_TOKEN=your-scoped-token-here\n' | sudo tee /etc/systemd/system/caddy.service.d/override.conf

sudo systemctl daemon-reload
sudo systemctl enable --now caddy
```

**Verification:** `sudo systemctl status caddy` should report `active
(running)`. Watch certificate issuance in real time with `journalctl -u
caddy -f` on first start — a successful DNS-01 issuance logs the TXT record
being created and verified, then "certificate obtained successfully."

## End-to-end verification checklist

- [ ] `connect.yourchurch.org` resolves to the server's LAN IP from a device
      on the church Wi-Fi, both with internet up and with the WAN link
      unplugged (proves the local DNS override actually works offline).
- [ ] Opening `https://connect.yourchurch.org` shows a padlock with no
      warning, on a phone that has never visited this server before.
- [ ] `journalctl -u caddy -e` (Linux) / the NSSM-managed service log
      (Windows) shows a successful certificate issuance, and again ~60-90
      days later shows a renewal, without anyone doing anything.
- [ ] With the WAN link unplugged, existing HTTPS access keeps working
      until the cached certificate's actual expiry — confirming this is
      genuinely offline-capable, not just "worked once while online."
