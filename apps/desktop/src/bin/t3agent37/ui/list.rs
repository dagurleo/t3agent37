//! The machine list: a row per machine with
//! its agent, what it's doing, the one obvious action and a ⋯ menu; the costs under it.

use std::time::Instant;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::{ElsewhereMachine, MachineState as State, MachineView};

use super::kit::{self, Btn, Icon, Size};
use super::{Panel, View};
use crate::model::{self, MenuItem, machine_key, pixel_state};
use crate::pixel::paint::{ICON_H, ICON_W, paint_spy_icon};
use crate::pixel::{Px, seed_of};
use crate::theme::{DISPLAY, MONO, Tokens};

/// A click handler on a control inside a clickable row: it doesn't open the row too.
macro_rules! inside_row {
    ($cx:expr, |$this:ident, $window:ident, $cx2:ident| $body:expr) => {
        $cx.listener(move |$this: &mut Panel, _: &ClickEvent, $window: &mut Window, $cx2: &mut Context<Panel>| {
            $cx2.stop_propagation();
            $body
        })
    };
}

impl Panel {
    pub(super) fn render_list(&mut self, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        // One being added shows as a machine already.
        let elsewhere: Vec<ElsewhereMachine> = self
            .state
            .elsewhere
            .iter()
            .filter(|e| !self.state.machines.iter().any(|m| m.name.eq_ignore_ascii_case(&e.name)))
            .cloned()
            .collect();
        if self.state.machines.is_empty() {
            return div()
                .flex()
                .flex_col()
                .justify_center()
                .flex_1()
                .gap(px(10.))
                .px(px(24.))
                .pb(px(28.))
                .child(div().font_family(DISPLAY).font_weight(FontWeight::SEMIBOLD).text_size(px(22.)).line_height(relative(1.1)).child("No machines yet"))
                .child(div().text_color(tk.muted).line_height(relative(1.5)).child(
                    "A machine is a cloud computer your T3 Code agents work on. It sleeps when nobody's working, and then only its disk is billed.",
                ))
                .child(
                    kit::button("new-machine", Btn::Solid, Size::Lg, false, &tk)
                        .mt(px(8.))
                        .child(kit::icon(Icon::Plus, 16., tk.accent_fg))
                        .child("New machine")
                        .on_click(cx.listener(|this, _, window, cx| this.go(View::New, window, cx))),
                )
                .when(!elsewhere.is_empty(), |d| d.child(div().mx(px(-24.)).mt(px(12.)).child(self.render_elsewhere(&elsewhere, &tk, cx))))
                .into_any_element();
        }

        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(8.))
            .border_b_1()
            .border_color(tk.line)
            .child(kit::caption(&tk).flex().items_center().child("machines").child(kit::count(self.state.machines.len(), &tk)))
            .child(
                kit::button("new-machine", Btn::Solid, Size::Sm, false, &tk)
                    .child(kit::icon(Icon::Plus, 12., tk.accent_fg))
                    .child("New machine")
                    .on_click(cx.listener(|this, _, window, cx| this.go(View::New, window, cx))),
            );

