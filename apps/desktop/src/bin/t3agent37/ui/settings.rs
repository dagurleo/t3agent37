//! Settings and the first-run setup.

use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::Setup;
use t3agent37_lib::mcp::Client;

use t3agent37_lib::store::{AWAKE_ALERTS, PanelSize, Settings, WALLET_ALERTS};

use super::details::{segment, sub_header};
use super::kit::{self, Btn, Icon, Size};
use super::{Panel, value};
use crate::model;
use crate::theme::{DISPLAY, MONO, Tokens};

const CLIENTS: [(Client, &str); 2] = [(Client::ClaudeCode, "Claude Code"), (Client::Codex, "Codex")];

/// A setting picked from a few choices: its label, then a segmented control.
fn choices(
    id: &'static str,
    label: &'static str,
    options: &[String],
    on: Option<usize>,
    tk: &Tokens,
    cx: &mut Context<Panel>,
    set: fn(&mut Settings, usize),
) -> impl IntoElement + use<> {
    let mut seg = div().flex().flex_none().w(px(196.)).border_1().border_color(tk.frame);
    for (i, option) in options.iter().enumerate() {
        seg = seg.child(
            segment((id, i), i == 0, on == Some(i), tk)
                .h(px(26.))
                .child(option.clone())
                .when(on != Some(i), |s| s.on_click(cx.listener(move |this, _, _, cx| this.change_settings(|s| set(s, i), cx)))),
        );
    }
    div().flex().items_center().justify_between().gap(px(12.)).child(div().min_w_0().child(label)).child(seg)
}

/// A setting that is on or off, with a line on what it does.
fn toggle(
    id: &'static str,
    label: &'static str,
    hint: &'static str,
    on: bool,
    tk: &Tokens,
    cx: &mut Context<Panel>,
    set: fn(&mut Settings, bool),
) -> impl IntoElement + use<> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .cursor_pointer()
        .child(div().flex().flex_col().gap(px(2.)).min_w_0().child(div().child(label)).child(kit::hint(hint, tk)))
        .child(kit::switch(SharedString::from(format!("{id}-switch")), on, false, tk))
        .on_click(cx.listener(move |this, _, _, cx| this.change_settings(|s| set(s, !on), cx)))
}

