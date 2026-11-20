#!/usr/bin/env bash
# Runs on an Agent37 VM (agent37-ubuntu) as root. Idempotent; also used to upgrade a machine.
#   T3A37_SECRET=<watchdog-secret> provision.sh <t3-version> <sleep-after-secs> <heartbeat-url> <mint-upstream-token:0|1> <name>
# Expects watchdog.mjs next to it. Prints one JSON line: {"environmentId", "upstreamToken"?}.
#
# Only $HOME (/root) is the machine's billed, persistent disk, so everything lives there.
#
# The VM has no init system to hand services to: PID 1 is Agent37's own, and the image's
# entrypoint starts only dockerd and its desktop. Docker's restart policy is the one thing that
# runs on every boot, so a container holding nothing but the image's static busybox enters the
# VM's own namespaces and runs boot.sh, which keeps the T3 server and the watchdog running.
# A sleep keeps them running as they were; only a cold boot (start, resize, a move) reruns it.
set -euo pipefail

version="$1" sleep_after="$2" heartbeat="$3" mint_upstream="$4" name="$5"
secret="$T3A37_SECRET"
here="$(cd "$(dirname "$0")" && pwd)"
export HOME=/root
bin="$HOME/.local/bin"
t3="$bin/t3"
dir="$HOME/.t3agent37"
mkdir -p "$dir" "$bin" "$HOME/.t3/userdata"
chmod 700 "$dir"
export PATH="$bin:$PATH"

# T3 server pinned to the desktop app's version. It is restarted only when something it runs
# on changed: a restart drops every T3 Code connection.
restart_t3=0
if [ "$("$t3" --version 2>/dev/null || true)" != "t3 v$version" ]; then
  if ! curl -fsSL https://t3.codes/install.sh | T3CODE_VERSION="$version" sh >"$dir/install.log" 2>&1; then
    echo "T3 install failed" >&2
    tail -n 40 "$dir/install.log" >&2
    exit 1
  fi
  restart_t3=1
fi
settings="$HOME/.t3/userdata/settings.json"
before="$(cat "$settings" 2>/dev/null || true)"
node -e '
  const fs = require("fs"), f = process.argv[1];
  const s = fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, "utf8")) : {};
  s.linkedEnvironmentsEnabled = true;
  fs.writeFileSync(f, JSON.stringify(s, null, 2));
' "$settings"
[ "$before" = "$(cat "$settings")" ] || restart_t3=1
# The machine's name, as other devices show it (T3 Connect, linked environments). T3 reads it
# from /etc/machine-info as it starts; that file is on the image's layer, so boot.sh writes it
# again on every boot from the copy kept here.
if [ "$(cat "$dir/name" 2>/dev/null || true)" != "$name" ]; then
  printf '%s' "$name" >"$dir/name"
  restart_t3=1
fi
echo "PRETTY_HOSTNAME=$name" >/etc/machine-info

# Root has no profile on the image: a shell of the user's own (SSH, VS Code) finds what is
# installed below there too.
grep -qs 't3agent37: PATH' "$HOME/.profile" || cat >>"$HOME/.profile" <<'PROFILE'
# t3agent37: PATH
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) PATH="$HOME/.local/bin:$PATH" ;; esac
[ -f "$HOME/.bashrc" ] && [ -n "$BASH_VERSION" ] && . "$HOME/.bashrc"
PROFILE

# The agents T3 runs, and gh for GitHub. None come with the image; each installs into $HOME
# once, and Claude Code keeps itself up to date from there.
if [ ! -x "$bin/claude" ]; then
  curl -fsSL https://claude.ai/install.sh | bash >"$dir/claude-install.log" 2>&1 || echo "Claude Code install failed" >&2
