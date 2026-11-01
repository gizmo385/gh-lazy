//! A terminal screen as plain data, and its renderers: an SVG picture and plain text.
//!
//! [`Screen`] knows nothing about any UI library. The app converts its drawn frame into one for the palette's
//! "Screenshot" command (#291), and the parity harness and visual tests can convert their own grids the same way
//! (#207). This module also holds where a screenshot goes: [`downloads_dir`] and [`screenshot_file_name`].

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

/// A cell's width in the SVG, in pixels. Every run of text is stretched to its cells (`textLength`), so columns line
/// up whatever monospace font the viewer picks.
const CELL_WIDTH: u32 = 10;

/// A cell's height in the SVG, in pixels.
const CELL_HEIGHT: u32 = 20;

/// Where the text's baseline sits below the top of its cell.
const BASELINE: u32 = 15;

/// The font size: most monospace fonts advance 0.6 em per character, so at 10 / 0.6 px their natural width is the
/// cell's, and columns still line up in viewers that ignore `textLength` (librsvg, as of 2.62).
const FONT_SIZE: &str = "16.67";

/// The local monospace fonts the SVG asks for, so it needs no network.
const FONT_FAMILY: &str =
    "ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace";

/// How opaque dim text is drawn: Rich's `dim`, as the app resolves it elsewhere (`Theme::resolve_dim`).
const DIM_OPACITY: &str = "0.66";

/// The prefix of a screenshot's file name: Textual's `App.title`, which is the app class's name in Python.
pub const SCREENSHOT_TITLE: &str = "LazyGithub";

/// A terminal color, as the terminal was sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    /// One of the 256 xterm colors: 0 to 15 are the named ones.
    Indexed(u8),
    /// An exact 24-bit color.
    Rgb(u8, u8, u8),
}

/// How one cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per SGR attribute, as a terminal has them"
)]
pub struct CellStyle {
    /// The text color.
    pub fg: Color,
    /// The background color.
    pub bg: Color,
    /// Bold.
    pub bold: bool,
    /// Dim (faint).
    pub dim: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
    /// Foreground and background swapped.
    pub reversed: bool,
    /// Struck through.
    pub crossed_out: bool,
}

/// One cell of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// What the cell shows: one grapheme. Empty for the cells a wide grapheme covers after its first.
    pub symbol: String,
    /// How it's drawn.
    pub style: CellStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            symbol: " ".to_owned(),
            style: CellStyle::default(),
        }
    }
}

/// A whole terminal screen: `width` by `height` cells, row by row.
#[derive(Clone, PartialEq, Eq)]
pub struct Screen {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl fmt::Debug for Screen {
    // A screen is thousands of cells; its text says what it shows.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Screen")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("text", &self.to_text())
            .finish_non_exhaustive()
    }
}

/// What [`Screen::to_svg`] needs besides the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgOptions {
    /// The SVG's `<title>`.
    pub title: String,
    /// What [`Color::Default`] text is drawn in.
    pub default_fg: (u8, u8, u8),
    /// What a [`Color::Default`] background is drawn in.
    pub default_bg: (u8, u8, u8),
}

