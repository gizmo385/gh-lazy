//! One comment of a conversation as a block: Python's `IssueCommentContainer`. The PR Conversation tab and the issue
//! Comments tab stack these in one scroll view.
//!
//! Laid out, a block is one cell of margin, a left border `│` in `$secondary-background`, then (one more cell in) the
//! body as markdown with its references expanded, the author line `<login> • <%c>` in `$text-muted` and a blank row,
//! then the reactions section when the comment has any, and a blank row of margin below the border. With focus on
//! it or inside it, the border becomes a dashed box all round in `$success`.
//!
//! The block doesn't own focus: the view that stacks the blocks runs their focus ring over each block's
//! [`CommentBlock::stops`], and says which stop is focused when it lays a block out.

use std::ops::Range;

use jiff::Timestamp;
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{IssueComment, Reactions, User};
use lazy_github_core::{references, time};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::collapsible::{CONTENT_INDENT, Collapsible, Placement, SectionFocus};
use super::list_view::{ListView, append, pad_line};
use super::markdown::{self, Markdown, Rendered, TextRun};
use super::reactions::ReactionsSummary;
use super::scroll_view::LayoutCache;
use crate::component::{ChainLink, Owner};
use crate::keymap::BindingId;
use crate::theme::{Paint, Theme};

/// A focused block's bindings: `IssueCommentContainer.BINDINGS`. Reply is a no-op until #182.
pub const COMMENT_BINDINGS: &[BindingId] = &[BindingId::ConversationCommentReply];

/// A focused reactions title's bindings: Textual's `CollapsibleTitle`.
const TITLE_BINDINGS: &[BindingId] = &[BindingId::TextualCollapsibleToggle];

/// A focused reactions list's bindings: Textual's `ListView`.
const LIST_BINDINGS: &[BindingId] = &[
    BindingId::TextualListSelect,
    BindingId::TextualListCursorUp,
    BindingId::TextualListCursorDown,
];

/// The block's `margin-left: 1`.
const MARGIN: u16 = 1;
/// Where the reactions section starts: after the margin and the left border.
const SECTION_LEFT: u16 = 2;
/// Where the body and the author line start: the margin, the border, and their own `margin-left: 1`. The body's
/// markdown has no padding here, so its text runs from this column to the block's right edge.
const TEXT_LEFT: u16 = 3;

/// A focus stop inside a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStop {
    /// The block itself.
    Block,
    /// Its reactions section's title.
    ReactionsTitle,
    /// Its open reactions section's list.
    ReactionsList,
}

/// What a click at a cell of a laid-out block lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentHit {
    /// The block, outside its reactions section's title and list.
    Block,
    /// The reactions title: a click focuses and toggles it.
    ReactionsTitle,
    /// Reactions list item `n`: a click focuses the list and highlights it.
    ReactionsItem(usize),
}

/// One comment, ready to lay out: its parsed body, its author line and its reactions once they're known.
#[derive(Debug)]
pub struct CommentBlock {
    id: u64,
    body: Markdown,
    author: String,
    reactions: Option<ReactionsSection>,
    /// The body's layouts, by markdown width: a focused block's is one cell narrower.
    body_layouts: LayoutCache,
}

/// A comment's reactions section: the summary it shows, whether it's open, and its list's highlight.
#[derive(Debug)]
struct ReactionsSection {
    summary: ReactionsSummary,
    collapsible: Collapsible,
    list: ListView,
}

/// A block laid out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommentLayout {
    /// Its rows, each as wide as the block, its links by column from the block's left edge.
    pub rendered: Rendered,
    /// Where its reactions section landed, when it shows one: rows into [`Self::rendered`], and the title's columns
    /// from the block's left edge.
    pub reactions: Option<Placement>,
    /// The rows of each reactions list item, into [`Self::rendered`]: empty while the section is closed.
    pub reaction_items: Vec<Range<usize>>,
}

impl CommentLayout {
    /// What a cell of the block hits, by row into [`Self::rendered`] and column from the block's left edge. `None`
    /// off the block: past its rows, or on the margin row below it.
    pub fn hit(&self, row: usize, column: u16) -> Option<CommentHit> {
        // The last row is the margin below the border, which isn't part of the block.
        if row + 1 >= self.rendered.lines.len() {
            return None;
        }
        if let Some(placement) = &self.reactions {
            if row == placement.title_row && placement.title_columns.contains(&column) {
                return Some(CommentHit::ReactionsTitle);
            }
            if column >= SECTION_LEFT + CONTENT_INDENT
                && let Some(item) = self.reaction_items.iter().position(|rows| rows.contains(&row))
            {
                return Some(CommentHit::ReactionsItem(item));
            }
        }
        Some(CommentHit::Block)
    }
}

