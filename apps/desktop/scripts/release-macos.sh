#!/bin/sh
# t3agent37 for other Macs: built for Apple silicon and Intel, signed with a Developer ID,
# notarized by Apple and stapled, in a DMG; with Sparkle's appcast.xml for updates. Into
# target/release/dist. And the `t3agent37` command alone (no app), signed and notarized too, as
# t3agent37-macos.tar.gz with its .sha256, into target/release/dist-cli. With --publish, tagged
# v<version> and released on GitHub with those files, and with the Linux builds of the command
# in $EXTRA_ASSETS_DIR if set (the workflow builds them): the app checks the latest release's
# appcast.xml for updates, the site's download button links to the latest release's
# t3agent37.dmg, and install.sh downloads the command from the latest release.
#
# Releases are made on GitHub's runners (.github/workflows/release.yml): push a tag v<version>
# and approve the run. That workflow runs this script with the keys from its secrets:
#   NOTARY_KEY, NOTARY_KEY_ID, NOTARY_ISSUER   an App Store Connect API key (.p8) for notarytool
#   SPARKLE_KEY_FILE                           the update key, as generate_keys -x writes it
# and the Developer ID in a keychain of its own.
#
# On a Mac instead (the fallback), once, before the first release:
#   1. Notary credentials in your keychain, with an app-specific password from
#      account.apple.com > Sign-In and Security:
#        xcrun notarytool store-credentials <profile> --apple-id <your Apple ID> --team-id <team ID>
#      Release with NOTARY_PROFILE=<profile> unless it's named F2APP (the maintainer's).
#   2. The update key in your keychain: target/sparkle-2.10.0/bin/generate_keys --account t3agent37
#      -f <the exported key file>. (generate_keys made it there, with no -f, the first time.)
#   3. Rust's Intel target, in this folder: rustup target add x86_64-apple-darwin
#   4. For --publish: gh auth login
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
# The public half of t3agent37's update key, in every copy ever shipped: each release must be
# signed with its private half, or installed copies refuse the update.
UPDATE_KEY=hwfYLfIGhKFyKFgRkDU8xkQ+UeiT7H03GbqKXTbxm0Y=
PUBLISH=""
[ "${1:-}" = "--publish" ] && PUBLISH=1

# The public half of a private key file from generate_keys -x (the Ed25519 seed, in base64).
public_key_of() {
    OPENSSL=$(brew --prefix openssl@3 2>/dev/null || true)/bin/openssl
    [ -x "$OPENSSL" ] || OPENSSL=openssl
    # (The seed as PKCS#8, which OpenSSL reads: a fixed 16-byte header, then the seed.)
    { printf '\060\056\002\001\000\060\005\006\003\053\145\160\004\042\004\040'; base64 -d < "$1"; } |
        "$OPENSSL" pkey -inform DER -pubout -outform DER | tail -c 32 | base64
}

if [ -n "${NOTARY_KEY:-}" ]; then
    NOTARY="--key $NOTARY_KEY --key-id ${NOTARY_KEY_ID:?} --issuer ${NOTARY_ISSUER:?}"
else
    NOTARY="--keychain-profile $NOTARY_PROFILE"
fi
if [ -n "${SPARKLE_KEY_FILE:-}" ]; then
    SIGNING_KEY=$(public_key_of "$SPARKLE_KEY_FILE" 2>/dev/null || true)
    APPCAST_KEY="--ed-key-file $SPARKLE_KEY_FILE"
else
    SIGNING_KEY=$("$SPARKLE/bin/generate_keys" --account t3agent37 -p 2>/dev/null | grep -E '^[A-Za-z0-9+/]{43}=$' || true)
    APPCAST_KEY="--account t3agent37"
fi

security find-identity -v -p codesigning | grep -q "Developer ID Application" || { echo "no Developer ID Application certificate in the keychain"; exit 1; }
# shellcheck disable=SC2086
xcrun notarytool history $NOTARY >/dev/null 2>&1 || { echo "notarytool can't sign in: see the notary credentials above"; exit 1; }
if [ -z "${SPARKLE_KEY_FILE:-}" ] && [ ! -x "$SPARKLE/bin/generate_keys" ]; then
    echo "no Sparkle tools yet: run scripts/bundle-macos.sh once"
    exit 1
