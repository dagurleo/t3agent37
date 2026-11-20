//! A machine's details, as `MachineDetails.tsx`: what it's doing and costs at a glance, power,
//! its agents' sign-ins, T3 Connect and the wake link, its projects, then size and delete
//! folded away; the main action at the foot.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::{MachineState as State, MachineView};

use super::kit::{self, Btn, Icon, Size};
use super::Panel;
use crate::backend::{Cmd, Details};
use crate::model::{self, Action, machine_key, pixel_state};
use crate::pixel::paint::{ICON_H, ICON_W, paint_spy_icon};
use crate::pixel::{Px, seed_of};
use crate::theme::{DISPLAY, MONO, Tokens};

impl Panel {
    pub(super) fn render_details(&mut self, m: &MachineView, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let key = machine_key(m);
        let pending = self.pending.get(&key).copied();
        let busy = pending.is_some();
        let gone = m.state == State::Missing;
        let d = self.details.clone().filter(|d| d.id == m.id);
        let latest = self.latest_thread.as_ref().filter(|(id, _)| *id == m.id).and_then(|(_, t)| t.clone());
        let primary = model::primary_action(m).unwrap_or_else(|| if m.paired { model::open_in(latest) } else { model::pair() });

        let mut body: Vec<AnyElement> = vec![self.glance(m, d.as_ref(), &tk, window).into_any_element()];
        if !gone {
            body.push(self.power(m, busy, &tk, cx).into_any_element());
        }
        body.push(self.agents(m, d.as_ref(), pending, &tk, cx).into_any_element());
        body.push(self.connect(m, d.as_ref(), pending, &tk, cx).into_any_element());
        if !(gone && m.projects.is_empty()) {
            body.push(self.projects(m, &tk, window, cx).into_any_element());
        }
        body.push(self.more(m, busy, &tk, cx).into_any_element());
        body.push(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .gap_x(px(6.))
                .gap_y(px(2.))
                .font_family(MONO)
                .text_size(px(10.))
                .line_height(relative(1.4))
                .text_color(tk.muted)
                .map(|meta| match &d {
                    Some(d) => meta
                        .child(format!("T3 {}", d.t3_version))
                        .child(div().text_color(tk.frame).child("·"))
                        .child(format!("created {}", model::short_date(d.created_at)))
                        .child(div().text_color(tk.frame).child("·"))
                        .child(m.id.clone()),
                    None => meta.child(kit::skel(self.t(), px(210.), 10., &tk)),
                })
                .into_any_element(),
        );

        let armed_key = format!("details:{key}");
        let armed = self.is_armed(&armed_key);
        let kind = if armed {
            Btn::Danger
        } else if primary.solid {
            Btn::Solid
        } else {
            Btn::Outline
        };
        let shows_open = matches!(primary.cmd, Cmd::Open { .. }) && !primary.solid;
        let t = self.t();
        let m2 = m.clone();
        let foot = div().flex_none().flex().flex_col().px(px(16.)).pt(px(10.)).pb(px(14.)).border_t_1().border_color(tk.line).child(
            kit::button("details-primary", kind, Size::Lg, busy, &tk)
                .when(busy, |b| b.opacity(1.))
                .map(|b| match pending {
                    Some(label) => b.child(kit::dots(t, if primary.solid { tk.accent_fg } else { tk.accent_text })).child(label),
                    None if armed => b.child("Confirm"),
                    None => b.child(primary.label.clone()).when(shows_open, |b| b.child(div().ml(px(2.)).child(kit::icon(Icon::Open, 10., tk.accent_text)))),
                })
                .when(!busy, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        if matches!(primary.cmd, Cmd::Delete { .. }) && !this.arm(&armed_key) {
                            cx.notify();
                            return;
                        }
                        this.track(&m2, primary.clone(), cx);
                    }))
                }),
        );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(sub_header(&m.name, &tk, cx))
            .child(
                div()
                    .id("details-body")
                    .flex()
                    .flex_col()
                    .gap(px(22.))
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .pt(px(4.))
                    .pb(px(16.))
                    .overflow_y_scroll()
                    .track_scroll(&self.details_scroll)
                    .children(body),
            )
            .child(foot)
    }

    /// What a user scans for first: is it working, what is it, what does it cost.
    fn glance(&mut self, m: &MachineView, d: Option<&Details>, tk: &Tokens, window: &mut Window) -> impl IntoElement + use<> {
        let detail = model::detail(m);
        // "Working · 2 threads · 3 subagents" → "Working", the rest underneath
        let mut parts = detail.split(" · ");
        let headline = parts.next().unwrap_or_default().to_string();
        let rest: Vec<&str> = parts.collect();
        let size = model::size_of(self.state.sizes, &m.size);
        let t = self.t();

        let mut icon = Px::new(ICON_W, ICON_H);
        paint_spy_icon(&mut icon, crate::theme::theme_of(window), pixel_state(m.state), seed_of(&machine_key(m)), self.clock());
        let icon = self.image(&icon, (3. * window.scale_factor()).round() as usize);

        let (head_color, rest_color) = match m.state {
            State::Working => (tk.accent_text, tk.accent_text),
            State::Error | State::Missing => (tk.danger, tk.muted),
            State::Asleep | State::Unknown => (tk.muted, tk.muted),
            _ => (tk.fg, tk.muted),
        };
        let dot = state_dot(m.state, t, tk);
        let stat = |label: &'static str, value: Option<String>, first: bool| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(12.))
                .pt(px(8.))
                .pb(px(9.))
                .when(!first, |s| s.border_l_1().border_color(tk.line))
                .child(kit::caption(tk).child(label))
                .child(match value {
                    Some(v) => div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(v),
                    None => div().h(px(19.6)).flex().items_center().child(kit::skel(t, px(44.), 12., tk)),
                })
        };
        div()
            .flex_none()
            .border_1()
            .border_color(tk.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .p(px(12.))
                    .child(div().flex_none().w(px(80.)).h(px(70.)).flex().items_center().justify_center().bg(tk.panel).child(kit::pixels(icon, 72., 63.)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .font_family(DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(17.))
                                    .line_height(relative(1.2))
                                    .text_color(head_color)
                                    .child(dot)
                                    .child(headline),
                            )
                            .when(!rest.is_empty(), |d| {
                                d.child(div().font_family(MONO).text_size(px(11.5)).line_height(relative(1.35)).text_color(rest_color).child(rest.join(" · ")))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x(px(6.))
                                    .gap_y(px(4.))
                                    .mt(px(6.))
                                    .font_family(MONO)
                                    .text_size(px(10.5))
                                    .line_height(relative(1.2))
                                    .text_color(tk.muted)
                                    .child(kit::chip(m.size.clone(), tk))
                                    .when_some(size, |d, s| d.child(s.spec).child(div().text_color(tk.accent_text).child(model::hourly(s.hourly)))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .bg(tk.panel)
                    .border_t_1()
                    .border_color(tk.line)
                    .child(stat("today", d.map(|d| model::money(d.cost_today)), true))
                    .child(stat("this month", d.map(|d| model::money(d.cost_month)), false))
                    .child(stat("awake today", d.map(|d| model::duration(d.awake_secs_today)), false)),
            )
    }

    fn power(&mut self, m: &MachineView, busy: bool, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let power = model::power_action(m);
        // optimistic, so the switch moves on click; the next state is the source of truth
        let on = self.power_want.unwrap_or(!m.keep_awake_forever);
        let m2 = m.clone();
        let toggle = div()
            .id("auto-sleep")
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .pt(px(8.))
            .pb(px(12.))
            .when(!busy, |t| t.cursor_pointer())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(div().font_weight(FontWeight(560.)).child("Auto sleep"))
                    .child(kit::hint(
                        if on { "Sleeps 5 min after agents finish; asleep bills disk only" } else { "Stays up and bills until you turn this back on" },
                        &tk,
                    )),
            )
            .child(kit::switch("auto-sleep-switch", on, busy, &tk))
            .when(!busy, |t| {
                t.on_click(cx.listener(move |this, _, _, cx| {
                    this.power_want = Some(!on);
                    this.track(&m2, model::auto_sleep(!on), cx);
                }))
            });

        let label = power.as_ref().map_or_else(|| model::sleep().label, |p| p.label.clone());
        let m3 = m.clone();
        let power_button = kit::button("power", Btn::Quiet, Size::Sm, busy || power.is_none(), &tk).child(label).when(!busy && power.is_some(), |b| {
            let power = power.clone().unwrap();
            b.on_click(cx.listener(move |this, _, _, cx| this.track(&m3, power.clone(), cx)))
        });
        let row = div()
            .flex()
            .items_start()
            .gap(px(8.))
            .pt(px(12.))
            .border_t_1()
            .border_color(tk.line)
            .child(power_button)
            .when(on, |r| r.child(self.stay_awake(m, busy, &tk, cx)));
        kit::section("power", None, None, div().flex().flex_col().child(toggle).child(row), &tk)
    }

    /// Like caffeinate: a one-time "stay up at least this long", which simply runs out.
    fn stay_awake(&mut self, m: &MachineView, busy: bool, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        if let Some(until) = m.keep_awake_until {
            let left = model::time_left(model::secs_left(until));
            let m = m.clone();
            return div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(26.))
                .pl(px(9.))
                .pr(px(4.))
                .bg(tk.panel)
                .border_1()
                .border_color(tk.line)
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(tk.accent_text)
                .child(div().size(px(7.)).bg(tk.accent_text).opacity(kit::blink(t)))
                .child(format!("Awake · {left} left"))
                .child(
                    kit::link("stop-awake", "Stop", busy, &tk)
                        .px(px(4.))
                        .py(px(2.))
                        .font_family(crate::theme::BODY)
                        .text_size(px(11.5))
                        .when(!busy, |l| l.on_click(cx.listener(move |this, _, _, cx| this.track(&m, model::keep_awake(0), cx)))),
                )
                .into_any_element();
        }
        let open = self.stay_awake_open;
        let options = open.then(|| {
            let mut seg = div().flex().border_1().border_color(tk.frame);
            for (i, min) in model::STAY_AWAKE.into_iter().enumerate() {
                let m = m.clone();
                seg = seg.child(
                    segment(("stay", i), i == 0, false, &tk)
                        .h(px(28.))
                        .px(px(12.))
                        .child(model::duration(min as u64 * 60))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.stay_awake_open = false;
                            this.track(&m, model::keep_awake(min), cx);
                        })),
                );
            }
            seg
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_w_0()
            .child(
                kit::button("stay-awake", Btn::Quiet, Size::Sm, busy, &tk)
                    .when(open, |b| b.text_color(tk.fg).border_color(tk.frame))
                    .child("Stay awake for…")
                    .child(kit::icon(if open { Icon::ChevronUp } else { Icon::Chevron }, 8., if open { tk.fg } else { tk.muted }))
                    .when(!busy, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            this.stay_awake_open = !this.stay_awake_open;
                            cx.notify();
                        }))
                    }),
            )
            .children(options)
            .into_any_element()
    }

    fn agents(&mut self, m: &MachineView, d: Option<&Details>, pending: Option<&'static str>, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let items: Vec<AnyElement> = match d {
            None => skeleton_lines(2, t, &tk),
            Some(d) => d
                .agents
                .iter()
                .map(|(name, signed_in)| {
                    let right: AnyElement = if *signed_in {
                        kit::ok("signed in", &tk).into_any_element()
                    } else if m.signed_out.contains(name) {
                        let action = model::sign_in_to(name);
                        let running = self.signing_in.as_deref() == Some(name.as_str()) && pending == Some(action.busy);
                        let disabled = pending.is_some() || m.state == State::Missing;
                        let (m, name2) = (m.clone(), name.clone());
                        kit::button(SharedString::from(format!("signin-{name}")), Btn::Outline, Size::Sm, disabled && !running, &tk)
                            .map(|b| if running { b.child(kit::dots(t, tk.accent_text)).child("Signing in…") } else { b.child("Sign in") })
                            .when(!disabled, |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    this.signing_in = Some(name2.clone());
                                    this.track(&m, action.clone(), cx);
                                }))
                            })
                            .into_any_element()
                    } else {
                        kit::caption(&tk).child("signed out").into_any_element()
                    };
                    kit::line()
                        .child(div().flex_1().min_w_0().child(kit::agent_name(name, 13., tk.fg)))
                        .child(right)
                        .into_any_element()
                })
                .collect(),
        };
        kit::section("agents", None, None, kit::lines(items, &tk), &tk)
    }

    /// T3 Connect: the user's phone or a browser reach the machine with their T3 account.
    fn connect(&mut self, m: &MachineView, d: Option<&Details>, pending: Option<&'static str>, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let disabled = pending.is_some() || m.state == State::Missing;
        let button = |this: &mut Self, id: &'static str, action: Action, cx: &mut Context<Self>| {
            let running = pending == Some(action.busy);
            let m = m.clone();
            let _ = this;
            kit::button(id, Btn::Outline, Size::Sm, disabled && !running, &tk)
                .map(|b| if running { b.child(kit::dots(t, tk.accent_text)).child(action.busy) } else { b.child(action.label.clone()) })
                .when(!disabled, |b| b.on_click(cx.listener(move |this, _, _, cx| this.track(&m, action.clone(), cx))))
        };
        let items: Vec<AnyElement> = match d {
            None => skeleton_lines(1, t, &tk),
            Some(d) => {
                let mut items = vec![kit::line()
                    .child(div().flex_1().min_w_0().child("T3 Connect"))
                    .child(if d.connect { kit::ok("on", &tk).into_any_element() } else { button(self, "connect", model::connect(), cx).into_any_element() })
                    .into_any_element()];
                if d.connect {
                    items.push(
                        kit::line().child(div().flex_1().min_w_0().child("Wake link")).child(button(self, "wake-link", model::wake_link(), cx)).into_any_element(),
                    );
                    let renew = model::new_wake_link();
                    let replacing = pending == Some(renew.busy);
                    let m2 = m.clone();
                    items.push(
                        line_note(&tk)
                            .child(format!(
                                "T3 Connect can't wake {}. Open this link on your phone first, then pick it in T3 Code. The link only wakes it; using it still takes your T3 sign-in.",
                                m.name
                            ))
                            .child(
                                kit::link("replace-link", if replacing { renew.busy } else { "Replace link" }, disabled, &tk)
                                    .px_0()
                                    .py(px(2.))
                                    .text_size(px(11.5))
                                    .when(!disabled, |l| l.on_click(cx.listener(move |this, _, _, cx| this.track(&m2, renew.clone(), cx)))),
                            )
                            .into_any_element(),
                    );
                } else {
                    items.push(line_note(&tk).child("T3 Code on your phone or in a browser, with your T3 account").into_any_element());
                }
                items
            }
        };
        kit::section("other devices", None, None, kit::lines(items, &tk), &tk)
    }

    /// Its T3 projects, and adding one the way T3 Code's "Add to another machine" does.
    fn projects(&mut self, m: &MachineView, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let gone = m.state == State::Missing;
        let mut items: Vec<AnyElement> = m
            .projects
            .iter()
            .map(|p| {
                kit::line()
                    .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                    .when_some(p.status.clone(), |l, status| {
                        l.child(
                            kit::caption(&tk)
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .max_w(relative(0.6))
                                .overflow_hidden()
                                .child(kit::dots(t, tk.muted))
                                .child(div().min_w_0().truncate().child(status)),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        if m.projects.is_empty() && !gone {
            items.push(kit::line().child(div().flex_1().text_color(tk.muted).child("Cloned from its git remote, .env files included")).into_any_element());
        }
        if !gone {
            let open = self.picker.is_some();
            let add = div()
                .id("add-project")
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .min_h(px(34.))
                .text_color(tk.accent_text)
                .text_size(px(12.5))
                .cursor_pointer()
                .child(kit::icon(if open { Icon::Close } else { Icon::Plus }, 10., tk.accent_text))
                .child(model::PICK_PROJECT)
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.picker.is_some() {
                        this.picker = None;
                        this.focus.focus(window, cx);
                    } else {
                        this.open_picker(window, cx);
                    }
                    cx.notify();
                }));
            let picker = open.then(|| self.render_picker(m, &tk, window, cx));
            items.push(div().flex().flex_col().child(add).children(picker).into_any_element());
        }
        kit::section("projects", Some(m.projects.len()), None, kit::lines(items, &tk), &tk)
    }

    /// The rare stuff: size and delete, folded away.
    fn more(&mut self, m: &MachineView, busy: bool, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let key = machine_key(m);
        let gone = m.state == State::Missing;
        let size = model::size_of(self.state.sizes, &m.size);
        let delete_error = self.delete_errors.get(&key).cloned();
        // a refused delete (from here or the list) must be visible
        let open = self.more_open || delete_error.is_some();
        let peek = if gone {
            "gone".to_string()
        } else {
            size.map_or(m.size.clone(), |s| format!("{} · {}", m.size, model::hourly(s.hourly)))
        };
        let head = div()
            .id("more")
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(px(20.))
            .child(kit::caption(&tk).child("machine"))
            .child(div().flex_1().h(px(1.)).bg(tk.line))
            .child(kit::caption(&tk).text_color(tk.accent_text).child(peek))
            .child(kit::icon(if open { Icon::ChevronUp } else { Icon::Chevron }, 8., tk.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.more_open = !open;
                cx.notify();
            }));
        let body = open.then(|| {
            let mut groups = div().flex().flex_col().gap(px(16.)).pt(px(12.));
            if !gone {
                let mut seg = div().flex().border_1().border_color(tk.frame);
                for (i, s) in self.state.sizes.iter().enumerate() {
                    let on = s.id == m.size;
                    let m2 = m.clone();
                    let id = s.id;
                    seg = seg.child(
                        segment(("size", i), i == 0, on, &tk)
                            .flex_col()
                            .gap(px(3.))
                            .py(px(7.))
                            .when(busy, |b| b.opacity(0.6))
                            .child(capitalize(s.id))
                            .child(div().text_size(px(9.5)).opacity(0.75).child(s.spec))
                            .child(div().text_size(px(9.5)).opacity(0.75).child(model::hourly(s.hourly)))
                            .when(!busy && !on, |b| b.on_click(cx.listener(move |this, _, _, cx| this.track(&m2, model::resize_to(id), cx)))),
                    );
                }
                groups = groups.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight(560.)).child("Size"))
                        .child(seg)
                        .child(kit::hint(model::resize_hint(m), &tk)),
                );
            }
            let armed_key = format!("delete:{key}");
            let armed = self.is_armed(&armed_key);
            let m2 = m.clone();
            let delete = kit::button("delete", if armed { Btn::Danger } else { Btn::QuietDanger }, Size::Sm, busy, &tk)
                .child(if armed { "Click again to delete forever" } else { "Delete machine…" })
                .when(!busy, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        if this.arm(&armed_key) {
                            this.track(&m2, model::delete(), cx);
                        }
                        cx.notify();
                    }))
                });
            let mut group = div().flex().flex_col().items_start().gap(px(8.)).child(delete);
            if let Some(error) = delete_error {
                group = group.child(self.delete_error_note(m, &error, busy, &tk, cx));
            }
            groups.child(group)
        });
        div().flex().flex_col().flex_none().child(head).children(body)
    }
}

