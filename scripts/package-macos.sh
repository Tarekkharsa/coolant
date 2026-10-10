#!/bin/sh
# Builds Coolant.app (the dashboard plus the coolant CLI) and a .dmg.
#
#   scripts/package-macos.sh            → dist/Coolant-<version>.dmg
#
# Signing: ad-hoc by default, so the app runs but macOS asks the user to
# confirm it once (System Settings → Privacy & Security → Open Anyway).
# With a Developer ID it is signed and, with a notary profile, notarized,
# so it opens like any app downloaded from the web:
#
#   CODESIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   NOTARY_PROFILE=coolant scripts/package-macos.sh
#
# (Create the profile once with: xcrun notarytool store-credentials coolant)
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
SHORT="${VERSION%%-*}" # CFBundleShortVersionString must be numeric
APP="dist/Coolant.app"
DMG="dist/Coolant-$VERSION.dmg"

cargo build --release -p coolant-app -p coolant-cli

rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/coolant-app target/release/coolant "$APP/Contents/MacOS/"
cp packaging/macos/AppIcon.icns "$APP/Contents/Resources/"
cat >"$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Coolant</string>
  <key>CFBundleDisplayName</key><string>Coolant</string>
  <key>CFBundleIdentifier</key><string>io.github.tarekkharsa.coolant</string>
  <key>CFBundleExecutable</key><string>coolant-app</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$SHORT</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT License</string>
</dict>
</plist>
EOF

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
  # Hardened runtime and a timestamp are required for notarization.
  codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$APP/Contents/MacOS/coolant"
  codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$APP"
else
  codesign --force --sign - "$APP/Contents/MacOS/coolant"
  codesign --force --sign - "$APP"
fi
codesign --verify --strict "$APP"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Coolant" -srcfolder "$STAGE" -ov -format UDZO "$DMG"

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$CODESIGN_IDENTITY" "$DMG"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
  fi
fi
# scripts/install.sh checks downloads against this file (upload it with the .dmg).
(cd dist && shasum -a 256 "$(basename "$DMG")" >"$(basename "$DMG").sha256")
cat "$DMG.sha256"
echo "built $DMG"
