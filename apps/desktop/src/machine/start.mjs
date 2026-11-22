// t3agent37's start URL, served by the inbox (issues.mjs) at <inbox URL>/start/<token>: a way for
// an assistant such as Grok, a phone shortcut or any script to start agent threads in the
// machine's projects, check on them and follow up, with nothing but HTTPS. A request to it wakes
// a sleeping machine, as for the inbox's other deliveries.
//
//   GET  <start URL>                    the machine's projects and models, and how to call this
//   POST <start URL>                    {project, prompt, title?, model?, effort?, worktree?}:
//                                       starts a thread, answers 201 with its threadUrl
//   GET  <threadUrl>                    its status and the agent's latest reply
//   POST <threadUrl>                    {prompt}: a follow-up, the thread's next turn
//
// The token is the only key: t3agent37 makes it, keeps it in the app and here, and replacing it
// retires the old one. It comes in the path (<inbox URL>/start/<token>, for tools that take only
// a URL) or, kept out of the address, as `Authorization: Bearer <token>` to <inbox URL>/start; the
// thread URLs handed out follow suit. A thread is readable here only if this URL started it. What
// comes in goes to the agent as the user's own message: whoever holds the token acts as the user.
//
// Run directly, it is the command t3agent37 uses to turn the URL on (or change its settings) and
// off:
//   node start.mjs on  < {token, url, agent: {provider, model, options}, models: [...]}
//   node start.mjs off

import { timingSafeEqual } from "node:crypto";
import { readFileSync, writeFileSync, renameSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

export const START_PREFIX = "/start/";
const DIR = join(homedir(), ".t3agent37");
/**
 * { token, url: the inbox's public URL, agent: {provider, model, options}: the model for a
 *   project with none of its own, models: [{provider, providerName, model, name, aliases,
 *   effort: {id, levels: [[id, label]], default} | null}] }
 */
const CONFIG_PATH = join(DIR, "start.json");
/** { threadId: { projectId, at } }: the threads this URL started, the only ones it shows. */
const THREADS_PATH = join(DIR, "start-threads.json");
const DB_PATH = join(homedir(), ".t3", "userdata", "statev2.sqlite");
/** Threads and follow-ups started in an hour, so a leaked URL can't run up much. */
export const MAX_STARTS_PER_HOUR = 30;
const HOUR_MS = 60 * 60_000;
const MAX_PROMPT = 100_000;
const KEEP_THREADS = 500;
/** T3's thread statuses while a run is in flight. */
const LIVE = new Set(["queued", "preparing", "starting", "running", "waiting"]);
/** Statuses a run ends in without an answer. */
const ENDED = new Set(["failed", "interrupted", "cancelled"]);

const readJson = (path, fallback) => {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return fallback;
  }
};
const writeJson = (path, value) => {
  writeFileSync(`${path}.tmp`, JSON.stringify(value, null, 2), { mode: 0o600 });
  renameSync(`${path}.tmp`, path);
};

/** A path the start URL answers: `/start` and below. */
export function isStartPath(url) {
  const path = String(url ?? "").split("?")[0];
  return path === "/start" || path.startsWith(START_PREFIX);
}

/**
 * `{token, threadId?}` for `/start/<token>[/threads/<id>]`, or with token null for
 * `/start[/threads/<id>]`, whose token comes in the Authorization header; `{token, bad}` for
 * anything else below `/start`; null for any other path. (Tokens are long and random, so none
 * is ever "threads".)
 */
export function parseStartPath(url) {
  if (!isStartPath(url)) return null;
  const segments = String(url).split("?")[0].split("/").slice(2);
  if (segments.at(-1) === "") segments.pop();
  const token = segments[0] === "threads" ? null : (segments.shift() ?? null);
  if (segments.length === 0) return { token };
  if (segments.length === 2 && segments[0] === "threads" && segments[1]) {
    try {
      return { token, threadId: decodeURIComponent(segments[1]) };
    } catch {}
  }
  return { token, bad: true };
}

/** The token of an `Authorization: Bearer <token>` header. */
export function bearer(header) {
  return /^\s*Bearer\s+(\S+)\s*$/i.exec(String(header ?? ""))?.[1] ?? null;
}

export function sameToken(want, got) {
  if (typeof want !== "string" || !want || typeof got !== "string") return false;
  const a = Buffer.from(want);
  const b = Buffer.from(got);
  return a.length === b.length && timingSafeEqual(a, b);
}

const fold = (s) => String(s ?? "").trim().toLowerCase();

