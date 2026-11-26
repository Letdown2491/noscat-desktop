//! noscat desktop: GPUI front end over the embedded noscat-client runtime.

mod app;
mod keystore;
mod mesh;
mod palette;
mod ui;
mod paths;
mod state;
mod tray;
mod views;

/// The name people see (launcher, window, tray, notifications). The
/// command, crate and data folders stay lowercase `noscat`.
pub const APP_NAME: &str = "Noscat";
/// The Wayland app id and desktop entry (`noscat-desktop.desktop`), so
/// docks and launchers match the window to the entry and its icon.
pub const APP_ID: &str = "noscat-desktop";

use gpui_kit::gpui::{
    px, AppContext as _, QuitMode, TitlebarOptions, WindowBounds, WindowOptions,
};

fn main() {
    // RUST_LOG turns on the client's logs (on stderr), for diagnosing a
    // start or a connection: `RUST_LOG=noscat_core=debug noscat-desktop`.
    if let Ok(filter) = std::env::var("RUST_LOG") {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init();
    }
    // The storage key first: nothing may touch the device dir before it.
    keystore::init();
    gpui_kit::application()
        // Tray mode: closing the last window keeps the node running, like
        // the Android foreground service. Quit happens from the tray.
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut gpui_kit::gpui::App| {
            gpui_kit::init(cx);
            app::bind_keys(cx);

            // Appearance before the first window renders: follow the
            // desktop's light or dark mode unless the user pinned one.
            let settings = paths::Settings::load();
            state::apply_theme(settings.theme, None, cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(
                    gpui_kit::gpui::Bounds {
                        origin: gpui_kit::gpui::Point::new(px(0.), px(0.)),
                        size: gpui_kit::gpui::Size::new(px(960.), px(680.)),
                    },
                )),
                // Below this the tab layout and chat bubbles crush.
                window_min_size: Some(gpui_kit::gpui::Size::new(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                app_id: Some(APP_ID.into()),
                ..Default::default()
            };

            tray::init(cx);

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| views::root_view(window, cx))
            })
            .expect("failed to open window");

            // Terminal-launch feedback: closing the window is by design not
            // an exit (the node keeps running like the Android foreground
            // service); the tray menu is the way back in and the way out.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    eprintln!(
                        "# Noscat keeps running in the system tray. Reopen or \
                         quit it from the tray menu (Show Noscat / Quit)."
                    );
                }
            })
            .detach();
        });
}
