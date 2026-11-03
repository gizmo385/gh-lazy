//! How many colors the terminal shows, and reducing theme colors to them the way Rich does.
//!
//! Textual hands every color to Rich as 24-bit RGB, and Rich downgrades it to what it detected from the environment:
//! the nearest xterm-256 index, or the nearest of the 16 ANSI colors. Doing the same reduction here means the two
//! apps look alike in a 256-color terminal such as macOS Terminal.app, not just in a truecolor one.
//!
//! Which depth that is goes further than Rich (decision `color-depth-detection`): terminals that show 24-bit color
//! without setting `COLORTERM`, such as Windows Terminal, are detected, and `appearance.color_depth` can name one.

use std::fmt;

use lazy_github_core::config::ColorDepthSetting;
use ratatui::style::Color;

use super::color::{AnsiColor, Paint, Rgb};

/// How many colors the terminal can show: Rich's `ColorSystem`, without its legacy Windows console mode. Rich's
/// "no color" never reaches the screen in Textual (see [`TerminalColors::detect`]), so it has no variant here.
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
    /// The depth to draw with, from the environment `env` reads and the `appearance.color_depth` setting, and why.
    /// [`TerminalColors`] has the rules.
    pub fn detect(env: &impl Fn(&str) -> Option<String>, setting: ColorDepthSetting) -> DetectedDepth {
        TerminalColors::detect(env).depth(setting)
    }

    /// What the debug info calls it: `truecolor`, `256` or `16`.
    pub fn name(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "truecolor",
            ColorDepth::EightBit => "256",
            ColorDepth::Standard => "16",
        }
    }
}

/// Why the app draws with the depth it does: the variable, value or setting that decided it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepthReason {
    /// `appearance.color_depth` named it.
    Setting,
    /// Windows, where the console isn't probed.
    Windows,
    /// This variable is set, whatever its value (`WT_SESSION`).
    Set(&'static str),
    /// This variable has this value (`TERM=xterm-256color`). An empty value is an unset variable.
    Value(&'static str, String),
}

impl fmt::Display for DepthReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DepthReason::Setting => f.write_str("appearance.color_depth"),
            DepthReason::Windows => f.write_str("Windows"),
            DepthReason::Set(name) => f.write_str(name),
            DepthReason::Value(name, value) if value.is_empty() => write!(f, "{name} unset"),
            DepthReason::Value(name, value) => write!(f, "{name}={value}"),
        }
    }
}

/// A depth and why it was chosen. Shown as the debug info's "Color Depth" line: `truecolor (WT_SESSION)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedDepth {
    /// The depth.
    pub depth: ColorDepth,
    /// Why.
    pub reason: DepthReason,
}

impl DetectedDepth {
    fn new(depth: ColorDepth, reason: DepthReason) -> Self {
        Self { depth, reason }
    }
}

impl fmt::Display for DetectedDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.depth.name(), self.reason)
    }
}

/// What the environment says about colors, read once at startup. A Settings change picks the depth again from the
/// same reading ([`TerminalColors::depth`]).
///
/// The depth comes from, in order:
/// 1. `TEXTUAL_COLOR_SYSTEM` (`truecolor`, `256` or `standard`), as in Textual. `auto`, empty or anything else
///    falls through.
/// 2. `appearance.color_depth`, unless it's `auto`.
/// 3. Detection ([`TerminalColors::detect`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalColors {
    /// What `TEXTUAL_COLOR_SYSTEM` named, if anything.
    forced: Option<DetectedDepth>,
    /// What detection found.
    detected: DetectedDepth,
    /// `NO_COLOR` is set, to any value, even an empty one, as Textual checks it.
    no_color: bool,
}

/// `TERM_PROGRAM`s of terminals that show 24-bit color but don't set `COLORTERM`.
const TRUECOLOR_PROGRAMS: &[&str] = &["iTerm.app", "WezTerm", "vscode", "ghostty", "Hyper", "rio"];

/// `TERM`s of terminals that show 24-bit color, lowercase.
const TRUECOLOR_TERMS: &[&str] = &["xterm-kitty", "alacritty", "xterm-ghostty", "wezterm"];

/// The first `VTE_VERSION` with 24-bit color (VTE 0.36).
const VTE_TRUECOLOR: u32 = 3600;

