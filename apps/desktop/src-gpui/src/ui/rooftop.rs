//! The rooftop strip, as `Rooftop.tsx`: a field agent per machine with its tag, the brand
//! and the settings button on the sky. Sub-views get a compact strip that crops the sky;
//! a roof wider than the strip scrolls sideways and follows the agent being called out.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::Setup;

use super::kit::{self, Btn, Icon, Size};
use super::{Panel, STRIP_EASE, View};
use crate::model::{machine_key, pixel_state};
use crate::pixel::paint::{Agent, STRIP_W, paint_roof, roof_for};
use crate::pixel::{MachineState, Px, Theme};
use crate::theme::{DISPLAY, MONO, Tokens};

pub const FULL: f32 = 148.;
pub const COMPACT: f32 = 100.;
/// How close (points) the active agent may get to the strip's edge before it scrolls into view.
const FOLLOW_MARGIN: f32 = 40.;
/// Tags keep this far from the roof's ends, sliding off-centre from their agent if they must.
const TAG_INSET: f32 = 4.;

struct RoofAgent {
    key: String,
    name: String,
    state: MachineState,
    /// Tag text instead of the state word.
    label: Option<String>,
    /// Clickable: opens its details.
    pickable: bool,
}

/// `--strip-ease`, cubic-bezier(0.3, 0.7, 0.4, 1), near enough.
fn ease(p: f32) -> f32 {
    1. - (1. - p).powi(3)
}

impl Panel {
    pub(super) fn strip_height(&self) -> f32 {
        let (from, to, since) = self.strip;
        if self.reduce_motion {
            return to;
        }
        let p = (since.elapsed().as_secs_f32() / STRIP_EASE.as_secs_f32()).min(1.);
        from + (to - from) * ease(p)
    }