/// `<login> • <%c>`, or `Unknown • <%c>` when the comment has no user: the block's author line.
pub fn author_line(user: Option<&User>, created_at: Timestamp) -> String {
    let author = user.map_or("Unknown", |user| user.login.as_str());
    format!("{author} • {}", time::c_format(created_at))
}

impl CommentBlock {
    /// The block for `comment`, in the repository named `repo_full_name` (`owner/name`), which its `#N` references
    /// point into. Its reactions aren't known yet.
    pub fn new(comment: &IssueComment, repo_full_name: &str) -> Self {
        Self {
            id: comment.id,
            body: Markdown::parse(&references::expand(&comment.body, repo_full_name)),
            author: author_line(comment.user.as_ref(), comment.created_at),
            reactions: None,
            body_layouts: LayoutCache::default(),
        }
    }

    /// The comment's id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The comment's reactions arrived. Any show as a closed section after the author line; none, or a failed
    /// request, show nothing, and a failure is logged at debug, as Python does.
    pub fn set_reactions(&mut self, reactions: Result<Reactions, GhError>) {
        self.reactions = match reactions {
            Ok(reactions) => ReactionsSummary::new(&reactions).map(|summary| ReactionsSection {
                summary,
                collapsible: Collapsible::default(),
                list: ListView::default(),
            }),
            Err(error) => {
                tracing::debug!("Could not find comment with ID {}: {error}", self.id);
                None
            }
        };
    }

    /// Whether it shows a reactions section.
    pub fn has_reactions(&self) -> bool {
        self.reactions.is_some()
    }

    /// Whether its reactions section is open.
    pub fn reactions_open(&self) -> bool {
        self.reactions
            .as_ref()
            .is_some_and(|section| section.collapsible.is_open())
    }

    /// The highlighted reactions list item, if any.
    pub fn reaction_highlight(&self) -> Option<usize> {
        self.reactions.as_ref().and_then(|section| section.list.highlight())
    }

    /// Highlights reactions list item `item`, as a click on it does.
    pub fn set_reaction_highlight(&mut self, item: Option<usize>) {
        if let Some(section) = &mut self.reactions {
            section.list.set_highlight(item);
        }
    }

    /// The block's focus stops, in order: the block, then its reactions title when it shows one, then the list when
    /// the section is open.
    pub fn stops(&self) -> Vec<CommentStop> {
        let mut stops = vec![CommentStop::Block];
        if let Some(section) = &self.reactions {
            stops.push(CommentStop::ReactionsTitle);
            if section.collapsible.is_open() && !section.summary.rows.is_empty() {
                stops.push(CommentStop::ReactionsList);
            }
        }
        stops
    }

    /// Opens or closes the reactions section: `enter` on its title, or a click on it. Nothing without one.
    pub fn toggle_reactions(&mut self) {
        if let Some(section) = &mut self.reactions {
            section.collapsible.toggle();
        }
    }

    /// A binding matched at one of [`Self::chain`]'s links for `stop`, applied. Says whether it applied: reply
    /// (`conversation.comment.reply`) doesn't until #182, so the owner treats it as not built yet.
    pub fn on_binding(&mut self, stop: CommentStop, id: BindingId) -> bool {
        let Some(section) = &mut self.reactions else {
            return false;
        };
        let len = section.summary.rows.len();
        match (stop, id) {
            (CommentStop::ReactionsTitle, BindingId::TextualCollapsibleToggle) => section.collapsible.toggle(),
            (CommentStop::ReactionsList, BindingId::TextualListCursorUp) => section.list.cursor_up(len),
            (CommentStop::ReactionsList, BindingId::TextualListCursorDown) => section.list.cursor_down(len),
            // Textual posts `Selected`, which nothing handles.
            (CommentStop::ReactionsList, BindingId::TextualListSelect) => {}
            _ => return false,
        }
        true
    }

    /// The binding chain links for focus at `stop`, innermost first. The owner puts its own links after them.
    pub fn chain(stop: CommentStop) -> Vec<ChainLink> {
        let block = ChainLink::new(Owner::CommentBlock, COMMENT_BINDINGS);
        match stop {
            CommentStop::Block => vec![block],
            CommentStop::ReactionsTitle => vec![ChainLink::new(Owner::CollapsibleTitle, TITLE_BINDINGS), block],
            CommentStop::ReactionsList => vec![ChainLink::new(Owner::ListView, LIST_BINDINGS), block],
        }
    }

