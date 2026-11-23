// What a machine is doing, for the details in t3agent37: the threads its agents work on and what
// each is up to, what they finished lately, and the machine's load (CPU, memory, disk, the
// busiest processes). The watchdog runs it; the functions take T3's shell snapshot and /proc's
// text, so they're tested off the VM (`node --test activity.test.mjs`).

import { readFileSync, readdirSync, statfsSync } from "node:fs";

/** Runs that end a thread's turn, as T3 names them. */
const ENDED = new Set(["completed", "interrupted", "failed", "cancelled", "rolled_back"]);
const LIVE_STATUSES = new Set(["queued", "preparing", "starting", "running", "waiting"]);
const LIVE_MAX = 10;
/** Finished threads listed: the latest, from the last day. */
const RECENT_MAX = 5;
const RECENT_MS = 24 * 3600_000;
/** Processes listed: the busiest, from this share of one core up. */
const TOP_MAX = 5;
const TOP_MIN_CPU = 1;
const CLOCK_TICKS = 100; // USER_HZ, 100 on every Linux t3agent37 runs on
const PAGE_KB = 4;

/** Any agent run in flight, including one waiting on an approval: stopping would kill it. */
export function isLive(t) {
  return (
    t.activeRunId != null ||
    t.activityRunStatus != null ||
    LIVE_STATUSES.has(t.status) ||
    t.pendingRuntimeRequest != null ||
    (t.pendingBackgroundTasks?.length ?? 0) > 0
  );
}

const isSubagent = (t) => t.lineage?.relationshipToParent === "subagent";

/** What a live thread is up to, in a few words: "working", "needs approval", …. */
export function threadState(t) {
  const request = t.pendingRuntimeRequest;
  if (request && request.blocking !== false) {
    const kind = String(request.kind ?? "");
    if (kind === "user_input") return "asks you something";
    if (kind === "auth_refresh") return "needs a sign-in";
    if (/approval|permission/.test(kind)) return "needs your approval";
    return "waits for you";
  }
  const tasks = t.pendingBackgroundTasks ?? [];
  const status = t.activityRunStatus ?? t.status;
  if (status === "queued") return "queued";
  if (status === "preparing" || status === "starting") return "starting";
  if (status === "running") return "working";
  if (t.blockedOnThreadWait) return "waits on another thread";
  if (tasks.some((k) => k.kind === "subagent")) return "waits on subagents";
  if (tasks.length > 0) return "waits on a background task";
  return status === "waiting" ? "waiting" : "working";
}

const clip = (s, n) => {
  const one = String(s).replace(/\s+/g, " ").trim();
  return one.length > n ? `${one.slice(0, n - 1)}…` : one;
};

/**
 * The live top-level threads with what each is up to, and the latest finished ones (from the
 * last day), from T3's shell snapshot.
 */
export function summarizeThreads(threads, now = Date.now()) {
  const live = threads.filter(isLive);
  const subagentsOf = (id) =>
    live.filter((t) => isSubagent(t) && (t.lineage?.rootThreadId === id || t.lineage?.parentThreadId === id)).length;
  const working = live
    .filter((t) => !isSubagent(t))
    .slice(0, LIVE_MAX)
    .map((t) => ({
      id: String(t.id ?? ""),
      title: String(t.title ?? ""),
      projectId: t.projectId ?? null,
      provider: t.modelSelection?.instanceId ?? t.providerInstanceId ?? null,
      model: t.modelSelection?.model ?? null,
      options: (t.modelSelection?.options ?? []).filter((o) => o?.id && o.value != null).map((o) => `${o.id}=${o.value}`),
      state: threadState(t),
      startedAt: t.activityRunStartedAt ?? t.latestRunStartedAt ?? null,
      subagents: subagentsOf(t.id),
      // Commands it left running (a dev server, a watcher), not its subagents.
      background: (t.pendingBackgroundTasks ?? [])
        .filter((k) => k.kind !== "subagent")
        .map((k) => k.description ?? k.command)
        .filter(Boolean)
        .slice(0, 3)
        .map((s) => clip(s, 100)),
      branch: t.branch ?? null,
    }));
  const ended = (t) => Date.parse(t.latestRunCompletedAt ?? "");
  const recent = threads
    .filter((t) => !isSubagent(t) && !isLive(t) && ENDED.has(t.status) && !t.deletedAt && !t.archivedAt)
    .filter((t) => now - ended(t) < RECENT_MS)
    .sort((a, b) => ended(b) - ended(a))
    .slice(0, RECENT_MAX)
    .map((t) => ({
      id: String(t.id ?? ""),
      title: String(t.title ?? ""),
      projectId: t.projectId ?? null,
      status: t.latestRunStopRequested && t.status !== "failed" ? "stopped" : t.status,
      endedAt: t.latestRunCompletedAt,
      error: t.status === "failed" && t.lastError ? clip(t.lastError, 160) : null,
    }));
  return { working, recent };
}

