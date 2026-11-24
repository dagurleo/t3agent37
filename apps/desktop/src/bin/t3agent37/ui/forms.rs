//! The new machine form and the project picker: both
//! list T3 Code's projects that have a git remote, fetched fresh each time they open.

use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::MachineView;
use t3agent37_lib::machine::provision::SizeInfo;
use t3agent37_lib::projects::Project;

use super::details::sub_header;
use super::kit::{self, Btn, Size};
use super::{Load, NewForm, Panel, Picker, View, value};
use crate::model;
use crate::theme::{MONO, Tokens};

/// How an option of a project list looks and what a click on it does.
struct Opt {
    active: bool,
    /// Listed, not pickable: on the machine already, or its repository is picked.
    done: bool,
    badge: Option<AnyElement>,
    click: Option<Box<dyn Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static>>,
    hover: Option<Box<dyn Fn(&mut Panel, &mut Context<Panel>) + 'static>>,
}

impl Panel {
    pub(super) fn open_new_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let taken: Vec<String> = self.state.machines.iter().map(|m| m.name.clone()).collect();
        let suggested = model::suggest_name(&taken);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(suggested));
        name.update(cx, |s, cx| s.select_all(window, cx));
        let sub = cx.subscribe_in(&name, window, |this: &mut Self, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                let raw = input.read(cx).value().to_string();
                let clean = model::clean_name(&raw);
                if clean != raw {
                    input.update(cx, |s, cx| s.set_value(clean, window, cx));
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.launch(window, cx),
            _ => {}
        });
        // 4 vCPU · 8 GB: room for builds and tests without the large price.
        let size = if self.state.sizes.iter().any(|s| s.id == "default") { "default" } else { self.state.sizes.first().map_or("", |s| s.id) };
        self.new_form = Some(NewForm { name, size: size.into(), picked: Vec::new(), copy_of: None, delete_after: None, _sub: sub });
        self.load_projects(cx);
    }

    /// The new machine form as a copy of `m`: its size, and a name after it.
    pub(super) fn open_copy_form(&mut self, m: &MachineView, window: &mut Window, cx: &mut Context<Self>) {
        self.go(View::New, window, cx);
        let taken: Vec<&str> = self.state.machines.iter().map(|m| m.name.as_str()).collect();
        let base: String = m.name.chars().take(27).collect();
        let name = (1..).map(|i| if i == 1 { format!("{base}-copy") } else { format!("{base}-copy{i}") }).find(|n| !taken.contains(&n.as_str())).unwrap_or_default();
        if let Some(form) = &mut self.new_form {
            form.copy_of = Some((m.id.clone(), m.name.clone()));
            form.size = m.size.clone();
            form.name.update(cx, |s, cx| {
                s.set_value(name, window, cx);
                s.select_all(window, cx);
            });
        }
        cx.notify();
    }

    /// What's wrong with the name, if anything.
    fn name_problem(&self, name: &str) -> Option<&'static str> {
        if name.is_empty() {
            Some("Give it a name")
        } else if !model::valid_name(name) {
            Some("Start with a letter or number")
        } else if self.state.machines.iter().any(|m| m.name == name) {
            Some("You already have a machine with this name")
        } else {
            None
        }
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.new_form else { return };
        let name = value(&form.name, cx);
        if self.name_problem(&name).is_some() || form.size.is_empty() {
            return;
        }
        let job = match &form.copy_of {
            Some((id, _)) => futures::future::Either::Left(self.backend.copy_machine(id.clone(), name, form.size.clone(), form.delete_after)),
            None => futures::future::Either::Right(self.backend.create_machine(name, form.size.clone(), form.picked.clone(), form.delete_after)),
        };
        self.go(View::List, window, cx);
        cx.spawn(async move |this, cx| {
            if let Err(e) = job.await {
                let _ = this.update(cx, |this, cx| this.toast(e, true, cx));
            }
        })
        .detach();
    }

    pub(super) fn render_new(&mut self, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let Some(form) = &self.new_form else { return div().into_any_element() };
        let (input, size, picked) = (form.name.clone(), form.size.clone(), form.picked.clone());
        let (copy_of, delete_after) = (form.copy_of.clone(), form.delete_after);
        let name = value(&input, cx);
        let problem = self.name_problem(&name);
        let focused = input.focus_handle(cx).is_focused(window);

        let name_field = field("name", &tk)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(42.))
                    .px(px(12.))
                    .bg(tk.panel)
                    .border_1()
                    .border_color(if focused { tk.accent_text } else { tk.frame })
                    .when(focused, |d| d.shadow(vec![inset_ring(tk.accent_text)]))
                    .child(div().mr(px(10.)).font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(16.)).text_color(tk.accent_text).child("$"))
                    .child(div().flex_1().min_w_0().font_family(MONO).text_size(px(16.)).child(Input::new(&input).appearance(false).px_0().py_0().h(px(24.)))),
            )
            .child(
                kit::hint(problem.unwrap_or("Shows up under this name in T3 Code"), &tk)
                    .when(problem.is_some() && !name.is_empty(), |h| h.text_color(tk.danger)),
            );

        let mut sizes = div().grid().grid_cols(3).gap(px(8.));
        for s in self.state.sizes {
            let id = s.id;
            sizes = sizes.child(
                size_card(SharedString::from(format!("size-{id}")), s, s.id == size, &tk)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(f) = &mut this.new_form {
                            f.size = id.into();
                        }
                        cx.notify();
                    })),
            );
        }
        let size_field = field("size", &tk).child(sizes).child(kit::hint("Billed only while awake · sleeps 5 min after agents finish · resize anytime", &tk));

        let list: Vec<Project> = match &self.projects {
            Load::Ready(list) => list.clone(),
            _ => Vec::new(),
        };
        let options: Vec<(Project, Opt)> = list
            .iter()
            .map(|p| {
                let on = picked.contains(&p.id);
                // One clone per repository: another project of the same repo is already picked.
                let same_repo = !on && list.iter().any(|o| o.repo_key == p.repo_key && picked.contains(&o.id));
                let id = p.id.clone();
                let opt = Opt {
                    active: on,
                    done: same_repo,
                    badge: on.then(|| kit::ok("", &tk).into_any_element()),
                    click: (!same_repo).then(|| {
                        Box::new(move |this: &mut Panel, _: &mut Window, cx: &mut Context<Panel>| {
                            if let Some(f) = &mut this.new_form {
                                if let Some(i) = f.picked.iter().position(|x| *x == id) {
                                    f.picked.remove(i);
                                } else {
                                    f.picked.push(id.clone());
                                }
                            }
                            cx.notify();
                        }) as Box<dyn Fn(&mut Panel, &mut Window, &mut Context<Panel>)>
                    }),
                    hover: None,
                };
                (p.clone(), opt)
            })
            .collect();
        let projects_field = match &copy_of {
            Some((_, from)) => field("what comes along", &tk).child(kit::hint(
                format!("{from}'s disk as it is now: its projects and their files, work not pushed, its sign-ins and tools. Its threads too. Not T3 Connect, GitHub issues, the start URL or links to other machines."),
                &tk,
            )),
            None => {
                let projects = self.project_list("new-projects", options, "", &tk, cx);
                field("projects", &tk)
                    .child(div().border_1().border_color(tk.frame).bg(tk.bg).child(div().max_h(px(150.)).child(projects).overflow_y_scrollbar().id("new-projects-scroll")))
                    .child(kit::hint("Optional · cloned from git once it's up, .env files included", &tk))
            }
        };
        let mut times = div().flex().h(px(30.)).border_1().border_color(tk.frame);
        for (i, (secs, label)) in model::DELETE_AFTER.iter().enumerate() {
            let secs = *secs;
            times = times.child(super::details::segment(SharedString::from(format!("delete-after-{i}")), i == 0, delete_after == secs, &tk).child(*label).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(f) = &mut this.new_form {
                        f.delete_after = secs;
                    }
                    cx.notify();
                },
            )));
        }
        let lifetime_field = field("delete it after", &tk).child(times).child(kit::hint(
            match delete_after {
                Some(_) => "A throwaway machine: deleted then, with everything on it, or once its agents finish",
                None => "Kept until you delete it",
            },
            &tk,
        ));

        let n = picked.len();
        let then = if n > 0 { format!(", then {n} {}", if n == 1 { "project" } else { "projects" }) } else { String::new() };
        let disabled = problem.is_some() || size.is_empty();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(self.state.settings.animated_rooftop, |d| d.child(sub_header("New machine", &tk, cx)))
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
                    .child(name_field)
                    .child(size_field)
                    .child(projects_field)
                    .child(lifetime_field)
                    .overflow_y_scrollbar()
                    .id("new-body"),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .pt(px(10.))
                    .pb(px(14.))
                    .child(
                        kit::button("launch", Btn::Solid, Size::Lg, disabled, &tk)
                            .child(if copy_of.is_some() { "Make the copy" } else { "Launch machine" })
                            .when(!disabled, |b| b.on_click(cx.listener(|this, _, window, cx| this.launch(window, cx)))),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(tk.muted)
                            .child(if copy_of.is_some() {
                                "Takes a few minutes. T3 Code will ask to add it.".to_owned()
                            } else {
                                format!("Takes about a minute{then}. T3 Code will ask to add it.")
                            }),
                    ),
            )
            .into_any_element()
    }

    // ── the details view's project picker ──

    pub(super) fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("filter projects"));
        let sub = cx.subscribe_in(&query, window, |this: &mut Self, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                if let Some(p) = &mut this.picker {
                    p.active = 0;
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.pick_active(window, cx),
            _ => {}
        });
        query.update(cx, |s, cx| s.focus(window, cx));
        self.picker = Some(Picker { query, active: 0, _sub: sub });
        self.load_projects(cx);
        // opened from further down (or from a row's ⋯ menu): bring the whole picker into view,
        // once it's laid out
        let scroll = self.details_scroll.clone();
        cx.on_next_frame(window, move |_, _, cx| {
            scroll.scroll_to_bottom();
            cx.notify();
        });
    }

    /// The projects the filter matches: on name and ~-path, so the home folder never matches all.
    fn shown_projects(&self, cx: &App) -> Vec<Project> {
        let q = self.picker.as_ref().map(|p| value(&p.query, cx).trim().to_lowercase()).unwrap_or_default();
        match &self.projects {
            Load::Ready(list) => list.iter().filter(|p| q.is_empty() || format!("{}\n{}", p.title, model::home_path(&p.path)).to_lowercase().contains(&q)).cloned().collect(),
            _ => Vec::new(),
        }
    }

    fn picker_machine(&self) -> Option<MachineView> {
        let View::Machine(id) = &self.view else { return None };
        self.state.machines.iter().find(|m| &m.id == id).cloned()
    }

    fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.shown_projects(cx);
        let (Some(picker), Some(m)) = (&self.picker, self.picker_machine()) else { return };
        let Some(p) = shown.get(picker.active.min(shown.len().saturating_sub(1))) else { return };
        if model::has_repo(&m, &p.repo_key) {
            return;
        }
        let p = p.clone();
        self.picker = None;
        self.focus.focus(window, cx);
        self.add_project(&m, &p, cx);
        cx.notify();
    }

    fn move_active(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.shown_projects(cx).len();
        if let Some(p) = &mut self.picker
            && n > 0
        {
            let current = p.active.min(n - 1) as isize;
            p.active = (current + by).rem_euclid(n as isize) as usize;
            cx.notify();
        }
    }

    pub(super) fn render_picker(&mut self, m: &MachineView, tk: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let Some(picker) = &self.picker else { return div().into_any_element() };
        let query = picker.query.clone();
        let q = value(&query, cx);
        let shown = self.shown_projects(cx);
        let current = picker.active.min(shown.len().saturating_sub(1));
        let focused = query.focus_handle(cx).is_focused(window);
        let options: Vec<(Project, Opt)> = shown
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let there = model::has_repo(m, &p.repo_key);
                let opt = Opt {
                    active: i == current,
                    done: there,
                    badge: there.then(|| kit::ok(format!("on {}", m.name), &tk).text_size(px(10.)).into_any_element()),
                    click: Some(Box::new(move |this: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
                        if let Some(p) = &mut this.picker {
                            p.active = i;
                        }
                        this.pick_active(window, cx);
                    })),
                    hover: Some(Box::new(move |this: &mut Panel, cx: &mut Context<Panel>| {
                        if let Some(p) = &mut this.picker
                            && p.active != i
                        {
                            p.active = i;
                            cx.notify();
                        }
                    })),
                };
                (p.clone(), opt)
            })
            .collect();
        let list = self.project_list("pick", options, q.trim(), &tk, cx);
        div()
            .id("picker")
            .mb(px(10.))
            .bg(tk.bg)
            .border_1()
            .border_color(tk.frame)
            // arrows move through the list, caught before the field moves its caret
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_active(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_active(1, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(10.))
                    .bg(tk.panel)
                    .border_b_1()
                    .border_color(if focused { tk.accent_text } else { tk.line })
                    .child(div().font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(12.)).text_color(tk.accent_text).child("$"))
                    .child(div().flex_1().min_w_0().font_family(MONO).text_size(px(12.)).child(Input::new(&query).appearance(false).px_0().py_0().h(px(18.)))),
            )
            .child(div().h(px(180.)).child(list).overflow_y_scrollbar().id("pick-scroll"))
            .into_any_element()
    }

    /// The list body: projects, an empty or error note, or skeleton rows while loading.
    fn project_list(&mut self, id: &'static str, options: Vec<(Project, Opt)>, query: &str, tk: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tk = *tk;
        let t = self.t();
        match &self.projects {
            Load::Failed(e) => {
                return pick_note(&tk)
                    .text_color(tk.danger)
                    .child(format!("Couldn't list T3 Code's projects: {e}"))
                    .child(
                        kit::link(SharedString::from(format!("{id}-retry")), "Try again", false, &tk)
                            .p_0()
                            .font_family(crate::theme::BODY)
                            .text_size(px(11.5))
                            .on_click(cx.listener(|this, _, _, cx| this.load_projects(cx))),
                    )
                    .into_any_element();
            }
            Load::Loading => {
                return div()
                    .children((0..4).map(|i| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .h(px(40.))
                            .px(px(10.))
                            .child(kit::skel(t, px(22.), 22., &tk))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .flex_1()
                                    .child(kit::skel(t, relative(0.46 - i as f32 * 0.08), 9., &tk))
                                    .child(kit::skel(t, relative(0.7 - i as f32 * 0.06), 7., &tk)),
                            )
                    }))
                    .into_any_element();
            }
            Load::Ready(_) => {}
        }
        if options.is_empty() {
            let text = if query.is_empty() { "No project in T3 Code has a git remote yet".to_string() } else { format!("No project matches “{query}”") };
            return pick_note(&tk).child(text).into_any_element();
        }
        let mut rows = Vec::new();
        for (i, (p, opt)) in options.into_iter().enumerate() {
            let icon = self.favicon(&p);
            let initial = p.title.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
            let name_color = if opt.done {
                tk.muted
            } else if opt.active {
                tk.accent_text
            } else {
                tk.fg
            };
            let mut row = div()
                .id((SharedString::from(id), i))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(40.))
                .px(px(10.))
                .when(!opt.done, |r| r.cursor_pointer())
                .when(opt.active, |r| r.bg(tk.panel).border_l_2().border_color(tk.accent_text).pl(px(8.)))
                .child(
                    div()
                        .flex_none()
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(tk.panel)
                        .border_1()
                        .border_color(if opt.active { tk.frame } else { tk.line })
                        .font_family(MONO)
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(11.))
                        .text_color(tk.accent_text)
                        .map(|tile| match icon {
                            Some(icon) => tile.child(img(icon).size(px(16.)).object_fit(ObjectFit::Contain)),
                            None => tile.child(initial),
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .min_w_0()
                                .text_size(px(12.5))
                                .line_height(relative(1.2))
                                .child(div().flex_1().min_w_0().truncate().text_color(name_color).child(p.title.clone()))
                                .children(opt.badge),
                        )
                        // a long path loses its start, never the folder name
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis_start()
                                .font_family(MONO)
                                .text_size(px(10.5))
                                .line_height(relative(1.2))
                                .text_color(tk.muted)
                                .child(model::home_path(&p.path)),
                        ),
                );
            if let Some(click) = opt.click {
                row = row.on_click(cx.listener(move |this, _, window, cx| click(this, window, cx)));
            }
            if let Some(hover) = opt.hover {
                row = row.on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| hover(this, cx)));
            }
            rows.push(row);
        }
        div().children(rows).into_any_element()
    }
}

