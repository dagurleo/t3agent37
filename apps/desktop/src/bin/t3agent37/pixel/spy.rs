//! The field agent, : hat, sunglasses, trench coat. Sprite space is 8 px wide and
//! 14 tall, facing right; the feet stand on roof row `wy`.

use std::f64::consts::PI;

use super::palette::{Palette, SpyColors};
use super::scene::js_round;
use super::{Color, MachineState, Px, bayer};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    Stand,
    WalkA,
    WalkB,
    Perch,
    Sit,
}

#[derive(Clone)]
pub struct Pose {
    pub frame: Frame,
    pub dx: f64,
    pub flip: bool,
    pub dim: bool,
    pub ghost: bool,
    pub drop: f64,
    pub rope: bool,
    pub alarm: bool,
    pub zzz: Vec<f64>,
}

const BASE: Pose = Pose { frame: Frame::Stand, dx: 0., flip: false, dim: false, ghost: false, drop: 0., rope: false, alarm: false, zzz: Vec::new() };

fn clamp01(v: f64) -> f64 {
    v.clamp(0., 1.)
}
fn ease(v: f64) -> f64 {
    1. - (1. - v) * (1. - v)
}
fn cycle(t: f64, rise: f64, period: f64) -> f64 {
    ease(clamp01((t % period) / rise))
}

const DROP_SECS: f64 = 3.;
const DROP_PERIOD: f64 = 4.2;

/// `t` in seconds (`None`: reduced motion's still frame); `room`: px it may pace either way.
pub fn pose_for(state: MachineState, t: Option<f64>, seed: f64, room: f64) -> Pose {
    let still = t.is_none();
    let time = t.unwrap_or(0.) + seed * 7.3;
    match state {
        MachineState::Working => {
            if still {
                return Pose { frame: Frame::WalkA, ..BASE };
            }
            let phase = time * 2. * PI / 14.;
            Pose {
                frame: if (time * 4.).floor() as i64 % 2 != 0 { Frame::WalkA } else { Frame::WalkB },
                dx: js_round(phase.sin() * room),
                flip: phase.cos() < 0.,
                ..BASE
            }
        }
        MachineState::Idle => Pose { flip: !still && time % 7. > 5., ..BASE },
        MachineState::Sleeping => {
            let p = if still { 0.5 } else { cycle(time, 1.8, 2.6) };
            Pose { frame: if p < 0.35 { Frame::Stand } else if p < 0.75 { Frame::Perch } else { Frame::Sit }, ..BASE }
        }
        MachineState::Asleep => Pose {
            frame: Frame::Sit,
            dim: true,
            zzz: if still { vec![0.3, 0.7] } else { [0., 0.5].iter().map(|k| (time / 3.6 + k) % 1.).collect() },
            ..BASE
        },
        MachineState::Waking => {
            let p = if still { 0.5 } else { cycle(time, 1.6, 2.4) };
            Pose { frame: if p < 0.35 { Frame::Sit } else if p < 0.75 { Frame::Perch } else { Frame::Stand }, ..BASE }
        }
        MachineState::Creating => {
            if still {
                return Pose { drop: 0.5, rope: true, ..BASE };
            }
            let p = clamp01((time % DROP_PERIOD) / DROP_SECS);
            Pose { drop: 1. - ease(p), rope: time % DROP_PERIOD < DROP_PERIOD - 0.6, ..BASE }
        }
        MachineState::Error => Pose { alarm: still || time % 1. < 0.6, ..BASE },
        MachineState::Missing => Pose { ghost: true, dim: true, ..BASE },
        MachineState::Unknown => Pose { dim: true, ..BASE },
    }
}

