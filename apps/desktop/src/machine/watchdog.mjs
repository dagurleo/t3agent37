// t3agent37 watchdog. Runs on the Agent37 VM next to the T3 server and decides when the machine
// sleeps: once no agent works and nobody used it for `sleepAfterSecs`. It lives here, not on the
// laptop, so machines finish their work and then sleep even when the laptop is closed.
//
// Going to sleep, it "drains": it publishes a manifest of the final orchestration state so the
// laptop proxy can keep answering T3 Code truthfully while the machine sleeps. The proxy captures
// it and acks; without an ack (laptop gone) the watchdog stops after a grace period.
//
// It holds no Agent37 key (a workspace key could create and delete machines), so it cannot sleep
// the VM itself. Instead the VM runs with Agent37's auto-sleep on, which sleeps it once nothing
// moved through its URLs for a few minutes, and the watchdog keeps it awake by calling its own
// public heartbeat URL every minute until it decides to stop. Stopping, it goes quiet: the
// laptop app, if it runs, sleeps the VM right away through the API; otherwise auto-sleep does.

import { createServer } from "node:http";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { availableParallelism, homedir } from "node:os";
import { join } from "node:path";
import { startInbox } from "./issues.mjs";
import { busiest, isLive, readLoad, readProcs, readSteps, summarizeThreads } from "./activity.mjs";

const HOME = homedir();
const CONFIG_PATH = join(HOME, ".t3agent37", "config.json");
// On disk, so it survives watchdog restarts (upgrades); a stop ends it (see BOOT_ID).
const KEEP_AWAKE_PATH = join(HOME, ".t3agent37", "keep-awake-until");
// Awake time by UTC day and project: { "YYYY-MM-DD": { "<projectId>": secs, "": secs } }. A tick's
// time goes to the projects whose threads work then; idle time after work, to the projects that
// worked last since the wake (their turns are why it's still up); the rest to "" (waking, setup,
// a wake for nothing). Kept USAGE_KEEP_DAYS, saved every USAGE_SAVE_MS and before it sleeps.
const USAGE_PATH = join(HOME, ".t3agent37", "usage.json");
const USAGE_KEEP_DAYS = 62;
const USAGE_SAVE_MS = 30_000;
const MAX_KEEP_AWAKE_MINUTES = 24 * 60;
const DB_PATH = join(HOME, ".t3", "userdata", "statev2.sqlite");
const T3_ORIGIN = "http://127.0.0.1:3773";
const PORT = 3774;
// Agent37 routes this port's public URL here: every call to it counts as activity on the VM.
const HEARTBEAT_PORT = 3775;
const HEARTBEAT_MS = 60_000;
// The wake link's public URL routes here (see the server at the bottom).
const WAKE_PORT = 3776;
const TICK_MS = 5_000;
const DRAIN_GRACE_MS = 20_000;
// A tick that arrives this late means the VM was frozen: a warm resume after a stop.
const RESUMED_GAP_MS = 30_000;
// A wake for nothing (a GitHub delivery the inbox ignored, within BRIEF_WINDOW_MS of the wake):
// the machine sleeps again after BRIEF_SLEEP_MS idle instead of sleepAfterSecs, unless it's used.
const BRIEF_WINDOW_MS = 2 * 60_000;
const BRIEF_SLEEP_MS = 60_000;
// Still running this long after going quiet: neither the laptop app nor auto-sleep (idle 5 min)
// took it, so something else keeps traffic flowing. Back to active, and try again later.
const STOP_SETTLE_MS = 10 * 60_000;

const config = JSON.parse(readFileSync(CONFIG_PATH, "utf8"));
const log = (...args) => console.log(new Date().toISOString(), ...args);