    pub(super) fn render_strip(&mut self, theme: Theme, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let height = self.strip_height();
        if height != self.strip.1 {
            window.request_animation_frame();
        }
        let ready = self.state.setup == Setup::Ready;
        let mut agents: Vec<RoofAgent> = if ready {
            let pickable = |m: &t3agent37_lib::app::MachineView| !m.id.is_empty() && m.state != t3agent37_lib::app::MachineState::Creating;
            self.state
                .machines
                .iter()
                .map(|m| RoofAgent { key: machine_key(m), name: m.name.clone(), state: pixel_state(m.state), label: None, pickable: pickable(m) })
                .collect()
        } else {
            vec![RoofAgent { key: "setup".into(), name: "t3agent37".into(), state: MachineState::Creating, label: None, pickable: false }]
        };
        if ready && self.view == View::New {
            let draft = self.new_form.as_ref().map(|f| f.name.read(cx).value().to_string()).unwrap_or_default();
            agents.push(RoofAgent {
                key: "__draft".into(),
                name: draft.clone(),
                state: MachineState::Creating,
                label: Some(if draft.is_empty() { "new".into() } else { draft }),
                pickable: false,
            });
        }
        let active: Option<String> = match &self.view {
            View::New => Some("__draft".into()),
            View::Machine(id) => Some(id.clone()),
            _ => self.hover.clone(),
        };
        let picking = ready && self.view != View::New;

        let dpr = window.scale_factor() as f64;
        let roof = roof_for(agents.len(), dpr);
        let mut buffer = Px::new(roof.w, roof.h);
        let painted: Vec<Agent> = agents.iter().map(|a| Agent { key: &a.key, state: a.state }).collect();
        let anchors = paint_roof(&mut buffer, &roof, theme, &painted, self.clock());
        let image = self.image(&buffer, (roof.scale * dpr).round() as usize);
        let scale = roof.scale as f32;
        let (world_w, world_h) = (roof.w as f32 * scale, roof.h as f32 * scale);
        let show_all_tags = agents.len() <= 4;

        // a wide roof follows the agent being called out
        if roof.scrolls && active != self.roof_followed {
            if let Some(i) = active.as_ref().and_then(|k| agents.iter().position(|a| &a.key == k)) {
                let x = roof.spots[i].cx as f32 * scale;
                let left = -f32::from(self.roof_scroll.offset().x);
                if x < left + FOLLOW_MARGIN || x > left + STRIP_W as f32 - FOLLOW_MARGIN {
                    let to = (x - STRIP_W as f32 / 2.).clamp(0., (world_w - STRIP_W as f32).max(0.));
                    self.roof_scroll.set_offset(point(px(-to), px(0.)));
                }
            }
        }
        self.roof_followed = active.clone();

        let tk = *tk;
        let mut tags = Vec::new();
        let mut hits = Vec::new();
        for (a, anchor) in agents.iter().zip(&anchors) {
            let is_active = active.as_deref() == Some(a.key.as_str());
            let x = anchor.x as f32 * scale;
            if show_all_tags || is_active || a.state == MachineState::Error {
                let text = a.label.clone().unwrap_or_else(|| if is_active { a.name.clone() } else { a.state.tag().into() });
                let dim = matches!(a.state, MachineState::Asleep | MachineState::Missing | MachineState::Unknown);
                let (bg, fg) = if is_active {
                    (tk.ink, tk.paper)
                } else if a.state == MachineState::Error {
                    (tk.danger, rgb(0xffffff))
                } else if dim {
                    (tk.tag_dim_bg, tk.tag_dim_fg)
                } else {
                    (tk.tag_bg, tk.tag_fg)
                };
                // Plex Mono is 0.6 em a character: centred on the agent, kept off the roof's ends.
                let w = text.chars().count() as f32 * 6. + 8.;
                let cx_ = x.clamp(w / 2. + TAG_INSET, (world_w - w / 2. - TAG_INSET).max(w / 2. + TAG_INSET));
                let bottom = (anchor.y as f32 - 1.) * scale;
                tags.push((
                    is_active,
                    div()
                        .absolute()
                        .left(px(cx_ - w / 2.))
                        .top(px(bottom - 13.))
                        .h(px(13.))
                        .px(px(4.))
                        .pt(px(2.))
                        .bg(bg)
                        .text_color(fg)
                        .font_family(MONO)
                        .text_size(px(10.))
                        .line_height(px(10.))
                        .whitespace_nowrap()
                        .child(text),
                ));
            }
            if picking && a.pickable {
                let key = a.key.clone();
                hits.push(
                    div()
                        .id(SharedString::from(format!("hit-{}", a.key)))
                        .absolute()
                        .left(px(x - 7. * scale))
                        .top(px(anchor.y as f32 * scale))
                        .w(px(14. * scale))
                        .h(px(17. * scale))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(m) = this.machine(&key) {
                                let id = m.id.clone();
                                this.go(View::Machine(id), window, cx);
                            }
                        })),
                );
            }
        }
        // the called-out tag on top
        tags.sort_by_key(|(active, _)| *active);

        let world = div()
            .absolute()
            .left_0()
            .top(px(height - world_h))
            .w(px(world_w))
            .h(px(world_h))
            .child(kit::pixels(image, world_w, world_h))
            .children(tags.into_iter().map(|(_, t)| t))
            .children(hits);

        let scroller = div()
            .id("roof-scroll")
            .absolute()
            .inset_0()
            .when(roof.scrolls, |s| {
                let handle = self.roof_scroll.clone();
                s.overflow_x_scroll().track_scroll(&self.roof_scroll).on_scroll(cx.listener(move |_, e: &ScrollEvent, _, cx| {
                    // a mouse wheel scrolls the roof sideways
                    let d = e.delta.pixel_delta(px(16.));
                    if d.y.abs() > d.x.abs() {
                        let max = (world_w - STRIP_W as f32).max(0.);
                        let x = (f32::from(handle.offset().x) + f32::from(d.y)).clamp(-max, 0.);
                        handle.set_offset(point(px(x), px(0.)));
                        cx.notify();
                    }
                }))
            })
            .child(world);

        // sky overlays ride the sky: compact crops 48 pt off the top, so they slide out with it
        let lift = FULL - height;
        let on_sky = lift < FULL - COMPACT;
        let brand = div()
            .absolute()
            .top(px(8. - lift))
            .left(px(10.))
            .px(px(6.))
            .py(px(4.))
            .bg(rgba(0x140e184d))
            .font_family(DISPLAY)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(21.))
            .line_height(px(21.))
            // (Or the shadow copy, 1 px narrower, wraps its last "7" onto a second line.)
            .whitespace_nowrap()
            .child(
                div()
                    .relative()
                    .child(div().absolute().left(px(1.)).top(px(1.)).text_color(rgba(0x0c081099)).child("t3agent37"))
                    .child(div().relative().text_color(rgb(0xffffff)).child("t3agent37")),
            );
        let settings = (ready && on_sky).then(|| {
            kit::button("settings", Btn::OnSky, Size::Md, false, &tk)
                .absolute()
                .top(px(10. - lift))
                .right(px(12.))
                .child(kit::icon(Icon::Sliders, 16., rgb(0xffffff)))
                .on_click(cx.listener(|this, _, window, cx| this.go(View::Settings, window, cx)))
        });

        div()
            .relative()
            .flex_none()
            .w(px(STRIP_W as f32))
            .h(px(height))
            .overflow_hidden()
            .child(scroller)
            .when(on_sky, |s| s.child(brand))
            .children(settings)
    }
}
