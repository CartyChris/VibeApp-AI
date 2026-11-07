#!/bin/bash
# Package the release .app into a drag-to-install .dmg.
#
# Uses hdiutil directly rather than create-dmg, because the fancy layout path
# drives Finder over AppleScript and fails wherever automation isn't granted.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
APP="$ROOT/src-tauri/target/release/bundle/macos/VibeApp AI.app"
NAME="VibeApp AI"
VERSION="$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")"
OUT_DIR="${1:-$HOME/Downloads}"
DMG="$OUT_DIR/$NAME $VERSION.dmg"

[ -d "$APP" ] || { echo "error: no built app at $APP — run: npx tauri build --bundles app"; exit 1; }

echo "==> Ad-hoc signing"
# Sign nested code first, then the bundle, so the seal covers everything.
codesign --remove-signature "$APP" 2>/dev/null || true
codesign --force --deep --sign - --timestamp=none "$APP"
codesign --verify --deep --strict --verbose=1 "$APP" 2>&1 | tail -2

echo "==> Staging"
STAGE="$(mktemp -d)/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

# A short read-me so the window isn't bare without a Finder-scripted layout.
cat > "$STAGE/Read Me.txt" <<EOF
$NAME $VERSION
================

To install: drag "$NAME.app" onto the Applications folder beside it.

First launch
------------
This build is ad-hoc signed rather than notarised, so Gatekeeper will ask.
Right-click the app in Applications and choose Open, then confirm once.
Every launch after that is normal. (Or: System Settings > Privacy & Security >
"Open Anyway".)

Getting started
---------------
1. Open the Keys panel and paste an OpenRouter key (openrouter.ai/keys).
   Keys are stored in your macOS Keychain and are never sent anywhere except
   the provider you are calling.
2. Turn on Free Mode in the title bar to build using only \$0 models.
3. Describe your app. It appears on the simulator as it is written.
4. Ship: export a real Xcode project, an Electron desktop app, or a PWA.
EOF

echo "==> Building disk image"
rm -f "$DMG"
mkdir -p "$OUT_DIR"
hdiutil create \
  -volname "$NAME" \
  -srcfolder "$STAGE" \
  -fs HFS+ \
  -format UDZO \
  -imagekey zlib-level=9 \
  -ov -quiet \
  "$DMG"

rm -rf "$(dirname "$STAGE")"

SIZE="$(du -h "$DMG" | cut -f1 | tr -d ' ')"
echo "==> Done: $DMG ($SIZE)"
