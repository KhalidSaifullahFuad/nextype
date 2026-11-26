#!/bin/zsh
# Build a beta: scripts/release.sh 0.1.0 1  ->  target/dist/NexType-0.1.0-beta.1.{zip,dmg}, plus
# its entry in appcast.xml (the auto-update feed).
# Not notarized (no Developer ID): testers must allow it once in Privacy & Security.
# Signed with the stable "NexType Dev" identity so the Accessibility grant survives updates.
set -euo pipefail
cd "${0:A:h}/.."
VERSION=${1:?usage: release.sh <version> <beta number>}
BETA=${2:?usage: release.sh <version> <beta number>}
# Sparkle compares CFBundleVersion, so it must grow with every release: the commit count does.
BUILD=$(git rev-list --count HEAD)
TAG=v$VERSION-beta.$BETA
VERSION=$VERSION-beta.$BETA BUILD=$BUILD scripts/bundle.sh
mkdir -p target/dist
ZIP=target/dist/NexType-$VERSION-beta.$BETA.zip
rm -f "$ZIP"
ditto -c -k --keepParent target/NexType.app "$ZIP"

# Update feed entry, signed with the EdDSA key in the login keychain (account "nextype",
# made by Sparkle's generate_keys; back it up: generate_keys --account nextype -x <file>).
SIG=$(shell/.build/artifacts/sparkle/Sparkle/bin/sign_update --account nextype "$ZIP")
ITEM="    <item>
      <title>$VERSION beta $BETA</title>
      <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
      <sparkle:version>$BUILD</sparkle:version>
      <sparkle:shortVersionString>$VERSION-beta.$BETA</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url=\"https://github.com/KhalidSaifullahFuad/nextype/releases/download/$TAG/${ZIP:t}\" $SIG type=\"application/octet-stream\"/>
    </item>"
ITEM=$ITEM perl -0pi -e 's/(    <!-- new versions go below -->\n)/$1$ENV{ITEM}\n/' appcast.xml
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
echo "Next: upload both files to the $TAG release, then commit and push appcast.xml (users update from it)."
