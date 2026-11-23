//! t3agent37: a menu-bar item and the panel under it, drawn natively with GPUI (no webview),
//! and the `t3agent37` command (`cli/`), both on the backend in this package's library. With a
//! command (`t3agent37 list`) it's the command; without one, the app. Built without the `panel`
//! feature it's the command alone, for Linux or a Mac without the app. One backend runs at a
//! time, the app's or `t3agent37 serve`: a second would share the machines, ports and state of
//! the first.

mod cli;
#[cfg(feature = "panel")]
mod backend;
#[cfg(feature = "panel")]
mod macos;
#[cfg(feature = "panel")]
mod model;
#[cfg(feature = "panel")]
mod pixel;
#[cfg(feature = "panel")]
mod theme;
#[cfg(feature = "panel")]
mod ui;

#[cfg(feature = "panel")]
pub use panel::hide_window;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A bare `t3agent37` typed in a terminal while t3agent37 runs already (the app's program
    // linked as the command): the help, not a second app.
    let typed_while_running = args.is_empty()
        && std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::net::TcpStream::connect(("127.0.0.1", t3agent37_lib::mcp::port())).is_ok();
    if cli::wants(&args) || typed_while_running || cfg!(not(feature = "panel")) {
        std::process::exit(cli::main(args));
    }
    #[cfg(feature = "panel")]
    panel::main();
}

#[cfg(feature = "panel")]
mod panel {
    use std::borrow::Cow;
    use std::time::{Duration, Instant};

    use futures::StreamExt;
    use futures::channel::mpsc::unbounded;
    use gpui_kit::*;
    use t3agent37_lib::app::data_dir;
    use t3agent37_lib::mcp;

    use crate::backend::{Backend, Signal};
    use crate::macos::{self, Click};
    use crate::theme;
    use crate::ui::Panel;

    /// The app's one panel (made at its first opening, then kept) and whether it shows.
    struct Shell {
        backend: Backend,
        panel: Option<(AnyWindowHandle, Entity<Panel>)>,
        visible: bool,
        /// When the panel last went away by itself (clicked away): a click on the menu-bar item
        /// right after is the same gesture, not a request to open it again.
        closed_at: Option<Instant>,
    }

    impl Global for Shell {}

    pub fn main() {
        tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())).init();

        // One t3agent37 at a time: the running one serves MCP here.
        let port = mcp::port();
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            let why = "t3agent37 is already running. Quit it (menu-bar icon → Quit), or stop `t3agent37 serve`, to start this one.";
            eprintln!("{why} (port {port} is taken)");
            macos::notify("t3agent37", why);
            std::process::exit(1);
        }

        let data_dir = data_dir();
        gpui_kit::application().with_assets(gpui_kit::assets::Assets).with_quit_mode(QuitMode::Explicit).run(move |cx| {
            gpui_kit::init(cx);
            cx.set_activation_policy(ActivationPolicy::Accessory);
            let fonts: Vec<Cow<'static, [u8]>> = theme::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                tracing::warn!("fonts: {e}");
            }

            let (signals, mut heard) = unbounded();
            let backend = Backend::start(data_dir, signals);
            cx.set_global(Shell { backend, panel: None, visible: false, closed_at: None });
            cx.spawn(async move |cx| {
                while let Some(signal) = heard.next().await {
                    cx.update(|cx| on_signal(signal, cx));
                }
            })
            .detach();

            let (clicks, mut clicked) = unbounded();
            macos::install_status_item(clicks);
            if macos::start_updates() {
                tracing::info!("updates on");
            }
            cx.spawn(async move |cx| {
                while let Some(click) = clicked.next().await {
                    cx.update(|cx| match click {
                        Click::Toggle => toggle(cx),
                        Click::Open => show(cx),
                        Click::Quit => cx.quit(),
                    });
                }
            })
            .detach();

            // `--open`: up at launch, for a quick look.
            if std::env::args().any(|a| a == "--open") {
                show(cx);
            }
        });
    }

    fn on_signal(signal: Signal, cx: &mut App) {
        match signal {
            Signal::Changed => {
                let state = cx.global::<Shell>().backend.state();
                macos::set_tooltip(&Panel::tooltip(&state));
                if let Some((_, panel)) = cx.global::<Shell>().panel.clone() {
                    panel.update(cx, |p, cx| p.refresh(state, cx));
                }
            }
            Signal::Notify { title, body } => macos::notify(&title, &body),
            Signal::LoginItem(on) => {
                if on != macos::login_item_on() && macos::bundled() {
                    if let Err(e) = macos::set_login_item(on) {
                        tracing::warn!("start at login: {e}");
                    }
                }
            }
        }
    }

    fn toggle(cx: &mut App) {
        let shell = cx.global::<Shell>();
        if shell.visible {
            if let Some((handle, panel)) = shell.panel.clone() {
                let _ = handle.update(cx, |_, window, cx| panel.update(cx, |p, cx| p.hide(window, cx)));
            }
            return;
        }
        let just_closed = shell.closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(300));
        if !just_closed {
            show(cx);
        }
    }

    /// Shows the panel under the menu-bar item, at the size Settings remembers, making it the
    /// first time.
    fn show(cx: &mut App) {
        let (w, h) = cx.global::<Shell>().backend.state().settings.panel_size.dims();
        if let Some((handle, panel)) = cx.global::<Shell>().panel.clone() {
            cx.global_mut::<Shell>().visible = true;
            let _ = handle.update(cx, |_, window, cx| {
                panel.update(cx, |p, cx| p.set_visible(true, window, cx));
                macos::present_panel(window, w, h);
            });
            return;
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size: size(px(w), px(h)) })),
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Made hidden: shown under the menu-bar item once drawn.
            show: false,
            focus: false,
            ..Default::default()
        };
        let backend = cx.global::<Shell>().backend.clone();
        match gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| Panel::new(backend, window, cx))) {
            Ok((handle, panel)) => {
                let shell = cx.global_mut::<Shell>();
                shell.panel = Some((handle, panel.clone()));
                shell.visible = true;
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        panel.update(cx, |p, cx| p.set_visible(true, window, cx));
                        macos::present_panel(window, w, h);
                    });
                });
            }
            Err(e) => tracing::error!("couldn't open the panel: {e}"),
        }
    }

    /// The panel went away: hidden, not closed. Called by the panel itself (`Panel::hide`).
    pub fn hide_window(window: &mut Window, cx: &mut App) {
        macos::hide_panel(window);
        let shell = cx.global_mut::<Shell>();
        shell.visible = false;
        shell.closed_at = Some(Instant::now());
    }
}
