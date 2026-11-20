// After `vite build`: fills dist/index.html with the prerendered site, writes its markdown twin
// (index.md) plus llms.txt and llms-full.txt for agents. worker.ts serves the twin to agents at `/`.
import { htmlToMarkdown, llmsFullTxt, llmsTxt } from "@usenotra/agentside/build";
import { rm } from "node:fs/promises";
import { join } from "node:path";
import { SITE } from "../src/links";

const TITLE = "t3agent37";
const DESCRIPTION =
  "Run your T3 Code agents in the cloud, on your own Agent37 Cloud account. Machines sleep when idle and bill only disk, so you pay compute only for the minutes your agents run. macOS 13+.";

const web = join(import.meta.dir, "..");
const dist = join(web, "dist");
const ssr = join(web, "dist-ssr");

const { render } = await import(join(ssr, "prerender.js"));
await rm(ssr, { recursive: true });
const root = `<div id="root">${render()}</div>`;

const index = Bun.file(join(dist, "index.html"));
const html = await index.text();
if (!html.includes('<div id="root"></div>')) throw new Error('dist/index.html has no empty <div id="root">');
await Bun.write(index, html.replace('<div id="root"></div>', root));

const pages = [{ path: "/", title: TITLE, description: DESCRIPTION, markdown: htmlToMarkdown(root, { selector: "#root", siteUrl: SITE }) }];
await Bun.write(join(dist, "index.md"), pages[0].markdown);
await Bun.write(join(dist, "llms.txt"), llmsTxt(pages, { title: TITLE, description: DESCRIPTION, siteUrl: SITE }));
await Bun.write(join(dist, "llms-full.txt"), llmsFullTxt(pages, { siteUrl: SITE }));
