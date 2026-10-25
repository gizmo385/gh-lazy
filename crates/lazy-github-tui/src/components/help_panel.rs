//! The keybindings panel `app.open_help` toggles: Textual's `HelpPanel` and its `KeyPanel`, docked on the right of
//! the main screen.
//!
//! Textual draws the keys as a Rich table with a `SIMPLE` box: a blank edge row above and below, a blank edge column
//! either side, a right-aligned key column and a wrapping description column, and a blank row between chain links.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::keymap::ActiveBinding;
use crate::theme::{AnsiColor, Paint, Theme, ThemeColor};

/// Rich's `dim`, as Textual resolves it: the text color blended 66% of the way from the background.
const DIM_FACTOR: f64 = 0.66;

/// The panel's left border glyph (Textual's `vkey` border).
const BORDER: &str = "▏";

/// One row of the panel: the bindings of one action in one chain link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    /// Every key of the action, as displayed, in chain order and without duplicates.
    pub keys: Vec<String>,
    /// The first binding's label.
    pub description: &'static str,
    /// The first binding's tooltip, or empty.
    pub tooltip: &'static str,
}

/// The rows of one run of keys from the same chain link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpSection {
    /// The link's heading, drawn above its rows (Textual's `BINDING_GROUP_TITLE`).
    pub title: Option<&'static str>,
    /// The rows, in first-key order.
    pub rows: Vec<HelpRow>,
}

/// The panel's sections for the active keys from [`crate::keymap::active_bindings`], as Textual's `KeyPanel` groups
/// them: each run of keys from one chain link is a section, and its keys are grouped by action. Unlike the footer it
/// lists `show = false` bindings, but never `system` ones.
pub fn help_sections(active: &[ActiveBinding]) -> Vec<HelpSection> {
    active
        .chunk_by(|a, b| a.owner == b.owner)
        .filter_map(|run| {
            let mut rows: Vec<(&str, HelpRow)> = Vec::new();
            for binding in run.iter().filter(|binding| !binding.def().system) {
                let def = binding.def();
                match rows.iter_mut().find(|(action, _)| *action == def.action) {
                    Some((_, row)) if row.keys.contains(&binding.key) => {}
                    Some((_, row)) => row.keys.push(binding.key.clone()),
                    None => rows.push((
                        def.action,
                        HelpRow {
                            keys: vec![binding.key.clone()],
                            description: def.description,
                            tooltip: def.tooltip,
                        },
                    )),
                }
            }
            let title = run[0].owner.group_title();
            (title.is_some() || !rows.is_empty()).then(|| HelpSection {
                title,
                rows: rows.into_iter().map(|(_, row)| row).collect(),
            })
        })
        .collect()
}

/// The panel's width on a screen `screen_width` columns wide: Textual's `width: 33%; min-width: 30; max-width: 60`.
pub fn help_width(screen_width: u16) -> u16 {
    let third = u16::try_from(u32::from(screen_width) * 33 / 100).unwrap_or(u16::MAX);
    third.clamp(30, 60).min(screen_width)
}

/// The styles of the panel's text.
#[derive(Debug, Clone, Copy, Default)]
pub struct HelpStyles {
    /// The keys.
    pub key: Style,
    /// A binding's description.
    pub description: Style,
    /// A binding's tooltip, after its description.
    pub tooltip: Style,
    /// A section heading.
    pub title: Style,
}

impl HelpStyles {
    /// The `KeyPanel` styles for `theme`.
    pub fn new(theme: &Theme) -> Self {
        let palette = theme.palette;
        let bg = theme.background();
        let description = Style::new().fg(theme.color(palette.foreground, bg));
        let tooltip = match palette.foreground {
            // Textual can't blend the terminal's own colors, so the terminal dims them.
            ThemeColor::Ansi(_) => description.add_modifier(Modifier::DIM),
            fg => Style::new().fg(theme.color(fg.multiply_alpha(DIM_FACTOR), bg)),
        };
        Self {
            key: Style::new()
                .fg(theme.color(palette.text_accent, bg))
                .add_modifier(Modifier::BOLD),
            description,
            tooltip,
            title: Style::new()
                .fg(theme.color(palette.text_primary, bg))
                .add_modifier(Modifier::UNDERLINED),
        }
    }
}