/** @type {"active" | "draining" | "stopping"} */
let phase = "active";
/** Last time anything held the machine up, CPU load included: the idle countdown starts here. */
let lastActivityAt = Date.now();
/** Last time an agent, the user or a keep-awake held it up: CPU load holds it only so long after. */
let lastUsedAt = Date.now();
// The last wake, whether anything used the machine since, whether it's a brief one, and when the
// inbox last ignored a delivery (which can arrive before the tick notices the wake).
let wokeAt = 0;
// What woke it, for the details in t3agent37: the first use since the wake, or what came in that
// says so ("GitHub", "the start URL") even when it wasn't used after all. And the latest use.
let wokeFor = null;
let lastUse = null;
let usedSinceWake = true;
let briefWake = false;
let ignoredAt = 0;
let usage = (() => {
  try {
    return JSON.parse(readFileSync(USAGE_PATH, "utf8"));
  } catch {
    return {};
  }
})();
let usageSavedAt = Date.now();
let meteredAt = Date.now();
/** The projects the latest work since the wake was for. */
let lastProjects = [];
/** Agent runs in flight: top-level threads and subagents, and the top-level ones by id and title. */
let working = { threads: 0, subagents: 0, list: [], projects: [], detail: { working: [], recent: [] } };
// T3 not answering: an agent might be mid-turn, so count it as working, but only this long, so a
// broken server cannot keep the machine billing.
const UNKNOWN_BUSY_MS = 30 * 60_000;
let unknownSince = 0;
// T3 boots after this watchdog: until it first answers it is starting, not failing, and no agent
// can be mid-turn on it.
let t3Answered = false;
const unknownBusy = () => unknownSince > 0 && Date.now() - unknownSince < UNKNOWN_BUSY_MS;
const agentsWorking = () => working.threads + working.subagents > 0;
const agentsBusy = () => agentsWorking() || unknownBusy();

/**
 * One busy check for the tick, "Sleep now" and the delete guard. A failure starts (or continues)
 * "can't tell".
 */
async function refreshWorking() {
  try {
    working = await checkWorking();
    unknownSince = 0;
    // T3 answering for the first time since this watchdog started (a boot, an upgrade).
    if (!t3Answered) inbox.catchUp();
    t3Answered = true;
  } catch (e) {
    working = { threads: 0, subagents: 0, list: [], projects: [], detail: { working: [], recent: working.detail.recent } };
    if (t3Answered) unknownSince ||= Date.now();
    log(e.message);
  }
}
// A build or job started outside an agent turn (in the background, in a terminal) shows as CPU
// use; an idle dev server does not. Counted in cores, so a single-threaded build counts on any
// size, and smoothed over ~30s so a short spike doesn't count. It holds the machine up for at most
// CPU_HOLD_MS after the last use, so a busy dev server or a stray loop cannot keep it billing;
// longer jobs take a keep-awake.
const CPU_BUSY_CORES = 0.5;
const CPU_HOLD_MS = 30 * 60_000;
const CPU_COUNT = availableParallelism();
/** Cores in use. */
let cpuLoad = 0;
/** "Sleep now" was asked for: CPU load no longer holds the machine up, agents still do. */
let sleepRequested = false;
let cpuPrev = readCpu();
// The busiest processes over the last tick, for the details in t3agent37.
let procsPrev = readProcs();
let procsAt = Date.now();
let topProcs = [];
function sampleProcs() {
  const now = readProcs();
  topProcs = busiest(procsPrev, now, (Date.now() - procsAt) / 1000);
  procsPrev = now;
  procsAt = Date.now();
}
const cpuBusy = () => !sleepRequested && cpuLoad > CPU_BUSY_CORES && Date.now() - lastUsedAt < CPU_HOLD_MS;
const isBusy = () => agentsBusy() || cpuBusy();

/** Busy and total jiffies across all cores since boot, from /proc/stat. */
function readCpu() {
  const fields = readFileSync("/proc/stat", "utf8").split("\n")[0].trim().split(/\s+/).slice(1).map(Number);
  const idle = fields[3] + (fields[4] ?? 0);
  const total = fields.reduce((a, b) => a + b, 0);
  return { busy: total - idle, total };
}

function sampleCpu() {
  const now = readCpu();
  const total = now.total - cpuPrev.total;
  const usage = total > 0 ? ((now.busy - cpuPrev.busy) / total) * CPU_COUNT : 0;
  cpuPrev = now;
  cpuLoad = cpuLoad * 0.8 + usage * 0.2;
}
let manifest = null;
let drainStartedAt = 0;
let lastTickAt = Date.now();
let stopRequestedAt = 0;
/** "Keep awake": no sleeping before this (ms since epoch), working or not; Infinity = until stopped. */
// A stop ends any hold: after a cold resume (new boot) the stored one is void. A warm resume
// keeps the boot, so the resume check in tick() ends it there.
const BOOT_ID = readFileSafe("/proc/sys/kernel/random/boot_id").trim();
const [storedUntil, storedBoot] = readFileSafe(KEEP_AWAKE_PATH).split(" ");
let keepAwakeUntil = storedBoot === BOOT_ID ? Number(storedUntil) || 0 : 0;
const keptAwake = () => Date.now() < keepAwakeUntil;

