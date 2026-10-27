//! Drawing the editor as Textual's `TextArea` draws it: the border, `padding: 0 1`, the gutter, the wrapped rows, the
//! cursor's line and the cursor, and the vertical scrollbar inside the padding.
//!
//! The code editor's colors are Textual's monokai `TextAreaTheme`, which ignores the app theme; the plain editor's come
//! from the theme as `TextArea.DEFAULT_CSS` and the `css` text area theme pick them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::wrap::{TAB_WIDTH, char_cells};
use super::{EditorBorder, EditorVariant, PADDING, TextEditor};
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState, VERTICAL_WIDTH};
use crate::components::text_input::TallBorder;
use crate::theme::{Paint, Rgb, Theme};

/// Textual's `_MONOKAI` text area theme.
mod monokai {
    use crate::theme::Rgb;

    pub(super) const BACKGROUND: Rgb = Rgb::new(0x27, 0x28, 0x22);
    pub(super) const TEXT: Rgb = Rgb::new(0xf8, 0xf8, 0xf2);
    pub(super) const GUTTER: Rgb = Rgb::new(0x90, 0x90, 0x8a);
    pub(super) const CURSOR: Rgb = Rgb::new(0xf8, 0xf8, 0xf0);
    pub(super) const CURSOR_LINE: Rgb = Rgb::new(0x3e, 0x3d, 0x32);
    pub(super) const CURSOR_LINE_GUTTER: Rgb = Rgb::new(0xc2, 0xc2, 0xbf);
}

/// The styles of one frame.
struct Styles {
    /// The text and the empty rows.
    base: Style,
    /// The padding: the background alone.
    padding: Style,
    /// The text on the cursor's line, every wrapped row of it.
    cursor_line: Style,
    gutter: Style,
    cursor_line_gutter: Style,
    cursor: Style,
    border: TallBorder,
    scrollbar: ScrollbarColors,
}

impl Styles {
    fn new(editor: &TextEditor, theme: &Theme, under: Paint, focused: bool) -> Self {
        let p = theme.palette;
        let rgb = |c: Rgb| theme.paint(Paint::Rgb(c));
        let (bg, base, cursor_line, gutter, cursor_line_gutter, cursor) = match editor.variant {
            EditorVariant::Code => {
                let bg = Paint::Rgb(monokai::BACKGROUND);
                let base = Style::new().fg(rgb(monokai::TEXT)).bg(theme.paint(bg));
                let line_bg = rgb(monokai::CURSOR_LINE);
                (
                    bg,
                    base,
                    base.bg(line_bg),
                    base.fg(rgb(monokai::GUTTER)),
                    Style::new().fg(rgb(monokai::CURSOR_LINE_GUTTER)).bg(line_bg),
                    Style::new().fg(rgb(monokai::BACKGROUND)).bg(rgb(monokai::CURSOR)),
                )
            }
            EditorVariant::Plain => {
                let bg = p.surface.over(under);
                let base = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
                let cursor_bg = p.input_cursor_background.over(bg);
                let cursor = if theme.ansi {
                    // `&:ansi .text-area--cursor`.
                    Style::new()
                        .fg(theme.color(p.input_cursor_foreground, cursor_bg))
                        .bg(theme.paint(cursor_bg))
                        .add_modifier(Modifier::REVERSED)
                } else if theme.dark {
                    p.input_cursor_text_style.apply(
                        Style::new()
                            .fg(theme.color(p.input_cursor_foreground, cursor_bg))
                            .bg(theme.paint(cursor_bg)),
                    )
                } else {
                    // `&:light .text-area--cursor`: `$text 90%` on `$foreground 70%`.
                    let light_bg = p.foreground.multiply_alpha(0.7).over(bg);
                    p.input_cursor_text_style.apply(
                        Style::new()
                            .fg(theme.color(p.text.multiply_alpha(0.9), light_bg))
                            .bg(theme.paint(light_bg)),
                    )
                };
                (bg, base, base.bg(theme.color(p.boost, bg)), base, base, cursor)
            }
        };
        let border = theme.color(if focused { p.border } else { p.border_blurred }, bg);
        let back = p.scrollbar_background.over(bg);
        Self {
            base,
            padding: Style::new().fg(Color::Reset).bg(theme.paint(bg)),
            cursor_line,
            gutter,
            cursor_line_gutter,
            cursor,
            border: TallBorder {
                left: Style::new()
                    .fg(border)
                    .bg(theme.paint(under))
                    .add_modifier(Modifier::REVERSED),
                inner: Style::new().fg(border).bg(theme.paint(bg)),
                right: Style::new().fg(border).bg(theme.paint(under)),
                fill: base,
            },
            scrollbar: ScrollbarColors {
                bar: theme.paint(p.scrollbar.over(back)),
                back: theme.paint(back),
                fg: base.fg.unwrap_or(Color::Reset),
                bg: base.bg.unwrap_or(Color::Reset),
            },
        }
    }
}