/// The panel's content laid out `width` columns wide, one entry per screen row.
///
/// Each row is an edge column, the keys joined by spaces and right-aligned to the widest keys of the whole panel, a
/// space, then the description and the tooltip. Those wrap at word boundaries with a hanging indent under the
/// description, and a word wider than the column is broken. Sections are separated by a blank row, and the content
/// starts and ends with one.
pub fn help_lines(sections: &[HelpSection], width: u16, styles: &HelpStyles) -> Vec<Line<'static>> {
    let keys_width = sections
        .iter()
        .flat_map(|section| &section.rows)
        .map(|row| text_width(&row.keys.join(" ")))
        .max()
        .unwrap_or(0);
    let column = usize::from(width).saturating_sub(keys_width + 3).max(1);
    let indent = " ".repeat(keys_width + 2);
    let mut lines = vec![Line::default()];
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        if let Some(title) = section.title {
            for spans in wrap(&words(title, styles.title), column) {
                lines.push(Line::from([vec![Span::raw(indent.clone())], spans].concat()));
            }
        }
        for row in &section.rows {
            let keys = row.keys.join(" ");
            let pad = " ".repeat(keys_width - text_width(&keys));
            let mut tokens = words(row.description, styles.description);
            tokens.extend(words(row.tooltip, styles.tooltip));
            let mut wrapped = wrap(&tokens, column).into_iter();
            let first = wrapped.next().unwrap_or_default();
            lines.push(Line::from(
                [
                    vec![Span::raw(" "), Span::styled(pad + &keys, styles.key), Span::raw(" ")],
                    first,
                ]
                .concat(),
            ));
            lines.extend(wrapped.map(|spans| Line::from([vec![Span::raw(indent.clone())], spans].concat())));
        }
    }
    lines.push(Line::default());
    lines
}

fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn words(text: &'static str, style: Style) -> Vec<(&'static str, Style)> {
    text.split_whitespace().map(|word| (word, style)).collect()
}

/// Rich's word wrap: words joined by a space in the style of the word before it, a new line when the next word
/// doesn't fit, and a word wider than `width` folded across lines.
fn wrap(words: &[(&'static str, Style)], width: usize) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut line: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let mut previous = Style::new();
    for &(word, style) in words {
        let word_width = text_width(word);
        if used > 0 && used + 1 + word_width <= width {
            line.push(Span::styled(" ", previous));
            used += 1;
        } else if used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if word_width <= width - used {
            line.push(Span::styled(word, style));
            used += word_width;
        } else {
            // Only at the start of a line: fold it into width-wide pieces.
            let mut piece = String::new();
            for c in word.chars() {
                let c_width = text_width(c.encode_utf8(&mut [0; 4]));
                if used + c_width > width && used > 0 {
                    line.push(Span::styled(std::mem::take(&mut piece), style));
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                piece.push(c);
                used += c_width;
            }
            line.push(Span::styled(piece, style));
        }
        previous = style;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// The panel, drawn into the area [`help_width`] gives it. Content past the bottom is cut off; Python scrolls it with
/// the mouse wheel, which isn't built yet. As in Textual, a scrollbar takes the last two content columns when the
/// content overflows, and the text wraps narrower to make room.
pub struct HelpPanel<'a> {
    /// The sections, from [`help_sections`].
    pub sections: &'a [HelpSection],
    /// The theme.
    pub theme: &'a Theme,
}

impl Widget for HelpPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let palette = theme.palette;
        let bg = theme.background();
        let fg = theme.color(palette.foreground, bg);
        Block::new()
            .style(Style::new().fg(fg).bg(theme.paint(bg)))
            .render(area, buf);
        if area.is_empty() {
            return;
        }
        // Textual's `border-left: vkey $foreground 30%`, and `ansi_black` under the ANSI themes.
        let border = if theme.ansi {
            theme.paint(Paint::Ansi(AnsiColor::Black))
        } else {
            theme.color(palette.foreground.multiply_alpha(0.3), bg)
        };
        for y in area.top()..area.bottom() {
            buf[(area.x, y)].set_symbol(BORDER).set_fg(border);
        }
        // The border, then `padding: 0 1`.
        let mut content = Rect {
            x: area.x + 2,
            width: area.width.saturating_sub(3),
            ..area
        };
        if content.is_empty() {
            return;
        }
        let styles = HelpStyles::new(theme);
        let mut lines = help_lines(self.sections, content.width, &styles);
        let overflows = lines.len() > usize::from(content.height);
        if overflows && content.width > scrollbar::VERTICAL_WIDTH {
            content.width -= scrollbar::VERTICAL_WIDTH;
            lines = help_lines(self.sections, content.width, &styles);
            let back = palette.scrollbar_background.over(bg);
            let colors = ScrollbarColors {
                bar: theme.paint(palette.scrollbar.over(back)),
                back: theme.paint(back),
                fg,
                bg: theme.paint(bg),
            };
            let bar = Rect {
                x: content.right(),
                width: scrollbar::VERTICAL_WIDTH,
                ..content
            };
            let state = ScrollbarState {
                content: lines.len(),
                window: usize::from(content.height),
                position: 0,
            };
            scrollbar::render(buf, bar, Axis::Vertical, state, colors);
        }
        Paragraph::new(lines).render(content, buf);
    }
}

#[cfg(test)]
mod tests;
