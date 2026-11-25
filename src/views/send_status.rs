//! Send feedback where the user acted: under a device's row on the Devices
//! page, and above the input of its conversation. While files go to that
//! device it shows what and how far ("Sending photo.jpg · 42%" with a thin
//! bar); a finished send shows "Sent photo.jpg" for a moment. Failures are
//! toasts (with Try again), and the Files tab keeps the history.

use gpui_kit::component::label::Label;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::{div, px, AnyElement, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _};
use gpui_kit_assets::IconName;

use crate::palette;
use crate::state::{Session, TransferState, SENT_SHOWN_MS};

/// The strip for sends to `peer`, or `None` when there is nothing to say.
pub fn send_status(session: &Session, peer: &str, muted: Hsla) -> Option<AnyElement> {
    let sends: Vec<_> = session
        .transfers
        .iter()
        .filter(|t| t.live && t.outbound && t.peer == peer)
        .collect();

    let active: Vec<_> = sends.iter().filter(|t| t.state == TransferState::Active).collect();
    if !active.is_empty() {
        let sent: u64 = active.iter().map(|t| t.sent).sum();
        let total: u64 = active.iter().map(|t| t.total).sum();
        let pct = if total > 0 { (sent as f64 * 100.0 / total as f64) as f32 } else { 0.0 };
        let what = match active.as_slice() {
            [one] => one.name.clone(),
            many => format!("{} files", many.len()),
        };
        let text = if sent == 0 {
            format!("Sending {what} · connecting")
        } else {
            format!("Sending {what} · {pct:.0}%")
        };
        return Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(Label::new(text).text_xs().truncate().text_color(muted))
                .child(
                    Progress::new(SharedString::from(format!("send-{peer}")))
                        .xsmall()
                        .value(pct)
                        .loading(sent == 0),
                )
                .into_any_element(),
        );
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let just_sent = sends.iter().rev().find(|t| {
        t.state == TransferState::Done && t.ended_at_ms.is_some_and(|at| now.saturating_sub(at) < SENT_SHOWN_MS)
    })?;
    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .text_color(palette::success())
            .child(Icon::new(IconName::Check).with_size(px(13.)))
            .child(Label::new(format!("Sent {}", just_sent.name)).text_xs().truncate())
            .into_any_element(),
    )
}
