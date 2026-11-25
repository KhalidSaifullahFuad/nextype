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
# DMG with an Applications link, so it opens as "drag NexType to Applications".
DMG=${ZIP%.zip}.dmg
STAGE=$(mktemp -d)
cp -R target/NexType.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname NexType -srcfolder "$STAGE" -format UDZO "$DMG"
rm -rf "$STAGE"
shasum -a 256 "$ZIP" "$DMG"