/** The project `name` means: its id, its title or its folder's name, in any case. */
export function findProject(projects, name) {
  const want = fold(name);
  if (!want) return null;
  return (
    projects.find((p) => p.id === String(name).trim()) ??
    projects.find((p) => fold(p.title) === want) ??
    projects.find((p) => fold(basename(p.root ?? "")) === want) ??
    null
  );
}

/** A model's name without its agent's: "Opus 5.5" for Claude's "Claude Opus 5.5". */
const shortName = (m) => (m.name.startsWith(`${m.providerName} `) ? m.name.slice(m.providerName.length + 1) : m.name);

/**
 * The model and reasoning level asked for, among the machine's: `{provider, model, options}`,
 * or `{error}`. A model is found by its slug, name, short name or T3's other names for it; a
 * level by its id or label. No level: T3's default for that model.
 */
export function pickModel(models, name, effort) {
  const want = fold(name);
  const m = (models ?? []).find((m) => [m.model, m.name, shortName(m), ...(m.aliases ?? [])].some((n) => fold(n) === want));
  if (!m) return { error: `No model "${name}" here. Pick one of: ${(models ?? []).map((m) => m.model).join(", ") || "(none signed in)"}` };
  if (effort === undefined || effort === null || effort === "") return { provider: m.provider, model: m.model, options: [] };
  if (!m.effort) return { error: `${m.name} has no reasoning levels to choose from` };
  const level = m.effort.levels.find(([id, label]) => fold(id) === fold(effort) || fold(label) === fold(effort));
  if (!level) return { error: `${m.name} takes ${m.effort.levels.map(([id]) => id).join(", ")}` };
  return { provider: m.provider, model: m.model, options: [`${m.effort.id}=${level[0]}`] };
}

/** "Opus 5.5, high": how a model and its options read. */
export function describeAgent(models, agent) {
  if (!agent) return null;
  const m = (models ?? []).find((m) => m.provider === agent.provider && m.model === agent.model);
  const level = (agent.options ?? []).map((o) => o.split("=")[1]).find(Boolean);
  const label = m?.effort?.levels.find(([id]) => id === level)?.[1]?.toLowerCase() ?? level;
  return [m ? shortName(m) : agent.model, label].filter(Boolean).join(", ");
}

/** What `t3 thread start` takes for this request, the prompt itself on stdin. */
export function startArgs({ projectId, title, agent, worktree }) {
  const args = ["thread", "start", projectId, "-", "--json"];
  if (title) args.push("--title", title);
  else args.push("--generate-title");
  if (agent) {
    args.push("--provider", agent.provider, "--model", agent.model);
    for (const o of agent.options ?? []) args.push("--option", o);
  }
  if (worktree) args.push("--worktree", worktree);
  return args;
}

/** Starts within the last hour, and whether another may go. */
export function startsLeft(times, now) {
  const recent = (times ?? []).filter((t) => now - t < HOUR_MS);
  return { recent, full: recent.length >= MAX_STARTS_PER_HOUR, freeAt: recent.length ? recent[0] + HOUR_MS : now };
}

/** What `t3 thread show --json` says, as this URL tells it. */
export function threadReport(shown, threadUrl) {
  const t = shown.thread ?? {};
  const messages = shown.messages ?? [];
  const lastUser = messages.map((m) => m.role).lastIndexOf("user");
  const reply = messages.slice(lastUser + 1).filter((m) => m.role === "assistant").pop()?.text ?? null;
  const pending = t.pendingRequestCount ?? 0;
  // A follow-up just sent may not have started its run yet: not done until it's answered.
  const working = LIVE.has(t.status) || t.activeRunId != null || (reply === null && lastUser !== -1 && !ENDED.has(t.status));
  return {
    threadId: t.threadId,
    title: t.title ?? null,
    status: t.status ?? "unknown",
    done: !working && pending === 0,
    waitingForYou: pending > 0,
    reply: working ? null : reply,
    link: t.link ?? null,
    threadUrl,
  };
}

/** The machine's projects, from T3's database: id, title, folder and default model. */
async function listProjects() {
  const { DatabaseSync } = await import("node:sqlite");
  const db = new DatabaseSync(DB_PATH, { readOnly: true });
  try {
    return db
      .prepare(
        "SELECT project_id AS id, title, workspace_root AS root, default_model_selection_json AS model FROM projection_projects WHERE deleted_at IS NULL AND archived_at IS NULL ORDER BY title COLLATE NOCASE",
      )
      .all()
      .map((p) => ({ id: p.id, title: p.title, root: p.root, hasModel: !!p.model }));
  } finally {
    db.close();
  }
}

