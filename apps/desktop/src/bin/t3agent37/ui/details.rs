//! A machine's details: what it's doing and costs at a glance, power,
//! its agents' sign-ins, T3 Connect and the wake link, its projects, then size and delete
//! folded away; the main action at the foot.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::{GITHUB, MachineState as State, MachineView};
use t3agent37_lib::machine::issues::{self, IssueSettings, ModelChoice};
use t3agent37_lib::machine::projects::MachineProject;

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
        if !gone {
            body.push(self.previews(m, d.as_ref(), pending, &tk, window, cx).into_any_element());
        }
        if !(gone && m.projects.is_empty()) {
            body.push(self.projects(m, pending, &tk, window, cx).into_any_element());
        }
        if let Some(d) = d.as_ref().filter(|d| !d.webhooks.is_empty()) {
            body.push(webhooks(m, d, &tk, cx).into_any_element());
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
                // The picker scrolls this by hand (see `open_picker`), so it has its own handle;
                // the bar sits on top, outside what scrolls.
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("details-body")
                            .flex()
                            .flex_col()
                            .gap(px(22.))
                            .size_full()
                            .px(px(16.))
                            .pt(px(4.))
                            .pb(px(16.))
                            .overflow_y_scroll()
                            .track_scroll(&self.details_scroll)
                            .children(body),
                    )
                    .vertical_scrollbar(&self.details_scroll),
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
            Some(d) => {
                // A cancel is over once nothing runs (one started by an agent has no `track`).
                if m.signing_in.is_none() && pending.is_none() {
                    self.canceled_sign_in.remove(&machine_key(m));
                }
                let now = model::now_ms() as i64;
                let disabled = pending.is_some() || m.state == State::Missing;
                let mut items: Vec<AnyElement> = d
                    .agents
                    .iter()
                    .flat_map(|a| {
                        let name = &a.name;
                        // Its sign-in runs: as the machine says, or as just clicked here.
                        let mine = m.signing_in.as_deref() == Some(name.as_str())
                            || (self.signing_in.as_deref() == Some(name.as_str()) && pending.is_some_and(|p| p == model::SWITCHING || p == model::sign_in_to(name).busy));
                        let canceling = self.canceled_sign_in.contains(&machine_key(m));
                        let right: AnyElement = if mine {
                            // "Switching account…  Cancel": the old sign-in stays if it's canceled.
                            let (m2, label) = (m.clone(), if a.signed_in { model::SWITCHING } else { "Signing in…" });
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .font_family(MONO)
                                        .text_size(px(11.))
                                        .text_color(tk.accent_text)
                                        .child(kit::dots(t, tk.accent_text))
                                        .child(if canceling { "Canceling…" } else { label }),
                                )
                                .when(!canceling, |r| {
                                    r.child(
                                        kit::link(SharedString::from(format!("cancel-signin-{name}")), "Cancel", false, &tk)
                                            .text_size(px(11.5))
                                            .on_click(cx.listener(move |this, _, _, cx| this.cancel_sign_in(&m2, cx))),
                                    )
                                })
                                .into_any_element()
                        } else if a.signed_in {
                            kit::link(SharedString::from(format!("switch-{name}")), "Switch account", disabled, &tk)
                                .text_size(px(11.5))
                                .when(!disabled, |l| l.on_click(start_sign_in(m, name, model::switch_account(name), cx)))
                                .into_any_element()
                        } else if m.signed_out.contains(name) {
                            let (action, another) = (model::sign_in_to(name), model::sign_in_another(name));
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    kit::link(SharedString::from(format!("another-{name}")), "Other account", disabled, &tk)
                                        .text_size(px(11.5))
                                        .when(!disabled, |l| l.on_click(start_sign_in(m, name, another, cx))),
                                )
                                .child(
                                    kit::button(SharedString::from(format!("signin-{name}")), Btn::Outline, Size::Sm, disabled, &tk)
                                        .child("Sign in")
                                        .when(!disabled, |b| b.on_click(start_sign_in(m, name, action, cx))),
                                )
                                .into_any_element()
                        } else {
                            kit::caption(&tk).child("signed out").into_any_element()
                        };
                        // Who it's signed in as, and the plan's limits, under its name.
                        let who = a.signed_in.then(|| model::account_line(a)).flatten();
                        let usage = a.signed_in.then(|| model::usage_line(&a.account, now)).flatten();
                        let high = model::usage_high(&a.account, now);
                        let left = div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .py(px(7.))
                            .child(kit::agent_name(name, 13., tk.fg))
                            .when(a.signed_in && who.is_none(), |c| c.child(kit::ok("signed in", &tk)))
                            .children(who.map(|w| kit::caption(&tk).truncate().child(w)))
                            .children(usage.map(|u| kit::caption(&tk).when(high, |c| c.text_color(tk.accent_text)).child(u)));
                        let line = kit::line().child(left).child(right).into_any_element();
                        let code = (name == GITHUB).then(|| m.github_code.clone()).flatten();
                        std::iter::once(line).chain(code.map(|code| github_code(code, &tk, cx).into_any_element()))
                    })
                    .collect();
                if !items.is_empty() {
                    let how = if t3agent37_lib::app::private_window_available() {
                        "Other account and Switch account sign in from a private browser window."
                    } else {
                        "For Other account and Switch account, sign in on the page with the account this machine should use."
                    };
                    items.push(line_note(&tk).child(format!("Sign-ins are per machine, so each can use its own account. {how}")).into_any_element());
                }
                items
            }
        };
        kit::section("agents", None, None, kit::lines(items, &tk), &tk)
    }

    /// Its dev servers, and opening them in the browser here at localhost.
    fn previews(&mut self, m: &MachineView, d: Option<&Details>, pending: Option<&'static str>, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let Some(d) = d else {
            return kit::section("previews", None, None, kit::lines(skeleton_lines(1, t, &tk), &tk), &tk);
        };
        let disabled = pending.is_some() || m.state == State::Missing;
        // Forwarded ones first, in the order they were added; then the rest T3 sees listening.
        let process = |port: u16| d.servers.iter().find(|s| s.port == port).and_then(|s| s.process.clone());
        let mut rows: Vec<(u16, Option<u16>, Option<String>)> = d.previews.iter().map(|p| (p.port, Some(p.local_port), process(p.port))).collect();
        rows.extend(d.servers.iter().filter(|s| !d.previews.iter().any(|p| p.port == s.port)).map(|s| (s.port, None, s.process.clone())));
        let mut items: Vec<AnyElement> = Vec::new();
        for (port, local, process) in rows {
            let open = model::open_port(port);
            let running = pending == Some(open.busy) && self.opening_port == Some(port);
            let m2 = m.clone();
            let left = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .flex_1()
                .min_w_0()
                .child(div().flex_none().font_family(MONO).text_size(px(12.5)).child(format!(":{port}")))
                .when_some(process, |l, p| l.child(div().min_w_0().truncate().text_size(px(12.)).text_color(tk.muted).child(p)))
                .when(local.is_some_and(|l| l != port), |l| {
                    l.child(div().flex_none().font_family(MONO).text_size(px(10.5)).text_color(tk.accent_text).child(format!("here :{}", local.unwrap_or_default())))
                });
            let open_button = kit::button(("open-port", port as usize), Btn::Outline, Size::Sm, disabled && !running, &tk)
                .map(|b| if running { b.child(kit::dots(t, tk.accent_text)).child("Opening…") } else { b.child("Open").child(kit::icon(Icon::Open, 9., tk.accent_text)) })
                .when(!disabled, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.opening_port = Some(port);
                        this.track(&m2, model::open_port(port), cx);
                    }))
                });
            let m3 = m.clone();
            let close = local.map(|_| {
                kit::button(("close-port", port as usize), Btn::Icon, Size::Sm, disabled, &tk)
                    .child(kit::icon(Icon::Close, 9., tk.muted))
                    .when(!disabled, |b| b.on_click(cx.listener(move |this, _, _, cx| this.track(&m3, model::close_port(port), cx))))
            });
            items.push(kit::line().gap(px(6.)).child(left).child(open_button).children(close).into_any_element());
        }
        let awake = matches!(m.state, State::Working | State::Idle);
        let listed = !items.is_empty();
        if !listed {
            let text = if awake {
                "No servers from its agents or terminals yet. When one starts, it shows here.".to_string()
            } else {
                format!("Servers {} runs show here while it's awake.", m.name)
            };
            items.push(line_note(&tk).child(text).into_any_element());
        }
        // Any port by number: T3 lists only what its agents and terminals started.
        items.push(self.port_field(m, disabled, &tk, window, cx).into_any_element());
        if listed {
            items.push(
                line_note(&tk)
                    .child(format!(
                        "Opens in your browser at localhost, through T3. Opening a page wakes {}; one left open doesn't keep it awake.",
                        m.name
                    ))
                    .into_any_element(),
            );
        }
        kit::section("previews", None, None, kit::lines(items, &tk), &tk)
    }

    /// "+ Open another port…", and the field it opens: a port number, Enter opens it.
    fn port_field(&mut self, m: &MachineView, disabled: bool, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let Some((input, _)) = &self.port_input else {
            let m = m.clone();
            return div()
                .id("open-another-port")
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .min_h(px(34.))
                .text_color(tk.accent_text)
                .text_size(px(12.5))
                .when(!disabled, |r| r.cursor_pointer())
                .child(kit::icon(Icon::Plus, 10., tk.accent_text))
                .child("Open another port…")
                .when(!disabled, |r| r.on_click(cx.listener(move |this, _, window, cx| this.open_port_field(&m, window, cx))))
                .into_any_element();
        };
        let focused = input.focus_handle(cx).is_focused(window);
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(34.))
            .px(px(10.))
            .my(px(4.))
            .border_1()
            .border_color(if focused { tk.accent_text } else { tk.frame })
            .child(div().font_family(MONO).text_size(px(12.)).text_color(tk.accent_text).child(":"))
            .child(div().flex_1().min_w_0().font_family(MONO).text_size(px(12.)).child(Input::new(input).appearance(false).px_0().py_0().h(px(18.))))
            .child(kit::caption(&tk).child("enter opens it"))
            .into_any_element()
    }

    fn open_port_field(&mut self, m: &MachineView, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("3000"));
        let m = m.clone();
        let sub = cx.subscribe_in(&input, window, move |this: &mut Self, input, event: &InputEvent, window, cx| {
            if !matches!(event, InputEvent::PressEnter { .. }) {
                return;
            }
            match input.read(cx).value().trim().parse::<u16>() {
                Ok(port) if port > 0 => {
                    this.port_input = None;
                    this.focus.focus(window, cx);
                    this.opening_port = Some(port);
                    this.track(&m, model::open_port(port), cx);
                }
                _ => this.toast("A port is a number from 1 to 65535", true, cx),
            }
            cx.notify();
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.port_input = Some((input, sub));
        cx.notify();
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
                    let busy = pending.is_some() || m.state == State::Missing;
                    let want = self.connect_want;
                    let approval = want.filter(|(k, _)| *k == "approval").map_or(d.connect_approval.unwrap_or(false), |(_, on)| on);
                    let push = want.filter(|(k, _)| *k == "push").map_or(d.connect_push.unwrap_or(false), |(_, on)| on);
                    items.push(
                        connect_switch(
                            "approval",
                            "Approve new devices",
                            "A device that signs in to your T3 account waits until you approve it in T3 Code",
                            approval,
                            busy,
                            m,
                            model::connect_approval(!approval),
                            &tk,
                            cx,
                        )
                        .into_any_element(),
                    );
                    items.push(
                        connect_switch(
                            "push",
                            "Push to your phone",
                            "Agent activity as notifications and Live Activities in the T3 Code phone app, laptop closed or not",
                            push,
                            busy,
                            m,
                            model::connect_push(!push),
                            &tk,
                            cx,
                        )
                        .into_any_element(),
                    );
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

    /// Its T3 projects, each GitHub one with its issues switch and settings, and adding one the
    /// way T3 Code's "Add to another machine" does.
    fn projects(&mut self, m: &MachineView, pending: Option<&'static str>, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let gone = m.state == State::Missing;
        let busy = pending.is_some() || gone;
        let want = self.issues_want.clone();
        let choices = self.details.as_ref().filter(|d| d.id == m.id).map(|d| d.issue_models.clone()).unwrap_or_default();
        let costs = self.details.as_ref().filter(|d| d.id == m.id).map(|d| d.project_costs.clone()).unwrap_or_default();
        let on_github = |p: &MachineProject| p.status.is_none() && p.repo_key.as_deref().and_then(issues::github_repo).is_some();
        let mut items: Vec<AnyElement> = Vec::new();
        for (i, p) in m.projects.iter().enumerate() {
            let on = want.as_ref().filter(|(id, _)| *id == p.id).map_or(p.issues.is_some(), |(_, on)| *on);
            let form_here = self.issues_form.as_ref().is_some_and(|f| f.machine == m.id && f.project == p.id);
            let cost = costs.iter().find(|c| c.project_id.as_deref() == Some(p.id.as_str())).filter(|_| p.status.is_none()).map(model::project_cost);
            let line = kit::line()
                .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                .when_some(cost, |l, cost| l.child(kit::caption(&tk).flex_none().child(cost)))
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
                .when(on_github(p), |l| l.child(issues_switch(p, on, form_here, busy, m, &tk, cx)));
            let below = match (&self.issues_form, &p.issues) {
                (Some(form), _) if form_here => Some(issues_form_view(form, m, &choices, busy, &tk, window, cx).into_any_element()),
                (_, Some(settings)) if on => Some(issues_summary(i, p, settings, &choices, busy, m, &tk, cx).into_any_element()),
                _ => None,
            };
            items.push(div().flex().flex_col().child(line).children(below).into_any_element());
        }
        if m.projects.is_empty() && !gone {
            items.push(kit::line().child(div().flex_1().text_color(tk.muted).child("Cloned from its git remote, .env files included")).into_any_element());
        }
        let ids: Vec<&str> = m.projects.iter().map(|p| p.id.as_str()).collect();
        if let Some(note) = model::project_costs_note(&costs, &ids) {
            items.push(line_note(&tk).child(note).into_any_element());
        }
        if m.projects.iter().any(on_github) {
            let note = if m.projects.iter().any(|p| p.issues.is_some()) {
                format!("An issue given its project's label becomes a thread on {} that opens a pull request, and reviews, comments starting /agent and failed CI there go back to it. Only people who can push count.", m.name)
            } else {
                format!("Issues: label a GitHub issue and {} works on it and opens a pull request, waking up for it.", m.name)
            };
            items.push(line_note(&tk).child(note).into_any_element());
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
                let mut seg = div().grid().grid_cols(3).gap(px(8.));
                for (i, s) in self.state.sizes.iter().enumerate() {
                    let on = s.id == m.size;
                    let m2 = m.clone();
                    let id = s.id;
                    seg = seg.child(
                        super::forms::size_card(("size", i), s, on, &tk)
                            .when(busy, |b| b.opacity(0.6))
                            .when(busy || on, |b| b.cursor_default())
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

/// T3's webhook tasks on the machine, with addresses that wake it for their deliveries.
fn webhooks(m: &MachineView, d: &Details, tk: &Tokens, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let mut items: Vec<AnyElement> = Vec::new();
    for (i, w) in d.webhooks.iter().enumerate() {
        let about: Vec<&str> = [w.project.as_deref(), (!w.enabled).then_some("off"), w.signed.then_some("signed")].into_iter().flatten().collect();
        let left = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .flex_1()
            .min_w_0()
            .child(div().min_w_0().truncate().child(w.title.clone()))
            .when(!about.is_empty(), |l| l.child(kit::caption(tk).flex_none().child(about.join(" · "))));
        let right = match w.waking_url.clone() {
            Some(url) => {
                let title = w.title.clone();
                kit::link(("copy-webhook", i), "Copy URL", false, tk)
                    .px(px(4.))
                    .py(px(2.))
                    .text_size(px(11.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        this.toast(format!("Waking URL for \"{title}\" copied"), false, cx);
                    }))
                    .into_any_element()
            }
            None => kit::caption(tk).flex_none().child("URL once it's up").into_any_element(),
        };
        items.push(kit::line().gap(px(6.)).child(left).child(right).into_any_element());
    }
    items.push(
        line_note(tk)
            .child(format!(
                "T3 Code's scheduled tasks with a webhook. Deliveries to these URLs wake {} first, so they don't wait for it; T3 checks the token, signature and filters as usual. Anyone with a URL can send it deliveries.",
                m.name
            ))
            .into_any_element(),
    );
    kit::section("webhooks", Some(d.webhooks.len()), None, kit::lines(items, tk), tk)
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

/// One of T3 Connect's switches, flipped optimistically on click.
#[allow(clippy::too_many_arguments)]
fn connect_switch(
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    on: bool,
    busy: bool,
    m: &MachineView,
    action: Action,
    tk: &Tokens,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let m = m.clone();
    div()
        .id(SharedString::from(format!("connect-{key}")))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .py(px(8.))
        .when(!busy, |s| s.cursor_pointer())
        .child(div().flex().flex_col().gap(px(2.)).min_w_0().child(label).child(kit::hint(hint, tk)))
        .child(kit::switch(SharedString::from(format!("connect-{key}-switch")), on, busy, tk))
        .when(!busy, |s| {
            s.on_click(cx.listener(move |this, _, _, cx| {
                this.connect_want = Some((key, !on));
                this.track(&m, action.clone(), cx);
            }))
        })
}

/// A project's "issues" switch. Off goes at once (optimistically); on first opens the settings,
/// and clicking it again while they're open closes them.
#[allow(clippy::too_many_arguments)]
fn issues_switch(p: &MachineProject, on: bool, form_open: bool, busy: bool, m: &MachineView, tk: &Tokens, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let (m, p) = (m.clone(), p.clone());
    div()
        .id(SharedString::from(format!("issues-{}", p.id)))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .when(!busy, |s| s.cursor_pointer())
        .child(kit::caption(tk).child("issues"))
        .child(kit::switch(SharedString::from(format!("issues-{}-switch", p.id)), on, busy, tk))
        .when(!busy, |s| {
            s.on_click(cx.listener(move |this, _, window, cx| {
                if on {
                    this.issues_form = None;
                    this.issues_want = Some((p.id.clone(), false));
                    this.track(&m, model::issues_off(&p.id), cx);
                } else if form_open {
                    this.issues_form = None;
                    this.focus.focus(window, cx);
                } else {
                    this.open_issues_form(&m, &p, window, cx);
                }
                cx.notify();
            }))
        })
}

/// Under a project whose issues are on: its label and agent, and "Change".
#[allow(clippy::too_many_arguments)]
fn issues_summary(
    i: usize,
    p: &MachineProject,
    settings: &IssueSettings,
    choices: &[ModelChoice],
    busy: bool,
    m: &MachineView,
    tk: &Tokens,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let (m, p) = (m.clone(), p.clone());
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pb(px(8.))
        .child(
            kit::caption(tk)
                .flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(div().flex_none().text_color(tk.accent_text).child(format!("\"{}\"", settings.label)))
                .child("→")
                .child(div().min_w_0().truncate().child(settings.agent(choices))),
        )
        .child(
            kit::link(("issues-change", i), "Change", busy, tk)
                .px_0()
                .py(px(2.))
                .text_size(px(11.5))
                .when(!busy, |l| l.on_click(cx.listener(move |this, _, window, cx| this.open_issues_form(&m, &p, window, cx)))),
        )
}

/// The settings a project's issues get: the label, the model, how hard it thinks. Turns
/// issues on, or saves, which wakes the machine.
fn issues_form_view(
    form: &IssuesForm,
    m: &MachineView,
    choices: &[ModelChoice],
    busy: bool,
    tk: &Tokens,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> impl IntoElement + use<> {
    let tk = *tk;
    let focused = form.label.focus_handle(cx).is_focused(window);
    let label = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(10.))
        .border_1()
        .border_color(if focused { tk.accent_text } else { tk.frame })
        .child(div().flex_1().min_w_0().font_family(MONO).text_size(px(12.)).child(Input::new(&form.label).appearance(false).px_0().py_0().h(px(18.))))
        .child(kit::caption(&tk).child("made on GitHub if new"));
    let chosen = form.model.as_ref().and_then(|(provider, model)| choices.iter().find(|c| &c.provider == provider && &c.model == model));
    let mut agent = div().flex().flex_col().gap(px(6.));
    if choices.is_empty() {
        agent = agent.child(kit::hint(
            format!("{}. T3 Code shows t3agent37 {}'s models once it has opened it; then they're here to pick from.", form.current.agent(&[]), m.name),
            &tk,
        ));
    }
    let mut providers: Vec<&str> = choices.iter().map(|c| c.provider_name.as_str()).collect();
    providers.dedup();
    for provider in &providers {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, c) in choices.iter().enumerate().filter(|(_, c)| c.provider_name == *provider) {
            let picked = chosen.is_some_and(|ch| ch == c);
            let c2 = c.clone();
            row = row.child(choice_chip(("issues-model", i), c.short_name().to_owned(), picked, &tk).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(f) = &mut this.issues_form {
                    f.level = c2.level(f.level.as_deref());
                    f.model = Some((c2.provider.clone(), c2.model.clone()));
                }
                cx.notify();
            })));
        }
        agent = agent.when(providers.len() > 1, |a| a.child(kit::caption(&tk).child(provider.to_string()))).child(row);
    }
    let levels = chosen.and_then(|c| c.effort.as_ref()).map(|effort| {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, (id, name)) in effort.levels.iter().enumerate() {
            let id2 = id.clone();
            row = row.child(choice_chip(("issues-level", i), name.clone(), form.level.as_deref() == Some(id), &tk).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(f) = &mut this.issues_form {
                    f.level = Some(id2.clone());
                }
                cx.notify();
            })));
        }
        field("reasoning", &tk).child(row)
    });
    let m2 = m.clone();
    let actions = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            kit::button("issues-submit", Btn::Solid, Size::Sm, busy, &tk)
                .child(if form.saving { "Save" } else { "Turn on" })
                .when(!busy, |b| b.on_click(cx.listener(move |this, _, window, cx| this.submit_issues_form(&m2, window, cx)))),
        )
        .child(kit::link("issues-cancel", "Cancel", false, &tk).px_0().text_size(px(11.5)).on_click(cx.listener(|this, _, window, cx| {
            this.issues_form = None;
            this.focus.focus(window, cx);
            cx.notify();
        })))
        .child(div().flex_1())
        .child(kit::caption(&tk).child(format!("wakes {}", m.name)));
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .pt(px(4.))
        .pb(px(12.))
        .child(field("label", &tk).child(label))
        .child(field("agent", &tk).child(agent))
        .children(levels)
        .child(actions)
}

