#!/bin/sh
# OpenSanctuary install.sh -- fetches, verifies, and installs the generic
# Linux tarball release (docs/installer.md's "install.sh -- everyone else":
# Fedora, Arch, anything not apt-based -- Debian/Ubuntu/Mint should use the
# .deb instead, which resolves its own runtime deps via `apt`).
#
# Verification mirrors src/network/updater.rs exactly: GitHub Releases API,
# then checksums.txt + checksums.txt.minisig signed with the same embedded
# Ed25519 public key as the app's own auto-updater. A stolen GitHub token
# can upload a fake release, but can't forge a valid signature without the
# private key. Fails closed: any verification failure aborts before
# anything is written to disk.
#
# Usage:
#   curl -fsSL <url>/install.sh | sh
#   curl -fsSL <url>/install.sh | sh -s -- --system
#
# Honest caveat (docs/installer.md): piping straight to `sh` means trusting
# this script sight-unseen, on top of whatever it downloads -- the
# verification above only covers the downloaded payload, not this script
# itself. To inspect first:
#   curl -fsSL <url>/install.sh -o install.sh
#   less install.sh   # read it
#   sh install.sh

set -eu

# --- Configuration ---------------------------------------------------------

# Mirrors `RELEASES_REPO` in src/network/updater.rs. Keep both, and
# PUBLIC_KEY below, in sync.
REPO="Craterbrain/opensanctuary"

# Mirrors `RELEASE_PUBLIC_KEY_B64` in src/network/updater.rs exactly -- the
# production release key (minisign ID 80DBAA0356AE9867).
PUBLIC_KEY="RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42"

# --- Argument parsing --------------------------------------------------------

SYSTEM_INSTALL=0
FORCE=0
for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=1 ;;
    --force) FORCE=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: install.sh [--system] [--force]

  --system   Install to /opt/opensanctuary + /usr/local/bin (needs root --
             re-run with sudo, or this script will call sudo itself).
             Default is a per-user install, no root needed:
             ~/.local/share/opensanctuary-install + ~/.local/bin.
  --force    Reinstall even if the installed version is already current.
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg (see --help)" >&2
      exit 1
      ;;
  esac
done

if [ "$SYSTEM_INSTALL" = 1 ]; then
  INSTALL_DIR="/opt/opensanctuary"
  BIN_DIR="/usr/local/bin"
  DESKTOP_DIR="/usr/share/applications"
  SUDO=""
  if [ "$(id -u)" != "0" ]; then
    if command -v sudo >/dev/null 2>&1; then
      SUDO="sudo"
    else
      echo "error: --system needs root (no sudo on PATH -- re-run this script as root)" >&2
      exit 1
    fi
  fi
else
  # Deliberately NOT ~/.local/share/opensanctuary: the app's own data
  # directory (library.db, bibles/, songs/, media/ -- docs/paths.md) is
  # ~/.local/share/OpenSanctuary. Different spelling only by case, which is
  # exactly the kind of thing worth never relying on -- a reinstall below
  # does `rm -rf` on this directory, and that must never be able to land on
  # the data directory under any filesystem's case-sensitivity rules.
  INSTALL_DIR="$HOME/.local/share/opensanctuary-install"
  BIN_DIR="$HOME/.local/bin"
  DESKTOP_DIR="$HOME/.local/share/applications"
  SUDO=""
fi

# --- Preflight: platform + required tools -----------------------------------

os="$(uname -s)"
if [ "$os" != "Linux" ]; then
  echo "error: this installer is Linux-only (detected: $os)" >&2
  exit 1
fi

arch="$(uname -m)"
if [ "$arch" != "x86_64" ]; then
  echo "error: only x86_64 builds are published right now (detected: $arch)" >&2
  exit 1
fi

missing=""
for tool in curl tar sha256sum jq minisign; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing="$missing $tool"
  fi
done
if [ -n "$missing" ]; then
  echo "error: missing required tool(s):$missing" >&2
  echo "  Debian/Ubuntu: sudo apt install curl tar coreutils jq minisign" >&2
  echo "  Fedora:        sudo dnf install curl tar coreutils jq minisign" >&2
  echo "  Arch:          sudo pacman -S curl tar coreutils jq minisign" >&2
  exit 1
fi

# --- Fetch + verify ----------------------------------------------------------

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "==> Checking latest release ($REPO)..."
RELEASE_JSON="$WORK_DIR/release.json"
curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" -o "$RELEASE_JSON"

TAG=$(jq -r '.tag_name' "$RELEASE_JSON")
if [ "$TAG" = "null" ] || [ -z "$TAG" ]; then
  echo "error: couldn't read tag_name from the latest release of $REPO" >&2
  echo "  response: $(cat "$RELEASE_JSON")" >&2
  exit 1
fi
VERSION="${TAG#v}"

if [ "$FORCE" != 1 ] && [ -f "$INSTALL_DIR/VERSION" ]; then
  CURRENT="$(cat "$INSTALL_DIR/VERSION" 2>/dev/null || true)"
  if [ "$CURRENT" = "$VERSION" ]; then
    echo "Already up to date (opensanctuary $VERSION). Use --force to reinstall anyway."
    exit 0
  fi
fi