fi
if [ ! -x "$bin/codex" ]; then
  tmp="$(mktemp -d)"
  if curl -fsSL -o "$tmp/codex.tar.gz" https://github.com/openai/codex/releases/latest/download/codex-x86_64-unknown-linux-musl.tar.gz \
    && tar -xzf "$tmp/codex.tar.gz" -C "$tmp" \
    && f="$(find "$tmp" -type f -name 'codex*' ! -name '*.tar.gz' | head -n 1)" && [ -n "$f" ]; then
    install -m 755 "$f" "$bin/codex"
  else
    echo "Codex install failed" >&2
  fi
  rm -rf "$tmp"
fi
if [ ! -x "$bin/gh" ]; then
  tmp="$(mktemp -d)"
  gh_url="$(curl -fsSL https://api.github.com/repos/cli/cli/releases/latest | node -e '
    let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
      const a = JSON.parse(s).assets.find((a) => /_linux_amd64\.tar\.gz$/.test(a.name));
      if (a) process.stdout.write(a.browser_download_url);
    });' || true)"
  if [ -n "$gh_url" ] && curl -fsSL -o "$tmp/gh.tar.gz" "$gh_url" && tar -xzf "$tmp/gh.tar.gz" -C "$tmp" \
    && f="$(find "$tmp" -type f -path '*/bin/gh' | head -n 1)" && [ -n "$f" ]; then
    install -m 755 "$f" "$bin/gh"
  else
    echo "gh install failed" >&2
  fi
  rm -rf "$tmp"
fi
# Git asks gh for GitHub credentials once gh is signed in (t3agent37 does that per machine). The
# empty helper first drops any inherited one, as `gh auth setup-git` does.
for host in github.com gist.github.com; do
  git config --global --unset-all "credential.https://$host.helper" || true
  git config --global --add "credential.https://$host.helper" ""
  git config --global --add "credential.https://$host.helper" "!$bin/gh auth git-credential"
done

# Watchdog: its own read-only T3 token, kept across upgrades. Minted once T3 runs (below).
config="$dir/config.json"
watchdog_token="$(node -e 'try{process.stdout.write(require(process.argv[1]).t3Token||"")}catch{}' "$config")"

install_if_changed() { # src dest mode: prints 1 when it changed dest
  if cmp -s "$1" "$2"; then echo 0; else install -m "$3" "$1" "$2"; echo 1; fi
}
restart_watchdog="$(install_if_changed "$here/watchdog.mjs" "$dir/watchdog.mjs" 600)"

cat >"$dir/boot.sh.new" <<'BOOT'
#!/bin/bash
# t3agent37's boot hook. The t3agent37-boot container runs it on every boot of this VM, in the
# VM's own namespaces: it keeps the T3 server and the watchdog running, each restarted when it
# exits. provision.sh restarts one by killing its process group (the pid file).
# Everything runs as root, the image's only user. Claude Code refuses full access
# (--dangerously-skip-permissions) as root unless told it is in a sandbox, which this
# single-purpose VM is.
export HOME=/root LANG=C.UTF-8 IS_SANDBOX=1
export PATH=/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
dir=/root/.t3agent37
# One at a time: a restarted container waits for the one it replaces to be gone.
exec 9>"$dir/boot.lock"
flock 9
[ -f "$dir/name" ] && echo "PRETTY_HOSTNAME=$(cat "$dir/name")" >/etc/machine-info
run() {
  local name="$1"
  shift
  while true; do
    log="$dir/$name.log"
    [ "$(stat -c %s "$log" 2>/dev/null || echo 0)" -lt 20000000 ] || mv -f "$log" "$log.1"
    setsid "$@" >>"$log" 2>&1 </dev/null &
    echo $! >"$dir/$name.pid"
    wait $!
    echo "$(date -u +%FT%TZ) $name exited ($?)" >>"$log"
    sleep 2
  done
}
cd /root
run t3 /root/.local/bin/t3 serve --host 0.0.0.0 --port 3773 --no-browser &
run watchdog node --no-warnings "$dir/watchdog.mjs" &
wait
BOOT
restart_boot="$(install_if_changed "$dir/boot.sh.new" "$dir/boot.sh" 700)"
rm -f "$dir/boot.sh.new"