function readFileSafe(path) {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "";
  }
}

/** null: until stopped; 0: off. */
function setKeepAwake(minutes) {
  keepAwakeUntil = minutes === null ? Infinity : minutes > 0 ? Date.now() + minutes * 60_000 : 0;
  writeFileSync(KEEP_AWAKE_PATH, `${keepAwakeUntil} ${BOOT_ID}`);
}

// Work T3 starts on its own at a set time: an automation, a webhook run it held back, an agent
// resuming after a usage limit. Due this soon, the machine waits for it instead of sleeping and
// waking again. Overdue this long, it is not coming (T3 runs due work within seconds), or the
// time stays behind in T3's rows after it ran.
const DUE_SOON_MS = 5 * 60_000;
const OVERDUE_MS = 2 * 60_000;
/** When T3 next starts work on its own, ms since epoch; null for never. t3agent37 wakes the machine for it. */
let nextDueAt = null;
const dueSoon = () => nextDueAt !== null && nextDueAt - Date.now() < DUE_SOON_MS;

// Events only a person causes, from any client: T3 Code through t3agent37, but also a phone or
// another computer through T3 Connect, which t3agent37 never sees.
const PERSON_EVENTS = [
  "thread.created",
  "thread.visit-recorded",
  "thread.visited",
  "thread.marked-unread",
  "thread.archived",
  "thread.unarchived",
  "thread.deleted",
  "thread.pinned",
  "thread.unpinned",
  "thread.pin-reordered",
  "thread.snoozed",
  "thread.unsnoozed",
  "thread.tags-updated",
  "thread.runtime-mode-updated",
  "thread.interaction-mode-updated",
  "thread.model-selection-updated",
];
/** Last orchestration event looked at; null until the first look. */
let eventCursor = null;

/**
 * Reads T3's database for the tick: sets `nextDueAt`, and says whether a person acted since the
 * last tick. A table this T3 version lacks has nothing due.
 */
function readT3State() {
  const db = new DatabaseSync(DB_PATH, { readOnly: true });
  try {
    const times = (sql) => {
      try {
        return db.prepare(sql).all().map((r) => (typeof r.t === "string" ? Date.parse(r.t) : r.t));
      } catch {
        return [];
      }
    };
    const recovering = `FROM orchestration_v2_projection_threads
      WHERE deleted_at IS NULL AND json_extract(payload_json, '$.limitRecovery.autoResume') = 1`;
    const due = [
      // Parsed here, not compared as text: T3 notes expanded years sort first.
      ...times(`SELECT next_run_at AS t FROM scheduled_tasks
        WHERE enabled = 1 AND next_run_at IS NOT NULL AND last_run_status <> 'running'`),
      ...times(`SELECT MIN(due_at) AS t FROM scheduled_task_webhook_pending WHERE state = 'pending'`),
      ...times(`SELECT json_extract(payload_json, '$.limitRecovery.resetAt') AS t ${recovering}`),
      ...times(`SELECT json_extract(payload_json, '$.snoozedUntil') AS t ${recovering}`),
    ].filter((t) => Number.isFinite(t) && Date.now() - t < OVERDUE_MS);
    nextDueAt = due.length > 0 ? Math.min(...due) : null;

    if (eventCursor === null) {
      eventCursor = db.prepare("SELECT MAX(sequence) AS s FROM orchestration_events").get()?.s ?? 0;
      return false;
    }
    const events = db
      .prepare(
        `SELECT MAX(sequence) AS last, SUM(event_type IN (${PERSON_EVENTS.map(() => "?").join(",")})) AS person
         FROM orchestration_events WHERE sequence > ?`,
      )
      .get(...PERSON_EVENTS, eventCursor);
    eventCursor = events?.last ?? eventCursor;
    return (events?.person ?? 0) > 0;
  } finally {
    db.close();
  }
}

