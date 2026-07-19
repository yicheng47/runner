#!/usr/bin/env bash
# Phase 1 exit criterion 5 (impl 0031): prove the no-Tauri packaging
# path once — assemble a .app from the bare cargo binary, codesign
# with hardened runtime, notarize, staple, and pass Gatekeeper.
#
# Usage:
#   crates/native-spike/package.sh                 # build + sign only
#   crates/native-spike/package.sh --notarize      # + notarize/staple
#
# Signing identity: first "Developer ID Application" in the keychain,
# or override with SIGN_IDENTITY. Notarization credentials: either a
# notarytool keychain profile (NOTARY_PROFILE=<name>) or the same env
# vars CI uses (APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID).
set -euo pipefail

cd "$(dirname "$0")/../.."

APP_NAME="Runner Native Spike"
BUNDLE_ID="com.wycstudios.runner.native-spike"
DIST="target/native-spike-dist"
APP="$DIST/$APP_NAME.app"
NOTARIZE=false
[[ "${1:-}" == "--notarize" ]] && NOTARIZE=true

echo "==> cargo build --release -p native-spike"
cargo build --release -p native-spike --bin native-spike

echo "==> assembling $APP"
rm -rf "$DIST"
mkdir -p "$APP/Contents/MacOS"
cp target/release/native-spike "$APP/Contents/MacOS/native-spike"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>native-spike</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.0.0</string>
    <key>CFBundleVersion</key><string>0.0.0</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

SIGN_IDENTITY="${SIGN_IDENTITY:-$(security find-identity -v -p codesigning \
    | awk -F'"' '/Developer ID Application/ {print $2; exit}')}"
if [[ -z "$SIGN_IDENTITY" ]]; then
    echo "!! no Developer ID Application identity found; set SIGN_IDENTITY" >&2
    exit 1
fi

echo "==> codesign as: $SIGN_IDENTITY"
codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
codesign --verify --strict --verbose=2 "$APP"

if $NOTARIZE; then
    ZIP="$DIST/native-spike.zip"
    echo "==> notarizing"
    ditto -c -k --keepParent "$APP" "$ZIP"
    if [[ -n "${NOTARY_PROFILE:-}" ]]; then
        xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
    else
        : "${APPLE_ID:?set NOTARY_PROFILE or APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID}"
        xcrun notarytool submit "$ZIP" \
            --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
            --team-id "$APPLE_TEAM_ID" --wait
    fi
    echo "==> stapling"
    xcrun stapler staple "$APP"
    echo "==> gatekeeper assessment"
    spctl -a -vv "$APP"
fi

echo "==> done: $APP"