        let machines = self.state.machines.clone();
        let rows: Vec<AnyElement> = machines.iter().map(|m| self.render_row(m, &tk, window, cx).into_any_element()).collect();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(head)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .children(rows)
                    .when(!elsewhere.is_empty(), |d| d.child(self.render_elsewhere(&elsewhere, &tk, cx)))
                    .overflow_y_scrollbar()
                    .id("list"),
            )
            .into_any_element()
    }

    /// Machines made with t3agent37 on another computer, on the same Agent37 account.
    fn render_elsewhere(&mut self, list: &[ElsewhereMachine], tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows = list.iter().enumerate().map(|(i, e)| {
            let about: Vec<&str> = [e.size.as_deref(), Some(if e.asleep { "asleep" } else { "awake" })].into_iter().flatten().collect();
            let (id, name) = (e.id.clone(), e.name.clone());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .flex_1()
                        .min_w_0()
                        .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(e.name.clone()))
                        .child(kit::caption(tk).flex_none().child(about.join(" · "))),
                )
                .child(
                    kit::button(("add-here", i), Btn::Outline, Size::Sm, false, tk)
                        .child("Add here")
                        .on_click(cx.listener(move |this, _, _, cx| this.add_machine_here(id.clone(), name.clone(), cx))),
                )
        });
        div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(tk.line)
            .pt(px(4.))
            .child(kit::caption(tk).flex().items_center().px(px(16.)).pt(px(8.)).pb(px(2.)).child("on your agent37 account").child(kit::count(list.len(), tk)))
            .children(rows)
            .child(
                div()
                    .px(px(16.))
                    .pb(px(10.))
                    .text_size(px(12.))
                    .line_height(relative(1.4))
                    .text_color(tk.muted)
                    .child("Made with t3agent37 on another computer. Adding one here wakes it once and signs this computer in to it; the other computer keeps it, and both show the same machine."),
            )
    }

    /// Not a tracked action: the machine shows as one being added meanwhile.
    fn add_machine_here(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let job = self.backend.add_machine_here(id);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(name) => this.toast(format!("{name} is on this computer too. Switch to T3 Code to accept it."), false, cx),
                Err(e) => this.toast(format!("Couldn't add {name}: {e}"), true, cx),
            });
        })
        .detach();
    }

    fn render_row(&mut self, m: &MachineView, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let key = machine_key(m);
        let openable = !m.id.is_empty() && m.state != State::Creating;
        let pending = self.pending.get(&key).copied();
        let primary = model::primary_action(m);
        let detail = model::detail(m);
        let t = self.t();

        let mut icon = Px::new(ICON_W, ICON_H);
        paint_spy_icon(&mut icon, crate::theme::theme_of(window), pixel_state(m.state), seed_of(&key), self.clock());
        let icon = self.image(&icon, (2. * window.scale_factor()).round() as usize);

        let name_color = if matches!(m.state, State::Asleep | State::Missing) { tk.muted } else { tk.fg };
        let detail_color = match m.state {
            State::Working => tk.accent_text,
            State::Error | State::Missing => tk.danger,
            _ => tk.muted,
        };
        let text = div()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(div().min_w_0().text_size(px(14.)).font_weight(FontWeight(560.)).text_color(name_color).truncate().child(m.name.clone()))
                    .child(kit::chip(m.size.clone(), &tk))
                    // who's signed in: one logo per coding agent
                    .children(m.signed_in.iter().filter_map(|a| kit::agent_logo(a, 12., tk.muted))),
            )
            .child(div().mt(px(3.)).font_family(MONO).text_size(px(11.)).line_height(relative(1.3)).text_color(detail_color).truncate().child(detail));

        let armed_key = format!("primary:{key}");
        let armed = self.is_armed(&armed_key);
        let actions = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .when_some(pending, |d, busy| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(tk.muted)
                        .child(kit::dots(t, tk.muted))
                        .child(busy),
                )
            })
            .when_some(primary.filter(|_| pending.is_none()), |d, primary| {
                let kind = if armed {
                    Btn::Danger
                } else if primary.solid {
                    Btn::Solid
                } else {
                    Btn::Outline
                };
                let label = if armed { "Confirm".to_string() } else { primary.label.clone() };
                let m = m.clone();
                let armed_key = armed_key.clone();
                d.child(kit::button(SharedString::from(format!("primary-{key}")), kind, Size::Sm, false, &tk).child(label).on_click(inside_row!(
                    cx,
                    |this, _window, cx| {
                        // "Remove" asks twice
                        if matches!(primary.cmd, crate::backend::Cmd::Delete { .. }) && !this.arm(&armed_key) {
                            cx.notify();
                            return;
                        }
                        this.track(&m, primary.clone(), cx);
                    }
                )))
            })
            .when(!m.id.is_empty() && pending.is_none(), |d| d.child(self.render_row_menu(m, &tk, cx)));

        let top = div().flex().items_center().gap(px(12.)).child(kit::pixels(icon, 48., 42.)).child(text).child(actions);

        // notes under the name: a refused delete, sign-ins, setting it up, projects arriving
        let mut notes: Vec<AnyElement> = Vec::new();
        if let Some(error) = self.delete_errors.get(&key).cloned() {
            notes.push(self.delete_error_note(m, &error, pending.is_some(), &tk, cx).into_any_element());
        }
        let ask = model::sign_ins_to_ask(&m.signed_in, &m.signed_out);
        if !m.id.is_empty() && !ask.is_empty() {
            let one = ask.len() == 1;
            let mut note = note(&tk).child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .overflow_hidden()
                    .children(kit::agent_names(&ask, 11., tk.muted))
                    .child(div().whitespace_nowrap().child(if one { "\u{a0}needs a sign-in" } else { "\u{a0}need a sign-in" })),
            );
            for agent in &ask {
                let action = model::sign_in_to(agent);
                let m2 = m.clone();
                let label = if one { kit::link(SharedString::from(format!("signin-{key}-{agent}")), "Sign in", pending.is_some(), &tk) } else {
                    kit::link(SharedString::from(format!("signin-{key}-{agent}")), "", pending.is_some(), &tk).child(kit::agent_name(agent, 11.5, tk.accent_text))
                };
                note = note.child(label.px(px(4.)).py(px(2.)).text_size(px(11.5)).when(pending.is_none(), |l| {
                    l.on_click(inside_row!(cx, |this, _window, cx| this.track(&m2, action.clone(), cx)))
                }));
            }
            notes.push(note.into_any_element());
        } else if model::needs_project(m) && openable {
            let id = m.id.clone();
            notes.push(
                note(&tk)
                    .child(div().flex_1().child("No projects yet"))
                    .child(
                        kit::link(SharedString::from(format!("add-one-{key}")), "Add one", false, &tk)
                            .px(px(4.))
                            .py(px(2.))
                            .text_size(px(11.5))
                            .on_click(inside_row!(cx, |this, window, cx| this.go_picking(View::Machine(id.clone()), true, window, cx))),
                    )
                    .into_any_element(),
            );
        }
        for p in m.projects.iter().filter(|p| p.status.is_some()) {
            notes.push(
                note(&tk)
                    .child(kit::dots(t, tk.muted))
                    .child(div().flex_1().min_w_0().truncate().child(format!("{} · {}", p.title, p.status.clone().unwrap_or_default())))
                    .into_any_element(),
            );
        }

        let id = m.id.clone();
        let hover_key = key.clone();
        div()
            .id(SharedString::from(format!("row-{key}")))
            .flex()
            .flex_col()
            .pl(px(16.))
            .pr(px(12.))
            .py(px(9.))
            .border_b_1()
            .border_color(tk.line)
            .hover(move |s| s.bg(tk.panel))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover = Some(hover_key.clone());
                } else if this.hover.as_deref() == Some(hover_key.as_str()) {
                    this.hover = None;
                }
                cx.notify();
            }))
            .when(openable, |row| {
                row.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| this.go(View::Machine(id.clone()), window, cx)))
            })
            .child(top)
            .children((!notes.is_empty()).then(|| div().pl(px(60.)).mt(px(6.)).flex().flex_col().children(notes)))
    }

    /// The ⋯ button and, when open, its menu.
    fn render_row_menu(&mut self, m: &MachineView, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let key = machine_key(m);
        let open = self.menu.as_deref() == Some(key.as_str());
        let toggle_key = key.clone();
        let button = kit::button(SharedString::from(format!("more-{key}")), Btn::Icon, Size::Md, false, &tk)
            .when(open, |b| b.bg(tk.panel).text_color(tk.fg))
            .child(kit::icon(Icon::More, 16., if open { tk.fg } else { tk.muted }))
            .on_click(inside_row!(cx, |this, _window, cx| {
                // (a click on the button while open closes it on the way down)
                let just_closed = this.menu_closed.as_ref().is_some_and(|(k, at)| *k == toggle_key && at.elapsed().as_millis() < 250);
                this.menu = if this.menu.is_some() || just_closed { None } else { Some(toggle_key.clone()) };
                this.menu_confirm = false;
                cx.notify();
            }));
        let menu = open.then(|| {
            let items = model::menu_actions(m, self.state.sizes);
            let mut list = div()
                .id(SharedString::from(format!("menu-{key}")))
                .min_w(px(208.))
                .py(px(4.))
                .bg(tk.bg)
                .border_1()
                .border_color(tk.frame)
                .shadow(kit::hard_shadow(&tk))
                .on_mouse_down_out(cx.listener({
                    let key = key.clone();
                    move |this, _, _, cx| {
                        this.menu = None;
                        this.menu_confirm = false;
                        this.menu_closed = Some((key.clone(), Instant::now()));
                        cx.notify();
                    }
                }))
                .child(div().px(px(12.)).pt(px(5.)).pb(px(4.)).font_family(MONO).text_size(px(10.)).text_color(tk.muted).child(m.name.clone()));
            for (i, item) in items.into_iter().enumerate() {
                let label = match &item {
                    MenuItem::Act(a) => a.label.clone(),
                    MenuItem::PickProject => model::PICK_PROJECT.to_string(),
                };
                let m = m.clone();
                list = list.child(menu_item(SharedString::from(format!("menu-{key}-{i}")), &tk).child(label).on_click(inside_row!(
                    cx,
                    |this, window, cx| {
                        this.menu = None;
                        match &item {
                            MenuItem::Act(a) => this.track(&m, a.clone(), cx),
                            MenuItem::PickProject => this.go_picking(View::Machine(m.id.clone()), true, window, cx),
                        }
                    }
                )));
            }
            let armed = self.menu_confirm;
            let m = m.clone();
            list.child(div().h(px(1.)).my(px(4.)).bg(tk.line)).child(
                menu_item(SharedString::from(format!("menu-{key}-delete")), &tk)
                    .text_color(if armed { rgb(0xffffff) } else { tk.danger })
                    .when(armed, |d| d.bg(tk.danger))
                    .child(if armed { "Click again to delete forever" } else { "Delete machine…" })
                    .on_click(inside_row!(cx, |this, _window, cx| {
                        if this.menu_confirm {
                            this.menu = None;
                            this.menu_confirm = false;
                            this.track(&m, model::delete(), cx);
                        } else {
                            this.menu_confirm = true;
                            cx.notify();
                        }
                    })),
            )
        });
        div().relative().child(button).children(menu.map(|menu| {
            // anchored to the button's edge, opening toward the side with more room
            div().absolute().top(px(32.)).right_0().child(deferred(anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(1))
        }))
    }

    /// Why the last delete was refused, with "Delete anyway" and "Keep".
    pub(super) fn delete_error_note(&mut self, m: &MachineView, error: &str, busy: bool, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let key = machine_key(m);
        let m = m.clone();
        let keep_key = key.clone();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(4.))
            .gap_y(px(6.))
            .py(px(2.))
            .font_family(MONO)
            .text_size(px(11.))
            .line_height(relative(1.3))
            .text_color(tk.danger)
            .child(div().w_full().child(error.to_string()))
            .child(kit::button(SharedString::from(format!("anyway-{key}")), Btn::Danger, Size::Sm, busy, &tk).child("Delete anyway").when(!busy, |b| {
                b.on_click(inside_row!(cx, |this, _window, cx| this.track(&m, model::delete_anyway(), cx)))
            }))
            .child(kit::button(SharedString::from(format!("keep-{key}")), Btn::Quiet, Size::Sm, false, &tk).child("Keep").on_click(inside_row!(
                cx,
                |this, _window, cx| {
                    this.delete_errors.remove(&keep_key);
                    cx.notify();
                }
            )))
    }

    /// Today, this month, the wallet.
    pub(super) fn render_costs(&self, tk: &Tokens) -> impl IntoElement + use<> {
        let costs = &self.state.costs;
        let low = costs.balance.is_some_and(|b| b < 2.);
        let cost = |label: &'static str, value: String, first: bool, warn: bool| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(16.))
                .pt(px(9.))
                .pb(px(10.))
                .when(!first, |d| d.border_l_1().border_color(tk.line))
                .child(kit::caption(tk).child(label))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(if warn { tk.danger } else { tk.fg }).child(value))
        };
        div()
            .flex_none()
            .flex()
            .bg(tk.panel)
            .border_t_1()
            .border_color(tk.line)
            .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1), ("cv11".into(), 1), ("ss01".into(), 1)])))
            .child(cost("today", model::money(costs.today), true, false))
            .child(cost("this month", model::money(costs.month), false, false))
            .child(cost("balance", costs.balance.map_or("–".into(), model::money), false, low))
    }
}

/// A quiet line under a row's name.
fn note(tk: &Tokens) -> Div {
    div().flex().items_center().gap(px(6.)).min_h(px(22.)).font_family(MONO).text_size(px(11.)).line_height(relative(1.3)).text_color(tk.muted)
}

fn menu_item(id: SharedString, tk: &Tokens) -> Stateful<Div> {
    let panel = tk.panel;
    div().id(id).w_full().px(px(12.)).py(px(7.)).text_size(px(13.)).cursor_pointer().hover(move |s| s.bg(panel))
}
