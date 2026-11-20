//! The rooftop strip, : a dithered sky over a city skyline with a searchlight,
//! and a gravel roof fading into the page.

use std::f64::consts::PI;

use super::palette::Palette;
use super::{Px, Theme, bayer, hash, hash2, ramp};

/// `Math.round`: halves go up.
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

pub const HORIZON: f64 = 32.;

/// A block of scene px, ends exclusive.
#[derive(Clone, Copy)]
pub struct Rect {
    pub x1: f64,
    pub y1: f64,
}

struct Geometry {
    horizon: f64,
    deep: f64,
    dissolve: f64,
    haze: f64,
    sun: (f64, f64),
    moon: (f64, f64),
    searchlight: f64,
}

fn geometry(w: f64, k: f64) -> Geometry {
    let horizon = js_round(HORIZON * k);
    Geometry {
        horizon,
        deep: horizon + js_round(22. * k),
        dissolve: js_round(7. * k),
        haze: js_round(21. * k),
        sun: (js_round(122. * k), horizon - 8.),
        moon: (js_round(112. * k), js_round(10. * k)),
        searchlight: w - js_round(30. * k),
    }
}

const LIGHT_W: f64 = 13.;
const LIGHT_H: f64 = 13.;

struct Building {
    x0: f64,
    x1: f64,
    h: f64,
    i: f64,
}

#[allow(clippy::too_many_arguments)]
fn skyline(w: f64, horizon: f64, seed: f64, min_h: f64, max_h: f64, min_w: f64, max_w: f64, calm: Option<Rect>) -> Vec<Building> {
    let mut out = Vec::new();
    let (mut x, mut i) = (-3., 0.);
    while x < w {
        let bw = min_w + (hash2(i, seed) * (max_w - min_w + 1.)).floor();
        let mut h = min_h + (hash2(i, seed + 1.) * (max_h - min_h + 1.)).floor();
        if let Some(calm) = calm
            && x < calm.x1
        {
            h = h.min(horizon - calm.y1 - 1.);
        }
        out.push(Building { x0: x, x1: x + bw, h: h.max(2.), i });
        x += bw + if hash2(i, seed + 2.) > 0.7 { 1. } else { 0. };
        i += 1.;
    }
    out
}

const CLOUD_AREA: f64 = 4500.;

fn draw_clouds(px: &mut Px, g: &Geometry, pal: &Palette, time: f64) {
    let w = px.w as f64;
    let rows = g.horizon - 14.;
    if rows <= 0. {
        return;
    }
    let count = js_round(w * rows / CLOUD_AREA).max(1.) as i32;
    for i in 0..count {
        let i = i as f64;
        let base = 8. + js_round((i + hash2(i, 41.)) / count as f64 * rows);
        let near = 1. - 0.5 * base / g.horizon;
        let width = js_round((20. + hash2(i, 42.) * 22.) * near);
        let speed = (0.5 + hash2(i, 43.) * 0.6) * near;
        let period = w + 2. * width;
        let cx = (hash2(i, 44.) * period + time * speed) % period - width;
        let puffs: Vec<(f64, f64)> = (0..4)
            .map(|k| {
                let k = k as f64;
                let x = cx + ((k + 0.5) / 4. - 0.5) * width * 0.75;
                let r = width * (0.13 + hash(i, k, 45.) * 0.1 + if k == 1. || k == 2. { 0.07 } else { 0. });
                (x, r)
            })
            .collect();
        let tall = puffs.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let mut y = (base - tall).floor();
        while y <= base {
            let v = base - y;
            let mut x = (cx - width / 2.).floor();
            while x <= (cx + width / 2.).ceil() {
                if puffs.iter().any(|&(px_, r)| (x - px_).powi(2) + v * v <= r * r) {
                    px.set(x, y, ramp(pal.cloud, 1. - v / (tall * 0.7), x, y));
                }
                x += 1.;
            }
            y += 1.;
        }
    }
}

