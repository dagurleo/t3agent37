/**
 * The site's own address (a Cloudflare Worker, wrangler.jsonc). index.html (through vite.config.ts),
 * scripts/prerender.ts and the launch video read it from this one place.
 */
export const SITE = "https://t3agent37.dagurleo.com";

export const REPO = "https://github.com/dagurleo/t3agent37";
/** The newest release's DMG: every release has one under this name (apps/desktop/scripts/release-macos.sh). */
export const DOWNLOAD = `${REPO}/releases/latest/download/t3agent37.dmg`;
export const T3CODE = "https://t3.codes";
export const AGENT37 = "https://www.agent37.com";
/** t3agent37 is a fork of t3boat, by Pau Kraft. */
export const T3BOAT = "https://github.com/paukraft/t3boat";
