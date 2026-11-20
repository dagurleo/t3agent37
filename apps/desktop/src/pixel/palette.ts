import { rgb, type Color } from "./px";

export type Theme = "light" | "dark";

/** One field agent's outfit, hat to shoes. */
export interface SpyColors {
  hat: Color;
  band: Color;
  face: Color;
  faceShade: Color;
  glasses: Color;
  coat: Color;
  shirt: Color;
  legs: Color;
  shoes: Color;
  briefcase: Color;
}

/** Noir amber: plum and amber dusk by day, charcoal and sodium-lit windows by night. */
export interface Palette {
  bg: Color;
  sky: Color[]; // top → horizon
  roof: Color[]; // lightest → darkest
  roofFrom: number; // ramp index just past the ledge
  roofTo: number; // ramp index in the deepest row
  roofFade: number; // ramp index the near roof fades toward (≈ page background)
  ledge: Color;
  ledgeShade: Color;
  far: Color; // distant skyline
  farWindow: Color;
  near: Color; // nearer skyline
  window: Color;
  windowLit: Color;
  tower: Color; // the searchlight's building and the roof's fixtures
  lamp: Color; // searchlight at night
  beacon: Color; // aircraft light by day
  glow: Color[]; // sun / moon, inner → outer
  star: Color[];
  cloud: Color[]; // lit top → shaded underside
  crate: Color;
  crateShade: Color;
  rope: Color;
  alarm: Color;
  zzz: Color;
  spy: SpyColors;
  spyDim: SpyColors; // asleep, gone, or not heard from
}

const spy = (c: Record<keyof SpyColors, string>): SpyColors =>
  Object.fromEntries(Object.entries(c).map(([k, v]) => [k, rgb(v)])) as unknown as SpyColors;

const light: Palette = {
  bg: rgb("#fdfaf6"),
  sky: ["#2a2235", "#3b2c45", "#5a3b52", "#8a4f57", "#c26f55", "#e9a46a", "#f6d3a4", "#fbeedd"].map(rgb),
  roof: ["#fdfaf6", "#f3e7da", "#e2cdb8", "#c5a58a", "#9b7a68", "#6e5560", "#4a3a4f"].map(rgb),
  roofFrom: 2,
  roofTo: 4.4,
  roofFade: 0.6,
  ledge: rgb("#f3e7da"),
  ledgeShade: rgb("#6e5560"),
  far: rgb("#8a6f80"),
  farWindow: rgb("#a98d98"),
  near: rgb("#3b2c45"),
  window: rgb("#5a3b52"),
  windowLit: rgb("#ffcf7a"),
  tower: rgb("#2a2235"),
  lamp: rgb("#fff7e8"),
  beacon: rgb("#e0452f"),
  glow: ["#fff7e8", "#fbeedd", "#f6d3a4"].map(rgb),
  star: [],
  cloud: ["#fdfaf6", "#fbeedd", "#f6d3a4", "#e9a46a"].map(rgb),
  crate: rgb("#c5a58a"),
  crateShade: rgb("#9b7a68"),
  rope: rgb("#6e5560"),
  alarm: rgb("#e0452f"),
  zzz: rgb("#2a2235"),
  spy: spy({
    hat: "#1b1622",
    band: "#c26f55",
    face: "#f6e3cf",
    faceShade: "#d9b99a",
    glasses: "#0d0b10",
    coat: "#2a2235",
    shirt: "#fdfaf6",
    legs: "#1b1622",
    shoes: "#0d0b10",
    briefcase: "#5a3b2a",
  }),
  spyDim: spy({
    hat: "#4a3a4f",
    band: "#9b7a68",
    face: "#efe0d2",
    faceShade: "#c5a58a",
    glasses: "#4a3a4f",
    coat: "#6e5560",
    shirt: "#f3e7da",
    legs: "#4a3a4f",
    shoes: "#33293a",
    briefcase: "#6e5560",
  }),
};

const dark: Palette = {
  bg: rgb("#141117"),
  sky: ["#0c0a0e", "#141117", "#141117", "#1c1720", "#261e2b", "#352736", "#4a3140"].map(rgb),
  roof: ["#f6d3a4", "#c5a58a", "#9b7a68", "#6e5560", "#4a3a4f", "#33293a", "#241d29", "#18141b"].map(rgb),
  roofFrom: 4,
  roofTo: 6.2,
  roofFade: 7,
  ledge: rgb("#6e5560"),
  ledgeShade: rgb("#0c0a0e"),
  far: rgb("#1f1925"),
  farWindow: rgb("#3b2c45"),
  near: rgb("#0c0a0e"),
  window: rgb("#1c1720"),
  windowLit: rgb("#ffb547"),
  tower: rgb("#0c0a0e"),
  lamp: rgb("#ffe2a8"),
  beacon: rgb("#ff6a4d"),
  glow: ["#fbeedd", "#f6d3a4", "#8a4f57"].map(rgb),
  star: ["#fbeedd", "#c5a58a", "#6e5560"].map(rgb),
  cloud: [],
  crate: rgb("#4a3a4f"),
  crateShade: rgb("#33293a"),
  rope: rgb("#6e5560"),
  alarm: rgb("#ff6a4d"),
  zzz: rgb("#f6d3a4"),
  spy: spy({
    hat: "#0c0a0e",
    band: "#c26f55",
    face: "#f6e3cf",
    faceShade: "#c5a58a",
    glasses: "#000000",
    coat: "#c9a77c",
    shirt: "#fdfaf6",
    legs: "#0c0a0e",
    shoes: "#000000",
    briefcase: "#0c0a0e",
  }),
  spyDim: spy({
    hat: "#33293a",
    band: "#6e5560",
    face: "#c5a58a",
    faceShade: "#9b7a68",
    glasses: "#33293a",
    coat: "#8a7466",
    shirt: "#c5a58a",
    legs: "#241d29",
    shoes: "#18141b",
    briefcase: "#33293a",
  }),
};

export const palettes: Record<Theme, Palette> = { light, dark };
