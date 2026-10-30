//! Textual's `*:disabled:can-focus { opacity: 0.7 }`: a disabled input, switch or select is drawn as usual, then
//! faded toward its parent's background. Buttons have their own `:disabled` styles (`button.rs`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::theme::{Paint, Theme};

/// The opacity Textual gives a disabled widget that could otherwise take focus.
const OPACITY: f64 = 0.7;

/// Fades the cells of `area` already drawn into `buf` toward `under`, the parent's background. The ANSI themes can't
/// blend, so their cells are dimmed instead.
///
/// Textual applies the opacity twice to a widget's background: once as it composites the background over its
/// parent's, and again as it blends the rendered cells into the parent. Text is blended once. (Its translucent border
/// colors, composited over the faded background, come out between the two; they're faded once here.)
pub fn fade(buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint) {
    for position in area.intersection(buf.area).positions() {
        let cell = &mut buf[position];
        if theme.ansi {
            cell.modifier.insert(Modifier::DIM);
        } else {
            cell.fg = theme.fade(cell.fg, under, OPACITY);
            cell.bg = theme.fade(theme.fade(cell.bg, under, OPACITY), under, OPACITY);
        }
    }
}
