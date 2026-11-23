//! The rooftop's pixel engine: a small buffer of scene pixels, drawn with ordered dithering,
//! upscaled by whole device pixels for GPUI. First written in TypeScript for the old webview
//! panel; this port matched it byte for byte (86 pictures, 2026-11-20), so its rounding and
//! hashing keep JavaScript's arithmetic.

pub mod flat;
pub mod paint;
pub mod palette;
pub mod scene;
pub mod spy;

/// Straight RGBA, `0xRRGGBBAA`. `CLEAR` is transparent.
pub type Color = u32;
pub const CLEAR: Color = 0;

pub const fn rgb(hex: u32) -> Color {
    (hex << 8) | 0xff
}

pub struct Px {
    pub w: i32,
    pub h: i32,
    pub data: Vec<Color>,
}

impl Px {
    pub fn new(w: i32, h: i32) -> Self {
        Self { w, h, data: vec![CLEAR; (w * h) as usize] }
    }

    /// Coordinates rounded the way JavaScript's `Math.round` does (half up).
    pub fn set(&mut self, x: f64, y: f64, c: Color) {
        let x = (x + 0.5).floor() as i64;
        let y = (y + 0.5).floor() as i64;
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        self.data[(y * self.w as i64 + x) as usize] = c;
    }

    pub fn fill(&mut self, c: Color) {
        self.data.fill(c);
    }

    /// The buffer blown up `k` times (nearest neighbour), as BGRA bytes: what GPUI's
    /// `RenderImage` holds. Drawn 1:1 on device pixels, it stays crisp.
    pub fn to_bgra(&self, k: usize) -> Vec<u8> {
        let (w, h) = (self.w as usize, self.h as usize);
        let row = w * k * 4;
        let mut out = vec![0u8; row * h * k];
        for y in 0..h {
            let line = &mut out[y * k * row..(y * k + 1) * row];
            for x in 0..w {
                let c = self.data[y * w + x];
                let px = [(c >> 8) as u8, (c >> 16) as u8, (c >> 24) as u8, c as u8];
                for i in 0..k {
                    line[(x * k + i) * 4..(x * k + i) * 4 + 4].copy_from_slice(&px);
                }
            }
            for i in 1..k {
                out.copy_within(y * k * row..(y * k + 1) * row, (y * k + i) * row);
            }
        }
        out
    }
}

const BAYER: [f64; 16] = [0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.];

/// Ordered-dither threshold in [0,1) for pixel (x, y).
pub fn bayer(x: f64, y: f64) -> f64 {
    let (x, y) = (x as i64, y as i64);
    (BAYER[(((y & 3) * 4) + (x & 3)) as usize] + 0.5) / 16.
}

/// A colour from a ramp at t∈[0,1], ordered-dithered between steps.
pub fn ramp(colors: &[Color], t: f64, x: f64, y: f64) -> Color {
    let pos = t.clamp(0., 1.) * (colors.len() - 1) as f64;
    let i = pos.floor() as usize;
    if i >= colors.len() - 1 {
        return colors[colors.len() - 1];
    }
    if pos - i as f64 > bayer(x, y) { colors[i + 1] } else { colors[i] }
}

/// Deterministic hash → [0,1), bit for bit JavaScript's `Math.imul` and `>>>` version.
pub fn hash(a: f64, b: f64, c: f64) -> f64 {
    let (a, b, c) = (a as i64 as i32, b as i64 as i32, c as i64 as i32);
    let mut h = a.wrapping_mul(374761393) ^ b.wrapping_mul(668265263) ^ c.wrapping_mul(2147483647);
    h = (h ^ ((h as u32 >> 13) as i32)).wrapping_mul(1274126177);
    h ^= (h as u32 >> 16) as i32;
    h as u32 as f64 / 4294967296.
}

pub fn hash2(a: f64, b: f64) -> f64 {
    hash(a, b, 0.)
}

/// Per-agent phase from its machine key (UTF-16 units, as JavaScript strings).
pub fn seed_of(key: &str) -> f64 {
    let mut h: i32 = 0;
    for unit in key.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h as u32 as f64 / 4294967296.
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MachineState {
    Working,
    Idle,
    Sleeping,
    Asleep,
    Waking,
    Creating,
    Error,
    Missing,
    Unknown,
}

impl MachineState {
    /// The tag over its agent, as `stateTag`.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Sleeping => "dozing off",
            Self::Asleep => "zzz",
            Self::Waking => "waking",
            Self::Creating => "building",
            Self::Error => "error",
            Self::Missing => "gone",
            Self::Unknown => "…",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_js() {
        // the TypeScript original's results
        assert_eq!(hash(3., 4., 0.), 0.8613321061711758);
        assert_eq!(hash(-7., 12., 5.), 0.9729832888115197);
    }

    #[test]
    fn seed_matches_js() {
        assert_eq!(seed_of("skiff"), 0.025494192028418183);
    }
}
