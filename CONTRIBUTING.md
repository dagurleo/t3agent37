# Contributing to t3agent37

Notes for anyone changing t3agent37, people and coding agents alike. Read them before changing anything: every fact under "verified live" cost a debugging round to learn.

Bug reports and pull requests are welcome. For a security problem, see [SECURITY.md](SECURITY.md) instead of opening an issue.

## Ground rules

- **The Agent37 API key never goes onto a VM and is never printed.** A workspace key can create and delete machines and spend the wallet. It lives in the app's data folder only.
- **Never print a wake link, a T3 session token or other secret in full.** Redact `eyJ…` tokens in command output. Wake links are capability URLs.
- **Only touch machines t3agent37 created.** The Agent37 workspace can hold other VMs that aren't this app's.
- **Coding agents: don't commit or push unless asked.**
- **Remember that any request to a sleeping VM's public URL wakes it** (and bills it). Nothing in the app should talk to a sleeping machine unless it means to wake it.
- **Run one t3agent37 at a time**, the app or `t3agent37 serve`. A second copy would share the data folder, ports and machines, so it refuses to start while port 47370 is taken.
- **Don't rebuild the binary a running copy runs from.** macOS kills a process whose signed binary changes underneath it (no crash report). Run the bundle, or a copy of `target/debug/t3agent37`.
- **Nothing the panel does on each refresh may reach a machine.** The panel refreshes on every change, every few seconds while a machine is up, and a hidden panel too. The details used to look again at the machine's settings on each (a command over Agent37's exec): with skiff's details left open, those piled up to 20 node processes at once (load 27), which kept it busy and awake. They look again when the view opens, then once a minute (`DETAILS_REFRESH_EVERY` in `backend.rs`).
- **Take a value out of a `Mutex` in a `let` before a struct literal that locks it again.** A guard made in one field (`m.cache.lock().unwrap().signed_in_providers()`) lives until the whole literal is built, so a later field that takes the same lock deadlocks. That froze the whole app once, in `app::view`.

## Build, install, test (macOS)

