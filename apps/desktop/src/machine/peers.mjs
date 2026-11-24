// Links from the user's other machines, served by the inbox (issues.mjs). T3 links one server to
// another (its "linked environments"): the linking machine's agents then work here with T3's own
// tools (delegate_task, the thread tools). t3agent37 makes those links between the user's machines,
// as the user allows each one, and they reach this machine's T3 through the inbox's public URL,
// so a call from another machine wakes this one.
//
// Only T3's link endpoints pass, and only while t3agent37 has said this machine takes links
// (peers.json, { accept: true }); T3 checks the pairing code, then the link's token, itself.
// OAuth's `resource` is taken out of the decision and the token request: T3 issues tokens only for
// the address a request reaches it at, which behind Agent37's edge isn't the public one, and given
// none it issues them for itself (the desktop's proxy does the same, proxy/link.rs). /mcp answers
// stream (a delegated task's follower waits up to 50 s a call), so they're passed through as they
// come, unread.

import { request } from "node:http";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PEERS_PATH = join(homedir(), ".t3agent37", "peers.json");
const T3 = { host: "127.0.0.1", port: 3773 };
/** Headers that are about this hop, not the request. */
const HOP = new Set(["host", "connection", "keep-alive", "transfer-encoding", "content-length", "upgrade", "te", "trailer", "proxy-authorization", "proxy-connection"]);

/** Whether `url` is one of T3's link endpoints. */
export function isPeerPath(url) {
  const path = String(url ?? "").split("?")[0];
  return path === "/.well-known/t3/environment" || path === "/mcp" || /^\/oauth\/mcp\/(register|decision|token)$/.test(path);
}

/** Whether t3agent37 has said this machine takes links from the user's other machines. */
export function accepting(path = PEERS_PATH) {
  try {
    return JSON.parse(readFileSync(path, "utf8"))?.accept === true;
  } catch {
    return false;
  }
}

/** A link step's body without OAuth's `resource`; other bodies as they are. */
export function withoutResource(path, contentType, body) {
  const type = String(contentType ?? "");
  if (path.startsWith("/oauth/mcp/decision") && type.includes("json")) {
    const d = JSON.parse(body.toString("utf8"));
    if (d?.authorization && typeof d.authorization === "object") delete d.authorization.resource;
    return Buffer.from(JSON.stringify(d));
  }
  if (path.startsWith("/oauth/mcp/token")) {
    if (type.includes("json")) {
      const d = JSON.parse(body.toString("utf8"));
      delete d.resource;
      return Buffer.from(JSON.stringify(d));
    }
    const form = new URLSearchParams(body.toString("utf8"));
    form.delete("resource");
    return Buffer.from(form.toString());
  }
  return body;
}

/**
 * Passes a link request to T3 here and its answer back as it comes. `noteWake` says what woke the
 * machine; `wokeForNothing` lets a machine woken only to be looked at (a list of links checks
 * each one) sleep again soon: the work a link asks for keeps it up itself (the watchdog sees the
 * agent).
 */
export async function handlePeer(req, body, res, { waitForT3, noteWake, wokeForNothing, log }) {
  if (!accepting()) {
    wokeForNothing();
    res.writeHead(404).end();
    return;
  }
  const path = req.url ?? "/";
  if (path.startsWith("/mcp")) noteWake("another of your machines");
  else wokeForNothing();
  let sent;
  try {
    sent = withoutResource(path, req.headers["content-type"], body);
  } catch {
    res.writeHead(400).end();
    return;
  }
  try {
    // Just after a cold boot T3 may still be starting; T3 gives a waking peer 10 s.
    await waitForT3(20_000);
  } catch {
    res.writeHead(503, { "retry-after": "10" }).end();
    return;
  }
  const headers = Object.fromEntries(Object.entries(req.headers).filter(([name]) => !HOP.has(name)));
  if (sent.length) headers["content-length"] = String(sent.length);
  const up = request({ ...T3, method: req.method, path, headers }, (answer) => {
    res.writeHead(answer.statusCode ?? 502, answer.headers);
    answer.pipe(res);
  });
  up.on("error", (e) => {
    log(`peers: passing ${req.method} ${path.split("?")[0]} to T3: ${e.message}`);
    if (!res.headersSent) res.writeHead(502);
    res.end();
  });
  // A caller that goes away mid-answer (it gave up waiting) ends the request to T3 too.
  res.on("close", () => up.destroy());
  up.end(sent.length ? sent : undefined);
}
