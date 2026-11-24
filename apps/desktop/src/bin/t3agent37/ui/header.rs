//! The panel's top. By default a still night skyline: the brand, a one-line summary of the
//! machines and the panel's buttons on the list; a back button and the view's title on the
//! others. The animated rooftop (`rooftop.rs`) is a setting.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Instant;
use t3agent37_lib::app::{MachineState as State, Setup};
use t3agent37_lib::store::PanelSize;

use super::kit::{self, Btn, Icon, Size};
use super::{Panel, View, rooftop};
use crate::pixel::flat::{Flat, color, paint_skyline};
use crate::pixel::{Px, Theme};
use crate::theme::{DISPLAY, MONO, Tokens};

/// The still header's heights: on the list, and on the other views.
pub const STILL_FULL: f32 = 76.;
pub const STILL_COMPACT: f32 = 44.;
/// The skyline's height along the bottom, in points: on the list, and on the other views.
const SKY_FULL: f32 = 30.;
const SKY_COMPACT: f32 = 18.;

/// The still look's colours, from the theme.
pub fn flat(tk: &Tokens) -> Flat {
    Flat {
        sky: color(tk.sky),
        buildings: color(tk.sky_buildings),
        windows: color(tk.sky_windows),
        ink: color(tk.spy_ink),
        skin: color(tk.skin),
        coat: color(tk.coat),
        busy: color(tk.accent),
        dim: color(tk.tone_400),
        danger: color(tk.danger),
        bg: color(tk.panel),
    }
}

