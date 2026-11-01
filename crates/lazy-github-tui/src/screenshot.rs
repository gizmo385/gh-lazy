//! The palette's "Screenshot" command (Textual's system command): the drawn frame saved as an SVG in the Downloads
//! directory.
//!
//! `App` only records that a screenshot was asked for ([`ScreenshotRequest`]). The terminal loop takes the request
//! after the next frame is drawn, which no longer has the palette, and turns that frame's buffer into
//! [`Effect::SaveScreenshot`]. The runtime writes the file ([`save`]) and answers with `Event::ScreenshotSaved`, which
//! `App` shows as a toast ([`saved_toast`], [`failed_toast`]).

use std::fmt;
use std::path::{Path, PathBuf};

use lazy_github_core::screen::{self, Cell, CellStyle, Screen, SvgOptions};
use ratatui::buffer::{Buffer, CellWidth as _};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::components::table::py_repr_str;
use crate::effect::Effect;
use crate::theme::{AnsiColor, Paint, Theme};
use crate::toast::Toast;

/// The toasts' title, as Textual's.
const TOAST_TITLE: &str = "Screenshot";

/// What [`SvgOptions`] uses for the terminal's default colors when the theme leaves them to the terminal (the ANSI
/// themes), which a screenshot can't ask.
const ANSI_DEFAULT_DARK: ((u8, u8, u8), (u8, u8, u8)) = ((0xd4, 0xd4, 0xd4), (0x1e, 0x1e, 0x1e));
const ANSI_DEFAULT_LIGHT: ((u8, u8, u8), (u8, u8, u8)) = ((0x1e, 0x1e, 0x1e), (0xff, 0xff, 0xff));

/// A screenshot asked for, waiting for the next frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotRequest {
    /// The file's name, from the time it was asked for ([`screen::screenshot_file_name`]).
    pub file_name: String,
    /// How to draw it, with the theme's colors as the defaults.
    pub options: SvgOptions,
}

impl ScreenshotRequest {
    /// A request named for local time `now`, drawn with `theme`'s background and foreground as the defaults.
    pub fn new(now: &jiff::civil::DateTime, theme: &Theme) -> Self {
        Self {
            file_name: screen::screenshot_file_name(now),
            options: svg_options(theme),
        }
    }

    /// The effect that saves `frame`, the buffer drawn after the request.
    pub fn into_effect(self, frame: &Buffer) -> Effect {
        Effect::SaveScreenshot {
            file_name: self.file_name,
            screen: Box::new(screen_from_buffer(frame)),
            options: self.options,
        }
    }
}

/// A screenshot that couldn't be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotError {
    /// The file it tried to write.
    pub path: PathBuf,
    /// Why it failed: the I/O error's text.
    pub message: String,
}

impl fmt::Display for ScreenshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// `buffer` as a [`Screen`]. The cells a wide symbol covers after its first are left empty, as ratatui's own diff
/// skips them; the named colors are xterm's 0 to 15 and `Reset` is the terminal's default.
pub fn screen_from_buffer(buffer: &Buffer) -> Screen {
    let area = buffer.area;
    let mut screen = Screen::new(area.width, area.height);
    for y in 0..area.height {
        let mut covered = 0;
        for x in 0..area.width {
            let Some(cell) = buffer.cell((area.x + x, area.y + y)) else {
                continue;
            };
            let Some(out) = screen.cell_mut(x, y) else { continue };
            if covered > 0 {
                covered -= 1;
                *out = Cell {
                    symbol: String::new(),
                    style: out.style,
                };
                continue;
            }
            covered = cell.cell_width().saturating_sub(1);
            *out = Cell {
                symbol: cell.symbol().to_owned(),
                style: cell_style(cell.fg, cell.bg, cell.modifier),
            };
        }
    }
    screen
}

fn cell_style(fg: Color, bg: Color, modifier: Modifier) -> CellStyle {
    CellStyle {
        fg: screen_color(fg),
        bg: screen_color(bg),
        bold: modifier.contains(Modifier::BOLD),
        dim: modifier.contains(Modifier::DIM),
        italic: modifier.contains(Modifier::ITALIC),
        underline: modifier.contains(Modifier::UNDERLINED),
        reversed: modifier.contains(Modifier::REVERSED),
        crossed_out: modifier.contains(Modifier::CROSSED_OUT),
    }
}

fn screen_color(color: Color) -> screen::Color {
    match color {
        Color::Reset => screen::Color::Default,
        Color::Black => screen::Color::Indexed(0),
        Color::Red => screen::Color::Indexed(1),
        Color::Green => screen::Color::Indexed(2),
        Color::Yellow => screen::Color::Indexed(3),
        Color::Blue => screen::Color::Indexed(4),
        Color::Magenta => screen::Color::Indexed(5),
        Color::Cyan => screen::Color::Indexed(6),
        Color::Gray => screen::Color::Indexed(7),
        Color::DarkGray => screen::Color::Indexed(8),
        Color::LightRed => screen::Color::Indexed(9),
        Color::LightGreen => screen::Color::Indexed(10),
        Color::LightYellow => screen::Color::Indexed(11),
        Color::LightBlue => screen::Color::Indexed(12),
        Color::LightMagenta => screen::Color::Indexed(13),
        Color::LightCyan => screen::Color::Indexed(14),
        Color::White => screen::Color::Indexed(15),
        Color::Indexed(index) => screen::Color::Indexed(index),
        Color::Rgb(r, g, b) => screen::Color::Rgb(r, g, b),
    }
}

