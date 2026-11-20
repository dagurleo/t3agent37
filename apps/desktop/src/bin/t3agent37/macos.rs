//! The Objective-C side (`src/macos/shell.m`): the menu-bar item, the panel under it,
//! notifications, the login item and updates (Sparkle).

use std::ffi::{CString, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// What the menu-bar item asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// A left click on the icon.
    Toggle,
    /// "Open t3agent37" in its menu.
    Open,
    Quit,
}

unsafe extern "C" {
    fn t3a_status_install(png1x: *const u8, len1x: usize, png2x: *const u8, len2x: usize, tooltip: *const i8, clicked: extern "C" fn(i32));
    fn t3a_status_tooltip(tooltip: *const i8);
    fn t3a_panel_present(view: *mut c_void, width: f64, height: f64, gap: f64);
    fn t3a_panel_hide(view: *mut c_void);
    fn t3a_bundled() -> bool;
    fn t3a_notify(title: *const i8, body: *const i8);
    fn t3a_login_item_on() -> bool;
    fn t3a_login_item_set(on: bool) -> i32;
    fn t3a_reduce_motion() -> bool;
    fn t3a_updates_start() -> bool;
    fn t3a_updates_check() -> bool;
}

static CLICKS: OnceLock<UnboundedSender<Click>> = OnceLock::new();

// (Called inside AppKit's event handling: acted on from GPUI's loop.)
extern "C" fn clicked(what: i32) {
    let click = match what {
        0 => Click::Toggle,
        1 => Click::Open,
        _ => Click::Quit,
    };
    if let Some(tx) = CLICKS.get() {
        let _ = tx.unbounded_send(click);
    }
}

const ICON_1X: &[u8] = include_bytes!("../../../icons/tray.png");
const ICON_2X: &[u8] = include_bytes!("../../../icons/tray@2x.png");

fn c(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

pub fn install_status_item(tx: UnboundedSender<Click>) {
    let _ = CLICKS.set(tx);
    let tooltip = c("t3agent37");
    unsafe { t3a_status_install(ICON_1X.as_ptr(), ICON_1X.len(), ICON_2X.as_ptr(), ICON_2X.len(), tooltip.as_ptr(), clicked) }
}

pub fn set_tooltip(text: &str) {
    let text = c(text);
    unsafe { t3a_status_tooltip(text.as_ptr()) }
}

fn ns_view(window: &Window) -> Option<*mut c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr()),
        _ => None,
    }
}

/// Shows the hidden panel under the menu-bar item, `width`×`height` points, 6 pt below the bar.
pub fn present_panel(window: &Window, width: f32, height: f32) {
    if let Some(view) = ns_view(window) {
        unsafe { t3a_panel_present(view, width as f64, height as f64, 6.) }
    }
}

pub fn hide_panel(window: &Window) {
    if let Some(view) = ns_view(window) {
        unsafe { t3a_panel_hide(view) }
    }
}

/// Running from an app bundle: notifications and the login item work only then.
pub fn bundled() -> bool {
    unsafe { t3a_bundled() }
}

/// A notification: the system's from the app bundle, else through `osascript` (shown as
/// Script Editor's), so `cargo run` still shows them.
pub fn notify(title: &str, body: &str) {
    if bundled() {
        let (title, body) = (c(title), c(body));
        unsafe { t3a_notify(title.as_ptr(), body.as_ptr()) }
        return;
    }
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let script = format!("display notification {} with title {}", quote(body), quote(title));
    let _ = std::process::Command::new("osascript").args(["-e", &script]).spawn();
}

pub fn login_item_on() -> bool {
    bundled() && unsafe { t3a_login_item_on() }
}

pub fn set_login_item(on: bool) -> Result<(), String> {
    if !bundled() {
        return Err("only the app bundle can start at login".into());
    }
    match unsafe { t3a_login_item_set(on) } {
        0 => Ok(()),
        _ => Err("macOS refused; see System Settings → General → Login Items".into()),
    }
}

pub fn reduce_motion() -> bool {
    unsafe { t3a_reduce_motion() }
}

static UPDATES: AtomicBool = AtomicBool::new(false);

/// Starts Sparkle, if this build has it and is set up for updates (a release, or a bundle built
/// with the signing key in the keychain), and adds "Check for Updates…" to the menu-bar item's
/// menu. Call after [`install_status_item`]. Whether updates are on.
pub fn start_updates() -> bool {
    let on = unsafe { t3a_updates_start() };
    UPDATES.store(on, Ordering::Relaxed);
    on
}

/// Whether this build updates itself.
pub fn updates_on() -> bool {
    UPDATES.load(Ordering::Relaxed)
}

/// Looks for an update now, in Sparkle's window. False if updates are off.
pub fn check_for_updates() -> bool {
    unsafe { t3a_updates_check() }
}
