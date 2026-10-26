//! How many colors the terminal shows, and reducing theme colors to them the way Rich does.
//!
//! Textual hands every color to Rich as 24-bit RGB, and Rich downgrades it to what it detected from the environment:
//! the nearest xterm-256 index, or the nearest of the 16 ANSI colors. Doing the same reduction here means the two
//! apps look alike in a 256-color terminal such as macOS Terminal.app, not just in a truecolor one.

use ratatui::style::Color;

use super::color::{AnsiColor, Paint, Rgb};

/// How many colors the terminal can show: Rich's `ColorSystem`, without its legacy Windows console mode. Rich's
/// "no color" never reaches the screen in Textual (see [`ColorDepth::detect`]), so it has no variant here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorDepth {
    /// 24-bit RGB.
    #[default]
    TrueColor,
    /// The xterm-256 palette.
    EightBit,
    /// The 16 ANSI colors.
    Standard,
}

impl ColorDepth {
    /// The depth Textual would use, from the environment `env` reads.
    ///
    /// `TEXTUAL_COLOR_SYSTEM` (`truecolor`, `256`, `standard` or `auto`, the default) picks one outright, as in
    /// Textual. Otherwise Rich's `Console._detect_color_system`: `COLORTERM` of `truecolor` or `24bit` means
    /// truecolor, and failing that the part of `TERM` after its last `-` decides (`256color` and `kitty` are 8-bit);
    /// anything else is the 16 standard colors. `TERM` of `dumb` or `unknown` is truecolor: Rich detects no color
    /// there, but Textual's `Strip.render` (textual 8.2.5 `strip.py` line 718,
    /// `console._color_system or ColorSystem.TRUECOLOR`) then renders in truecolor. Textual always tells Rich it's
    /// writing to a terminal, so there is no "not a terminal" case.
    ///
    /// Windows is treated as truecolor rather than probing the console as Rich does; Windows is a stretch goal.
    pub fn detect(env: &impl Fn(&str) -> Option<String>) -> Self {
        Self::detect_on(env, cfg!(windows))
    }

    fn detect_on(env: &impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        match env("TEXTUAL_COLOR_SYSTEM").as_deref() {
            Some("truecolor") => return ColorDepth::TrueColor,
            Some("256") => return ColorDepth::EightBit,
            Some("standard") => return ColorDepth::Standard,
            Some("auto" | "") | None => {}
            // Rich would raise on an unknown name; detecting is the useful fallback.
            Some(other) => tracing::debug!("unknown TEXTUAL_COLOR_SYSTEM {other:?}, detecting the color depth"),
        }

        let term = env("TERM").unwrap_or_default();
        // Rich detects no color here, which Textual's `Strip.render` turns into truecolor. Rich doesn't strip `TERM`
        // for this check, only for the one below.
        if windows || matches!(term.to_lowercase().as_str(), "dumb" | "unknown") {
            return ColorDepth::TrueColor;
        }
        let color_term = env("COLORTERM").unwrap_or_default().trim().to_lowercase();
        if matches!(color_term.as_str(), "truecolor" | "24bit") {
            return ColorDepth::TrueColor;
        }
        let term = term.trim().to_lowercase();
        // Python's `rpartition("-")`: the whole name when there is no `-`.
        match term.rsplit('-').next().unwrap_or_default() {
            "256color" | "kitty" => ColorDepth::EightBit,
            _ => ColorDepth::Standard,
        }
    }
}

/// Everything about the terminal that changes how a theme color is shown. Detected once at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColorMode {
    /// How many colors the terminal shows.
    pub depth: ColorDepth,
    /// `NO_COLOR` is set: RGB colors become gray and the terminal's own colors become its default.
    pub no_color: bool,
}

impl ColorMode {
    /// The mode for the environment `env` reads: [`ColorDepth::detect`], and `NO_COLOR` set to any value, even an
    /// empty one, as Textual checks it.
    pub fn detect(env: &impl Fn(&str) -> Option<String>) -> Self {
        Self {
            depth: ColorDepth::detect(env),
            no_color: env("NO_COLOR").is_some(),
        }
    }

