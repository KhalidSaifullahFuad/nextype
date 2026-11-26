#!/bin/zsh
# Build target/NexType.app: Rust core (static lib) + uniffi bindings + SwiftUI shell, signed with the
# stable "NexType Dev" identity (scripts/make-dev-cert.sh) so the Accessibility grant survives rebuilds.
set -euo pipefail
cd "${0:A:h}/.."
# Match LSMinimumSystemVersion below, so the C++ objects aren't built for the host's newer macOS.
export MACOSX_DEPLOYMENT_TARGET=14.0
cargo build --release -p nextype-ffi -p uniffi-bindgen
GEN=target/uniffi-swift
mkdir -p "$GEN"
target/release/uniffi-bindgen generate --library target/release/libnextype_ffi.a --language swift --out-dir "$GEN"
mkdir -p shell/Sources/NexType/Generated shell/Sources/nextype_ffiFFI/include
cp "$GEN/nextype_ffi.swift" shell/Sources/NexType/Generated/
cp "$GEN/nextype_ffiFFI.h" shell/Sources/nextype_ffiFFI/include/
(cd shell && swift build -c release)
APP=target/NexType.app
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp assets/AppIcon.icns "$APP/Contents/Resources/"
cp shell/.build/release/NexType "$APP/Contents/MacOS/NexType"
# Sparkle (auto-update). Its XPC services only matter for sandboxed apps, so they're left out.
rm -rf "$APP/Contents/Frameworks"
mkdir -p "$APP/Contents/Frameworks"
ditto shell/.build/release/Sparkle.framework "$APP/Contents/Frameworks/Sparkle.framework"
rm -rf "$APP/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices" "$APP/Contents/Frameworks/Sparkle.framework/XPCServices"
install_name_tool -add_rpath @executable_path/../Frameworks "$APP/Contents/MacOS/NexType" 2>/dev/null || true
# SwiftPM records the deployment target as the SDK version, and AppKit then draws the old
# (pre-macOS 26) window chrome. Record the real SDK so the current look is used; still runs on 14.
vtool -set-build-version macos 14.0 "$(xcrun --show-sdk-version)" -replace \
  -output "$APP/Contents/MacOS/NexType" "$APP/Contents/MacOS/NexType"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.nextype.NexType</string>
  <key>CFBundleName</key><string>NexType</string>
  <key>CFBundleExecutable</key><string>NexType</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION:-0.1.0}</string>
  <key>CFBundleVersion</key><string>${BUILD:-$(git rev-list --count HEAD)}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>SUFeedURL</key><string>https://raw.githubusercontent.com/KhalidSaifullahFuad/nextype/main/appcast.xml</string>
  <key>SUPublicEDKey</key><string>6cCxY+BDatWQ6hfHsfr9PTYaNzyuh1kxcxU70G2fLPI=</string>
  <key>SUEnableAutomaticChecks</key><true/>
</dict></plist>
PLIST
ID="NexType Dev"
if security find-identity -v -p codesigning | grep -q "\"$ID\""; then
  SIGN=$ID
else
  echo "warning: \"$ID\" not found (run scripts/make-dev-cert.sh); ad-hoc signing, Accessibility grant will not survive rebuilds" >&2
  SIGN=-
fi
# Inside out: Sparkle's helpers, the framework, then the app. Sparkle only installs an update
# signed by the same identity as the running app.
FW="$APP/Contents/Frameworks/Sparkle.framework/Versions/B"
codesign --force --sign "$SIGN" "$FW/Autoupdate" "$FW/Updater.app"
codesign --force --sign "$SIGN" "$APP/Contents/Frameworks/Sparkle.framework"
codesign --force --sign "$SIGN" "$APP"
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Authority'
