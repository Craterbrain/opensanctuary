#!/usr/bin/env bash
# OpenSanctuary Android TV Build Script
set -e

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

echo "======================================================="
echo "Building OpenSanctuary Android TV Live View Application"
echo "======================================================="

if command -v gradle &> /dev/null; then
    gradle assembleRelease assembleDebug
elif [ -f "./gradlew" ]; then
    chmod +x ./gradlew
    ./gradlew assembleRelease assembleDebug
else
    echo "Notice: Gradle build tool not found in local path. To build the APK, run 'gradle assembleRelease' or import into Android Studio."
fi

echo "Android TV Project generated successfully at: $DIR"
