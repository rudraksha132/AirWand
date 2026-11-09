#!/bin/bash
# Builds AirWand.app for this Mac. Needs Xcode or the Command Line Tools.
#   ./build.sh        build
#   ./build.sh run    build and open
set -euo pipefail
cd "$(dirname "$0")"

APP="build/AirWand.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

echo "Compiling…"
swiftc -O \
  -o "$APP/Contents/MacOS/AirWand" \
  Sources/*.swift ../Shared/Protocol.swift \
  -framework AppKit -framework CoreBluetooth -framework ApplicationServices

cp Info.plist "$APP/Contents/Info.plist"
cp Resources/game.html "$APP/Contents/Resources/game.html"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

# Ad-hoc signature. After every rebuild macOS may forget the Accessibility
# permission: turn AirWand off and on again in
# System Settings > Privacy & Security > Accessibility.
codesign --force --deep --sign - "$APP" >/dev/null

echo "Built $APP"
if [[ "${1:-}" == "run" ]]; then
  pkill -x AirWand 2>/dev/null || true
  open "$APP"
fi
