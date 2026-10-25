//! Theme color values and Textual's compositing arithmetic.

use ratatui::style::{Color, Modifier, Style};

/// An opaque 24-bit color.
///
/// There is deliberately no `From<Rgb> for Color` (nor for [`Paint`]): an exact-RGB conversion would skip the
/// terminal's color depth. Turn colors into terminal colors with `Theme::paint` or `Theme::color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    pub const WHITE: Rgb = Rgb::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Textual's `Color.brightness`: perceived brightness from 0 (black) to 1 (white).
    fn brightness(self) -> f64 {
        let [r, g, b] = [self.r, self.g, self.b].map(|c| f64::from(c) / 255.0);
        (299.0 * r + 587.0 * g + 114.0 * b) / 1000.0
    }

    /// Textual's `Color.lighten`: `amount` (0 to 1) added to the CIE-L*ab lightness, with Textual's conversions,
    /// `int()` truncation and clamping. A negative `amount` darkens.
    #[must_use]
    pub fn lighten(self, amount: f64) -> Rgb {
        let (l, a, b) = rgb_to_lab(self);
        lab_to_rgb(l + amount * 100.0, a, b)
    }

    /// Textual's `Color.blend` between two opaque colors, including its `int()` truncation.
    #[must_use]
    pub fn blend(self, to: Rgb, factor: f64) -> Rgb {
        if factor <= 0.0 {
            return self;
        }
        if factor >= 1.0 {
            return to;
        }
        // Both ends are in 0..=255 and factor is in (0, 1), so the result is too; `as` truncates like `int()`.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * factor) as u8;
        Rgb::new(mix(self.r, to.r), mix(self.g, to.g), mix(self.b, to.b))
    }
}

/// Textual's `rgb_to_lab`: sRGB to CIE-L*ab through XYZ, D65/2°.
// Textual's formula, with its single-letter names and constants.
#[allow(clippy::many_single_char_names, clippy::unreadable_literal)]
fn rgb_to_lab(c: Rgb) -> (f64, f64, f64) {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v > 0.04045 {
            ((v + 0.055) / 1.055).powf(2.4)
        } else {
            v / 12.92
        }
    };
    let (r, g, b) = (linear(c.r), linear(c.g), linear(c.b));
    let x = (r * 41.24 + g * 35.76 + b * 18.05) / 95.047;
    let y = (r * 21.26 + g * 71.52 + b * 7.22) / 100.0;
    let z = (r * 1.93 + g * 11.92 + b * 95.05) / 108.883;
    let f = |t: f64| {
        if t > 0.008856 {
            t.powf(1.0 / 3.0)
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (x, y, z) = (f(x), f(y), f(z));
    (116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z))
}

/// Textual's `lab_to_rgb` followed by `Color.clamped`.
#[allow(clippy::many_single_char_names, clippy::unreadable_literal)]
fn lab_to_rgb(l: f64, a: f64, b: f64) -> Rgb {
    let off = 16.0 / 116.0;
    let y = (l + 16.0) / 116.0;
    let x = a / 500.0 + y;
    let z = y - b / 200.0;
    let cube = |t: f64, white: f64, linear: f64| {
        if t > 0.206_893_034_4 {
            white * t.powi(3)
        } else {
            linear * (t - off)
        }
    };
    let y = if y > 0.206_893_034_4 {
        y.powi(3)
    } else {
        (y - off) / 7.787
    };
    let x = cube(x, 0.95047, 0.122_059);
    let z = cube(z, 1.08883, 0.139_827);
    let gamma = |v: f64| {
        if v > 0.003_130_8 {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        } else {
            12.92 * v
        }
    };
    let r = gamma(x * 3.2406 + y * -1.5372 + z * -0.4986);
    let g = gamma(x * -0.9689 + y * 1.8758 + z * 0.0415);
    let b = gamma(x * 0.0557 + y * -0.2040 + z * 1.0570);
    // `int()` truncates toward zero, then `clamped` limits to 0..=255; `as` saturates the same way.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let channel = |v: f64| (v * 255.0).trunc().clamp(0.0, 255.0) as u8;
    Rgb::new(channel(r), channel(g), channel(b))
}

/// One of the terminal's 16 named colors, or its default color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    /// The terminal's default foreground or background (`ansi_default`).
    Default,
}

