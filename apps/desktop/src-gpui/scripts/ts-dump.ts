// Renders the TS rooftop and spy icons with fixed inputs, as raw RGBA, for comparison with the Rust port.
(globalThis as any).ImageData = class { width: number; height: number; data: Uint8ClampedArray; constructor(w: number, h: number) { this.width = w; this.height = h; this.data = new Uint8ClampedArray(w * h * 4); } };
const root = process.argv[2];
const out = process.argv[3];
const { Px } = await import(`${root}/pixel/px.ts`);
const { paintRoof, roofFor, paintSpyIcon, ICON_W, ICON_H } = await import(`${root}/pixel/paint.ts`);
const { seedOf } = await import(`${root}/machine.ts`);
const states = ["working", "idle", "asleep", "creating", "error", "sleeping", "waking", "missing", "unknown"];
for (const theme of ["light", "dark"]) {
  for (const n of [1, 3, 5, 9]) {
    for (const t of [0, 12.34, 101.7, null]) {
      const agents = Array.from({ length: n }, (_, i) => ({ key: `demo-${i}`, state: states[i % states.length] }));
      const roof = roofFor(n, 2);
      const px = new Px(roof.w, roof.h);
      const anchors = paintRoof(px, roof, theme, agents, t);
      await Bun.write(`${out}/ts-roof-${theme}-${n}-${t}.rgba`, new Uint8Array(px.data.buffer));
      await Bun.write(`${out}/ts-roof-${theme}-${n}-${t}.anchors`, agents.map((g: any) => anchors.get(g.key)).map((a: any) => `${a.x},${a.y}`).join(";"));
    }
  }
  for (const s of states) {
    for (const t of [0, 7.77, null]) {
      const px = new Px(ICON_W, ICON_H);
      paintSpyIcon(px, theme, s, seedOf(`demo-${s}`), t);
      await Bun.write(`${out}/ts-icon-${theme}-${s}-${t}.rgba`, new Uint8Array(px.data.buffer));
    }
  }
}
console.log("ts done");
