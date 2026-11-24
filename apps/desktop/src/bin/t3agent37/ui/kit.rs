//! The panel's parts: square buttons with 1 px frames,
//! pixel icons on an 8×8 grid, captions in Plex Mono, switches, sections and lines.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{MONO, Tokens};

// ── buttons ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Solid,
    Outline,
    Danger,
    Quiet,
    /// Quiet, in the danger colour: "Delete machine…".
    QuietDanger,
    /// A square icon button.
    Icon,
    /// The settings button on the sky.
    OnSky,
    /// A square icon button on the still header's night.
    SkyIcon,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    Sm,
    Md,
    /// Full width, 40 tall: a view's main action.
    Lg,
}

/// A button's box; the caller adds its content and, unless `disabled`, its click.
pub fn button(id: impl Into<ElementId>, kind: Btn, size: Size, disabled: bool, tk: &Tokens) -> Stateful<Div> {
    let (h, pad, text) = match size {
        Size::Sm => (26., 9., 12.),
        Size::Md => (28., 10., 12.5),
        Size::Lg => (40., 10., 14.),
    };
    let tk = *tk;
    let b = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(h))
        .px(px(pad))
        .border_1()
        .border_color(transparent_black())
        .text_size(px(text))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .overflow_hidden()
        .when(size == Size::Lg, |b| b.w_full())
        .when(disabled, |b| b.opacity(0.5))
        .when(!disabled, |b| b.cursor_pointer());
    let live = !disabled;
    match kind {
        Btn::Solid => b.bg(tk.accent).text_color(tk.accent_fg).when(live, |b| b.hover(move |s| s.bg(tk.accent_strong))),
        Btn::Outline => b.border_color(tk.frame).text_color(tk.accent_text).when(live, |b| b.hover(move |s| s.bg(tk.panel))),
        Btn::Danger => b.bg(tk.danger).text_color(rgb(0xffffff)),
        Btn::Quiet => b
            .border_color(tk.line)
            .text_color(tk.muted)
            .when(live, |b| b.hover(move |s| s.text_color(tk.fg).border_color(tk.frame))),
        Btn::QuietDanger => b.border_color(tk.line).text_color(tk.danger).when(live, |b| b.hover(move |s| s.border_color(tk.danger))),
        Btn::Icon => b.w(px(28.)).px_0().text_color(tk.muted).when(live, |b| b.hover(move |s| s.bg(tk.panel).text_color(tk.fg))),
        Btn::OnSky => b
            .w(px(28.))
            .px_0()
            .text_color(rgb(0xffffff))
            .border_color(rgba(0xffffff73))
            .hover(|s| s.bg(rgba(0xffffff29))),
        Btn::SkyIcon => b.w(px(28.)).px_0().text_color(tk.sky_muted).when(live, |b| b.hover(|s| s.bg(rgba(0xffffff1f)))),
    }
}

/// An underlined text button in the accent: "Sign in", "Get a key".
pub fn link(id: impl Into<ElementId>, text: impl Into<SharedString>, disabled: bool, tk: &Tokens) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .p(px(4.))
        .text_color(tk.accent_text)
        .font_weight(FontWeight::NORMAL)
        .whitespace_nowrap()
        .underline()
        .text_decoration_color(tk.tone_300)
        .when(disabled, |b| b.opacity(0.5))
        .when(!disabled, |b| b.cursor_pointer())
        .child(text.into())
}

// ── pixel icons ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Sliders,
    More,
    Plus,
    /// The plus turned 45°: an open "Add a project…".
    Close,
    Back,
    Check,
    Chevron,
    ChevronUp,
    Open,
    Alert,
    /// Corners pointing out: a bigger panel.
    Expand,
    /// Corners pointing in: back to the standard size.
    Shrink,
}

fn rows(icon: Icon) -> [&'static str; 8] {
    match icon {
        Icon::Sliders => ["........", "..##....", "########", "..##....", ".....##.", "########", ".....##.", "........"],
        Icon::More => ["........", "........", "........", "##.##.##", "##.##.##", "........", "........", "........"],
        Icon::Plus => ["........", "...##...", "...##...", ".######.", ".######.", "...##...", "...##...", "........"],
        Icon::Close => ["........", ".##..##.", ".######.", "..####..", "..####..", ".######.", ".##..##.", "........"],
        Icon::Back => ["...#....", "..##....", ".#######", "########", ".#######", "..##....", "...#....", "........"],
        Icon::Check => ["........", ".......#", "......##", "#....##.", "##..##..", ".####...", "..##....", "........"],
        Icon::Chevron => ["........", "........", "##....##", ".##..##.", "..####..", "...##...", "........", "........"],
        Icon::ChevronUp => ["........", "........", "...##...", "..####..", ".##..##.", "##....##", "........", "........"],
        Icon::Open => ["..######", "......##", ".....###", "....##.#", "...##..#", "..##....", ".##.....", "##......"],
        Icon::Alert => ["...##...", "...##...", "...##...", "...##...", "...##...", "........", "...##...", "...##..."],
        Icon::Expand => ["####....", "##......", "#.#.....", "#..#....", "....#..#", ".....#.#", "......##", "....####"],
        Icon::Shrink => ["#..#....", ".#.#....", "..##....", "####....", "....####", "....##..", "....#.#.", "....#..#"],
    }
}

