import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { SITE } from "./src/links.ts";

/** Fills `%SITE%` in the pages with the site's address, so it is set in one place (src/links.ts). */
const site = (): Plugin => ({
  name: "site-address",
  transformIndexHtml: (html) => html.split("%SITE%").join(SITE),
});

// Two pages: the site, and demo.html, the panel on its mock backend (iframed by the site).
export default defineConfig({
  plugins: [react(), site()],
  build: {
    rollupOptions: {
      input: { index: "index.html", demo: "demo.html" },
    },
  },
});
