// The hero: the panel's rooftop, full bleed, drawn by the panel's own pixel engine. Nav and copy
// sit on the plain dusk sky; the skyline, the roof and a crew of field agents come below them,
// one agent per machine, each in the pose of its machine's state.
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useAnimation, useTheme } from "./panel/pixel/clock";
import { palettes } from "./panel/pixel/palette";
import { Px } from "./panel/pixel/px";
import { drawScene } from "./panel/pixel/scene";
import { drawSpy, poseFor } from "./panel/pixel/spy";
import { seedOf, stateTag } from "./panel/machine";
import type { MachineState } from "./panel/types";

/**
 * The crew, left to right. `depth`: where on the roof the agent stands, from the ledge (0) to
 * the hero's bottom (1); `room`: how far a working agent paces; `rank`: who stays on a narrow
 * roof (lowest first), so a phone still shows one working, one asleep and one waking.
 */
const CREW: { key: string; state: MachineState; depth: number; room: number; rank: number }[] = [
  { key: "raven", state: "working", depth: 0.5, room: 12, rank: 0 },
  { key: "falcon", state: "asleep", depth: 0.36, room: 0, rank: 1 },
  { key: "kestrel", state: "idle", depth: 0.6, room: 0, rank: 4 },
  { key: "lynx", state: "waking", depth: 0.42, room: 0, rank: 2 },
  { key: "heron", state: "working", depth: 0.56, room: 9, rank: 5 },
  // drops in on a rope from above: on the right, clear of the headline
  { key: "magpie", state: "creating", depth: 0.4, room: 0, rank: 3 },
];
/** Scene px each agent needs with its tag; a narrower roof carries fewer. */
const SLOT = 22;
/** Scene px kept free of agents at either end, where the roof's vent and AC unit stand. */
const MARGIN = 14;
/** Rows between the copy's last line and the horizon: room for the skyline under the copy. */
const GAP = 32;
/** Rows of sky gradient above the horizon; above them the sky is plain, behind the copy. */
const SKY = 72;
/** Rows of roof in the panel's close-up strip: the hero's roof zooms from there. */
const ROOF_ROWS = 42;

/** CSS px per scene px: twice the panel's, a whole number so the pixels stay crisp. */
const PIXEL = 4;

/** `nav` and the copy (`children`) sit on plain sky; the scene fills the hero below them. */
export function Hero({ nav, children }: { nav: ReactNode; children: ReactNode }) {
  const box = useRef<HTMLElement>(null);
  const copy = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const buffer = useRef<Px | null>(null);
  const tags = useRef(new Map<string, HTMLSpanElement>());
  /** hero size and where the copy ends, in CSS px */
  const [layout, setLayout] = useState({ w: 0, h: 0, copy: 0 });
  const theme = useTheme();

  useLayoutEffect(() => {
    const el = box.current!;
    const measure = () =>
      setLayout({ w: el.clientWidth, h: el.clientHeight, copy: copy.current!.offsetTop + copy.current!.offsetHeight });
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    ro.observe(copy.current!);
    // line boxes move once the web fonts land
    document.fonts.ready.then(measure);
    return () => ro.disconnect();
  }, []);

  const s = PIXEL;
  const w = Math.ceil(layout.w / s);
  const h = Math.ceil(layout.h / s);
  const horizon = Math.ceil(layout.copy / s) + GAP;
  const skyTop = Math.max(0, horizon - SKY);
  const zoom = Math.max(1, (h - horizon) / ROOF_ROWS);
  const fits = Math.max(2, Math.min(CREW.length, Math.floor((w - 2 * MARGIN) / SLOT)));
  const crew = CREW.filter((a) => a.rank < fits);

  useAnimation(
    (t) => {
      const ctx = canvas.current?.getContext("2d");
      if (!ctx || !w || !h) return;
      if (buffer.current?.w !== w || buffer.current.h !== h) buffer.current = new Px(w, h);
      const px = buffer.current;
      const pal = palettes[theme];
      const frame = {
        horizon,
        skyTop,
        zoom,
        sun: { x: Math.round(w * 0.3), y: horizon - 8 },
        moon: { x: w - Math.round(48 * Math.min(zoom, 2)), y: Math.max(8, horizon - 44) },
        // fewer than the panel's strip: at this size they'd speckle the copy
        stars: 0.007,
        cloudTop: horizon - GAP + 2,
      };
      drawScene(px, frame, pal, theme, t ?? 0, undefined);
      const slot = (w - 2 * MARGIN) / crew.length;
      // back row first so nearer agents overlap farther ones
      const agents = crew.map((a, i) => ({
        ...a,
        cx: Math.round(MARGIN + slot * (i + 0.5)),
        wy: Math.round(horizon + 6 + a.depth * (h - horizon - 6 - 2 * zoom * 7)),
      }));
      agents.sort((a, z) => a.wy - z.wy);
      for (const a of agents) {
        const top = drawSpy(px, a.cx, a.wy, pal, poseFor(a.state, t, seedOf(a.key), a.room));
        const tag = tags.current.get(a.key);
        if (tag) tag.style.transform = `translate(${top.x * s}px, ${(top.y - 1) * s}px) translate(-50%, -100%)`;
      }
      px.blit(ctx);
    },
    [theme, layout, crew.length],
  );

  return (
    <header ref={box} className="hero">
      <div className="roof" aria-hidden>
        <canvas ref={canvas} width={w} height={h} style={{ width: w * s, height: h * s }} />
        {crew.map((a) => (
          <span
            key={a.key}
            ref={(el) => {
              if (el) tags.current.set(a.key, el);
              else tags.current.delete(a.key);
            }}
            className={`roof-tag is-${a.state}`}
          >
            {stateTag(a.state)}
          </span>
        ))}
      </div>
      <nav className="nav">{nav}</nav>
      <div ref={copy} className="hero-copy">
        {children}
      </div>
    </header>
  );
}