impl AnsiColor {
    /// The RGB Textual assumes for this color when it has to do arithmetic with it (`COLOR_NAME_TO_RGB`).
    fn textual_rgb(self) -> Rgb {
        match self {
            AnsiColor::Black | AnsiColor::Default => Rgb::new(0, 0, 0),
            AnsiColor::Red => Rgb::new(128, 0, 0),
            AnsiColor::Green => Rgb::new(0, 128, 0),
            AnsiColor::Yellow => Rgb::new(128, 128, 0),
            AnsiColor::Blue => Rgb::new(0, 0, 128),
            AnsiColor::Magenta => Rgb::new(128, 0, 128),
            AnsiColor::Cyan => Rgb::new(0, 128, 128),
            AnsiColor::White => Rgb::new(192, 192, 192),
            AnsiColor::BrightBlack => Rgb::new(128, 128, 128),
            AnsiColor::BrightRed => Rgb::new(255, 0, 0),
            AnsiColor::BrightGreen => Rgb::new(0, 255, 0),
            AnsiColor::BrightYellow => Rgb::new(255, 255, 0),
            AnsiColor::BrightBlue => Rgb::new(0, 0, 255),
            AnsiColor::BrightMagenta => Rgb::new(255, 0, 255),
            AnsiColor::BrightCyan => Rgb::new(0, 255, 255),
            AnsiColor::BrightWhite => Rgb::new(255, 255, 255),
        }
    }
}

impl From<AnsiColor> for Color {
    /// Ratatui names the 16 colors differently from ANSI: ANSI 7 (`ansi_white`) is `Gray`, ANSI 8
    /// (`ansi_bright_black`) is `DarkGray` and ANSI 15 (`ansi_bright_white`) is `White`.
    fn from(c: AnsiColor) -> Self {
        match c {
            AnsiColor::Black => Color::Black,
            AnsiColor::Red => Color::Red,
            AnsiColor::Green => Color::Green,
            AnsiColor::Yellow => Color::Yellow,
            AnsiColor::Blue => Color::Blue,
            AnsiColor::Magenta => Color::Magenta,
            AnsiColor::Cyan => Color::Cyan,
            AnsiColor::White => Color::Gray,
            AnsiColor::BrightBlack => Color::DarkGray,
            AnsiColor::BrightRed => Color::LightRed,
            AnsiColor::BrightGreen => Color::LightGreen,
            AnsiColor::BrightYellow => Color::LightYellow,
            AnsiColor::BrightBlue => Color::LightBlue,
            AnsiColor::BrightMagenta => Color::LightMagenta,
            AnsiColor::BrightCyan => Color::LightCyan,
            AnsiColor::BrightWhite => Color::White,
            AnsiColor::Default => Color::Reset,
        }
    }
}

/// What a cell ends up painted with: an opaque RGB color or one of the terminal's own colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Paint {
    Rgb(Rgb),
    Ansi(AnsiColor),
}

impl Paint {
    /// The RGB Textual's arithmetic uses for this paint.
    fn textual_rgb(self) -> Rgb {
        match self {
            Paint::Rgb(c) => c,
            Paint::Ansi(c) => c.textual_rgb(),
        }
    }
}

impl From<Rgb> for Paint {
    fn from(c: Rgb) -> Self {
        Paint::Rgb(c)
    }
}

/// A theme color variable's value, as Textual's CSS parser reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThemeColor {
    /// `#RRGGBB`, `#RRGGBBAA` or `#RRGGBB N%`. The alpha is the exact float Textual blends with (the parsed alpha
    /// times N/100), because Textual truncates every blended channel and a rounded alpha would shift some by one.
    Rgba { r: u8, g: u8, b: u8, a: f64 },
    /// `auto N%`: white or black, whichever contrasts with what it's drawn on, at alpha N/100.
    Auto { alpha: f64 },
    /// `ansi_blue`, `ansi_default`, …: the terminal's own color. Textual ignores alpha on these.
    Ansi(AnsiColor),
    /// `transparent` or `initial`.
    Transparent,
}

/// An opaque [`ThemeColor::Rgba`], for the generated palette.
pub(super) const fn rgb(r: u8, g: u8, b: u8) -> ThemeColor {
    ThemeColor::Rgba { r, g, b, a: 1.0 }
}

/// A [`ThemeColor::Rgba`], for the generated palette.
pub(super) const fn rgba(r: u8, g: u8, b: u8, a: f64) -> ThemeColor {
    ThemeColor::Rgba { r, g, b, a }
}