/// Draws `editor` into its last laid-out area.
pub(super) fn draw(editor: &TextEditor, buf: &mut Buffer, theme: &Theme, under: Paint, focused: bool) {
    let area = editor.view.area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let styles = Styles::new(editor, theme, under, focused);
    match editor.border {
        EditorBorder::Tall => styles.border.draw(buf, area),
        EditorBorder::Blank => buf.set_style(area, styles.base),
    }
    if area.width <= 2 * (1 + PADDING) || area.height <= 2 {
        return;
    }
    let scrollbar_width = if editor.view.scrollbar { VERTICAL_WIDTH } else { 0 };
    let inner_width = area.width - 2 * (1 + PADDING);
    let text_area = Rect {
        x: area.x + 1 + PADDING,
        y: area.y + 1,
        width: inner_width.saturating_sub(scrollbar_width),
        height: area.height - 2,
    };
    for y in text_area.top()..text_area.bottom() {
        buf[(area.x + 1, y)].set_symbol(" ").set_style(styles.padding);
        buf[(area.right() - 2, y)].set_symbol(" ").set_style(styles.padding);
    }

    let lines = editor.lines();
    let cursor = editor.cursor();
    let gutter_width = editor.gutter_width();
    let digits = gutter_width.saturating_sub(super::GUTTER_MARGIN);
    for (i, y) in (text_area.top()..text_area.bottom()).enumerate() {
        let row = Rect {
            y,
            height: 1,
            ..text_area
        };
        let Some((line, section)) = editor.layout.row(editor.scroll_y + i) else {
            fill(buf, row, styles.base);
            continue;
        };
        let on_cursor_line = line == cursor.0;
        let text_style = if on_cursor_line {
            styles.cursor_line
        } else {
            styles.base
        };
        fill(buf, row, text_style);
        let mut x = row.x;
        if gutter_width > 0 {
            let gutter_style = if on_cursor_line {
                styles.cursor_line_gutter
            } else {
                styles.gutter
            };
            let number = if section == 0 {
                format!("{:>digits$}", line + 1)
            } else {
                String::new()
            };
            let label = format!("{number:<gutter_width$}");
            buf.set_stringn(x, y, &label, usize::from(row.width), gutter_style);
            x = x.saturating_add(u16::try_from(gutter_width).unwrap_or(u16::MAX));
        }
        let text = editor.layout.sections(lines, line)[section];
        draw_section(buf, x, row, text, text_style);
    }

    if focused {
        let (cx, cy) = editor.layout.location_to_offset(lines, cursor);
        if let Some(screen_row) = cy
            .checked_sub(editor.scroll_y)
            .filter(|&r| r < usize::from(text_area.height))
        {
            let x = usize::from(text_area.x) + gutter_width + cx;
            if x < usize::from(text_area.right()) {
                // Both fit in u16: they're inside `text_area`.
                let (x, y) = (
                    u16::try_from(x).unwrap_or(u16::MAX),
                    text_area.y + u16::try_from(screen_row).unwrap_or(u16::MAX),
                );
                buf[(x, y)].set_style(styles.cursor);
            }
        }
    }

    if editor.view.scrollbar {
        let bar = Rect {
            x: text_area.right(),
            width: VERTICAL_WIDTH,
            ..text_area
        };
        let state = ScrollbarState {
            content: editor.layout.height(),
            window: usize::from(text_area.height),
            position: editor.scroll_y,
        };
        scrollbar::render(buf, bar, Axis::Vertical, state, styles.scrollbar);
    }
}

fn fill(buf: &mut Buffer, row: Rect, style: Style) {
    for x in row.left()..row.right() {
        buf[(x, row.y)].set_symbol(" ").set_style(style);
    }
}

/// One wrapped row's text from `x`, tabs expanded within the row, cut off at `row`'s right edge.
fn draw_section(buf: &mut Buffer, mut x: u16, row: Rect, text: &str, style: Style) {
    let mut cells = 0;
    for c in text.chars() {
        let (symbol, width) = if c == '\t' {
            (" ".repeat(TAB_WIDTH - cells % TAB_WIDTH), TAB_WIDTH - cells % TAB_WIDTH)
        } else {
            (c.to_string(), char_cells(c))
        };
        cells += width;
        let room = usize::from(row.right().saturating_sub(x));
        if room == 0 {
            return;
        }
        if width > 0 {
            buf.set_stringn(x, row.y, &symbol, room, style);
        }
        x = x.saturating_add(u16::try_from(width).unwrap_or(u16::MAX));
    }
}
