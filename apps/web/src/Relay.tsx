// Under the hood: T3 Code talks only to t3agent37 on the laptop, which fans out to the machines on
// Agent37 Cloud. T3 Code sees every machine online; behind t3agent37 most sleep. Messages keep coming
// for random sleeping machines: each lands on t3agent37, which holds it, wakes that machine, then hands
// it over; the agent works a while, then the machine dozes off. Work runs long or short, so one
// machine may still be busy while the next is woken, and now and then a message goes to a busy
// machine, straight through t3agent37. Wires and packets are drawn over the laid-out page,
// measured, so the same picture works across or down the screen.
import { useLayoutEffect, useRef, useState } from "react";
import icon from "./assets/icon.png";
import { SpyIcon } from "./panel/components/SpyIcon";
import { useAnimation } from "./panel/pixel/clock";
import { seedOf, stateTag } from "./panel/machine";
import type { MachineState } from "./panel/types";
import { T3CodeMark } from "./logos";

const MACHINES = ["raven", "falcon", "kestrel", "lynx"] as const;
type Name = (typeof MACHINES)[number];
/** A wire: T3 Code to t3agent37, or t3agent37 to a machine. */
type Link = "t3" | Name;

/**
 * A message for a sleeping machine, from second `at`: it reaches t3agent37, which holds it, wakes
 * the machine, then passes it on; the agent works for `work` seconds, then the machine dozes off.
 */
interface Job {
  kind: "job";
  name: Name;
  at: number;
  work: number;
}

/** A message for a machine already working: t3agent37 passes it straight on, no waking. */
interface Nudge {
  kind: "nudge";
  name: Name;
  at: number;
}

type Message = Job | Nudge;

/** Each step of a job, in seconds from its start. The machine starts waking the moment it lands. */
const LANDS = 1.2;
const PASSES = 5.7;
const WORKS = 6.9;
const IDLE = 3;
const DOZE = 3;
/** A nudge: to t3agent37, then straight on to the machine. */
const NUDGED = 2 * LANDS;
/** How often a message goes to a machine already working, when one is. */
const NUDGE_ODDS = 0.4;

const endOf = (m: Message) => (m.kind === "job" ? m.at + WORKS + m.work + IDLE + DOZE : m.at + NUDGED);
/** Until t3agent37 is free again: the next message comes no sooner. */
const hubBusy = (m: Message) => (m.kind === "job" ? WORKS : NUDGED);
const between = (lo: number, hi: number) => lo + Math.random() * (hi - lo);
const pick = <T,>(xs: T[]) => xs[Math.floor(Math.random() * xs.length)];

/**
 * The next message after `messages`, a few seconds on: now and then for a machine still working
 * when it arrives, which then works on a while longer; otherwise for one asleep by then.
 */
function nextMessage(messages: Message[]): Message {
  const last = messages[messages.length - 1];
  let at = last.at + hubBusy(last) + between(1, 6);
  const jobs = messages.filter((m): m is Job => m.kind === "job");
  const working = jobs.filter((j) => j.at + WORKS <= at && at + NUDGED + 1 < j.at + WORKS + j.work);
  if (working.length && Math.random() < NUDGE_ODDS) {
    const j = pick(working);
    j.work = Math.max(j.work, at + NUDGED - j.at - WORKS + between(4, 10));
    return { kind: "nudge", name: j.name, at };
  }
  for (;;) {
    const free = MACHINES.filter((m) => m !== last.name && jobs.every((j) => j.name !== m || endOf(j) <= at));
    if (free.length) return { kind: "job", name: pick(free), at, work: between(5, 20) };
    at += 1;
  }
}

/** Already under way when the map comes on screen: one machine working, the next message on its way. */
function firstMessages(): Message[] {
  return [{ kind: "job", name: pick([...MACHINES]), at: -WORKS - 1, work: between(10, 16) }];
}

function stateIn(j: Job, s: number): MachineState {
  const into = s - j.at;
  if (into < LANDS || into >= endOf(j) - j.at) return "asleep";
  if (into < PASSES) return "waking";
  if (into < WORKS) return "idle";
  if (into < WORKS + j.work) return "working";
  if (into < WORKS + j.work + IDLE) return "idle";
  return "sleeping";
}

/** What shows at second `s`: each machine's state, t3agent37's label, and whether it holds a message. */
interface Scene {
  states: Record<Name, MachineState>;
  doing: string;
  held: boolean;
}

function sceneAt(messages: Message[], s: number): Scene {
  const states = Object.fromEntries(MACHINES.map((m) => [m, "asleep"])) as Record<Name, MachineState>;
  let doing = "";
  let held = false;
  for (const j of messages) {
    if (j.kind === "nudge") continue;
    const state = stateIn(j, s);
    if (state !== "asleep") states[j.name] = state;
    const into = s - j.at;
    if (into >= LANDS && into < PASSES) held = true;
    if (into >= LANDS && into < PASSES) doing = `waking ${j.name}`;
    else if (into >= PASSES && into < WORKS) doing = "passing it on";
  }
  return { states, doing, held };
}

