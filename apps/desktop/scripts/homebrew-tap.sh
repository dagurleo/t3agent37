#!/bin/bash
# Writes t3agent37's cask (the app) and formula (the command alone, for macOS and Linux) for a
# published release into the Homebrew tap, dagurleo/homebrew-tap:
#
#   scripts/homebrew-tap.sh v0.1.10 <a clone of the tap>
#
# With TAP_KEY set (the tap's deploy key, which the Release workflow has), it clones the tap
# itself, then commits and pushes the change. The checksums come from the release's own files.
# The tap's other casks are left alone.
set -euo pipefail

TAG=${1:?usage: homebrew-tap.sh <tag> [<tap folder>]}
VERSION=${TAG#v}
REPO=dagurleo/t3agent37
TAP=dagurleo/homebrew-tap
DOWNLOAD="https://github.com/$REPO/releases/download/$TAG"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ -n "${TAP_KEY:-}" ]; then
    # GitHub's own host key (api.github.com/meta), so nothing else answers for github.com.
    printf '%s\n' "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$TMP/known_hosts"
    (umask 077 && printf '%s\n' "$TAP_KEY" > "$TMP/key")
    export GIT_SSH_COMMAND="ssh -i $TMP/key -o IdentitiesOnly=yes -o UserKnownHostsFile=$TMP/known_hosts -o StrictHostKeyChecking=yes"
    git clone --quiet --depth 1 "git@github.com:$TAP.git" "$TMP/tap"
    DIR="$TMP/tap"
else
    DIR=${2:?usage: homebrew-tap.sh <tag> <tap folder> (or TAP_KEY set)}
fi

sha() { curl -fsSL "$DOWNLOAD/$1.sha256" | awk '{print $1}'; }
MACOS=$(sha t3agent37-macos.tar.gz)
LINUX_X86=$(sha t3agent37-linux-x86_64.tar.gz)
LINUX_ARM=$(sha t3agent37-linux-arm64.tar.gz)
curl -fsSL "$DOWNLOAD/t3agent37.dmg" -o "$TMP/t3agent37.dmg"
DMG=$(shasum -a 256 "$TMP/t3agent37.dmg" | awk '{print $1}')
for s in "$MACOS" "$LINUX_X86" "$LINUX_ARM" "$DMG"; do
    [[ "$s" =~ ^[0-9a-f]{64}$ ]] || { echo "a checksum for $TAG is missing" >&2; exit 1; }
done

# The command has `completions` from 0.1.10 on.
COMPLETIONS=""
if [ "$(printf '%s\n' 0.1.10 "$VERSION" | sort -V | head -1)" = "0.1.10" ]; then
    COMPLETIONS='
    generate_completions_from_executable(bin/"t3agent37", "completions")'
fi

mkdir -p "$DIR/Casks" "$DIR/Formula"
cat > "$DIR/Casks/t3agent37.rb" <<EOF
cask "t3agent37" do
  version "$VERSION"
  sha256 "$DMG"

  url "https://github.com/$REPO/releases/download/v#{version}/t3agent37.dmg"
  name "t3agent37"
  desc "Menu bar app for T3 Code machines on Agent37 Cloud that sleep when idle"
  homepage "https://t3agent37.dagurleo.com/"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: :ventura

  app "t3agent37.app"
  # The app's program is the \`t3agent37\` command too, and updates with the app.
  binary "#{appdir}/t3agent37.app/Contents/MacOS/t3agent37"

  zap trash: [
    "~/Library/Application Support/dev.t3agent37.app",
    "~/Library/Caches/dev.t3agent37.app",
    "~/Library/HTTPStorages/dev.t3agent37.app",
    "~/Library/LaunchAgents/dev.t3agent37.serve.plist",
    "~/Library/Logs/t3agent37.log",
    "~/Library/Preferences/dev.t3agent37.app.plist",
  ]
end
EOF

cat > "$DIR/Formula/t3agent37.rb" <<EOF
class T3agent37 < Formula
  desc "Command for T3 Code machines on Agent37 Cloud that sleep when idle"
  homepage "https://t3agent37.dagurleo.com/"
  license "MIT"

  livecheck do
    url :stable
    strategy :github_latest
  end

  # One build for both chips on macOS.
  on_macos do
    on_arm do
      url "$DOWNLOAD/t3agent37-macos.tar.gz"
      sha256 "$MACOS"
    end
    on_intel do
      url "$DOWNLOAD/t3agent37-macos.tar.gz"
      sha256 "$MACOS"
    end
  end

  on_linux do
    on_intel do
      url "$DOWNLOAD/t3agent37-linux-x86_64.tar.gz"
      sha256 "$LINUX_X86"
    end
    on_arm do
      url "$DOWNLOAD/t3agent37-linux-arm64.tar.gz"
      sha256 "$LINUX_ARM"
    end
  end

  def install
    bin.install "t3agent37"$COMPLETIONS
  end

  def caveats
    <<~TEXT
      The command works with the t3agent37 app while the app runs. Without the app:
        t3agent37 setup
        brew services start t3agent37
      Install this or the app (brew install --cask t3agent37, which has the command too), not both.
    TEXT
  end

  service do
    run [opt_bin/"t3agent37", "serve"]
    environment_variables PATH: std_service_path_env
    keep_alive crashed: true
    log_path var/"log/t3agent37.log"
    error_log_path var/"log/t3agent37.log"
  end

  test do
    assert_match "t3agent37 #{version}", shell_output("#{bin}/t3agent37 --version")
  end
end
EOF
echo "wrote t3agent37 $VERSION into $DIR (Casks/t3agent37.rb, Formula/t3agent37.rb)"

if [ -n "${TAP_KEY:-}" ]; then
    cd "$DIR"
    if git diff --quiet -- Casks Formula && [ -z "$(git ls-files --others --exclude-standard -- Casks Formula)" ]; then
        echo "the tap has t3agent37 $VERSION already"
        exit 0
    fi
    git add Casks/t3agent37.rb Formula/t3agent37.rb
    git -c user.name="t3agent37 release" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
        commit --quiet -m "t3agent37 $VERSION"
    git push --quiet origin HEAD
    echo "pushed t3agent37 $VERSION to $TAP"
fi
