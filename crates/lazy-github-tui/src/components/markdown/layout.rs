//! Block tree to styled rows at a width, after Textual's `Markdown` CSS: each block's margins, padding, borders and
//! colors.

use std::iter;
use std::ops::Range;
use std::sync::Arc;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrap::{self, Cell};
use super::{Block, Inline, LinkSpan, Rendered};
use crate::theme::{Paint, Theme, ThemeColor};

/// `Markdown { padding: 0 2 }`.
const PADDING: usize = 2;
/// `Markdown.BULLETS`, by how many bullet-list items enclose the list.
const BULLETS: [&str; 5] = ["• ", "▪ ", "‣ ", "⭑ ", "◦ "];
/// A block quote's `outer` left border.
const QUOTE_BAR: &str = "▌";
/// The border-left column plus `padding: 0 1`.
const QUOTE_INSET: usize = 3;
/// A fence's `& > Label { padding: 1 2 }` columns (none under ANSI themes).
const FENCE_PADDING: usize = 2;
/// Pygments' `tabsize`, which Textual's highlighter passes.
const TAB_SIZE: usize = 8;

pub(super) fn layout(blocks: &[Block], width: u16, theme: &Theme) -> Rendered {
    let width = usize::from(width);
    if width == 0 {
        return Rendered::default();
    }
    // Too narrow for the padding: draw without it rather than in no columns at all.
    let pad = if width > 2 * PADDING { PADDING } else { 0 };
    let painter = Painter { theme };
    let scope = Scope {
        under: theme.background(),
        root: true,
        in_list: false,
        bullets: 0,
    };
    let rows = painter.blocks(blocks, width - 2 * pad, scope);

    let indent = " ".repeat(pad);
    let mut rendered = Rendered::default();
    for (index, row) in rows.into_iter().enumerate() {
        rendered
            .links
            .extend(row.links.into_iter().map(|(columns, target)| LinkSpan {
                row: index,
                columns: column(columns.start + pad)..column(columns.end + pad),
                target,
            }));
        rendered.lines.push(if row.spans.is_empty() {
            Line::default()
        } else {
            Line::from(
                iter::once(Span::raw(indent.clone()))
                    .chain(row.spans)
                    .collect::<Vec<_>>(),
            )
        });
    }
    rendered
}

/// A column as a `u16`. Layout is at most a `u16` wide, except for containers nested past their width, which only
/// lose cells past the edge.
fn column(x: usize) -> u16 {
    u16::try_from(x).unwrap_or(u16::MAX)
}

/// One laid-out row, with the links in it by column from its left edge.
#[derive(Debug, Default)]
struct Row {
    spans: Vec<Span<'static>>,
    links: Vec<(Range<usize>, Arc<str>)>,
}

impl Row {
    fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }

    /// `self` after `prefix`, with its links moved over.
    fn after(mut self, prefix: Vec<Span<'static>>) -> Row {
        let shift: usize = prefix.iter().map(Span::width).sum();
        for (columns, _) in &mut self.links {
            *columns = columns.start + shift..columns.end + shift;
        }
        self.spans.splice(0..0, prefix);
        self
    }
}

/// Where a block sits, which decides its margins and colors.
#[derive(Debug, Clone, Copy)]
struct Scope {
    /// The background it's drawn on.
    under: Paint,
    /// Directly inside the document: only those paragraphs get a margin (`Markdown > MarkdownParagraph`).
    root: bool,
    /// Inside a list: nested lists get no margin (`MarkdownList MarkdownList`).
    in_list: bool,
    /// How many bullet-list items enclose it, which picks the bullet glyph.
    bullets: usize,
}

impl Scope {
    fn inside(self, under: Paint) -> Scope {
        Scope {
            under,
            root: false,
            ..self
        }
    }
}

struct Painter<'t> {
    theme: &'t Theme,
}