fn sky_t(g: &Geometry, x: f64, y: f64, time: f64, theme: Theme, beam: f64) -> f64 {
    let light = theme == Theme::Light;
    let mut t = (y / g.horizon).powf(if light { 2.6 } else { 1.3 });

    if y > g.haze {
        let len = 12. + hash2(y, 1.) * 22.;
        let seg = ((x + time * (0.6 + hash2(y, 2.)) + hash2(y, 3.) * 400.) / len).floor();
        if y % 3. != 0. && hash(seg, y, 7.) > 0.78 {
            t += if light { 0.22 } else { 0.12 };
        }
    }

    if light {
        let d = ((x - g.sun.0) / 1.8).hypot(y - g.sun.1);
        if d < 16. {
            t = t.max(1.05 - d / 22.);
        }
    } else {
        let d = (x - g.moon.0).hypot(y - g.moon.1);
        if d < 11. {
            t += (1. - d / 11.) * 0.35;
        }
        let lamp_y = g.horizon - LIGHT_H - 3.;
        let (dx, dy) = (x - g.searchlight, y - lamp_y);
        let dist = dx.hypot(dy);
        if dy <= 0. && dist > 3. && dist < 140. {
            let mut a = dy.atan2(dx) - beam;
            a = a.sin().atan2(a.cos());
            if a.abs() < 0.045 + 1.5 / (dist + 30.) {
                t += 0.55 * (1. - dist / 140.);
            }
        }
    }
    t
}

