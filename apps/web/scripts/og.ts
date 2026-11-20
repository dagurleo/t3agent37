// Renders the share card (public/og.png) and its loop (public/og.mp4) from the hero itself.
// Needs the dev server (`bun dev`), Google Chrome and ffmpeg. Run: bun run og
import { chromium, type Page } from "playwright-core";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const URL = process.env.SITE ?? "http://localhost:5173/";
const W = 1200;
const H = 630;
/** the scene's own frame rate (pixel/clock.ts) */
const FPS = 12;
const SECONDS = 8;
const OUT = join(import.meta.dir, "../public");

/** The hero alone at card size, built to read small: wordmark, a big headline and the rooftop. */
const CARD_CSS = `
  .hero { height: ${H}px; min-height: 0; padding-bottom: 0; }
  .nav, .hero-copy { width: calc(100% - 128px); }
  .nav { height: 120px; }
  .hero-copy { padding-block: 40px 56px; }
  .hero h1 { font-size: 84px; max-width: none; }
  .roof-tag { font-size: 20px; padding: 4px 8px 3px; }
  .nav-links, .hero-copy p, .hero-cta { display: none; }
`;

async function open(page: Page) {
  await page.goto(URL);
  await page.addStyleTag({ content: CARD_CSS });
  await page.evaluate(() => document.fonts.ready);
}

// Google Chrome if installed, else playwright's own Chromium (`bunx playwright-core install chromium`)
const browser = await chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
const card = { viewport: { width: W, height: H }, colorScheme: "light" as const };

const still = await browser.newPage({ ...card, deviceScaleFactor: 2 });
await open(still);
await still.waitForTimeout(1000);
await still.screenshot({ path: join(OUT, "og.png") });

// frame by frame on a fake clock, so every scene tick lands in the video
const video = await browser.newPage(card);
await video.clock.install();
await open(video);
await video.clock.runFor(1000);
const frames = await mkdtemp(join(tmpdir(), "og-"));
for (let i = 0; i < FPS * SECONDS; i++) {
  await video.clock.runFor(1000 / FPS);
  await video.screenshot({ path: join(frames, `${String(i).padStart(4, "0")}.png`) });
}
await browser.close();

const ffmpeg = Bun.spawnSync([
  "ffmpeg", "-y", "-loglevel", "error",
  "-framerate", String(FPS), "-i", join(frames, "%04d.png"),
  "-r", "24", "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
  join(OUT, "og.mp4"),
], { stdio: ["ignore", "inherit", "inherit"] });
await rm(frames, { recursive: true });
if (!ffmpeg.success) process.exit(1);
