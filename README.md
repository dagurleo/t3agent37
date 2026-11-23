# t3agent37

t3agent37 is a menu-bar app that creates cloud machines on your [Agent37 Cloud](https://www.agent37.com) account and adds them to [T3 Code](https://t3.codes) as remote environments. They sleep when idle, billing disk alone, so you pay compute only for the minutes your agents run.

Chat in T3 Code as usual. A message to a sleeping machine wakes it in a few seconds; running work finishes with your laptop closed.

**[t3agent37.dagurleo.com](https://t3agent37.dagurleo.com)** · [Download](https://github.com/dagurleo/t3agent37/releases/latest) · macOS 13+ · [the command](#the-command): macOS and Linux

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
  - A new look: "field agents" over a night skyline instead of boats, in a quiet Graphite palette. The original animated rooftop is a setting.
  - A native panel drawn with [GPUI](https://gpui.rs) in place of t3boat's Tauri webview, so there's no JavaScript in the app (see [History](CONTRIBUTING.md#history)).

## What you get

- Create, resize and delete machines (Agent37 VMs: Ubuntu with Docker, 2–8 vCPU) from the menu bar
- Add projects the way T3 Code's "Add to another machine" does, `.env` files included
- Sign Claude Code, Codex and GitHub in on a machine with their own logins. Each machine has its own sign-ins, so each can use a different account: a different Claude subscription per machine, say. Its details show who each agent is signed in as, the plan, and how much of the plan's limits is used. GitHub can instead take a token that reaches only the repos you pick
- T3 Connect, to reach a machine from your phone or a browser, plus a wake link for when it's asleep. Optionally, new devices wait for your approval, and agent activity is pushed to the T3 Code phone app
- Previews: a dev server an agent starts on a machine opens in your own browser at `localhost`, through T3's tunnel. Opening the page wakes the machine; a tab left open doesn't keep it awake
- Projects set up as they're added: their T3 setup scripts run, or their dependencies install from the lockfile (Bun, pnpm, Yarn, npm, uv and more), and their T3 scripts come along
- GitHub issues as pull requests (a prototype): turn on **issues** for a project, pick the label and the agent, label an issue, and the machine wakes, works on it in a branch of its own and opens a pull request. Reviews, `/agent` comments and failed CI on that pull request go back to the agent, which pushes fixes to it
- Webhooks that wake the machine: T3 Code's scheduled tasks can run on a webhook, and each machine's details give those tasks an address that wakes it, so a Sentry alert, a Linear issue or a Zapier step starts an agent on a sleeping machine right away
- A start URL per machine: an assistant such as Grok, a phone shortcut or a script starts threads in any of the machine's projects with plain web requests, then checks on them, reads the agent's reply and follows up. A request wakes the machine
- Machines shared between computers: t3agent37 on another computer with the same Agent37 account lists the machines made elsewhere, and **Add here** gives that computer a sign-in of its own to one. GitHub issues and the start URL are the machine's settings, so both computers show the same
- What each machine is doing: its details show the threads its agents work on (project, model, how long, and whether one waits for your approval), what woke it, its CPU, memory and disk with the busiest processes, and what its agents finished in the last day; a thread opens in T3 Code on click. The list says which thread is at work, which needs you, or what keeps the CPU busy
- Notifications when a machine's agents finish, when your Agent37 wallet runs low, and when a machine stays awake for hours
- Cost per machine, today and this month, and your wallet balance. Each machine's details also show what each of its projects cost this month: the time its agents worked, plus the idle minutes after, at the machine's hourly rate
- An MCP server, so agents on your computer can manage machines and open their ports
- A `t3agent37` command for the terminal: the machines, their details and what they're doing, waking and sleeping them, projects, sign-ins, GitHub issues and the rest, alongside the app. Without the app (on Linux, or a Mac you'd rather not run it on), `t3agent37 serve` runs t3agent37 in its place, at login if you like

## What you need

- [T3 Code](https://t3.codes) desktop app
- An [Agent37 Cloud](https://www.agent37.com) account and an API key from [its dashboard](https://www.agent37.com/dashboard/cloud/api-keys). The key stays on this computer (in the app's data folder, readable by you only) and is never copied to a machine.
- macOS 13 or later for the app. (The panel is GPUI, which runs on Windows and Linux too, but the menu-bar item and the panel's placement are macOS code for now.) The command alone runs on macOS and Linux (x86_64 and arm64), with T3 Code's desktop app on the same computer.

## Install

Download `t3agent37.dmg` from the [latest release](https://github.com/dagurleo/t3agent37/releases/latest), open it and drag t3agent37 to Applications. It's signed and notarized by Apple. Or with [Homebrew](https://brew.sh), which also links the `t3agent37` command:

```sh
brew install --cask dagurleo/tap/t3agent37
```

It updates itself: it looks for a new release once a day, and **Check for Updates…** (right-click the menu-bar icon) or **Settings → Check now** looks right away.

To build it yourself instead, see [CONTRIBUTING.md](CONTRIBUTING.md#build-install-test-macos). It lives in the menu bar and can run next to t3boat.

### The command

```sh
curl -fsSL https://raw.githubusercontent.com/dagurleo/t3agent37/main/install.sh | sh
```

That puts `t3agent37` in `~/.local/bin`, from the latest release, checked against its checksum. `t3agent37 update` updates it the same way (`--check` only says whether there's a new one). Or with Homebrew, on macOS or Linux, which `brew upgrade` updates:

```sh
brew install --formula dagurleo/tap/t3agent37
```

With the app installed, the app's own program is the command too, and updates with it (the cask links it for you):

```sh
ln -s /Applications/t3agent37.app/Contents/MacOS/t3agent37 ~/.local/bin/t3agent37
```

The command works with the app while the app runs. Without the app:

```sh
t3agent37 setup              # your Agent37 API key, asked for (hidden) and kept on this computer
t3agent37 serve --install    # t3agent37 runs now and at every login (launchd on macOS, systemd on Linux)
t3agent37 create raven       # a machine; T3 Code asks you to add it
```

With Homebrew, `brew services start t3agent37` does what `serve --install` does. Tab completion, machine names and projects included: `t3agent37 completions zsh > "${fpath[1]}/_t3agent37"` for zsh, `source <(t3agent37 completions bash)` in `~/.bashrc`, or `t3agent37 completions fish > ~/.config/fish/completions/t3agent37.fish` (Homebrew's formula installs them).

`serve` is what the app does, without the panel: T3 Code reaches the machines through it, it puts them to sleep and wakes them, and it answers the command and coding agents. Its log is `~/Library/Logs/t3agent37.log` on macOS and `journalctl --user -u t3agent37` on Linux; `t3agent37 serve` alone runs it in the terminal instead. Run either the app or `serve`, not both: the second stops at once.

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

To give a machine only some of your GitHub repos, click **Some repos only…** next to GitHub in its details, then **Open GitHub**. GitHub's page for a new fine-grained token opens with its name, 90 days and the permissions machines need filled in. Pick the repositories under **Repository access**, generate the token, paste it into the field (it stays hidden) and press Enter. The app checks it with GitHub before waking the machine, signs the machine's gh in with it (the account it had is signed out), and then lists the repos it reaches and when it expires. One token reaches one owner's repos: your account's or one organization's. Fine-grained tokens can't read GitHub's Checks API, so with one, feedback on the agent's pull requests sees CI from GitHub Actions only, not other checks such as Cloudflare builds.

To see what an agent built, open it under **previews** in the machine's details: servers its agents and terminals started are listed there, and **Open another port…** takes any port by number. Each one opens at `http://localhost:<port>` (the same number when it's free). While the machine sleeps, opening or reloading the page wakes it; anything a page sends on its own is dropped, so a forgotten tab never keeps waking it.

To hand GitHub issues to a machine, flip **issues** next to the project in its details (the machine must be signed in to GitHub with admin rights on the repo). Pick the label (`agent` unless you change it; it's made in the repo if it's new), the model and how hard it thinks, from what the machine's signed-in agents offer, then **Turn on**. **Change**, under the project, changes them later. Then label an issue:

1. GitHub tells the machine through a webhook, which wakes it if it sleeps.
2. The machine checks that whoever added the label can push to the repo, comments on the issue, and starts a thread on it in T3 Code, in a new worktree on the branch `issue-<number>`.
3. The agent pushes the branch and opens a pull request that fixes the issue. Then the machine sleeps as usual.

Then work with it on the pull request, as with a colleague:

- **Reviews** from people who can push go to the agent: the summary and the line comments.
- **Comments** go to it when a line starts with `/agent`, such as `/agent rename the flag too`. Other comments are left alone.
- **CI** failing on the branch's latest commit sends it the failed checks.

The machine reacts 👀 to what it passed on. Comments within 30 seconds of each other go as one message. The agent works in the same thread, so it remembers the issue, then pushes to the same branch and replies once on the pull request. To stop a loop from running up costs, a pull request gets at most 6 rounds a day, and CI at most 3 fixes in a row; the machine says so on the pull request when it pauses. Each comment, review and CI run on the repo reaches the machine, and wakes it if it sleeps; when it's nothing for the agent, the machine goes back to sleep after about a minute.

The agent's first message has the issue's text and its comments up to the label (the latest 30, without the machine's own), each with its author's role in the repo, and says to go by the author and the repo's owner, members and collaborators where comments disagree. Comments made after it started don't reach it; on its pull request, `/agent` comments do.

The issue's text goes to an agent that can push, so only labels from people with push access count. Anyone who can open an issue can still write its text, so on a public repo read an issue before you label it.

To have any service start an agent, make a scheduled task with a webhook in T3 Code: its prompt, project and agent, and optionally filters (run only when `body.action` is `created`, say) and a signing secret. T3's own webhook URL goes through T3 Connect, which can't wake a sleeping machine: the delivery waits until something else wakes it. So the machine's details list its webhook tasks under **webhooks**, each with **Copy URL**: an address that wakes the machine first. T3 checks the token, signature and filters, logs the delivery and runs the task, as with its own URL. A delivery T3 turns away or filters out lets the machine sleep again after about a minute. The addresses work without T3 Connect too. Anyone with one can send the task deliveries, so treat it like a password, and use a signing secret for anything that matters.

To start threads from Grok, a phone shortcut or a script, turn on **start url** in the machine's details, and **Copy URL**. Then, with any HTTP client:

- `GET` the URL: the machine's projects and models, and how to call it.
- `POST` it JSON `{"project": "Tantalum", "prompt": "Fix the flaky login test"}` to start a thread there. `project` is a project's title, folder or id. Optionally, `title`, `model` and `effort` (from the list), and `"worktree": true` for a git worktree of its own instead of the project's folder. It answers `201` with a `threadUrl` and T3's own link to the thread.
- `GET` the `threadUrl`: its status, whether it's done or waits for you, and the agent's reply once it's done.
- `POST` the `threadUrl` JSON `{"prompt": "Also update the README"}`: a follow-up, the thread's next turn.

To keep the token out of the address, click **Copy address** and **Copy token** instead, and send the token as a header, `Authorization: Bearer <token>`, to the address (`…/start`). The `threadUrl`s it hands out then have no token in them either, and take the same header. Tools that log every address they call never see the token that way.

A request wakes the machine if it sleeps. Threads run in full-access mode, with the project's own default model, else Claude Opus 5.5 at high (or the machine's first model without Claude). The thread shows up in T3 Code as usual. Only threads the URL started can be read or followed up through it, and it takes 30 starts and follow-ups an hour at most. Whoever has the token acts as you, so keep it like a password: **Replace URL** retires the old one at once.

For Grok Bot, which has a computer of its own, give it the address and the token and tell it to call the address with `curl` and the header when you ask it to start something on your machine. Don't give it the **Wake link** by mistake: that one only wakes the machine and answers with an "is awake" page.

To use a machine from a second computer, install t3agent37 there with the same Agent37 API key. Its list shows the machines made on other computers under **on your agent37 account**. **Add here** wakes one, signs this computer in to it with a T3 sign-in of its own, and asks T3 Code there to add it. Then both computers use it as their own: its threads and projects, sign-ins, GitHub issues, webhook tasks and start URL are the machine's. What each app shows comes from the machine whenever it's up, so a change made on one computer shows on the other the next time it sees the machine awake, and a wake from the other computer shows within a minute. Keep both copies up to date: a machine remembers which version of t3agent37 set it up, and an older one leaves a newer one's scripts alone, so nothing that one added stops working.

To see what a machine is doing, open its details. While it's up, **now** lists the threads its agents work on, each with its project, model, how long it has run and what it's up to ("working", "needs your approval", "waits on subagents"), with any command it left running (a dev server). Under them: its CPU, memory and disk, and its busiest processes, which say what's behind "busy CPU"; at the top right, when it woke and what woke it (T3 Code, GitHub, the start URL, a webhook, scheduled work). **finished** lists what its agents finished in the last day; while it sleeps, as of when it went to sleep. Click a thread to open it in T3 Code.

From a terminal, `t3agent37 help` lists the commands. For example:

```sh
t3agent37 list                          # the machines, what each is doing, and the costs
t3agent37 details skiff                 # its agents, what it's doing and finished, its load, projects and costs
t3agent37 wake skiff                    # and sleep, keep-awake skiff 30, auto-sleep skiff off
t3agent37 add-project skiff Tantalum    # one of `t3agent37 projects`
t3agent37 sign-in skiff github          # in your browser; the GitHub code shows in the terminal (--cancel stops one)
t3agent37 issues skiff Tantalum on --model claude-opus-5-5 --effort high
t3agent37 start-url skiff on            # prints the URL: it's a secret
```

A machine is its name or id. `list`, `details` and `projects` take `--json` for what the MCP tools give agents. A long command (creating a machine, a sign-in, adding a project) shows the machine's progress as it goes, and ^C stops the waiting, not the work. `details` never wakes a machine, and leaves secrets out (the webhooks' waking URLs, the start URL): `wake-link` and `start-url` print theirs when you ask, and `details --json` has them all.

Settings has the notifications (wallet line, hours awake, agents finishing), whether projects get set up when they're added, and the look: the panel's size (Standard, Large or Huge; the expand button at the top of the list, ⌘+ and ⌘− change it while it's open) and the animated rooftop instead of the still skyline.

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

## Status (2026-11-23)

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
- Feedback on the agent's pull requests, on a private repo: a `/agent` comment, a review with a line comment and a failed Cloudflare Workers build each reached the issue's thread about 30 s later (the build's failure 1.5 minutes after the push that broke it). Each time the agent pushed a fix to the branch (for the build, a revert of the breaking commit) and replied once; its replies started no new round. Webhooks made before this got the new events on the inbox's first start, and pull requests already open were taken as seen. A plain comment (no `/agent`) on a sleeping machine's repo woke it, and it slept again about 70 s later instead of after 5 minutes.
- Webhooks that wake, on skiff: a delivery to a webhook task's waking URL, sent while the machine slept, was answered in 1.5 s with T3's own 202, and the task ran 0.1 s later. The app read the task from T3 and made the address. T3 took deliveries at the task's path directly: a matching one ran, a filtered one was answered 202 without a run, and a wrong token got 404. A filtered-out delivery to the sleeping machine let it sleep again 72 s later.
- The start URL, on skiff (the machine's side, set up by hand, called with curl from a laptop): the project list in 0.6 s, a thread started in a project in its own worktree (from `origin/main`, on a branch T3 named) with the model and effort asked for, its reply read once done, a follow-up answered in the same thread, a wrong token answered 404 and a thread it didn't start refused. With the token in an `Authorization: Bearer` header instead: the list, a thread started (its `threadUrl` without the token) and its reply read; no header, a wrong token or a `Basic` one answered 404, and the token-in-the-URL form kept working.
- Two computers on one machine (a second copy of the app's backend with its own data folder, on the same Mac): it listed skiff as on the account and no other VM, added it in 12 s (waking it) with a T3 sign-in of its own and the same watchdog key, and showed the same GitHub issue settings and start URL. Waking skiff from there provisioned it, leaving its version on the machine without restarting anything; this computer's app kept reaching skiff's watchdog, and saw the wake within a minute. Both apps put skiff to sleep as it went idle. With a newer version recorded on the machine, the second copy's provisioning changed nothing. A start URL replaced on this computer showed on the other when it next read skiff's settings.
- What a machine is doing, on faktor (through the MCP tool; the panel section not yet seen): the threads it finished, shown while it slept, after the watchdog's update; then, woken from the app, "woke for an agent on your computer", its CPU, memory and disk, and a one-core loop as its busiest process at 100%, with the list line "Working · busy CPU · sh -c while :; do :; done". A thread at work and one waiting for approval were checked against T3's real shell fields in tests only. Then in the panel, on skiff: a demo thread showed under **now** while it worked, with the load and its busiest processes (Claude Code, then a CPU loop), and under **finished** once it was done.
- Cost per project, on skiff: an agent turn in one project counted for it, the idle minute after it too, and the wake before it as other time; the app showed both, priced at the machine's rate.
- Accounts per machine, on the machine's side: the account, plan and usage limits read from what T3 recorded on skiff. Starting Claude's and GitHub's logins again over an existing sign-in, then canceling, kept the old sign-in, and the login's end was recorded.
- T3 Code's own update, on the machine's side: switching a running machine to T3's service launcher (T3 Connect came back by itself), the server then reporting that it can update itself, T3's update call taking it from one nightly to the next (back in about 5 s), provisioning afterwards with the older desktop version keeping the newer T3 without a restart, and a reboot of the boot container starting the new version.

- The `t3agent37` command, on a Mac: `list`, `details`, `status`, `sizes` and its errors against the running app (against an older app too, through MCP's own calls), and through a link to the app's own program. `setup` refused an empty key, checked the real one with Agent37 and saved it readable by its owner only. `serve`, with a data folder and port of its own: it listed the account's machines as made on another computer, refused a second `serve`, and stopped on SIGTERM. `serve --install` and `--uninstall` with launchd: with the app running, the one launchd started stopped at once and wasn't started again.
- The command alone on Linux (arm64, in Docker): its tests and a release build.
- The command against a new machine (probe, small), on 2026-11-23: `create` with its progress line, `details`, `add-project` (waking it, cloning, setting up), `port` (a server on the machine answering at `localhost` here), `sign-in github` showing the code and `sign-in --cancel` stopping it (the waiting command then ended with "the sign-in was canceled"), `resize` there and back, with ^C 4 s into one (the command stopped, the resize finished), `keep-awake`, `auto-sleep` off and on, `sleep`, `wake`, and their refusals of bad input. `issues` refused without GitHub, and turned on skiff's again with its settings. Creating a machine first failed with "Argument list too long" (see CONTRIBUTING.md); fixed, creating took about 30 s and re-provisioning skiff onto the new scripts about 30 s.
- An issue's comments reaching the agent, on a private repo: an issue with two comments from its owner, one asking for an exact last line and one for a mention of Bun. Labeled, the thread's first message quoted both, with the author's role, and the pull request did both.
- `serve --install` on Linux with systemd (Ubuntu 24.04 in Docker, a user with lingering, a stand-in key and `t3`): the unit started, logged to the journal and answered the command; killed with SIGABRT it was started again 30 s later, stopped it stayed stopped; `update` replaced the program under it and restarted it; `--uninstall` stopped and removed it. Completions there in bash and fish, and in zsh on a Mac (Tab completed a project with a space in its name).
- `update` on a Mac and on Linux: `--check`, a release that doesn't exist, `--to v0.1.9` (the Mac's download passed Gatekeeper as notarized), and its refusals of the app's program and of a build from source. `install.sh` against the published 0.1.9 on a Mac.
- The Homebrew tap: `brew style` and `brew audit --strict --online` of the cask and the formula, and the formula installed from a test tap, its `brew test`, and the command it put on PATH answering.

**Not tried yet:**
- Resize, delete, create, sign-ins, T3 Connect and the wake link from the GPUI panel. (They worked from the Tauri panel; the GPUI one calls the same backend functions.)
- The GPUI panel's setup view (first run, no key yet).
- The GitHub code in the panel during a sign-in (the machine was already signed in).
- Copying a project's T3 scripts on a real add. The settings call it uses was tried by hand.
- How long T3 Connect takes to come back after a machine has slept for many hours.
- The **issues** switch and its settings in the panel. The same calls worked through the MCP tool (`github_issues`): off, on, and changing the label and the agent.
- Finishing a switch to another account in the browser, the private window opening, and the agents section in the panel itself. Also, signing Codex in over an existing sign-in.
- The projects' costs and the webhooks section in the panel's details (the same figures and addresses came through the MCP tool).
- The start URL from the panel (turning it on, replacing it, turning it off), and from Grok itself. Turning it on and replacing it worked through the MCP tool.
- A second computer for real: the list's **on your agent37 account** section and **Add here**, and T3 Code there adding the machine.
- A thread opening in T3 Code from the **now** and **finished** sections.
- `serve` with T3 Code's Linux app, on a Linux desktop. The cask installed (its audit passed), and the formula on Linux.
- T3 Connect turned on for a new machine under T3's service launcher: on probe the sign-in finished and T3 restarted, then T3's relay refused the link because the T3 account already had its 3 tunnels.
- The Graphite look, the panel's sizes and the animated rooftop setting on screen (built and unit-tested; the mockups were reviewed).
- A GitHub token limited to some repos, end to end: GitHub's filled-in page, the panel's field, gh on the machine with the token, the repos it lists, and pull request CI from GitHub Actions. What was tried: the machine's check of its GitHub login (on skiff, gh's own sign-in read as reaching every repo) and GitHub refusing a made-up token (401).
- The "Update N machines?" dialog in T3 Code updating a machine through the proxy (the same update call was made on the machine itself). Also, under the launcher: moving a machine to a newer desktop version by provisioning.

**Known gaps:**
- **T3 Connect allows 3 tunnels per T3 account** (T3's free plan, as seen in November 2026). Past that, turning it on for a machine signs in and restarts T3, then fails with T3's own reason, "Relay managed tunnel limit reached": unlink an environment in T3 Connect first.
- **Previews list only servers T3's agents and terminals started.** Others (Docker, a database) open by number.
- **GitHub issues are a prototype.** The agent gets the issue and its comments as they were when it was labeled, not comments added after (except `/agent` ones on its pull request). On the pull request, CI counts through check runs (GitHub Actions, Cloudflare and other GitHub apps), not the older commit statuses some services post, and only the first 100 reviews and comments are read. The machine's GitHub login covers all its repos unless it's signed in with a token limited to some (**Some repos only…**). A label renamed in the settings leaves the old one in the repo.
- **The details don't say what an agent is saying or running right now**, only its thread's state: T3's thread list leaves the latest message out. Open the thread in T3 Code for that. They also miss what T3 doesn't track and what's quick: a server an agent starts as a plain shell command (not with its agent's background option) isn't listed under its thread, and subagents that finish within the app's 5-second refresh aren't counted (both seen on skiff).
- **The command needs T3 Code's desktop app on the same computer**, as the app does: t3agent37 adds the machines to it and reads its projects. `serve`'s notifications go through `osascript` on macOS (shown as Script Editor's) and `notify-send` on Linux, and the command doesn't restart a `serve` that runs in a terminal when it updates (it restarts the one `serve --install` set up).
- **Project costs are an estimate.** They're awake time at the machine's current hourly rate, so a resize reprices the month, and the disk (billed asleep too) isn't split between projects. Time with several projects' agents working is split evenly between them.
- **Usage limits are as of the machine's last check.** T3 checks them while the machine is awake, so a sleeping machine's details say how old they are, and use of the same plan elsewhere (your laptop, another machine) shows up only at the next check.
- **Some things stay on each computer:** notifications, ports forwarded to it, and what it recorded for T3 Code while the machine sleeps. A computer that didn't see the machine's last work wakes it once to catch up, as after a laptop was closed.
- **Copies before this version don't know about other computers:** one that old can put its older scripts back on a shared machine, so update every copy.
- **The machine's model list, as t3agent37 records it, once lost Claude's models** for about a minute after T3 re-checked its providers (cause not found; see CONTRIBUTING.md). The issue settings fall back on the last full list.

## Security

The Agent37 API key stays on your computer, in the app's data folder, readable by you only; it's never copied to a machine. Wake links are secret URLs: anyone with one can wake that machine (not use it). A start URL's token is a secret too, and a stronger one: anyone with it can start agents on that machine in full-access mode. It can travel in an `Authorization` header instead of the URL, so tools that log addresses don't keep it. To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) has how to build, test and release, the repo map, and the Agent37 and T3 Code behaviors that were verified live. Read it before changing anything; coding agents get it through [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE), as t3boat. Not affiliated with or endorsed by T3 Code, Agent37 or Boat.
