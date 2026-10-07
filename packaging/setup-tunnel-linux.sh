#!/bin/bash
# Automates the Linux half of docs/TUNNELS.md: installs Caddy (with the
# Cloudflare DNS-01 plugin) as a systemd service reverse-proxying to
# OS-Next's loopback-only plane, so a church gets a real, browser-trusted
# HTTPS domain instead of self-signed-cert warnings. Idempotent -- re-running
# it updates an existing install's Caddyfile/token and reloads rather than
# failing.
#
# Usage:
#   sudo packaging/setup-tunnel-linux.sh --domain connect.yourchurch.org [--port 8080] [--dry-run]
#   (Cloudflare API token is prompted for, never passed as a CLI arg --
#    a plain arg would land in shell history and `ps`.)
#
# What this does NOT automate (see docs/TUNNELS.md for why, and the manual
# steps if you'd rather do any of this by hand):
#   - The local DNS override that makes the domain resolve on-site even when
#     the building's internet is down -- that's done in the church's router
#     or an internal DNS server (Pi-hole/AdGuard), which this script has no
#     access to.
#   - Setting the "Public HTTPS URL" setting in OS-Next itself once this is
#     up -- that's a one-time step in Settings > Network, verified live
#     there via the reachability badge.
#
# Requires root (binds 80/443, writes /etc/systemd and /etc/caddy, manages
# the firewall) -- re-run with sudo if it exits complaining about that.

set -euo pipefail

DOMAIN=""
PORT="8080"
DRY_RUN=0

while [ $# -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

run() {
  # Prints every command this script would run even in dry-run mode, so an
  # admin can review exactly what's about to happen to their machine before
  # it does -- and actually runs it otherwise.
  echo "+ $*"
  if [ "$DRY_RUN" -eq 0 ]; then
    "$@"
  fi
}

if [ "$DRY_RUN" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
  echo "This script needs root (binds 80/443, manages systemd/firewall/caddy). Re-run with sudo." >&2
  exit 1
fi

if [ -z "$DOMAIN" ]; then
  read -rp "Public domain (e.g. connect.yourchurch.org): " DOMAIN
fi
if [ -z "$DOMAIN" ]; then
  echo "A domain is required." >&2
  exit 1
fi

# Never echoed, never passed as a CLI arg -- only needs the Cloudflare
# Zone.DNS:Edit permission scoped to this one zone (docs/TUNNELS.md), not
# full account access.
read -rsp "Cloudflare API token (Zone.DNS:Edit, scoped to this zone): " CLOUDFLARE_API_TOKEN
echo
if [ -z "$CLOUDFLARE_API_TOKEN" ]; then
  echo "A Cloudflare API token is required." >&2
  exit 1
fi

echo "============================================================"
echo " OS-Next Tunnel Setup (Linux) -- docs/TUNNELS.md"
echo "============================================================"
echo " Domain          : $DOMAIN"
echo " OS-Next port    : $PORT"
echo " Dry run         : $([ "$DRY_RUN" -eq 1 ] && echo yes || echo no)"
echo "============================================================"

# --- 1. Port collision check -------------------------------------------
# The single most common reason this kind of setup fails is something else
# already bound to 80/443 (Apache, a stray previous Caddy, etc.) -- fail
# fast with a clear answer instead of a cryptic bind error deep inside
# Caddy's own startup log.
echo "==> Checking 80/443 for existing listeners..."
check_port_collision() {
  local port="$1"
  local listing=""
  if command -v ss >/dev/null 2>&1; then
    listing=$(ss -tlnp "sport = :$port" 2>/dev/null || true)
  elif command -v netstat >/dev/null 2>&1; then
    listing=$(netstat -tlnp 2>/dev/null | grep ":$port " || true)
  else
    echo "    (neither ss nor netstat available -- skipping port $port check)"
    return 0
  fi
  if [ -n "$listing" ] && echo "$listing" | grep -q "LISTEN"; then
    echo "!! Port $port is already in use:"
    echo "$listing" | sed 's/^/     /'
    echo "   Stop or reconfigure whatever that is before continuing, or run this on a different machine."
    return 1
  fi
  return 0
}
PORT_CONFLICT=0
check_port_collision 80 || PORT_CONFLICT=1
check_port_collision 443 || PORT_CONFLICT=1
if [ "$PORT_CONFLICT" -eq 1 ]; then
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "    (dry run -- would abort here for real)"
  else
    exit 1
  fi
fi

# --- 2. Fetch Caddy with the Cloudflare DNS plugin ----------------------
# Caddy's official download API builds a custom binary with exactly the
# plugins requested -- the same official source docs/TUNNELS.md already
# points admins to for the manual download, just automated.
CADDY_BIN="/usr/bin/caddy"
if [ -x "$CADDY_BIN" ] && "$CADDY_BIN" list-modules 2>/dev/null | grep -q "dns.providers.cloudflare"; then
  echo "==> Caddy with the Cloudflare plugin already installed at $CADDY_BIN -- skipping download."
else
  echo "==> Downloading Caddy (with github.com/caddy-dns/cloudflare) from caddyserver.com..."
  ARCH="amd64"
  case "$(uname -m)" in
    aarch64|arm64) ARCH="arm64" ;;
  esac
  DOWNLOAD_URL="https://caddyserver.com/api/download?os=linux&arch=${ARCH}&p=github.com/caddy-dns/cloudflare"
  run curl -fsSL -o /tmp/caddy-tunnel-setup "$DOWNLOAD_URL"
  run chmod +x /tmp/caddy-tunnel-setup
  run mv /tmp/caddy-tunnel-setup "$CADDY_BIN"
fi

# --- 3. Caddyfile -------------------------------------------------------
echo "==> Writing /etc/caddy/Caddyfile..."
if [ "$DRY_RUN" -eq 1 ]; then
  echo "+ write /etc/caddy/Caddyfile:"
  cat <<EOF | sed 's/^/    /'
$DOMAIN {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }

    reverse_proxy localhost:$PORT
}
EOF
else
  mkdir -p /etc/caddy
  cat > /etc/caddy/Caddyfile <<EOF
