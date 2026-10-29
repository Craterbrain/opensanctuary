#!/bin/sh
# OpenSanctuary uninstall.sh -- removes what packaging/install.sh put in place (the generic Linux tarball install).
# Also shipped inside the tarball, so an installed copy has it in its own folder:
#   ~/.local/share/opensanctuary-install/uninstall.sh
#
# It removes only what install.sh created: the install folder, the launcher link in the bin folder (only if it points at that install),
# and the desktop entry. Your library (songs, Bibles, media, settings) is KEPT unless you ask for --delete-data.
# A .deb is removed with `sudo apt remove opensanctuary`, a Flatpak with `flatpak uninstall`; this script doesn't touch those.
#
# Usage:
#   sh uninstall.sh                 # per-user install (~/.local/share/opensanctuary-install)
#   sh uninstall.sh --system        # system install (/opt/opensanctuary); uses sudo if not root
#   sh uninstall.sh --delete-data   # also delete the library and settings (asks first; --yes skips the question)

set -eu

SYSTEM=0
DELETE_DATA=0
YES=0
for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM=1 ;;
    --delete-data) DELETE_DATA=1 ;;
    --yes) YES=1 ;;
    -h|--help)
      sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg (see --help)" >&2
      exit 1
      ;;
  esac
done

# Same locations as install.sh -- keep both in step.
if [ "$SYSTEM" = 1 ]; then
  INSTALL_DIR="/opt/opensanctuary"
  BIN_DIR="/usr/local/bin"
  DESKTOP_DIR="/usr/share/applications"
  MIME_DIR="/usr/share/mime"
  SUDO=""
  if [ "$(id -u)" != "0" ]; then
    if command -v sudo >/dev/null 2>&1; then SUDO="sudo"; else
      echo "error: --system needs root (no sudo on PATH -- re-run this script as root)" >&2
      exit 1
    fi
  fi
else
  INSTALL_DIR="$HOME/.local/share/opensanctuary-install"
  BIN_DIR="$HOME/.local/bin"
  DESKTOP_DIR="$HOME/.local/share/applications"
  MIME_DIR="$HOME/.local/share/mime"
  SUDO=""
fi

# Safety: only ever remove a folder that is recognisably an OpenSanctuary tarball install (the VERSION file and the program are there).
if [ ! -f "$INSTALL_DIR/VERSION" ] || [ ! -e "$INSTALL_DIR/opensanctuary" ]; then
  echo "No OpenSanctuary install found at $INSTALL_DIR."
  if [ "$SYSTEM" = 1 ]; then echo "  (For a per-user install, run this without --system.)"; else echo "  (For a system install under /opt, run this with --system.)"; fi
  exit 1
fi
VERSION="$(cat "$INSTALL_DIR/VERSION" 2>/dev/null || true)"

if pgrep -x opensanctuary >/dev/null 2>&1; then
  echo "OpenSanctuary is running. Close it first (it may be live in a service), then run this again." >&2
  exit 1
fi

if [ "$DELETE_DATA" = 1 ] && [ "$YES" != 1 ]; then
  DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/OpenSanctuary"
  printf 'This also deletes your library (songs, Bibles, media, settings) in %s. Type "delete" to continue: ' "$DATA_DIR"
  read -r answer || answer=""
  if [ "$answer" != "delete" ]; then echo "Cancelled; nothing was removed."; exit 1; fi
fi

echo "==> Removing OpenSanctuary $VERSION from $INSTALL_DIR..."
# The launcher link, only if it points into this install.
if [ -L "$BIN_DIR/opensanctuary" ] && [ "$(readlink "$BIN_DIR/opensanctuary")" = "$INSTALL_DIR/opensanctuary" ]; then
  $SUDO rm -f "$BIN_DIR/opensanctuary"
fi
# The desktop entry, only if it launches this install.
if [ -f "$DESKTOP_DIR/opensanctuary.desktop" ] && grep -q "^Exec=$INSTALL_DIR/opensanctuary\( \|\$\)" "$DESKTOP_DIR/opensanctuary.desktop"; then
  $SUDO rm -f "$DESKTOP_DIR/opensanctuary.desktop"
fi
$SUDO rm -rf "$INSTALL_DIR"
# The file types this install registered.
if [ -f "$MIME_DIR/packages/opensanctuary.xml" ]; then
  $SUDO rm -f "$MIME_DIR/packages/opensanctuary.xml"
  if command -v update-mime-database >/dev/null 2>&1; then $SUDO update-mime-database "$MIME_DIR" >/dev/null 2>&1 || true; fi
fi
if [ "$SYSTEM" = 1 ]; then
  $SUDO rm -f /usr/libexec/opensanctuary/setup-local-https /usr/share/polkit-1/actions/io.github.craterbrain.opensanctuary.local-https.policy
  $SUDO rmdir /usr/libexec/opensanctuary 2>/dev/null || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then $SUDO update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true; fi

if [ "$DELETE_DATA" = 1 ]; then
  DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/OpenSanctuary"
  CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/OpenSanctuary"
  if [ -f "$CONFIG_DIR/install.json" ]; then
    echo "Note: $CONFIG_DIR/install.json points your library at another folder; that folder was not touched."
  fi
  rm -rf "$DATA_DIR" "$CONFIG_DIR"
  echo "Deleted your library and settings."
else
  echo "Your library and settings were kept (~/.local/share/OpenSanctuary, ~/.config/OpenSanctuary). Add --delete-data to remove them too."
fi
echo "Removed."