/** The agents at work now, and what each is up to, from T3's shell snapshot. */
async function checkWorking() {
  const res = await fetch(`${T3_ORIGIN}/api/orchestration/shell`, {
    headers: { authorization: `Bearer ${config.t3Token}`, "x-t3-orchestration-protocol": "2" },
    signal: AbortSignal.timeout(10_000),
  });
  if (!res.ok) throw new Error(`shell snapshot: HTTP ${res.status}`);
  const shell = await res.json();
  const live = (shell.threads ?? []).filter(isLive);
  const top = live.filter((t) => t.lineage?.relationshipToParent !== "subagent");
  // Names the threads in t3agent37's "finished" notification.
  const list = top.slice(0, 10).map((t) => ({ id: String(t.id ?? ""), title: String(t.title ?? "") }));
  const projects = [...new Set(live.map((t) => String(t.projectId ?? "")).filter(Boolean))];
  const detail = summarizeThreads(shell.threads ?? []);
  if (detail.working.length > 0) {
    // What each is running and last said: T3's shell leaves both out.
    let db = null;
    try {
      db = new DatabaseSync(DB_PATH, { readOnly: true });
      for (const t of detail.working) Object.assign(t, readSteps(db, t.id));
    } catch {
    } finally {
      db?.close();
    }
  }
  return { threads: top.length, subagents: live.length - top.length, list, projects, detail };
}

/** The orchestration high-water marks T3 itself compares resume cursors against. */
function readManifest() {
  const db = new DatabaseSync(DB_PATH, { readOnly: true });
  try {
    const global = db
      .prepare(
        `SELECT MAX(sequence) AS s FROM orchestration_events
         WHERE aggregate_kind = 'project' OR (application_event_version = 2 AND aggregate_kind = 'thread')`,
      )
      .get();
    const rows = db
      .prepare(
        `SELECT stream_id AS id, MAX(sequence) AS s FROM orchestration_events
         WHERE application_event_version = 2 AND aggregate_kind = 'thread' GROUP BY stream_id`,
      )
      .all();
    return {
      sequence: global?.s ?? 0,
      threads: Object.fromEntries(rows.map((r) => [r.id, r.s])),
    };
  } finally {
    db.close();
  }
}

function drain() {
  try {
    manifest = readManifest();
  } catch (e) {
    log("manifest unavailable", e.message);
    manifest = null;
  }
  phase = "draining";
  drainStartedAt = Date.now();
  log("draining", manifest?.sequence);
}

/** Someone uses the machine: it stays up, and CPU load may hold it up again. */
function touch(reason) {
  lastUsedAt = Date.now();
  if (reason !== "resumed") {
    usedSinceWake = true;
    briefWake = false;
    noteWake(reason);
    lastUse = { reason, at: lastUsedAt };
  }
  hold(reason);
}

/** What came in, as the reason for the wake if it's the first thing since. */
function noteWake(reason) {
  if (wokeAt > 0 && wokeFor === null) wokeFor = reason;
}

/** The inbox ignored a delivery: if that's all that woke the machine, it sleeps again soon. */
function wokeForNothing() {
  ignoredAt = Date.now();
  if (!usedSinceWake && ignoredAt - wokeAt < BRIEF_WINDOW_MS && !briefWake) {
    briefWake = true;
    log("woken for nothing: sleeping again soon");
  }
}

/** Keeps the machine up and restarts the idle countdown, calling off a sleep in progress. */
function hold(reason) {
  sleepRequested = false;
  lastActivityAt = Date.now();
  if (phase !== "active") log(`back to active: ${reason}`);
  phase = "active";
  manifest = null;
}

/** Adds the time since the last tick to the projects it went to (see USAGE_PATH). */
function meter(now) {
  const gap = now - meteredAt;
  meteredAt = now;
  // A longer gap was a sleep: one tick of it was awake.
  const secs = (gap > RESUMED_GAP_MS ? TICK_MS : gap) / 1000;
  if (secs <= 0) return;
  if (working.projects.length) lastProjects = working.projects;
  const to = lastProjects.length ? lastProjects : [""];
  const day = (usage[new Date(now).toISOString().slice(0, 10)] ??= {});
  for (const p of to) day[p] = (day[p] ?? 0) + secs / to.length;
  if (now - usageSavedAt >= USAGE_SAVE_MS) saveUsage(now);
}