/// The SVG's title and default colors: the theme's background and its foreground over it.
fn svg_options(theme: &Theme) -> SvgOptions {
    let background = theme.background();
    let foreground = theme.palette.foreground.over(background);
    let defaults = if theme.dark {
        ANSI_DEFAULT_DARK
    } else {
        ANSI_DEFAULT_LIGHT
    };
    SvgOptions {
        title: screen::SCREENSHOT_TITLE.to_owned(),
        default_fg: paint_rgb(foreground, defaults.0),
        default_bg: paint_rgb(background, defaults.1),
    }
}

/// The RGB a paint shows: exact, an ANSI color as xterm draws it, or `default` for the terminal's default color.
fn paint_rgb(paint: Paint, default: (u8, u8, u8)) -> (u8, u8, u8) {
    match paint {
        Paint::Rgb(rgb) => (rgb.r, rgb.g, rgb.b),
        Paint::Ansi(AnsiColor::Default) => default,
        Paint::Ansi(ansi) => match screen_color(Color::from(ansi)) {
            screen::Color::Indexed(index) => screen::xterm_rgb(index),
            screen::Color::Rgb(r, g, b) => (r, g, b),
            screen::Color::Default => default,
        },
    }
}

/// Where screenshots go, from the environment the app started in: the home directory and `XDG_CONFIG_HOME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadsEnv {
    /// The home directory, if it's known.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, when it's set and not blank.
    pub xdg_config_home: Option<PathBuf>,
}

impl DownloadsEnv {
    /// The process's own.
    pub fn from_env() -> Self {
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.to_string_lossy().trim().is_empty())
            .map(PathBuf::from);
        Self {
            home: std::env::home_dir(),
            xdg_config_home,
        }
    }

    /// The Downloads directory ([`screen::downloads_dir`]), reading `user-dirs.dirs`. Blocking. `None` when the home
    /// directory is unknown.
    pub fn downloads_dir(&self) -> Option<PathBuf> {
        let home = self.home.as_deref()?;
        Some(screen::downloads_dir(home, self.xdg_config_home.as_deref(), |path| {
            std::fs::read_to_string(path).ok()
        }))
    }

    /// Resolves the directory and [`save`]s the screenshot there. Blocking.
    pub fn save(&self, file_name: &str, screen: &Screen, options: &SvgOptions) -> Result<PathBuf, ScreenshotError> {
        let Some(dir) = self.downloads_dir() else {
            return Err(ScreenshotError {
                path: PathBuf::from(file_name),
                message: "could not determine your home directory".to_owned(),
            });
        };
        save(&dir, file_name, screen, options)
    }
}

/// Renders `screen` and writes it to `file_name` in `dir`, replacing any file of that name. The directory isn't
/// created: Textual doesn't either. Blocking; the runtime calls it off the async threads.
pub fn save(dir: &Path, file_name: &str, screen: &Screen, options: &SvgOptions) -> Result<PathBuf, ScreenshotError> {
    let path = dir.join(file_name);
    match std::fs::write(&path, screen.to_svg(options)) {
        Ok(()) => Ok(path),
        Err(err) => Err(ScreenshotError {
            path,
            message: err.to_string(),
        }),
    }
}

/// Textual's toast for a saved screenshot: the path as Python's `repr`, in `$text-success`.
pub fn saved_toast(path: &Path, theme: &Theme) -> Toast {
    let palette = theme.palette;
    // The toast's own background, which the color is composited over.
    let panel = palette.panel_lighten_1.over(theme.background());
    // Exact: the toast reduces span colors to the terminal's depth when it draws them.
    let color = match palette.text_success.over(panel) {
        Paint::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        Paint::Ansi(ansi) => Color::from(ansi),
    };
    let quoted = py_repr_str(&path.to_string_lossy());
    Toast::info(Line::from(vec![
        Span::raw("Saved screenshot to "),
        Span::styled(quoted, Style::new().fg(color)),
    ]))
    .with_title(TOAST_TITLE)
}

/// The toast for a screenshot that couldn't be written: the path and why (Py bug B170's answer, where Textual says
/// only "Failed to save screenshot").
pub fn failed_toast(error: &ScreenshotError) -> Toast {
    let quoted = py_repr_str(&error.path.to_string_lossy());
    Toast::error(format!("Failed to save screenshot to {quoted}: {}", error.message)).with_title(TOAST_TITLE)
}

#[cfg(test)]
mod tests;
