#!/bin/bash
# Builds the Windows installer (docs/installer.md) via Inno Setup, run under
# Wine on Linux. Requires:
#   - `cargo install cargo-xwin` and `rustup target add x86_64-pc-windows-msvc`
#     -- cross-compiles to real MSVC-ABI Windows via clang-cl/lld-link plus a
#     fetched copy of the Windows SDK/CRT (no Windows machine, no MSVC
#     license needed). See docs/installer.md for why this replaced the
#     earlier mingw-w64-based cross-compile (two independent mingw
#     toolchains both crashed WebView2 init; MSVC is the toolchain
#     WebView2/COM is actually built and tested against).
#   - Inno Setup 6 installed under Wine: wine innosetup-6.x.x.exe /VERYSILENT
#   - `curl` and `unzip` on the build machine, to fetch and stage the
#     bundled ffmpeg/ffprobe (see the "Bundled ffmpeg/ffprobe" comment below
#     for why Windows bundles these directly instead of relying on a system
#     package manager the way the `.deb` does).
#
# Usage: packaging/build-windows-installer.sh
# Output:
#   target/installer/opensanctuary-setup-x64.exe                       (installer)
#   target/release-artifacts/opensanctuary-windows-x64-<version>.zip   (portable, no install)

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ISCC="$HOME/.wine/drive_c/Program Files (x86)/Inno Setup 6/ISCC.exe"
if [ ! -f "$ISCC" ]; then
  echo "Inno Setup not found at: $ISCC" >&2
  echo "Install it once: wine <innosetup-installer.exe> /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-" >&2
  exit 1
fi

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')
echo "==> Building version $VERSION"

echo "==> Building web UI assets..."
(cd web && bun install --frozen-lockfile && bun run build)

# The Android TV client APK (docs/CLIENT_PAIRING.md's ADB-provisioning plan)
# -- built once, reused by both this script and build-deb-in-container.sh,
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

# Bundled ffmpeg/ffprobe (BUILDING.md's "Online media import" section,
# docs/installer.md's ffmpeg-bundling decision) -- an LGPLv3 static build
# from BtbN/FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds), NOT
# gyan.dev's builds (those are all GPLv3 regardless of variant). LGPL covers
# everything this app actually uses: ffmpeg's own built-in decoders
# (H.264/HEVC/AV1/VP9 -- no GPL-only library needed to decode any of them)
# and libvpx/libopus (both BSD-licensed) for the VP9/Opus encode side
# (src/storage/transcode.rs) -- this app never touches a GPL-only encoder
# like libx264/libx265. Windows has no system package manager to install it
# via the way the `.deb` does (`Recommends`), so it's bundled directly.
#
# Pinned to a specific dated autobuild tag, not the rolling "latest" alias:
# BtbN only publishes continuous master-branch autobuilds (no stable-
# release-branch builds), so pinning here is what makes this script
# reproducible build-to-build instead of silently picking up whatever
# changed upstream today. Re-pin periodically by updating these two values
# together (check https://github.com/BtbN/FFmpeg-Builds/releases for the
# current win64-lgpl.zip asset name under a recent autobuild-* tag).
FFMPEG_RELEASE_TAG="autobuild-2026-10-01-13-06"
FFMPEG_ASSET_NAME="ffmpeg-N-127054-g9d3f0f2c58-win64-lgpl.zip"
FFMPEG_CACHE="target/ffmpeg-lgpl-cache"
FFMPEG_ZIP="$FFMPEG_CACHE/$FFMPEG_ASSET_NAME"
if [ ! -f "$FFMPEG_ZIP" ]; then
  echo "==> Downloading bundled ffmpeg ($FFMPEG_RELEASE_TAG)..."
  mkdir -p "$FFMPEG_CACHE"
  curl -fsSL -o "$FFMPEG_ZIP" \
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/$FFMPEG_RELEASE_TAG/$FFMPEG_ASSET_NAME"
else
  echo "==> Reusing already-downloaded ffmpeg at $FFMPEG_ZIP"
fi
FFMPEG_ZIP_ROOT="${FFMPEG_ASSET_NAME%.zip}"

echo "==> Cross-compiling release binary (x86_64-pc-windows-msvc via cargo-xwin)..."
# +crt-static (set in .cargo/config.toml for this target) avoids needing the
# VC++ Redistributable (VCRUNTIME140*.dll) on the target machine.
cargo xwin build --release --target x86_64-pc-windows-msvc \
  --no-default-features --features desktop-webview

