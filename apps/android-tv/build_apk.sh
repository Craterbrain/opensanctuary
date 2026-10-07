#!/usr/bin/env bash
# Builds the Android TV client for ADB sideloading (see docs/CLIENT_PAIRING.md's
# ADB-provisioning plan). Builds the **debug** variant only: `adb install`
# doesn't need Play Store-grade signing, and Gradle's own auto-generated
# debug keystore already signs it -- assembleRelease has no signingConfig
# set up (and doesn't need one until/unless this ever goes to the Play
# Store for real), so building it here would just fail or produce an
# unsigned, uninstallable APK.
#
# Uses this project's own pinned `./gradlew`, not a system `gradle`, so the
# build is reproducible regardless of what else is installed on the machine
# running it (matches packaging/Dockerfile.linux-build's Android SDK setup).
#
# Usage: apps/android-tv/build_apk.sh
# Output: apps/android-tv/app/build/outputs/apk/debug/app-debug.apk

set -euo pipefail

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

echo "==> Building OpenSanctuary Android TV client (debug, for sideloading)..."
chmod +x ./gradlew
./gradlew assembleDebug --console=plain

OUT="$DIR/app/build/outputs/apk/debug/app-debug.apk"
if [ ! -f "$OUT" ]; then
    echo "Build reported success but $OUT is missing." >&2
    exit 1
fi
echo "==> Built: $OUT"
echo "$OUT"