impl Painter<'_> {
    /// Blocks stacked top to bottom, with their vertical margins between them; adjacent margins collapse to the
    /// larger, and the first and last keep theirs.
    fn blocks(&self, blocks: &[Block], width: usize, scope: Scope) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut margin: Option<usize> = None;
        let mut stack = |rows: &mut Vec<Row>, (top, bottom): (usize, usize), block_rows: Vec<Row>| {
            let gap = margin.map_or(top, |previous| previous.max(top));
            rows.extend(iter::repeat_with(Row::default).take(gap));
            rows.extend(block_rows);
            margin = Some(bottom);
        };
        for block in blocks {
            if let Block::Table { head, rows: body } = block {
                // Until #216: each table row reads as a paragraph of its cells' text.
                for cells in iter::once(head).chain(body) {
                    let text = table_row_text(cells);
                    let block_rows = self.inlines(&text, width, self.text_style(scope.under), scope.under);
                    stack(&mut rows, paragraph_margins(scope), block_rows);
                }
                continue;
            }
            let block_rows = self.block(block, width, scope);
            stack(&mut rows, self.margins(block, scope), block_rows);
        }
        rows.extend(iter::repeat_with(Row::default).take(margin.unwrap_or(0)));
        rows
    }

    /// A block's (top, bottom) margins, from Textual's CSS.
    #[allow(clippy::match_same_arms)] // an arm per CSS rule
    fn margins(&self, block: &Block, scope: Scope) -> (usize, usize) {
        match block {
            // `MarkdownHeader { margin: 2 0 1 0 }`, and `margin: 1 0` from H3 down.
            Block::Heading { level: 1 | 2, .. } => (2, 1),
            Block::Heading { .. } => (1, 1),
            Block::Paragraph(_) | Block::Table { .. } => paragraph_margins(scope),
            // `MarkdownBulletList`/`MarkdownOrderedList { margin: 0 0 1 0 }`, but `MarkdownList MarkdownList`
            // has none.
            Block::List { .. } if scope.in_list => (0, 0),
            Block::List { .. } => (0, 1),
            Block::Quote(_) => (1, 1),
            // `MarkdownFence { margin: 1 0 }`; `:ansi` has none.
            Block::Code { .. } if self.theme.ansi => (0, 0),
            Block::Code { .. } => (1, 1),
            // `MarkdownHorizontalRule { margin-bottom: 1 }`; its `padding-top` is a row of the block.
            Block::Rule => (0, 1),
        }
    }

    fn block(&self, block: &Block, width: usize, scope: Scope) -> Vec<Row> {
        match block {
            Block::Heading { level, text } => self.heading(*level, text, width, scope.under),
            Block::Paragraph(text) => self.inlines(text, width, self.text_style(scope.under), scope.under),
            Block::List { start, items } => self.list(*start, items, width, scope),
            Block::Quote(blocks) => self.quote(blocks, width, scope),
            Block::Code { text, .. } => self.code(text, width, scope.under),
            Block::Rule => vec![
                Row::default(),
                Row {
                    spans: vec![Span::styled(
                        "─".repeat(width),
                        self.fg(self.palette().secondary, scope.under),
                    )],
                    links: Vec::new(),
                },
            ],
            // Laid out by `blocks`.
            Block::Table { .. } => Vec::new(),
        }
    }

    fn heading(&self, level: u8, text: &[Inline], width: usize, under: Paint) -> Vec<Row> {
        let p = self.palette();
        let (color, text_style) = match level {
            1 => (p.markdown_h1_color, p.markdown_h1_text_style),
            2 => (p.markdown_h2_color, p.markdown_h2_text_style),
            3 => (p.markdown_h3_color, p.markdown_h3_text_style),
            4 => (p.markdown_h4_color, p.markdown_h4_text_style),
            5 => (p.markdown_h5_color, p.markdown_h5_text_style),
            _ => (p.markdown_h6_color, p.markdown_h6_text_style),
        };
        // The `$markdown-hN-background`s are transparent in every built-in theme.
        let style = text_style.apply(self.fg(color, under));
        let rows = self.inlines(text, width, style, under);
        if level != 1 {
            return rows;
        }
        // `MarkdownH1 { content-align: center middle }`: the whole row is the header, its spaces styled too.
        rows.into_iter()
            .map(|row| {
                let excess = width.saturating_sub(row.width());
                let left = excess / 2;
                let mut row = row.after(vec![Span::styled(" ".repeat(left), style)]);
                row.spans.push(Span::styled(" ".repeat(excess - left), style));
                row
            })
            .collect()
    }

    fn list(&self, start: Option<u64>, items: &[Vec<Block>], width: usize, scope: Scope) -> Vec<Row> {
        let bullets: Vec<String> = match start {
            None => {
                let bullet = BULLETS[scope.bullets % BULLETS.len()];
                vec![bullet.to_owned(); items.len()]
            }
            // `MarkdownOrderedList`: "N. " right-aligned one column past the widest.
            Some(start) => {
                let numbers: Vec<String> = (0_u64..)
                    .take(items.len())
                    .map(|i| format!("{}. ", start.saturating_add(i)))
                    .collect();
                let widest = numbers.iter().map(String::len).max().unwrap_or(0);
                numbers.into_iter().map(|n| format!("{n:>0$}", widest + 1)).collect()
            }
        };
        let bullet_style = self.fg(
            if self.theme.dark {
                self.palette().text_primary
            } else {
                self.palette().text_secondary
            },
            scope.under,
        );
        let inner = Scope {
            in_list: true,
            bullets: scope.bullets + usize::from(start.is_none()),
            ..scope.inside(scope.under)
        };
        let mut rows = Vec::new();
        for (bullet, blocks) in bullets.into_iter().zip(items) {
            let bullet_width = Span::raw(bullet.as_str()).width();
            // An empty item has no rows, so its bullet doesn't show either, as in Textual.
            let item_rows = self.blocks(blocks, width.saturating_sub(bullet_width).max(1), inner);
            let mut bullet = Some(Span::styled(bullet, bullet_style));
            rows.extend(item_rows.into_iter().map(|row| {
                let prefix = bullet.take().unwrap_or_else(|| Span::raw(" ".repeat(bullet_width)));
                row.after(vec![prefix])
            }));
        }
        rows
    }

    fn quote(&self, blocks: &[Block], width: usize, scope: Scope) -> Vec<Row> {
        let p = self.palette();
        let background = p.boost.over(scope.under);
        let bg = Style::new().bg(self.theme.paint(background));
        // `border-left: outer $text-primary 50%`, and `$text-secondary` on light themes.
        let bar_color = if self.theme.dark {
            p.text_primary.multiply_alpha(0.5)
        } else {
            p.text_secondary
        };
        let bar = bg.fg(self.theme.color(bar_color, scope.under));
        let inner_width = width.saturating_sub(QUOTE_INSET).max(1);
        let mut rows = self.blocks(blocks, inner_width, scope.inside(background));
        if rows.is_empty() {
            // A quote with nothing in it is still a line, as its empty `Static` is.
            rows.push(Row::default());
        }
        rows.into_iter()
            .map(|row| {
                let fill = inner_width.saturating_sub(row.width()) + 1;
                let mut row = row.after(vec![Span::styled(QUOTE_BAR, bar), Span::styled(" ", bg)]);
                for span in &mut row.spans {
                    span.style = bg.patch(span.style);
                }
                row.spans.push(Span::styled(" ".repeat(fill), bg));
                row
            })
            .collect()
    }

    /// A fenced or indented code block: never wrapped, long lines cut at the edge. #217 colors its tokens.
    fn code(&self, text: &str, width: usize, under: Paint) -> Vec<Row> {
        let p = self.palette();
        let ansi = self.theme.ansi;
        let background = self.theme.markdown_fence_background().over(under);
        let bg = if ansi {
            Style::new()
        } else {
            Style::new().bg(self.theme.paint(background))
        };
        // The highlighter puts `$text` under every token, and Textual resolves that `auto` color at full strength.
        let text_color = match p.text {
            ThemeColor::Auto { .. } => ThemeColor::Auto { alpha: 1.0 },
            other => other,
        };
        let style = bg.fg(self.theme.color(text_color, background));
        let padding = if ansi { 0 } else { FENCE_PADDING };
        let blank = || {
            if ansi {
                Row::default()
            } else {
                Row {
                    spans: vec![Span::styled(" ".repeat(width), bg)],
                    links: Vec::new(),
                }
            }
        };

        let mut rows = vec![blank()];
        for line in code_lines(text) {
            let visible = cut(&line, width.saturating_sub(padding));
            let used = padding + Span::raw(visible.as_str()).width();
            let mut spans = Vec::new();
            if padding > 0 {
                spans.push(Span::styled(" ".repeat(padding), bg));
            }
            spans.push(Span::styled(visible, style));
            if !ansi && used < width {
                spans.push(Span::styled(" ".repeat(width - used), bg));
            }
            rows.push(Row {
                spans,
                links: Vec::new(),
            });
        }
        rows.push(blank());
        rows
    }

    /// Inline runs wrapped to `width`, over `base`: a `\n` (a hard break) starts a row, and each line takes at
    /// least one.
    fn inlines(&self, text: &[Inline], width: usize, base: Style, under: Paint) -> Vec<Row> {
        let styles: Vec<Style> = text.iter().map(|run| self.inline_style(base, under, run)).collect();
        let mut rows = Vec::new();
        let mut line: Vec<Cell<'_>> = Vec::new();
        let mut finish = |line: &mut Vec<Cell<'_>>| {
            let cells = std::mem::take(line);
            let ranges = wrap::rows(&cells, width);
            if ranges.is_empty() {
                rows.push(Row::default());
            }
            for range in ranges {
                rows.push(row_of(&cells[range], text, &styles));
            }
        };
        for (index, run) in text.iter().enumerate() {
            let mut pieces = run.text.split('\n');
            if let Some(first) = pieces.next() {
                line.extend(wrap::cells(first, index));
            }
            for piece in pieces {
                finish(&mut line);
                line.extend(wrap::cells(piece, index));
            }
        }
        finish(&mut line);
        rows
    }

    fn inline_style(&self, base: Style, under: Paint, run: &Inline) -> Style {
        let p = self.palette();
        let mut style = base;
        if run.style.strong {
            style = style.add_modifier(Modifier::BOLD);
        }
        if run.style.emphasis {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if run.style.strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if run.style.code {
            // `&:dark > .code_inline` and `&:light > .code_inline`; `:ansi` keeps the terminal's background.
            let (color, background) = if self.theme.dark {
                (p.text_warning, p.warning.multiply_alpha(0.1))
            } else {
                (p.text_error, p.error.multiply_alpha(0.05))
            };
            let mut behind = under;
            if !self.theme.ansi {
                behind = background.component_background().over(under);
                style = style.bg(self.theme.paint(behind));
            }
            style = style.fg(self.theme.color(color.multiply_alpha(0.95), behind));
        }
        if run.link.is_some() {
            // Over the block's background even on inline code, as Textual resolves the link color there.
            style = p.link_style.apply(style.fg(self.theme.color(p.link_color, under)));
        }
        style
    }

    fn text_style(&self, under: Paint) -> Style {
        self.fg(self.palette().foreground, under)
    }

    fn fg(&self, color: ThemeColor, under: Paint) -> Style {
        Style::new().fg(self.theme.color(color, under))
    }

    fn palette(&self) -> &'static crate::theme::Palette {
        self.theme.palette
    }
}