function saveUsage(now = Date.now()) {
  usageSavedAt = now;
  const oldest = new Date(now - USAGE_KEEP_DAYS * 86_400_000).toISOString().slice(0, 10);
  for (const day of Object.keys(usage)) if (day < oldest) delete usage[day];
  try {
    writeFileSync(`${USAGE_PATH}.tmp`, JSON.stringify(usage));
    renameSync(`${USAGE_PATH}.tmp`, USAGE_PATH);
  } catch (e) {
    log("saving usage", e.message);
  }
}

/** This UTC month's and today's awake seconds per project, for the app. */
function usageSummary(now) {
  const today = new Date(now).toISOString().slice(0, 10);
  const month = {};
  for (const [day, projects] of Object.entries(usage)) {
    if (!day.startsWith(today.slice(0, 7))) continue;
    for (const [p, secs] of Object.entries(projects)) month[p] = (month[p] ?? 0) + secs;
  }
  const whole = (o) => Object.fromEntries(Object.entries(o).map(([p, secs]) => [p, Math.round(secs)]));
  return { day: today, today: whole(usage[today] ?? {}), month: whole(month) };
}

/** Goes quiet so the VM can sleep: the laptop app sleeps it now, or auto-sleep in a few minutes. */
function stopSelf() {
  saveUsage();
  phase = "stopping";
  stopRequestedAt = Date.now();
  log("stopping: heartbeat off");
}

/**
 * Keeps Agent37's auto-sleep off the VM while the watchdog wants it up. Never awaited by the
 * loop: a slow edge must not stretch a tick into what looks like a resume (which ends a keep-awake).
 */
let lastHeartbeatAt = 0;
let heartbeatInFlight = false;
function heartbeat() {
  if (heartbeatInFlight || phase === "stopping" || !config.heartbeatUrl || Date.now() - lastHeartbeatAt < HEARTBEAT_MS) return;
  lastHeartbeatAt = Date.now();
  heartbeatInFlight = true;
  fetch(config.heartbeatUrl, { signal: AbortSignal.timeout(15_000) })
    .then((res) => res.arrayBuffer())
    .catch((e) => {
      log("heartbeat failed", e.message);
      lastHeartbeatAt = 0;
    })
    .finally(() => (heartbeatInFlight = false));
}

async function tick() {
  const now = Date.now();
  if (now - lastTickAt > RESUMED_GAP_MS) {
    setKeepAwake(0);
    wokeAt = now;
    // What arrived while the tick was frozen woke it.
    wokeFor = lastUse && lastUse.at > lastTickAt ? lastUse.reason : null;
    usedSinceWake = false;
    briefWake = now - ignoredAt < BRIEF_WINDOW_MS;
    lastProjects = [];
    touch("resumed");
    inbox.catchUp();
  }
  lastTickAt = now;
  meter(now);
  if (phase === "stopping") {
    if (now - stopRequestedAt > STOP_SETTLE_MS) return touch("still running after stop");
    // Until the VM actually sleeps, use from elsewhere (a phone through T3 Connect, an agent
    // starting on its own) calls the stop off: the heartbeat comes back with it.
    await refreshWorking();
    let acted = false;
    try {
      acted = readT3State();
    } catch {}
    if (agentsWorking()) touch("agent working");
    else if (acted) touch("used in T3");
    return;
  }

  sampleCpu();
  sampleProcs();
  await refreshWorking();
  let personActed = false;
  try {
    personActed = readT3State();
  } catch (e) {
    log("T3 state unavailable", e.message);
  }
  if (agentsWorking()) return touch("agent working");
  if (personActed) return touch("used in T3");
  if (keptAwake()) return touch("kept awake");
  if (cpuBusy()) return hold("cpu busy");
  if (dueSoon()) return hold("work due");
  // Can't tell: don't sleep, but it isn't activity either, so flaky checks can't hold it up forever.
  if (unknownBusy()) return;

  const idleMs = now - lastActivityAt;
  const sleepAfterMs = briefWake ? Math.min(BRIEF_SLEEP_MS, config.sleepAfterSecs * 1000) : config.sleepAfterSecs * 1000;
  // Not awaited: ticks go on during the save, so an agent starting to work calls it off.
  if (phase === "active" && idleMs >= sleepAfterMs) drain();
  else if (phase === "draining" && now - drainStartedAt >= DRAIN_GRACE_MS) finishDrain();
}

