// The rooftop strip: a dithered sky over a city skyline with a searchlight, and a gravel
// roof fading into the page.
import type { Palette, Theme } from "./palette";
import { hash, ramp, bayer, type Px } from "./px";

/**
 * Zoom-out of a scene: 1 = the close-up strip, 2 = twice as much city in the same space.
 * Distances grow with it, things keep their pixel size.
 */
export type Zoom = number;

/** In the close-up strip: the skyline's base row, and the rows of roof below it. */
export const HORIZON = 32;
export const ROOF = 42;

/** A block of scene px, ends exclusive. */
export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/**
 * A scene laid out by its host rather than from one zoom (the site's hero): the horizon on row
 * `horizon`, the sky plain above `skyTop` and its gradient from there down, the roof below at
 * `zoom`, and the sun and moon where given.
 */
export interface Frame {
  horizon: number;
  skyTop: number;
  zoom: Zoom;
  sun?: { x: number; y: number };
  moon?: { x: number; y: number };
  /** the highest row clouds reach: they drift below it, clear of what's above */
  cloudTop?: number;
  /** share of the night sky's px that are stars (the strip's is 0.016) */
  stars?: number;
}

/** Where things sit in a `w`-wide scene at `zoom`, or as `frame` lays it out. */
function geometry(w: number, at: Zoom | Frame) {
  const frame = typeof at === "number" ? null : at;
  const k = frame ? frame.zoom : (at as Zoom);
  const horizon = frame ? frame.horizon : Math.round(HORIZON * k);
  const skyTop = frame?.skyTop ?? 0;
  return {
    horizon,
    skyTop,
    deep: horizon + Math.round(22 * k), // darkest roof row; below it the roof lightens into the page
    dissolve: Math.round(7 * k), // bottom rows that dither away into the page background
    haze: skyTop + Math.round(21 * k), // haze bands drift below this row
    sun: frame?.sun ?? { x: Math.round(122 * k), y: horizon - 8 },
    moon: frame?.moon ?? { x: Math.round(112 * k), y: Math.round(10 * k) },
    searchlight: w - Math.round(30 * k), // the lamp's x, on the building it stands on
    stars: frame?.stars ?? 0.016,
    cloudTop: frame?.cloudTop ?? null,
  };
}
type Geometry = ReturnType<typeof geometry>;

/** The searchlight's building: px wide, px tall; the lamp sits on it. */
const LIGHT_W = 13;
const LIGHT_H = 13;

interface Building {
  x0: number;
  x1: number; // exclusive
  h: number;
  i: number;
}

/** A row of buildings across `w` px, each shorter than `top` allows above `calm`. */
function skyline(w: number, horizon: number, seed: number, minH: number, maxH: number, minW: number, maxW: number, calm?: Rect): Building[] {
  const out: Building[] = [];
  for (let x = -3, i = 0; x < w; i++) {
    const bw = minW + Math.floor(hash(i, seed) * (maxW - minW + 1));
    let h = minH + Math.floor(hash(i, seed + 1) * (maxH - minH + 1));
    if (calm && x < calm.x1) h = Math.min(h, horizon - calm.y1 - 1);
    out.push({ x0: x, x1: x + bw, h: Math.max(h, 2), i });
    x += bw + (hash(i, seed + 2) > 0.7 ? 1 : 0);
  }
  return out;
}

/** Sky px per cloud. */
const CLOUD_AREA = 4500;

/** Flat-bottomed puffs drifting right, spread down the sky, smaller and slower towards the horizon. */
function drawClouds(px: Px, g: Geometry, pal: Palette, time: number) {
  const { w } = px;
  // a frame's clouds keep below its cloudTop: their bases start a puff's height under it
  const from = g.cloudTop === null ? g.skyTop + 8 : g.cloudTop + 13;
  const rows = g.horizon - 6 - from;
  if (rows <= 0) return;
  const count = Math.max(1, Math.round((w * rows) / CLOUD_AREA));
  for (let i = 0; i < count; i++) {
    const base = from + Math.round(((i + hash(i, 41)) / count) * rows);
    const near = 1 - (0.5 * base) / g.horizon;
    const width = Math.round((20 + hash(i, 42) * 22) * near);
    const speed = (0.5 + hash(i, 43) * 0.6) * near;
    const period = w + 2 * width;
    const cx = ((hash(i, 44) * period + time * speed) % period) - width;
    const puffs = [0, 1, 2, 3].map((k) => ({
      x: cx + ((k + 0.5) / 4 - 0.5) * width * 0.75,
      r: width * (0.13 + hash(i, k, 45) * 0.1 + (k === 1 || k === 2 ? 0.07 : 0)),
    }));
    const tall = Math.max(...puffs.map((p) => p.r));
    for (let y = Math.max(Math.floor(base - tall), g.cloudTop ?? 0); y <= base; y++) {
      const v = base - y;
      for (let x = Math.floor(cx - width / 2); x <= Math.ceil(cx + width / 2); x++) {
        if (!puffs.some((p) => (x - p.x) ** 2 + v ** 2 <= p.r ** 2)) continue;
        px.set(x, y, ramp(pal.cloud, 1 - v / (tall * 0.7), x, y));
      }
    }
  }
}