/// Fills `px` with the scene at `zoom`. Buildings under `calm` (the brand) stay short.
pub fn draw_scene(px: &mut Px, zoom: f64, pal: &Palette, theme: Theme, time: f64, calm: Option<Rect>) {
    let (w, h) = (px.w as f64, px.h as f64);
    let g = geometry(w, zoom);
    let horizon = g.horizon;
    let beam = (time * 0.4).sin() - PI / 2.;

    for y in 0..horizon as i32 {
        for x in 0..px.w {
            let (x, y) = (x as f64, y as f64);
            px.set(x, y, ramp(pal.sky, sky_t(&g, x, y, time, theme, beam), x, y));
        }
    }

    if theme == Theme::Light {
        if !pal.cloud.is_empty() {
            draw_clouds(px, &g, pal, time);
        }
        let (sx, sy) = g.sun;
        for y in (sy - 3.) as i32..=(sy + 3.) as i32 {
            for x in (sx - 3.) as i32..=(sx + 3.) as i32 {
                let (x, y) = (x as f64, y as f64);
                if (x - sx).hypot(y - sy) <= 2.6 {
                    px.set(x, y, pal.glow[0]);
                }
            }
        }
    } else {
        for y in 0..(horizon - 8.) as i32 {
            for x in 0..px.w {
                let (x, y) = (x as f64, y as f64);
                let s = hash(x, y, 3.);
                if s > 0.984 {
                    let twinkle = hash(x, y, (time * 1.5 + s * 40.).floor()) > 0.8;
                    px.set(x, y, pal.star[if twinkle { 2 } else if s > 0.995 { 0 } else { 1 }]);
                }
            }
        }
        let (mx, my) = g.moon;
        for y in (my - 5.) as i32..=(my + 5.) as i32 {
            for x in (mx - 5.) as i32..=(mx + 5.) as i32 {
                let (x, y) = (x as f64, y as f64);
                let d = (x - mx).hypot(y - my);
                if d <= 4.2 {
                    let shade = (x - mx - 1.6).hypot(y - my + 1.) < 2. && d > 1.;
                    px.set(x, y, if shade { pal.glow[1] } else { pal.glow[0] });
                }
            }
        }
    }

    // far skyline: a few lit windows, the odd antenna
    for b in skyline(w, horizon, 10., 9., 21., 6., 12., calm) {
        let mut x = b.x0;
        while x < b.x1 {
            let mut y = horizon - b.h;
            while y <= horizon {
                let lit = (x - b.x0) % 2. == 1. && x < b.x1 - 1. && (horizon - y) % 3. == 1. && horizon - y > 1. && hash(x, y, 77.) > 0.7;
                px.set(x, y, if lit { pal.far_window } else { pal.far });
                y += 1.;
            }
            x += 1.;
        }
        if hash2(b.i, 33.) > 0.75 {
            for k in 1..=4 {
                px.set(((b.x0 + b.x1) as i64 >> 1) as f64, horizon - b.h - k as f64, pal.far);
            }
        }
    }
    // near skyline: windows that come on and go off
    for b in skyline(w, horizon, 20., 3., 12., 5., 10., calm) {
        let mut x = b.x0;
        while x < b.x1 {
            let mut y = horizon - b.h;
            while y <= horizon {
                let is_window = (x - b.x0) % 2. == 1. && x < b.x1 - 1. && (horizon - y) % 2. == 0. && horizon - y > 1. && y > horizon - b.h + 1.;
                let lit = is_window && hash(x, y, (time / 5. + hash(x, y, 5.) * 9.).floor()) > 0.6;
                px.set(x, y, if is_window { if lit { pal.window_lit } else { pal.window } } else { pal.near });
                y += 1.;
            }
            x += 1.;
        }
    }

    // the searchlight on its building; by day an aircraft light blinks on its mast instead
    let l = g.searchlight;
    let x0 = l - 6.;
    let top = horizon - LIGHT_H;
    for x in x0 as i32..(x0 + LIGHT_W) as i32 {
        for y in top as i32..=horizon as i32 {
            let (x, y) = (x as f64, y as f64);
            let is_window = (x - x0) % 3. == 1. && (horizon - y) % 3. == 1. && y > top + 2.;
            px.set(x, y, if is_window && hash(x, y, 9.) > 0.5 { pal.window } else { pal.tower });
        }
    }
    for x in (x0 - 1.) as i32..=(x0 + LIGHT_W) as i32 {
        px.set(x as f64, top, pal.tower);
    }
    let lamp_on = theme == Theme::Dark;
    for x in (l - 1.) as i32..=(l + 1.) as i32 {
        px.set(x as f64, top - 1., pal.tower);
        px.set(x as f64, top - 2., if lamp_on { pal.lamp } else { pal.tower });
    }
    px.set(l, top - 3., if lamp_on { pal.lamp } else { pal.tower });
    if !lamp_on {
        let mast = x0 + LIGHT_W - 3.;
        for k in 1..=6 {
            px.set(mast, top - k as f64, pal.tower);
        }
        if time % 2. < 0.9 {
            px.set(mast, top - 7., pal.beacon);
        }
    }

    // the roof: a ledge, then gravel with tar seams fanning out towards you
    for x in 0..px.w {
        px.set(x as f64, horizon + 1., pal.ledge);
        px.set(x as f64, horizon + 2., pal.ledge_shade);
    }
    let span = pal.roof_to - pal.roof_from;
    let last = (pal.roof.len() - 1) as f64;
    for y in (horizon + 3.) as i32..px.h {
        let y = y as f64;
        let d = ((y - horizon) / (g.deep - horizon)).min(1.);
        let near = if y > g.deep { (y - g.deep) / (h - 1. - g.deep) } else { 0. };
        let base = pal.roof_from + span * d.powf(0.8) + (pal.roof_fade - pal.roof_to) * near.powf(1.4);
        let fade = (y - (h - 1. - g.dissolve)) / g.dissolve;
        for x in 0..px.w {
            let x = x as f64;
            if y == h - 1. {
                px.set(x, y, pal.bg);
                continue;
            }
            if fade > 0. {
                let jag = (hash2((x as i64 >> 2) as f64, 4.) - 0.5) * 0.5 + (hash(x, y, 4.) - 0.5) * 0.25;
                if fade + jag > bayer(x, y) {
                    px.set(x, y, pal.bg);
                    continue;
                }
            }
            let seam = js_round(x + (x - w / 2.) * (y - horizon) / 30.) as i64;
            let idx = base + if seam % 38 == 0 { 0.6 } else { 0. } + (hash(x, y, 2.) - 0.5) * 0.3;
            px.set(x, y, ramp(pal.roof, idx / last, x, y));
        }
    }

    // fixtures at either end of the roof: a vent pipe, an AC unit with its fan turning
    for y in (horizon + 2.) as i32..=(horizon + 7.) as i32 {
        px.set(7., y as f64, pal.tower);
        px.set(8., y as f64, pal.tower);
    }
    for x in 5..=10 {
        px.set(x as f64, horizon + 2., pal.tower);
    }
    for x in (w - 17.) as i32..=(w - 7.) as i32 {
        for y in (horizon + 4.) as i32..=(horizon + 10.) as i32 {
            let y = y as f64;
            px.set(x as f64, y, if y == horizon + 4. { pal.ledge } else { pal.near });
        }
    }
    let spin = (time * 8.).floor() as i64 % 2;
    let blades: [(f64, f64); 4] = if spin != 0 { [(-1., -1.), (1., 1.), (-1., 1.), (1., -1.)] } else { [(0., -1.), (0., 1.), (-1., 0.), (1., 0.)] };
    for (dx, dy) in blades {
        px.set(w - 12. + dx, horizon + 7. + dy, pal.ledge);
    }
}