You need [Rust](https://rustup.rs) (rustup fetches the 1.99 toolchain the app pins) and Xcode's command line tools.

```sh
cd apps/desktop
scripts/bundle-macos.sh --install   # release build → target/release/bundle, copied to /Applications and opened (quits a running copy)
scripts/bundle-macos.sh --open      # the same, opened from target/release/bundle instead
cargo run -- --open                 # a debug build, panel open at launch (quit the bundled one first)
cargo test                          # backend, panel and command tests
cargo test --no-default-features    # the same without the panel, as CI runs it on Linux
cargo build --release --no-default-features   # the `t3agent37` command alone: no GPUI, no AppKit
(cd src/machine && node --test *.test.mjs)   # the machine's scripts (issues, start URL, activity)
cargo run --bin t3agent37-dev       # the backend alone, driven from stdin (see the file's header)
```

- **Signing:** `bundle-macos.sh` signs with your Developer ID if the keychain has one, else your Apple Development certificate, else ad hoc (fine for your own Mac). `SIGN_IDENTITY` picks one.
- **Updates in your own builds:** on only when Sparkle's key for t3agent37 is in your keychain (the maintainer's builds). Otherwise Settings says "This build doesn't update itself".
- **Toolchain:** Rust 1.99, picked by `apps/desktop/rust-toolchain.toml`, because gpui-pre 0.3.12 uses std APIs newer than 1.96. A first build compiles GPUI and takes several minutes.
- **The command** is the same binary: given a command (`target/debug/t3agent37 list`) it's the command, without one the app. Built without the `panel` feature it's the command alone, and a bare `t3agent37` prints its help. To try `serve` next to the running app, give it a data folder and port of its own: `T3AGENT37_DATA_DIR=<dir> T3AGENT37_MCP_PORT=47399 t3agent37 serve`, and the same two for the commands that talk to it. The key goes in with `t3agent37 setup < <key file>`; delete that folder after.
- **Environment:** `T3AGENT37_DATA_DIR` points at another data folder, `T3AGENT37_MCP_PORT` the MCP server (and the command) at another port. `T3AGENT37_KEEP_PANEL_OPEN=1` keeps the panel up when you click elsewhere (for recordings). `T3A_DEBUG=1` logs where the panel was placed. `RUST_LOG` sets log levels.
- **Outside a bundle** (`cargo run`), notifications go through `osascript` (shown as Script Editor's) and Start at login refuses: both need the bundle.
- **App data:** `~/Library/Application Support/dev.t3agent37.app/`. It holds `agent37-key`, `state.json`, `t3-token` (the app's read-only session on the desktop T3 server) and `machines/` (captures per machine).
- **On a VM:** everything lives in `/root/.t3agent37/`, including `boot.sh`, the watchdog, `t3.pid`, the machine's name and `app-version`, the version of the t3agent37 that last provisioned it (an older one leaves its scripts alone, for when several computers share the machine).
- **Ports on the VM:** T3 `3773`, watchdog `3774`, heartbeat `3775` (public, answers 204), wake link `3776` (public, static page), inbox `3777` (public while a repo's issues, a webhook task or the start URL needs it; acts only on signed GitHub deliveries, T3 webhook paths, which T3 checks, and `/start` with the start URL's token in its path or an `Authorization: Bearer` header).
- **Ports on this computer:** MCP server at `http://127.0.0.1:47370/mcp`, and `/cli` on the same port for the `t3agent37` command (the same tools, answered once a call is done); machine proxies from `47371` up.

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
- **How updates reach people:** the app (Sparkle 2.10, in `Contents/Frameworks`, loaded by `shell.m`) checks `https://github.com/dagurleo/t3agent37/releases/latest/download/appcast.xml` once a day, and "Check for Updates…" in the menu-bar item's menu or Settings checks now. The site's download button links to `releases/latest/download/t3agent37.dmg`. `install.sh` (at the repo's root, run from GitHub's raw URL) downloads `t3agent37-macos.tar.gz`, `t3agent37-linux-x86_64.tar.gz` or `t3agent37-linux-arm64.tar.gz` and its `.sha256` from `releases/latest/download/`; the command doesn't update itself. All of it relies on every release having those files under those names.
- **The command's builds:** the workflow's `linux` jobs build it on GitHub's x86_64 and arm64 Ubuntu runners (`--no-default-features`) before the approval, and `release-macos.sh` builds the macOS one for both chips, signs it with the Developer ID and notarizes it (in a zip: a bare program can't be stapled), into `target/release/dist-cli`, after the appcast so Sparkle never sees it. The Linux ones come in through `EXTRA_ASSETS_DIR`.
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
      hooks.rs                T3's webhook tasks, at addresses that wake the machine
      start.rs, start.mjs     the start URL: threads in its projects from anywhere, over HTTPS (served by the inbox)
      activity.rs, .mjs       what a machine is doing: its threads, what finished, what woke it, its load (in the watchdog's status)
      shared.rs               a machine used from several computers: adding one made elsewhere, its settings read from it
      preview.rs              a machine's ports at localhost here, through T3's preview tunnel
      setup-project.sh        on the VM: a newly added project's setup (its T3 scripts, or its lockfile)
      connect.rs              T3 Connect turn-on, restart, wake link
      signin.rs               Claude / Codex / GitHub logins on the machine
      projects.rs, link.rs, capture.rs, work.rs
    proxy/                    t3boat's sleep-aware T3 proxy (only upstream.rs changed)
    bin/t3agent37-dev.rs      the backend headless, for development
    bin/t3agent37/            the menu-bar app (GPUI), and the `t3agent37` command
      main.rs                 a command goes to cli/; else the app: one instance at a time, the menu-bar item, showing and hiding the panel
      cli/                    the command: mod.rs (the commands and their help), client.rs (the running t3agent37, at /cli), show.rs (list, details), serve.rs (serve, --install, setup), args.rs
      backend.rs              the backend on its own tokio runtime: setup, events, commands
      model.rs                what the panel does with a machine: actions, labels, numbers
      theme.rs                brand tokens and fonts
      macos.rs, macos/shell.m status item, panel placement, notifications, login item, updates (Objective-C, after F2)
      ui/                     the panel: mod.rs (views, toasts, commands), header (still skyline), rooftop (animated), list, details, now (a machine's activity), forms, settings, kit
      pixel/                  the pixel engine: palette, scene, spy sprites (the rooftop), flat (the still look)
  assets/fonts/               Inter, Funnel Display, IBM Plex Mono (OFL), embedded
  icons/                      tray.png, tray@2x.png (menu-bar template), app-icon.png, icon.icns
  scripts/                    bundle-macos.sh (the app), release-macos.sh (signed, notarized DMG and appcast), make_icons.py
  docs/                       architecture, T3 protocol spec
apps/web/                     the site, t3agent37.dagurleo.com (React 19 + Vite, prerendered): see "The site" below
  src/panel/                  the React port of the panel, restored from 41635c4, on its mock backend only
.github/workflows/
  ci.yml                      on each pull request and push to main: the app's tests on a Mac, the command's on Linux, and the site's build
  release.yml                 on a v* tag: the signed, notarized release (see "Releasing"), and the command's builds
install.sh                    installs the `t3agent37` command from the latest release (macOS, Linux)
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
6. If the command should have it (it needs step 5): `cli/mod.rs`, an entry in `COMMANDS` and its arm in `run`.

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

**GitHub tokens** (November 2026)
- A fine-grained token's `/user/repos` lists the account's repositories, whatever the token reaches: one made for the account with all its repositories listed other owners' too, and couldn't push to any of those. GitHub has no list of a token's repositories. Asking a repository's git push service (`https://github.com/<repo>.git/info/refs?service=git-receive-pack`, the token as the password) answers 200 only where the token can push, and changes nothing: on skiff, gh's own sign-in got 200 for all 229 repositories its account lists, in 11 s with 16 at a time.
- A fine-grained token covers one owner (the account or one organization), picked as "Resource owner" at the top of GitHub's page, which starts as the account. A deleted token makes gh answer `Bad credentials (HTTP 401)`.

**Listing and sharing**
- `GET /v1/instances` lists the workspace's instances with `name` (`t3agent37 · <name>` for t3agent37's), `status`, `resources`, `public_ports` and `created` (unix seconds).
- A sleep asked while another start or sleep of the same instance runs is answered `try_again` ("Another operation on this instance is still running"); the client retries with backoff. Two apps sleeping one machine hit this, and both ended with it asleep.

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
- **T3 Code pairs with DPoP since the 0.1.12 nightlies.** 0.1.12-nightly.20261122.5071 sends a `dpop` proof with the pairing exchange, then `Authorization: DPoP <token>` (plus a proof) on every request; environments paired with an older T3 Code keep sending `Bearer`. The proxy read only `Bearer`, so faktor, added on 2026-11-22, got 401 on every websocket ticket and stayed "Reconnecting…" in T3 Code while skiff worked. It takes both now (`proxy::auth::bearer`), without checking the proof, and answers T3 Code's renewal (the access token traded for a new one) with the same token. Details in `docs/t3-protocol.md` §1.3.
- **`projects.listIgnoredFiles` lists 200 ignored files at most** (`truncated: true` past that), sorted by path, from `git ls-files --others --ignored --exclude-standard --directory`. In a big monorepo, build output and logs filled the 200 before any `apps/*/.env`, so only the top-level `.env` reached the machine. Its `query`, a case-insensitive path substring, applies before the cap: t3agent37 asks for `.env` and `.dev.vars`.
- **A project whose clone fails stays in T3, empty.** `projectClone.start` registers the project before cloning, and a failed clone (a private repo on a machine with no GitHub access yet) leaves it there with an empty folder: on 2026-11-22 two projects added to a new machine before its GitHub sign-in were left that way. t3agent37 now removes such a project (`t3 project remove <id>`, which refuses one with threads) and its folder if empty. A later `projectClone.start` to the same destination clones into the project that owns it.
- **`t3 thread show <id> --json`** (`--messages N` for the latest N) answers `{thread, messages: [{role, runId, text}]}`; `thread` has `status`, `activeRunId`, `pendingRequestCount` and `link`, T3's own address for the thread (`https://app.t3.codes/threads/<environment>/<thread>`). `start.mjs` doesn't count a thread as done while its latest message is the user's, in case a follow-up's run hasn't started yet. `t3 thread start --worktree <ref>` without `--branch` names the branch itself (`t3/<slug>`), and `--generate-title` titles the thread from the message. Each `t3` command takes about 3 s on a large machine (Node starting).
- **A thread's live state is in the shell snapshot** (`/api/orchestration/shell`, protocol 2), for every thread: `activityRunStatus` (`preparing`, `starting`, `running`, `waiting`, or null), `activityRunStartedAt`, `status` (the latest run's: `queued` … `completed`, `interrupted`, `failed`, `cancelled`, `rolled_back`, or `idle`), `pendingRuntimeRequest` (`{kind, blocking, userOnly}`; `kind` is a provider's approval kind, `user_input`, `auth_refresh` or `dynamic_tool_call`; `blocking: false` is a question the run works past), `blockedOnThreadWait`, `pendingBackgroundTasks` (`[{kind: subagent | command | monitor | background_task, command?, description?}]`), `modelSelection` (`{instanceId, model, options: [{id, value}]}`), `latestRunCompletedAt`, `latestRunStopRequested`, `lastError`, `branch` and `worktreePath`; subagents have `lineage.relationshipToParent: "subagent"` and their `rootThreadId`. `latestVisibleMessage` is always null there (T3 sets it so), so what an agent is saying right now isn't in it.
- **Projects live in T3's database too:** `projection_projects` in `~/.t3/userdata/statev2.sqlite` has `project_id`, `title`, `workspace_root`, `default_model_selection_json` (null when the project has no default model), `archived_at` and `deleted_at`.
- **A webhook task's prompt can quote the request:** `{{body.path}}`, `{{headers.name}}`, `{{query.name}}`, `{{body}}` and `{{request}}`. T3 puts each value in `<webhook-data-…>` tags and adds a line telling the agent to treat it as untrusted data; on skiff, Claude Opus 5.5 still did what a task's own prompt said to do with it.
- **`t3 thread start <project> <message>`** works on a machine over exec (`--provider claudeAgent --model ...` when the project has no default model; `--wait` prints the reply). Handy for tests.
- **Models come from the server config:** `providers[].models` has each model's `slug`, `name`, `aliases`, `isLegacy` and `capabilities.optionDescriptors`. Reasoning is a `select` (Claude's has id `effort`: low, medium, high, xhigh, max, and `ultrathink`, which is in `promptInjectedValues`, i.e. goes into the prompt). `t3 thread start --option effort=low` sets it; the thread's `modelSelection` then holds `options: [{id: "effort", value: "low"}]`. A provider that isn't signed in lists no models.
- **`providerStatusDeltas`** are made per subscription, against what that subscription was last sent (`withProviderStatusDeltas`); each lists every provider whole, minus `models`, `slashCommands` and `skills` when unchanged, and T3 Code drops a provider whose lists it can't fill. Once (November 2026), the proxy's recorded config held Claude with no models for about a minute after T3 re-checked its providers, and Codex's sign-in state went missing, while T3 itself listed all 14. A `server.refreshProviders` from another client, whose full `providerStatuses` goes to every subscriber, set it right. The cause wasn't found; `MachineRuntime::issue_models` keeps the last full list meanwhile.
- **T3 Code asks `issues.detail` (`{projectId, number}`) as it connects**, for each thread linked to a GitHub issue, which threads started from issues are. The proxy didn't know the call, so each reconnect (an app restart, or T3 Code's own after a sleep) woke a machine that had worked on an issue. It's recorded and served as last seen now, like `pullRequests.detail`; the first connect after a new issue thread still wakes once to record it.
- **`t3 thread start --worktree origin/main --branch issue-1`** makes the worktree from a remote-tracking ref (under `~/.t3/worktrees/<project>/`) and runs the thread in full-access mode. `--json` answers with the thread's id, which starts with `mcp:` for threads started this way.
- **T3 Code updates only servers that report a self-update capability.** Its "Update N machines?" dialog skips any other as "needs a manual update". A server reports `serverSelfUpdate: "boot-service"` (in the descriptor's `capabilities`) when T3's service launcher started it: `t3 __service-launcher` with `T3CODE_HOME` set, which runs `<runtime>/t3 serve` with no flags as a child with an IPC channel, its environment from `$T3CODE_HOME/service.env` (`T3CODE_HOST`, `T3CODE_PORT`, `T3CODE_NO_BROWSER`), and the version from `runtime/service-state.json` (`{"protocol": 4, "activeVersion": …}`; 0.1.10, 0.1.11 and the 0.1.12 nightlies all read protocol 3 or 4). Without `T3_BOOT_SERVICE_UNIT` it touches no systemd unit, so it runs fine under `boot.sh`. The update is `server.updateServerWithProgress {targetVersion}` (a stream: `downloading`, `installing`, then `complete`). On skiff (4994 → 5038), the download took 5 s, then the launcher backed up the database, started the new server (migrations, then listening, in 5 s), marked the state `committed` and pointed `~/.local/bin/t3` at it. The launcher process itself stays on the old version until it restarts. It accepts only newer versions. A request sent to the old server as it shut down hung, unanswered, until the client's own timeout (Node's fetch waits 300 s), so give such reads a timeout. curl has no timeout at all: on 2026-11-22 provisioning skiff onto a new nightly hung for six minutes on one `curl` to the old server as it restarted, so the machine's own checks pass `-m 5`. `t3 up --connect` writes `T3CODE_LAN=false` into `service.env`, but an explicit `T3CODE_HOST` wins over it.
- **T3 knows who each agent is signed in as.** A provider status's `auth` has `status`, `email`, `type` ("Claude Ultra") and `label` ("Claude Ultra Subscription"; Codex's are like "ChatGPT Plus Subscription"), and `usageLimits` has `checkedAt` and `windows` (`label` such as "Weekly" or "Weekly · Fable", `usedPercent`, `resetsAt`). T3 0.1.12 also has sign-in flows of its own (`provider.auth.start`, `provider.auth.logout`), but its statuses on a machine say `setup.canAuthenticate: false` for Claude and Codex, so t3agent37 runs the CLIs' logins.
- **Logins over an existing one** (November 2026, on skiff): `claude auth login` starts a fresh OAuth while signed in (`--email` adds `login_hint` to its URL), and `claude auth status` keeps saying signed in until it finishes, so only the login's exit says it's done. Killed before finishing, it leaves the old login as it was. `gh auth login --web` (gh 2.105) runs without asking while signed in, shows a new device code, and leaves the old login alone when stopped. A finished one adds the account, which becomes the active one. When killing a login by name, note that `pkill -f 'claude auth login'` also matches the shell that runs it, if its command line has those words.
- **Scheduled tasks can run on a webhook.** `scheduledTasks.subscribe` answers `{tasks, timeZone}`; a webhook task has `schedule: {type: "webhook", filters, signature, maxDeliveryAgeMinutes}` and `webhook: {path, url, hasSecret}`, where `path` is `/api/hooks/<task id, URL-encoded>/<token>` on T3's server and `url` is the same through T3 Connect's relay (`https://relay.t3.codes/v1/hooks/<endpoint key>/…`), null without T3 Connect. T3's server takes a delivery at `path` directly (POST, PUT, PATCH or GET): 202 `{deliveryId}` for an accepted one, and for one its filters turn away too (only the accepted one runs, and shows in `scheduledTasks.listWebhookDeliveries`); 404 `hook_not_found` for a wrong token. It limits deliveries to 60 a minute. `scheduledTasks.upsert` needs `title`, `prompt`, `enabled`, `schedule`, `projectId`, `workspaceStrategy` (`{type: "root"}`, or a worktree), `modelSelection` (`{instanceId, model}`), `runtimeMode` and `interactionMode`. The orchestrator's `schedule_task` tool can't make one in a project with no default model, and on skiff, `t3_project_update` didn't set one (`defaultModelSelection` stayed null).
- **Claude Code refuses `--dangerously-skip-permissions` (T3's full-access mode) as root** unless `IS_SANDBOX=1` is set. Everything on the VM runs as root, so `boot.sh` exports `IS_SANDBOX=1` for the T3 server.

## Design

The panel's theme is "field agents", in the "Graphite" palette (light and dark): neutral greys, coral only where something is happening. It was toned down from the animated "Noir amber" rooftop on 2026-11-22, because the moving pixel art got in the way of real work. The site and the app icon are still Noir amber.

**The still look (the default)**
- The panel's top is a still night skyline (`ui/header.rs`, `pixel/flat.rs`): flat buildings with a few lit windows, two points a pixel, drawn for the panel's width. On the list it carries the brand, a summary ("1 working · 2 idle · 1 asleep") and the size and settings buttons; on the other views, the back button and the view's title, so they have no header of their own.
- Each machine is a small flat spy (11×13 pixels, three points each) whose coat shows its state: coral working, grey idle, faint asleep (with a "z"), red on an error. Nothing moves.
- Status lines are in Inter; IBM Plex Mono is kept for chips, numbers and small print.

**The animated rooftop** (Settings → look → Animated rooftop)
- A pixel-art city rooftop: a skyline, a searchlight at night, a beacon by day, and a gravel roof.
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
- The engine is in `src/bin/t3agent37/pixel/`: bayer dithering and ramps, redrawn on the panel's 12 fps clock. `scene.rs` draws the roof, `spy.rs` the agents and `paint.rs` the strip and row icons; `flat.rs` the still skyline and spies.
- Tokens are in `theme.rs`, including the still header's night (`sky*`) and the spies' colours. The accent is `#d4552b` in light mode and `#ff8a5c` in dark; danger is a red apart from it. Body text is Inter, display Funnel Display SemiBold, small print IBM Plex Mono.

**Sizes**
- The panel opens Standard (380×560), Large (460×760) or Huge (580×940), as Settings remembers; the expand button on the list's header, ⌘+ and ⌘− change it while it's open, ⌘0 puts it back. It grows down and to both sides of the menu-bar item and never past the bottom of the screen (`shell.m` caps the height). The views are fluid; the skyline and the rooftop are drawn for the panel's width.

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
