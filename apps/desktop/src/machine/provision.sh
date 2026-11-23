#!/usr/bin/env bash
# Runs on an Agent37 VM (agent37-ubuntu) as root. Idempotent; also used to upgrade a machine.
#   T3A37_SECRET=<watchdog-secret> provision.sh <t3-version> <sleep-after-secs> <heartbeat-url> <mint-upstream-token:0|1> <name> [<app-version>]
# Expects watchdog.mjs, issues.mjs, start.mjs and activity.mjs next to it. Prints one JSON line:
# {"environmentId", "t3Version" (what runs: the one asked for, or a newer one found), "upstreamToken"?},
# or {"environmentId", "skipped": <version>} when a newer t3agent37 provisioned it (below).
#
# Only $HOME (/root) is the machine's billed, persistent disk, so everything lives there.
#
# The VM has no init system to hand services to: PID 1 is Agent37's own, and the image's
# entrypoint starts only dockerd and its desktop. Docker's restart policy is the one thing that
# runs on every boot, so a container holding nothing but the image's static busybox enters the
# VM's own namespaces and runs boot.sh, which keeps the T3 server and the watchdog running.
# A sleep keeps them running as they were; only a cold boot (start, resize, a move) reruns it.
set -euo pipefail

version="$1" sleep_after="$2" heartbeat="$3" mint_upstream="$4" name="$5" app_version="${6:-}"
secret="$T3A37_SECRET"
here="$(cd "$(dirname "$0")" && pwd)"
export HOME=/root
bin="$HOME/.local/bin"
t3="$bin/t3"
dir="$HOME/.t3agent37"
mkdir -p "$dir" "$bin" "$HOME/.t3/userdata"
chmod 700 "$dir"
export PATH="$bin:$PATH"

# t3agent37 on several computers can share a machine. One with an older app leaves the scripts a
# newer one put here alone: its own would take that one's features away. Versions as x.y.z; one
# that doesn't read that way never holds anything back.
if [ -n "$app_version" ] && [ -f "$dir/app-version" ] && ! OURS="$app_version" node -e '
  const p = (v) => String(v).trim().split(/[-+]/)[0].split(".").map((n) => Number.parseInt(n, 10));
  const ours = p(process.env.OURS), theirs = p(require("fs").readFileSync(process.argv[1], "utf8"));
  if (ours.length !== 3 || theirs.length !== 3 || [...ours, ...theirs].some(Number.isNaN)) process.exit(0);
  for (let i = 0; i < 3; i++) if (ours[i] !== theirs[i]) process.exit(ours[i] > theirs[i] ? 0 : 1);
' "$dir/app-version"; then
  NEWER="$(cat "$dir/app-version")" node -e '
    let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
      let id = null; try { id = JSON.parse(s).environmentId; } catch {}
      console.log(JSON.stringify({ environmentId: id, skipped: process.env.NEWER.trim() }));
    });
  ' < <(curl -sf -m 5 http://127.0.0.1:3773/.well-known/t3/environment || true)
  exit 0
fi

# T3 server pinned to the desktop app's version. It is restarted only when something it runs
# on changed: a restart drops every T3 Code connection.
#
# T3's service launcher runs it, as `t3 up` has systemd run it elsewhere, so that T3 Code's own
# update ("Update N machines?") updates the machine as well. The launcher starts the version that
# service-state.json selects, with service.env's settings. An update swaps the server in place and
# selects the new version, or rolls back if the new one doesn't start: one under way finishes
# before anything here touches T3.
state="$HOME/.t3/runtime/service-state.json"
for _ in $(seq 1 360); do
  grep -qsE '"status": *"pending"' "$state" || break
  sleep 0.5
done
restart_t3=0
# Never back, though: T3 Code's update may have taken the machine past the version asked for, and
# T3's database doesn't go back with it (T3 refuses remote downgrades too).
installed="$("$t3" --version 2>/dev/null | sed -n 's/^t3 v//p' || true)"
if [ -n "$installed" ] && node -e '
  const parse = (v) => { const [core, pre] = v.split("+")[0].split(/-(.*)/s);
    return { core: core.split(".").map(Number), pre: pre ? pre.split(".") : [] }; };
  const cmp = (x, y) => { const a = parse(x), b = parse(y);
    for (let i = 0; i < 3; i++) if (a.core[i] !== b.core[i]) return a.core[i] < b.core[i] ? -1 : 1;
    if (!a.pre.length || !b.pre.length) return b.pre.length - a.pre.length;
    for (let i = 0; i < Math.max(a.pre.length, b.pre.length); i++) {
      const p = a.pre[i], q = b.pre[i];
      if (p === q) continue;
      if (p === undefined || q === undefined) return p === undefined ? -1 : 1;
      const pn = /^\d+$/.test(p), qn = /^\d+$/.test(q);
      if (pn && qn) return Number(p) < Number(q) ? -1 : 1;
      if (pn !== qn) return pn ? -1 : 1;
      return p < q ? -1 : 1;
    }
    return 0; };
  process.exit(cmp(process.argv[1], process.argv[2]) > 0 ? 0 : 1);
' "$installed" "$version"; then
  version="$installed"
fi
if [ "$installed" != "$version" ]; then
  if ! curl -fsSL https://t3.codes/install.sh | T3CODE_VERSION="$version" sh >"$dir/install.log" 2>&1; then
    echo "T3 install failed" >&2
    tail -n 40 "$dir/install.log" >&2
    exit 1
  fi
  restart_t3=1
