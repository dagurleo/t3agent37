# t3agent37

t3agent37 is a menu-bar app that creates cloud machines on your [Agent37 Cloud](https://www.agent37.com) account and adds them to [T3 Code](https://t3.codes) as remote environments. They sleep when idle, billing disk alone, so you pay compute only for the minutes your agents run.

Chat in T3 Code as usual. A message to a sleeping machine wakes it in a few seconds; running work finishes with your laptop closed.

**[t3agent37.dagurleo.com](https://t3agent37.dagurleo.com)** · [Download](https://github.com/dagurleo/t3agent37/releases/latest) · macOS 13+

## Where it comes from, and why

t3agent37 is a fork of [t3boat](https://github.com/paukraft/t3boat) by Pau Kraft (MIT). t3boat does the same job on [Boat](https://boat.dev): it gives T3 Code remote machines that sleep when nobody is working. It was forked in November 2026 to run on Agent37 Cloud instead of Boat, because the VMs were already there.

What was kept and what changed:

- **Kept from t3boat:** the localhost proxy (`apps/desktop/src/proxy`), changed by two lines in `upstream.rs` to send the Agent37 key. It keeps T3 Code working while a machine sleeps, so most of the hard T3 protocol work is Pau Kraft's. `apps/desktop/docs/t3-protocol.md` is t3boat's spec of that protocol.
- **New:** everything that talks to the cloud. `agent37.rs` replaces `boat.rs`. It covers how a machine boots, how it decides to sleep and how costs are read, because Agent37 VMs differ from Boat sandboxes in ways that mattered (see [Agent37 behaviors](CONTRIBUTING.md#agent37-behaviors-verified-live)).
- **Added on top:**
  - Restarting T3 after the T3 Connect sign-in.
  - A wake link, so a phone can wake a machine.
  - Claude Code's full-access mode working as root.
  - A new look: "field agents" on a rooftop instead of boats.
  - A native panel drawn with [GPUI](https://gpui.rs) in place of t3boat's Tauri webview, so there's no JavaScript in the app (see [History](CONTRIBUTING.md#history)).

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

Download `t3agent37.dmg` from the [latest release](https://github.com/dagurleo/t3agent37/releases/latest), open it and drag t3agent37 to Applications. It's signed and notarized by Apple.

It updates itself: it looks for a new release once a day, and **Check for Updates…** (right-click the menu-bar icon) or **Settings → Check now** looks right away.

To build it yourself instead, see [CONTRIBUTING.md](CONTRIBUTING.md#build-install-test-macos). It lives in the menu bar and can run next to t3boat.

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

## Status (2026-11-20)

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

**Known gaps:**
- **GitHub sign-in copies its device code to the clipboard.** Copying anything else afterwards loses it. Showing the code in the panel would be better.
- **T3 Connect's "require approval for new devices" option** is not turned on by the app.
- **The panel has no scrollbars.** Its views scroll, but no bar shows.

## Security

The Agent37 API key stays on your computer, in the app's data folder, readable by you only; it's never copied to a machine. Wake links are secret URLs: anyone with one can wake that machine (not use it). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) has how to build, test and release, the repo map, and the Agent37 and T3 Code behaviors that were verified live. Read it before changing anything; coding agents get it through [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE), as t3boat. Not affiliated with or endorsed by T3 Code, Agent37 or Boat.
