import { useRef } from "react";
import type { MachineState } from "../types";
import { useAnimation, useTheme } from "../pixel/clock";
import { ICON_H, ICON_W, paintSpyIcon } from "../pixel/paint";
import { Px } from "../pixel/px";

/** A machine's field agent on its own patch of roof, for list rows. */
export function SpyIcon({ state, seed }: { state: MachineState; seed: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const buffer = useRef<Px | null>(null);
  const theme = useTheme();

  useAnimation(
    (t) => {
      const ctx = canvas.current?.getContext("2d");
      if (!ctx) return;
      const px = (buffer.current ??= new Px(ICON_W, ICON_H));
      paintSpyIcon(px, theme, state, seed, t);
      px.blit(ctx);
    },
    [theme, state, seed],
  );

  return <canvas ref={canvas} width={ICON_W} height={ICON_H} className="spy-icon" aria-hidden />;
}