impl TerminalColors {
    /// Reads the environment through `env`.
    ///
    /// Detection, when the setting is `auto`:
    /// - `COLORTERM` of `truecolor` or `24bit` is truecolor, as in Rich.
    /// - `TERM_PROGRAM=Apple_Terminal` is 256, whatever else is set: Terminal.app before macOS 26 can't show 24-bit
    ///   color. It comes after `COLORTERM`, so setting that still asks for truecolor.
    /// - A terminal known to show 24-bit color without saying so in `COLORTERM` is truecolor: `WT_SESSION` set
    ///   (Windows Terminal, WSL included), a `TERM_PROGRAM` in [`TRUECOLOR_PROGRAMS`], `KITTY_WINDOW_ID` set, a
    ///   `TERM` in [`TRUECOLOR_TERMS`], `KONSOLE_VERSION` set, or `VTE_VERSION` of at least 3600.
    /// - Otherwise Rich's `Console._detect_color_system`: the part of `TERM` after its last `-` decides (`256color`
    ///   and `kitty` are 8-bit), and anything else is the 16 standard colors. `TERM` of `dumb` or `unknown` is
    ///   truecolor: Rich detects no color there, but Textual's `Strip.render` (textual 8.2.5 `strip.py` line 718,
    ///   `console._color_system or ColorSystem.TRUECOLOR`) then renders in truecolor. Textual always tells Rich it's
    ///   writing to a terminal, so there is no "not a terminal" case.
    ///
    /// Under tmux or screen the outer terminal's variables usually still arrive (`WT_SESSION`, say; tmux replaces
    /// `TERM_PROGRAM`), and they're used as they are; tmux isn't asked.
    ///
    /// Windows is treated as truecolor rather than probing the console as Rich does; Windows is a stretch goal.
    pub fn detect(env: &impl Fn(&str) -> Option<String>) -> Self {
        Self::detect_on(env, cfg!(windows))
    }

    /// [`TerminalColors::detect`] on this process's environment.
    pub fn from_env() -> Self {
        Self::detect(&|name| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned()))
    }

    /// What `COLORTERM=truecolor` and nothing else gives: truecolor in color, as the parity harness runs.
    pub fn truecolor() -> Self {
        Self::detect(&|name| (name == "COLORTERM").then(|| "truecolor".to_owned()))
    }

    fn detect_on(env: &impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        Self {
            forced: forced(env),
            detected: detected(env, windows),
            no_color: env("NO_COLOR").is_some(),
        }
    }

    /// The depth for `setting`, and why.
    pub fn depth(&self, setting: ColorDepthSetting) -> DetectedDepth {
        let chosen = match setting {
            ColorDepthSetting::Auto => None,
            ColorDepthSetting::TrueColor => Some(ColorDepth::TrueColor),
            ColorDepthSetting::EightBit => Some(ColorDepth::EightBit),
            ColorDepthSetting::Standard => Some(ColorDepth::Standard),
        };
        self.forced
            .clone()
            .or_else(|| chosen.map(|depth| DetectedDepth::new(depth, DepthReason::Setting)))
            .unwrap_or_else(|| self.detected.clone())
    }

    /// The color mode for `setting`: [`TerminalColors::depth`], and `NO_COLOR`.
    pub fn mode(&self, setting: ColorDepthSetting) -> ColorMode {
        ColorMode {
            depth: self.depth(setting).depth,
            no_color: self.no_color,
        }
    }
}

/// What `TEXTUAL_COLOR_SYSTEM` names, if it names a depth.
fn forced(env: &impl Fn(&str) -> Option<String>) -> Option<DetectedDepth> {
    let value = env("TEXTUAL_COLOR_SYSTEM")?;
    let depth = match value.as_str() {
        "truecolor" => ColorDepth::TrueColor,
        "256" => ColorDepth::EightBit,
        "standard" => ColorDepth::Standard,
        "auto" | "" => return None,
        // Rich would raise on an unknown name; detecting is the useful fallback.
        other => {
            tracing::debug!("unknown TEXTUAL_COLOR_SYSTEM {other:?}, detecting the color depth");
            return None;
        }
    };
    Some(DetectedDepth::new(
        depth,
        DepthReason::Value("TEXTUAL_COLOR_SYSTEM", value),
    ))
}

