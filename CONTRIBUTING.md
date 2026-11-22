# Contributing to t3agent37

Notes for anyone changing t3agent37, people and coding agents alike. Read them before changing anything: every fact under "verified live" cost a debugging round to learn.

Bug reports and pull requests are welcome. For a security problem, see [SECURITY.md](SECURITY.md) instead of opening an issue.

## Ground rules

- **The Agent37 API key never goes onto a VM and is never printed.** A workspace key can create and delete machines and spend the wallet. It lives in the app's data folder only.
- **Never print a wake link, a T3 session token or other secret in full.** Redact `eyJ…` tokens in command output. Wake links are capability URLs.
- **Only touch machines t3agent37 created.** The Agent37 workspace can hold other VMs that aren't this app's.
- **Coding agents: don't commit or push unless asked.**
- **Remember that any request to a sleeping VM's public URL wakes it** (and bills it). Nothing in the app should talk to a sleeping machine unless it means to wake it.
- **Run one t3agent37 at a time.** A second copy would share the data folder, ports and machines, so it refuses to start while port 47370 is taken.
- **Don't rebuild the binary a running copy runs from.** macOS kills a process whose signed binary changes underneath it (no crash report). Run the bundle, or a copy of `target/debug/t3agent37`.

## Build, install, test (macOS)

