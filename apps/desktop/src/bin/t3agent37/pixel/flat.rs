//! The still look (the default): a flat night skyline for the panel's header and a small flat
//! spy per machine, its coat in its state's colour. Nothing in them moves; the animated
//! rooftop (`paint`) is a setting.

use super::{CLEAR, Color, MachineState, Px, hash};

/// A colour from the panel's tokens, as the pixel buffer holds it.
pub fn color(c: gpui_kit::Rgba) -> Color {
    let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    (ch(c.r) << 24) | (ch(c.g) << 16) | (ch(c.b) << 8) | ch(c.a)
}

/// The colours of the still look.
#[derive(Clone, Copy)]
pub struct Flat {
    pub sky: Color,
    pub buildings: Color,
    pub windows: Color,
    pub ink: Color,
    pub skin: Color,
    pub coat: Color,
    pub busy: Color,
    pub dim: Color,
    pub danger: Color,
    /// The panel's background, for the spy's collar.
    pub bg: Color,
}

/// Buildings along the bottom of a `w`×`h` sky, none taller than `tallest`, the same every time
/// for the same width. One pixel here is two points on screen.
pub fn paint_skyline(px: &mut Px, tallest: i32, c: &Flat) {
    px.fill(c.sky);
    let (w, h) = (px.w, px.h);
    let mut x = 0;
    let mut n = 0.;
    while x < w {
        n += 1.;
        let bw = 5 + (hash(n, 1., 7.) * 9.) as i32;
        let bh = (tallest as f64 * (0.45 + hash(n, 2., 7.) * 0.55)).round() as i32;
        for yy in (h - bh)..h {
            for xx in x..(x + bw).min(w) {
                px.set(xx as f64, yy as f64, c.buildings);
            }
        }
        // A few lit windows, on a grid of every other pixel.
        let mut wy = h - bh + 2;
        while wy < h - 1 {
            let mut wx = x + 1;
            while wx < x + bw - 1 {
                if hash(wx as f64, wy as f64, 3.) < 0.16 {
                    px.set(wx as f64, wy as f64, c.windows);
                }
                wx += 2;
            }
            wy += 2;
        }
        x += bw + (hash(n, 3., 7.) * 2.) as i32;
    }
}

/// The spy's size in pixels.
pub const SPY_W: i32 = 11;
pub const SPY_H: i32 = 13;

/// A spy in a hat and shades, 11×13 pixels, its coat showing the machine's state: coral at work,
/// grey asleep, red on an error, faint while it's made or gone.
pub fn paint_flat_spy(px: &mut Px, state: MachineState, c: &Flat) {
    px.fill(CLEAR);
    let faint = matches!(state, MachineState::Asleep | MachineState::Sleeping | MachineState::Missing | MachineState::Unknown);
    let coat = match state {
        MachineState::Working => c.busy,
        MachineState::Error => c.danger,
        MachineState::Creating | MachineState::Waking => c.coat,
        _ if faint => c.dim,
        _ => c.coat,
    };
    let (ink, skin) = if faint { (c.dim, c.dim) } else { (c.ink, c.skin) };
    let mut rect = |x: i32, y: i32, w: i32, h: i32, col: Color| {
        for yy in y..y + h {
            for xx in x..x + w {
                px.set(xx as f64, yy as f64, col);
            }
        }
    };
    rect(3, 0, 4, 2, ink); // hat
    rect(1, 2, 8, 1, ink); // brim
    rect(3, 3, 4, 2, skin); // face
    rect(3, 4, 4, 1, ink); // shades
    rect(2, 5, 6, 6, coat); // coat
    rect(4, 5, 2, 3, c.bg); // collar
    rect(3, 11, 1, 2, ink); // legs
    rect(6, 11, 1, 2, ink);
    if matches!(state, MachineState::Asleep | MachineState::Sleeping) {
        // A small "z" by the hat.
        rect(8, 0, 3, 1, c.dim);
        rect(9, 1, 1, 1, c.dim);
        rect(8, 2, 3, 1, c.dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Flat {
        Flat { sky: 1, buildings: 2, windows: 3, ink: 4, skin: 5, coat: 6, busy: 7, dim: 8, danger: 9, bg: 10 }
    }

    #[test]
    fn colours_from_tokens() {
        assert_eq!(color(gpui_kit::rgb(0xff8a5c)), 0xff8a5cff);
    }

    #[test]
    fn the_skyline_fills_the_width_and_stays_low() {
        let mut px = Px::new(190, 17);
        paint_skyline(&mut px, 12, &flat());
        // Buildings reach the right edge; nothing above the tallest allowed.
        assert_ne!(px.data[(16 * 190 + 189) as usize], 1);
        assert!(px.data[..(4 * 190) as usize].iter().all(|&c| c == 1));
        // The same width draws the same skyline.
        let mut again = Px::new(190, 17);
        paint_skyline(&mut again, 12, &flat());
        assert_eq!(px.data, again.data);
    }

    #[test]
    fn a_spy_wears_its_state() {
        let coat_at = |state| {
            let mut px = Px::new(SPY_W, SPY_H);
            paint_flat_spy(&mut px, state, &flat());
            px.data[(8 * SPY_W + 2) as usize]
        };
        assert_eq!(coat_at(MachineState::Working), 7);
        assert_eq!(coat_at(MachineState::Idle), 6);
        assert_eq!(coat_at(MachineState::Asleep), 8);
        assert_eq!(coat_at(MachineState::Error), 9);
    }
}
