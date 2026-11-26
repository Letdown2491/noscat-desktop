//! System tray over StatusNotifierItem (ksni): the node keeps running when
//! the window closes, like the Android foreground service. The tray menu
//! reopens the window or quits the app.

use std::sync::OnceLock;

use gpui_kit::gpui::{App, AsyncApp};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem};

use crate::views;

/// Commands the tray sends into the GPUI app thread.
pub enum Command {
    Show,
    Quit,
}

pub struct NoscatTray {
    tx: async_channel::Sender<Command>,
    /// A member has a terminal open on this computer: the icon turns
    /// amber, so it shows even with the window closed.
    hosting: bool,
}

static HANDLE: OnceLock<ksni::blocking::Handle<NoscatTray>> = OnceLock::new();

/// Say whether a member has a terminal open here. Off the main thread:
/// the tray update waits on its D-Bus service.
pub fn set_hosting(on: bool) {
    if let Some(h) = HANDLE.get().cloned() {
        std::thread::spawn(move || {
            h.update(|t| t.hosting = on);
        });
    }
}

impl ksni::Tray for NoscatTray {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    fn title(&self) -> String {
        if self.hosting { format!("{}: a terminal is open on this computer", crate::APP_NAME) } else { crate::APP_NAME.into() }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.title(), ..Default::default() }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // 32x32 solid noscat green (amber while hosting a terminal),
        // ARGB32 network byte order.
        let pixel = if self.hosting { [0xff, 0xf5, 0x9e, 0x0b] } else { [0xff, 0x22, 0xc5, 0x5e] };
        let mut data = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..32 * 32 {
            data.extend_from_slice(&pixel);
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
                label: format!("Show {}", crate::APP_NAME),
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
            match (NoscatTray { tx, hosting: false }).spawn() {
                Ok(h) => {
                    let _ = HANDLE.set(h);
                }
                Err(e) => eprintln!("tray unavailable: {e}"),
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

/// Activate the main window, or reopen it.
fn show_window(cx: &mut AsyncApp) {
    let _ = cx.update(views::show_main_window);
}