/** Stops, unless anything happened since the manifest: then someone uses the machine after all. */
function finishDrain() {
  let current = null;
  try {
    current = readManifest();
  } catch {}
  if (manifest && current && current.sequence !== manifest.sequence) return touch("new events");
  stopSelf();
}

function status() {
  return {
    phase,
    busy: isBusy(),
    cpuBusy: !agentsBusy() && cpuBusy(),
    agentsUnknown: unknownBusy(),
    workingThreads: working.threads,
    workingSubagents: working.subagents,
    working: working.list,
    keepAwakeForever: keepAwakeUntil === Infinity,
    keepAwakeSecs: Number.isFinite(keepAwakeUntil) ? Math.max(0, Math.ceil((keepAwakeUntil - Date.now()) / 1000)) : 0,
    idleSecs: Math.floor((Date.now() - lastActivityAt) / 1000),
    sleepAfterSecs: briefWake ? Math.min(BRIEF_SLEEP_MS / 1000, config.sleepAfterSecs) : config.sleepAfterSecs,
    usage: usageSummary(Date.now()),
    activity: {
      working: working.detail.working,
      recent: working.detail.recent,
      load: readLoad(cpuLoad, CPU_COUNT),
      top: topProcs,
      wokeAt: wokeAt || null,
      wokeFor,
      lastUse,
    },
    nextDueAt,
    workDue: dueSoon(),
    // While stopping, it names the sequence the machine stops at: the proxy trusts its
    // capture only when they match.
    manifest: phase === "active" ? null : manifest,
  };
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.on("data", (c) => {
      body += c;
      if (body.length > 64_000) req.destroy();
    });
    req.on("end", () => resolve(body));
    req.on("error", reject);
  });
}

const server = createServer(async (req, res) => {
  const reply = (code, value) => {
    res.writeHead(code, { "content-type": "application/json" });
    res.end(value === undefined ? "" : JSON.stringify(value));
  };
  if (req.headers.authorization !== `Bearer ${config.secret}`) return reply(401, { error: "unauthorized" });
  try {
    if (req.method === "GET" && req.url === "/status") return reply(200, status());
    if (req.method === "GET" && req.url === "/status?fresh") {
      // The delete guard: agents as of now, not as of the last tick.
      await refreshWorking();
      return reply(200, status());
    }
    if (req.method === "POST" && req.url === "/activity") {
      // { resumed }: the proxy just resumed the sandbox, so the stop asked for here is over. A
      // tick only notices a freeze of RESUMED_GAP_MS or more.
      const { resumed, by } = JSON.parse((await readBody(req)) || "{}");
      if (resumed) {
        setKeepAwake(0);
        // The app says why it woke the machine, if it knows.
        const reason = typeof by === "string" && by ? `app: ${by.slice(0, 60)}` : "resumed by the app";
        if (wokeAt === 0 || Date.now() - wokeAt > RESUMED_GAP_MS) {
          wokeAt = Date.now();
          wokeFor = null;
        }
        touch(reason);
        inbox.catchUp();
        return reply(200, status());
      }
      // Going quiet is not final: until the VM actually sleeps, use calls it off.
      touch("user activity");
      return reply(200, status());
    }
    if (req.method === "POST" && req.url === "/drain") {
      // "Sleep now" from the tray. Never mid-turn; it does end a keep-awake.
      await refreshWorking();
      // A busy CPU alone doesn't refuse: "Sleep now" is the user's explicit call.
      if (agentsBusy()) {
        const error = agentsWorking() ? "an agent is working" : "could not check agents";
        return reply(409, { error, ...status() });
      }
      setKeepAwake(0);
      if (phase === "active") drain();
      sleepRequested = true;
      return reply(200, status());
    }
    if (req.method === "POST" && req.url === "/keep-awake") {
      // { minutes }: null = until stopped, 0 ends it. Counted on this machine's clock, never the laptop's.
      const { minutes } = JSON.parse((await readBody(req)) || "{}");
      const valid = minutes === null || (Number.isInteger(minutes) && minutes >= 0 && minutes <= MAX_KEEP_AWAKE_MINUTES);
      if (!valid) return reply(400, { error: "minutes must be null (until stopped) or 0 to " + MAX_KEEP_AWAKE_MINUTES });
      setKeepAwake(minutes);
      if (minutes !== 0) touch("keep awake");
      return reply(200, status());
    }
    if (req.method === "POST" && req.url === "/issues/check") {
      // A repo was just turned on: issues labeled before that.
      inbox.catchUp();
      return reply(200, status());
    }
    if (req.method === "POST" && req.url === "/sleep-ack") {
      // The proxy captured this state: no need to wait out the grace period.
      const { sequence } = JSON.parse((await readBody(req)) || "{}");
      const acked = () => phase === "draining" && manifest && sequence === manifest.sequence;
      if (!acked()) return reply(200, status());
      // A run started since the last tick (an automation, not via the proxy) shows only here.
      await refreshWorking();
      if (agentsWorking()) touch("agent working");
      // Can't tell: the next tick decides. A hold during the check calls the stop off.
      else if (!unknownBusy() && acked()) finishDrain();
      return reply(200, status());
    }
    reply(404, { error: "not found" });
  } catch (e) {
    reply(400, { error: e.message });
  }
});

