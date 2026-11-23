//! System tray: the node keeps running when the window closes, like the
//! Android foreground service. The tray offers show/hide and quit.
//!
//! Status: wired to a no-op for now. The Linux implementation plugs in here
//! over StatusNotifierItem (the ksni crate), added in the tray milestone.

/// Handle to the tray so the window can update its icon/menu state.
pub struct TrayHandle;

impl TrayHandle {
    pub fn new() -> Self {
        Self
    }
}