$DOMAIN {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }

    reverse_proxy localhost:$PORT
}
EOF
fi

# --- 4. systemd service --------------------------------------------------
echo "==> Registering Caddy as a systemd service..."
if ! id caddy >/dev/null 2>&1; then
  run useradd --system --shell /bin/false --create-home caddy
fi
if [ ! -f /etc/systemd/system/caddy.service ]; then
  run curl -sSL https://raw.githubusercontent.com/caddyserver/dist/master/init/caddy.service -o /etc/systemd/system/caddy.service
fi
run mkdir -p /etc/systemd/system/caddy.service.d
if [ "$DRY_RUN" -eq 1 ]; then
  echo "+ write /etc/systemd/system/caddy.service.d/override.conf (mode 600, carries CLOUDFLARE_API_TOKEN)"
else
  printf '[Service]\nEnvironment=CLOUDFLARE_API_TOKEN=%s\n' "$CLOUDFLARE_API_TOKEN" > /etc/systemd/system/caddy.service.d/override.conf
  chmod 600 /etc/systemd/system/caddy.service.d/override.conf
fi
run systemctl daemon-reload
if systemctl is-active --quiet caddy 2>/dev/null; then
  echo "==> Caddy already running -- reloading with the updated config instead of restarting."
  run systemctl reload caddy
else
  run systemctl enable --now caddy
fi

# --- 5. Firewall ---------------------------------------------------------
echo "==> Opening the firewall for 80/443..."
firewall_manual_fallback() {
  echo "   Could not open the firewall automatically. Open it manually with whichever of these applies:"
  echo "     UFW (Debian/Ubuntu):      sudo ufw allow 80/tcp && sudo ufw allow 443/tcp"
  echo "     firewalld (RHEL/Fedora):  sudo firewall-cmd --permanent --add-service=http --add-service=https && sudo firewall-cmd --reload"
  echo "   See docs/TUNNELS.md 'Setup for Linux' step 2 for the full reference."
}
if command -v ufw >/dev/null 2>&1; then
  if ! run ufw allow 80/tcp || ! run ufw allow 443/tcp; then
    firewall_manual_fallback
  fi
elif command -v firewall-cmd >/dev/null 2>&1; then
  if ! run firewall-cmd --permanent --add-service=http || ! run firewall-cmd --permanent --add-service=https || ! run firewall-cmd --reload; then
    firewall_manual_fallback
  fi
else
  echo "   No supported firewall tool found (ufw/firewalld)."
  firewall_manual_fallback
fi

echo "============================================================"
echo " Still to do manually (docs/TUNNELS.md has the full detail):"
echo "============================================================"
echo " 1. Local DNS override: in the church's router (or Pi-hole/AdGuard),"
echo "    add an A record pointing $DOMAIN -> this machine's LAN IP, so"
echo "    on-site devices can resolve it even when the internet is down."
echo "    If the router has DNS rebinding protection, allow-list the domain."
echo " 2. In OS-Next: Settings > Network > Public HTTPS URL, set it to"
echo "    https://$DOMAIN -- the badge there live-verifies it reaches this"
echo "    server once DNS/Caddy are both up."
echo "============================================================"

if [ "$DRY_RUN" -eq 0 ]; then
  echo "==> Waiting a moment for Caddy to obtain the certificate, then verifying..."
  sleep 5
  if curl -fsS -o /dev/null -w '%{http_code}' "https://$DOMAIN" 2>/dev/null | grep -q '^[23]'; then
    echo "    https://$DOMAIN is responding. Watch 'journalctl -u caddy -f' if you want to see issuance/renewal logs."
  else
    echo "    https://$DOMAIN isn't responding yet -- this is normal if DNS hasn't propagated or the local override"
    echo "    above isn't in place yet. Check 'journalctl -u caddy -e' for the actual certificate-issuance status."
  fi
fi