impl Screen {
    /// A blank screen: every cell a space in the default colors.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::default(); usize::from(width) * usize::from(height)],
        }
    }

    /// The number of columns.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The number of rows.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The cell at column `x` of row `y`, if it's on the screen.
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// The cell at column `x` of row `y` to change, if it's on the screen.
    pub fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.width && y < self.height).then(|| usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        // `max(1)`: `chunks` panics on 0, and a zero-width screen has no cells to chunk anyway.
        self.cells.chunks(usize::from(self.width).max(1))
    }

    /// The screen as plain text: rows joined by `\n`, trailing spaces trimmed, the cells a wide grapheme covers left
    /// out.
    pub fn to_text(&self) -> String {
        let rows: Vec<String> = self
            .rows()
            .map(|row| {
                let line: String = row.iter().map(|cell| cell.symbol.as_str()).collect();
                line.trim_end_matches(' ').to_owned()
            })
            .collect();
        rows.join("\n")
    }

    /// The screen as an SVG picture: every cell's grapheme, colors, bold, italic, underline, strikethrough and dim
    /// (drawn at 66% opacity), with reversed cells' colors swapped. Each row is one `<rect>` per run of equal
    /// background and one `<text>` per run of equal style, stretched to the run's cells (`textLength`) so columns line
    /// up in any font; a wide or multi-character grapheme is a run of its own. Text the XML can't hold becomes U+FFFD,
    /// so the file always parses.
    pub fn to_svg(&self, options: &SvgOptions) -> String {
        let width = u32::from(self.width) * CELL_WIDTH;
        let height = u32::from(self.height) * CELL_HEIGHT;
        let mut classes = TextClasses::default();
        let mut rects = String::new();
        let mut texts = String::new();
        for (y, row) in (0u32..).zip(self.rows()) {
            let top = y * CELL_HEIGHT;
            let glyphs = glyphs(row, options);
            for run in glyphs.chunk_by(|a, b| a.bg == b.bg) {
                let bg = run[0].bg;
                if bg != options.default_bg {
                    let (x, run_width) = span(run);
                    let _ = writeln!(
                        rects,
                        r#"<rect x="{x}" y="{top}" width="{run_width}" height="{CELL_HEIGHT}" fill="{}"/>"#,
                        hex(bg)
                    );
                }
            }
            for run in glyphs.chunk_by(|a, b| a.text == b.text && a.simple() && b.simple()) {
                let symbols: String = run.iter().map(|g| g.symbol).collect();
                let text = run[0].text;
                if symbols.chars().all(|c| c == ' ') && !text.underline && !text.crossed_out {
                    continue;
                }
                let (x, run_width) = span(run);
                let class = classes.class_of(text);
                let _ = writeln!(
                    texts,
                    r#"<text class="s{class}" x="{x}" y="{}" textLength="{run_width}" lengthAdjust="spacingAndGlyphs">{}</text>"#,
                    top + BASELINE,
                    escape_xml(&symbols)
                );
            }
        }

        let mut svg = String::new();
        let _ = writeln!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" xml:space="preserve">"#
        );
        let _ = writeln!(svg, "<title>{}</title>", escape_xml(&options.title));
        svg.push_str("<style>\n");
        let _ = writeln!(
            svg,
            "text {{ font-family: {FONT_FAMILY}; font-size: {FONT_SIZE}px; white-space: pre; }}"
        );
        for (index, style) in classes.order.iter().enumerate() {
            let _ = writeln!(svg, ".s{index} {{ {} }}", style.css());
        }
        svg.push_str("</style>\n");
        let _ = writeln!(
            svg,
            r#"<rect width="{width}" height="{height}" fill="{}"/>"#,
            hex(options.default_bg)
        );
        if !rects.is_empty() {
            svg.push_str("<g shape-rendering=\"crispEdges\">\n");
            svg.push_str(&rects);
            svg.push_str("</g>\n");
        }
        svg.push_str(&texts);
        svg.push_str("</svg>\n");
        svg
    }
}

type Rgb = (u8, u8, u8);

/// What a run of text is drawn with, its colors resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::struct_excessive_bools, reason = "the CSS flags of one text class")]
struct TextStyle {
    fg: Rgb,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    crossed_out: bool,
}

impl TextStyle {
    fn css(self) -> String {
        let mut css = format!("fill: {};", hex(self.fg));
        if self.bold {
            css.push_str(" font-weight: bold;");
        }
        if self.italic {
            css.push_str(" font-style: italic;");
        }
        match (self.underline, self.crossed_out) {
            (true, true) => css.push_str(" text-decoration: underline line-through;"),
            (true, false) => css.push_str(" text-decoration: underline;"),
            (false, true) => css.push_str(" text-decoration: line-through;"),
            (false, false) => {}
        }
        if self.dim {
            let _ = write!(css, " opacity: {DIM_OPACITY};");
        }
        css
    }
}

/// The text classes in the order they were first used.
#[derive(Default)]
struct TextClasses {
    order: Vec<TextStyle>,
    index: HashMap<TextStyle, usize>,
}

impl TextClasses {
    fn class_of(&mut self, style: TextStyle) -> usize {
        *self.index.entry(style).or_insert_with(|| {
            self.order.push(style);
            self.order.len() - 1
        })
    }
}

/// One grapheme of a row, with the cells it covers and its colors resolved.
struct Glyph<'a> {
    /// Its first cell's column.
    col: u32,
    /// How many cells it covers: more than 1 for a wide grapheme.
    cells: u32,
    symbol: &'a str,
    bg: Rgb,
    text: TextStyle,
}

impl Glyph<'_> {
    /// Whether it's one cell and one character in the Basic Multilingual Plane, which a font draws at the width of the
    /// run's other characters. Anything else (wide graphemes, combining sequences, emoji) is a `<text>` of its own,
    /// so stretching it to its cells can't shift its neighbours.
    fn simple(&self) -> bool {
        let mut chars = self.symbol.chars();
        self.cells == 1 && chars.next().is_some_and(|c| u32::from(c) <= 0xFFFF) && chars.next().is_none()
    }
}

/// The graphemes of `row`. A cell followed by empty ones is a wide grapheme covering them; a stray empty cell (one
/// no wide grapheme comes before) is drawn as a space.
fn glyphs<'a>(row: &'a [Cell], options: &SvgOptions) -> Vec<Glyph<'a>> {
    let mut glyphs = Vec::with_capacity(row.len());
    let mut col = 0;
    while col < row.len() {
        let cell = &row[col];
        let (symbol, cells) = if cell.symbol.is_empty() {
            (" ", 1)
        } else {
            let covered = row[col + 1..].iter().take_while(|c| c.symbol.is_empty()).count();
            (cell.symbol.as_str(), 1 + covered)
        };
        let (bg, text) = resolve(cell.style, options);
        glyphs.push(Glyph {
            // Both fit: a row has at most `u16::MAX` cells.
            col: u32::try_from(col).unwrap_or(u32::MAX),
            cells: u32::try_from(cells).unwrap_or(1),
            symbol,
            bg,
            text,
        });
        col += cells;
    }
    glyphs
}