// H hat · h band · F face · f face shade · G glasses · C coat · T shirt · L legs · S shoes
// B briefcase · K crate · k crate shade
fn rows(frame: Frame) -> &'static [&'static str; 14] {
    match frame {
        Frame::Stand => &["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCC..", "..L.L...", "..S.S..."],
        Frame::WalkA => &["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCCBB", ".L...LBB", "S.....S."],
        Frame::WalkB => &["..HHH...", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "CCCCCCC.", "FCCCCCF.", ".CCCCC..", "...LL.BB", "...SS.BB"],
        Frame::Perch => &["........", "..HHH...", "..hhh...", ".HHHHHH.", "..FGG...", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "FCCCCCF.", "kCCCCLLL", "KKKK...L", "KkkK...L", "KKKK..SS"],
        Frame::Sit => &["........", "........", "..HHH...", "..hhh...", ".HHHHHH.", "..FFf...", ".CCTCC..", "CCCTCCC.", "CCCTCCC.", "FCCCCCF.", "kCCCCLLL", "KKKK...L", "KkkK...L", "KKKK..SS"],
    }
}

const W: f64 = 8.;
const H: f64 = 14.;

fn color(ch: u8, c: &SpyColors, pal: &Palette) -> Color {
    match ch {
        b'H' => c.hat,
        b'h' => c.band,
        b'F' => c.face,
        b'f' => c.face_shade,
        b'G' => c.glasses,
        b'C' => c.coat,
        b'T' => c.shirt,
        b'L' => c.legs,
        b'S' => c.shoes,
        b'B' => c.briefcase,
        b'K' => pal.crate_,
        _ => pal.crate_shade,
    }
}

const Z_SMALL: [&str; 4] = ["###", ".#.", "#..", "###"];
const Z_BIG: [&str; 5] = ["####", "..#.", ".#..", "#...", "####"];

/// The agent's centre, and the top of what's drawn over it, in buffer px.
#[derive(Clone, Copy)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
}

/// Draws an agent standing on row `wy`, centred on `cx`. `z_rise`: how far z's float up.
pub fn draw_spy(px: &mut Px, cx: f64, wy: f64, pal: &Palette, pose: &Pose, z_rise: f64) -> Anchor {
    let outfit = if pose.dim { &pal.spy_dim } else { &pal.spy };
    let ox = cx - W / 2. + pose.dx;
    let oy = wy - H - js_round(pose.drop * (wy + 2.));

    if pose.drop == 0. && !pose.ghost {
        let shade = pal.roof[(pal.roof.len() - 1).min(js_round(pal.roof_to) as usize + 1)];
        let half: i32 = if matches!(pose.frame, Frame::Sit | Frame::Perch) { 4 } else { 3 };
        for d in -half..=half {
            let x = cx + pose.dx + d as f64;
            if d.abs() < half || bayer(x, wy) > 0.5 {
                px.set(x, wy, shade);
            }
        }
    }

    let mut top = oy + H;
    for (r, row) in rows(pose.frame).iter().enumerate() {
        for (lx, ch) in row.bytes().enumerate() {
            if ch == b'.' {
                continue;
            }
            let lx = lx as f64;
            let x = ox + if pose.flip { W - 1. - lx } else { lx };
            let y = oy + r as f64;
            if !(pose.ghost && (x as i64 + y as i64) & 1 != 0) {
                px.set(x, y, color(ch, outfit, pal));
            }
            top = top.min(oy + r as f64);
        }
    }
    let hat_x = ox + if pose.flip { W - 4. } else { 3. };

    if pose.rope {
        let mut y = 0.;
        while y < top {
            px.set(hat_x, y, pal.rope);
            y += 1.;
        }
    }

    let mut anchor_y = top;
    if pose.alarm {
        for dy in [7., 6., 5., 3.] {
            px.set(hat_x, top - dy, pal.alarm);
            px.set(hat_x + 1., top - dy, pal.alarm);
        }
        anchor_y = top - 7.;
    }

    for &p in &pose.zzz {
        if p > 0.9 {
            continue;
        }
        let glyph: &[&str] = if p < 0.45 { &Z_SMALL } else { &Z_BIG };
        let gx = cx + 2. + js_round(p * 3.);
        let gy = top + 1. - js_round(p * z_rise) - glyph.len() as f64;
        for (r, row) in glyph.iter().enumerate() {
            for (c, ch) in row.bytes().enumerate() {
                if ch == b'#' {
                    px.set(gx + c as f64, gy + r as f64, pal.zzz);
                }
            }
        }
    }

    Anchor { x: cx + pose.dx, y: anchor_y }
}