const sameScene = (a: Scene, b: Scene) =>
  a.doing === b.doing && a.held === b.held && MACHINES.every((m) => a.states[m] === b.states[m]);

/** Something on a wire: `out` runs from T3 Code's side, `in` back towards it. */
interface Packet {
  link: Link;
  at: number;
  dur: number;
  dir: "out" | "in";
  kind?: "reply";
}

function packetsOf(j: Message): Packet[] {
  if (j.kind === "nudge")
    return [
      { link: "t3", at: j.at, dur: LANDS, dir: "out" },
      { link: j.name, at: j.at + LANDS, dur: LANDS, dir: "out" },
    ];
  const out: Packet[] = [
    { link: "t3", at: j.at, dur: LANDS, dir: "out" },
    { link: j.name, at: j.at + PASSES, dur: WORKS - PASSES, dir: "out" },
  ];
  // the agent's work streaming back to T3 Code
  for (let at = j.at + WORKS + 0.3; at < j.at + WORKS + j.work - 1; at += 2) {
    out.push({ link: j.name, at, dur: 1.4, dir: "in", kind: "reply" });
    out.push({ link: "t3", at: at + 1.4, dur: 1, dir: "in", kind: "reply" });
  }
  return out;
}

/** For reduced motion: one machine working, nothing moving. */
const STILL: Scene = {
  states: { raven: "working", falcon: "asleep", kestrel: "asleep", lynx: "asleep" },
  doing: "",
  held: false,
};

type Point = [number, number];

/**
 * A right-angled path from `a`'s box to `b`'s: across if `b` is beside `a`, else down. `level`:
 * straight from `a`'s side wherever it faces `b`'s middle, rather than from `a`'s middle.
 */
function elbow(a: DOMRect, b: DOMRect, gap: number, level = false): Point[] {
  if (b.left >= a.right) {
    const y1 = b.top + b.height / 2;
    const y0 = level && y1 >= a.top && y1 <= a.bottom ? y1 : a.top + a.height / 2;
    const [x0, x1] = [a.right + gap, b.left - gap];
    const mid = Math.round((x0 + x1) / 2);
    return [[x0, y0], [mid, y0], [mid, y1], [x1, y1]];
  }
  const x1 = b.left + b.width / 2;
  const x0 = level && x1 >= a.left && x1 <= a.right ? x1 : a.left + a.width / 2;
  const [y0, y1] = [a.bottom + gap, b.top - gap];
  const mid = Math.round((y0 + y1) / 2);
  return [[x0, y0], [x0, mid], [x1, mid], [x1, y1]];
}

/**
 * Down from `a` into a list of rows below it: across the gap above `region` to a spine at the
 * rows' left, down the spine, then into row `b`.
 */
function spine(a: DOMRect, b: DOMRect, region: DOMRect, gap: number): Point[] {
  const x0 = a.left + a.width / 2;
  const y = b.top + b.height / 2;
  const x = b.left - 3 * gap;
  const mid = region.top - 4 * gap;
  return [[x0, a.bottom + gap], [x0, mid], [x, mid], [x, y], [b.left - gap, y]];
}

/** The point `f` of the way along `path`. */
function along(path: Point[], f: number): Point {
  const legs = path.slice(1).map((p, i) => Math.hypot(p[0] - path[i][0], p[1] - path[i][1]));
  let left = f * legs.reduce((a, b) => a + b, 0);
  for (let i = 0; i < legs.length; i++) {
    if (left <= legs[i] || i === legs.length - 1) {
      const k = legs[i] ? Math.min(1, left / legs[i]) : 0;
      return [path[i][0] + (path[i + 1][0] - path[i][0]) * k, path[i][1] + (path[i + 1][1] - path[i][1]) * k];
    }
    left -= legs[i];
  }
  return path[0];
}

/** CSS px a packet snaps to, for the pixel look. */
const SNAP = 2;

