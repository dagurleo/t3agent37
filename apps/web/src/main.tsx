import { StrictMode } from "react";
import { createRoot, hydrateRoot } from "react-dom/client";
import "./panel/brand.css";
import "./site.css";
import { Site } from "./Site";

const root = document.getElementById("root")!;
const site = (
  <StrictMode>
    <Site />
  </StrictMode>
);
// the build prerenders the site into index.html (scripts/prerender.ts); the dev server doesn't
if (import.meta.env.DEV) createRoot(root).render(site);
else hydrateRoot(root, site);
