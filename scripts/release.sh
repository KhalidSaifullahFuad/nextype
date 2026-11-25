#!/bin/zsh
# Build a beta: scripts/release.sh 0.1.0 1  ->  target/dist/NexType-0.1.0-beta.1.{zip,dmg}
# Not notarized (no Developer ID): testers must allow it once in Privacy & Security.
# Signed with the stable "NexType Dev" identity so the Accessibility grant survives updates.
set -euo pipefail
cd "${0:A:h}/.."
VERSION=${1:?usage: release.sh <version> <beta number>}
BETA=${2:?usage: release.sh <version> <beta number>}
VERSION=$VERSION BUILD=$BETA scripts/bundle.sh
mkdir -p target/dist
ZIP=target/dist/NexType-$VERSION-beta.$BETA.zip
rm -f "$ZIP"
ditto -c -k --keepParent target/NexType.app "$ZIP"
# DMG that opens as "drag NexType to Applications" (layout like other Mac apps).
# create-dmg (brew install create-dmg) briefly drives Finder to save the window layout.
DMG=${ZIP%.zip}.dmg
STAGE=$(mktemp -d)
cp -R target/NexType.app "$STAGE/"
rm -f "$DMG"
create-dmg --volname NexType --volicon assets/AppIcon.icns \
  --window-pos 200 160 --window-size 660 400 --icon-size 128 --text-size 14 \
  --icon NexType.app 180 180 --app-drop-link 480 180 \
  --no-internet-enable "$DMG" "$STAGE" >/dev/null
rm -rf "$STAGE"
shasum -a 256 "$ZIP" "$DMG"
