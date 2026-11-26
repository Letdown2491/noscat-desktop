//! A terminal on another device, in its own window titled with the
//! device's name. The member runs the shell (its user's login shell, as
//! that user); this window parses and draws what it sends, with
//! Alacritty's terminal state machine, and turns keys into terminal bytes.
//! Closing the window ends the session.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    canvas, div, fill, font, point, px, size, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    FontStyle, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels,
    Render, Rgba, ScrollEvent, SharedString, Styled as _, TextAlign, TextRun, TitlebarOptions, UnderlineStyle,
    Window, WindowBounds, WindowOptions,
};
use noscat_client::TerminalHandle;

use crate::app::Dismiss;
use crate::state::Session;

const FONT_SIZE: f32 = 14.;
const LINE_HEIGHT: f32 = 1.25;
const PAD: f32 = 6.;
const SCROLLBACK: usize = 10_000;

/// Open a terminal window on `peer`. The session starts once the window
/// knows its size; the member may ask its user first.
pub fn open(session: Entity<Session>, peer: String, name: String, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(None, size(px(820.), px(520.)), cx)),
        window_min_size: Some(size(px(320.), px(160.))),
        titlebar: Some(TitlebarOptions { title: Some(format!("{name}: terminal").into()), ..Default::default() }),
        ..Default::default()
    };
    let _ = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| TerminalView::new(session, peer, name, window, cx))
    });
}