// On all interfaces: the VM has no public address, and Agent37's edge lets only requests with the
// workspace key through to it. The secret above is the second lock.
server.listen(PORT, "0.0.0.0", () => log(`watchdog on ${PORT}, sleep after ${config.sleepAfterSecs}s`));
// The heartbeat's target: answers anything, knows nothing.
createServer((_req, res) => res.writeHead(204).end()).listen(HEARTBEAT_PORT, "0.0.0.0");
// The wake link's target. Agent37 wakes a sleeping VM for any request to this port's public URL
// before passing it on; here it holds the machine up long enough to pick it in T3 Code, and says
// so. Anyone with the link can do that much and nothing more: it reads no input and serves no
// data. Using the machine still takes the user's own T3 Connect sign-in.
createServer((req, res) => {
  if (req.method !== "GET" && req.method !== "HEAD") return res.writeHead(405, { allow: "GET, HEAD" }).end();
  touch("wake link");
  res.writeHead(200, {
    "content-type": "text/html; charset=utf-8",
    "cache-control": "no-store",
    "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'",
    "referrer-policy": "no-referrer",
    "x-content-type-options": "nosniff",
    "x-robots-tag": "noindex",
  });
  res.end(req.method === "HEAD" ? undefined : wakePage());
}).listen(WAKE_PORT, "0.0.0.0");

// GitHub issues, for the repos the user turned them on for (issues.mjs). Its public URL is made
// with the first such repo; until then nothing reaches this port.
const inbox = startInbox({ touch, log, wokeForNothing, noteWake });

function wakePage() {
  const name = (readFileSafe(join(HOME, ".t3agent37", "name")).trim() || "Your machine").replace(
    /[&<>"']/g,
    (c) => `&#${c.charCodeAt(0)};`,
  );
  const minutes = Math.max(1, Math.round(config.sleepAfterSecs / 60));
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="apple-mobile-web-app-title" content="Wake ${name}">
<meta name="color-scheme" content="light dark">
<title>${name} is awake</title>
<style>
  :root { --bg: #fdfaf6; --fg: #1b1622; --muted: #7a6872; --accent: #c2581f; }
  @media (prefers-color-scheme: dark) { :root { --bg: #141117; --fg: #f6efe6; --muted: #c5a58a; --accent: #ffb547; } }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: var(--bg); color: var(--fg);
    font: 16px/1.45 -apple-system, system-ui, sans-serif; }
  main { padding: 24px; max-width: 360px; }
  h1 { margin: 0 0 8px; font-size: 26px; letter-spacing: -0.02em; }
  h1::before { content: ""; display: inline-block; width: 10px; height: 10px; margin-right: 10px; background: var(--accent); vertical-align: middle; }
  p { margin: 0; color: var(--muted); }
</style></head>
<body><main><h1>${name} is awake</h1>
<p>Open T3 Code and pick ${name}. It stays up for ${minutes} minutes, and for as long as you use it.</p></main></body></html>`;
}
// One tick at a time: a slow T3 check must never finish after a newer tick started a stop.
async function loop() {
  await tick().catch((e) => log("tick failed", e.message));
  heartbeat();
  setTimeout(loop, TICK_MS);
}
loop();
