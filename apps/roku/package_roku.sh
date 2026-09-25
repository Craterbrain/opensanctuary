#!/usr/bin/env bash
# OpenSanctuary Roku Channel Packaging Script
set -e

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

OUT_ZIP="opensanctuary-roku.zip"
rm -f "$OUT_ZIP"

echo "======================================================="
echo "Packaging OpenSanctuary Roku SceneGraph Channel ($OUT_ZIP)"
echo "======================================================="

# Verify zip utility
if ! command -v zip &> /dev/null; then
    echo "Error: 'zip' command is required."
    exit 1
fi

zip -r "$OUT_ZIP" manifest source/ components/ images/ -x "*.DS_Store" "*~"

echo ""
echo "✓ Successfully created Roku Channel package: $DIR/$OUT_ZIP"
echo "To sideload onto your Roku device:"
echo "1. Enable Developer Mode on your Roku (Home x3, Up x2, Right, Left, Right, Left, Right)."
echo "2. Open http://<ROKU_IP> in your browser."
echo "3. Upload '$OUT_ZIP' and click 'Install'."