function skyT(g: Geometry, x: number, y: number, time: number, theme: Theme, beam: number): number {
  let t = Math.pow(Math.max(0, y - g.skyTop) / (g.horizon - g.skyTop), theme === "light" ? 2.6 : 1.3);

  // drifting haze bands, like long exposure clouds
  if (y > g.haze) {
    const len = 12 + hash(y, 1) * 22;
    const seg = Math.floor((x + time * (0.6 + hash(y, 2)) + hash(y, 3) * 400) / len);
    if (y % 3 !== 0 && hash(seg, y, 7) > 0.78) t += theme === "light" ? 0.22 : 0.12;
  }

  if (theme === "light") {
    const d = Math.hypot((x - g.sun.x) / 1.8, y - g.sun.y);
    if (d < 16) t = Math.max(t, 1.05 - d / 22);
  } else {
    const d = Math.hypot(x - g.moon.x, y - g.moon.y);
    if (d < 11) t += (1 - d / 11) * 0.35;
    // the searchlight's beam, swinging across the sky
    const lampY = g.horizon - LIGHT_H - 3;
    const dx = x - g.searchlight;
    const dy = y - lampY;
    const dist = Math.hypot(dx, dy);
    if (dy <= 0 && dist > 3 && dist < 140) {
      let a = Math.atan2(dy, dx) - beam;
      a = Math.atan2(Math.sin(a), Math.cos(a));
      if (Math.abs(a) < 0.045 + 1.5 / (dist + 30)) t += 0.55 * (1 - dist / 140);
    }
  }
  return t;
}

/**
 * Fills `px` with the scene at `zoom`. Buildings under `calm` (the brand on the sky) stay
 * short enough to leave it clear.
 */