/// The depth detection finds; [`TerminalColors::detect`] has the rules.
fn detected(env: &impl Fn(&str) -> Option<String>, windows: bool) -> DetectedDepth {
    use DepthReason::{Set, Value};

    let set = |name| env(name).is_some_and(|value| !value.is_empty());
    let value = |name: &'static str, value: &str| Value(name, value.to_owned());
    let truecolor = |reason| DetectedDepth::new(ColorDepth::TrueColor, reason);

    let color_term = env("COLORTERM").unwrap_or_default();
    if matches!(color_term.trim().to_lowercase().as_str(), "truecolor" | "24bit") {
        return truecolor(value("COLORTERM", &color_term));
    }
    let program = env("TERM_PROGRAM").unwrap_or_default();
    if program == "Apple_Terminal" {
        return DetectedDepth::new(ColorDepth::EightBit, value("TERM_PROGRAM", &program));
    }
    let raw_term = env("TERM").unwrap_or_default();
    let term = raw_term.trim().to_lowercase();
    if set("WT_SESSION") {
        return truecolor(Set("WT_SESSION"));
    }
    if TRUECOLOR_PROGRAMS.contains(&program.as_str()) {
        return truecolor(value("TERM_PROGRAM", &program));
    }
    if set("KITTY_WINDOW_ID") {
        return truecolor(Set("KITTY_WINDOW_ID"));
    }
    if TRUECOLOR_TERMS.contains(&term.as_str()) {
        return truecolor(value("TERM", &raw_term));
    }
    if set("KONSOLE_VERSION") {
        return truecolor(Set("KONSOLE_VERSION"));
    }
    let vte = env("VTE_VERSION").unwrap_or_default();
    if vte.trim().parse::<u32>().is_ok_and(|version| version >= VTE_TRUECOLOR) {
        return truecolor(value("VTE_VERSION", &vte));
    }
    if windows {
        return truecolor(DepthReason::Windows);
    }
    // Rich detects no color here, which Textual's `Strip.render` turns into truecolor. Rich doesn't strip `TERM`
    // for this check, only for the one below.
    if matches!(raw_term.to_lowercase().as_str(), "dumb" | "unknown") {
        return truecolor(value("TERM", &raw_term));
    }
    // Python's `rpartition("-")`: the whole name when there is no `-`.
    let depth = match term.rsplit('-').next().unwrap_or_default() {
        "256color" | "kitty" => ColorDepth::EightBit,
        _ => ColorDepth::Standard,
    };
    DetectedDepth::new(depth, value("TERM", &raw_term))
}

/// Everything about the terminal that changes how a theme color is shown: [`TerminalColors::mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColorMode {
    /// How many colors the terminal shows.
    pub depth: ColorDepth,
    /// `NO_COLOR` is set: RGB colors become gray and the terminal's own colors become its default.
    pub no_color: bool,
}

impl ColorMode {
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

    /// The depth for `setting` on a non-Windows machine, shown as the debug info shows it.
    fn detect_with(vars: &[(&str, &str)], setting: ColorDepthSetting) -> String {
        TerminalColors::detect_on(&env(vars), false).depth(setting).to_string()
    }

    fn detect(vars: &[(&str, &str)]) -> String {
        detect_with(vars, ColorDepthSetting::Auto)
    }

    /// Rich's rules, and Textual's truecolor for a terminal Rich finds no color in.
    #[rstest]
    #[case(&[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")], "truecolor (COLORTERM=truecolor)")]
    #[case(&[("COLORTERM", "24bit"), ("TERM", "xterm")], "truecolor (COLORTERM=24bit)")]
    #[case(&[("COLORTERM", " TrueColor ")], "truecolor (COLORTERM= TrueColor )")]
    #[case(&[("TERM", "xterm-256color")], "256 (TERM=xterm-256color)")]
    #[case(&[("TERM", "kitty")], "256 (TERM=kitty)")]
    #[case(&[("TERM", "rxvt-16color")], "16 (TERM=rxvt-16color)")]
    #[case(&[("TERM", "xterm")], "16 (TERM=xterm)")]
    #[case(&[("TERM", "screen")], "16 (TERM=screen)")]
    #[case(&[("COLORTERM", "yes"), ("TERM", "xterm")], "16 (TERM=xterm)")]
    #[case(&[], "16 (TERM unset)")]
    // Rich says no color; Textual's `Strip.render` falls back to truecolor.
    #[case(&[("TERM", "dumb")], "truecolor (TERM=dumb)")]
    #[case(&[("TERM", "Unknown"), ("COLORTERM", "yes")], "truecolor (TERM=Unknown)")]
    fn detect_follows_rich_and_textual(#[case] vars: &[(&str, &str)], #[case] want: &str) {
        assert_eq!(detect(vars), want);
    }

