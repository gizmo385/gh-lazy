//! Colors the Python app writes as markup, which stay the same in every theme.

use super::color::Rgb;

/// A named color from Textual markup (`[greenyellow]…`), with Textual's RGB for that name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Accent {
    GreenYellow,
    Red,
    Orchid,
    Yellow,
    Cyan,
    Green,
}

impl Accent {
    pub const fn rgb(self) -> Rgb {
        match self {
            Accent::GreenYellow => Rgb::new(0xAD, 0xFF, 0x2F),
            Accent::Red => Rgb::new(0xFF, 0x00, 0x00),
            Accent::Orchid => Rgb::new(0xDA, 0x70, 0xD6),
            Accent::Yellow => Rgb::new(0xFF, 0xFF, 0x00),
            Accent::Cyan => Rgb::new(0x00, 0xFF, 0xFF),
            Accent::Green => Rgb::new(0x00, 0x80, 0x00),
        }
    }
}

/// An ANSI color from Rich markup inside `DataTable` cells (`[green]★[/]`). Outside the ANSI themes, Textual paints
/// these through its terminal theme: MONOKAI for dark themes, ALABASTER for light ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RichAnsi {
    Green,
}

impl RichAnsi {
    /// The color under a dark theme (MONOKAI).
    pub const fn dark(self) -> Rgb {
        match self {
            RichAnsi::Green => Rgb::new(0x98, 0xE0, 0x24),
        }
    }

    /// The color under a light theme (ALABASTER).
    pub const fn light(self) -> Rgb {
        match self {
            RichAnsi::Green => Rgb::new(0x44, 0x8C, 0x27),
        }
    }
}