TARBALL_NAME=$(jq -r '[.assets[].name | select(startswith("opensanctuary-linux-x64-") and endswith(".tar.gz"))][0] // empty' "$RELEASE_JSON")
if [ -z "$TARBALL_NAME" ]; then
  echo "error: release $TAG has no opensanctuary-linux-x64-*.tar.gz asset" >&2
  exit 1
fi
TARBALL_URL=$(jq -r --arg name "$TARBALL_NAME" '.assets[] | select(.name == $name) | .browser_download_url' "$RELEASE_JSON")
CHECKSUMS_URL=$(jq -r '.assets[] | select(.name == "checksums.txt") | .browser_download_url' "$RELEASE_JSON")
SIG_URL=$(jq -r '.assets[] | select(.name == "checksums.txt.minisig") | .browser_download_url' "$RELEASE_JSON")

if [ -z "$CHECKSUMS_URL" ] || [ -z "$SIG_URL" ]; then
  echo "error: release $TAG is missing checksums.txt or checksums.txt.minisig -- refusing to install unverified" >&2
  exit 1
fi

echo "==> Downloading opensanctuary $VERSION..."
curl -fsSL "$TARBALL_URL" -o "$WORK_DIR/$TARBALL_NAME"
curl -fsSL "$CHECKSUMS_URL" -o "$WORK_DIR/checksums.txt"
curl -fsSL "$SIG_URL" -o "$WORK_DIR/checksums.txt.minisig"

echo "==> Verifying signature..."
if ! minisign -V -q -P "$PUBLIC_KEY" -m "$WORK_DIR/checksums.txt" -x "$WORK_DIR/checksums.txt.minisig"; then
  echo "error: checksums.txt.minisig did not verify against the embedded public key -- refusing to install" >&2
  exit 1
fi

echo "==> Verifying checksum..."
EXPECTED_HASH=$(grep -E "  ${TARBALL_NAME}\$" "$WORK_DIR/checksums.txt" | awk '{print $1}')
if [ -z "$EXPECTED_HASH" ]; then
  echo "error: checksums.txt has no entry for $TARBALL_NAME -- refusing to install" >&2
  exit 1
fi
ACTUAL_HASH=$(sha256sum "$WORK_DIR/$TARBALL_NAME" | awk '{print $1}')
if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
  echo "error: checksum mismatch for $TARBALL_NAME (expected $EXPECTED_HASH, got $ACTUAL_HASH) -- refusing to install" >&2
  exit 1
fi

echo "Verified: signature and checksum both match."

# --- Install -----------------------------------------------------------------

echo "==> Installing to $INSTALL_DIR..."
$SUDO rm -rf "$INSTALL_DIR"
$SUDO mkdir -p "$INSTALL_DIR"
$SUDO tar -xzf "$WORK_DIR/$TARBALL_NAME" -C "$INSTALL_DIR"
$SUDO chmod +x "$INSTALL_DIR/opensanctuary"
printf '%s' "$VERSION" | $SUDO tee "$INSTALL_DIR/VERSION" >/dev/null

$SUDO mkdir -p "$BIN_DIR"
$SUDO ln -sf "$INSTALL_DIR/opensanctuary" "$BIN_DIR/opensanctuary"

$SUDO mkdir -p "$DESKTOP_DIR"
DESKTOP_FILE="$WORK_DIR/opensanctuary.desktop"
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=OpenSanctuary
Comment=Next-generation church presentation engine
Exec=$INSTALL_DIR/opensanctuary
Icon=$INSTALL_DIR/web/favicon.svg
Terminal=false
Categories=AudioVideo;Presentation;
EOF
$SUDO cp "$DESKTOP_FILE" "$DESKTOP_DIR/opensanctuary.desktop"

echo
echo "Installed opensanctuary $VERSION to $INSTALL_DIR"
if [ "$SYSTEM_INSTALL" != 1 ]; then
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH -- add it to your shell profile, or run $BIN_DIR/opensanctuary directly." ;;
  esac
fi

# Optional, not required for the app itself -- BUILDING.md's "Online media
# import (yt-dlp + ffmpeg)" section. Unlike curl/tar/jq/minisign above,
# missing these doesn't block or fail the install -- the Media tab's
# "import from URL" just won't work (no yt-dlp), or imported H.264 video
# won't get converted to VP9 (no ffmpeg). Same two packages the .deb lists
# as `Recommends` (Cargo.toml's [package.metadata.deb]); there's no
# apt-style "recommends" equivalent for a bare tarball, so this is the
# install.sh version of that nudge.
optional_missing=""
for tool in ffmpeg yt-dlp; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    optional_missing="$optional_missing $tool"
  fi
done
if [ -n "$optional_missing" ]; then
  echo
  echo "Optional, for the Media tab's online video import/conversion:$optional_missing"
  echo "  Debian/Ubuntu: sudo apt install ffmpeg && pip install -U yt-dlp (or your distro's yt-dlp package)"
  echo "  Fedora:        sudo dnf install ffmpeg yt-dlp"
  echo "  Arch:          sudo pacman -S ffmpeg yt-dlp"
  echo "  yt-dlp install options: https://github.com/yt-dlp/yt-dlp#installation"
fi

echo "Run: opensanctuary"