    /// [`ColorMode::detect`] on this process's environment.
    pub fn from_env() -> Self {
        Self::detect(&|name| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned()))
    }

    /// The terminal color for a composited paint in this mode.
    ///
    /// The terminal's own colors pass through at every depth. Under `NO_COLOR` they become the terminal's default
    /// color, which is a deliberate deviation from the Python app. Textual chooses its `NO_COLOR` filter in
    /// `App.__init__`, before LazyGithub applies the configured theme in `on_mount`, so Python always gets
    /// `Monochrome`, even under an ANSI theme. That filter has no RGB for an ANSI color and paints it `#000000`,
    /// which makes the ANSI themes black on black. `Reset` is what Textual's own `NoColor` filter would give.
    pub fn paint(self, paint: Paint) -> Color {
        match paint {
            Paint::Rgb(c) => downsample(c, self),
            Paint::Ansi(_) if self.no_color => Color::Reset,
            Paint::Ansi(c) => c.into(),
        }
    }
}

/// `rgb` as the terminal shows it in `mode`: gray first under `NO_COLOR` (Textual's `Color.monochrome`), then reduced
/// to the depth the way Rich's `Color.downgrade` does it.
pub fn downsample(rgb: Rgb, mode: ColorMode) -> Color {
    let rgb = if mode.no_color { monochrome(rgb) } else { rgb };
    match mode.depth {
        ColorDepth::TrueColor => Color::Rgb(rgb.r, rgb.g, rgb.b),
        ColorDepth::EightBit => Color::Indexed(eight_bit(rgb)),
        ColorDepth::Standard => STANDARD_PALETTE[standard(rgb)].0.into(),
    }
}

/// The RGB a terminal color stands for: exact for RGB, the xterm-256 cube or gray ramp for an index from
/// [`downsample`], and Rich's [`STANDARD_PALETTE`] for the 16 named colors. `None` for the terminal's default color.
///
/// This undoes [`downsample`] closely enough to blend a color that's already been drawn.
pub(crate) fn rgb_of(color: Color) -> Option<Rgb> {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match color {
        Color::Rgb(r, g, b) => Some(Rgb::new(r, g, b)),
        Color::Indexed(i @ 16..=231) => {
            let i = usize::from(i - 16);
            Some(Rgb::new(CUBE[i / 36], CUBE[i / 6 % 6], CUBE[i % 6]))
        }
        Color::Indexed(i @ 232..=255) => {
            let gray = 8 + 10 * (i - 232);
            Some(Rgb::new(gray, gray, gray))
        }
        Color::Indexed(i) => Some(STANDARD_PALETTE[usize::from(i)].1),
        Color::Reset => None,
        named => STANDARD_PALETTE
            .iter()
            .find(|&&(ansi, _)| Color::from(ansi) == named)
            .map(|&(_, rgb)| rgb),
    }
}

/// Textual's `Color.monochrome`.
fn monochrome(c: Rgb) -> Rgb {
    // Python's `round` rounds half to even. The weights sum to 1, so the result is in 0..=255 and `as` can't wrap.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let gray = (f64::from(c.r) * 0.2126 + f64::from(c.g) * 0.7152 + f64::from(c.b) * 0.0722).round_ties_even() as u8;
    Rgb::new(gray, gray, gray)
}

/// Rich's truecolor to 8-bit downgrade: a gray ramp index for low saturation, else the 6×6×6 cube.
fn eight_bit(c: Rgb) -> u8 {
    let (lightness, saturation) = lightness_saturation(c);
    // Both values below are rounded the way Python's `round` does it (half to even) and are in 0..=25 and 0..=5.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if saturation < 0.15 {
        return match (lightness * 25.0).round_ties_even() as u8 {
            0 => 16,
            25 => 231,
            gray => 231 + gray,
        };
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let six = |v: u8| {
        let v = f64::from(v);
        (if v < 95.0 { v / 95.0 } else { 1.0 + (v - 95.0) / 40.0 }).round_ties_even() as u8
    };
    16 + 36 * six(c.r) + 6 * six(c.g) + six(c.b)
}

/// Lightness and saturation from Python's `colorsys.rgb_to_hls`, with its exact float operations.
fn lightness_saturation(c: Rgb) -> (f64, f64) {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    let lightness = sum / 2.0;
    #[allow(clippy::float_cmp)] // Exactly Python's `minc == maxc`.
    if min == max {
        return (lightness, 0.0);
    }
    let range = max - min;
    let saturation = if lightness <= 0.5 {
        range / sum
    } else {
        range / (2.0 - max - min)
    };
    (lightness, saturation)
}

