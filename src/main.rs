//! noscat desktop: GPUI front end over the embedded noscat-client runtime.

mod app;
mod identity;
mod mesh;
mod paths;
mod state;
mod tray;
mod views;

use gpui_kit::gpui::{px, AppContext as _, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
    gpui_kit::application()
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
                titlebar: Some(TitlebarOptions {
                    title: Some("noscat".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            };

            let _tray = tray::TrayHandle::new();

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::NoscatApp::new(window, cx))
            })
            .expect("failed to open window");
        });
}
