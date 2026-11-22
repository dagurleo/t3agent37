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
  - No "Co-Authored-By: Claude" or "Generated with Claude Code" on what agents commit and open from a machine, unless its Claude settings already say otherwise.
  - T3 Code's own update ("Update N machines?") updates the machines too, instead of skipping them as "needs a manual update".
  - A new look: "field agents" on a rooftop instead of boats.
  - A native panel drawn with [GPUI](https://gpui.rs) in place of t3boat's Tauri webview, so there's no JavaScript in the app (see [History](CONTRIBUTING.md#history)).

## What you get

- Create, resize and delete machines (Agent37 VMs: Ubuntu with Docker, 2–8 vCPU) from the menu bar
- Add projects the way T3 Code's "Add to another machine" does, `.env` files included
- Sign Claude Code, Codex and GitHub in on a machine with their own logins. Each machine has its own sign-ins, so each can use a different account: a different Claude subscription per machine, say. Its details show who each agent is signed in as, the plan, and how much of the plan's limits is used
- T3 Connect, to reach a machine from your phone or a browser, plus a wake link for when it's asleep. Optionally, new devices wait for your approval, and agent activity is pushed to the T3 Code phone app
- Previews: a dev server an agent starts on a machine opens in your own browser at `localhost`, through T3's tunnel. Opening the page wakes the machine; a tab left open doesn't keep it awake
- Projects set up as they're added: their T3 setup scripts run, or their dependencies install from the lockfile (Bun, pnpm, Yarn, npm, uv and more), and their T3 scripts come along
- GitHub issues as pull requests (a prototype): turn on **issues** for a project, pick the label and the agent, label an issue, and the machine wakes, works on it in a branch of its own and opens a pull request
- Notifications when a machine's agents finish, when your Agent37 wallet runs low, and when a machine stays awake for hours
- Cost per machine, today and this month, and your wallet balance
- An MCP server, so agents on your computer can manage machines and open their ports

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

To give a machine another account than your browser's (a second Claude subscription, a work GitHub), click **Other account** next to **Sign in**, or **Switch account** once it's signed in. The page opens in a private window of your browser, where you sign in with that account. A switch takes effect when you finish it. **Cancel**, next to it while it waits, stops it, and the machine keeps its old sign-in (closing the browser window doesn't stop it). Safari can't be opened in a private window from another app, so there the page opens as usual: switch accounts on the page, or sign out there first.

To use a machine from your phone:

1. In the machine's details, turn on T3 Connect under **other devices**.
2. In the T3 Code phone app, add the machine under **Settings → Environments → T3 Connect**.
3. T3 Connect can't wake a sleeping machine, so click **Copy link** next to "Wake link" and put that link on your phone's home screen.
4. Tap the link, wait for "awake", then pick the machine in T3 Code.

The wake link only wakes the machine and holds it up for 5 minutes. Using the machine still takes your T3 sign-in. **Replace link** retires the old link within about 10 seconds.

To see what an agent built, open it under **previews** in the machine's details: servers its agents and terminals started are listed there, and **Open another port…** takes any port by number. Each one opens at `http://localhost:<port>` (the same number when it's free). While the machine sleeps, opening or reloading the page wakes it; anything a page sends on its own is dropped, so a forgotten tab never keeps waking it.

To hand GitHub issues to a machine, flip **issues** next to the project in its details (the machine must be signed in to GitHub with admin rights on the repo). Pick the label (`agent` unless you change it; it's made in the repo if it's new), the model and how hard it thinks, from what the machine's signed-in agents offer, then **Turn on**. **Change**, under the project, changes them later. Then label an issue:

1. GitHub tells the machine through a webhook, which wakes it if it sleeps.
2. The machine checks that whoever added the label can push to the repo, comments on the issue, and starts a thread on it in T3 Code, in a new worktree on the branch `issue-<number>`.
3. The agent pushes the branch and opens a pull request that fixes the issue. Then the machine sleeps as usual.

The issue's text goes to an agent that can push, so only labels from people with push access count. Anyone who can open an issue can still write its text, so on a public repo read an issue before you label it.

Settings has the notifications (wallet line, hours awake, agents finishing) and whether projects get set up when they're added.

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

- **Each machine is an Agent37 VM** (`agent37-ubuntu` template). It runs the T3 server at your desktop app's version, or a newer one if T3 Code updated it first, under T3's own service launcher so T3 Code can update it.
- **T3 Code talks to the machine through a local proxy** at `http://localhost:<port>`, one per machine. The proxy adds the Agent37 key on the way out. While the machine sleeps, the proxy answers T3 Code from what it recorded, so opening a chat doesn't wake the machine. A real request, such as sending a message, wakes it.
- **A watchdog on the VM decides when to sleep:** 5 minutes after the last agent turn and the last use. It works out "idle" itself, so work finishes even with the laptop closed. It holds Agent37's auto-sleep off with a heartbeat until it is idle. Then the app sleeps the VM, or auto-sleep does if the app isn't running.

The full internals are in [apps/desktop/docs/architecture.md](apps/desktop/docs/architecture.md).

## Status (2026-11-22)

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
- Previews: listing, opening in Chrome, a port with nothing on it, a 20 MB transfer, and the sleep rules (a background request leaves the machine asleep; a page wakes it in about 5 s).
- Project setup on a real add (Bun install from a frozen lockfile), and npm and pnpm on a bare machine.
- T3 Connect's device approval and phone push switches, both ways.
- The three notifications, the scrollbars, and the settings.
- Linking a machine into T3 Code 0.1.12 for agents on this computer, and a call over the link waking it (about 5 s).
- GitHub issues, end to end on a private repo: labeling an issue on a sleeping machine woke it, started a thread in its own worktree, and the agent opened a pull request (a README change, with the repo's checks run). Deliveries woke a sleeping machine in 0.8 and 1.3 s. Once it took over a minute, so GitHub gave up after 10 s, and the machine took the issue on when it came up.
- A sleeping machine that worked on issues stays asleep when T3 Code reconnects. T3 Code asks for the issue's details then, and the last answer is kept for that.
- Choosing the label and the agent (through the MCP tool): changing them kept the webhook and its secret and made the new label. An issue given the old label was left alone; the new label started a thread on Claude Sonnet 5.5 at low effort, which opened a pull request.
- Accounts per machine, on the machine's side: the account, plan and usage limits read from what T3 recorded on skiff. Starting Claude's and GitHub's logins again over an existing sign-in, then canceling, kept the old sign-in, and the login's end was recorded.
- T3 Code's own update, on the machine's side: switching a running machine to T3's service launcher (T3 Connect came back by itself), the server then reporting that it can update itself, T3's update call taking it from one nightly to the next (back in about 5 s), provisioning afterwards with the older desktop version keeping the newer T3 without a restart, and a reboot of the boot container starting the new version.

**Not tried yet:**
- Resize, delete, create, sign-ins, T3 Connect and the wake link from the GPUI panel. (They worked from the Tauri panel; the GPUI one calls the same backend functions.)
- The GPUI panel's setup view (first run, no key yet).
- The GitHub code in the panel during a sign-in (the machine was already signed in).
- Copying a project's T3 scripts on a real add. The settings call it uses was tried by hand.
- How long T3 Connect takes to come back after a machine has slept for many hours.
- The **issues** switch and its settings in the panel. The same calls worked through the MCP tool (`github_issues`): off, on, and changing the label and the agent.
- Finishing a switch to another account in the browser, the private window opening, and the agents section in the panel itself. Also, signing Codex in over an existing sign-in.
- The "Update N machines?" dialog in T3 Code updating a machine through the proxy (the same update call was made on the machine itself). Also, under the launcher: creating a machine, moving one to a newer desktop version by provisioning, and turning on T3 Connect.

**Known gaps:**
- **Previews list only servers T3's agents and terminals started.** Others (Docker, a database) open by number.
- **GitHub issues are a prototype.** Only an issue's title and description go to the agent, not its comments. The machine's GitHub login covers all its repos: there's no token limited to one repo yet. A label renamed in the settings leaves the old one in the repo.
- **Usage limits are as of the machine's last check.** T3 checks them while the machine is awake, so a sleeping machine's details say how old they are, and use of the same plan elsewhere (your laptop, another machine) shows up only at the next check.
- **The machine's model list, as t3agent37 records it, once lost Claude's models** for about a minute after T3 re-checked its providers (cause not found; see CONTRIBUTING.md). The issue settings fall back on the last full list.

## Security

The Agent37 API key stays on your computer, in the app's data folder, readable by you only; it's never copied to a machine. Wake links are secret URLs: anyone with one can wake that machine (not use it). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) has how to build, test and release, the repo map, and the Agent37 and T3 Code behaviors that were verified live. Read it before changing anything; coding agents get it through [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE), as t3boat. Not affiliated with or endorsed by T3 Code, Agent37 or Boat.
