//! Whole pictures from the parts, : the rooftop strip and a row's agent icon.

use super::palette::palette;
use super::scene::{Rect, draw_scene, js_round};
use super::spy::{Anchor, draw_spy, pose_for};
use super::{CLEAR, MachineState, Px, Theme, hash, ramp, seed_of};

/// The strip's height in points; it's as wide as the panel.
pub const STRIP_H: f64 = 148.;
const MARGIN: f64 = 18.;
const SLOT: f64 = 19.;
const SCROLL_SLOT: f64 = 24.;
/// Where the brand sits, in points: the skyline stays low under it.
const BRAND_RIGHT: f64 = 132.;
const BRAND_BOTTOM: f64 = 38.;

pub struct Spot {
    pub cx: f64,
    pub wy: f64,
    pub room: f64,
}

pub struct Roof {
    /// Points per scene px: 2 close up … 1 zoomed out.
    pub scale: f64,
    pub w: i32,
    pub h: i32,
    /// Wider than the strip: it scrolls.
    pub scrolls: bool,
    pub spots: Vec<Spot>,
}

/// Zoom and room for `n` agents on a `dpr` screen, on a strip `strip_w` points wide (the
/// panel's width), as `roofFor`.
pub fn roof_for(n: usize, dpr: f64, strip_w: f64) -> Roof {
    let scales: &[f64] = if (1.5 * dpr).fract() == 0. { &[2., 1.5, 1.] } else { &[2., 1.] };
    let need = 2. * MARGIN + n as f64 * SLOT;
    let scale = scales.iter().copied().find(|s| need <= (strip_w / s).floor()).unwrap_or(1.);
    let k = 2. / scale;
    let fit = (strip_w / scale).ceil();
    let scrolls = need > fit;
    let w = if scrolls { 2. * MARGIN + n as f64 * SCROLL_SLOT } else { fit };
    let slot = (w - 2. * MARGIN) / n.max(1) as f64;
    Roof {
        scale,
        w: w as i32,
        h: (STRIP_H / scale).ceil() as i32,
        scrolls,
        spots: (0..n)
            .map(|i| Spot {
                cx: js_round(MARGIN + slot * (i as f64 + 0.5)),
                wy: js_round(k * if n == 1 { 54. } else { 52. + (i % 2) as f64 * 4. }),
                room: ((slot - 14.) / 2.).floor().clamp(0., 7.),
            })
            .collect(),
    }
}

pub struct Agent<'a> {
    pub key: &'a str,
    pub state: MachineState,
}

/// Draws the roof with its agents at `t` (None: still frame). Where each agent's tag goes,
/// in scene px, in `agents` order.
pub fn paint_roof(px: &mut Px, roof: &Roof, theme: Theme, agents: &[Agent], t: Option<f64>) -> Vec<Anchor> {
    let pal = palette(theme);
    let brand = Rect { x1: (BRAND_RIGHT / roof.scale).ceil(), y1: (BRAND_BOTTOM / roof.scale).ceil() };
    draw_scene(px, 2. / roof.scale, pal, theme, t.unwrap_or(0.), Some(brand));
    let mut anchors = vec![Anchor { x: 0., y: 0. }; agents.len()];
    // back row first so nearer agents overlap farther ones
    let mut order: Vec<usize> = (0..agents.len()).collect();
    order.sort_by(|&a, &b| roof.spots[a].wy.total_cmp(&roof.spots[b].wy));
    for i in order {
        let s = &roof.spots[i];
        let pose = pose_for(agents[i].state, t, seed_of(agents[i].key), s.room);
        anchors[i] = draw_spy(px, s.cx, s.wy, pal, &pose, 7.);
    }
    anchors
}

pub const ICON_W: i32 = 24;
pub const ICON_H: i32 = 21;
const FLOOR: f64 = 19.;

/// An agent on its own patch of roof, for list rows.
pub fn paint_spy_icon(px: &mut Px, theme: Theme, state: MachineState, seed: f64, t: Option<f64>) {
    let pal = palette(theme);
    px.fill(CLEAR);
    for y in (FLOOR - 2.) as i32..ICON_H {
        for x in 0..ICON_W {
            let (x, y) = (x as f64, y as f64);
            let edge = x.min(ICON_W as f64 - 1. - x) / 5. - (y - FLOOR + 1.) * 0.22;
            if edge < hash(x, y, 1.) * 0.6 {
                continue;
            }
            let idx = pal.roof_to - 1.4 + (y - FLOOR) * 0.4 + (hash(x, y, 2.) - 0.5) * 0.6;
            px.set(x, y, ramp(pal.roof, idx / (pal.roof.len() - 1) as f64, x, y));
        }
    }
    draw_spy(px, ICON_W as f64 / 2., FLOOR, pal, &pose_for(state, t, seed, 0.), 3.);
}