fi
changed() { # file content: writes content to file when it differs, and prints 1 if it did
  if [ "$(cat "$1" 2>/dev/null || true)" = "$2" ]; then echo 0; else printf '%s\n' "$2" >"$1"; echo 1; fi
}
# boot.sh reads the mode. Releases without the launcher (it is in 0.1.10 and later), or whose
# launcher didn't start the server here (below), run as plain `t3 serve`.
mode=serve
if grep -aqF __service-launcher "$HOME/.t3/runtime/versions/$version/t3" 2>/dev/null \
  && [ "$(cat "$dir/t3-launcher-failed" 2>/dev/null || true)" != "$version" ]; then
  mode=launcher
fi
[ "$(changed "$dir/t3-mode" "$mode")" = 0 ] || restart_t3=1
if [ "$mode" = launcher ]; then
  # A state that selects this version stays, with its update's record; any other is replaced.
  mkdir -p "$(dirname "$state")"
  [ "$(V="$version" node -e '
    const fs = require("fs"), f = process.argv[1], v = process.env.V;
    let s; try { s = JSON.parse(fs.readFileSync(f, "utf8")); } catch {}
    if (s?.activeVersion === v) { process.stdout.write("0"); process.exit(0); }
    fs.writeFileSync(f, JSON.stringify({ protocol: 4, activeVersion: v }, null, 2) + "\n", { mode: 0o600 });
    process.stdout.write("1");
  ' "$state")" = 0 ] || restart_t3=1
  # An explicit host wins over T3CODE_LAN, which T3 Connect's sign-in (`t3 up`) writes here too.
  env_file="$HOME/.t3/service.env"
  [ "$(changed "$env_file" "$(
    grep -vsE '^(T3CODE_HOST|T3CODE_PORT|T3CODE_NO_BROWSER)=' "$env_file" || true
    printf 'T3CODE_HOST=0.0.0.0\nT3CODE_PORT=3773\nT3CODE_NO_BROWSER=true'
  )")" = 0 ] || restart_t3=1
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
# Claude Code signs what it commits and opens ("Co-Authored-By: Claude", "Generated with Claude
# Code") unless told not to. Off here, unless the machine's settings already say either way; a
# settings file that doesn't parse is left alone.
mkdir -p "$HOME/.claude"
node -e '
  const fs = require("fs"), f = process.argv[1];
  let s = {};
  try { s = JSON.parse(fs.readFileSync(f, "utf8")); } catch (e) { if (e.code !== "ENOENT") process.exit(0); }
  if ("attribution" in s || "includeCoAuthoredBy" in s) process.exit(0);
  s.attribution = { commit: "", pr: "", sessionUrl: false };
  fs.writeFileSync(f, JSON.stringify(s, null, 2) + "\n");
' "$HOME/.claude/settings.json"
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
# The GitHub inbox, which the watchdog runs; t3agent37 also runs it as a command.
[ "$(install_if_changed "$here/issues.mjs" "$dir/issues.mjs" 600)" = 0 ] || restart_watchdog=1
# The start URL, which the inbox serves; t3agent37 also runs it as a command.
[ "$(install_if_changed "$here/start.mjs" "$dir/start.mjs" 600)" = 0 ] || restart_watchdog=1
# What the machine is doing, for t3agent37's details; the watchdog reports it.
[ "$(install_if_changed "$here/activity.mjs" "$dir/activity.mjs" 600)" = 0 ] || restart_watchdog=1

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
# The mode provision.sh chose: T3's service launcher, which runs `t3 serve` with service.env's
# settings and swaps it for T3 Code's updates, or `t3 serve` directly.
run t3 /bin/bash -c 'if [ "$(cat /root/.t3agent37/t3-mode 2>/dev/null)" = launcher ]; then
  T3CODE_HOME=/root/.t3 exec /root/.local/bin/t3 __service-launcher
fi
exec /root/.local/bin/t3 serve --host 0.0.0.0 --port 3773 --no-browser' &
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
    if curl -sf -m 5 -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment; then return 0; fi
    sleep 0.5
  done
  return 1
}
if ! wait_t3; then
  if [ "$mode" = launcher ] && tail -n 40 "$dir/t3.log" 2>/dev/null | grep -qF '[service-launcher]'; then
    # This release's launcher refused to start T3 here (a newer state format, say): it runs
    # directly instead, as before the launcher, until another release comes.
    echo "T3's service launcher did not start T3; running it directly" >&2
    printf '%s\n' "$version" >"$dir/t3-launcher-failed"
    echo serve >"$dir/t3-mode"
    restart t3
    wait_t3 || true
  fi
  if ! curl -sf -m 5 -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment; then
    echo "T3 server did not start" >&2
    tail -n 40 "$dir/t3.log" >&2 || true
    exit 1
  fi
fi

if [ -z "$watchdog_token" ]; then
  watchdog_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3agent37-watchdog --scope orchestration:read)"
  [ "$(write_config "$watchdog_token")" = 0 ] || restart_watchdog=1
fi
[ "$restart_watchdog" = 0 ] || restart watchdog

# Which t3agent37 put these scripts here (see the top).
[ -z "$app_version" ] || printf '%s' "$app_version" >"$dir/app-version"

upstream_token=""
if [ "$mint_upstream" = 1 ]; then
  upstream_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3agent37)"
fi
environment_id="$(curl -sf -m 5 http://127.0.0.1:3773/.well-known/t3/environment | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).environmentId))')"
ENV_ID="$environment_id" TOK="$upstream_token" V="$version" node -e '
  const o = { environmentId: process.env.ENV_ID, t3Version: process.env.V };
  if (process.env.TOK) o.upstreamToken = process.env.TOK;
  console.log(JSON.stringify(o));
'