    /// Terminals that show 24-bit color without `COLORTERM`, here all with the `TERM` that otherwise means 256.
    #[rstest]
    #[case(&[("WT_SESSION", "0f9a2c1e")], "truecolor (WT_SESSION)")]
    #[case(&[("TERM_PROGRAM", "iTerm.app")], "truecolor (TERM_PROGRAM=iTerm.app)")]
    #[case(&[("TERM_PROGRAM", "WezTerm")], "truecolor (TERM_PROGRAM=WezTerm)")]
    #[case(&[("TERM_PROGRAM", "vscode")], "truecolor (TERM_PROGRAM=vscode)")]
    #[case(&[("TERM_PROGRAM", "ghostty")], "truecolor (TERM_PROGRAM=ghostty)")]
    #[case(&[("TERM_PROGRAM", "Hyper")], "truecolor (TERM_PROGRAM=Hyper)")]
    #[case(&[("TERM_PROGRAM", "rio")], "truecolor (TERM_PROGRAM=rio)")]
    #[case(&[("KITTY_WINDOW_ID", "1")], "truecolor (KITTY_WINDOW_ID)")]
    #[case(&[("KONSOLE_VERSION", "230804")], "truecolor (KONSOLE_VERSION)")]
    #[case(&[("VTE_VERSION", "3600")], "truecolor (VTE_VERSION=3600)")]
    #[case(&[("VTE_VERSION", "7600")], "truecolor (VTE_VERSION=7600)")]
    // Too old, or not a version: Rich's rules.
    #[case(&[("VTE_VERSION", "3405")], "256 (TERM=xterm-256color)")]
    #[case(&[("VTE_VERSION", "new")], "256 (TERM=xterm-256color)")]
    // Set means set to something.
    #[case(&[("WT_SESSION", "")], "256 (TERM=xterm-256color)")]
    // Program names are matched exactly.
    #[case(&[("TERM_PROGRAM", "wezterm")], "256 (TERM=xterm-256color)")]
    // tmux replaces `TERM_PROGRAM`, but the outer terminal's other variables still count.
    #[case(&[("TERM_PROGRAM", "tmux"), ("TMUX", "/tmp/tmux-1000/default,1,0")], "256 (TERM=xterm-256color)")]
    #[case(&[("TERM_PROGRAM", "tmux"), ("WT_SESSION", "x")], "truecolor (WT_SESSION)")]
    fn known_truecolor_terminals(#[case] vars: &[(&str, &str)], #[case] want: &str) {
        let vars = [vars, &[("TERM", "xterm-256color")]].concat();
        assert_eq!(detect(&vars), want);
    }

    #[rstest]
    #[case("xterm-kitty")]
    #[case("alacritty")]
    #[case("xterm-ghostty")]
    #[case("wezterm")]
    #[case("Alacritty")]
    fn known_truecolor_terms(#[case] term: &str) {
        assert_eq!(detect(&[("TERM", term)]), format!("truecolor (TERM={term})"));
    }

    /// Terminal.app is 256 colors whatever else is set, unless `COLORTERM` says otherwise.
    #[rstest]
    #[case(&[("VTE_VERSION", "7600")])]
    #[case(&[("WT_SESSION", "x")])]
    #[case(&[("TERM", "xterm-kitty")])]
    #[case(&[("TERM", "xterm")])]
    fn apple_terminal_is_256_colors(#[case] vars: &[(&str, &str)]) {
        let vars = [vars, &[("TERM_PROGRAM", "Apple_Terminal")]].concat();
        assert_eq!(detect(&vars), "256 (TERM_PROGRAM=Apple_Terminal)");
        let vars = [&vars[..], &[("COLORTERM", "truecolor")]].concat();
        assert_eq!(detect(&vars), "truecolor (COLORTERM=truecolor)");
    }

    #[rstest]
    #[case("256", "256 (TEXTUAL_COLOR_SYSTEM=256)")]
    #[case("standard", "16 (TEXTUAL_COLOR_SYSTEM=standard)")]
    #[case("truecolor", "truecolor (TEXTUAL_COLOR_SYSTEM=truecolor)")]
    #[case("auto", "16 (TERM=xterm)")]
    #[case("", "16 (TERM=xterm)")]
    #[case("bogus", "16 (TERM=xterm)")]
    fn textual_color_system_overrides_detection(#[case] system: &str, #[case] want: &str) {
        assert_eq!(detect(&[("TEXTUAL_COLOR_SYSTEM", system), ("TERM", "xterm")]), want);
        // An override wins even over a dumb terminal, as Rich skips detection entirely.
        if !want.contains("TERM=") {
            assert_eq!(detect(&[("TEXTUAL_COLOR_SYSTEM", system), ("TERM", "dumb")]), want);
        }
    }