/// One pick among several that wrap: a model, a reasoning level.
fn choice_chip(id: impl Into<ElementId>, text: String, on: bool, tk: &Tokens) -> Stateful<Div> {
    let panel = tk.panel;
    div()
        .id(id)
        .flex_none()
        .px(px(8.))
        .py(px(5.))
        .border_1()
        .border_color(if on { tk.accent } else { tk.frame })
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(relative(1.))
        .cursor_pointer()
        .when(on, |s| s.bg(tk.accent).text_color(tk.accent_fg))
        .when(!on, |s| s.hover(move |s| s.bg(panel)))
        .child(text)
}

/// A caption over its control.
fn field(label: &'static str, tk: &Tokens) -> Div {
    div().flex().flex_col().gap(px(6.)).min_w_0().child(kit::caption(tk).child(label))
}

/// A project's issue settings being picked: before its issues go on, or to change them.
pub(super) struct IssuesForm {
    machine: String,
    project: String,
    label: Entity<InputState>,
    /// The model picked (provider, slug), and its reasoning level. `None`: keep `current`'s,
    /// which the machine didn't list (it hasn't shown its models yet).
    model: Option<(String, String)>,
    level: Option<String>,
    current: IssueSettings,
    /// Its issues are on already: this saves new settings.
    saving: bool,
    _enter: Subscription,
}

