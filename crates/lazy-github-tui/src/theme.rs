//! Colors. A placeholder until #77 maps Textual's theme names to palettes; components take every color from here.

use ratatui::style::Color;

/// The colors components draw with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Text.
    pub foreground: Color,
    /// The screen behind everything.
    pub background: Color,
    /// Pane borders.
    pub border: Color,
}

impl Default for Theme {
    /// `textual-dark`, Textual's default theme, approximately.
    fn default() -> Self {
        Self {
            foreground: Color::Rgb(0xe0, 0xe0, 0xe0),
            background: Color::Rgb(0x12, 0x12, 0x12),
            border: Color::Rgb(0x01, 0x78, 0xd4),
        }
    }
}
