//! A markdown document laid out as styled ratatui lines, the way Textual's `Markdown` widget draws it in Python.
//!
//! Two phases. [`Markdown::parse`] reads the source once with `pulldown-cmark` into a small block tree. Then
//! [`Markdown::layout`] lays the tree out at a width in a theme's colors, as [`Rendered`] lines plus the cells every
//! link covers. Layout is pure, so a view keeps the parsed document and caches the layout for its width and theme.
//!
//! Not drawn yet, with the issues that add them: tables as tables (#216, until then a table's rows read as
//! paragraphs), syntax colors in fenced code (#217) and links for bare URLs (#218).

mod layout;
mod parse;
mod wrap;

#[cfg(test)]
mod tests;

use std::ops::Range;
use std::sync::Arc;

use ratatui::text::Line;

use crate::theme::Theme;

pub use wrap::wrap;

/// A parsed markdown document, ready to lay out at any width.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Markdown {
    blocks: Vec<Block>,
}

impl Markdown {
    /// Parses `source` as `markdown-it`'s `gfm-like` preset reads it, which Python uses: `CommonMark` with tables and
    /// `~~` strikethrough. Raw HTML is dropped (inline tags lose their markup but keep their text).
    pub fn parse(source: &str) -> Markdown {
        Markdown {
            blocks: parse::parse(source),
        }
    }

    /// Lays the document out `width` cells wide in `theme`'s colors. A width of 0 gives no lines.
    pub fn layout(&self, width: u16, theme: &Theme) -> Rendered {
        layout::layout(&self.blocks, width, theme)
    }
}

/// A laid-out document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    /// One line per row, top to bottom. A line is at most the layout width, and holds only the cells it styles: what
    /// it doesn't cover shows the background it's drawn on.
    pub lines: Vec<Line<'static>>,
    /// The cells each link's text covers, in reading order. A link that wraps gives one span per row.
    pub links: Vec<LinkSpan>,
}

/// Where a link's text landed: one row's worth of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// The index into [`Rendered::lines`].
    pub row: usize,
    /// The columns the text covers, from the left edge of the layout.
    pub columns: Range<u16>,
    /// Where the link goes: a link's URL or an image's source.
    pub target: Arc<str>,
}

/// A block of the document tree.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// `#` to `######`: `level` is 1 to 6.
    Heading {
        level: u8,
        text: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    /// A bullet list (`start` is `None`) or an ordered list counting from `start`. Each item is its blocks.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    /// Fenced or indented code. `language` is a fence's info string, for the highlighting #217 adds.
    Code {
        language: Option<String>,
        text: String,
    },
    Rule,
    /// A GFM table: its header row and body rows, each a list of cells. #216 lays it out as a table.
    Table {
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// A run of text in one style.
#[derive(Debug, Clone, PartialEq)]
struct Inline {
    /// The text, with whitespace already collapsed. A `\n` is a hard line break.
    text: String,
    style: InlineStyle,
    /// The target when the run is part of a link or an image.
    link: Option<Arc<str>>,
}

/// The inline styles a run has on, from its enclosing markup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent flags, as markdown nests them
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
}
