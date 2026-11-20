#!/bin/sh
# t3agent37 for other Macs: built for Apple silicon and Intel, signed with your Developer ID,
# notarized by Apple and stapled, in a DMG; with Sparkle's appcast.xml for updates. Into
# target/release/dist. With --publish, tagged v<version> and released on GitHub with both files:
# the app checks the latest release's appcast.xml for updates, and the site's download button
# links to the latest release's t3agent37.dmg.
#
# Once, before the first release:
#   1. Notary credentials in your keychain, with an app-specific password from
#      account.apple.com > Sign-In and Security:
#        xcrun notarytool store-credentials <profile> --apple-id <your Apple ID> --team-id <team ID>
#      Release with NOTARY_PROFILE=<profile> unless it's named F2APP (the maintainer's).
#   2. Sparkle's signing key for t3agent37, in your keychain. It prints the public half, which
#      bundle-macos.sh reads from there. Keep the keychain item, and a copy of it somewhere safe:
#      every update is signed with it, and without it t3agent37 can't be updated.
#        target/sparkle-2.10.0/bin/generate_keys --account t3agent37
#        target/sparkle-2.10.0/bin/generate_keys --account t3agent37 -x <file>   # the copy
#      (target/sparkle-2.10.0 is there after the first scripts/bundle-macos.sh.)
#   3. Rust's Intel target, in this folder: rustup target add x86_64-apple-darwin
#   4. For --publish: gh auth login
#
# Each release: bump the version in Cargo.toml, commit and push, then
#   scripts/release-macos.sh --publish
set -eu
cd "$(dirname "$0")/.."

NAME=t3agent37
REPO=${REPO:-dagurleo/t3agent37}
NOTARY_PROFILE=${NOTARY_PROFILE:-F2APP}
FEED_URL=https://github.com/$REPO/releases/latest/download/appcast.xml
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TAG=v$VERSION
SPARKLE=target/sparkle-2.10.0
PUBLISH=""
[ "${1:-}" = "--publish" ] && PUBLISH=1

security find-identity -v -p codesigning | grep -q "Developer ID Application" || { echo "no Developer ID Application certificate in the keychain"; exit 1; }
xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1 || { echo "no notary credentials named $NOTARY_PROFILE: see step 1 above"; exit 1; }
if [ -n "$PUBLISH" ]; then
    gh auth status >/dev/null 2>&1 || { echo "gh isn't signed in: gh auth login"; exit 1; }
    # (The build number is the commit count, and the tag goes on this commit: it should be the
    # one on GitHub.)
    [ -z "$(git status --porcelain)" ] || { echo "commit first: a release is built from a commit"; exit 1; }
    git fetch -q origin
    [ -n "$(git branch -r --contains HEAD)" ] || { echo "push first: the release is tagged on this commit"; exit 1; }
    if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
        echo "t3agent37 $VERSION is already out: bump the version in Cargo.toml"
        exit 1
    fi
fi

RELEASE=1 FEED_URL="$FEED_URL" ./scripts/bundle-macos.sh
APP=target/release/bundle/$NAME.app
/usr/libexec/PlistBuddy -c "Print :SUPublicEDKey" "$APP/Contents/Info.plist" >/dev/null 2>&1 || { echo "no Sparkle key in the keychain: see step 2 above"; exit 1; }
# (By its hash, as bundle-macos.sh picks it: two certificates can have the same name.)
IDENTITY=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*) \([0-9A-F]\{40\}\) "Developer ID Application: .*/\1/p' | head -1)}
codesign -dvv "$APP" 2>&1 | grep -q "^Authority=Developer ID Application" || { echo "$NAME.app isn't signed with a Developer ID"; exit 1; }
BUILD=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$APP/Contents/Info.plist")

DIST=target/release/dist
rm -rf "$DIST"
mkdir -p "$DIST"

# The app, notarized and stapled (so it opens offline too, as Sparkle installs it).
echo "notarizing the app…"
ditto -c -k --keepParent "$APP" "$DIST/$NAME.zip"
xcrun notarytool submit "$DIST/$NAME.zip" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$APP"
rm "$DIST/$NAME.zip"

# The DMG: the app, and Applications to drag it to; signed, notarized, stapled. (One name for
# every release, so releases/latest/download/t3agent37.dmg is always the newest.)
DMG="$DIST/$NAME.dmg"
STAGE=$(mktemp -d)
ditto "$APP" "$STAGE/$NAME.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null
rm -rf "$STAGE"
# (The app as it'll be installed, from the DMG, checked strictly.)
hdiutil attach -readonly -nobrowse -mountpoint "$DIST/mnt" "$DMG" >/dev/null
STRICT=$(codesign --verify --deep --strict "$DIST/mnt/$NAME.app" 2>&1 && echo ok || true)
hdiutil detach "$DIST/mnt" >/dev/null
[ "$STRICT" = ok ] || { echo "$NAME.app in the DMG: $STRICT"; exit 1; }
codesign --force --timestamp --sign "$IDENTITY" "$DMG"
echo "notarizing the DMG…"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"

# As Gatekeeper sees them, on a Mac that's never seen t3agent37.
spctl --assess --type open --context context:primary-signature --verbose "$DMG"
spctl --assess --type execute --verbose "$APP"

# Sparkle's appcast: the DMG, signed with the key in your keychain, where this release will
# have it.
"$SPARKLE/bin/generate_appcast" --account t3agent37 \
    --download-url-prefix "https://github.com/$REPO/releases/download/$TAG/" \
    --full-release-notes-url "https://github.com/$REPO/releases/tag/$TAG" \
    -o "$DIST/appcast.xml" "$DIST"
grep -q "sparkle:edSignature" "$DIST/appcast.xml" || { echo "the appcast isn't signed: see step 2 above"; exit 1; }
echo "built $DMG and $DIST/appcast.xml: t3agent37 $VERSION ($BUILD)"

if [ -n "$PUBLISH" ]; then
    git tag "$TAG"
    git push -q origin "$TAG"
    gh release create "$TAG" "$DMG" "$DIST/appcast.xml" --repo "$REPO" --verify-tag \
        --title "t3agent37 $VERSION" --generate-notes
    echo "released t3agent37 $VERSION: https://github.com/$REPO/releases/tag/$TAG (updates from $FEED_URL)"
fi
