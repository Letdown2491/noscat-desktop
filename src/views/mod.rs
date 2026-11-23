//! The tab views. Each is an entity with local widget state (text inputs),
//! holding `Entity<Session>` and re-rendering on session notifications.

pub mod chat;
pub mod devices;
pub mod files;
pub mod join;
pub mod settings;
pub mod shares;

use gpui_kit::gpui::{App, AppContext as _, Entity, Global, Window};

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
