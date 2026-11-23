//! What a machine is doing, in its details: "now" lists its agents' threads and what each is up
//! to, its load and busiest processes, and what woke it; "finished" what its agents finished in
//! the last day. Asleep, "finished" says what they did before it went to sleep. A thread opens
//! in T3 Code on click.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::MachineView;
use t3agent37_lib::machine::activity::{ActivityView, FinishedView, Load, Process, ThreadView};

use super::Panel;
use super::kit::{self, Icon};
use crate::model;
use crate::theme::{MONO, Tokens};

/// Processes shown: the busiest few.
const SHOW_TOP: usize = 3;

impl Panel {
    /// "now", while the machine is up and its watchdog reports what it does.
    pub(super) fn now(&mut self, m: &MachineView, a: &ActivityView, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let now = model::now_ms();
        let mut items: Vec<AnyElement> = Vec::new();
        for (i, t) in a.threads.iter().enumerate() {
            items.push(self.thread_row(m, i, t, now, &tk, cx).into_any_element());
        }
        if a.threads.is_empty() {
            items.push(note(&tk).child("No agent is working right now.").into_any_element());
        }
        if let Some(load) = &a.load {
            items.push(load_row(load, &tk).into_any_element());
        }
        if !a.top.is_empty() {
            items.push(processes(&a.top, &tk).into_any_element());
        }
        let woke = a.woke_at.map(|at| match &a.woke_for {
            Some(why) => format!("woke {} · {why}", model::clock(at)),
            None => format!("woke {}", model::clock(at)),
        });
        let extra = woke.map(|w| kit::caption(&tk).flex_none().child(w).into_any_element());
        kit::section("now", Some(a.threads.len()), extra, kit::lines(items, &tk), &tk)
    }

    /// "finished": the latest threads its agents finished, from the last day.
    pub(super) fn finished(&mut self, m: &MachineView, a: &ActivityView, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let now = model::now_ms();
        let mut items: Vec<AnyElement> = a.finished.iter().enumerate().map(|(i, f)| self.finished_row(m, i, f, now, &tk, cx).into_any_element()).collect();
        if items.is_empty() {
            items.push(note(&tk).child("Nothing finished in the last day.").into_any_element());
        }
        // Asleep: as of when it went to sleep, which the list can't refresh.
        let extra = (!a.live).then(|| kit::caption(&tk).flex_none().child(format!("before it slept, {}", model::clock(a.as_of))).into_any_element());
        kit::section("finished", None, extra, kit::lines(items, &tk), &tk)
    }

    fn thread_row(&mut self, m: &MachineView, i: usize, t: &ThreadView, now: u64, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let blink = kit::blink(self.t());
        let mut about: Vec<String> = [t.project.clone(), t.agent.clone()].into_iter().flatten().collect();
        if t.subagents > 0 {
            about.push(if t.subagents == 1 { "1 subagent".into() } else { format!("{} subagents", t.subagents) });
        }
        let (m2, id) = (m.clone(), t.id.clone());
        div()
            .id(("now-thread", i))
            .flex()
            .gap(px(9.))
            .py(px(8.))
            .cursor_pointer()
            .hover(move |s| s.bg(tk.panel))
            .on_click(cx.listener(move |this, _, _, cx| this.track(&m2, model::open_in(Some(id.clone())), cx)))
            .child(div().flex_none().mt(px(5.)).size(px(7.)).bg(tk.accent_text).opacity(if t.needs_you { blink } else { 1. }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(if t.title.is_empty() { "Untitled thread".to_owned() } else { t.title.clone() }))
                            .when_some(t.started_at, |r, at| r.child(kit::caption(&tk).flex_none().child(model::duration(now.saturating_sub(at) / 1000))))
                            .child(kit::icon(Icon::Open, 9., tk.muted)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(5.))
                            .text_size(px(11.5))
                            .line_height(relative(1.35))
                            .text_color(tk.muted)
                            .child(div().text_color(tk.accent_text).when(t.needs_you, |d| d.font_weight(FontWeight::SEMIBOLD)).child(t.state.clone()))
                            .children(about.into_iter().map(|a| div().child(format!("· {a}")))),
                    )
                    .children(t.background.iter().map(|b| kit::caption(&tk).truncate().child(format!("↳ {b}")))),
            )
    }

    fn finished_row(&mut self, m: &MachineView, i: usize, f: &FinishedView, now: u64, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let failed = f.status == "failed";
        let status = match f.status.as_str() {
            "completed" => "done",
            "rolled_back" => "rolled back",
            other => other,
        }
        .to_owned();
        let (m2, id) = (m.clone(), f.id.clone());
        div()
            .id(("finished-thread", i))
            .flex()
            .flex_col()
            .gap(px(2.))
            .py(px(8.))
            .cursor_pointer()
            .hover(move |s| s.bg(tk.panel))
            .on_click(cx.listener(move |this, _, _, cx| this.track(&m2, model::open_in(Some(id.clone())), cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(if f.title.is_empty() { "Untitled thread".to_owned() } else { f.title.clone() }))
                    .when_some(f.ended_at, |r, at| r.child(kit::caption(&tk).flex_none().child(model::ago_ms(at, now))))
                    .child(kit::icon(Icon::Open, 9., tk.muted)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(5.))
                    .text_size(px(11.5))
                    .line_height(relative(1.35))
                    .text_color(tk.muted)
                    .child(div().when(failed, |d| d.text_color(tk.danger)).child(status))
                    .when_some(f.project.clone(), |d, p| d.child(format!("· {p}"))),
            )
            .when_some(f.error.clone(), |d, e| d.child(kit::caption(&tk).truncate().child(e)))
    }
}

