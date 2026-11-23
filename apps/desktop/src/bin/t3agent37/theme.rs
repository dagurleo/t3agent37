//! The brand's tokens and fonts: Graphite, light and dark.

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
    /// The still header's night: its sky, its buildings, their lit windows, and the text on it.
    pub sky: Rgba,
    pub sky_buildings: Rgba,
    pub sky_windows: Rgba,
    pub sky_fg: Rgba,
    pub sky_muted: Rgba,
    pub sky_accent: Rgba,
    /// The still spies: an idle one's coat, faces, and hats and shades.
    pub coat: Rgba,
    pub skin: Rgba,
    pub spy_ink: Rgba,
    /// "On", "signed in".
    pub ok: Rgba,
}

/// Graphite: neutral greys, coral where something is happening.
pub fn tokens(theme: Theme) -> Tokens {
    let base = Tokens {
        bg: rgb(0xfafafa),
        fg: rgb(0x18181b),
        muted: rgb(0x6b6b74),
        line: rgb(0xe8e8eb),
        frame: rgb(0xc4c4cb),
        panel: rgb(0xf2f2f4),
        accent: rgb(0xd4552b),
        accent_fg: rgb(0xffffff),
        accent_strong: rgb(0xb0441f),
        accent_text: rgb(0xc04c25),
        danger: rgb(0xc8243f),
        shadow: rgb(0xdcdce0),
        tag_bg: rgb(0xfafafa),
        tag_fg: rgb(0x18181b),
        tag_dim_bg: rgb(0xdcdce0),
        tag_dim_fg: rgb(0x4a4a52),
        tone_300: rgb(0xc4c4cb),
        tone_400: rgb(0xa1a1aa),
        ink: rgb(0x18181b),
        paper: rgb(0xfafafa),
        sky: rgb(0x1d1d21),
        sky_buildings: rgb(0x2c2c32),
        sky_windows: rgb(0xc9a587),
        sky_fg: rgb(0xededf0),
        sky_muted: rgb(0xb4b4bc),
        sky_accent: rgb(0xff9f78),
        coat: rgb(0x8f8f99),
        skin: rgb(0xe3b796),
        spy_ink: rgb(0x18181b),
        ok: rgb(0x2f8f4e),
    };
    match theme {
        Theme::Light => base,
        Theme::Dark => Tokens {
            bg: rgb(0x121214),
            fg: rgb(0xededf0),
            muted: rgb(0x9a9aa3),
            line: rgb(0x232327),
            frame: rgb(0x3f3f46),
            panel: rgb(0x19191c),
            accent: rgb(0xff8a5c),
            accent_fg: rgb(0x160d09),
            accent_strong: rgb(0xffb08f),
            accent_text: rgb(0xff9f78),
            danger: rgb(0xff5c7a),
            shadow: rgb(0x08080a),
            tag_bg: rgb(0xededf0),
            tag_fg: rgb(0x121214),
            tag_dim_bg: rgb(0x2c2c32),
            tag_dim_fg: rgb(0xa1a1aa),
            tone_300: rgb(0x4a4a52),
            tone_400: rgb(0x6b6b74),
            sky: rgb(0x18181b),
            sky_buildings: rgb(0x26262b),
            sky_windows: rgb(0xb8977c),
            coat: rgb(0x9e9ea8),
            spy_ink: rgb(0x0a0a0b),
            ok: rgb(0x7fcf95),
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