/// The x position and width, in pixels, of a non-empty run of glyphs.
fn span(run: &[Glyph<'_>]) -> (u32, u32) {
    let cells: u32 = run.iter().map(|g| g.cells).sum();
    (run[0].col * CELL_WIDTH, cells * CELL_WIDTH)
}

/// A cell's background and text style with its colors resolved: defaults from `options`, reversed swapped.
fn resolve(style: CellStyle, options: &SvgOptions) -> (Rgb, TextStyle) {
    let mut fg = color_rgb(style.fg, options.default_fg);
    let mut bg = color_rgb(style.bg, options.default_bg);
    if style.reversed {
        std::mem::swap(&mut fg, &mut bg);
    }
    let text = TextStyle {
        fg,
        bold: style.bold,
        dim: style.dim,
        italic: style.italic,
        underline: style.underline,
        crossed_out: style.crossed_out,
    };
    (bg, text)
}

fn color_rgb(color: Color, default: Rgb) -> Rgb {
    match color {
        Color::Default => default,
        Color::Indexed(index) => xterm_rgb(index),
        Color::Rgb(r, g, b) => (r, g, b),
    }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The RGB of xterm color `index`: xterm's 16 named colors, the 6×6×6 cube and the 24 grays.
pub fn xterm_rgb(index: u8) -> (u8, u8, u8) {
    const BASE: [Rgb; 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x00, 0x00),
        (0x00, 0xcd, 0x00),
        (0xcd, 0xcd, 0x00),
        (0x00, 0x00, 0xee),
        (0xcd, 0x00, 0xcd),
        (0x00, 0xcd, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x7f, 0x7f, 0x7f),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x5c, 0x5c, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => BASE[usize::from(index)],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| LEVELS[usize::from(v)];
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            (v, v, v)
        }
    }
}

/// `text` escaped for XML character data or an attribute value. Characters XML 1.0 doesn't allow (the C0 controls
/// other than tab, line feed and carriage return, and U+FFFE and U+FFFF) become U+FFFD.
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// The Downloads directory as Python's platformdirs finds it (`user_downloads_path`), which is where Textual saves a
/// screenshot.
///
/// On Linux and the BSDs that's `XDG_DOWNLOAD_DIR` from `user-dirs.dirs` in the XDG config directory
/// (`xdg_config_home`, or `<home>/.config` when it's unset), with its quotes stripped and `$HOME` replaced by `home`;
/// with no such file or key, `<home>/Downloads`. `read` returns a file's text, `None` when it can't be read. On macOS
/// and Windows it's `<home>/Downloads`, and nothing is read.
pub fn downloads_dir(
    home: &Path,
    xdg_config_home: Option<&Path>,
    read: impl FnOnce(&Path) -> Option<String>,
) -> PathBuf {
    if cfg!(any(target_os = "macos", windows)) {
        return home.join("Downloads");
    }
    let config_home = xdg_config_home.map_or_else(|| home.join(".config"), Path::to_path_buf);
    read(&config_home.join("user-dirs.dirs"))
        .and_then(|text| user_dirs_entry(&text, "XDG_DOWNLOAD_DIR"))
        .map_or_else(
            || home.join("Downloads"),
            |dir| PathBuf::from(dir.replace("$HOME", &home.to_string_lossy())),
        )
}

/// `key`'s value in a `user-dirs.dirs` file, read as Python's `ConfigParser` reads it: `key=value` or `key: value`
/// lines, keys case-insensitive, `#` and `;` comment lines, the value trimmed and then its double quotes stripped
/// from both ends. An empty value counts as none.
fn user_dirs_entry(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            return None;
        }
        let split = line.find(['=', ':'])?;
        let (name, value) = (line[..split].trim(), line[split + 1..].trim());
        if !name.eq_ignore_ascii_case(key) {
            return None;
        }
        let value = value.trim_matches('"');
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// A screenshot's file name for local time `now`, as Textual makes it (`generate_datetime_filename`):
/// `LazyGithub_2026-11-01T05_20_25_627683.svg`. The time is Python's `isoformat`, which leaves the microseconds out
/// when they're 0, with `:` and `.` replaced by `_`.
pub fn screenshot_file_name(now: &jiff::civil::DateTime) -> String {
    let mut name = format!(
        "{SCREENSHOT_TITLE}_{:04}-{:02}-{:02}T{:02}_{:02}_{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let micros = now.subsec_nanosecond() / 1000;
    if micros != 0 {
        let _ = write!(name, "_{micros:06}");
    }
    name.push_str(".svg");
    name
}

#[cfg(test)]
mod tests;
