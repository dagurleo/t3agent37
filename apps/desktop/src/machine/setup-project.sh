#!/bin/bash
# Sets up a project t3agent37 just cloned onto this machine, run in the project's folder, so its
# first chat finds it ready. With $1, a file of the project's own T3 setup scripts (those set to
# run on worktree creation), one "name<TAB>command" per line: those run, in order. Without, its
# submodules and dependencies, from the lockfiles it has; a missing package manager is installed
# when that is quick, else the step is left to the agents.
#
# Each step is announced as "t3agent37-step: <what>" for t3agent37's panel. Exits 0 when done, 1
# when a step failed, 3 when there was nothing to set up.
set -u
export PATH="$HOME/.local/bin:$HOME/.bun/bin:$HOME/.cargo/bin:/usr/local/go/bin:$PATH"

did=0
step() { echo "t3agent37-step: $*"; }
has() { command -v "$1" >/dev/null 2>&1; }
# run <what> <command...>: stops the setup when it fails.
run() {
    local what="$1"
    shift
    step "$what"
    if ! "$@"; then
        echo "t3agent37: $what failed" >&2
        exit 1
    fi
    did=1
}
# A frozen install first, so the lockfile stays as committed; a plain one when it's out of date.
install() { # <what> <frozen command> <plain command>
    run "$1" bash -c "$2 || { echo 't3agent37: the lockfile is out of date; installing without it'; $3; }"
}

if [ -n "${1:-}" ] && [ -s "$1" ]; then
    while IFS=$'\t' read -r name command; do
        [ -n "$command" ] && run "$name" bash -lc "$command"
    done <"$1"
    exit 0
fi

[ -f .gitmodules ] && run "git submodules" git submodule update --init --recursive

# JavaScript: by lockfile, its package manager first if the machine has none. The image has
# Node with corepack but no npm; corepack brings pnpm and Yarn at the version package.json asks
# for, and npm is the one that ships with this Node release. All into ~/.local: only /root
# outlasts a cold boot.
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
export npm_config_prefix="$HOME/.local"
need_npm() {
    has npm && return
    run "Installing npm" bash -c 'set -e
        v=$(node -e "fetch(\"https://nodejs.org/dist/index.json\").then(r => r.json()).then(l => console.log(l.find(r => r.version === process.version).npm))")
        url="https://registry.npmjs.org/npm/-/npm-$v.tgz"
        rm -rf "$HOME/.local/lib/npm" && mkdir -p "$HOME/.local/lib/npm" "$HOME/.local/bin"
        curl -fsSL "$url" | tar -xz -C "$HOME/.local/lib/npm" --strip-components=1
        chmod +x "$HOME/.local/lib/npm/bin/npm-cli.js" "$HOME/.local/lib/npm/bin/npx-cli.js"
        ln -sf "$HOME/.local/lib/npm/bin/npm-cli.js" "$HOME/.local/bin/npm"
        ln -sf "$HOME/.local/lib/npm/bin/npx-cli.js" "$HOME/.local/bin/npx"'
}
if [ -f bun.lock ] || [ -f bun.lockb ]; then
    has bun || run "Installing Bun" bash -c 'curl -fsSL https://bun.sh/install | bash'
    install "bun install" "bun install --frozen-lockfile" "bun install"
elif [ -f pnpm-lock.yaml ]; then
    has pnpm || { has corepack && run "Enabling pnpm" corepack enable --install-directory "$HOME/.local/bin" pnpm; }
    has pnpm || run "Installing pnpm" bash -c 'curl -fsSL https://get.pnpm.io/install.sh | env PNPM_HOME="$HOME/.local/share/pnpm" SHELL=bash sh - && ln -sf "$HOME/.local/share/pnpm/pnpm" "$HOME/.local/bin/pnpm"'
    install "pnpm install" "pnpm install --frozen-lockfile" "pnpm install"
elif [ -f yarn.lock ]; then
    has yarn || { has corepack && run "Enabling Yarn" corepack enable --install-directory "$HOME/.local/bin" yarn; }
    has yarn || { need_npm && run "Installing Yarn" npm install -g yarn; }
    install "yarn install" "yarn install --frozen-lockfile" "yarn install"
elif [ -f package-lock.json ]; then
    need_npm
    install "npm ci" "npm ci" "npm install"
elif [ -f package.json ]; then
    need_npm
    run "npm install" npm install
fi

# Python with uv.
if [ -f uv.lock ]; then
    has uv || run "Installing uv" bash -c 'curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR="$HOME/.local/bin" sh'
    install "uv sync" "uv sync --locked" "uv sync"
fi

# Toolchains too big to install unasked: only when the machine has them already.
[ -f Cargo.toml ] && has cargo && run "cargo fetch" cargo fetch
[ -f go.mod ] && has go && run "go mod download" go mod download
[ -f Gemfile.lock ] && has bundle && run "bundle install" bundle install

[ "$did" = 1 ] || exit 3
exit 0