/// Rich's `STANDARD_PALETTE`: the RGB it assumes for each of the 16 ANSI colors, in ANSI order.
const STANDARD_PALETTE: [(AnsiColor, Rgb); 16] = [
    (AnsiColor::Black, Rgb::new(0, 0, 0)),
    (AnsiColor::Red, Rgb::new(170, 0, 0)),
    (AnsiColor::Green, Rgb::new(0, 170, 0)),
    (AnsiColor::Yellow, Rgb::new(170, 85, 0)),
    (AnsiColor::Blue, Rgb::new(0, 0, 170)),
    (AnsiColor::Magenta, Rgb::new(170, 0, 170)),
    (AnsiColor::Cyan, Rgb::new(0, 170, 170)),
    (AnsiColor::White, Rgb::new(170, 170, 170)),
    (AnsiColor::BrightBlack, Rgb::new(85, 85, 85)),
    (AnsiColor::BrightRed, Rgb::new(255, 85, 85)),
    (AnsiColor::BrightGreen, Rgb::new(85, 255, 85)),
    (AnsiColor::BrightYellow, Rgb::new(255, 255, 85)),
    (AnsiColor::BrightBlue, Rgb::new(85, 85, 255)),
    (AnsiColor::BrightMagenta, Rgb::new(255, 85, 255)),
    (AnsiColor::BrightCyan, Rgb::new(85, 255, 255)),
    (AnsiColor::BrightWhite, Rgb::new(255, 255, 255)),
];

/// The index of the nearest [`STANDARD_PALETTE`] color: Rich's `Palette.match`, the first of any tie.
fn standard(c: Rgb) -> usize {
    (0..STANDARD_PALETTE.len())
        .min_by_key(|&i| distance_squared(c, STANDARD_PALETTE[i].1))
        .unwrap_or_default()
}

