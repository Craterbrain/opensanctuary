#!/bin/bash
# Runs *inside* packaging/Dockerfile.linux-build (invoked by build-deb.sh via
# `podman run`) -- against the container's own pinned toolchain so the
# result links against Ubuntu 22.04's glibc/webkit2gtk instead of whatever's
# on the machine actually running the build.
#
# Produces two artifacts from the one build (docs/installer.md's "two
# artifacts, covering different audiences" plan):
#   - target/debian/opensanctuary_<version>-1_amd64.deb -- Debian/Ubuntu/Mint,
#     via `apt`, which resolves the webkit2gtk/gtk3 runtime deps automatically.
#   - target/release-artifacts/opensanctuary-linux-x64-<version>.tar.gz --
#     everyone else (Fedora, Arch, anything not apt-based): the binary + the
#     same runtime-only `web/` subset the Windows portable/installer staging
#     uses (packaging/build-windows-installer.sh), laid out the same way
#     (binary and `web/` as siblings) so `resolve_web_dir()`/`paths::resolve()`
#     find it exactly the way they'd find a portable Windows folder --
#     extract and run, no installer needed. The runtime deps (webkit2gtk-4.1,
#     gtk3) still have to already be on the system; this doesn't bundle them
#     the way the .deb's `apt` dependency resolution does.
#
# Not meant to be run outside the container -- it assumes the rustup/bun/
# cargo-deb setup Dockerfile.linux-build performs.

set -euo pipefail
cd /src

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')

echo "==> Building web UI assets..."
(cd web && bun install --frozen-lockfile && bun run build)

# The Android TV client APK (docs/CLIENT_PAIRING.md's ADB-provisioning plan)
# -- built once, reused by both this script and build-windows-installer.sh,
# since an APK is cross-platform bytecode+resources, not a native binary.
# Idempotent: whichever of the two release scripts runs first actually
# builds it; the other just reuses the cached output.
# SKIP_TV_APK=1 leaves the APK out entirely (no Android toolchain needed).
TV_APK=target/tv-client/opensanctuary-tv.apk
if [ "${SKIP_TV_APK:-0}" = "1" ]; then
  echo "==> SKIP_TV_APK=1: not building or bundling the Android TV client APK"
elif [ ! -f "$TV_APK" ]; then
  echo "==> Building Android TV client APK..."
  mkdir -p target/tv-client
  cp "$(bash apps/android-tv/build_apk.sh | tail -1)" "$TV_APK"
else
  echo "==> Reusing already-built Android TV client APK at $TV_APK"
fi

echo "==> Building release binary..."
cargo build --release

echo "==> Building .deb..."
if [ "${SKIP_TV_APK:-0}" = "1" ]; then
  cargo deb --no-build --variant no-tv-apk
else
  cargo deb --no-build
fi

echo "==> Staging generic Linux tarball..."
STAGE=target/linux-tarball-stage
rm -rf "$STAGE"
mkdir -p "$STAGE/web/dist" "$STAGE/web/plugins" "$STAGE/tv-client"
cp target/release/os-next "$STAGE/opensanctuary"
for f in index.html live.html stage.html remote.html pairing.html style.css favicon.svg favicon.ico favicon.png logo-banner.svg; do
  cp "web/$f" "$STAGE/web/$f"
done
cp web/dist/*.js "$STAGE/web/dist/"
cp -r web/icons "$STAGE/web/icons"
cp web/plugins/hello_world.js "$STAGE/web/plugins/"
[ "${SKIP_TV_APK:-0}" = "1" ] || cp "$TV_APK" "$STAGE/tv-client/opensanctuary-tv.apk"

mkdir -p target/release-artifacts
TARBALL="target/release-artifacts/opensanctuary-linux-x64-${VERSION}.tar.gz"
tar -czf "$TARBALL" -C "$STAGE" .
echo "==> Built: $TARBALL"
