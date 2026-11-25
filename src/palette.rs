//! The app's fixed colors in one place. The theme supplies surfaces and
//! text; these are the status colors that must read the same in light and
//! dark mode (presence dots, delivery states, the blue of sent bubbles and
//! badges), plus the per-device avatar hues.

use gpui_kit::gpui::{rgb, Hsla};

fn hex(value: u32) -> Hsla {
    Hsla::from(rgb(value))
}

/// Connected, delivered, running.
pub fn success() -> Hsla {
    hex(0x22c55e)
}

/// Starting, partial, stopped by the user.
pub fn warning() -> Hsla {
    hex(0xeab308)
}

/// Online but unreachable (the amber presence dot).
pub fn amber() -> Hsla {
    hex(0xf59e0b)
}

/// Errors and failures on the theme's surfaces.
pub fn danger() -> Hsla {
    hex(0xef4444)
}

/// Failure text on the blue of a sent bubble.
pub fn danger_on_blue() -> Hsla {
    hex(0xfca5a5)
}

/// Offline (the grey presence dot).
pub fn offline() -> Hsla {
    hex(0x94a3b8)
}

/// Sent bubbles, unread and request badges, the join request accent.
pub fn blue() -> Hsla {
    hex(0x2563eb)
}

/// Text on `blue` and on avatar discs.
pub fn on_color() -> Hsla {
    hex(0xffffff)
}

/// A stable avatar color per device, from its ID: devices keep their color
/// everywhere, and a list of them scans by color. Mid tones that carry
/// white text in both themes.
pub fn avatar(id: &str) -> Hsla {
    const HUES: [u32; 8] = [
        0x2563eb, 0x7c3aed, 0xdb2777, 0xdc2626, 0xd97706, 0x059669, 0x0891b2, 0x4f46e5,
    ];
    // FNV-1a: stable across runs and platforms (std's hasher is not).
    let mut h: u32 = 0x811c9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    hex(HUES[(h as usize) % HUES.len()])
}
