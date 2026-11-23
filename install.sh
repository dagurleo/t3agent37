#!/bin/sh
# Installs the `t3agent37` command, for macOS and Linux:
#
#   curl -fsSL https://raw.githubusercontent.com/dagurleo/t3agent37/main/install.sh | sh
#
# It downloads the latest release's build for this computer from GitHub, checks it against its
# .sha256, and puts it in ~/.local/bin (T3AGENT37_INSTALL_DIR picks another folder;
# T3AGENT37_VERSION=v0.1.9 another release). Run it again to update.
set -eu

REPO=dagurleo/t3agent37
DIR=${T3AGENT37_INSTALL_DIR:-$HOME/.local/bin}

case "$(uname -s)" in
    Darwin) ASSET=t3agent37-macos.tar.gz ;;
    Linux)
        case "$(uname -m)" in
            x86_64 | amd64) ASSET=t3agent37-linux-x86_64.tar.gz ;;
            aarch64 | arm64) ASSET=t3agent37-linux-arm64.tar.gz ;;
            *) echo "t3agent37 has no build for $(uname -m) Linux" >&2; exit 1 ;;
        esac
        ;;
    *) echo "the t3agent37 command is for macOS and Linux" >&2; exit 1 ;;
esac
if [ -n "${T3AGENT37_VERSION:-}" ]; then
    URL="https://github.com/$REPO/releases/download/$T3AGENT37_VERSION/$ASSET"
else
    URL="https://github.com/$REPO/releases/latest/download/$ASSET"
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
echo "downloading $URL"
curl -fsSL "$URL" -o "$TMP/$ASSET"
curl -fsSL "$URL.sha256" -o "$TMP/$ASSET.sha256"
(
    cd "$TMP"
    if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$ASSET.sha256" >/dev/null; else shasum -a 256 -c "$ASSET.sha256" >/dev/null; fi
) || { echo "the download doesn't match its checksum; nothing was installed" >&2; exit 1; }
tar -xzf "$TMP/$ASSET" -C "$TMP"
mkdir -p "$DIR"
# (Moved into place, so a t3agent37 running from the old file keeps running.)
cp "$TMP/t3agent37" "$DIR/.t3agent37.new"
chmod 755 "$DIR/.t3agent37.new"
mv -f "$DIR/.t3agent37.new" "$DIR/t3agent37"
echo "installed $("$DIR/t3agent37" --version) at $DIR/t3agent37"

case ":$PATH:" in
    *":$DIR:"*) ;;
    *) echo "$DIR isn't on your PATH: add it (export PATH=\"$DIR:\$PATH\" in your shell's profile)" ;;
esac
if [ -d /Applications/t3agent37.app ]; then
    echo "The t3agent37 app is installed: the command works with it while it runs, so you don't need t3agent37 serve."
else
    echo "Next: t3agent37 setup, then t3agent37 serve --install (starts it at login), then t3agent37 create <name>."
fi