    /// `TEXTUAL_COLOR_SYSTEM` over the setting over detection.
    #[rstest]
    #[case(&[("WT_SESSION", "x")], ColorDepthSetting::Auto, "truecolor (WT_SESSION)")]
    #[case(&[("WT_SESSION", "x")], ColorDepthSetting::Standard, "16 (appearance.color_depth)")]
    #[case(&[("WT_SESSION", "x")], ColorDepthSetting::EightBit, "256 (appearance.color_depth)")]
    #[case(&[], ColorDepthSetting::TrueColor, "truecolor (appearance.color_depth)")]
    #[case(&[("COLORTERM", "truecolor")], ColorDepthSetting::Standard, "16 (appearance.color_depth)")]
    #[case(&[("TERM_PROGRAM", "Apple_Terminal")], ColorDepthSetting::TrueColor, "truecolor (appearance.color_depth)")]
    #[case(
        &[("TEXTUAL_COLOR_SYSTEM", "256"), ("WT_SESSION", "x")],
        ColorDepthSetting::TrueColor,
        "256 (TEXTUAL_COLOR_SYSTEM=256)"
    )]
    #[case(
        &[("TEXTUAL_COLOR_SYSTEM", "standard")],
        ColorDepthSetting::Auto,
        "16 (TEXTUAL_COLOR_SYSTEM=standard)"
    )]
    #[case(&[("TEXTUAL_COLOR_SYSTEM", "auto")], ColorDepthSetting::EightBit, "256 (appearance.color_depth)")]
    fn precedence(#[case] vars: &[(&str, &str)], #[case] setting: ColorDepthSetting, #[case] want: &str) {
        let vars = [vars, &[("TERM", "xterm")]].concat();
        assert_eq!(detect_with(&vars, setting), want);
    }

    #[test]
    fn color_depth_detect_is_terminal_colors_depth() {
        let vars = [("TERM", "xterm-256color"), ("VTE_VERSION", "6003")];
        let detected = ColorDepth::detect(&env(&vars), ColorDepthSetting::Auto);
        assert_eq!(detected.depth, ColorDepth::TrueColor);
        assert_eq!(detected.reason, DepthReason::Value("VTE_VERSION", "6003".to_owned()));
        assert_eq!(
            ColorDepth::detect(&env(&vars), ColorDepthSetting::Standard).depth,
            ColorDepth::Standard
        );
    }

    #[test]
    fn windows_is_truecolor_unless_overridden() {
        let detect = |vars, setting| TerminalColors::detect_on(&env(vars), true).depth(setting).to_string();
        assert_eq!(
            detect(&[("TERM", "xterm")], ColorDepthSetting::Auto),
            "truecolor (Windows)"
        );
        assert_eq!(
            detect(&[("TERM", "xterm-256color")], ColorDepthSetting::Auto),
            "truecolor (Windows)"
        );
        assert_eq!(
            detect(&[("WT_SESSION", "x")], ColorDepthSetting::Auto),
            "truecolor (WT_SESSION)"
        );
        assert_eq!(
            detect(&[("TEXTUAL_COLOR_SYSTEM", "standard")], ColorDepthSetting::Auto),
            "16 (TEXTUAL_COLOR_SYSTEM=standard)"
        );
        assert_eq!(detect(&[], ColorDepthSetting::EightBit), "256 (appearance.color_depth)");
    }

    #[test]
    fn no_color_is_set_by_any_value() {
        let no_color = |vars| {
            TerminalColors::detect(&env(vars))
                .mode(ColorDepthSetting::Auto)
                .no_color
        };
        assert!(no_color(&[("NO_COLOR", "")]));
        assert!(no_color(&[("NO_COLOR", "1")]));
        assert!(!no_color(&[]));
        // And whatever the depth.
        let colors = TerminalColors::detect(&env(&[("NO_COLOR", "1"), ("TERM", "xterm-256color")]));
        assert_eq!(
            colors.mode(ColorDepthSetting::TrueColor),
            ColorMode {
                depth: ColorDepth::TrueColor,
                no_color: true
            }
        );
    }

    #[test]
    fn truecolor_is_the_harness_environment() {
        assert_eq!(
            TerminalColors::truecolor().depth(ColorDepthSetting::Auto).to_string(),
            "truecolor (COLORTERM=truecolor)"
        );
        assert_eq!(
            TerminalColors::truecolor().mode(ColorDepthSetting::Auto),
            ColorMode::default()
        );
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