/// The square of Rich's weighted ("redmean") color distance. Rich takes the square root, which keeps the order.
fn distance_squared(a: Rgb, b: Rgb) -> i32 {
    let (r1, g1, b1) = (i32::from(a.r), i32::from(a.g), i32::from(a.b));
    let (r2, g2, b2) = (i32::from(b.r), i32::from(b.g), i32::from(b.b));
    // Python's `(r1 + r2) // 2`; both are non-negative, so rounding toward zero is the same as flooring.
    let red_mean = i32::midpoint(r1, r2);
    let (red, green, blue) = (r1 - r2, g1 - g2, b1 - b2);
    (((512 + red_mean) * red * red) >> 8) + 4 * green * green + (((767 - red_mean) * blue * blue) >> 8)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    fn detect(vars: &[(&str, &str)]) -> ColorDepth {
        ColorDepth::detect_on(&env(vars), false)
    }

    #[rstest]
    #[case(&[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")], ColorDepth::TrueColor)]
    #[case(&[("COLORTERM", "24bit"), ("TERM", "xterm")], ColorDepth::TrueColor)]
    #[case(&[("COLORTERM", " TrueColor ")], ColorDepth::TrueColor)]
    #[case(&[("TERM", "xterm-256color")], ColorDepth::EightBit)]
    #[case(&[("TERM", "xterm-kitty")], ColorDepth::EightBit)]
    #[case(&[("TERM", "kitty")], ColorDepth::EightBit)]
    #[case(&[("TERM", "rxvt-16color")], ColorDepth::Standard)]
    #[case(&[("TERM", "xterm")], ColorDepth::Standard)]
    #[case(&[("TERM", "screen")], ColorDepth::Standard)]
    #[case(&[("COLORTERM", "yes"), ("TERM", "xterm")], ColorDepth::Standard)]
    #[case(&[], ColorDepth::Standard)]
    // Rich says no color; Textual's `Strip.render` falls back to truecolor.
    #[case(&[("TERM", "dumb")], ColorDepth::TrueColor)]
    #[case(&[("TERM", "Unknown"), ("COLORTERM", "yes")], ColorDepth::TrueColor)]
    fn detect_follows_rich_and_textual(#[case] vars: &[(&str, &str)], #[case] want: ColorDepth) {
        assert_eq!(detect(vars), want);
    }

    #[rstest]
    #[case("256", ColorDepth::EightBit)]
    #[case("standard", ColorDepth::Standard)]
    #[case("truecolor", ColorDepth::TrueColor)]
    #[case("auto", ColorDepth::Standard)]
    #[case("bogus", ColorDepth::Standard)]
    fn textual_color_system_overrides_detection(#[case] system: &str, #[case] want: ColorDepth) {
        assert_eq!(detect(&[("TEXTUAL_COLOR_SYSTEM", system), ("TERM", "xterm")]), want);
        // An override wins even over a dumb terminal, as Rich skips detection entirely.
        if system != "bogus" && system != "auto" {
            assert_eq!(detect(&[("TEXTUAL_COLOR_SYSTEM", system), ("TERM", "dumb")]), want);
        }
    }

    #[test]
    fn windows_is_truecolor_unless_overridden() {
        assert_eq!(
            ColorDepth::detect_on(&env(&[("TERM", "xterm")]), true),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect_on(&env(&[("TERM", "xterm-256color")]), true),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect_on(&env(&[("TEXTUAL_COLOR_SYSTEM", "standard")]), true),
            ColorDepth::Standard
        );
    }

    #[test]
    fn no_color_is_set_by_any_value() {
        assert!(ColorMode::detect(&env(&[("NO_COLOR", "")])).no_color);
        assert!(ColorMode::detect(&env(&[("NO_COLOR", "1")])).no_color);
        assert!(!ColorMode::detect(&env(&[])).no_color);
    }

    const PRIMARY: Rgb = Rgb::new(0x01, 0x78, 0xD4);

    fn mode(depth: ColorDepth, no_color: bool) -> ColorMode {
        ColorMode { depth, no_color }
    }

    #[test]
    fn no_color_grays_before_downsampling() {
        assert_eq!(
            downsample(PRIMARY, mode(ColorDepth::TrueColor, true)),
            Color::Rgb(101, 101, 101)
        );
        // Rich: Color.from_rgb(101, 101, 101).downgrade(EIGHT_BIT).number == 241.
        assert_eq!(
            downsample(PRIMARY, mode(ColorDepth::EightBit, true)),
            Color::Indexed(241)
        );
        assert_eq!(
            downsample(Rgb::new(101, 101, 101), mode(ColorDepth::EightBit, false)),
            Color::Indexed(241)
        );
        assert_eq!(
            downsample(PRIMARY, mode(ColorDepth::EightBit, false)),
            Color::Indexed(32)
        );
    }

    #[test]
    fn ansi_paint_passes_through_except_under_no_color() {
        let blue = Paint::Ansi(AnsiColor::Blue);
        for depth in [ColorDepth::TrueColor, ColorDepth::EightBit, ColorDepth::Standard] {
            assert_eq!(mode(depth, false).paint(blue), Color::Blue);
            assert_eq!(mode(depth, true).paint(blue), Color::Reset);
        }
    }

    #[rstest]
    // Grays: black and white are the cube's corners, everything between is the 24-step ramp.
    #[case(Rgb::new(0, 0, 0), 16)]
    #[case(Rgb::new(255, 255, 255), 231)]
    #[case(Rgb::new(0x12, 0x12, 0x12), 233)]
    // 155 is exactly 2.5 cube steps, which Python rounds to 2.
    #[case(Rgb::new(155, 0, 0), 88)]
    #[case(Rgb::new(0xFF, 0xA6, 0x2B), 214)]
    fn eight_bit_matches_rich(#[case] rgb: Rgb, #[case] want: u8) {
        assert_eq!(eight_bit(rgb), want);
    }

    #[rstest]
    #[case(PRIMARY, Color::Cyan)]
    #[case(Rgb::new(0x12, 0x12, 0x12), Color::Black)]
    #[case(Rgb::new(0xE0, 0xE0, 0xE0), Color::White)]
    #[case(Rgb::new(0x4E, 0xBF, 0x71), Color::LightGreen)]
    fn standard_matches_rich(#[case] rgb: Rgb, #[case] want: Color) {
        assert_eq!(downsample(rgb, mode(ColorDepth::Standard, false)), want);
    }
}