impl ThemeColor {
    /// The paint a cell gets when `self` is drawn over `under`, with Textual's arithmetic (`under + self`).
    ///
    /// An ANSI color always wins, since Textual can't blend the terminal's colors. Anything else blends with the
    /// RGB Textual assumes for `under`, which for an ANSI `under` is its nominal color (`ansi_default` is black).
    #[must_use]
    pub fn over(self, under: Paint) -> Paint {
        let (to, factor) = match self {
            ThemeColor::Ansi(c) => return Paint::Ansi(c),
            ThemeColor::Transparent => return under,
            ThemeColor::Rgba { r, g, b, a } => (Rgb::new(r, g, b), a),
            ThemeColor::Auto { alpha } => (contrast_text(under.textual_rgb()), alpha),
        };
        if factor <= 0.0 {
            return under;
        }
        Paint::Rgb(under.textual_rgb().blend(to, factor))
    }

    /// Textual's `Color.multiply_alpha`, which is how CSS such as `$foreground 5%` reads. ANSI colors are unchanged.
    #[must_use]
    pub fn multiply_alpha(self, factor: f64) -> ThemeColor {
        match self {
            ThemeColor::Rgba { r, g, b, a } => ThemeColor::Rgba { r, g, b, a: a * factor },
            ThemeColor::Auto { alpha } => ThemeColor::Auto { alpha: alpha * factor },
            ThemeColor::Ansi(_) | ThemeColor::Transparent => self,
        }
    }
}

/// Textual's `get_contrast_text`, before its alpha is applied.
fn contrast_text(background: Rgb) -> Rgb {
    if background.brightness() < 0.5 {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

/// A Textual text-style variable such as `bold`, `b reverse` or `bold not underline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    /// Turned on.
    pub add: Modifier,
    /// Turned off (`not …`).
    pub sub: Modifier,
}

impl TextStyle {
    /// `style` with this text style applied.
    #[must_use]
    pub fn apply(self, style: Style) -> Style {
        style.add_modifier(self.add).remove_modifier(self.sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_truncates_like_textual() {
        // 0x12 + (0xFF - 0x12) * 10/255 = 18 + 9.29 → 27
        let boost = rgba(0xFF, 0xFF, 0xFF, 10.0 / 255.0);
        assert_eq!(
            boost.over(Rgb::new(0x12, 0x12, 0x12).into()),
            Paint::Rgb(Rgb::new(27, 27, 27))
        );
    }

    #[test]
    fn auto_contrasts_with_what_it_is_drawn_on() {
        let text = ThemeColor::Auto { alpha: 0.87 };
        assert_eq!(text.over(Rgb::BLACK.into()), Paint::Rgb(Rgb::new(221, 221, 221)));
        assert_eq!(text.over(Rgb::WHITE.into()), Paint::Rgb(Rgb::new(33, 33, 33)));
    }

    #[test]
    fn ansi_passes_through_and_transparent_shows_what_is_under() {
        let under = Paint::Rgb(Rgb::new(1, 2, 3));
        assert_eq!(
            ThemeColor::Ansi(AnsiColor::Blue).over(under),
            Paint::Ansi(AnsiColor::Blue)
        );
        assert_eq!(ThemeColor::Transparent.over(under), under);
        assert_eq!(rgba(9, 9, 9, 0.0).over(under), under);
        let ansi_under = Paint::Ansi(AnsiColor::Default);
        assert_eq!(ThemeColor::Transparent.over(ansi_under), ansi_under);
    }

    #[test]
    fn multiply_alpha_leaves_ansi_alone() {
        assert_eq!(rgb(1, 2, 3).multiply_alpha(0.05), rgba(1, 2, 3, 0.05));
        assert_eq!(
            ThemeColor::Ansi(AnsiColor::Default).multiply_alpha(0.5),
            ThemeColor::Ansi(AnsiColor::Default)
        );
    }

    #[test]
    fn ansi_names_map_to_ratatui_by_number() {
        assert_eq!(Color::from(AnsiColor::Black), Color::Black);
        assert_eq!(Color::from(AnsiColor::White), Color::Gray);
        assert_eq!(Color::from(AnsiColor::BrightBlack), Color::DarkGray);
        assert_eq!(Color::from(AnsiColor::BrightWhite), Color::White);
        assert_eq!(Color::from(AnsiColor::Default), Color::Reset);
    }

    #[test]
    fn text_style_adds_and_removes() {
        let hover = TextStyle {
            add: Modifier::BOLD,
            sub: Modifier::UNDERLINED,
        };
        let style = hover.apply(Style::new().add_modifier(Modifier::UNDERLINED | Modifier::ITALIC));
        assert_eq!(style.add_modifier, Modifier::BOLD | Modifier::ITALIC);
    }
}
