//! noscat desktop: GPUI front end over the embedded noscat-client runtime.

mod app;
mod mesh;
mod paths;
mod state;
mod tray;
mod views;

use gpui_kit::gpui::{
    px, AppContext as _, QuitMode, TitlebarOptions, WindowBounds, WindowOptions,
};

fn main() {
    gpui_kit::application()
        // Tray mode: closing the last window keeps the node running, like
        // the Android foreground service. Quit happens from the tray.
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut gpui_kit::gpui::App| {
            gpui_kit::init(cx);

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
                    title: Some("noscat".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
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
                        "# noscat keeps running in the system tray. Reopen or \
                         quit it from the tray menu (Show noscat / Quit)."
                    );
                }
            })
            .detach();
        });
}
