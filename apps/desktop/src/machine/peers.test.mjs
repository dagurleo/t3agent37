// The inbox's rules for links from the user's other machines (peers.mjs). `cargo test` runs this
// through node --test (issues.rs).

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { accepting, isPeerPath, withoutResource } from "./peers.mjs";

test("only T3's link endpoints pass", () => {
  for (const path of ["/.well-known/t3/environment", "/mcp", "/mcp?x=1", "/oauth/mcp/register", "/oauth/mcp/decision", "/oauth/mcp/token"]) {
    assert.ok(isPeerPath(path), path);
  }
  for (const path of ["/", "/api/orchestration/shell", "/oauth/mcp/authorize", "/mcpx", "/.well-known/t3", "/start/abc", undefined]) {
    assert.ok(!isPeerPath(path), String(path));
  }
});

test("the address T3 checks is taken out of the link's steps", () => {
  const decision = { authorization: { client_id: "c", resource: "https://x.agent37.app/mcp", state: "s" }, decision: { _tag: "pairing-code", code: "abc" } };
  const out = JSON.parse(withoutResource("/oauth/mcp/decision", "application/json", Buffer.from(JSON.stringify(decision))));
  assert.deepEqual(out, { authorization: { client_id: "c", state: "s" }, decision: { _tag: "pairing-code", code: "abc" } });
  const form = withoutResource("/oauth/mcp/token", "application/x-www-form-urlencoded", Buffer.from("grant_type=authorization_code&code=k&resource=https%3A%2F%2Fx%2Fmcp"));
  assert.equal(form.toString(), "grant_type=authorization_code&code=k");
  const json = JSON.parse(withoutResource("/oauth/mcp/token", "application/json", Buffer.from('{"code":"k","resource":"r"}')));
  assert.deepEqual(json, { code: "k" });
  // Everything else passes as it came.
  const mcp = Buffer.from('{"jsonrpc":"2.0","method":"tools/call","params":{"resource":"kept"}}');
  assert.equal(withoutResource("/mcp", "application/json", mcp), mcp);
  assert.throws(() => withoutResource("/oauth/mcp/decision", "application/json", Buffer.from("not json")));
});

test("links pass only once t3agent37 has said the machine takes them", () => {
  const dir = mkdtempSync(join(tmpdir(), "peers-"));
  const path = join(dir, "peers.json");
  assert.equal(accepting(path), false);
  writeFileSync(path, '{"accept": false}');
  assert.equal(accepting(path), false);
  writeFileSync(path, '{"accept": true}');
  assert.equal(accepting(path), true);
  writeFileSync(path, "{nonsense");
  assert.equal(accepting(path), false);
});
