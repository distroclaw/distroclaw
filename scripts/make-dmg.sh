#!/usr/bin/env bash
# Packs the built DistroClaw.app into a drag-to-Applications DMG with hdiutil.
# Used instead of Tauri's DMG bundler, which needs Finder automation (AppleScript) permission.
set -euo pipefail

cd "$(dirname "$0")/.."
bundle=src-tauri/target/release/bundle
app="$bundle/macos/DistroClaw.app"
# No version in the name, so ".../releases/latest/download/DistroClaw_macOS_arm64.dmg" always works;
# the version is in the release and in Settings > About.
out="$bundle/dmg/DistroClaw_macOS_$(uname -m).dmg"

[ -d "$app" ] || { echo "make-dmg: $app not found (run tauri build first)" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -Rc "$app" "$stage/"            # -c: APFS clone, avoids duplicating 4 GB
ln -s /Applications "$stage/Applications"

mkdir -p "$(dirname "$out")"
rm -f "$out"
# ULFO (lzfse): good speed/size trade-off for mostly incompressible model weights.
hdiutil create -volname "DistroClaw" -srcfolder "$stage" -fs APFS -format ULFO -ov "$out"
echo "make-dmg: $out"