/// Where a session stands, for the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Opening,
    Waiting,
    Open,
    Ended(String),
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        if matches!(event, TermEvent::PtyWrite(_) | TermEvent::Title(_) | TermEvent::ResetTitle) {
            if let Ok(mut q) = self.0.lock() {
                q.push(event);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct TerminalView {
    session: Entity<Session>,
    pub peer: String,
    name: String,
    pub phase: Phase,
    handle: Option<TerminalHandle>,
    term: Term<Listener>,
    listener: Listener,
    parser: Processor,
    grid: GridSize,
    requested: bool,
    title: Option<String>,
    focus: FocusHandle,
}

impl TerminalView {
    fn new(session: Entity<Session>, peer: String, name: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let listener = Listener::default();
        let grid = GridSize { cols: 80, rows: 24 };
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            session,
            peer,
            name,
            phase: Phase::Opening,
            handle: None,
            term: Term::new(config, &grid, listener.clone()),
            listener,
            parser: Processor::new(),
            grid,
            requested: false,
            title: None,
            focus,
        }
    }

    /// Output from the member.
    pub fn feed(&mut self, data: &[u8], cx: &mut Context<Self>) {
        self.parser.advance(&mut self.term, data);
        let events = self.listener.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for e in events {
            match e {
                // Replies the terminal owes the program (cursor position,
                // device attributes): they go back to the member.
                TermEvent::PtyWrite(s) => self.write(s.as_bytes()),
                TermEvent::Title(t) => self.title = Some(t),
                TermEvent::ResetTitle => self.title = None,
                _ => {}
            }
        }
        cx.notify();
    }

    pub fn waiting(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Opening {
            self.phase = Phase::Waiting;
            cx.notify();
        }
    }

    pub fn started(&mut self, handle: TerminalHandle, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Ended(_)) {
            return; // dropping the handle ends the late session
        }
        // The window may have changed size while the member decided.
        handle.resize(self.grid.cols as u16, self.grid.rows as u16);
        self.handle = Some(handle);
        self.phase = Phase::Open;
        cx.notify();
    }

    pub fn ended(&mut self, text: String, cx: &mut Context<Self>) {
        self.handle = None;
        self.phase = Phase::Ended(text);
        cx.notify();
    }

    fn write(&self, bytes: &[u8]) {
        if let Some(h) = &self.handle {
            h.write(bytes);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        // Escape arrives as the app's Dismiss action (bound globally).
        if ks.key == "escape" && !m.control && !m.alt && !m.shift {
            return;
        }
        if m.control && m.shift && ks.key == "v" {
            self.paste(cx);
            return;
        }
        if let Some(bytes) = key_bytes(ks, *self.term.mode()) {
            self.term.scroll_display(Scroll::Bottom);
            self.write(&bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            self.write(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let line = px(FONT_SIZE * LINE_HEIGHT);
        let dy = ev.delta.pixel_delta(line).y;
        let lines = (f32::from(dy) / f32::from(line)).round() as i32;
        if lines != 0 {
            self.term.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }

    /// Fit the grid to the window; tell the member when it changed.
    fn fit(&mut self, cell: (Pixels, Pixels), window: &Window) {
        let area = window.viewport_size();
        let cols = ((f32::from(area.width) - 2. * PAD) / f32::from(cell.0)).floor().max(4.) as usize;
        let rows = ((f32::from(area.height) - 2. * PAD) / f32::from(cell.1)).floor().max(2.) as usize;
        let next = GridSize { cols, rows };
        if next != self.grid {
            self.grid = next;
            self.term.resize(next);
            if let Some(h) = &self.handle {
                h.resize(cols as u16, rows as u16);
            }
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// One row, ready to paint.
struct Row {
    text: String,
    runs: Vec<TextRun>,
    /// Background spans: (first column, columns, color).
    backgrounds: Vec<(usize, usize, Hsla)>,
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let family = cx.theme().mono_font_family.clone();
        let base = font(family);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&base);
        let cell_w = text_system.advance(font_id, font_size, 'm').map(|s| s.width).unwrap_or(px(FONT_SIZE * 0.6));
        let cell_h = px((FONT_SIZE * LINE_HEIGHT).round());
        self.fit((cell_w, cell_h), window);

        // Start once the size is known (deferred: it updates the session).
        if !self.requested {
            self.requested = true;
            let (cols, rows) = (self.grid.cols as u16, self.grid.rows as u16);
            let peer = self.peer.clone();
            cx.defer_in(window, move |this, _, cx| {
                let me = cx.weak_entity();
                this.session.update(cx, |s, cx| s.start_terminal(me, peer, cols, rows, cx));
            });
        }
        let title = match &self.title {
            Some(t) if !t.trim().is_empty() => format!("{}: {}", self.name, t.trim()),
            _ => format!("{}: terminal", self.name),
        };
        window.set_window_title(&title);

        let default_bg = rgb(BG);
        let (rows, cursor) = snapshot(&self.term, &base, default_bg);
        let focused = self.focus.is_focused(window);
        let show_cursor = self.phase == Phase::Open;

        let grid = canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, _, window, cx| {
                let origin = bounds.origin + point(px(PAD), px(PAD));
                for (i, row) in rows.iter().enumerate() {
                    let y = origin.y + cell_h * i as f32;
                    for &(col, n, color) in &row.backgrounds {
                        window.paint_quad(fill(
                            Bounds::new(point(origin.x + cell_w * col as f32, y), size(cell_w * n as f32, cell_h)),
                            color,
                        ));
                    }
                    if row.text.trim().is_empty() {
                        continue;
                    }
                    let line = window.text_system().shape_line(
                        SharedString::from(row.text.clone()),
                        font_size,
                        &row.runs,
                        Some(cell_w),
                    );
                    let _ = line.paint(point(origin.x, y), cell_h, TextAlign::Left, None, window, cx);
                }
                if let (true, Some((col, row))) = (show_cursor, cursor) {
                    let r = Bounds::new(
                        point(origin.x + cell_w * col as f32, origin.y + cell_h * row as f32),
                        size(cell_w, cell_h),
                    );
                    let c = rgb(CURSOR);
                    if focused {
                        window.paint_quad(fill(r, c.opacity(0.6)));
                    } else {
                        window.paint_quad(gpui_kit::gpui::outline(r, c, gpui_kit::gpui::BorderStyle::Solid));
                    }
                }
            },
        )
        .size_full();

        let overlay = match &self.phase {
            Phase::Opening | Phase::Waiting => Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .p_6()
                    .rounded_lg()
                    .bg(cx.theme().background)
                    .child(Spinner::new())
                    .child(Label::new(if self.phase == Phase::Waiting {
                        format!("Waiting for {} to approve", self.name)
                    } else {
                        format!("Opening a terminal on {}", self.name)
                    }))
                    .into_any_element(),
            ),
            Phase::Ended(text) => Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .p_6()
                    .rounded_lg()
                    .bg(cx.theme().background)
                    .child(Label::new(text.clone()))
                    .child(
                        Button::new("close-terminal")
                            .label("Close")
                            .primary()
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .into_any_element(),
            ),
            Phase::Open => None,
        };

        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(default_bg)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                this.write(b"\x1b");
                cx.notify();
            }))
            .on_mouse_down(gpui_kit::gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
                window.focus(&this.focus, cx);
            }))
            .child(grid)
            .when_some(overlay, |d, o| {
                d.child(div().absolute().inset_0().flex().items_center().justify_center().child(o))
            })
    }
}