/** Where a new worktree starts: the remote's default branch, fetched, else what's checked out. */
async function worktreeBase(root, run) {
  try {
    await run("git", ["fetch", "--quiet", "origin"], { cwd: root, timeoutMs: 120_000 });
    const head = (await run("git", ["rev-parse", "--abbrev-ref", "origin/HEAD"], { cwd: root })).trim();
    if (head && head !== "origin/HEAD") return head;
  } catch {}
  return "HEAD";
}

/**
 * Answers a request to the start URL. `ctx`: the inbox's `touch` (holds the machine up),
 * `wokeForNothing` (lets it sleep again soon), `log`, `run` (a command), `waitForT3` and
 * `machineName`.
 */
export function startHandler() {
  let starts = [];
  return async function handle(req, body, res, ctx) {
    const reply = (code, value) => {
      res.writeHead(code, { "content-type": "application/json", "cache-control": "no-store" });
      res.end(JSON.stringify(value, null, 2));
    };
    const parsed = parseStartPath(req.url);
    const config = readJson(CONFIG_PATH, null);
    const given = parsed?.token ?? bearer(req.headers.authorization);
    if (!parsed || parsed.bad || !config || !sameToken(config.token, given)) {
      ctx.wokeForNothing();
      return reply(404, { error: "not_found" });
    }
    // The thread URLs it hands out carry the token only if this request's URL did.
    const address = `${String(config.url ?? "").replace(/\/+$/, "")}/start`;
    const startUrl = parsed.token ? `${address}/${config.token}` : address;
    const threadUrl = (id) => `${startUrl}/threads/${encodeURIComponent(id)}`;
    let input = {};
    if (req.method === "POST") {
      try {
        input = body.length ? JSON.parse(body) : {};
      } catch {
        ctx.wokeForNothing();
        return reply(400, { error: "Send JSON." });
      }
      if (typeof input !== "object" || input === null || Array.isArray(input)) input = {};
    } else if (req.method !== "GET") {
      ctx.wokeForNothing();
      return reply(405, { error: "GET or POST" });
    }
    const prompt = String(input.prompt ?? input.request ?? input.message ?? "").trim();
    const limited = () => {
      const left = startsLeft(starts, Date.now());
      starts = left.recent;
      return left.full ? new Date(left.freeAt).toISOString() : null;
    };

    try {
      // A thread: its state, or a follow-up.
      if (parsed.threadId) {
        const mine = readJson(THREADS_PATH, {})[parsed.threadId];
        if (!mine) {
          ctx.wokeForNothing();
          return reply(404, { error: "This URL didn't start that thread." });
        }
        if (req.method === "POST") {
          const refused = !prompt
            ? [400, 'Say what to tell the agent in "prompt".']
            : prompt.length > MAX_PROMPT
              ? [413, "That prompt is too long."]
              : limited() && [429, `${MAX_STARTS_PER_HOUR} starts and follow-ups an hour at most; the next one can go at ${limited()}.`];
          if (refused) {
            ctx.wokeForNothing();
            return reply(refused[0], { error: refused[1] });
          }
          ctx.touch(`follow-up from the start URL`);
          await ctx.waitForT3();
          await ctx.run("t3", ["thread", "send", parsed.threadId, "-"], { input: prompt, timeoutMs: 120_000 });
          starts.push(Date.now());
          ctx.log(`start: follow-up to ${parsed.threadId}`);
          return reply(202, { threadId: parsed.threadId, sent: true, threadUrl: threadUrl(parsed.threadId) });
        }
        await ctx.waitForT3(20_000);
        const shown = JSON.parse(await ctx.run("t3", ["thread", "show", parsed.threadId, "--json", "--messages", "6"], { timeoutMs: 30_000 }));
        ctx.wokeForNothing();
        return reply(200, threadReport(shown, threadUrl(parsed.threadId)));
      }

      const projects = await listProjects();
      const models = config.models ?? [];
      if (req.method === "GET") {
        ctx.wokeForNothing();
        return reply(200, {
          machine: ctx.machineName,
          projects: projects.map((p) => ({ title: p.title, id: p.id, folder: basename(p.root ?? "") })),
          models: models.map((m) => ({ model: m.model, name: m.name, agent: m.providerName, efforts: m.effort?.levels.map(([id]) => id) ?? [] })),
          defaultModel: describeAgent(models, config.agent),
          howTo: {
            start: 'POST this URL with JSON {"project": "<title, folder or id>", "prompt": "<what to do>"}. Optional: "title", "model" and "effort" (from models), "worktree": true for a git worktree of its own. Answers 201 with a threadUrl.',
            check: "GET the threadUrl: status, done, and the agent's reply once it's done.",
            followUp: 'POST the threadUrl with JSON {"prompt": "<what to say>"}: the thread\'s next turn.',
            token: 'In the URL (.../start/<token>), or kept out of it: the header "Authorization: Bearer <token>" on .../start, whose thread URLs then come without the token and take the same header.',
            defaults: `A project's own default model, else ${describeAgent(models, config.agent) ?? "the machine's default"}. Threads run in full-access mode, in the project's folder unless "worktree" is true.`,
          },
        });
      }

      if (!prompt) {
        ctx.wokeForNothing();
        return reply(400, { error: 'Say what to do in "prompt".' });
      }
      if (prompt.length > MAX_PROMPT) {
        ctx.wokeForNothing();
        return reply(413, { error: "That prompt is too long." });
      }
      const project = findProject(projects, input.project);
      if (!project) {
        ctx.wokeForNothing();
        const known = projects.map((p) => p.title).join(", ");
        return reply(input.project ? 404 : 400, { error: input.project ? `No project "${input.project}" here. Projects: ${known}` : `Say which project in "project": ${known}` });
      }
      let agent = null;
      if (input.model) {
        agent = pickModel(models, input.model, input.effort);
        if (agent.error) {
          ctx.wokeForNothing();
          return reply(400, { error: agent.error });
        }
      } else if (!project.hasModel) {
        agent = config.agent ?? null;
      }
      const until = limited();
      if (until) {
        ctx.wokeForNothing();
        return reply(429, { error: `${MAX_STARTS_PER_HOUR} starts and follow-ups an hour at most; the next one can go at ${until}.` });
      }
      ctx.touch(`start URL: ${project.title}`);
      await ctx.waitForT3();
      const title = input.title ? String(input.title).trim().slice(0, 120) : null;
      const worktree = input.worktree === true ? await worktreeBase(project.root, ctx.run) : null;
      const out = await ctx.run("t3", startArgs({ projectId: project.id, title, agent, worktree }), { input: prompt, cwd: project.root, timeoutMs: 180_000 });
      const started = JSON.parse(out);
      const threadId = started.threadId ?? started.id ?? started.thread?.id ?? started.thread?.threadId;
      if (!threadId) throw new Error("T3 started the thread but didn't say its id");
      starts.push(Date.now());
      const threads = readJson(THREADS_PATH, {});
      threads[threadId] = { projectId: project.id, at: new Date().toISOString() };
      writeJson(THREADS_PATH, Object.fromEntries(Object.entries(threads).slice(-KEEP_THREADS)));
      ctx.log(`start: ${project.title} → ${threadId}`);
      let link = null;
      try {
        link = JSON.parse(await ctx.run("t3", ["thread", "show", threadId, "--json", "--messages", "0"], { timeoutMs: 30_000 })).thread?.link ?? null;
      } catch {}
      return reply(201, {
        threadId,
        project: project.title,
        model: agent ? describeAgent(models, agent) : "the project's default",
        worktree: worktree ? `a new worktree from ${worktree}` : null,
        threadUrl: threadUrl(threadId),
        link,
      });
    } catch (e) {
      ctx.log(`start: ${e.message}`);
      return reply(500, { error: e.message.slice(0, 600) });
    }
  };
}

// --- t3agent37's command: turning the URL on or off -------------------------------------------

function turnOn({ token, url, agent, models }) {
  if (typeof token !== "string" || token.length < 32) throw new Error("a token of 32 characters or more");
  if (typeof url !== "string" || !url.startsWith("https://")) throw new Error("the inbox's https URL");
  writeJson(CONFIG_PATH, { token, url, agent: agent ?? null, models: models ?? [] });
  return {};
}

function turnOff() {
  rmSync(CONFIG_PATH, { force: true });
  return {};
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const [command] = process.argv.slice(2);
  let input = "";
  process.stdin.on("data", (d) => (input += d));
  process.stdin.on("end", () => {
    try {
      const args = JSON.parse(input || "{}");
      const result = command === "on" ? turnOn(args) : command === "off" ? turnOff() : null;
      if (!result) throw new Error(`unknown command ${command}`);
      console.log(JSON.stringify(result));
    } catch (e) {
      console.error(e.message);
      process.exit(1);
    }
  });
}
