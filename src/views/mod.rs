//! The tab views. Each is an entity with local widget state (text inputs),
//! holding `Entity<Session>` and re-rendering on session notifications.

pub mod chat;
pub mod devices;
pub mod files;
pub mod join;
pub mod send_status;
pub mod settings;
pub mod services;
pub mod terminal;

use gpui_kit::gpui::{px, App, AppContext as _, Entity, Global, Window};

use crate::app::NoscatApp;
use crate::state::Session;

/// The session survives window close (tray mode keeps the node running) and
/// a reopened window must show the same state. GPUI globals are the
/// app-lifetime store.
struct SessionGlobal(Entity<Session>);

impl Global for SessionGlobal {}

/// Get the shared session, creating it on first call.
pub fn shared_session(cx: &mut App) -> Entity<Session> {
    if let Some(existing) = cx.try_global::<SessionGlobal>() {
        return existing.0.clone();
    }
    let session = cx.new(|cx| Session::new(cx));
    cx.set_global(SessionGlobal(session.clone()));
    session
}

/// Create the root view for a (new) window, reusing the shared session.
/// Called from the `open_window` render closure, so `cx` is the root's
/// context (it derefs to `App`).
pub fn root_view(window: &mut Window, cx: &mut gpui_kit::gpui::Context<NoscatApp>) -> NoscatApp {
    NoscatApp::new(shared_session(cx), window, cx)
}

/// Bring the main window forward (an access request needs an answer), or
/// reopen it when it was closed to the tray. Terminal windows don't count.
pub fn show_main_window(cx: &mut App) {
    let session = shared_session(cx);
    let main = session.read(cx).main_window;
    if let Some(h) = main.filter(|h| cx.windows().iter().any(|w| w.window_id() == h.window_id())) {
        let _ = h.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let options = gpui_kit::gpui::WindowOptions {
        titlebar: Some(gpui_kit::gpui::TitlebarOptions {
            title: Some("noscat".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        window_min_size: Some(gpui_kit::gpui::Size::new(px(720.), px(480.))),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| root_view(window, cx))).expect("failed to reopen window");
}
