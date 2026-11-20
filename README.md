# t3agent37

t3agent37 is a menu-bar app that creates cloud machines on your [Agent37 Cloud](https://www.agent37.com) account and adds them to [T3 Code](https://t3.codes) as remote environments. They sleep when idle, billing disk alone, so you pay compute only for the minutes your agents run.

Chat in T3 Code as usual. A message to a sleeping machine wakes it in a few seconds; running work finishes with your laptop closed.

## Where it comes from, and why

t3agent37 is a fork of [t3boat](https://github.com/paukraft/t3boat) by Pau Kraft (MIT). t3boat does the same job on [Boat](https://boat.dev): it gives T3 Code remote machines that sleep when nobody is working. It was forked in November 2026 to run on Agent37 Cloud instead of Boat, because the VMs were already there.

What was kept and what changed:

- **Kept from t3boat:** the localhost proxy (`apps/desktop/src/proxy`), changed by two lines in `upstream.rs` to send the Agent37 key. It keeps T3 Code working while a machine sleeps, so most of the hard T3 protocol work is Pau Kraft's. `apps/desktop/docs/t3-protocol.md` is t3boat's spec of that protocol.
- **New:** everything that talks to the cloud. `agent37.rs` replaces `boat.rs`. It covers how a machine boots, how it decides to sleep and how costs are read, because Agent37 VMs differ from Boat sandboxes in ways that mattered (see [Agent37 behaviors](#agent37-behaviors-verified-live)).
- **Added on top:**
  - Restarting T3 after the T3 Connect sign-in.
  - A wake link, so a phone can wake a machine.
  - Claude Code's full-access mode working as root.
  - A new look: "field agents" on a rooftop instead of boats.
  - A native panel drawn with [GPUI](https://gpui.rs) in place of t3boat's Tauri webview, so there's no JavaScript in the app (see [The panel](#the-panel)).

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
- macOS 13 or later. (The panel is GPUI, which runs on Windows and Linux too, but the menu-bar item and the panel's placement are macOS code for now.)

## Install

There is no prebuilt download. Build from source with [Rust](https://rustup.rs) (rustup fetches the 1.99 toolchain the app pins) and Xcode's command line tools.

```sh
cd t3agent37/apps/desktop
scripts/bundle-macos.sh --open
```

The app lands in `apps/desktop/target/release/bundle/t3agent37.app`, signed ad hoc for this Mac, and `--open` starts it. It lives in the menu bar and can run next to t3boat.

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

The full internals are in [apps/desktop/docs/architecture.md](apps/desktop/docs/architecture.md).

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
- The GPUI panel against the real account: the list, the ⋯ menu, details, the project picker, the new machine form (not launched), settings, toasts, dark mode, and a wake and a sleep.

**Not tried yet:**
- Resize, delete, create, sign-ins, T3 Connect and the wake link from the GPUI panel. (They worked from the Tauri panel; the GPUI one calls the same backend functions.)
- The GPUI panel's setup view (first run, no key yet).
- How long T3 Connect takes to come back after a machine has slept for many hours.

**Loose ends:**
- **Where the code lives:** `origin` is [dagurleo/t3agent37](https://github.com/dagurleo/t3agent37) (private), `main`; the local branch is `t3agent37`, tracking it. `upstream` is t3boat. `8992f61` is the last commit with the Tauri panel. The T3 Code scratch folder above this repo has the same `origin` but its own old history (one commit, a gitlink to this folder): don't pull or push there.
- **The site (`apps/web`) has no domain yet.** `SITE` in `apps/web/src/links.ts` is a placeholder (`https://t3agent37.example`); the canonical and share-card URLs, `llms.txt` and the launch video's end card all read it. Set it there once the domain is decided. The site links to `dagurleo/t3agent37`, which is private for now, so its GitHub links and the install prompt only work for people with access.
- **The site's launch video (`launch.html`, `scripts/launch.ts`) is restyled but not filmed yet.** Its page renders with the rooftop; `bun run launch` (dev server running) films it.
- **GitHub sign-in copies its device code to the clipboard.** Copying anything else afterwards loses it. Showing the code in the panel would be better.
- **T3 Connect's "require approval for new devices" option** is not turned on by the app.
- **The panel has no scrollbars.** Its views scroll, but no bar shows.

### The panel

t3boat's panel was a React page in a Tauri webview. t3agent37 drew it natively instead, to get the webview and JavaScript out of the app.
- [Rune](https://github.com/anomalyco/rune) was looked at first and set aside for now: it had no macOS menu-bar item and was weeks old.
- [GPUI](https://gpui.rs), through [gpui-kit](https://gpui-kit.com) 0.10, was chosen. The menu-bar item and the panel's placement follow [F2](https://github.com/getf2/f2), a GPUI app for macOS.
- The React panel was ported view for view, and the rooftop's pixel engine matched the TypeScript one byte for byte (86 pictures). Then Tauri and the React panel were removed on 2026-11-20. Both are in `8992f61`.
- The panel calls the backend directly; there's no JSON contract in between. The window is hidden, not closed, when the panel goes away, so a sign-in in the browser or a half-typed name is still there on the next click.

## Notes for future sessions

Read this section before changing anything. Every fact under "verified live" cost a debugging round to learn.

### Ground rules

- **The Agent37 API key never goes onto a VM and is never printed.** A workspace key can create and delete machines and spend the wallet. It lives in the app's data folder only.
- **Never print a wake link, a T3 session token or other secret in full.** Redact `eyJ…` tokens in command output. Wake links are capability URLs.
- **Only touch machines t3agent37 created.** The Agent37 workspace can hold other VMs that aren't this app's.
- **Don't commit or push unless asked.**
- **Remember that any request to a sleeping VM's public URL wakes it** (and bills it). Nothing in the app should talk to a sleeping machine unless it means to wake it.
- **Run one t3agent37 at a time.** A second copy would share the data folder, ports and machines, so it refuses to start while port 47370 is taken.
- **Don't rebuild the binary a running copy runs from.** macOS kills a process whose signed binary changes underneath it (no crash report). Run the bundle, or a copy of `target/debug/t3agent37`.

### Build, install, test (macOS)

```sh
cd apps/desktop
scripts/bundle-macos.sh --open      # release build → target/release/bundle/t3agent37.app; quits the running one, opens it
cargo run -- --open                 # a debug build, panel open at launch (quit the bundled one first)
cargo test                          # backend and panel tests
cargo run --bin t3agent37-dev       # the backend alone, driven from stdin (see the file's header)
```

The app runs straight from the bundle path. Nothing is copied to `/Applications`.

- **Toolchain:** Rust 1.99, picked by `apps/desktop/rust-toolchain.toml`, because gpui-pre 0.3.12 uses std APIs newer than 1.96. A first build compiles GPUI and takes several minutes.
- **Environment:** `T3AGENT37_DATA_DIR` points at another data folder. `T3AGENT37_KEEP_PANEL_OPEN=1` keeps the panel up when you click elsewhere (for recordings). `T3A_DEBUG=1` logs where the panel was placed. `RUST_LOG` sets log levels.
- **Outside a bundle** (`cargo run`), notifications go through `osascript` (shown as Script Editor's) and Start at login refuses: both need the bundle.
- **App data:** `~/Library/Application Support/dev.t3agent37.app/`. It holds `agent37-key`, `state.json`, `t3-token` (the app's read-only session on the desktop T3 server) and `machines/` (captures per machine).
- **On a VM:** everything lives in `/root/.t3agent37/`, including `boot.sh`, the watchdog, `t3.pid` and the machine's name.
- **Ports on the VM:** T3 `3773`, watchdog `3774`, heartbeat `3775` (public, answers 204), wake link `3776` (public, static page).
- **Ports on this computer:** MCP server at `http://127.0.0.1:47370/mcp`, machine proxies from `47371` up.

### Repo map

```
apps/desktop/                 the app: one Rust package (library = the backend, binary = the menu-bar app)
  src/
    agent37.rs                Agent37 API client (instances, exec, public ports, usage, billing)
    app.rs                    app model: setup, machines, ports, costs
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
    bin/t3agent37-dev.rs      the backend headless, for development
    bin/t3agent37/            the menu-bar app (GPUI)
      main.rs                 startup, one instance at a time, the menu-bar item, showing and hiding the panel
      backend.rs              the backend on its own tokio runtime: setup, events, commands
      model.rs                what the panel does with a machine: actions, labels, numbers
      theme.rs                brand tokens and fonts
      macos.rs, macos/shell.m status item, panel placement, notifications, login item (Objective-C, after F2)
      ui/                     the panel: mod.rs (views, toasts, commands), rooftop, list, details, forms, settings, kit
      pixel/                  the rooftop's pixel engine: palette, scene, spy sprites
  assets/fonts/               Inter, Funnel Display, IBM Plex Mono (OFL), embedded
  icons/                      tray.png, tray@2x.png (menu-bar template), app-icon.png, icon.icns
  scripts/                    bundle-macos.sh, make_icons.py
  docs/                       architecture, T3 protocol spec
apps/web/                     the marketing site (React 19 + Vite, prerendered): see "The site" below
  src/panel/                  the React port of the panel, restored from 8992f61, on its mock backend only
```

### The site

`apps/web` is t3agent37's marketing site, `@t3agent37/web` in the bun workspace (the root package is `t3agent37`). It runs on mock data only: no account, key or machine.

```sh
bun install                 # at the repo root
bun run dev                 # at the root or in apps/web: the dev server, http://localhost:5173
cd apps/web && bun run build   # tsc, vite, the SSR build, then scripts/prerender.ts → dist/
bun run preview             # in apps/web: serves dist/
```

- **The panel's code is copied in, not shared.** `src/panel` is the React panel from `8992f61` (components, pixel engine, `mock.ts`), with the Tauri bridge taken out of `api.ts`. The app's real panel is GPUI now; the React port looks the same, so the site's demo (`demo.html`, iframed by `Together.tsx`) stands in for it. If the GPUI panel's look changes, port the change here by hand.
- **One change to the engine:** `pixel/scene.ts` takes a `Frame` besides a zoom, so the hero can put the horizon under its copy, keep the sky plain behind it, thin the stars and keep clouds below the text. With a zoom it draws exactly what the panel drew.
- **The prerender** writes `index.md`, `llms.txt`, `llms-full.txt` and `vercel.json`'s routes (the markdown twin for agents). Commit `vercel.json` when it changes.
- **The share card:** `bun run og` (dev server running, ffmpeg with libx264) redraws `public/og.png` and `og.mp4` from the hero.
- **No analytics.** t3boat's OpenPanel tracking was removed with the fork.

### Adding a panel command

A new command touches all of these. The MCP test counts tools, so it fails until it is updated:

1. The backend: a method on `App` or `MachineRuntime` (`src/app.rs`, `src/machine/`).
2. `src/bin/t3agent37/backend.rs`: a `Cmd` variant and its arm in `act`, or a `Backend` method if it returns data.
3. `src/bin/t3agent37/model.rs`: an `Action` constructor with its label and busy text, if it's a button.
4. The view that shows it (`ui/`), and any toast it needs in `Panel::track` (`ui/mod.rs`).
5. If agents should have it: `src/mcp.rs`, a `Call` variant, a `tool(...)` entry and the tool count in its test.

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
- The engine is in `src/bin/t3agent37/pixel/`: bayer dithering and ramps, redrawn on the panel's 12 fps clock. `scene.rs` draws the roof, `spy.rs` the agents and `paint.rs` the strip and row icons.
- Brand tokens are in `theme.rs`. The accent is `#c2581f` in light mode and `#ffb547` in dark. Body text is Inter, display Funnel Display SemiBold, small print IBM Plex Mono.

**Icons**
- To redraw them, run `python3 apps/desktop/scripts/make_icons.py <out-dir>` (standard library only).
- Copy `tray.png`, `tray@2x.png` and `app-icon.png` into `apps/desktop/icons/`.
- Make `icon.icns` from `app-icon.png` (1024 px), in `apps/desktop/icons`:

```sh
mkdir AppIcon.iconset
for s in 16 32 128 256 512; do
  sips -z $s $s app-icon.png --out AppIcon.iconset/icon_${s}x${s}.png
  sips -z $((s*2)) $((s*2)) app-icon.png --out AppIcon.iconset/icon_${s}x${s}@2x.png
done
iconutil -c icns AppIcon.iconset -o icon.icns && rm -r AppIcon.iconset
```

## License

[MIT](LICENSE), as t3boat. Not affiliated with or endorsed by T3 Code, Agent37 or Boat.