You need [Rust](https://rustup.rs) (rustup fetches the 1.99 toolchain the app pins) and Xcode's command line tools.

```sh
cd apps/desktop
scripts/bundle-macos.sh --install   # release build → target/release/bundle, copied to /Applications and opened (quits a running copy)
scripts/bundle-macos.sh --open      # the same, opened from target/release/bundle instead
cargo run -- --open                 # a debug build, panel open at launch (quit the bundled one first)
cargo test                          # backend and panel tests
cargo run --bin t3agent37-dev       # the backend alone, driven from stdin (see the file's header)
```

- **Signing:** `bundle-macos.sh` signs with your Developer ID if the keychain has one, else your Apple Development certificate, else ad hoc (fine for your own Mac). `SIGN_IDENTITY` picks one.
- **Updates in your own builds:** on only when Sparkle's key for t3agent37 is in your keychain (the maintainer's builds). Otherwise Settings says "This build doesn't update itself".
- **Toolchain:** Rust 1.99, picked by `apps/desktop/rust-toolchain.toml`, because gpui-pre 0.3.12 uses std APIs newer than 1.96. A first build compiles GPUI and takes several minutes.
- **Environment:** `T3AGENT37_DATA_DIR` points at another data folder. `T3AGENT37_KEEP_PANEL_OPEN=1` keeps the panel up when you click elsewhere (for recordings). `T3A_DEBUG=1` logs where the panel was placed. `RUST_LOG` sets log levels.
- **Outside a bundle** (`cargo run`), notifications go through `osascript` (shown as Script Editor's) and Start at login refuses: both need the bundle.
- **App data:** `~/Library/Application Support/dev.t3agent37.app/`. It holds `agent37-key`, `state.json`, `t3-token` (the app's read-only session on the desktop T3 server) and `machines/` (captures per machine).
- **On a VM:** everything lives in `/root/.t3agent37/`, including `boot.sh`, the watchdog, `t3.pid` and the machine's name.
- **Ports on the VM:** T3 `3773`, watchdog `3774`, heartbeat `3775` (public, answers 204), wake link `3776` (public, static page), GitHub inbox `3777` (public once a repo's issues are on; acts only on signed deliveries).
- **Ports on this computer:** MCP server at `http://127.0.0.1:47370/mcp`, machine proxies from `47371` up.

## Releasing

A release is built for Apple silicon and Intel, signed with a Developer ID, notarized and stapled, in `t3agent37.dmg`, with Sparkle's `appcast.xml` signed by t3agent37's update key, and published as a GitHub release. `apps/desktop/scripts/release-macos.sh --publish` does all of it, on GitHub's runners or on a Mac.

**Each release:** bump `version` in `apps/desktop/Cargo.toml`, commit and push, then push its tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The Release workflow (`.github/workflows/release.yml`) then builds and publishes it in about half an hour, once you approve the run in the Actions tab.

- **Its keys** are secrets of the repo's `release` environment, which only runs for `v*` tags and waits for the maintainer's approval:

  | secret | what |
  |---|---|
  | `MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD` | a Developer ID Application certificate with its private key, exported as `.p12` (in base64) and its password. A certificate of its own for CI, not the one that signs F2, so revoking it if it leaked breaks nothing else. |
  | `NOTARY_KEY_P8`, `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID` | an App Store Connect API key for notarization (Users and Access > Integrations > Team Keys, Developer access): the `.p8` file's text, its key ID and the issuer ID |
  | `SPARKLE_PRIVATE_KEY` | the update key, as `generate_keys --account t3agent37 -x` writes it |

- **The update key** is the one thing that can't be replaced: every installed copy accepts only updates signed with it. Keep a copy in a password manager. `release-macos.sh` checks the key it's given against the public half shipped in the app (`UPDATE_KEY`) before building.
- **On a Mac instead** (the fallback): the one-time setup is in the script's header, then `scripts/release-macos.sh --publish` in `apps/desktop`. It pushes the tag itself, which starts the workflow too: reject that run. Without `--publish` it builds and notarizes into `target/release/dist` only.
- **How updates reach people:** the app (Sparkle 2.10, in `Contents/Frameworks`, loaded by `shell.m`) checks `https://github.com/dagurleo/t3agent37/releases/latest/download/appcast.xml` once a day, and "Check for Updates…" in the menu-bar item's menu or Settings checks now. The site's download button links to `releases/latest/download/t3agent37.dmg`. Both rely on every release having those two files under those names.
- **Build numbers** are commit counts on `main`, so they only go up. Sparkle compares them.
- **The site's download button** points at the latest release, so deploy the site (`bun run deploy` in `apps/web`) only once the first release is out.

## Repo map

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
      issues.rs, issues.mjs   GitHub issues as threads: the webhook, and the inbox on the VM (in the watchdog)
      preview.rs              a machine's ports at localhost here, through T3's preview tunnel
      setup-project.sh        on the VM: a newly added project's setup (its T3 scripts, or its lockfile)
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
      macos.rs, macos/shell.m status item, panel placement, notifications, login item, updates (Objective-C, after F2)
      ui/                     the panel: mod.rs (views, toasts, commands), rooftop, list, details, forms, settings, kit
      pixel/                  the rooftop's pixel engine: palette, scene, spy sprites
  assets/fonts/               Inter, Funnel Display, IBM Plex Mono (OFL), embedded
  icons/                      tray.png, tray@2x.png (menu-bar template), app-icon.png, icon.icns
  scripts/                    bundle-macos.sh (the app), release-macos.sh (signed, notarized DMG and appcast), make_icons.py
  docs/                       architecture, T3 protocol spec
apps/web/                     the site, t3agent37.dagurleo.com (React 19 + Vite, prerendered): see "The site" below
  src/panel/                  the React port of the panel, restored from 41635c4, on its mock backend only
.github/workflows/
  ci.yml                      on each pull request and push to main: the app's tests on a Mac, and the site's build
  release.yml                 on a v* tag: the signed, notarized release (see "Releasing")
```

## The site

`apps/web` is t3agent37's marketing site, `@t3agent37/web` in the bun workspace (the root package is `t3agent37`). It runs on mock data only: no account, key or machine.

```sh
bun install                 # at the repo root
bun run dev                 # at the root or in apps/web: the dev server, http://localhost:5173
cd apps/web && bun run build   # tsc, vite, the SSR build, then scripts/prerender.ts → dist/
bun run preview             # in apps/web: serves dist/
bun run deploy              # in apps/web: builds, then deploys to Cloudflare (wrangler signed in to the account in wrangler.jsonc)
```

- **The panel's code is copied in, not shared.** `src/panel` is the React panel from `41635c4` (components, pixel engine, `mock.ts`), with the Tauri bridge taken out of `api.ts`. The app's real panel is GPUI now; the React port looks the same, so the site's demo (`demo.html`, iframed by `Together.tsx`) stands in for it. If the GPUI panel's look changes, port the change here by hand.
- **One change to the engine:** `pixel/scene.ts` takes a `Frame` besides a zoom, so the hero can put the horizon under its copy, keep the sky plain behind it, thin the stars and keep clouds below the text. With a zoom it draws exactly what the panel drew.
- **Hosting:** a Cloudflare Worker with static assets (`apps/web/wrangler.jsonc`), at its custom domain `t3agent37.dagurleo.com`. `SITE` in `src/links.ts` is the address the canonical and share-card URLs, `llms.txt` and the launch video's end card read. Nothing deploys on push: run `bun run deploy`.
- **The prerender** writes `index.md`, `llms.txt` and `llms-full.txt` (the Markdown twin for agents). `worker.ts` serves `index.md` at `/` to clients that ask for `text/markdown` and to known AI agents, as t3boat's `vercel.json` routes did.
- **The share card:** `bun run og` (dev server running, ffmpeg with libx264) redraws `public/og.png` and `og.mp4` from the hero.
- **No analytics.** t3boat's OpenPanel tracking was removed with the fork.
- **The launch video (`launch.html`, `scripts/launch.ts`) is restyled but not filmed yet.** Its page renders with the rooftop; `bun run launch` (dev server running) films it.

## Adding a panel command

A new command touches all of these. The MCP test counts tools, so it fails until it is updated:

1. The backend: a method on `App` or `MachineRuntime` (`src/app.rs`, `src/machine/`).
2. `src/bin/t3agent37/backend.rs`: a `Cmd` variant and its arm in `act`, or a `Backend` method if it returns data.
3. `src/bin/t3agent37/model.rs`: an `Action` constructor with its label and busy text, if it's a button.
4. The view that shows it (`ui/`), and any toast it needs in `Panel::track` (`ui/mod.rs`).
5. If agents should have it: `src/mcp.rs`, a `Call` variant, a `tool(...)` entry and the tool count in its test.

## Agent37 behaviors verified live

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
- Node 22 is at `/usr/bin/node` with corepack, but without npm. `setup-project.sh` installs the npm that ships with that Node release (from nodejs.org's release index; the newest npm refuses Node 22.22.1) into `~/.local`. corepack's shims and global installs go there too, since `/usr` doesn't outlast a cold boot.

**Public ports**
- `POST /v1/instances/{id}/public-ports` gives a URL with a random 20-character slug and no auth at the edge.
- `DELETE …/public-ports/{port}` kills that URL in about 10 s; after that it answers 401.

**GitHub webhooks to a sleeping VM** (November 2026)
- A delivery to a public port wakes the VM. It was answered in 0.8 s (16 s after the VM fell asleep) and 1.3 s (after 2 minutes asleep), inside GitHub's 10 s limit.
- Once, a wake took over a minute: GitHub recorded a 500 ("context deadline exceeded") and doesn't retry by itself. That's why the inbox also catches up on every wake.
- Agent37's API can still say `sleeping` for a few seconds after a request woke the VM.
- A review with line comments arrives as `pull_request_review.submitted` plus a `pull_request_review_comment.created` per comment, within milliseconds, so the second costs no wake. A review's line comments are at `pulls/{n}/reviews/{id}/comments`.
- The machine's gh login is usually its owner's account, so the agent's pull requests are the owner's own: GitHub won't let them approve or request changes there, so their reviews come as `COMMENTED`. What the agent writes shows as the owner too, which is why the inbox goes by `<sub>t3agent37</sub>`, never by author. The same login sees the owner's unsubmitted (`PENDING`) reviews; they're skipped.
- Cloudflare Workers Builds reports as a check run (app `cloudflare-workers-and-pages`, "Workers Builds: <name>"), so `check_suite.completed` and `commits/{sha}/check-runs` cover it. Its log is only on Cloudflare's dashboard; the agent found the cause from the diff.
- Bot comments that edit themselves (Cloudflare's build status) send `issue_comment.edited` for each change.

**Resize and costs**
- Resize works only on a running VM, which then boots fresh onto its new shape.
- `GET /v1/usage` reports spend by instance and UTC day, settled hourly. `GET /v1/billing` gives the wallet.

## T3 Code behaviors verified live

- **T3 Connect sign-in only stores the link at first.** `t3 up --connect --headless --yes` stores it as "pending server startup", and T3 applies it the next time it starts. `t3 up` expects to restart its own background service, which doesn't exist here because `boot.sh` runs T3. So after the sign-in, `connect.rs` kills T3's process group (from `t3.pid`), lets `boot.sh` restart it, and waits for `/api/connect/link-state` to report linked with an active tunnel.
- **Connect preferences need a linked machine.** `POST /api/connect/preferences` answers 400 "Link this environment to T3 Connect first." until the restart above has happened.
- **The T3 Connect relay can't wake a sleeping machine.** The tunnel is dialed out from the VM. That's why the wake link exists.
- **The phone app lists only environments added on the phone.** Add them under **Settings → Environments → T3 Connect**. A free T3 account allowed 3 machines there.
- **The preview tunnel:** `GET /api/preview/tunnel?port=N` upgrades to a WebSocket that carries raw TCP bytes to that port on the machine's loopback (T3 dials `127.0.0.1`, then `::1`). It takes the proxy's own bearer token through Agent37's edge. Nothing listening is a `502` with `x-t3-tunnel-port-refused`, and the tunnel turned off in T3's settings (`enablePreviewTunnel`) is a `403` with `x-t3-tunnel-disabled`. Both headers survive the edge. A new connection costs about 0.5 s, and a 20 MB transfer took 2.6 s.
- **`subscribeDiscoveredLocalServers`** (payload `{}`) answers `{servers: [{port, processName, pid, terminal, agent}], scannedAt}` at once. It lists only servers T3's own terminals and agents started: a server started over Agent37's exec isn't in it.
- **Server settings:** `server.getSettings` and `server.updateSettings {patch}` take the proxy's token. A patch merges record fields by key (`{projectScriptOverrides: {}}` changes nothing), and `null` removes a key. `requireConnectDeviceApproval` is the "Require approval for new T3 Connect devices" switch. T3 Connect's `preferences` hold only `publishAgentActivity` (phone push and Live Activities) and `holdWebhooksWhileOffline`.
- **Project scripts live in two places:** the project's own (the shell's `scripts`) and the server's settings, where T3 Code's script editor saves them (`projectSettingsOverrides[projectId].defaultProjectScripts`; the older `projectScriptOverrides[projectId]` key is still accepted and translated, and `getSettings` derives it back). `orchestration.dispatchCommand` takes no `project.*` command from a client, so `copy_scripts` writes the settings, and reads them here first (`T3Desktop::project_scripts`; t3agent37's read-only session may).
- **Which address a machine's T3 issues link tokens for:** its OAuth `resource` is the origin of the request as it arrives, so through Agent37's edge it was the public hostname until T3 0.1.12 and is now the VM's internal address (`https://10.x.x.x:3773/mcp`; `x-forwarded-host` doesn't change it). A link asking for any other address fails with `PeerLinkError: This server only issues tokens for …`. T3 checks `resource` only at the approval and the token request, a missing one passes both, and the session it issues isn't bound to an address (`/mcp` checks only the session). So `proxy/link.rs` drops `resource` from both steps, which also survives the VM's address changing.
- **`t3 thread start <project> <message>`** works on a machine over exec (`--provider claudeAgent --model ...` when the project has no default model; `--wait` prints the reply). Handy for tests.
- **Models come from the server config:** `providers[].models` has each model's `slug`, `name`, `aliases`, `isLegacy` and `capabilities.optionDescriptors`. Reasoning is a `select` (Claude's has id `effort`: low, medium, high, xhigh, max, and `ultrathink`, which is in `promptInjectedValues`, i.e. goes into the prompt). `t3 thread start --option effort=low` sets it; the thread's `modelSelection` then holds `options: [{id: "effort", value: "low"}]`. A provider that isn't signed in lists no models.
- **`providerStatusDeltas`** are made per subscription, against what that subscription was last sent (`withProviderStatusDeltas`); each lists every provider whole, minus `models`, `slashCommands` and `skills` when unchanged, and T3 Code drops a provider whose lists it can't fill. Once (November 2026), the proxy's recorded config held Claude with no models for about a minute after T3 re-checked its providers, and Codex's sign-in state went missing, while T3 itself listed all 14. A `server.refreshProviders` from another client, whose full `providerStatuses` goes to every subscriber, set it right. The cause wasn't found; `MachineRuntime::issue_models` keeps the last full list meanwhile.
- **T3 Code asks `issues.detail` (`{projectId, number}`) as it connects**, for each thread linked to a GitHub issue, which threads started from issues are. The proxy didn't know the call, so each reconnect (an app restart, or T3 Code's own after a sleep) woke a machine that had worked on an issue. It's recorded and served as last seen now, like `pullRequests.detail`; the first connect after a new issue thread still wakes once to record it.
- **`t3 thread start --worktree origin/main --branch issue-1`** makes the worktree from a remote-tracking ref (under `~/.t3/worktrees/<project>/`) and runs the thread in full-access mode. `--json` answers with the thread's id, which starts with `mcp:` for threads started this way.
- **T3 Code updates only servers that report a self-update capability.** Its "Update N machines?" dialog skips any other as "needs a manual update". A server reports `serverSelfUpdate: "boot-service"` (in the descriptor's `capabilities`) when T3's service launcher started it: `t3 __service-launcher` with `T3CODE_HOME` set, which runs `<runtime>/t3 serve` with no flags as a child with an IPC channel, its environment from `$T3CODE_HOME/service.env` (`T3CODE_HOST`, `T3CODE_PORT`, `T3CODE_NO_BROWSER`), and the version from `runtime/service-state.json` (`{"protocol": 4, "activeVersion": …}`; 0.1.10, 0.1.11 and the 0.1.12 nightlies all read protocol 3 or 4). Without `T3_BOOT_SERVICE_UNIT` it touches no systemd unit, so it runs fine under `boot.sh`. The update is `server.updateServerWithProgress {targetVersion}` (a stream: `downloading`, `installing`, then `complete`). On skiff (4994 → 5038), the download took 5 s, then the launcher backed up the database, started the new server (migrations, then listening, in 5 s), marked the state `committed` and pointed `~/.local/bin/t3` at it. The launcher process itself stays on the old version until it restarts. It accepts only newer versions. A request sent to the old server as it shut down hung, unanswered, until the client's own timeout (Node's fetch waits 300 s), so give such reads a timeout. curl has no timeout at all: on 2026-11-22 provisioning skiff onto a new nightly hung for six minutes on one `curl` to the old server as it restarted, so the machine's own checks pass `-m 5`. `t3 up --connect` writes `T3CODE_LAN=false` into `service.env`, but an explicit `T3CODE_HOST` wins over it.
- **T3 knows who each agent is signed in as.** A provider status's `auth` has `status`, `email`, `type` ("Claude Ultra") and `label` ("Claude Ultra Subscription"; Codex's are like "ChatGPT Plus Subscription"), and `usageLimits` has `checkedAt` and `windows` (`label` such as "Weekly" or "Weekly · Fable", `usedPercent`, `resetsAt`). T3 0.1.12 also has sign-in flows of its own (`provider.auth.start`, `provider.auth.logout`), but its statuses on a machine say `setup.canAuthenticate: false` for Claude and Codex, so t3agent37 runs the CLIs' logins.
- **Logins over an existing one** (November 2026, on skiff): `claude auth login` starts a fresh OAuth while signed in (`--email` adds `login_hint` to its URL), and `claude auth status` keeps saying signed in until it finishes, so only the login's exit says it's done. Killed before finishing, it leaves the old login as it was. `gh auth login --web` (gh 2.105) runs without asking while signed in, shows a new device code, and leaves the old login alone when stopped. A finished one adds the account, which becomes the active one. When killing a login by name, note that `pkill -f 'claude auth login'` also matches the shell that runs it, if its command line has those words.
- **Claude Code refuses `--dangerously-skip-permissions` (T3's full-access mode) as root** unless `IS_SANDBOX=1` is set. Everything on the VM runs as root, so `boot.sh` exports `IS_SANDBOX=1` for the T3 server.

## Design

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

## History

t3agent37 forked [t3boat](https://github.com/paukraft/t3boat) in November 2026; t3boat is a good `upstream` remote to compare with. `41635c4` is the last commit with the Tauri shell and the React panel.

**The panel.** t3boat's panel was a React page in a Tauri webview. t3agent37 drew it natively instead, to get the webview and JavaScript out of the app.
- [Rune](https://github.com/anomalyco/rune) was looked at first and set aside for now: it had no macOS menu-bar item and was weeks old.
- [GPUI](https://gpui.rs), through [gpui-kit](https://gpui-kit.com) 0.10, was chosen. The menu-bar item and the panel's placement follow [F2](https://github.com/getf2/f2), a GPUI app for macOS.
- The React panel was ported view for view, and the rooftop's pixel engine matched the TypeScript one byte for byte (86 pictures). Then Tauri and the React panel were removed on 2026-11-20. Both are in `41635c4`.
- The panel calls the backend directly; there's no JSON contract in between. The window is hidden, not closed, when the panel goes away, so a sign-in in the browser or a half-typed name is still there on the next click.
