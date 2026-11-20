import { useEffect, useMemo, useRef, type CSSProperties, type ReactNode } from "react";
import type { MachineState } from "../types";
import { useAnimation, useDevicePixelRatio, useReducedMotion, useTheme } from "../pixel/clock";
import { Px } from "../pixel/px";
import { paintRoof, roofFor } from "../pixel/paint";
import { stateTag } from "../machine";

export interface RooftopAgent {
  key: string;
  name: string;
  state: MachineState;
  /** tag text instead of the state word */
  label?: string;
  /** clickable (opens its details) */
  pickable?: boolean;
}

/** The signature strip: a pixel city rooftop with one field agent per machine. */
interface Props {
  agents: RooftopAgent[];
  /** key of the agent to call out (hovered row, draft machine) */
  active?: string | null;
  /** shorter strip for sub-views: keeps the agents, crops the sky */
  compact?: boolean;
  /** a pickable agent was clicked */
  onPick?: (key: string) => void;
  children?: ReactNode;
}

/** How close (CSS px) the active agent may get to the strip's edge before it scrolls into view. */
const FOLLOW_MARGIN = 40;
/** Tags keep this far (CSS px) from the roof's ends, sliding off-centre from their agent if they must. */
const TAG_INSET = 4;

export function Rooftop({ agents, active, compact, onPick, children }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const buffer = useRef<Px | null>(null);
  const tags = useRef(new Map<string, HTMLSpanElement>());
  const hits = useRef(new Map<string, HTMLButtonElement>());
  const theme = useTheme();
  const reduced = useReducedMotion();
  const dpr = useDevicePixelRatio();
  const roof = useMemo(() => roofFor(agents.length, dpr), [agents.length, dpr]);
  const showAllTags = agents.length <= 4;
  const activeIndex = agents.findIndex((a) => a.key === active);

  const signature = agents.map((a) => `${a.key}:${a.state}`).join("|");

  useAnimation(
    (t) => {
      const ctx = canvas.current?.getContext("2d");
      if (!ctx) return;
      if (buffer.current?.w !== roof.w || buffer.current.h !== roof.h) buffer.current = new Px(roof.w, roof.h);
      const px = buffer.current;
      const worldW = roof.w * roof.scale;
      paintRoof(px, roof, theme, agents, t).forEach((a, key) => {
        const x = a.x * roof.scale;
        const tag = tags.current.get(key);
        if (tag) {
          const half = tag.offsetWidth / 2;
          const tx = Math.min(Math.max(x, half + TAG_INSET), worldW - half - TAG_INSET);
          tag.style.transform = `translate(${tx}px, ${(a.y - 1) * roof.scale}px) translate(-50%, -100%)`;
        }
        const hit = hits.current.get(key);
        if (hit) hit.style.transform = `translate(${x}px, ${a.y * roof.scale}px) translate(-50%, 0)`;
      });
      px.blit(ctx);
    },
    [theme, signature, active, roof],
  );

  // a wide roof follows the agent being called out
  useEffect(() => {
    const el = scroller.current;
    if (!el || activeIndex < 0) return;
    const x = roof.spots[activeIndex].cx * roof.scale;
    if (x >= el.scrollLeft + FOLLOW_MARGIN && x <= el.scrollLeft + el.clientWidth - FOLLOW_MARGIN) return;
    el.scrollTo({ left: x - el.clientWidth / 2, behavior: reduced ? "auto" : "smooth" });
  }, [activeIndex, roof, reduced]);

  return (
    <div className={`rooftop${compact ? " is-compact" : ""}`}>
      <div
        ref={scroller}
        className={`rooftop-scroll${roof.scrolls ? " is-scrolling" : ""}`}
        // a mouse wheel scrolls the roof sideways
        onWheel={(e) => {
          if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) e.currentTarget.scrollLeft += e.deltaY;
        }}
      >
        <div className="rooftop-world" style={{ width: roof.w * roof.scale, height: roof.h * roof.scale, "--roof-scale": roof.scale } as CSSProperties}>
          <canvas ref={canvas} width={roof.w} height={roof.h} className="rooftop-canvas" aria-hidden />
          <div className="rooftop-tags" aria-hidden>
            {agents.map((a) => {
              const isActive = active === a.key;
              if (!showAllTags && !isActive && a.state !== "error") return null;
              return (
                <span
                  key={a.key}
                  ref={(el) => {
                    if (el) tags.current.set(a.key, el);
                    else tags.current.delete(a.key);
                  }}
                  className={`tag tag-${a.state}${isActive ? " is-active" : ""}`}
                >
                  {a.label ?? (isActive ? a.name : stateTag(a.state))}
                </span>
              );
            })}
          </div>
          {onPick ? (
            <div className="rooftop-hits" aria-hidden>
              {agents.map((a) =>
                a.pickable ? (
                  <button
                    key={a.key}
                    ref={(el) => {
                      if (el) hits.current.set(a.key, el);
                      else hits.current.delete(a.key);
                    }}
                    className="rooftop-hit"
                    tabIndex={-1}
                    aria-label={`Details for ${a.name}`}
                    onClick={() => onPick(a.key)}
                  />
                ) : null,
              )}
            </div>
          ) : null}
        </div>
      </div>
      {children}
    </div>
  );
}
