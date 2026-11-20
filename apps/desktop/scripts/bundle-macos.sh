#!/bin/sh
# Builds t3agent37.app into target/release/bundle.
#   --open      then opens it from there
#   --install   then copies it to /Applications and opens it from there
# A copy running from where the new one goes is quit first (macOS kills a process whose signed
# binary changes under it) and opened again after; --open and --install quit any running copy.
#
# A release (RELEASE set, as scripts/release-macos.sh does) runs on Apple silicon and Intel
# alike: built for both and put in one app with lipo. Other builds are for this Mac only;
# UNIVERSAL=1 makes one for both too. (That needs Rust's target for the other kind of Mac:
# rustup target add x86_64-apple-darwin, or aarch64-, run in this folder.)
#
# Signed with your Developer ID (with the hardened runtime, as notarization wants), else your
# Apple Development certificate, else ad hoc; SIGN_IDENTITY picks one. Sparkle (updates) goes in
# Contents/Frameworks. Updates are on when Sparkle's key for t3agent37 is in your keychain (see
# scripts/release-macos.sh): the app then checks the latest GitHub release's appcast.xml once a
# day. Build numbers are commit counts, so a build of yours is offered the first release after it.
set -eu
cd "$(dirname "$0")/.."

NAME="t3agent37"
# (The Tauri app's before it: macOS keys notification permission and the login item to it,
# and the data folder, ~/Library/Application Support/dev.t3agent37.app, is named after it.)
BUNDLE_ID="dev.t3agent37.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 1)
REPO=${REPO:-dagurleo/t3agent37}
FEED_URL=${FEED_URL:-https://github.com/$REPO/releases/latest/download/appcast.xml}
# (The keychain item Sparkle's key is under: one per app, so it isn't another app's.)
SPARKLE_ACCOUNT=t3agent37
OUT=target/release/bundle
APP="$OUT/$NAME.app"
SPARKLE_VERSION=2.10.0
SPARKLE_SHA256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
SPARKLE="target/sparkle-$SPARKLE_VERSION"
MODE=${1:-}

# Quits a running t3agent37 (any copy: there's only ever one) and waits until it has let go
# of its ports.
quit_running() {
    osascript -e "tell application id \"$BUNDLE_ID\" to quit" 2>/dev/null || true
    while lsof -nP -iTCP:47370 -sTCP:LISTEN >/dev/null 2>&1; do sleep 0.5; done
}

cargo build --release --bin t3agent37
BINARY=target/release/t3agent37
if [ -n "${RELEASE:-}" ] || [ -n "${UNIVERSAL:-}" ]; then
    case "$(uname -m)" in
        arm64) OTHER=x86_64-apple-darwin ;;
        *) OTHER=aarch64-apple-darwin ;;
    esac
    if ! rustup target list --installed 2>/dev/null | grep -qx "$OTHER"; then
        echo "a build for Apple silicon and Intel needs Rust's $OTHER target: rustup target add $OTHER" >&2
        exit 1
    fi
    cargo build --release --bin t3agent37 --target "$OTHER"
    mkdir -p target/universal
    lipo -create target/release/t3agent37 "target/$OTHER/release/t3agent37" -output target/universal/t3agent37
    BINARY=target/universal/t3agent37
fi

# Sparkle, once: its release, checked against the hash it was vetted with.
if [ ! -d "$SPARKLE/Sparkle.framework" ]; then
    mkdir -p target/sparkle-dl "$SPARKLE"
    curl -sSL -o "target/sparkle-dl/Sparkle-$SPARKLE_VERSION.tar.xz" \
        "https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz"
    echo "$SPARKLE_SHA256  target/sparkle-dl/Sparkle-$SPARKLE_VERSION.tar.xz" | shasum -a 256 -c - >/dev/null
    tar -xf "target/sparkle-dl/Sparkle-$SPARKLE_VERSION.tar.xz" -C "$SPARKLE"
fi

RUNNING=""
if pgrep -f "^$(pwd)/$APP/Contents/MacOS/$NAME" >/dev/null 2>&1; then
    RUNNING=1
    quit_running
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BINARY" "$APP/Contents/MacOS/$NAME"
cp icons/icon.icns "$APP/Contents/Resources/AppIcon.icns"

# Sparkle, less its XPC services (only sandboxed apps need them).
ditto "$SPARKLE/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
rm -rf "$APP/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices" "$APP/Contents/Frameworks/Sparkle.framework/XPCServices"

# Updates: where to look, and the key their archives are signed with. (Without the key,
# generate_keys prints its error where the key would be: keep only a key, 32 bytes in base64.)
UPDATES=""
PUBLIC_KEY=$("$SPARKLE/bin/generate_keys" --account "$SPARKLE_ACCOUNT" -p 2>/dev/null | grep -E '^[A-Za-z0-9+/]{43}=$' || true)
if [ -n "$FEED_URL" ] && [ -n "$PUBLIC_KEY" ]; then
    UPDATES="<key>SUFeedURL</key><string>$FEED_URL</string>
    <key>SUPublicEDKey</key><string>$PUBLIC_KEY</string>
    <key>SUEnableAutomaticChecks</key><true/>"
fi

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
    <key>NSHumanReadableCopyright</key><string>MIT licensed. A fork of t3boat by Pau Kraft.</string>
    $UPDATES
</dict>
</plist>
PLIST

IDENTITY=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*) \([0-9A-F]\{40\}\) "Developer ID Application: .*/\1/p' | head -1)}
IDENTITY=${IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*) \([0-9A-F]\{40\}\) "Apple Development: .*/\1/p' | head -1)}
if [ -n "$IDENTITY" ]; then
    # (Inside out: Sparkle's helpers, Sparkle, then the app; the hardened runtime, and for a
    # release a secure timestamp from Apple's server, as notarization wants.)
    TIMESTAMP=$([ -n "${RELEASE:-}" ] && echo --timestamp || echo --timestamp=none)
    SIGN="codesign --force $TIMESTAMP --options runtime --sign $IDENTITY"
    FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
    $SIGN "$FRAMEWORK/Versions/B/Autoupdate"
    $SIGN "$FRAMEWORK/Versions/B/Updater.app"
    $SIGN "$FRAMEWORK"
    $SIGN --identifier "$BUNDLE_ID" "$APP"
else
    codesign --force --deep --sign - --identifier "$BUNDLE_ID" "$APP"
fi
codesign --verify --deep --strict "$APP"
SIGNER=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
echo "built $APP $VERSION ($BUILD) for $(lipo -archs "$APP/Contents/MacOS/$NAME" | sed 's/ / and /'), signed ${SIGNER:-ad hoc}$([ -n "$UPDATES" ] && echo ", updates from $FEED_URL")"

case "$MODE" in
    --install)
        quit_running
        rm -rf "/Applications/$NAME.app"
        ditto "$APP" "/Applications/$NAME.app"
        echo "installed /Applications/$NAME.app"
        open "/Applications/$NAME.app"
        ;;
    --open)
        quit_running
        open "$APP"
        ;;
    *)
        if [ -n "$RUNNING" ]; then open "$APP"; fi
        ;;
esac
