// node --test activity.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { busiest, describeProcess, isLive, parseMeminfo, parseStat, summarizeThreads, threadState } from "./activity.mjs";

const NOW = Date.parse("2026-11-23T10:00:00Z");
const thread = (over) => ({
  id: "t1",
  title: "Fix the login",
  projectId: "p1",
  status: "completed",
  activeRunId: null,
  activityRunStatus: null,
  pendingRuntimeRequest: null,
  pendingBackgroundTasks: [],
  lineage: { parentThreadId: null, relationshipToParent: null, rootThreadId: "t1" },
  modelSelection: { instanceId: "claudeAgent", model: "claude-opus-5-5", options: [{ id: "effort", value: "high" }] },
  ...over,
});

test("what a live thread is up to", () => {
  assert.equal(threadState(thread({ activityRunStatus: "running" })), "working");
  assert.equal(threadState(thread({ activityRunStatus: "starting" })), "starting");
  assert.equal(threadState(thread({ pendingRuntimeRequest: { kind: "command_approval" } })), "needs your approval");
  assert.equal(threadState(thread({ pendingRuntimeRequest: { kind: "user_input" } })), "asks you something");
  // A question the run keeps working past doesn't hold it.
  assert.equal(threadState(thread({ activityRunStatus: "running", pendingRuntimeRequest: { kind: "user_input", blocking: false } })), "working");
  assert.equal(threadState(thread({ activityRunStatus: "waiting", pendingBackgroundTasks: [{ kind: "subagent", taskId: "a" }] })), "waits on subagents");
  assert.equal(threadState(thread({ activityRunStatus: "waiting", pendingBackgroundTasks: [{ kind: "command", taskId: "b" }] })), "waits on a background task");
  assert.equal(threadState(thread({ activityRunStatus: "waiting", blockedOnThreadWait: true })), "waits on another thread");
});

test("live threads with their subagents and background work, and the latest finished", () => {
  const threads = [
    thread({ id: "a", title: "Working one", status: "running", activeRunId: "r", activityRunStatus: "running", activityRunStartedAt: "2026-11-23T09:50:00Z",
      pendingBackgroundTasks: [{ kind: "command", taskId: "x", command: "bun run   dev" }, { kind: "subagent", taskId: "y" }], branch: "t3/x" }),
    thread({ id: "s", title: "sub", status: "running", activityRunStatus: "running", lineage: { relationshipToParent: "subagent", rootThreadId: "a", parentThreadId: "a" } }),
    thread({ id: "b", title: "Done an hour ago", latestRunCompletedAt: "2026-11-23T09:00:00Z" }),
    thread({ id: "c", title: "Failed just now", status: "failed", lastError: "Usage limit\nreached", latestRunCompletedAt: "2026-11-23T09:58:00Z" }),
    thread({ id: "d", title: "Stopped", status: "interrupted", latestRunStopRequested: true, latestRunCompletedAt: "2026-11-23T09:30:00Z" }),
    thread({ id: "e", title: "Two days ago", latestRunCompletedAt: "2026-11-21T09:00:00Z" }),
    thread({ id: "f", title: "Archived", archivedAt: "2026-11-23T09:59:00Z", latestRunCompletedAt: "2026-11-23T09:59:00Z" }),
  ];
  const { working, recent } = summarizeThreads(threads, NOW);
  assert.deepEqual(working, [
    {
      id: "a", title: "Working one", projectId: "p1", provider: "claudeAgent", model: "claude-opus-5-5", options: ["effort=high"],
      state: "working", startedAt: "2026-11-23T09:50:00Z", subagents: 1, background: ["bun run dev"], branch: "t3/x",
    },
  ]);
  assert.deepEqual(recent.map((r) => [r.id, r.status, r.error]), [
    ["c", "failed", "Usage limit reached"],
    ["d", "stopped", null],
    ["b", "completed", null],
  ]);
  assert.ok(threads.filter(isLive).length === 2);
});

test("a process's CPU time and memory from /proc/<pid>/stat", () => {
  // A name with spaces and parentheses, as the kernel writes it.
  const stat = "4242 (node (dev) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 11 0 777 1000000 2048 rest";
  assert.deepEqual(parseStat(stat), { comm: "node (dev) x", jiffies: 300, rss: 2048 });
  assert.equal(parseStat("garbage"), null);
});

test("the busiest processes over a tick", () => {
  const before = new Map([
    ["10", { comm: "node", jiffies: 1000, rss: 25600 }],
    ["11", { comm: "cloudflared", jiffies: 50, rss: 1000 }],
    ["12", { comm: "sleep", jiffies: 5, rss: 10 }],
  ]);
  const after = new Map([
    ["10", { comm: "node", jiffies: 1900, rss: 25600 }], // 900 jiffies in 5 s: 180% of a core
    ["11", { comm: "cloudflared", jiffies: 60, rss: 1000 }], // 2%
    ["12", { comm: "sleep", jiffies: 5, rss: 10 }], // idle: left out
    ["13", { comm: "new", jiffies: 999, rss: 10 }], // not there before: left out
  ]);
  const argv = { 10: ["/usr/bin/node", "/root/app/node_modules/.bin/next", "build"], 11: ["cloudflared", "tunnel"] };
  const top = busiest(before, after, 5, (pid) => argv[pid] ?? []);
  assert.deepEqual(top, [
    { pid: 10, name: "next build", cpu: 180, memMb: 100 },
    { pid: 11, name: "T3 Connect tunnel", cpu: 2, memMb: 4 },
  ]);
});

test("names for T3's own processes", () => {
  assert.equal(describeProcess("node", ["node", "/root/.t3agent37/watchdog.mjs"]), "t3agent37 watchdog");
  assert.equal(describeProcess("node-MainThread", ["/root/.t3/runtime/0.1.12/t3", "serve"]), "T3 server");
  assert.equal(describeProcess("claude", ["/root/.local/bin/claude", "--output-format", "stream-json"]), "Claude Code");
  assert.equal(describeProcess("node", ["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js", "proto"]), "Codex");
  assert.equal(describeProcess("kworker/0:1", []), "kworker/0:1");
  assert.equal(describeProcess("bun", ["/root/.bun/bin/bun", "run", "dev"]), "bun run dev");
  assert.equal(describeProcess("python3", ["python3", "-m", "http.server"]), "python3 -m http.server");
});

test("memory in use from /proc/meminfo", () => {
  const text = "MemTotal:        8126464 kB\nMemFree:         3909632 kB\nMemAvailable:    6836224 kB\n";
  assert.deepEqual(parseMeminfo(text), { usedMb: 1260, totalMb: 7936 });
  assert.equal(parseMeminfo("nothing"), null);
});