impl Panel {
    fn open_issues_form(&mut self, m: &MachineView, p: &MachineProject, window: &mut Window, cx: &mut Context<Self>) {
        let current = p.issues.clone().unwrap_or_default();
        let choices = self.details.as_ref().filter(|d| d.id == m.id).map(|d| d.issue_models.clone()).unwrap_or_default();
        let (model, level) = match current.choice(&choices) {
            Some(c) => (Some((c.provider.clone(), c.model.clone())), c.level(c.effort.as_ref().and_then(|e| current.level(&e.id)))),
            None => (None, None),
        };
        let label = cx.new(|cx| InputState::new(window, cx).default_value(current.label.clone()));
        let m2 = m.clone();
        let enter = cx.subscribe_in(&label, window, move |this: &mut Self, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && !this.pending.contains_key(&machine_key(&m2)) {
                this.submit_issues_form(&m2, window, cx);
            }
        });
        label.update(cx, |s, cx| s.focus(window, cx));
        self.port_input = None;
        self.issues_form = Some(IssuesForm {
            machine: m.id.clone(),
            project: p.id.clone(),
            label,
            model,
            level,
            current,
            saving: p.issues.is_some(),
            _enter: enter,
        });
        cx.notify();
    }

    fn submit_issues_form(&mut self, m: &MachineView, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.issues_form else { return };
        let label = match issues::check_label(&super::value(&form.label, cx)) {
            Ok(label) => label,
            Err(e) => return self.toast(e.to_string(), true, cx),
        };
        let choices = self.details.as_ref().filter(|d| d.id == m.id).map(|d| d.issue_models.clone()).unwrap_or_default();
        let picked = form.model.as_ref().and_then(|(provider, model)| choices.iter().find(|c| &c.provider == provider && &c.model == model));
        let settings = match picked {
            Some(c) => c.settings(&label, form.level.as_deref()),
            None => IssueSettings { label, ..form.current.clone() },
        };
        let (project, saving) = (form.project.clone(), form.saving);
        self.issues_form = None;
        self.focus.focus(window, cx);
        if !saving {
            self.issues_want = Some((project.clone(), true));
        }
        self.track(m, model::issues_on(&project, settings, saving), cx);
        cx.notify();
    }
}

