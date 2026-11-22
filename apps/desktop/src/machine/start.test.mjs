// The start URL's rules. `cargo test` runs this through node --test (issues.rs), and so does
// `node --test src/machine/start.test.mjs` in apps/desktop.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseStartPath, isStartPath, bearer, sameToken, findProject, pickModel, describeAgent, startArgs, startsLeft, threadReport, MAX_STARTS_PER_HOUR } from "./start.mjs";

test("start URLs and their thread URLs", () => {
  assert.deepEqual(parseStartPath("/start/abc"), { token: "abc" });
  assert.deepEqual(parseStartPath("/start/abc/"), { token: "abc" });
  assert.deepEqual(parseStartPath("/start/abc?x=1"), { token: "abc" });
  assert.deepEqual(parseStartPath(`/start/abc/threads/${encodeURIComponent("thread:project:p:1")}`), { token: "abc", threadId: "thread:project:p:1" });
  assert.deepEqual(parseStartPath("/start/abc/threads/"), { token: "abc", bad: true });
  assert.deepEqual(parseStartPath("/start/abc/other"), { token: "abc", bad: true });
  assert.deepEqual(parseStartPath("/start/abc/threads/%E0%A4%A"), { token: "abc", bad: true });
  assert.equal(parseStartPath("/api/hooks/x/y"), null);
  assert.equal(parseStartPath("/startle"), null);
  // The token in a header instead: the address and its thread URLs carry none.
  assert.deepEqual(parseStartPath("/start"), { token: null });
  assert.deepEqual(parseStartPath("/start/?x=1"), { token: null });
  assert.deepEqual(parseStartPath(`/start/threads/${encodeURIComponent("mcp:1")}`), { token: null, threadId: "mcp:1" });
  assert.deepEqual(parseStartPath("/start/threads"), { token: null, bad: true });
  assert.ok(isStartPath("/start") && isStartPath("/start/abc") && !isStartPath("/starts") && !isStartPath(undefined));
  assert.equal(bearer("Bearer abc-123_x"), "abc-123_x");
  assert.equal(bearer("bearer  abc "), "abc");
  assert.equal(bearer("Basic abc"), null);
  assert.equal(bearer("Bearer a b"), null);
  assert.equal(bearer(undefined), null);
  assert.ok(sameToken("s3cret", "s3cret"));
  assert.ok(!sameToken("s3cret", "s3cre"));
  assert.ok(!sameToken("", ""));
  assert.ok(!sameToken(undefined, "x"));
});

test("a project is named by its id, title or folder", () => {
  const projects = [
    { id: "p1", title: "Tantalum", root: "/root/tantalum" },
    { id: "p2", title: "T3 37", root: "/root/t3agent37" },
  ];
  assert.equal(findProject(projects, "p2")?.title, "T3 37");
  assert.equal(findProject(projects, " tantalum ")?.id, "p1");
  assert.equal(findProject(projects, "t3agent37")?.id, "p2");
  assert.equal(findProject(projects, "cork"), null);
  assert.equal(findProject(projects, ""), null);
  assert.equal(findProject(projects, undefined), null);
});

const models = [
  { provider: "claudeAgent", providerName: "Claude", model: "claude-opus-5-5", name: "Claude Opus 5.5", aliases: ["opus"], effort: { id: "effort", levels: [["low", "Low"], ["high", "High"], ["max", "Max"]], default: "high" } },
  { provider: "codex", providerName: "Codex", model: "gpt-6", name: "GPT-6", aliases: [], effort: null },
];

