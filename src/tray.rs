//! System tray over StatusNotifierItem (ksni): the node keeps running when
//! the window closes, like the Android foreground service. The tray menu
//! reopens the window or quits the app.

use gpui_kit::gpui::{App, AppContext as _, AsyncApp};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem};

use crate::views;

/// Commands the tray sends into the GPUI app thread.
pub enum Command {
    Show,
    Quit,
}

struct NoscatTray {
    tx: async_channel::Sender<Command>,
}

impl ksni::Tray for NoscatTray {
    fn id(&self) -> String {
        "noscat-desktop".into()
    }

    fn title(&self) -> String {
        "noscat".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // 32x32 solid noscat green, ARGB32 network byte order.
        let mut data = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..32 * 32 {
            data.extend_from_slice(&[0xff, 0x22, 0xc5, 0x5e]);
        }
        vec![Icon {
            width: 32,
            height: 32,
            data,
        }]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: "Show noscat".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send_blocking(Command::Show);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send_blocking(Command::Quit);
                }),
                ..Default::default()
            }),
        ]
    }
}

/// Spawn the tray service on its own thread and the pump that applies its
/// commands on the app thread. Closing the window leaves the node running;
/// the tray is the way back in.
pub fn init(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<Command>();
    let _ = std::thread::Builder::new()
        .name("noscat-tray".into())
        .spawn(move || {
            use ksni::blocking::TrayMethods as _;
            // A missing StatusNotifierWatcher (headless, i3 without a tray,
            // ...) must not kill the app.
            if let Err(e) = (NoscatTray { tx }).spawn() {
                eprintln!("tray unavailable: {e}");
            }
        });

    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(cmd) = rx.recv().await {
            match cmd {
                Command::Show => show_window(cx),
                Command::Quit => {
                    let _ = cx.update(|cx| cx.quit());
                    break;
                }
            }
        }
    })
    .detach();
}

/// Activate the existing window, or reopen one bound to the shared session.
fn show_window(cx: &mut AsyncApp) {
    let _ = cx.update(|cx| {
        if let Some(window) = cx.windows().first() {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        } else {
            let options = gpui_kit::gpui::WindowOptions {
                titlebar: Some(gpui_kit::gpui::TitlebarOptions {
                    title: Some("noscat".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| views::root_view(window, cx))
            })
            .expect("failed to reopen window");
        }
    });
}