impl Panel {
    pub(super) fn render_settings(&mut self, tk: &Tokens, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let state = &self.state;
        // optimistic; the next state is the source of truth
        let at_login = self.login_want.unwrap_or(state.settings.start_at_login);

        let login = div()
            .id("start-at-login")
            .flex()
            .items_center()
            .justify_between()
            .py(px(11.))
            .cursor_pointer()
            .child(div().font_weight(FontWeight(560.)).child("Start at login"))
            .child(kit::switch("start-at-login-switch", at_login, false, &tk))
            .on_click(cx.listener(move |this, _, _, cx| this.set_start_at_login(!at_login, cx)));

        let names: Vec<String> = CLIENTS.iter().map(|(_, n)| n.to_string()).collect();
        let mut buttons = div().grid().grid_cols(2).gap(px(8.));
        for (client, name) in CLIENTS {
            let added = state.mcp.get(client);
            let adding = self.mcp_adding == Some(client);
            buttons = buttons.child(if added == Some(true) {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(tk.accent_text)
                    .bg(tk.panel)
                    .border_1()
                    .border_color(tk.line)
                    .child(kit::icon(Icon::Check, 10., tk.accent_text))
                    .child("Added to\u{a0}")
                    .child(kit::agent_name(name, 11., tk.accent_text))
                    .into_any_element()
            } else {
                let disabled = added.is_none() || self.mcp_adding.is_some();
                kit::button(SharedString::from(format!("mcp-{name}")), Btn::Outline, Size::Sm, disabled && !adding, &tk)
                    .map(|b| {
                        if adding {
                            b.child(kit::dots(t, tk.accent_text)).child("Adding…")
                        } else {
                            b.child("Add to\u{a0}").child(kit::agent_name(name, 12., tk.accent_text))
                        }
                    })
                    .when(!disabled, |b| b.on_click(cx.listener(move |this, _, _, cx| this.add_mcp(client, cx))))
                    .into_any_element()
            });
        }
        let missing: Vec<String> = CLIENTS.iter().filter(|(c, _)| state.mcp.get(*c).is_none()).map(|(_, n)| n.to_string()).collect();
        let mcp_hint: AnyElement = if missing.is_empty() {
            kit::hint("Their agents can then create, wake and send work to machines", &tk).into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .text_size(px(11.5))
                .text_color(tk.muted)
                .children(kit::agent_names(&missing, 11.5, tk.muted))
                .child(if missing.len() == 1 { "\u{a0}isn't installed on this computer" } else { "\u{a0}aren't installed on this computer" })
                .into_any_element()
        };
        let mcp = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(11.))
            .border_t_1()
            .border_color(tk.line)
            .child(kit::caption(&tk).child("mcp"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .font_weight(FontWeight(560.))
                    .child("Let\u{a0}")
                    .children(kit::agent_names(&names, 13., tk.fg))
                    .child("\u{a0}control t3agent37"),
            )
            .child(buttons)
            .child(mcp_hint);

        let updates_on = crate::macos::updates_on();
        let version = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .py(px(11.))
            .border_t_1()
            .border_color(tk.line)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().font_weight(FontWeight(560.)).child(format!("t3agent37 {}", env!("CARGO_PKG_VERSION"))))
                    .child(kit::hint(if updates_on { "Looks for updates once a day" } else { "This build doesn't update itself" }, &tk)),
            )
            .when(updates_on, |d| {
                d.child(kit::button("check-updates", Btn::Outline, Size::Sm, false, &tk).child("Check now").on_click(|_, _, _| {
                    crate::macos::check_for_updates();
                }))
            });

        let settings = state.settings.clone();
        let wallet = choices(
            "wallet-alert",
            "Wallet runs low",
            &WALLET_ALERTS.map(|v| v.map_or("Off".to_string(), |d| format!("${d:.0}"))),
            WALLET_ALERTS.iter().position(|v| *v == settings.wallet_alert_usd),
            &tk,
            cx,
            |s, i| s.wallet_alert_usd = WALLET_ALERTS[i],
        );
        let awake = choices(
            "awake-alert",
            "Awake too long",
            &AWAKE_ALERTS.map(|v| v.map_or("Off".to_string(), |h| format!("{h}h"))),
            AWAKE_ALERTS.iter().position(|v| *v == settings.awake_alert_hours),
            &tk,
            cx,
            |s, i| s.awake_alert_hours = AWAKE_ALERTS[i],
        );
        let finished = toggle(
            "notify-finished",
            "Agents finish",
            "A notification when a machine's agents are done",
            settings.notify_finished,
            &tk,
            cx,
            |s, on| s.notify_finished = on,
        );
        let alerts = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .py(px(11.))
            .border_t_1()
            .border_color(tk.line)
            .child(kit::caption(&tk).child("notify me when"))
            .child(wallet)
            .child(awake)
            .child(finished);
        // The panel's size takes effect at once, so it isn't one of the plain `choices`.
        let current = settings.panel_size;
        let mut sizes = div().flex().flex_none().w(px(196.)).border_1().border_color(tk.frame);
        for (i, size) in PanelSize::ALL.iter().copied().enumerate() {
            sizes = sizes.child(
                segment(("panel-size-choice", i), i == 0, size == current, &tk)
                    .h(px(26.))
                    .child(size.label())
                    .when(size != current, |s| s.on_click(cx.listener(move |this, _, window, cx| this.set_panel_size(size, window, cx)))),
            );
        }
        let look = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .py(px(11.))
            .border_t_1()
            .border_color(tk.line)
            .child(kit::caption(&tk).child("look"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(div().flex().flex_col().gap(px(2.)).min_w_0().child("Panel size").child(kit::hint("⌘+ and ⌘− while it's open", &tk)))
                    .child(sizes),
            )
            .child(toggle(
                "animated-rooftop",
                "Animated rooftop",
                "The pixel-art city with an agent walking for each machine, instead of the still skyline",
                settings.animated_rooftop,
                &tk,
                cx,
                |s, on| s.animated_rooftop = on,
            ));
        let projects = div().flex().flex_col().gap(px(8.)).py(px(11.)).border_t_1().border_color(tk.line).child(kit::caption(&tk).child("projects")).child(toggle(
            "set-up-projects",
            "Set up projects after adding them",
            "Runs the project's T3 setup scripts, or installs its dependencies from its lockfile",
            settings.set_up_projects,
            &tk,
            cx,
            |s, on| s.set_up_projects = on,
        ));

        let account = state.account.as_ref().map(|a| {
            if a.past_due {
                "Agent37 · past due: top up to wake machines".to_string()
            } else {
                format!("Agent37 · {} left · {} GB can run at once", model::money(a.balance_usd), a.memory_limit_gb)
            }
        });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(self.state.settings.animated_rooftop, |d| d.child(sub_header("Settings", &tk, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .pt(px(10.))
                    .pb(px(4.))
                    .child(kit::hint("Machines sleep 5 minutes after their agents finish and wake by themselves when you use them in T3 Code.", &tk).pb(px(7.)))
                    .child(login)
                    .child(mcp)
                    .child(look)
                    .child(alerts)
                    .child(projects)
                    .child(version)
                    .overflow_y_scrollbar()
                    .id("settings-body"),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .px(px(16.))
                    .pt(px(10.))
                    .pb(px(14.))
                    .border_t_1()
                    .border_color(tk.line)
                    .child(div().min_w_0().font_family(MONO).text_size(px(11.)).text_color(tk.muted).children(account))
                    .child(kit::button("quit", Btn::Quiet, Size::Sm, false, &tk).child("Quit t3agent37").on_click(|_, _, cx| cx.quit())),
            )
    }

    /// Saves one change to the settings.
    pub(super) fn change_settings(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let mut settings = self.state.settings.clone();
        change(&mut settings);
        // shown at once; the backend's next state agrees
        self.state.settings = settings.clone();
        if let Err(e) = self.backend.update_settings(settings) {
            self.toast(e, true, cx);
        }
        cx.notify();
    }

    fn set_start_at_login(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Err(e) = crate::macos::set_login_item(on) {
            self.toast(format!("Couldn't change start at login: {e}"), true, cx);
            return;
        }
        self.login_want = Some(on);
        let mut settings = self.state.settings.clone();
        settings.start_at_login = on;
        if let Err(e) = self.backend.update_settings(settings) {
            self.toast(e, true, cx);
        }
        cx.notify();
    }

    fn add_mcp(&mut self, client: Client, cx: &mut Context<Self>) {
        self.mcp_adding = Some(client);
        let job = self.backend.add_mcp(client);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_adding = None;
                if let Err(e) = result {
                    this.toast(e, true, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ── setup ──

    pub(super) fn render_setup(&mut self, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        let setup = self.state.setup;
        let busy = self.state.setup_busy;
        if matches!(setup, Setup::Starting | Setup::Ready) {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .p(px(24.))
                .child(kit::caption(&tk).flex().items_center().gap(px(6.)).child(kit::dots(t, tk.muted)).child("checking your setup"))
                .into_any_element();
        }
        let current = if setup == Setup::ConnectAgent37 { 0 } else { 1 };
        let steps = [
            (
                "Connect Agent37 Cloud",
                "Paste an API key from your Agent37 dashboard. Machines bill to your own Agent37 wallet; the key stays on this computer.",
            ),
            ("Install T3 Code", "Your agents' chats live there. t3agent37 adds machines to it."),
        ];
        let key = value(&self.key_input, cx);
        let looks_like_key = key.trim().starts_with("sk_live_");
        let focused = self.key_input.focus_handle(cx).is_focused(window);
        let mut list = div().flex().flex_col().border_b_1().border_color(tk.line);
        for (i, (title, body)) in steps.into_iter().enumerate() {
            let (done, now) = (i < current, i == current);
            let num = div()
                .flex_none()
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .font_family(MONO)
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(11.))
                .border_1()
                .map(|n| {
                    if done {
                        n.bg(tk.accent).border_color(tk.accent).child(kit::icon(Icon::Check, 12., tk.accent_fg))
                    } else if now {
                        n.border_color(tk.accent_text).text_color(tk.accent_text).child(format!("0{}", i + 1))
                    } else {
                        n.border_color(tk.line).text_color(tk.muted).child(format!("0{}", i + 1))
                    }
                });
            let mut text = div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .min_w_0()
                .flex_1()
                .pt(px(4.))
                .child(div().text_size(px(14.)).font_weight(FontWeight(560.)).when(done, |d| d.text_color(tk.muted)).child(title));
            if now {
                text = text.child(div().text_size(px(12.5)).text_color(tk.muted).child(body));
                if setup == Setup::ConnectAgent37 {
                    let enabled = looks_like_key && !busy;
                    text = text.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .mt(px(8.))
                            .child(
                                div()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .border_1()
                                    .border_color(if focused { tk.accent_text } else { tk.line })
                                    .font_family(MONO)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.5))
                                    .child(Input::new(&self.key_input).appearance(false).disabled(busy).px_0().py_0().h(px(18.))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(
                                        kit::button("connect", Btn::Solid, Size::Lg, !enabled && !busy, &tk)
                                            .flex_1()
                                            .map(|b| if busy { b.child(kit::dots(t, tk.accent_fg)).child("Checking key") } else { b.child("Connect") })
                                            .when(enabled, |b| b.on_click(cx.listener(|this, _, window, cx| this.save_key(window, cx)))),
                                    )
                                    .child(kit::link("get-key", "Get a key", busy, &tk).when(!busy, |l| l.on_click(cx.listener(|this, _, _, cx| this.setup_action("openKeys", cx))))),
                            ),
                    );
                } else {
                    text = text.child(
                        kit::button("install-t3", Btn::Solid, Size::Lg, busy, &tk)
                            .mt(px(8.))
                            .child("Install T3 Code")
                            .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.setup_action("installT3", cx)))),
                    );
                }
            }
            list = list.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(tk.line)
                    .when(!done && !now, |s| s.opacity(0.55))
                    .child(num)
                    .child(text),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .pt(px(10.))
                    .pb(px(4.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().font_family(DISPLAY).font_weight(FontWeight::SEMIBOLD).text_size(px(22.)).line_height(relative(1.1)).child("Get set up in two steps"))
                            .child(div().text_color(tk.muted).line_height(relative(1.5)).child("Then launch machines for T3 Code that sleep when no agent is working, billing disk alone.")),
                    )
                    .child(list)
                    .overflow_y_scrollbar()
                    .id("setup-body"),
            )
            .child(
                div().flex_none().flex().flex_col().items_center().px(px(16.)).pt(px(10.)).pb(px(14.)).child(
                    kit::link("recheck", "Done it already? Check again", busy, &tk).when(!busy, |l| l.on_click(cx.listener(|this, _, _, cx| this.setup_action("recheck", cx)))),
                ),
            )
            .into_any_element()
    }

    pub(super) fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = value(&self.key_input, cx).trim().to_string();
        if !key.starts_with("sk_live_") || self.state.setup_busy {
            return;
        }
        let job = self.backend.save_api_key(key);
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.key_input.update(cx, |s, cx| s.set_value("", window, cx)),
                Err(e) => this.toast(e, true, cx),
            });
        })
        .detach();
    }

    fn setup_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let job = self.backend.setup_action(action);
        cx.spawn(async move |this, cx| {
            if let Err(e) = job.await {
                let _ = this.update(cx, |this, cx| this.toast(e, true, cx));
            }
        })
        .detach();
    }
}