/// CPU, memory and disk: how much is in use, of how much.
fn load_row(l: &Load, tk: &Tokens) -> impl IntoElement + use<> {
    let tk = *tk;
    let stat = |label: &'static str, value: Option<String>, share: Option<f64>, first: bool| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(!first, |d| d.pl(px(10.)).border_l_1().border_color(tk.line))
            .when(first, |d| d.pr(px(10.)))
            .child(kit::caption(&tk).child(label))
            .child(div().truncate().font_family(MONO).text_size(px(11.5)).child(value.unwrap_or_else(|| "–".into())))
            .child(
                div().h(px(3.)).w_full().bg(tk.line).child(
                    div().h_full().w(relative(share.unwrap_or(0.).clamp(0., 1.) as f32)).bg(if share.unwrap_or(0.) >= 0.9 { tk.danger } else { tk.accent_text }),
                ),
            )
    };
    let cores = format!("{:.1} of {} cores", l.cores, l.cpus);
    let mem = l.mem_used_mb.zip(l.mem_total_mb);
    let disk = l.disk_used_gb.zip(l.disk_total_gb);
    div()
        .flex()
        .py(px(9.))
        .child(stat("cpu", Some(cores), (l.cpus > 0).then(|| l.cores / f64::from(l.cpus)), true))
        .child(stat(
            "memory",
            mem.map(|(u, t)| format!("{} of {}", model::megabytes(u), model::megabytes(t))),
            mem.filter(|(_, t)| *t > 0).map(|(u, t)| u as f64 / t as f64),
            false,
        ))
        .child(stat(
            "disk",
            disk.map(|(u, t)| format!("{} of {}", model::gigabytes(u), model::gigabytes(t))),
            disk.filter(|(_, t)| *t > 0.).map(|(u, t)| u / t),
            false,
        ))
}

/// The busiest processes: what they are, their share of one core, their memory.
fn processes(top: &[Process], tk: &Tokens) -> impl IntoElement + use<> {
    let tk = *tk;
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .py(px(8.))
        .child(kit::caption(&tk).child("busiest processes"))
        .children(top.iter().take(SHOW_TOP).map(|p| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .child(div().flex_1().min_w_0().truncate().child(p.name.clone()))
                .child(kit::caption(&tk).flex_none().child(format!("{}% cpu", p.cpu)))
                .child(kit::caption(&tk).flex_none().w(px(52.)).text_right().child(model::megabytes(p.mem_mb)))
        }))
}

fn note(tk: &Tokens) -> Div {
    div().w_full().py(px(8.)).text_size(px(12.)).line_height(relative(1.4)).text_color(tk.muted)
}