/// A pixel icon `size` points square, each cell's edges on device pixels.
pub fn icon(icon: Icon, size: f32, color: impl Into<Hsla>) -> impl IntoElement {
    let color = color.into();
    let rows = rows(icon);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let scale = window.scale_factor();
            let snap = |v: f32| (v * scale).round() / scale;
            let cell = f32::from(bounds.size.width) / 8.;
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            for (y, row) in rows.iter().enumerate() {
                for (x, c) in row.bytes().enumerate() {
                    if c != b'#' {
                        continue;
                    }
                    let (l, t) = (snap(x0 + cell * x as f32), snap(y0 + cell * y as f32));
                    let (r, b) = (snap(x0 + cell * (x + 1) as f32), snap(y0 + cell * (y + 1) as f32));
                    window.paint_quad(fill(Bounds::new(point(px(l), px(t)), gpui_kit::size(px(r - l), px(b - t))), color));
                }
            }
        },
    )
    .size(px(size))
    .flex_none()
}

// ── little animations, on the panel's 12 fps clock ──────────────────

/// Three blinking squares: something is running.
pub fn dots(t: f64, color: impl Into<Hsla>) -> Div {
    let color = color.into();
    div().flex().flex_none().items_center().gap(px(2.)).children((0..3).map(|i| {
        let phase = (t - i as f64 * 0.2).rem_euclid(1.2) / 1.2;
        div().size(px(3.)).bg(color).opacity(if phase >= 0.3 { 1. } else { 0.25 })
    }))
}

/// The state dot's blink: lit 60% of 1.4 s, dim the rest.
pub fn blink(t: f64) -> f32 {
    if t.rem_euclid(1.4) / 1.4 < 0.6 { 1. } else { 0.2 }
}

/// A loading placeholder bar.
pub fn skel(t: f64, w: impl Into<Length>, h: f32, tk: &Tokens) -> Div {
    let phase = t.rem_euclid(1.2) / 1.2;
    let o = if phase < 0.25 || phase >= 0.75 { 1. } else { 0.45 };
    div().flex_none().w(w.into()).h(px(h)).bg(tk.line).opacity(o)
}

// ── text ─────────────────────────────────────────────────────────────

/// Plex Mono 10.5, muted: labels.
pub fn caption(tk: &Tokens) -> Div {
    div().font_family(MONO).text_size(px(10.5)).line_height(relative(1.2)).text_color(tk.muted)
}

/// 11.5, muted: hints under fields.
pub fn hint(text: impl Into<SharedString>, tk: &Tokens) -> Div {
    div().text_size(px(11.5)).line_height(relative(1.4)).text_color(tk.muted).child(text.into())
}

/// A number in a box after a caption: "machines [3]".
pub fn count(n: usize, tk: &Tokens) -> Div {
    div()
        .flex_none()
        .min_w(px(16.))
        .ml(px(4.))
        .px(px(3.))
        .py(px(1.))
        .flex()
        .justify_center()
        .border_1()
        .border_color(tk.line)
        .text_color(tk.accent_text)
        .child(n.to_string())
}

/// A small boxed word: the machine's size.
pub fn chip(text: impl Into<SharedString>, tk: &Tokens) -> Div {
    div()
        .flex_none()
        .px(px(4.))
        .py(px(1.))
        .font_family(MONO)
        .text_size(px(10.))
        .line_height(relative(1.2))
        .text_color(tk.muted)
        .border_1()
        .border_color(tk.line)
        .child(text.into())
}

// ── controls ─────────────────────────────────────────────────────────

/// An on/off switch, 38×22.
pub fn switch(id: impl Into<ElementId>, on: bool, disabled: bool, tk: &Tokens) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(38.))
        .h(px(22.))
        .border_1()
        .border_color(if on { tk.accent } else { tk.frame })
        .bg(if on { tk.accent } else { tk.panel })
        .when(disabled, |s| s.opacity(0.5))
        .when(!disabled, |s| s.cursor_pointer())
        .child(div().absolute().top(px(3.)).left(px(if on { 19. } else { 3. })).size(px(14.)).bg(if on { tk.accent_fg } else { tk.tone_400 }))
}