fn paragraph_margins(scope: Scope) -> (usize, usize) {
    if scope.root { (0, 1) } else { (0, 0) }
}

/// One wrapped row's cells as spans, one per run they come from, and the columns of its links.
fn row_of(cells: &[Cell<'_>], runs: &[Inline], styles: &[Style]) -> Row {
    let mut row = Row::default();
    let mut column = 0;
    let mut i = 0;
    while i < cells.len() {
        let run = cells[i].run;
        let mut text = String::new();
        let start = column;
        while i < cells.len() && cells[i].run == run {
            text.push_str(cells[i].symbol);
            column += cells[i].width;
            i += 1;
        }
        if let Some(target) = &runs[run].link {
            // A link's runs are contiguous, so a run that continues the last link's columns extends it.
            match row.links.last_mut() {
                Some((columns, last)) if Arc::ptr_eq(last, target) && columns.end == start => {
                    columns.end = column;
                }
                _ => row.links.push((start..column, Arc::clone(target))),
            }
        }
        row.spans.push(Span::styled(text, styles[run]));
    }
    row
}

/// A table row's cells as one run list, a space between cells.
fn table_row_text(cells: &[Vec<Inline>]) -> Vec<Inline> {
    let mut text = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            text.push(Inline {
                text: " ".to_owned(),
                style: super::InlineStyle::default(),
                link: None,
            });
        }
        text.extend(cell.iter().cloned());
    }
    text
}

/// A code block's lines as Textual shows them: trailing whitespace (blank lines included) stripped from the end, tabs
/// expanded, and at least one line.
fn code_lines(text: &str) -> Vec<String> {
    let text = text.trim_end();
    if text.is_empty() {
        return vec![String::new()];
    }
    text.lines().map(expand_tabs).collect()
}

/// `str.expandtabs`: each tab to the next multiple of [`TAB_SIZE`] characters.
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut column = 0;
    for c in line.chars() {
        if c == '\t' {
            let spaces = TAB_SIZE - column % TAB_SIZE;
            out.extend(iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}

/// The graphemes of `line` that fit in `width` cells.
fn cut(line: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for cell in wrap::cells(line, 0) {
        if used + cell.width > width {
            break;
        }
        used += cell.width;
        out.push_str(cell.symbol);
    }
    out
}
