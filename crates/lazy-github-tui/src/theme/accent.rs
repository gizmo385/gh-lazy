//! Colors the Python app writes as markup, which stay the same in every theme.

use super::color::Rgb;

/// A named color from Textual markup (`[greenyellow]…`), with Textual's RGB for that name, or a hex color the Python
/// app styles text with (the diff viewer's line backgrounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Accent {
    GreenYellow,
    Red,
    Orchid,
    Yellow,
    Cyan,
    Green,
    /// A removed line in a hunk pane: `on #3a0a0a`.
    DiffRemoved,
    /// An added line in a hunk pane: `on #0a3a0a`.
    DiffAdded,
    /// The current line in a hunk pane, when it's removed: `on #8b0000`.
    DiffRemovedCurrent,
    /// The current line in a hunk pane, when it's added: `on #006400`.
    DiffAddedCurrent,
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
            Accent::DiffRemoved => Rgb::new(0x3A, 0x0A, 0x0A),
            Accent::DiffAdded => Rgb::new(0x0A, 0x3A, 0x0A),
            Accent::DiffRemovedCurrent => Rgb::new(0x8B, 0x00, 0x00),
            Accent::DiffAddedCurrent => Rgb::new(0x00, 0x64, 0x00),
        }
    }
}

/// An ANSI color from Rich markup inside `DataTable` cells (`[green]★[/]`) and `Tree` labels (the Jobs tab's `[red]✗`),
/// or a Rich `Text` style (the diff viewer's `green` counts). Outside the ANSI themes, Textual paints these through its terminal theme: MONOKAI for dark themes,
/// ALABASTER for light ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RichAnsi {
    Green,
    Red,
    Yellow,
    Cyan,
}

impl RichAnsi {
    /// The color under a dark theme (MONOKAI).
    pub const fn dark(self) -> Rgb {
        match self {
            RichAnsi::Green => Rgb::new(0x98, 0xE0, 0x24),
            RichAnsi::Red => Rgb::new(0xF4, 0x00, 0x5F),
            RichAnsi::Yellow => Rgb::new(0xFD, 0x97, 0x1F),
            RichAnsi::Cyan => Rgb::new(0x58, 0xD1, 0xEB),
        }
    }

    /// The color under a light theme (ALABASTER).
    pub const fn light(self) -> Rgb {
        match self {
            RichAnsi::Green => Rgb::new(0x44, 0x8C, 0x27),
            RichAnsi::Red => Rgb::new(0xAA, 0x37, 0x31),
            RichAnsi::Yellow => Rgb::new(0xCB, 0x90, 0x00),
            RichAnsi::Cyan => Rgb::new(0x00, 0x83, 0xB2),
        }
    }
}