/// Back button and title for sub-views. Esc goes back too (the panel's key handler).
pub(super) fn sub_header(title: &str, tk: &Tokens, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pl(px(10.))
        .pr(px(16.))
        .pt(px(10.))
        .pb(px(4.))
        .child(
            kit::button("back", Btn::Icon, Size::Md, false, tk)
                .child(kit::icon(Icon::Back, 16., tk.muted))
                .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(17.))
                .line_height(relative(1.2))
                .child(title.to_string()),
        )
}

/// One square pixel of status: lit = up, hollow = asleep, blinking = busy.
fn state_dot(state: State, t: f64, tk: &Tokens) -> Div {
    let dot = div().flex_none().size(px(7.));
    match state {
        State::Asleep | State::Unknown => dot.border_1().border_color(tk.muted),
        State::Error | State::Missing => dot.bg(tk.danger),
        State::Working | State::Waking | State::Sleeping => dot.bg(tk.accent_text).opacity(kit::blink(t)),
        _ => dot.bg(tk.accent_text),
    }
}

/// One choice of a segmented control.
pub(super) fn segment(id: impl Into<ElementId>, first: bool, on: bool, tk: &Tokens) -> Stateful<Div> {
    let panel = tk.panel;
    div()
        .id(id)
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(1.))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(relative(1.))
        .cursor_pointer()
        .when(!first, |s| s.border_l_1().border_color(tk.line))
        .when(on, |s| s.bg(tk.accent).text_color(tk.accent_fg))
        .when(!on, |s| s.hover(move |s| s.bg(panel)))
}

/// A sentence about the line above it: wraps instead of cutting off.
fn line_note(tk: &Tokens) -> Div {
    div().w_full().py(px(7.)).text_size(px(12.)).line_height(relative(1.4)).text_color(tk.muted).flex().flex_col().items_start()
}

fn skeleton_lines(n: usize, t: f64, tk: &Tokens) -> Vec<AnyElement> {
    (0..n).map(|i| kit::line().child(kit::skel(t, relative(0.4 - i as f32 * 0.1), 10., tk)).into_any_element()).collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
