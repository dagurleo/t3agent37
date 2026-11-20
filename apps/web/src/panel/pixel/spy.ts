// Procedural pixel field agent: hat, sunglasses, trench coat. Local sprite space: 8px wide,
// 14px tall, facing right; the feet stand on the roof row `wy`, and the rows go up from it.
import type { MachineState } from "../types";
import type { Palette, SpyColors } from "./palette";
import { bayer, type Color, type Px } from "./px";

type Frame = "stand" | "walkA" | "walkB" | "perch" | "sit";

export interface Pose {
  frame: Frame;
  dx: number;
  flip: boolean;
  dim: boolean;
  ghost: boolean;
  drop: number; // 0 on the roof … 1 still up out of sight, coming down a rope
  rope: boolean;
  alarm: boolean; // a red "!" over the hat
  zzz: number[]; // rise 0…1 of each z drifting off a sleeping agent
}

const BASE: Pose = { frame: "stand", dx: 0, flip: false, dim: false, ghost: false, drop: 0, rope: false, alarm: false, zzz: [] };

const clamp01 = (v: number) => Math.min(Math.max(v, 0), 1);
const ease = (v: number) => 1 - (1 - v) * (1 - v);
/** 0→1 over `rise` seconds, then hold until `period` and repeat. */
const loop = (t: number, rise: number, period: number) => ease(clamp01((t % period) / rise));

/** Rappelling down takes this long, and repeats this often while a machine is created. */
const DROP_SECS = 3;
const DROP_PERIOD = 4.2;

/**
 * @param t seconds; `null` = reduced motion (a single telling still frame)
 * @param seed per-agent phase offset
 * @param room horizontal px the agent may pace either way while working
 */
export function poseFor(state: MachineState, t: number | null, seed: number, room: number): Pose {
  const still = t === null;
  const time = (t ?? 0) + seed * 7.3;
  switch (state) {
    case "working": {
      if (still) return { ...BASE, frame: "walkA" };
      const phase = (time * 2 * Math.PI) / 14;
      return {
        ...BASE,
        frame: Math.floor(time * 4) % 2 ? "walkA" : "walkB",
        dx: Math.round(Math.sin(phase) * room),
        flip: Math.cos(phase) < 0,
      };
    }
    case "idle":
      // now and then glances back over the shoulder
      return { ...BASE, flip: !still && time % 7 > 5 };
    case "sleeping": {
      const p = still ? 0.5 : loop(time, 1.8, 2.6);
      return { ...BASE, frame: p < 0.35 ? "stand" : p < 0.75 ? "perch" : "sit" };
    }
    case "asleep":
      return { ...BASE, frame: "sit", dim: true, zzz: still ? [0.3, 0.7] : [0, 0.5].map((k) => (time / 3.6 + k) % 1) };
    case "waking": {
      const p = still ? 0.5 : loop(time, 1.6, 2.4);
      return { ...BASE, frame: p < 0.35 ? "sit" : p < 0.75 ? "perch" : "stand" };
    }
    case "creating": {
      if (still) return { ...BASE, drop: 0.5, rope: true };
      const p = clamp01((time % DROP_PERIOD) / DROP_SECS);
      return { ...BASE, drop: 1 - ease(p), rope: time % DROP_PERIOD < DROP_PERIOD - 0.6 };
    }
    case "error":
      return { ...BASE, alarm: still || time % 1 < 0.6 };
    case "missing":
      return { ...BASE, ghost: true, dim: true };
    case "unknown":
      return { ...BASE, dim: true };
  }
}

// H hat · h band · F face · f face shade · G glasses · C coat · T shirt · L legs · S shoes
// B briefcase · K crate · k crate shade
const FRAMES: Record<Frame, string[]> = {
  stand: ["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCC..", "..L.L...", "..S.S..."],
  walkA: ["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCCBB", ".L...LBB", "S.....S."],
  walkB: ["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCC..", "...LL.BB", "...SS.BB"],
  // seated on a crate, hat still up
  perch: ["........", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "FCCCCCF.", "kCCCCLLL", "KKKK...L", "KkkK...L", "KKKK..SS"],
  // hat pulled down over the eyes
  sit: ["........", "........", "..HHH...", "..hhh...", ".HHHHHH.", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "FCCCCCF.", "kCCCCLLL", "KKKK...L", "KkkK...L", "KKKK..SS"],
};
const W = 8;
const H = 14;

function colors(c: SpyColors, pal: Palette): Record<string, Color> {
  return {
    H: c.hat,
    h: c.band,
    F: c.face,
    f: c.faceShade,
    G: c.glasses,
    C: c.coat,
    T: c.shirt,
    L: c.legs,
    S: c.shoes,
    B: c.briefcase,
    K: pal.crate,
    k: pal.crateShade,
  };
}

const Z_SMALL = ["###", ".#.", "#..", "###"];
const Z_BIG = ["####", "..#.", ".#..", "#...", "####"];

export interface Anchor {
  x: number; // the agent's centre in buffer px
  y: number; // the top of what's drawn over the agent, in buffer px
}

/**
 * Draws an agent standing on buffer row `wy`, centred on buffer x `cx`. `zRise` is how far
 * z's float up: less in a small icon.
 */
export function drawSpy(px: Px, cx: number, wy: number, pal: Palette, pose: Pose, zRise = 7): Anchor {
  const map = colors(pose.dim ? pal.spyDim : pal.spy, pal);
  const ox = cx - W / 2 + pose.dx;
  const oy = wy - H - Math.round(pose.drop * (wy + 2));
  const put = (lx: number, row: number, col: Color) => {
    const X = ox + (pose.flip ? W - 1 - lx : lx);
    const Y = oy + row;
    if (pose.ghost && (X + Y) & 1) return;
    px.set(X, Y, col);
  };

  // a shadow on the gravel, once on the roof
  if (pose.drop === 0 && !pose.ghost) {
    const shade = pal.roof[Math.min(pal.roof.length - 1, Math.round(pal.roofTo) + 1)];
    const half = pose.frame === "sit" || pose.frame === "perch" ? 4 : 3;
    for (let d = -half; d <= half; d++) if (Math.abs(d) < half || bayer(cx + pose.dx + d, wy) > 0.5) px.set(cx + pose.dx + d, wy, shade);
  }

  const rows = FRAMES[pose.frame];
  let top = oy + H;
  rows.forEach((row, r) => {
    for (let lx = 0; lx < W; lx++) {
      const ch = row[lx];
      if (ch === ".") continue;
      put(lx, r, map[ch]);
      top = Math.min(top, oy + r);
    }
  });
  const hatX = ox + (pose.flip ? W - 4 : 3);

  if (pose.rope) for (let y = 0; y < top; y++) px.set(hatX, y, pal.rope);

  let anchorY = top;
  if (pose.alarm) {
    for (const dy of [7, 6, 5, 3]) {
      px.set(hatX, top - dy, pal.alarm);
      px.set(hatX + 1, top - dy, pal.alarm);
    }
    anchorY = top - 7;
  }

  // z's drifting up from under the hat
  for (const p of pose.zzz) {
    if (p > 0.9) continue;
    const glyph = p < 0.45 ? Z_SMALL : Z_BIG;
    const gx = cx + 2 + Math.round(p * 3);
    const gy = top + 1 - Math.round(p * zRise) - glyph.length;
    glyph.forEach((row, r) => [...row].forEach((ch, c) => ch === "#" && px.set(gx + c, gy + r, pal.zzz)));
  }

  return { x: cx + pose.dx, y: anchorY };
}