export function Relay() {
  const map = useRef<HTMLDivElement>(null);
  const t3 = useRef<HTMLDivElement>(null);
  const hub = useRef<HTMLImageElement>(null);
  const cloud = useRef<HTMLDivElement>(null);
  const agents = useRef(new Map<Name, HTMLElement>());
  const packets = useRef<SVGGElement>(null);
  const [paths, setPaths] = useState<Partial<Record<Link, Point[]>>>({});
  const [scene, setScene] = useState(STILL);
  /** messages from just before now on, made up as time goes */
  const messages = useRef(firstMessages());
  /** the clock reading the messages started at, once the map came on screen */
  const start = useRef<number | null>(null);

  useLayoutEffect(() => {
    const el = map.current!;
    const measure = () => {
      const o = el.getBoundingClientRect();
      const rel = (n: Element) => {
        const r = n.getBoundingClientRect();
        return new DOMRect(r.left - o.left, r.top - o.top, r.width, r.height);
      };
      // t3agent37's wires leave from its icon across to the machines, or from under its label down to them
      const icon = rel(hub.current!);
      const whole = rel(hub.current!.parentElement!);
      const region = rel(cloud.current!);
      const across = region.left >= whole.right;
      const rows = new Map(MACHINES.map((m) => [m, rel(agents.current.get(m)!)]));
      // the line through t3agent37 runs level with the machine row nearest its middle, if one faces it
      const middle = icon.top + icon.height / 2;
      const facing = [...rows.values()]
        .map((r) => r.top + r.height / 2)
        .filter((y) => y >= icon.top && y <= icon.bottom)
        .sort((a, b) => Math.abs(a - middle) - Math.abs(b - middle));
      const port = across ? new DOMRect(icon.left, facing[0] ?? middle, icon.width, 0) : icon;
      setPaths({
        t3: elbow(rel(t3.current!), port, 12, true),
        ...Object.fromEntries(
          MACHINES.map((m) => {
            const to = rows.get(m)!;
            return [m, across ? elbow(port, to, 10) : spine(whole, to, region, 6)];
          }),
        ),
      });
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    document.fonts.ready.then(measure);
    const io = new IntersectionObserver(([e]) => e.isIntersecting && (start.current ??= performance.now() / 1000), {
      threshold: 0.5,
    });
    io.observe(el);
    return () => {
      ro.disconnect();
      io.disconnect();
    };
  }, []);

  useAnimation(
    (t) => {
      let svg = "";
      if (t === null) setScene(STILL);
      else {
        const s = start.current === null ? 0 : t - start.current;
        let list = messages.current;
        while (list[list.length - 1].at < s) list.push(nextMessage(list));
        // keep the last, which the next message follows on from
        list = messages.current = list.filter((j, i) => endOf(j) > s || i === list.length - 1);
        const next = sceneAt(list, s);
        setScene((prev) => (sameScene(prev, next) ? prev : next));
        for (const p of list.flatMap(packetsOf)) {
          const path = paths[p.link];
          const into = s - p.at;
          if (!path || into < 0 || into >= p.dur) continue;
          const f = into / p.dur;
          const [x, y] = along(path, p.dir === "out" ? f : 1 - f);
          const size = p.kind === "reply" ? 6 : 8;
          const at = (v: number) => Math.round((v - size / 2) / SNAP) * SNAP;
          svg += `<rect class="relay-packet${p.kind ? ` is-${p.kind}` : ""}" x="${at(x)}" y="${at(y)}" width="${size}" height="${size}"/>`;
        }
      }
      packets.current!.innerHTML = svg;
    },
    [paths],
  );

  const { doing, held } = scene;
  const state = (m: Name) => scene.states[m];

  return (
    <section className="relay">
      <p className="eyebrow">Under the hood</p>
      <h2>Run agents on several machines at once. Each one wakes when a message comes in.</h2>

      <div ref={map} className="relay-map" data-ai-ignore>
        <div className="relay-laptop">
          <span className="relay-where">your laptop</span>

          <div ref={t3} className="relay-t3">
            <span className="relay-name" role="img" aria-label="T3 Code">
              <T3CodeMark aria-hidden />
              Code
            </span>
            <ul aria-label="Machines, as T3 Code sees them">
              {MACHINES.map((m) => (
                <li key={m}>
                  {m}
                  <span>online</span>
                </li>
              ))}
            </ul>
          </div>

          <div className="relay-hub">
            <img ref={hub} src={icon} alt="" width={56} height={56} />
            <span className="relay-held" data-on={held || undefined} />
            <span className="relay-name">t3agent37</span>
            <span key={doing} className="relay-doing" data-on={doing ? "" : undefined}>
              {doing}
            </span>
          </div>
        </div>

        <div ref={cloud} className="relay-cloud">
          <span className="relay-where">Agent37 Cloud</span>
          <ul>
            {MACHINES.map((m) => (
              <li
                key={m}
                ref={(el) => {
                  if (el) agents.current.set(m, el);
                }}
                data-state={state(m)}
              >
                <SpyIcon state={state(m)} seed={seedOf(m)} />
                <span className="relay-vm-name">{m}</span>
                <span className="relay-tag">{stateTag(state(m))}</span>
              </li>
            ))}
          </ul>
        </div>

        <svg className="relay-wires" aria-hidden>
          {/* the machines' wires share their first stretch: live ones drawn last, over the dashed; a waking machine's pulses */}
          {(Object.entries(paths) as [Link, Point[]][])
            .map(([link, path]) => ({ link, path, live: link === "t3" || state(link) !== "asleep" }))
            .sort((a, b) => Number(a.live) - Number(b.live))
            .map(({ link, path, live }) => (
              <polyline
                key={link}
                points={path.join(" ")}
                data-live={live || undefined}
                data-waking={(link !== "t3" && state(link) === "waking") || undefined}
              />
            ))}
          <g ref={packets} />
        </svg>
      </div>
    </section>
  );
}