export function drawScene(px: Px, zoom: Zoom | Frame, pal: Palette, theme: Theme, time: number, calm?: Rect) {
  const { w, h } = px;
  const g = geometry(w, zoom);
  const { horizon, sun, moon } = g;
  const beam = Math.sin(time * 0.4) - Math.PI / 2;

  for (let y = 0; y < horizon; y++) {
    for (let x = 0; x < w; x++) px.set(x, y, ramp(pal.sky, skyT(g, x, y, time, theme, beam), x, y));
  }

  if (theme === "light") {
    if (pal.cloud.length) drawClouds(px, g, pal, time);
    for (let y = sun.y - 3; y <= sun.y + 3; y++)
      for (let x = sun.x - 3; x <= sun.x + 3; x++) if (Math.hypot(x - sun.x, y - sun.y) <= 2.6) px.set(x, y, pal.glow[0]);
  } else {
    for (let y = 0; y < horizon - 8; y++)
      for (let x = 0; x < w; x++) {
        const s = hash(x, y, 3);
        if (s > 1 - g.stars) {
          const twinkle = hash(x, y, Math.floor(time * 1.5 + s * 40)) > 0.8;
          px.set(x, y, pal.star[twinkle ? 2 : s > 1 - g.stars * 0.3 ? 0 : 1]);
        }
      }
    for (let y = moon.y - 5; y <= moon.y + 5; y++)
      for (let x = moon.x - 5; x <= moon.x + 5; x++) {
        const d = Math.hypot(x - moon.x, y - moon.y);
        if (d <= 4.2) px.set(x, y, Math.hypot(x - moon.x - 1.6, y - moon.y + 1) < 2 && d > 1 ? pal.glow[1] : pal.glow[0]);
      }
  }

  // far skyline: a few lit windows, the odd antenna
  for (const b of skyline(w, horizon, 10, 9, 21, 6, 12, calm)) {
    for (let x = b.x0; x < b.x1; x++)
      for (let y = horizon - b.h; y <= horizon; y++) {
        const lit = (x - b.x0) % 2 === 1 && x < b.x1 - 1 && (horizon - y) % 3 === 1 && horizon - y > 1 && hash(x, y, 77) > 0.7;
        px.set(x, y, lit ? pal.farWindow : pal.far);
      }
    if (hash(b.i, 33) > 0.75) for (let k = 1; k <= 4; k++) px.set((b.x0 + b.x1) >> 1, horizon - b.h - k, pal.far);
  }
  // near skyline: windows that come on and go off
  for (const b of skyline(w, horizon, 20, 3, 12, 5, 10, calm)) {
    for (let x = b.x0; x < b.x1; x++)
      for (let y = horizon - b.h; y <= horizon; y++) {
        const isWindow = (x - b.x0) % 2 === 1 && x < b.x1 - 1 && (horizon - y) % 2 === 0 && horizon - y > 1 && y > horizon - b.h + 1;
        const lit = isWindow && hash(x, y, Math.floor(time / 5 + hash(x, y, 5) * 9)) > 0.6;
        px.set(x, y, isWindow ? (lit ? pal.windowLit : pal.window) : pal.near);
      }
  }

  // the searchlight on its building; by day an aircraft light blinks on its mast instead
  const L = g.searchlight;
  const x0 = L - 6;
  const top = horizon - LIGHT_H;
  for (let x = x0; x < x0 + LIGHT_W; x++)
    for (let y = top; y <= horizon; y++) {
      const isWindow = (x - x0) % 3 === 1 && (horizon - y) % 3 === 1 && y > top + 2;
      px.set(x, y, isWindow && hash(x, y, 9) > 0.5 ? pal.window : pal.tower);
    }
  for (let x = x0 - 1; x <= x0 + LIGHT_W; x++) px.set(x, top, pal.tower);
  const lampOn = theme === "dark";
  for (let x = L - 1; x <= L + 1; x++) {
    px.set(x, top - 1, pal.tower);
    px.set(x, top - 2, lampOn ? pal.lamp : pal.tower);
  }
  px.set(L, top - 3, lampOn ? pal.lamp : pal.tower);
  if (!lampOn) {
    const mast = x0 + LIGHT_W - 3;
    for (let k = 1; k <= 6; k++) px.set(mast, top - k, pal.tower);
    if (time % 2 < 0.9) px.set(mast, top - 7, pal.beacon);
  }

  // the roof: a ledge, then gravel with tar seams fanning out towards you
  for (let x = 0; x < w; x++) {
    px.set(x, horizon + 1, pal.ledge);
    px.set(x, horizon + 2, pal.ledgeShade);
  }
  const span = pal.roofTo - pal.roofFrom;
  const last = pal.roof.length - 1;
  for (let y = horizon + 3; y < h; y++) {
    const d = Math.min((y - horizon) / (g.deep - horizon), 1);
    const near = y > g.deep ? (y - g.deep) / (h - 1 - g.deep) : 0;
    const base = pal.roofFrom + span * Math.pow(d, 0.8) + (pal.roofFade - pal.roofTo) * Math.pow(near, 1.4);
    const fade = (y - (h - 1 - g.dissolve)) / g.dissolve;
    for (let x = 0; x < w; x++) {
      if (y === h - 1) {
        px.set(x, y, pal.bg);
        continue;
      }
      if (fade > 0) {
        const jag = (hash(x >> 2, 4) - 0.5) * 0.5 + (hash(x, y, 4) - 0.5) * 0.25;
        if (fade + jag > bayer(x, y)) {
          px.set(x, y, pal.bg);
          continue;
        }
      }
      const seam = Math.round(x + ((x - w / 2) * (y - horizon)) / 30);
      const idx = base + (seam % 38 === 0 ? 0.6 : 0) + (hash(x, y, 2) - 0.5) * 0.3;
      px.set(x, y, ramp(pal.roof, idx / last, x, y));
    }
  }

  // fixtures at either end of the roof: a vent pipe, an AC unit with its fan turning
  for (let y = horizon + 2; y <= horizon + 7; y++) {
    px.set(7, y, pal.tower);
    px.set(8, y, pal.tower);
  }
  for (let x = 5; x <= 10; x++) px.set(x, horizon + 2, pal.tower);
  for (let x = w - 17; x <= w - 7; x++)
    for (let y = horizon + 4; y <= horizon + 10; y++) px.set(x, y, y === horizon + 4 ? pal.ledge : pal.near);
  const spin = Math.floor(time * 8) % 2;
  const blades = spin ? [[-1, -1], [1, 1], [-1, 1], [1, -1]] : [[0, -1], [0, 1], [-1, 0], [1, 0]];
  for (const [dx, dy] of blades) px.set(w - 12 + dx, horizon + 7 + dy, pal.ledge);
}