fi
[ "$SIGNING_KEY" = "$UPDATE_KEY" ] || { echo "the update key here isn't t3agent37's (its public half is ${SIGNING_KEY:-missing}): see step 2 above"; exit 1; }
if [ -n "$PUBLISH" ]; then
    gh auth status >/dev/null 2>&1 || { echo "gh isn't signed in: gh auth login"; exit 1; }
    # (The build number is the commit count, and the tag goes on this commit: it should be the
    # one on GitHub.)
    [ -z "$(git status --porcelain)" ] || { echo "commit first: a release is built from a commit"; exit 1; }
    git fetch -q origin 2>/dev/null || true
    [ -n "$(git branch -r --contains HEAD)" ] || { echo "push first: the release is tagged on this commit"; exit 1; }
    if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && [ "$(git rev-list -n 1 "$TAG")" != "$(git rev-parse HEAD)" ]; then
        echo "the tag $TAG is on another commit"
        exit 1
    fi
    if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
        echo "t3agent37 $VERSION is already out: bump the version in Cargo.toml"
        exit 1
    fi
fi

SPARKLE_PUBLIC_KEY=$UPDATE_KEY RELEASE=1 FEED_URL="$FEED_URL" ./scripts/bundle-macos.sh
APP=target/release/bundle/$NAME.app
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
# shellcheck disable=SC2086
xcrun notarytool submit "$DIST/$NAME.zip" $NOTARY --wait
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
# shellcheck disable=SC2086
xcrun notarytool submit "$DMG" $NOTARY --wait
xcrun stapler staple "$DMG"

# As Gatekeeper sees them, on a Mac that's never seen t3agent37.
spctl --assess --type open --context context:primary-signature --verbose "$DMG"
spctl --assess --type execute --verbose "$APP"

# Sparkle's appcast: the DMG, signed with the update key, where this release will have it.
# shellcheck disable=SC2086
"$SPARKLE/bin/generate_appcast" $APPCAST_KEY \
    --download-url-prefix "https://github.com/$REPO/releases/download/$TAG/" \
    --full-release-notes-url "https://github.com/$REPO/releases/tag/$TAG" \
    -o "$DIST/appcast.xml" "$DIST"
grep -q "sparkle:edSignature" "$DIST/appcast.xml" || { echo "the appcast isn't signed: see step 2 above"; exit 1; }
echo "built $DMG and $DIST/appcast.xml: t3agent37 $VERSION ($BUILD)"

# The command alone (no panel, `--no-default-features`), for Apple silicon and Intel in one
# file, signed and notarized. A bare program can't be stapled: Gatekeeper asks Apple the first
# time it runs one that was downloaded with a quarantine (curl's aren't). In a folder of its own
# and after the appcast, so Sparkle never takes it for an update.
echo "building the command…"
CLI_TARGET=target/cli
CLI_DIST=target/release/dist-cli
case "$(uname -m)" in
    arm64) OTHER=x86_64-apple-darwin ;;
    *) OTHER=aarch64-apple-darwin ;;
esac
CARGO_TARGET_DIR=$CLI_TARGET cargo build --release --locked --no-default-features --bin t3agent37
CARGO_TARGET_DIR=$CLI_TARGET cargo build --release --locked --no-default-features --bin t3agent37 --target "$OTHER"
rm -rf "$CLI_DIST"
mkdir -p "$CLI_DIST" "$CLI_TARGET/universal"
lipo -create "$CLI_TARGET/release/t3agent37" "$CLI_TARGET/$OTHER/release/t3agent37" -output "$CLI_TARGET/universal/t3agent37"
codesign --force --timestamp --options runtime --sign "$IDENTITY" "$CLI_TARGET/universal/t3agent37"
ditto -c -k "$CLI_TARGET/universal/t3agent37" "$CLI_TARGET/t3agent37.zip"
echo "notarizing the command…"
# shellcheck disable=SC2086
xcrun notarytool submit "$CLI_TARGET/t3agent37.zip" $NOTARY --wait
rm "$CLI_TARGET/t3agent37.zip"
[ "$("$CLI_TARGET/universal/t3agent37" --version)" = "t3agent37 $VERSION" ] || { echo "the command doesn't say it's $VERSION"; exit 1; }
tar -czf "$CLI_DIST/t3agent37-macos.tar.gz" -C "$CLI_TARGET/universal" t3agent37
(cd "$CLI_DIST" && shasum -a 256 t3agent37-macos.tar.gz > t3agent37-macos.tar.gz.sha256)
echo "built $CLI_DIST/t3agent37-macos.tar.gz"

if [ -n "$PUBLISH" ]; then
    # (Pushed by hand to start the release on GitHub, or tagged here.)
    if ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
        git tag "$TAG"
        git push -q origin "$TAG"
    fi
    set -- "$DMG" "$DIST/appcast.xml" "$CLI_DIST"/*
    if [ -n "${EXTRA_ASSETS_DIR:-}" ]; then
        set -- "$@" "$EXTRA_ASSETS_DIR"/*
    fi
    gh release create "$TAG" "$@" --repo "$REPO" --verify-tag \
        --title "t3agent37 $VERSION" --generate-notes
    echo "released t3agent37 $VERSION: https://github.com/$REPO/releases/tag/$TAG (updates from $FEED_URL)"
fi