echo "==> Staging runtime files..."
STAGE=target/installer-stage
rm -rf "$STAGE"
mkdir -p "$STAGE/web/dist" "$STAGE/web/plugins" "$STAGE/tv-client" "$STAGE/ffmpeg"

cp target/x86_64-pc-windows-msvc/release/os-next.exe "$STAGE/OpenSanctuary.exe"

echo "==> Staging bundled ffmpeg..."
unzip -p "$FFMPEG_ZIP" "$FFMPEG_ZIP_ROOT/bin/ffmpeg.exe" > "$STAGE/ffmpeg/ffmpeg.exe"
unzip -p "$FFMPEG_ZIP" "$FFMPEG_ZIP_ROOT/bin/ffprobe.exe" > "$STAGE/ffmpeg/ffprobe.exe"
unzip -p "$FFMPEG_ZIP" "$FFMPEG_ZIP_ROOT/LICENSE.txt" > "$STAGE/ffmpeg/LICENSE-LGPLv3.txt"
cat > "$STAGE/ffmpeg/SOURCE.txt" <<EOF
This ffmpeg.exe/ffprobe.exe build is redistributed under the GNU Lesser
General Public License v3 (see LICENSE-LGPLv3.txt in this folder).

Build: BtbN/FFmpeg-Builds, release tag "$FFMPEG_RELEASE_TAG", asset
"$FFMPEG_ASSET_NAME".
Build source / build scripts: https://github.com/BtbN/FFmpeg-Builds/releases/tag/$FFMPEG_RELEASE_TAG
Upstream FFmpeg source (the exact commit this build was made from is
encoded in the asset filename above, e.g. "N-127054-g9d3f0f2c58" ->
commit 9d3f0f2c58): https://github.com/FFmpeg/FFmpeg
EOF
# No WebView2Loader.dll to bundle: webview2-com-sys statically links
# WebView2LoaderStatic on MSVC targets instead of dynamically importing
# WebView2Loader.dll (see docs/installer.md) -- one fewer runtime file, and
# the exact mechanism the crash-hunt found broken is simply not present.

for f in index.html live.html stage.html remote.html pairing.html style.css favicon.svg favicon.ico favicon.png logo-banner.svg; do
  cp "web/$f" "$STAGE/web/$f"
done
cp web/dist/*.js "$STAGE/web/dist/"
cp web/plugins/hello_world.js "$STAGE/web/plugins/"
[ "${SKIP_TV_APK:-0}" = "1" ] || cp "$TV_APK" "$STAGE/tv-client/opensanctuary-tv.apk"
# NOT songs/public_domain.db: same reasoning as the .deb (Cargo.toml comment) --
# songs_dir resolves to the per-user data dir, not this install-relative path,
# so bundling it here would silently never be read.

echo "==> Compiling installer..."
mkdir -p target/installer
# Wine needs a Windows-style path (Z:\...) for the /D define -- ISCC treats
# it as opaque text substituted verbatim into [Files] Source lines, so a
# raw Linux forward-slash path there doesn't reliably resolve.
STAGE_WINPATH=$(winepath -w "$(pwd)/$STAGE")
wine "$ISCC" "/DMyAppVersion=$VERSION" "/DSourceDir=$STAGE_WINPATH" packaging/opensanctuary.iss

OUT=target/installer/opensanctuary-setup-x64.exe
echo "==> Built: $OUT"

# Portable, no-install distribution: the exact same staged content the
# installer just packaged, just zipped instead of run through Inno Setup --
# "extract and run" for an operator who'd rather not install at all (the
# `dist/windows-x64/` layout this project already shipped by hand before an
# installer existed; `paths::resolve()` already treats a `web/` sibling as
# this exact case). Mirrors the Linux `.tar.gz` alongside the `.deb`.
echo "==> Building portable zip..."
mkdir -p target/release-artifacts
ZIP="target/release-artifacts/opensanctuary-windows-x64-${VERSION}.zip"
rm -f "$ZIP"
(cd "$STAGE" && zip -r -q "$OLDPWD/$ZIP" .)
echo "==> Built: $ZIP"

echo "$OUT"
echo "$ZIP"