/** CPU time (jiffies) and memory (pages) per process, from `/proc/<pid>/stat`. */
export function parseStat(text) {
  const open = text.indexOf("(");
  const close = text.lastIndexOf(")");
  if (open < 0 || close < open) return null;
  // Fields after the name, from the state (field 3): utime is field 14, stime 15, rss 24.
  const f = text.slice(close + 2).split(" ");
  const jiffies = Number(f[11]) + Number(f[12]);
  const rss = Number(f[21]);
  if (!Number.isFinite(jiffies)) return null;
  return { comm: text.slice(open + 1, close), jiffies, rss: Number.isFinite(rss) ? rss : 0 };
}

/** Every process's CPU time and memory now: pid → { comm, jiffies, rss }. */
export function readProcs(proc = "/proc") {
  const out = new Map();
  let names = [];
  try {
    names = readdirSync(proc);
  } catch {
    return out;
  }
  for (const pid of names) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      const s = parseStat(readFileSync(`${proc}/${pid}/stat`, "utf8"));
      if (s) out.set(pid, s);
    } catch {}
  }
  return out;
}

/** What a process is, for people: T3's own and t3agent37's by name, else its command line. */
export function describeProcess(comm, argv, home = "/root") {
  const line = argv.join(" ");
  if (line.includes(".t3agent37/watchdog.mjs")) return "t3agent37 watchdog";
  if (comm === "cloudflared") return "T3 Connect tunnel";
  if (/(^|\/)t3 (serve|__service-launcher)\b/.test(line) || /\bt3code\b.*\bserve\b/.test(line)) return "T3 server";
  if (/(^|\/)claude(\s|$)/.test(line) || /@anthropic-ai\/claude-code/.test(line)) return "Claude Code";
  if (/(^|\/)codex(\s|$)/.test(line) || /@openai\/codex/.test(line)) return "Codex";
  if (argv.length === 0) return comm;
  const base = (p) => p.split("/").pop();
  let [exe, ...rest] = argv;
  // `node …/.bin/next build` is "next build": the script, not its interpreter.
  const script = rest[0] && (rest[0].includes("/") || /\.(c|m)?[jt]s$|\.py$/.test(rest[0]));
  if (/^(node|bun|deno|python3?)$/.test(base(exe)) && script) [exe, ...rest] = rest;
  return clip([base(exe), ...rest].join(" ").split(home).join("~"), 80);
}

/**
 * The busiest processes between two `readProcs`, `secs` apart: their share of one core in
 * percent and their memory, busiest first.
 */
export function busiest(before, after, secs, cmdline = readCmdline) {
  if (!(secs > 0)) return [];
  const rows = [];
  for (const [pid, now] of after) {
    const then = before.get(pid);
    if (!then || then.comm !== now.comm) continue;
    const cpu = ((now.jiffies - then.jiffies) / CLOCK_TICKS / secs) * 100;
    if (cpu >= TOP_MIN_CPU) rows.push({ pid: Number(pid), comm: now.comm, cpu, rss: now.rss });
  }
  return rows
    .sort((a, b) => b.cpu - a.cpu)
    .slice(0, TOP_MAX)
    .map((r) => ({
      pid: r.pid,
      name: describeProcess(r.comm, cmdline(r.pid)),
      cpu: Math.round(r.cpu),
      memMb: Math.round((r.rss * PAGE_KB) / 1024),
    }));
}

function readCmdline(pid) {
  try {
    return readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0").filter(Boolean);
  } catch {
    return [];
  }
}

/** Memory in use and in all, in MB, from `/proc/meminfo`'s text. */
export function parseMeminfo(text) {
  const kb = (key) => Number(new RegExp(`^${key}:\\s+(\\d+)`, "m").exec(text)?.[1]);
  const total = kb("MemTotal");
  const available = kb("MemAvailable");
  if (!Number.isFinite(total) || !Number.isFinite(available)) return null;
  return { usedMb: Math.round((total - available) / 1024), totalMb: Math.round(total / 1024) };
}

/** The machine's load now: CPU (cores in use of all), memory, and the disk that persists. */
export function readLoad(cores, cpus) {
  let mem = null;
  try {
    mem = parseMeminfo(readFileSync("/proc/meminfo", "utf8"));
  } catch {}
  let disk = null;
  try {
    const s = statfsSync("/root");
    const gb = (blocks) => Math.round(((blocks * s.bsize) / 1024 ** 3) * 10) / 10;
    disk = { usedGb: gb(s.blocks - s.bfree), totalGb: gb(s.blocks) };
  } catch {}
  return {
    cores: Math.round(cores * 100) / 100,
    cpus,
    memUsedMb: mem?.usedMb ?? null,
    memTotalMb: mem?.totalMb ?? null,
    diskUsedGb: disk?.usedGb ?? null,
    diskTotalGb: disk?.totalGb ?? null,
  };
}