/// "1 working · 2 idle · 1 asleep": the machines by what they're doing, busiest first.
pub fn summary(states: &[State]) -> Vec<(usize, &'static str)> {
    let count = |want: &[State]| states.iter().filter(|s| want.contains(s)).count();
    [
        (count(&[State::Working]), "working"),
        (count(&[State::Error]), "need a look"),
        (count(&[State::Waking, State::Creating]), "starting"),
        (count(&[State::Idle]), "idle"),
        (count(&[State::Asleep, State::Sleeping]), "asleep"),
        (count(&[State::Missing]), "gone"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .collect()
}

impl Panel {
    /// The header's height for `view`, in the look the settings ask for.
    pub(super) fn strip_target(&self, view: &View) -> f32 {
        let compact = *view != View::List && self.state.setup == Setup::Ready;
        match (self.state.settings.animated_rooftop, compact) {
            (true, false) => rooftop::FULL,
            (true, true) => rooftop::COMPACT,
            (false, false) => STILL_FULL,
            (false, true) => STILL_COMPACT,
        }
    }

    pub(super) fn render_strip(&mut self, theme: Theme, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let animated = self.state.settings.animated_rooftop;
        // The look changed in Settings: the header takes its height at once.
        if self.rooftop_on != Some(animated) {
            let to = self.strip_target(&self.view.clone());
            self.strip = (to, to, Instant::now());
            self.rooftop_on = Some(animated);
        }
        if animated {
            self.render_rooftop(theme, tk, window, cx).into_any_element()
        } else {
            self.render_still(tk, window, cx).into_any_element()
        }
    }

    /// The panel at another size, and remembered.
    pub(super) fn set_panel_size(&mut self, size: PanelSize, window: &mut Window, cx: &mut Context<Self>) {
        if size == self.state.settings.panel_size {
            return;
        }
        self.change_settings(|s| s.panel_size = size, cx);
        let (w, h) = size.dims();
        crate::macos::present_panel(window, w, h);
    }

    fn render_still(&mut self, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let height = self.strip_height();
        if height != self.strip.1 {
            window.request_animation_frame();
        }
        let ready = self.state.setup == Setup::Ready;
        let compact = ready && self.view != View::List;
        let width = f32::from(window.viewport_size().width);

        // The skyline: two points a pixel, drawn for this width.
        let sky_h = if compact { SKY_COMPACT } else { SKY_FULL };
        let (pw, ph) = ((width / 2.).ceil() as i32, (sky_h / 2.) as i32);
        let mut buffer = Px::new(pw, ph);
        paint_skyline(&mut buffer, if compact { ph - 2 } else { ph - 3 }, &flat(&tk));
        let image = self.image(&buffer, (2. * window.scale_factor()).round() as usize);
        let sky = div().absolute().left_0().top(px(height - sky_h)).child(kit::pixels(image, pw as f32 * 2., sky_h));

        let content = if compact {
            let title = match &self.view {
                View::Machine(id) => self.state.machines.iter().find(|m| &m.id == id).map(|m| m.name.clone()).unwrap_or_default(),
                View::Settings => "Settings".into(),
                View::New => match self.new_form.as_ref().and_then(|f| f.copy_of.as_ref()) {
                    Some((_, name)) => format!("Copy of {name}"),
                    None => "New machine".into(),
                },
                View::List => String::new(),
            };
            div()
                .absolute()
                .top_0()
                .left(px(6.))
                .right(px(12.))
                .h(px(STILL_COMPACT - 6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    kit::button("back", Btn::SkyIcon, Size::Md, false, &tk)
                        .child(kit::icon(Icon::Back, 14., tk.sky_muted))
                        .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(DISPLAY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .text_color(tk.sky_fg)
                        .child(title),
                )
                .into_any_element()
        } else {
            let size = self.state.settings.panel_size;
            let bigger = size == PanelSize::Standard;
            let states: Vec<State> = self.state.machines.iter().map(|m| m.state).collect();
            let parts = summary(&states);
            let summary = div()
                .flex()
                .flex_wrap()
                .gap_x(px(5.))
                .font_family(MONO)
                .text_size(px(11.))
                .line_height(px(13.))
                .text_color(tk.sky_muted)
                .when(ready && parts.is_empty(), |d| d.child("no machines yet"))
                .children(parts.iter().enumerate().map(|(i, (n, word))| {
                    let text = if i == 0 { format!("{n} {word}") } else { format!("· {n} {word}") };
                    let c = match *word {
                        "working" => tk.sky_accent,
                        "need a look" => rgb(0xff8a9a),
                        _ => tk.sky_muted,
                    };
                    div().text_color(c).child(text)
                }));
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .pt(px(9.))
                .pl(px(16.))
                .pr(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex_1()
                                .font_family(DISPLAY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(17.))
                                .line_height(px(20.))
                                .text_color(tk.sky_fg)
                                .whitespace_nowrap()
                                .child("t3agent37"),
                        )
                        .when(ready, |row| {
                            row.child(
                                kit::button("panel-size", Btn::SkyIcon, Size::Md, false, &tk)
                                    .child(kit::icon(if bigger { Icon::Expand } else { Icon::Shrink }, 13., tk.sky_muted))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_panel_size(if bigger { PanelSize::Large } else { PanelSize::Standard }, window, cx)
                                    })),
                            )
                            .child(
                                kit::button("settings", Btn::SkyIcon, Size::Md, false, &tk)
                                    .child(kit::icon(Icon::Sliders, 14., tk.sky_muted))
                                    .on_click(cx.listener(|this, _, window, cx| this.go(View::Settings, window, cx))),
                            )
                        }),
                )
                .child(summary)
                .into_any_element()
        };

        div().relative().flex_none().w_full().h(px(height)).overflow_hidden().bg(tk.sky).child(sky).child(content)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui-kit's glob brings its own `test` attribute.
    use super::summary;
    use t3agent37_lib::app::MachineState as State;

    #[test]
    fn the_summary_counts_by_state_busiest_first() {
        let s = [State::Asleep, State::Working, State::Idle, State::Idle, State::Sleeping];
        assert_eq!(summary(&s), [(1, "working"), (2, "idle"), (2, "asleep")]);
        assert_eq!(summary(&[State::Error, State::Creating]), [(1, "need a look"), (1, "starting")]);
        assert!(summary(&[]).is_empty());
    }
}
