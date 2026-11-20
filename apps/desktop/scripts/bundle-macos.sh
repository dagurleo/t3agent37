#!/bin/sh
# Builds t3agent37.app into target/release/bundle, signed ad hoc (it runs on this Mac).
# --open: also quits a running t3agent37 and opens the new one from there.
set -eu
cd "$(dirname "$0")/.."

NAME="t3agent37"
# (The Tauri app's before it: macOS keys notification permission and the login item to it,
# and the data folder, ~/Library/Application Support/dev.t3agent37.app, is named after it.)
BUNDLE_ID="dev.t3agent37.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 1)
OUT=target/release/bundle
APP="$OUT/$NAME.app"

cargo build --release --bin t3agent37

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/t3agent37 "$APP/Contents/MacOS/$NAME"
cp icons/icon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>$NAME</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$BUILD</string>
    <!-- (Login items through SMAppService.) -->
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <!-- A menu-bar app: no Dock icon, no menu bar of its own. -->
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>MIT licensed</string>
</dict>
</plist>
PLIST
codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"
echo "$APP"

if [ "${1:-}" = "--open" ]; then
    osascript -e "tell application id \"$BUNDLE_ID\" to quit" 2>/dev/null || true
    # (it lets go of its ports as it quits)
    while lsof -nP -iTCP:47370 -sTCP:LISTEN >/dev/null 2>&1; do sleep 0.5; done
    open "$APP"
fi