// ---- drawing ---------------------------------------------------------------

/// Tomorrow Night, Alacritty's default palette.
const BG: u32 = 0x1d1f21;
const FG: u32 = 0xc5c8c6;
const CURSOR: u32 = 0xc5c8c6;
const ANSI: [u32; 16] = [
    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666, 0xd54e53, 0xb9ca4a,
    0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];

fn rgb(hex: u32) -> Hsla {
    Rgba { r: ((hex >> 16) & 0xff) as f32 / 255., g: ((hex >> 8) & 0xff) as f32 / 255., b: (hex & 0xff) as f32 / 255., a: 1. }
        .into()
}

fn from_rgb(c: Rgb) -> Hsla {
    rgb((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b))
}

/// The xterm 256-color palette above the 16 ANSI colors.
fn indexed(i: u8) -> u32 {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + u32::from(v) * 40 };
            (level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6)
        }
        _ => {
            let v = 8 + u32::from(i - 232) * 10;
            (v << 16) | (v << 8) | v
        }
    }
}

fn color(c: Color, colors: &alacritty_terminal::term::color::Colors, fg: bool) -> Hsla {
    match c {
        Color::Spec(rgb_) => from_rgb(rgb_),
        Color::Indexed(i) => colors[i as usize].map_or_else(|| rgb(indexed(i)), from_rgb),
        Color::Named(n) => {
            if let Some(c) = colors[n] {
                return from_rgb(c);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => rgb(FG),
                NamedColor::Background => rgb(BG),
                NamedColor::Cursor => rgb(CURSOR),
                NamedColor::DimForeground => rgb(FG).opacity(0.7),
                n if (n as usize) < 16 => rgb(ANSI[n as usize]),
                n => {
                    // Dim variants: the normal color, faded.
                    let i = (n as usize).saturating_sub(NamedColor::DimBlack as usize) % 8;
                    if fg { rgb(ANSI[i]).opacity(0.7) } else { rgb(ANSI[i]) }
                }
            }
        }
    }
}

/// The visible rows as text, style runs and background spans, plus the
/// cursor cell when it is on screen.
fn snapshot(term: &Term<Listener>, base: &gpui_kit::gpui::Font, default_bg: Hsla) -> (Vec<Row>, Option<(usize, usize)>) {
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let rows_n = term.screen_lines();
    let cols_n = term.columns();
    let mut rows: Vec<Row> = (0..rows_n).map(|_| Row { text: String::new(), runs: Vec::new(), backgrounds: Vec::new() }).collect();
    let mut cols_done = vec![0usize; rows_n];
    let colors = content.colors;
    for item in content.display_iter {
        let r = item.point.line.0 + offset;
        if r < 0 || r as usize >= rows_n {
            continue;
        }
        let r = r as usize;
        let col = item.point.column.0;
        if col >= cols_n {
            continue;
        }
        let cell = item.cell;
        let flags = cell.flags;
        let (mut fg, mut bg) = (color(cell.fg, colors, true), color(cell.bg, colors, false));
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = fg.opacity(0.7);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let ch = if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) || cell.c == '\0' {
            ' '
        } else {
            cell.c
        };
        let row = &mut rows[r];
        // Pad skipped columns (the iterator may skip nothing, but be safe).
        while cols_done[r] < col {
            push(row, ' ', base.clone(), rgb(FG), None);
            cols_done[r] += 1;
        }
        let mut f = base.clone();
        if flags.contains(Flags::BOLD) {
            f.weight = FontWeight::BOLD;
        }
        if flags.contains(Flags::ITALIC) {
            f.style = FontStyle::Italic;
        }
        let underline = flags.contains(Flags::UNDERLINE).then_some(UnderlineStyle { thickness: px(1.), color: Some(fg), wavy: false });
        push(row, ch, f, fg, underline);
        if bg != default_bg {
            match row.backgrounds.last_mut() {
                Some((start, n, c)) if *c == bg && *start + *n == col => *n += 1,
                _ => row.backgrounds.push((col, 1, bg)),
            }
        }
        cols_done[r] = col + 1;
    }
    let cursor = {
        let p = content.cursor.point;
        let r = p.line.0 + offset;
        (content.mode.contains(TermMode::SHOW_CURSOR) && r >= 0 && (r as usize) < rows_n)
            .then_some((p.column.0, r as usize))
    };
    (rows, cursor)
}

