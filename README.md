# t3agent37

t3agent37 is a menu-bar app that creates cloud machines on your [Agent37 Cloud](https://www.agent37.com) account and adds them to [T3 Code](https://t3.codes) as remote environments. They sleep when idle, billing disk alone, so you pay compute only for the minutes your agents run.

Chat in T3 Code as usual. A message to a sleeping machine wakes it in a few seconds; running work finishes with your laptop closed.

## Where it comes from, and why

t3agent37 is a fork of [t3boat](https://github.com/paukraft/t3boat) by Pau Kraft (MIT). t3boat does the same job on [Boat](https://boat.dev): it gives T3 Code remote machines that sleep when nobody is working. It was forked in November 2026 to run on Agent37 Cloud instead of Boat, because the VMs were already there.

What was kept and what changed:

- **Kept from t3boat:** the localhost proxy (`apps/desktop/src-tauri/src/proxy`), changed by two lines in `upstream.rs` to send the Agent37 key. It keeps T3 Code working while a machine sleeps, so most of the hard T3 protocol work is Pau Kraft's. `apps/desktop/docs/t3-protocol.md` is t3boat's spec of that protocol.
- **New:** everything that talks to the cloud. `agent37.rs` replaces `boat.rs`. It covers how a machine boots, how it decides to sleep and how costs are read, because Agent37 VMs differ from Boat sandboxes in ways that mattered (see [Agent37 behaviors](#agent37-behaviors-verified-live)).
- **Added on top:**
  - Restarting T3 after the T3 Connect sign-in.
  - A wake link, so a phone can wake a machine.
  - Claude Code's full-access mode working as root.
  - A new look: "field agents" on a rooftop instead of boats.

## What you get

- Create, resize and delete machines (Agent37 VMs: Ubuntu with Docker, 2–8 vCPU) from the menu bar
- Add projects the way T3 Code's "Add to another machine" does, `.env` files included
- Sign Claude Code, Codex and GitHub in on a machine with their own logins
- T3 Connect, to reach a machine from your phone or a browser, plus a wake link for when it's asleep
- Cost per machine, today and this month, and your wallet balance
- An MCP server, so agents on your computer can manage machines

## What you need

- [T3 Code](https://t3.codes) desktop app
- An [Agent37 Cloud](https://www.agent37.com) account and an API key from [its dashboard](https://www.agent37.com/dashboard/cloud/api-keys). The key stays on this computer (in the app's data folder, readable by you only) and is never copied to a machine.
- macOS, Windows or Linux. Only macOS has been run so far.

## Install

There is no prebuilt download. Build from source with [Rust](https://rustup.rs) and [Bun](https://bun.sh). Windows also needs the [Microsoft C++ Build Tools](https://v2.tauri.app/start/prerequisites/#windows), Linux [Tauri's system packages](https://v2.tauri.app/start/prerequisites/#linux).

```sh
cd t3agent37
bun install
bun run build:desktop
```

The app lands in `apps/desktop/src-tauri/target/release/bundle/` (`macos/t3agent37.app` on macOS). It lives in the menu bar or system tray, and can run next to t3boat.

## Usage

1. Click the icon in the menu bar, paste your Agent37 API key, and create a machine.
2. Accept the new backend in T3 Code.
3. Sign in Claude or Codex, and GitHub for private repos, from the machine's details.
4. Work in T3 Code as usual. Push or open a PR from the machine.

To use a machine from your phone:

1. In the machine's details, turn on T3 Connect under **other devices**.
2. In the T3 Code phone app, add the machine under **Settings → Environments → T3 Connect**.
3. T3 Connect can't wake a sleeping machine, so click **Copy link** next to "Wake link" and put that link on your phone's home screen.
4. Tap the link, wait for "awake", then pick the machine in T3 Code.

The wake link only wakes the machine and holds it up for 5 minutes. Using the machine still takes your T3 sign-in. **Replace link** retires the old link within about 10 seconds.

## Pricing, roughly

Agent37 meters per minute and everything draws on your prepaid Agent37 wallet. Asleep, a machine bills its 20 GB disk alone, about $1.80 a month. Awake, it bills the VM rate for its size:

| size | VM | per hour awake |
|---|---|---|
| small | 2 vCPU · 4 GB | $0.015 |
| default | 4 vCPU · 8 GB | $0.030 |
| large | 4 vCPU · 16 GB | $0.049 |
| xlarge | 4 vCPU · 32 GB | $0.088 |
| huge | 8 vCPU · 64 GB | $0.175 |

## How it works

```
T3 Code ──ws──> localhost proxy (t3agent37) ──https/wss + key──> Agent37 edge ──> T3 server on an Agent37 VM
                    │                                                                └─ watchdog: decides when it sleeps
                    └─ Agent37 API: create / start / sleep / resize / exec / delete
```

- **Each machine is an Agent37 VM** (`agent37-ubuntu` template). It runs the T3 server at your desktop app's version.
- **T3 Code talks to the machine through a local proxy** at `http://localhost:<port>`, one per machine. The proxy adds the Agent37 key on the way out. While the machine sleeps, the proxy answers T3 Code from what it recorded, so opening a chat doesn't wake the machine. A real request, such as sending a message, wakes it.
- **A watchdog on the VM decides when to sleep:** 5 minutes after the last agent turn and the last use. It works out "idle" itself, so work finishes even with the laptop closed. It holds Agent37's auto-sleep off with a heartbeat until it is idle. Then the app sleeps the VM, or auto-sleep does if the app isn't running.

The full internals are in [apps/desktop/docs/architecture.md](apps/desktop/docs/architecture.md), and the UI ↔ backend contract is in [apps/desktop/docs/ui-contract.md](apps/desktop/docs/ui-contract.md).

## Where things stand (2026-11-20)

**Works, tested live on Agent37:**
- Creating a machine and adding it to T3 Code.
- Signing in Claude Code, Codex and GitHub.
- Running agent threads, including Claude Code in full-access mode.
- Sleeping and waking.
- Re-provisioning a machine when T3's version changes.
- T3 Connect from the phone app.
- The wake link: about 1.3 s from asleep to page, with Connect reachable right after.
- The redesign, in both themes.

**Not tried yet:**
- Resize and delete through the app.
- Windows and Linux.
- How long T3 Connect takes to come back after a machine has slept for many hours.

**Loose ends:**
- **Nothing is committed.** The whole port is uncommitted, partly staged changes on top of t3boat's history (last upstream commit `bd488c8`). `origin` still points at `paukraft/t3boat`. The folder above this repo is a T3 Code scratch repo that records `t3agent37` as a gitlink with no `.gitmodules`.
- **`apps/web` is t3boat's marketing site, not ported.** It no longer builds: it imports `pixel/boat` and `BoatIcon`, which the redesign removed. Either delete it or redo it for t3agent37.
- **Package names are still `t3boat` / `@t3boat/*`** (root and `apps/desktop` `package.json`, the `bun --filter` scripts). The app itself is t3agent37 everywhere else (bundle id `dev.t3agent37.app`).
- **GitHub sign-in copies its device code to the clipboard.** Copying anything else afterwards loses it. Showing the code in the panel would be better.
- **T3 Connect's "require approval for new devices" option** is not turned on by the app.
- **Copy does nothing on Windows and Linux.** The clipboard is `pbcopy` only (`signin.rs`), so "Copy link" and the GitHub code are macOS-only.

**Moving from Tauri to GPUI.** The goal is a native panel with no webview and no JavaScript in the app.
- [Rune](https://github.com/anomalyco/rune) was looked at first and set aside for now: it has no macOS menu-bar item and is weeks old.
- [`apps/desktop/src-gpui`](apps/desktop/src-gpui/README.md) is the app on GPUI (gpui-kit 0.10), with every view of the React panel. Its menu-bar and panel code follows [F2](https://github.com/getf2/f2).
  - It runs the same backend (`src-tauri` built with `--no-default-features`, which leaves out `tray.rs`) on the same data folder.
  - The rooftop is ported to Rust and pixel-identical to the TypeScript engine.
- **Checked live on 2026-11-20**, against the real account, in place of the Tauri app:
  - The machine list, the ⋯ menu, a machine's details, the project picker (T3 Code's projects, filter, arrows), the new machine form (not launched), settings, toasts and dark mode.
  - Esc closes the innermost thing, then goes back, then hides the panel.
  - T3 Code reconnected to skiff through its proxy.
- **Not tried from it yet:**
  - The commands with side effects: wake, sleep, create, delete, resize, sign-ins, T3 Connect, copying the wake link. They call the same backend functions as `tray.rs`.
  - The setup view.
- **Left to do:**
  - A bundle script like F2's. Notifications and Start at login need it; the Tauri app's autostart LaunchAgent would need removing.
  - Scrollbars.
  - Windows and Linux.
  - Then delete `tray.rs`, the React panel, the Tauri dependencies and the JSON UI contract, and move the backend out of `src-tauri`.

## Notes for future sessions

Read this section before changing anything. Every fact under "verified live" cost a debugging round to learn.

### Ground rules

- **The Agent37 API key never goes onto a VM and is never printed.** A workspace key can create and delete machines and spend the wallet. It lives in the app's data folder only.
- **Never print a wake link, a T3 session token or other secret in full.** Redact `eyJ…` tokens in command output. Wake links are capability URLs.
- **Only touch machines t3agent37 created.** The Agent37 workspace can hold other VMs that aren't this app's.
- **Don't commit or push unless asked.**
- **Remember that any request to a sleeping VM's public URL wakes it** (and bills it). Nothing in the app should talk to a sleeping machine unless it means to wake it.
- **Run one t3agent37 at a time, the Tauri app or the GPUI one.** Both use the same data folder, ports and machines. The GPUI app refuses to start while port 47370 is taken.

### Build, install, test (macOS)

```sh
cd apps/desktop
bun run tauri build --bundles app            # faster than build:desktop (skips the dmg)
osascript -e 'tell application id "dev.t3agent37.app" to quit'
open src-tauri/target/release/bundle/macos/t3agent37.app

cd src-tauri && cargo test --lib             # Rust tests
cd .. && bun run build                       # tsc + vite: type-checks the UI
bun run dev                                  # UI alone in a browser, on the mock backend (src/mock.ts)

# the GPUI app (quit the Tauri one first)
cd src-gpui && cargo run -- --open           # Rust 1.99, picked by its rust-toolchain.toml
cargo test                                   # its tests; the README there has the pixel comparison
```

The app runs straight from that bundle path. Nothing is copied to `/Applications`.

- **App data:** `~/Library/Application Support/dev.t3agent37.app/`. It holds `agent37-key`, `state.json`, `t3-token` (the app's read-only session on the desktop T3 server) and `machines/` (captures per machine).
- **On a VM:** everything lives in `/root/.t3agent37/`, including `boot.sh`, the watchdog, `t3.pid` and the machine's name.
- **Ports on the VM:** T3 `3773`, watchdog `3774`, heartbeat `3775` (public, answers 204), wake link `3776` (public, static page).
- **Ports on this computer:** MCP server at `http://127.0.0.1:47370/mcp`, machine proxies from `47371` up.

### Repo map

```
apps/desktop/                 the app (Tauri 2: Rust backend, React + Vite panel)
  src-gpui/                   the same app on GPUI, replacing tray.rs and src/ (own Cargo project, Rust 1.99)
  src-tauri/src/
    agent37.rs                Agent37 API client (instances, exec, public ports, usage, billing)
    app.rs                    app model: setup, machines, ports, costs
    tray.rs                   Tauri commands the panel calls (see docs/ui-contract.md); the `tauri-app` feature
    mcp.rs                    MCP server for agents on this computer
    t3desktop.rs              talks to the desktop T3 Code app and its server
    machine/
      mod.rs                  per-machine state loop: wake, sleep, resize, upgrades, scheduled work
      provision.rs/.sh        what runs on the VM and how it starts on every boot
      watchdog.mjs            on the VM: idle detection, drain, heartbeat, wake-link page
      connect.rs              T3 Connect turn-on, restart, wake link
      signin.rs               Claude / Codex / GitHub logins on the machine
      projects.rs, link.rs, capture.rs, work.rs
    proxy/                    t3boat's sleep-aware T3 proxy (only upstream.rs changed)
  src/                        the panel UI
    pixel/                    pixel-art engine: palette, rooftop scene, spy sprites
    components/               views (Rooftop, MachineRow, MachineDetails, ...)
  scripts/make_icons.py       draws tray.png, tray@2x.png and app-icon.png
  docs/                       architecture, UI contract, T3 protocol spec
apps/web/                     t3boat's site, not ported (see Loose ends)
```

### Adding a panel command

A new command touches all of these. The MCP test counts tools, so it fails until it is updated:

1. `src-tauri/src/tray.rs`: the command, and its entry in `generate_handler!`.
2. `src/types.ts`: its args and result in the commands map.
3. `src/machine.ts`: an `Action` and its `ACTIONS` label, if it's a button.
4. `src/App.tsx` (the `act` switch) and `src/mock.ts` (mock handler).
5. `docs/ui-contract.md`: a row in the commands table.
6. If agents should have it: `src-tauri/src/mcp.rs`, a `Call` variant, a `tool(...)` entry and the tool count in its test.

### Agent37 behaviors verified live

These were checked on `agent37-ubuntu` VMs in November 2026. Several contradict Agent37's docs.

**Exec**
- The exec API is `POST https://sandbox.agent37.app/v1/instances/{id}/exec` with header `X-Agent37-Key`.
- It runs `sh -c`, which is **dash**. For example, `kill -- -PGID` fails there.
- The app wraps every command in `exec bash -lc`, so test by hand with `bash -c` to match.
- There is no timeout field.

**Sleep and wake**
- Sleep keeps memory and takes about 4 s.
- Any request to `https://{id}-{port}.agent37.app` wakes the VM with the same PIDs: about 4 s through the keyed URL, 1.3 s measured through a public port.
- A cold boot (stop/start) takes about 25 s.
- An open WebSocket to a VM that falls asleep isn't closed. It goes silent, and frames sent on it don't wake the VM.
- Auto-sleep's minimum idle time is 300 s, and in practice it fires about 2 minutes late.
- Calls from the VM to its own public port count as traffic. The watchdog's heartbeat relies on this.

**The image**
- There are no boot hooks (despite the docs) and no systemd. PID 1 is `agent37-vm-guest`.
- Only `/root` persists.
- The image's static busybox at `/.agent37/bin/busybox` has `nsenter`. A restart-policy Docker container uses it to run `boot.sh` on every boot (see `provision.sh`).
- No npm, claude, codex or gh are preinstalled. Root has no `~/.profile`, so `~/.local/bin` isn't on exec's `PATH`.

**Public ports**
- `POST /v1/instances/{id}/public-ports` gives a URL with a random 20-character slug and no auth at the edge.
- `DELETE …/public-ports/{port}` kills that URL in about 10 s; after that it answers 401.

**Resize and costs**
- Resize works only on a running VM, which then boots fresh onto its new shape.
- `GET /v1/usage` reports spend by instance and UTC day, settled hourly. `GET /v1/billing` gives the wallet.

### T3 Code behaviors verified live

- **T3 Connect sign-in only stores the link at first.** `t3 up --connect --headless --yes` stores it as "pending server startup", and T3 applies it the next time it starts. `t3 up` expects to restart its own background service, which doesn't exist here because `boot.sh` runs T3. So after the sign-in, `connect.rs` kills T3's process group (from `t3.pid`), lets `boot.sh` restart it, and waits for `/api/connect/link-state` to report linked with an active tunnel.
- **Connect preferences need a linked machine.** `POST /api/connect/preferences` answers 400 "Link this environment to T3 Connect first." until the restart above has happened.
- **The T3 Connect relay can't wake a sleeping machine.** The tunnel is dialed out from the VM. That's why the wake link exists.
- **The phone app lists only environments added on the phone.** Add them under **Settings → Environments → T3 Connect**. A free T3 account allowed 3 machines there.
- **Claude Code refuses `--dangerously-skip-permissions` (T3's full-access mode) as root** unless `IS_SANDBOX=1` is set. Everything on the VM runs as root, so `boot.sh` exports `IS_SANDBOX=1` for the T3 server.

### Design

The panel's theme is "field agents", in the "Noir amber" palette (light and dark).

**The rooftop strip**
- The panel's top strip is a pixel-art city rooftop: a skyline, a searchlight at night, a beacon by day, and a gravel roof.
- Each machine is a spy in a fedora, coat and shades. The spy's pose shows the machine's state:

| state | pose |
|---|---|
| working | paces the roof |
| creating | drops in on a rope |
| error | alarm |
| asleep | zzz |
| missing | ghost |

- New machines get codenames (raven, falcon, kestrel, …).

**Code and tokens**
- The engine is in `src/pixel/`: a 12 fps clock, bayer dithering and ramps. `scene.ts` draws the roof, `spy.ts` the agents and `paint.ts` the strip and row icons.
- Brand tokens are in `src/brand.css`. The accent is `#c2581f` in light mode and `#ffb547` in dark.

**Icons**
- To redraw them, run `python3 apps/desktop/scripts/make_icons.py <out-dir>` (standard library only).
- Copy `tray.png`, `tray@2x.png` and `app-icon.png` into `src-tauri/icons/`.
- Run `bun run tauri icon src-tauri/icons/app-icon.png` from `apps/desktop` to make every other size. Delete the `android/` and `ios/` folders it adds.

## License

[MIT](LICENSE), as t3boat. Not affiliated with or endorsed by T3 Code, Agent37 or Boat.