# The watchdog's config must exist before boot.sh first starts it; the token follows below.
write_config() {
  SECRET="$secret" TOKEN="$1" SLEEP="$sleep_after" HEARTBEAT="$heartbeat" node -e '
    const e = process.env, f = process.argv[1], fs = require("fs");
    const next = JSON.stringify({
      secret: e.SECRET, t3Token: e.TOKEN, sleepAfterSecs: Number(e.SLEEP), heartbeatUrl: e.HEARTBEAT
    }, null, 2);
    const prev = fs.existsSync(f) ? fs.readFileSync(f, "utf8") : "";
    if (prev !== next) { fs.writeFileSync(f, next, { mode: 0o600 }); process.stdout.write("1"); } else process.stdout.write("0");
  ' "$config"
}
[ "$(write_config "$watchdog_token")" = 0 ] || restart_watchdog=1

# The boot container: Docker's image store is on $HOME, so it outlives cold boots.
if ! docker image inspect t3agent37/boot >/dev/null 2>&1; then
  busybox=/.agent37/bin/busybox
  [ -x "$busybox" ] || { echo "no static busybox on this image to start T3 at boot" >&2; exit 1; }
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/bin"
  cp "$busybox" "$tmp/bin/busybox"
  tar -C "$tmp" -c . | docker import - t3agent37/boot >/dev/null
  rm -rf "$tmp"
fi
start_boot_container() {
  docker rm -f t3agent37-boot >/dev/null 2>&1 || true
  docker run -d --name t3agent37-boot --restart always --privileged --pid host --network host --ipc host --uts host \
    t3agent37/boot /bin/busybox nsenter -t 1 -m -u -i -n -p -- /bin/bash /root/.t3agent37/boot.sh >/dev/null
}
running="$(docker inspect -f '{{.State.Running}}' t3agent37-boot 2>/dev/null || echo false)"
if [ "$running" != true ] || [ "$restart_boot" = 1 ]; then
  # A new boot script restarts both services with it.
  start_boot_container
  restart_t3=0
  restart_watchdog=0
fi

restart() { # name: kills its process group and waits for boot.sh to start a new one
  local pid
  pid="$(cat "$dir/$1.pid" 2>/dev/null || true)"
  [ -n "$pid" ] || return 0
  kill -TERM -- "-$pid" 2>/dev/null || true
  # T3 shuts down gracefully, which takes a few seconds; until then it still answers.
  for _ in $(seq 1 120); do
    [ "$(cat "$dir/$1.pid" 2>/dev/null || true)" != "$pid" ] && return 0
    sleep 0.5
  done
  kill -KILL -- "-$pid" 2>/dev/null || true
}
[ "$restart_t3" = 0 ] || restart t3

wait_t3() {
  for _ in $(seq 1 240); do
    if curl -sf -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment; then return 0; fi
    sleep 0.5
  done
  echo "T3 server did not start" >&2
  tail -n 40 "$dir/t3.log" >&2 || true
  return 1
}
wait_t3

if [ -z "$watchdog_token" ]; then
  watchdog_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3agent37-watchdog --scope orchestration:read)"
  [ "$(write_config "$watchdog_token")" = 0 ] || restart_watchdog=1
fi
[ "$restart_watchdog" = 0 ] || restart watchdog

upstream_token=""
if [ "$mint_upstream" = 1 ]; then
  upstream_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3agent37)"
fi
environment_id="$(curl -sf http://127.0.0.1:3773/.well-known/t3/environment | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).environmentId))')"
ENV_ID="$environment_id" TOK="$upstream_token" node -e '
  const o = { environmentId: process.env.ENV_ID };
  if (process.env.TOK) o.upstreamToken = process.env.TOK;
  console.log(JSON.stringify(o));
'