/// GitHub's code, for as long as its sign-in waits: shown, so copying something else meanwhile
/// doesn't lose it.
fn github_code(code: String, tk: &Tokens, cx: &mut Context<Panel>) -> impl IntoElement + use<> {
    let copy = code.clone();
    line_note(tk)
        .child("Enter this code on the GitHub page that opened:")
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .pt(px(4.))
                .child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).text_size(px(17.)).text_color(tk.fg).child(code))
                .child(kit::link("copy-github-code", "Copy", false, tk).px_0().text_size(px(11.5)).on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    this.toast("GitHub code copied", false, cx);
                })))
                .child(kit::link("open-github-device", "Open page", false, tk).px_0().text_size(px(11.5)).on_click(|_, _, _| {
                    let _ = t3agent37_lib::app::open_url("https://github.com/login/device");
                })),
        )
}

/// Runs one of an agent's sign-ins, its spinner on the agent's line meanwhile.
fn start_sign_in(m: &MachineView, name: &str, action: Action, cx: &mut Context<Panel>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let (m, name) = (m.clone(), name.to_owned());
    cx.listener(move |this, _, _, cx| {
        this.signing_in = Some(name.clone());
        this.track(&m, action.clone(), cx);
    })
}

/// A sentence about the line above it: wraps instead of cutting off.
fn line_note(tk: &Tokens) -> Div {
    div().w_full().py(px(7.)).text_size(px(12.)).line_height(relative(1.4)).text_color(tk.muted).flex().flex_col().items_start()
}

fn skeleton_lines(n: usize, t: f64, tk: &Tokens) -> Vec<AnyElement> {
    (0..n).map(|i| kit::line().child(kit::skel(t, relative(0.4 - i as f32 * 0.1), 10., tk)).into_any_element()).collect()
}

