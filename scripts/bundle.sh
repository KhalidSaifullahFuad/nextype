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
  <key>CFBundleVersion</key><string>${BUILD:-1}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
</dict></plist>
PLIST
ID="NexType Dev"
if security find-identity -v -p codesigning | grep -q "\"$ID\""; then
  codesign --force --sign "$ID" "$APP"
else
  echo "warning: \"$ID\" not found (run scripts/make-dev-cert.sh); ad-hoc signing, Accessibility grant will not survive rebuilds" >&2
  codesign --force --sign - "$APP"
fi
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Authority'
