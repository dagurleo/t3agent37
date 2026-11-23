// The inbox's rules for feedback on pull requests. `cargo test` runs this through node --test
// (issues.rs), and so does `node --test src/machine/issues.test.mjs` in apps/desktop.

import { test } from "node:test";
import assert from "node:assert/strict";
import { asksAgent, issueComments, reviewCounts, ciVerdict, actionsAsRuns, roundsToday, feedbackPrompt, signedWith, MARK, MAX_ROUNDS_PER_DAY } from "./issues.mjs";
import { createHmac } from "node:crypto";

test("a comment asks for the agent with a line starting /agent", () => {
  assert.ok(asksAgent("/agent fix the tests"));
  assert.ok(asksAgent("Looks close.\n  /agent rename the flag too"));
  assert.ok(asksAgent("/AGENT do it"));
  assert.ok(!asksAgent("ask the /agent about it"));
  assert.ok(!asksAgent("/agents are great"));
  assert.ok(!asksAgent("Nice work!"));
  assert.ok(!asksAgent(undefined));
  // The machine's own words never count, even quoting someone.
  assert.ok(!asksAgent(`Done.\n/agent fix the tests\n\n${MARK}`));
});

test("reviews that say something count, approvals and the machine's own don't", () => {
  assert.ok(reviewCounts({ state: "COMMENTED", body: "" }));
  assert.ok(reviewCounts({ state: "CHANGES_REQUESTED", body: "Please split this" }));
  assert.ok(!reviewCounts({ state: "APPROVED", body: "LGTM" }));
  assert.ok(!reviewCounts({ state: "DISMISSED", body: "x" }));
  assert.ok(!reviewCounts({ state: "COMMENTED", body: `Fixed.\n\n${MARK}` }));
});

test("CI is judged once every check run finished", () => {
  const run = (status, conclusion, name = "test") => ({ status, conclusion, name });
  assert.deepEqual(ciVerdict([]), { state: "none" });
  assert.equal(ciVerdict([run("completed", "success"), run("in_progress", null)]).state, "pending");
  assert.equal(ciVerdict([run("completed", "success"), run("completed", "skipped")]).state, "passed");
  const failed = ciVerdict([run("completed", "success", "lint"), run("completed", "failure", "test"), run("completed", "timed_out", "e2e")]);
  assert.equal(failed.state, "failed");
  assert.deepEqual(failed.failed.map((r) => r.name), ["test", "e2e"]);
});

test("with a fine-grained token, CI comes from GitHub Actions' runs", () => {
  const runs = actionsAsRuns([
    { name: "CI", status: "completed", conclusion: "failure", html_url: "https://github.com/o/r/actions/runs/1", head_sha: "abc" },
    { name: "Lint", status: "completed", conclusion: "success", html_url: "u2" },
  ]);
  const verdict = ciVerdict(runs);
  assert.equal(verdict.state, "failed");
  assert.deepEqual(verdict.failed.map((r) => [r.name, r.html_url]), [["CI", "https://github.com/o/r/actions/runs/1"]]);
  assert.equal(ciVerdict(actionsAsRuns([{ name: "CI", status: "in_progress", conclusion: null }])).state, "pending");
  assert.deepEqual(actionsAsRuns(undefined), []);
});

test("a pull request gets so many rounds a day", () => {
  const now = Date.parse("2026-11-22T12:00:00Z");
  const hoursAgo = (h) => new Date(now - h * 3_600_000).toISOString();
  const some = roundsToday([hoursAgo(30), hoursAgo(5), hoursAgo(1)], now);
  assert.equal(some.recent.length, 2);
  assert.ok(!some.full);
  const many = roundsToday(Array.from({ length: MAX_ROUNDS_PER_DAY }, (_, i) => hoursAgo(20 - i)), now);
  assert.ok(many.full);
  assert.equal(new Date(many.freeAt).toISOString(), "2026-11-22T16:00:00.000Z");
  assert.ok(!roundsToday(undefined, now).full);
});

test("feedback goes to the thread quoted, in order, with what to do", () => {
  const text = feedbackPrompt({
    repo: "octocat/hello",
    number: 4,
    url: "https://github.com/octocat/hello/pull/4",
    branch: "issue-3",
    items: [
      { kind: "review", user: "ana", state: "CHANGES_REQUESTED", body: "Close, but:", comments: [{ path: "src/a.ts", line: 12, body: "Use the helper\nfrom utils" }] },
      { kind: "comment", user: "ben", body: "/agent also update the README" },
    ],
    ci: { sha: "0123456789abcdef", failed: [{ name: "test", url: "https://example.com/run/1" }] },
  });
  assert.match(text, /@ana reviewed it and asked for changes:\n> Close, but:\nOn `src\/a.ts` line 12:\n> Use the helper\n> from utils/);
  assert.match(text, /@ben commented:\n> \/agent also update the README/);
  assert.match(text, /CI failed on 0123456, the branch's latest commit:\n- test \(https:\/\/example.com\/run\/1\)/);
  assert.match(text, /gh pr checks 4 --repo octocat\/hello/);
  assert.match(text, /gh pr comment 4 --repo octocat\/hello --body-file -/);
  assert.ok(text.indexOf("@ana") < text.indexOf("@ben"));
  assert.ok(text.includes(MARK));
  // Without CI, no CI step.
  const quiet = feedbackPrompt({ repo: "o/r", number: 1, url: "u", branch: "b", items: [{ kind: "comment", user: "x", body: "/agent hi" }], ci: null });
  assert.ok(!quiet.includes("gh pr checks"));
});

test("deliveries are checked against the webhook's secret", () => {
  const body = Buffer.from('{"zen":"hi"}');
  const sig = `sha256=${createHmac("sha256", "s3cret").update(body).digest("hex")}`;
  assert.ok(signedWith("s3cret", body, sig));
  assert.ok(!signedWith("other", body, sig));
  assert.ok(!signedWith("s3cret", body, undefined));
  assert.ok(!signedWith("", body, sig));
});

test("an issue's comments go to its thread, without the machine's own", () => {
  assert.equal(issueComments([]), "");
  assert.equal(issueComments(undefined), "");
  const c = (login, body, author_association = "NONE") => ({ user: { login }, body, author_association });
  const text = issueComments([
    c("ana", "Also update the README.", "OWNER"),
    c("bot", `**skiff** is on it.\n\n${MARK}`),
    c("lee", "+1\nsecond line"),
    c("kim", "   "),
  ]);
  assert.match(text, /^Its comments so far, oldest first:/);
  assert.match(text, /@ana \(owner\) commented:\n> Also update the README\./);
  assert.match(text, /@lee commented:\n> \+1\n> second line/);
  assert.ok(!text.includes("skiff") && !text.includes("@kim"));
  assert.ok(text.indexOf("@ana") < text.indexOf("@lee"));
  // The latest that fit; the rest are counted.
  const many = Array.from({ length: 40 }, (_, i) => c("u", `comment ${i}`));
  const latest = issueComments(many, 140);
  assert.match(latest, /\(110 earlier ones left out\)/);
  assert.ok(latest.includes("comment 39") && latest.includes("comment 10") && !latest.includes("comment 9\n"));
  const long = issueComments([c("u", "x".repeat(10_000))]);
  assert.ok(long.length < 4_300 && long.includes("[…the rest left out]"));
});