/// Append one character, extending the last run when the style matches.
fn push(row: &mut Row, ch: char, f: gpui_kit::gpui::Font, fg: Hsla, underline: Option<UnderlineStyle>) {
    let len = ch.len_utf8();
    row.text.push(ch);
    if let Some(last) = row.runs.last_mut() {
        if last.font == f && last.color == fg && last.underline == underline {
            last.len += len;
            return;
        }
    }
    row.runs.push(TextRun { len, font: f, color: fg, background_color: None, underline, strikethrough: None });
}

// ---- keys ------------------------------------------------------------------

/// The bytes a terminal sends for a keystroke, or `None` for keys it
/// leaves alone.
fn key_bytes(ks: &gpui_kit::gpui::Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = ks.modifiers;
    if m.platform {
        return None;
    }
    // xterm modifier parameter: 1 + shift + 2 alt + 4 ctrl.
    let param = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let cursor_key = |c: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 { format!("\x1b[{n};{param}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let bytes = match ks.key.as_str() {
        "enter" => b"\r".to_vec(),
        "backspace" => if m.control { vec![0x08] } else if m.alt { vec![0x1b, 0x7f] } else { vec![0x7f] },
        "tab" => if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
        "escape" => vec![0x1b],
        "up" => cursor_key('A'),
        "down" => cursor_key('B'),
        "right" => cursor_key('C'),
        "left" => cursor_key('D'),
        "home" => cursor_key('H'),
        "end" => cursor_key('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        key => {
            let mut out = Vec::new();
            if m.control {
                // Ctrl with a letter or one of @[\]^_ gives a control code.
                let c = key.chars().next()?;
                if key.chars().count() != 1 {
                    return None;
                }
                let code = match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => l as u8 - b'a' + 1,
                    '@' | '2' | ' ' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '/' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return None,
                };
                if m.alt {
                    out.push(0x1b);
                }
                out.push(code);
                return Some(out);
            }
            let text = ks.key_char.clone().or_else(|| (key == "space").then(|| " ".to_string()))?;
            if m.alt {
                out.push(0x1b);
            }
            out.extend_from_slice(text.as_bytes());
            out
        }
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &'static str) -> gpui_kit::gpui::Keystroke {
        gpui_kit::gpui::Keystroke::parse(s).expect("keystroke")
    }

    #[test]
    fn keys_become_terminal_bytes() {
        let normal = TermMode::empty();
        assert_eq!(key_bytes(&ks("enter"), normal).unwrap(), b"\r");
        assert_eq!(key_bytes(&ks("ctrl-c"), normal).unwrap(), [3]);
        assert_eq!(key_bytes(&ks("up"), normal).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(&ks("up"), TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
        assert_eq!(key_bytes(&ks("ctrl-right"), normal).unwrap(), b"\x1b[1;5C");
        assert_eq!(key_bytes(&ks("shift-tab"), normal).unwrap(), b"\x1b[Z");
        assert_eq!(key_bytes(&ks("pagedown"), normal).unwrap(), b"\x1b[6~");
        assert_eq!(key_bytes(&ks("backspace"), normal).unwrap(), [0x7f]);
    }

    #[test]
    fn output_becomes_rows_with_colors_and_a_cursor() {
        let grid = GridSize { cols: 10, rows: 3 };
        let listener = Listener::default();
        let mut term = Term::new(Config::default(), &grid, listener.clone());
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"hi\r\n\x1b[31mred\x1b[0m!\x1b[6n");
        let base = font("monospace");
        let (rows, cursor) = snapshot(&term, &base, rgb(BG));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].text.trim_end(), "hi");
        assert_eq!(rows[1].text.trim_end(), "red!");
        assert_eq!(rows[1].runs[0].len, 3);
        assert_eq!(rows[1].runs[0].color, rgb(ANSI[1]));
        assert_eq!(rows[1].runs[1].color, rgb(FG));
        assert_eq!(cursor, Some((4, 1)));
        // The cursor position report owed to the program.
        let replies = listener.0.lock().unwrap();
        assert!(matches!(replies.as_slice(), [TermEvent::PtyWrite(r)] if r == "\x1b[2;5R"));
    }

    #[test]
    fn the_256_color_cube_matches_xterm() {
        assert_eq!(indexed(16), 0x000000);
        assert_eq!(indexed(196), 0xff0000);
        assert_eq!(indexed(231), 0xffffff);
        assert_eq!(indexed(232), 0x080808);
        assert_eq!(indexed(255), 0xeeeeee);
    }
}
