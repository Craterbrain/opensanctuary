#!/usr/bin/env bash
# Builds the Flatpak bundle (target/release-artifacts/opensanctuary-<version>-x86_64.flatpak)
# from the generic Linux tarball that packaging/build-deb.sh makes. See docs/installer.md "Flatpak".
#
#   packaging/build-flatpak.sh [TARBALL]
#
# TARBALL defaults to target/release-artifacts/opensanctuary-linux-x64-<version>.tar.gz. Needs
# flatpak; uses flatpak-builder if it's installed, otherwise the org.flatpak.Builder app, and
# installs the GNOME runtime + SDK (per user, from Flathub) if they're missing.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_ID=io.github.craterbrain.OpenSanctuary
MANIFEST=packaging/flatpak/$APP_ID.yml
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
RUNTIME_VERSION=$(sed -n "s/^runtime-version: '\(.*\)'/\1/p" "$MANIFEST")
TARBALL=${1:-target/release-artifacts/opensanctuary-linux-x64-$VERSION.tar.gz}
BUNDLE=target/release-artifacts/opensanctuary-$VERSION-x86_64.flatpak
FLATHUB=https://dl.flathub.org/repo/flathub.flatpakrepo

[ -f "$TARBALL" ] || { echo "No tarball at $TARBALL (run packaging/build-deb.sh first, or pass one)"; exit 1; }

echo "==> Staging $TARBALL"
STAGE=target/flatpak-stage
rm -rf "$STAGE"
mkdir -p "$STAGE/icons"
tar -xzf "$TARBALL" -C "$STAGE"
# Files double-clicked outside the sandbox reach the app through the documents portal only when the launcher says so (@@ %F @@).
sed -e "s/^Icon=.*/Icon=$APP_ID/" -e "s/^Exec=.*/Exec=opensanctuary @@ %F @@/" packaging/opensanctuary.desktop > "$STAGE/$APP_ID.desktop"
# The file types, under the app id (Flatpak only exports mime files named for the app); the icon name follows the app id too.
sed -e "s/<icon name=\"opensanctuary\"\/>/<icon name=\"$APP_ID\"\/>/" packaging/opensanctuary-mime.xml > "$STAGE/$APP_ID.mime.xml"
sed -e "s/@VERSION@/$VERSION/" -e "s/@DATE@/$(date -u +%F)/" "packaging/flatpak/$APP_ID.metainfo.xml" > "$STAGE/$APP_ID.metainfo.xml"
cp packaging/icons/*.png "$STAGE/icons/"
cp web/favicon.svg "$STAGE/icons/scalable.svg"

echo "==> GNOME $RUNTIME_VERSION runtime and SDK"
for ref in "org.gnome.Platform//$RUNTIME_VERSION" "org.gnome.Sdk//$RUNTIME_VERSION"; do
  if ! flatpak info "$ref" >/dev/null 2>&1; then
    flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
    flatpak install --user -y --noninteractive flathub "$ref"
  fi
done

if command -v flatpak-builder >/dev/null; then
  BUILDER=(flatpak-builder)
else
  flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
  flatpak install --user -y --noninteractive flathub org.flatpak.Builder
  BUILDER=(flatpak run --filesystem=host org.flatpak.Builder)
fi

echo "==> Building $APP_ID $VERSION"
"${BUILDER[@]}" --force-clean --disable-rofiles-fuse --state-dir=target/flatpak-builder-state --repo=target/flatpak-repo target/flatpak-build "$MANIFEST"

mkdir -p target/release-artifacts
flatpak build-bundle --runtime-repo="$FLATHUB" target/flatpak-repo "$BUNDLE" "$APP_ID"
echo "==> $BUNDLE"
