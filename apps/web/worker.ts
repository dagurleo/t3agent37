// t3agent37.dagurleo.com: the prerendered site in dist/, served as Workers static assets. This
// Worker runs first for pages and small files (run_worker_first in wrangler.jsonc). It does what
// t3boat's vercel.json did for agents:
// - `/` answers with its Markdown twin (index.md, from scripts/prerender.ts) when the client asks
//   for text/markdown, or when it's a known AI agent;
// - pages get a Link header pointing at that twin;
// - .md files are served as Markdown and kept out of search indexes.

type Env = { ASSETS: { fetch: (input: Request | URL) => Promise<Response> } };

/** AI agents that get the Markdown twin at `/` without asking for it, as on t3boat's site. */
const AGENTS =
  /ChatGPT-User|OAI-SearchBot|GPTBot|Claude-User|Claude-SearchBot|ClaudeBot|Claude-Web|Anthropic-ai|Perplexity-User|PerplexityBot|MistralAI-User|GoogleOther|Applebot-Extended|Meta-ExternalAgent|Meta-ExternalFetcher|DuckAssistBot|YouBot|cohere-ai|Amazonbot|Bytespider|CCBot|Diffbot|ImagesiftBot|Omgilibot/i;

/** Whether the Accept header asks for Markdown (and doesn't refuse it with q=0). */
function asksMarkdown(accept: string | null) {
  if (!accept) return false;
  for (const part of accept.split(",")) {
    const [type = "", ...params] = part.split(";").map((item) => item.trim());
    if (type.toLowerCase() !== "text/markdown") continue;
    const q = params.find((param) => param.toLowerCase().startsWith("q="));
    return q ? Number(q.slice(2)) > 0 : true;
  }
  return false;
}

const MARKDOWN = {
  "content-type": "text/markdown; charset=utf-8",
  vary: "Accept, User-Agent",
  "x-robots-tag": "noindex",
};

function withHeaders(response: Response, set: Record<string, string>) {
  const headers = new Headers(response.headers);
  for (const [name, value] of Object.entries(set)) headers.set(name, value);
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

export default {
  async fetch(request: Request, env: Env) {
    const url = new URL(request.url);
    const read = request.method === "GET" || request.method === "HEAD";

    if (read && url.pathname === "/") {
      const agent = asksMarkdown(request.headers.get("accept")) || AGENTS.test(request.headers.get("user-agent") ?? "");
      if (agent) {
        const twin = await env.ASSETS.fetch(new URL("/index.md", url));
        if (twin.ok) return withHeaders(twin, MARKDOWN);
      }
    }

    const response = await env.ASSETS.fetch(request);
    if (!response.ok) return response;
    if (url.pathname.endsWith(".md")) return withHeaders(response, MARKDOWN);
    if (url.pathname === "/" && response.headers.get("content-type")?.startsWith("text/html"))
      return withHeaders(response, {
        vary: "Accept, User-Agent",
        link: '</index.md>; rel="alternate"; type="text/markdown"',
      });
    return response;
  },
};