    /// Lays the block out `width` cells wide over `under`, the background it's drawn on. `focus` is the focused stop
    /// when focus is on the block or inside it, which draws the dashed box.
    pub fn layout(&mut self, width: u16, focus: Option<CommentStop>, under: Paint, theme: &Theme) -> CommentLayout {
        let mut out = CommentLayout::default();
        if width <= TEXT_LEFT {
            return out;
        }
        let palette = theme.palette;
        let focused = focus.is_some();
        let fill = Style::new().bg(theme.paint(under));
        let frame = Frame {
            width,
            focused,
            border: Style::new()
                .fg(theme.color(
                    if focused {
                        palette.success
                    } else {
                        palette.secondary_background
                    },
                    under,
                ))
                .bg(theme.paint(under)),
            fill,
        };
        // The cell after the content: a focused block's right border takes the last one.
        let right = if focused { width - 1 } else { width };
        let rendered = &mut out.rendered;

        if focused {
            rendered.lines.push(frame.horizontal('┏', '┓'));
        }

        let body_width = right - TEXT_LEFT;
        let body = self
            .body_layouts
            .get(body_width, theme, || self.body.layout_unpadded(body_width, theme))
            .clone();
        frame.push(rendered, body, TEXT_LEFT);

        let author = markdown::wrap_text(
            &[TextRun::new(
                self.author.as_str(),
                Style::new().fg(theme.color(palette.text_muted, under)),
            )],
            right - TEXT_LEFT,
        );
        frame.push(rendered, author, TEXT_LEFT);
        // The author line's `margin-bottom: 1`.
        frame.push(rendered, blank(), TEXT_LEFT);

        if let Some(section) = &self.reactions {
            let focus_title = focus == Some(CommentStop::ReactionsTitle);
            let focus_list = focus == Some(CommentStop::ReactionsList);
            let section_focus = SectionFocus {
                title: focus_title,
                within: focus_title || focus_list,
            };
            let title = [TextRun::new(section.summary.title.as_str(), Style::new())];
            let mut inside = Rendered::default();
            let mut items = Vec::new();
            let mut placement = section.collapsible.layout(
                &mut inside,
                &title,
                section_focus,
                right - SECTION_LEFT,
                theme,
                |content| {
                    let rows: Vec<Vec<TextRun>> = section
                        .summary
                        .rows
                        .iter()
                        .map(|row| vec![TextRun::new(row.as_str(), Style::new())])
                        .collect();
                    let list = section
                        .list
                        .layout(&rows, content.width, theme, focus_list, content.under);
                    items = list.items;
                    list.rendered
                },
            );
            let shift = rendered.lines.len();
            frame.push(rendered, inside, SECTION_LEFT);
            let moved = |rows: &Range<usize>| rows.start + shift..rows.end + shift;
            placement.rows = moved(&placement.rows);
            placement.content = moved(&placement.content);
            placement.title_row += shift;
            placement.title_columns =
                placement.title_columns.start + SECTION_LEFT..placement.title_columns.end + SECTION_LEFT;
            out.reaction_items = items
                .iter()
                .map(|rows| rows.start + placement.content.start..rows.end + placement.content.start)
                .collect();
            out.reactions = Some(placement);
        }

        let rendered = &mut out.rendered;
        if focused {
            rendered.lines.push(frame.horizontal('┗', '┛'));
        }
        // The block's `margin-bottom: 1`, outside the border.
        let mut margin = Line::default();
        pad_line(&mut margin, width, fill);
        rendered.lines.push(margin);
        out
    }
}

/// The block's border and background, for wrapping its rows.
struct Frame {
    width: u16,
    focused: bool,
    border: Style,
    fill: Style,
}

impl Frame {
    /// The dashed box's top or bottom row.
    fn horizontal(&self, left: char, right: char) -> Line<'static> {
        let dashes = "╍".repeat(usize::from(self.width.saturating_sub(MARGIN + 2)));
        Line::from(vec![
            Span::styled(" ".repeat(usize::from(MARGIN)), self.fill),
            Span::styled(format!("{left}{dashes}{right}"), self.border),
        ])
    }

    /// Adds `more`'s rows to `out`, each starting at column `left` inside the border and filled out to the edge, its
    /// links moved with it.
    fn push(&self, out: &mut Rendered, more: Rendered, left: u16) {
        let side = if self.focused { "╏" } else { "│" };
        let right = if self.focused { self.width - 1 } else { self.width };
        let gap = " ".repeat(usize::from(left - MARGIN - 1));
        let mut more = more;
        for line in &mut more.lines {
            for span in &mut line.spans {
                if span.style.bg.is_none() {
                    span.style.bg = self.fill.bg;
                }
            }
            let mut spans = vec![
                Span::styled(" ".repeat(usize::from(MARGIN)), self.fill),
                Span::styled(side, self.border),
                Span::styled(gap.clone(), self.fill),
            ];
            spans.append(&mut line.spans);
            let mut row = Line::from(spans);
            pad_line(&mut row, right, self.fill);
            if self.focused {
                row.spans.push(Span::styled(side, self.border));
            }
            *line = row;
        }
        append(out, more, left);
    }
}

/// One blank row.
fn blank() -> Rendered {
    Rendered {
        lines: vec![Line::default()],
        links: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
