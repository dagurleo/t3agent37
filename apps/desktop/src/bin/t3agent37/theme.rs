//! The brand's tokens and fonts: Noir amber, light and dark.

use gpui_kit::{Rgba, Window, WindowAppearance, rgb};

use crate::pixel::Theme;

pub const BODY: &str = "Inter";
pub const DISPLAY: &str = "Funnel Display";
pub const MONO: &str = "IBM Plex Mono";

/// The panel's fonts, embedded (all OFL).
pub const FONTS: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium560.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/FunnelDisplay-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/IBMPlexMono-Medium.ttf"),
];

#[derive(Clone, Copy)]
pub struct Tokens {
    pub bg: Rgba,
    pub fg: Rgba,
    pub muted: Rgba,
    pub line: Rgba,
    pub frame: Rgba,
    pub panel: Rgba,
    pub accent: Rgba,
    pub accent_fg: Rgba,
    pub accent_strong: Rgba,
    pub accent_text: Rgba,
    pub danger: Rgba,
    pub shadow: Rgba,
    pub tag_bg: Rgba,
    pub tag_fg: Rgba,
    pub tag_dim_bg: Rgba,
    pub tag_dim_fg: Rgba,
    /// `--tone-300`: link underlines, scrollbars.
    pub tone_300: Rgba,
    /// `--tone-400`: the switch's knob when off.
    pub tone_400: Rgba,
    pub ink: Rgba,
    pub paper: Rgba,
}

pub fn tokens(theme: Theme) -> Tokens {
    let base = Tokens {
        bg: rgb(0xfdfaf6),
        fg: rgb(0x1b1622),
        muted: rgb(0x7a6872),
        line: rgb(0xf3e7da),
        frame: rgb(0x9b7a68),
        panel: rgb(0xfaf4ed),
        accent: rgb(0xc2581f),
        accent_fg: rgb(0xffffff),
        accent_strong: rgb(0x9c4314),
        accent_text: rgb(0xb4501c),
        danger: rgb(0xc8372b),
        shadow: rgb(0xe2cdb8),
        tag_bg: rgb(0xfdfaf6),
        tag_fg: rgb(0x1b1622),
        tag_dim_bg: rgb(0xe2cdb8),
        tag_dim_fg: rgb(0x4a3a4f),
        tone_300: rgb(0xc5a58a),
        tone_400: rgb(0x9b7a68),
        ink: rgb(0x1b1622),
        paper: rgb(0xfdfaf6),
    };
    match theme {
        Theme::Light => base,
        Theme::Dark => Tokens {
            bg: rgb(0x141117),
            fg: rgb(0xf6efe6),
            muted: rgb(0xc5a58a),
            line: rgb(0x2a2230),
            frame: rgb(0x5a4658),
            panel: rgb(0x1c1720),
            accent: rgb(0xffb547),
            accent_fg: rgb(0x141117),
            accent_strong: rgb(0xffd08a),
            accent_text: rgb(0xffc061),
            danger: rgb(0xff7a62),
            shadow: rgb(0x08060a),
            tag_bg: rgb(0xf6d3a4),
            tag_fg: rgb(0x141117),
            tag_dim_bg: rgb(0x33293a),
            tag_dim_fg: rgb(0xc5a58a),
            ..base
        },
    }
}

pub fn theme_of(window: &Window) -> Theme {
    match window.appearance() {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::Dark,
        _ => Theme::Light,
    }
}

/// gpui-kit's own theme, which its text input draws with: our ink, caret and selection.
pub fn apply_to_kit(tk: &Tokens, cx: &mut gpui_kit::App) {
    use gpui_kit::component::theme::Theme as Kit;
    let (fg, muted, caret) = (tk.fg.into(), tk.muted.into(), tk.accent_text.into());
    let selection = gpui_kit::Hsla { a: 0.28, ..tk.accent.into() };
    let kit = Kit::global(cx);
    if kit.foreground == fg && kit.caret == caret && kit.selection == selection {
        return;
    }
    Kit::update(cx, |kit| {
        kit.foreground = fg;
        kit.muted_foreground = muted;
        kit.caret = caret;
        kit.selection = selection;
        kit.background = tk.bg.into();
        kit.font_family = BODY.into();
        // Scrollbars: a square, quiet thumb on no track, in the panel's muted ink.
        let muted: gpui_kit::Hsla = tk.muted.into();
        kit.scrollbar = gpui_kit::Hsla { a: 0., ..muted };
        kit.scrollbar_thumb = gpui_kit::Hsla { a: 0.45, ..muted };
        kit.scrollbar_thumb_hover = gpui_kit::Hsla { a: 0.7, ..muted };
    });
}
