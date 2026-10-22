#!/bin/bash
# Fetches the plugins every release package bundles into
# target/bundled-plugins/ -- the exact Craterbrain/opensanctuary-plugins commit
# named in packaging/plugins.ref, its plugins/ folder as-is, plus web/plugins/_shared/
# (the plugin SDK and theme the host serves; it lives in this repo, not that one).
#
# Run on the build machine, not inside the Linux build container: build-deb.sh
# calls it before `podman run` (the repo is mounted at /src, so the container
# sees the result), and build-windows-installer.sh calls it directly.
#
# Without this, releases shipped only web/plugins/hello_world.js -- no Genius,
# Online Bibles, Stock Media or yt-dlp plugin (v0.2.0 did exactly that).
#
# Usage: packaging/fetch-plugins.sh [--check]
#   --check  also fail if web/plugins/ (the dev copies) differs from the pinned
#            commit, so the two can't silently drift apart (ci.yml runs this).

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

REF_FILE=packaging/plugins.ref
REPO=$(sed -n 's/^repo=//p' "$REF_FILE")
COMMIT=$(sed -n 's/^commit=//p' "$REF_FILE")
if [ -z "$REPO" ] || ! [[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  echo "$REF_FILE needs repo=<url> and commit=<full 40-char SHA>" >&2
  exit 1
fi

OUT=target/bundled-plugins
SRC=target/bundled-plugins-src
STAMP=target/bundled-plugins.commit

if [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$COMMIT" ] && [ -d "$OUT" ] && [ -d "$SRC/plugins" ]; then
  echo "==> Bundled plugins already at ${COMMIT:0:12}"
else
  echo "==> Fetching plugins from $REPO @ ${COMMIT:0:12}..."
  rm -rf "$SRC" "$OUT" "$STAMP"
  git init -q "$SRC"
  git -C "$SRC" fetch -q --depth 1 "$REPO" "$COMMIT"
  git -C "$SRC" -c advice.detachedHead=false checkout -q FETCH_HEAD
  if [ ! -d "$SRC/plugins" ]; then
    echo "$REPO @ $COMMIT has no plugins/ folder" >&2
    exit 1
  fi
  mkdir -p "$OUT"
  cp -r "$SRC/plugins/." "$OUT/"
  cp -r web/plugins/_shared "$OUT/_shared"
  echo "$COMMIT" > "$STAMP"
fi
echo "    $(find "$OUT" -mindepth 1 -maxdepth 1 | sed 's|.*/||' | sort | tr '\n' ' ')"

if [ "${1:-}" = "--check" ]; then
  if ! diff -r --exclude=_shared "$SRC/plugins" web/plugins; then
    echo "web/plugins/ differs from opensanctuary-plugins @ ${COMMIT:0:12} (above). Update" >&2
    echo "the dev copies to match, or move packaging/plugins.ref to the commit that has them." >&2
    exit 1
  fi
  echo "==> web/plugins/ matches the pinned commit"
fi