/// A labelled field: caption, then the control and its hint.
fn field(label: &'static str, tk: &Tokens) -> Div {
    div().flex().flex_col().gap(px(6.)).min_w_0().child(kit::caption(tk).child(label))
}

fn pick_note(tk: &Tokens) -> Div {
    kit::caption(tk).flex().flex_col().items_start().gap(px(6.)).px(px(10.)).py(px(12.)).line_height(relative(1.45))
}

/// One machine size as a card: name, spec and price, framed in the accent when `on`. The new
/// machine form and the details' resize both pick from these, three to a row.
pub(super) fn size_card(id: impl Into<ElementId>, s: &SizeInfo, on: bool, tk: &Tokens) -> Stateful<Div> {
    let frame = tk.frame;
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(3.))
        .pt(px(10.))
        .px(px(10.))
        .pb(px(9.))
        .border_1()
        .cursor_pointer()
        .map(|d| if on { d.bg(tk.panel).border_color(tk.accent_text).shadow(vec![inset_ring(tk.accent_text)]) } else { d.border_color(tk.line).hover(move |s| s.border_color(frame)) })
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(capitalized(s.id)))
        .child(div().font_family(MONO).text_size(px(10.)).line_height(relative(1.3)).text_color(tk.muted).child(s.spec))
        .child(div().mt(px(4.)).font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(11.)).text_color(tk.accent_text).child(model::hourly(s.hourly)))
}

/// `box-shadow: inset 0 0 0 1px`: the second pixel of a focused frame.
fn inset_ring(color: Rgba) -> BoxShadow {
    BoxShadow { color: color.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: true }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
