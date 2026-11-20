// Whole pictures from the pixel parts: the rooftop strip and a row's agent icon. Pure
// drawing on a `t` clock, apart from the components that animate them.
import type { MachineState } from "../types";
import { seedOf } from "../machine";
import { drawSpy, poseFor, type Anchor } from "./spy";
import { palettes, type Theme } from "./palette";
import { CLEAR, hash, ramp, type Px } from "./px";
import { drawScene, type Rect } from "./scene";

/** The rooftop strip's size in CSS px. */
const STRIP_W = 380;
const STRIP_H = 148;
const MARGIN = 18; // scene px kept free of agents at either end
const SLOT = 19; // scene px an agent needs; fewer and the city zooms out, then scrolls
const SCROLL_SLOT = 24; // a scrolling roof spreads agents out: clearly wider, room to pace
/** Where the brand sits on the strip, in CSS px: the skyline stays low under it. */
const BRAND = { right: 132, bottom: 38 };

export interface Roof {
  /** CSS px per scene px: 2 close up … 1 zoomed out */
  scale: number;
  /** scene size in px */
  w: number;
  h: number;
  /** wider than the strip: it scrolls */
  scrolls: boolean;
  spots: { cx: number; wy: number; room: number }[];
}

/**
 * Zoom and room for `n` agents on a `dpr` screen: as close up as they fit, zooming out
 * only in steps that keep pixels crisp, then a roof wider than the strip.
 */
export function roofFor(n: number, dpr: number): Roof {
  const scales = Number.isInteger(1.5 * dpr) ? [2, 1.5, 1] : [2, 1];
  const need = 2 * MARGIN + n * SLOT;
  const scale = scales.find((s) => need <= Math.floor(STRIP_W / s)) ?? 1;
  const k = 2 / scale;
  const fit = Math.ceil(STRIP_W / scale);
  const scrolls = need > fit;
  const w = scrolls ? 2 * MARGIN + n * SCROLL_SLOT : fit;
  const slot = (w - 2 * MARGIN) / n;
  return {
    scale,
    w,
    h: Math.ceil(STRIP_H / scale),
    scrolls,
    spots: Array.from({ length: n }, (_, i) => ({
      cx: Math.round(MARGIN + slot * (i + 0.5)),
      wy: Math.round(k * (n === 1 ? 54 : 52 + (i % 2) * 4)),
      room: Math.max(0, Math.min(7, Math.floor((slot - 14) / 2))),
    })),
  };
}

/**
 * Draws `roof` with `agents` at `t` seconds (`null`: reduced motion's still frame) into a
 * `roof.w`×`roof.h` buffer. Returns where each agent's tag goes, in scene pixels.
 */
export function paintRoof(px: Px, roof: Roof, theme: Theme, agents: { key: string; state: MachineState }[], t: number | null) {
  const pal = palettes[theme];
  const brand: Rect = { x0: 0, y0: 0, x1: Math.ceil(BRAND.right / roof.scale), y1: Math.ceil(BRAND.bottom / roof.scale) };
  drawScene(px, 2 / roof.scale, pal, theme, t ?? 0, brand);
  const anchors = new Map<string, Anchor>();
  // back row first so nearer agents overlap farther ones
  const order = agents.map((a, i) => ({ a, s: roof.spots[i] })).sort((p, q) => p.s.wy - q.s.wy);
  for (const { a, s } of order) {
    anchors.set(a.key, drawSpy(px, s.cx, s.wy, pal, poseFor(a.state, t, seedOf(a.key), s.room)));
  }
  return anchors;
}

export const ICON_W = 24;
export const ICON_H = 21;
const FLOOR = 19;

/** Draws an `ICON_W`×`ICON_H` agent on its own patch of roof at `t` seconds (`null`: still frame). */
export function paintSpyIcon(px: Px, theme: Theme, state: MachineState, seed: number, t: number | null) {
  const pal = palettes[theme];
  px.fill(CLEAR);
  for (let y = FLOOR - 2; y < ICON_H; y++) {
    for (let x = 0; x < ICON_W; x++) {
      const edge = Math.min(x, ICON_W - 1 - x) / 5 - (y - FLOOR + 1) * 0.22;
      if (edge < hash(x, y, 1) * 0.6) continue;
      const idx = pal.roofTo - 1.4 + (y - FLOOR) * 0.4 + (hash(x, y, 2) - 0.5) * 0.6;
      px.set(x, y, ramp(pal.roof, idx / (pal.roof.length - 1), x, y));
    }
  }
  drawSpy(px, ICON_W / 2, FLOOR, pal, poseFor(state, t, seed, 0), 3);
}