test("a model is picked among the machine's, with a level it takes", () => {
  assert.deepEqual(pickModel(models, "Opus 5.5", "Max"), { provider: "claudeAgent", model: "claude-opus-5-5", options: ["effort=max"] });
  assert.deepEqual(pickModel(models, "opus"), { provider: "claudeAgent", model: "claude-opus-5-5", options: [] });
  assert.deepEqual(pickModel(models, "GPT-6"), { provider: "codex", model: "gpt-6", options: [] });
  assert.match(pickModel(models, "gpt-6", "high").error, /no reasoning levels/);
  assert.match(pickModel(models, "opus", "ultra").error, /takes low, high, max/);
  assert.match(pickModel(models, "sonnet").error, /claude-opus-5-5, gpt-6/);
  assert.equal(describeAgent(models, { provider: "claudeAgent", model: "claude-opus-5-5", options: ["effort=high"] }), "Opus 5.5, high");
  assert.equal(describeAgent(models, { provider: "codex", model: "gpt-6", options: [] }), "GPT-6");
  assert.equal(describeAgent([], { provider: "x", model: "y", options: ["effort=low"] }), "y, low");
  assert.equal(describeAgent(models, null), null);
});

test("what t3 thread start is given", () => {
  assert.deepEqual(startArgs({ projectId: "p1", title: null, agent: null, worktree: null }), ["thread", "start", "p1", "-", "--json", "--generate-title"]);
  assert.deepEqual(startArgs({ projectId: "p1", title: "Fix login", agent: { provider: "claudeAgent", model: "claude-opus-5-5", options: ["effort=high"] }, worktree: "origin/main" }), [
    "thread", "start", "p1", "-", "--json", "--title", "Fix login",
    "--provider", "claudeAgent", "--model", "claude-opus-5-5", "--option", "effort=high",
    "--worktree", "origin/main",
  ]);
});

test("so many starts an hour", () => {
  const now = Date.parse("2026-11-22T12:00:00Z");
  const minutesAgo = (m) => now - m * 60_000;
  const some = startsLeft([minutesAgo(90), minutesAgo(30), minutesAgo(1)], now);
  assert.equal(some.recent.length, 2);
  assert.ok(!some.full);
  const many = startsLeft(Array.from({ length: MAX_STARTS_PER_HOUR }, (_, i) => minutesAgo(50 - i)), now);
  assert.ok(many.full);
  assert.equal(new Date(many.freeAt).toISOString(), "2026-11-22T12:10:00.000Z");
  assert.ok(!startsLeft(undefined, now).full);
});

test("a thread's report: the reply once it's done", () => {
  const shown = (status, extra = {}) => ({
    thread: { threadId: "t1", title: "Boat facts", status, activeRunId: null, pendingRequestCount: 0, link: "https://app.t3.codes/threads/e/t1", ...extra },
    messages: [
      { role: "user", text: "make it" },
      { role: "assistant", text: "Made it." },
      { role: "user", text: "and a button" },
      { role: "assistant", text: "Working on it" },
      { role: "assistant", text: "Added the button." },
    ],
  });
  const done = threadReport(shown("completed"), "https://x/start/t/threads/t1");
  assert.deepEqual(done, { threadId: "t1", title: "Boat facts", status: "completed", done: true, waitingForYou: false, reply: "Added the button.", link: "https://app.t3.codes/threads/e/t1", threadUrl: "https://x/start/t/threads/t1" });
  const busy = threadReport(shown("running", { activeRunId: "r2" }), "u");
  assert.equal(busy.done, false);
  assert.equal(busy.reply, null);
  const asking = threadReport(shown("waiting", { pendingRequestCount: 1 }), "u");
  assert.ok(asking.waitingForYou && !asking.done);
  // A follow-up not answered yet: no reply of its own, and not done, even before its run starts.
  const sent = threadReport({ thread: { status: "completed" }, messages: [{ role: "assistant", text: "old" }, { role: "user", text: "new" }] }, "u");
  assert.equal(sent.reply, null);
  assert.equal(sent.done, false);
  // A run that failed is done, with no reply.
  assert.equal(threadReport({ thread: { status: "failed" }, messages: [{ role: "user", text: "go" }] }, "u").done, true);
});