/// "caption ────── extra": a section's head.
pub fn section_head(title: &str, n: Option<usize>, extra: Option<AnyElement>, tk: &Tokens) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(20.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(relative(1.2))
                .text_color(tk.fg)
                .child(title.to_string())
                .children(n.filter(|n| *n > 0).map(|n| count(n, tk))),
        )
        .child(div().flex_1().h(px(1.)).bg(tk.line))
        .children(extra)
}

/// A section: its head, then `body`.
pub fn section(title: &str, n: Option<usize>, extra: Option<AnyElement>, body: impl IntoElement, tk: &Tokens) -> Div {
    div().flex().flex_col().flex_none().min_w_0().child(section_head(title, n, extra, tk)).child(body)
}

/// Lines 34 tall, ruled between.
pub fn lines(items: Vec<AnyElement>, tk: &Tokens) -> Div {
    let line = tk.line;
    div().flex().flex_col().children(items.into_iter().enumerate().map(move |(i, item)| {
        div().when(i > 0, |d| d.border_t_1().border_color(line)).child(item)
    }))
}

pub fn line() -> Div {
    div().flex().items_center().gap(px(8.)).min_w_0().min_h(px(34.))
}

/// "✓ signed in": a quiet yes in the accent.
pub fn ok(text: impl Into<SharedString>, tk: &Tokens) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(tk.ok)
        .child(icon(Icon::Check, 10., tk.ok))
        .child(text.into())
}

/// The rooftop's 4 px hard shadow under menus and toasts.
pub fn hard_shadow(tk: &Tokens) -> Vec<BoxShadow> {
    vec![BoxShadow { color: tk.shadow.into(), offset: point(px(4.), px(4.)), blur_radius: px(0.), spread_radius: px(0.), inset: false }]
}

// ── coding agents' logos ─────────────────────────────────────────────

const CLAUDE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#D97757" fill-rule="evenodd" clip-rule="evenodd" d="M20.998 10.949H24v3.102h-3v3.028h-1.487V20H18v-2.921h-1.487V20H15v-2.921H9V20H7.488v-2.921H6V20H4.487v-2.921H3V14.05H0V10.95h3V5h17.998v5.949zM6 10.949h1.488V8.102H6v2.847zm10.51 0H18V8.102h-1.49v2.847z"/></svg>"##;

const CODEX_PATH: &str = "M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z";

static CLAUDE: LazyLock<Arc<Image>> = LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Svg, CLAUDE_SVG.as_bytes().to_vec())));
/// Codex's logo takes the text colour: one picture per colour asked for.
static CODEX: LazyLock<Mutex<HashMap<u32, Arc<Image>>>> = LazyLock::new(Default::default);

fn codex(color: Rgba) -> Arc<Image> {
    let hex = |c: f32| (c * 255.).round() as u32;
    let key = (hex(color.r) << 16) | (hex(color.g) << 8) | hex(color.b);
    CODEX
        .lock()
        .unwrap()
        .entry(key)
        .or_insert_with(|| {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#{key:06x}" fill-rule="evenodd" clip-rule="evenodd" d="{CODEX_PATH}"/></svg>"##
            );
            Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
        })
        .clone()
}

/// A coding agent's name led by its logo, as if part of the word. Keyed by the names the panel
/// shows: "Claude Code", the backend's "Claude" and "Codex". `color`: the text's.
pub fn agent_name(name: &str, size: f32, color: Rgba) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .whitespace_nowrap()
        .children(agent_logo(name, size, color).map(|l| l.mr(px(size * 0.2))))
        .child(name.to_string())
}

/// A coding agent's logo alone, `size` points square; none for an agent without one here.
pub fn agent_logo(name: &str, size: f32, color: Rgba) -> Option<Img> {
    let logo = match name {
        "Claude" | "Claude Code" => CLAUDE.clone(),
        "Codex" => codex(color),
        _ => return None,
    };
    Some(img(logo).size(px(size)).flex_none())
}

/// "Claude", "Claude and Codex", "A, B and C", logos included.
pub fn agent_names(names: &[String], size: f32, color: Rgba) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for (joiner, name) in crate::model::joiners(names.len()).zip(names) {
        if !joiner.is_empty() {
            out.push(div().whitespace_nowrap().child(joiner.replace(' ', "\u{a0}")).into_any_element());
        }
        out.push(agent_name(name, size, color).into_any_element());
    }
    out
}

// ── pixel pictures ───────────────────────────────────────────────────

/// An image painted 1:1 on device pixels, `w`×`h` points.
pub fn pixels(image: Arc<RenderImage>, w: f32, h: f32) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
        },
    )
    .w(px(w))
    .h(px(h))
    .flex_none()
}
