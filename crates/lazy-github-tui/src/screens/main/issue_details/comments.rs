//! An issue's Comments tab: the issue's comments as comment blocks, oldest first, stacked in one scroll view.
//! Python's `IssueConversationTabPane`.
//!
//! The comments are requested with the tab, which Textual mounts when the issue is selected, whatever tab shows.
//! Until they arrive the tab shows the loading overlay. Inside the tab, focus walks the scroll view itself, then each
//! block.

use std::ops::Range;
use std::time::Duration;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{Issue, IssueComment};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use tokio::time::Instant;

use crate::component::{ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::comment::{CommentBlock, CommentLayout, CommentStop};
use crate::components::info::Scroll;
use crate::components::loading;
use crate::components::markdown::{Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::super::details::BodyRenderCx;
use super::super::overview_text::{HeaderStyles, append, push_label};

/// `IssueConversationTabPane.BINDINGS`: a new comment (#182), in the footer while focus is anywhere in the tab.
pub const ISSUE_COMMENTS_BINDINGS: &[BindingId] = &[BindingId::ConversationCommentNew];

/// What the tab shows when there are no comments, or they failed to load (Py bug B60).
const NO_COMMENTS: &str = "No comments available";

/// Where the comments are.
#[derive(Debug)]
enum State {
    /// Requested: the loading overlay, animated from when it was first drawn.
    Loading { id: RequestId, since: Option<Instant> },
    /// Arrived, as the body's blocks: none when there are no comments or the request failed.
    Loaded,
}

/// The Comments tab of one issue: its blocks, its scroll position and its inner focus.
#[derive(Debug)]
pub(in crate::screens::main) struct IssueComments {
    state: State,
    scroll: ScrollView,
    body: Body,
    /// Whether the next draw scrolls the focused block into view.
    reveal: bool,
    /// Where the body was drawn last, for clicks.
    area: Rect,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    /// `owner/name` of the issue's repository, which the comments' `#N` references point into.
    repo: String,
    /// The comments, which a reply quotes, in the blocks' order.
    comments: Vec<IssueComment>,
    blocks: Vec<CommentBlock>,
    /// Whether "No comments available" shows: the comments loaded empty. A comment posted later goes below it, as in
    /// Python.
    none_loaded: bool,
    /// The focused block, or `None` for the scroll view itself.
    stop: Option<usize>,
    /// Whether focus is inside the content, so the focused block is drawn in its box.
    focused: bool,
    layouts: LayoutCache,
    /// Where each block landed in each cached layout, by width.
    placements: Vec<(u16, Vec<Placed>)>,
    /// The width of the last layout asked for: the one the last draw used.
    last_width: u16,
}

/// Where a block landed in a layout.
#[derive(Debug)]
struct Placed {
    /// Its rows in the body, the margin row below it included.
    rows: Range<usize>,
    /// The block as laid out, for clicks.
    layout: CommentLayout,
}

impl IssueComments {
    /// The tab for `issue`, with its comments requested: Python's `on_mount`.
    pub(super) fn new(issue: &Issue, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::IssueComments {
            issue: Box::new(issue.clone()),
        });
        Self {
            state: State::Loading { id, since: None },
            scroll: ScrollView::default(),
            body: Body {
                repo: issue.repo.full_name.clone(),
                comments: Vec::new(),
                blocks: Vec::new(),
                none_loaded: false,
                stop: None,
                focused: false,
                layouts: LayoutCache::default(),
                placements: Vec::new(),
                last_width: 0,
            },
            reveal: false,
            area: Rect::default(),
        }
    }

    /// Whether the comments are still loading.
    #[cfg(test)]
    pub(in crate::screens::main) fn is_loading(&self) -> bool {
        matches!(self.state, State::Loading { .. })
    }

    /// How many blocks show.
    #[cfg(test)]
    pub(in crate::screens::main) fn len(&self) -> usize {
        self.body.blocks.len()
    }

    /// The focused block, or `None` for the scroll view itself.
    #[cfg(test)]
    pub(in crate::screens::main) fn stop(&self) -> Option<usize> {
        self.body.stop
    }

    /// The first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    // Requests.

    /// Takes the answer to the comments request, and hands back any other response.
    pub(super) fn on_response(&mut self, id: RequestId, response: GithubResponse) -> Option<GithubResponse> {
        match (&self.state, response) {
            (State::Loading { id: awaited, .. }, GithubResponse::IssueComments(result)) if *awaited == id => {
                self.on_comments(result);
                None
            }
            (_, other) => Some(other),
        }
    }

    /// The comments arrived. A failure is logged and shows as no comments (Py bug B60: Python's worker fails and
    /// the app exits).
    fn on_comments(&mut self, result: Result<Vec<IssueComment>, GhError>) {
        let comments = result.unwrap_or_else(|error| {
            tracing::error!("Error retrieving issue comments");
            tracing::debug!("Fetching the issue comments failed: {error}");
            Vec::new()
        });
        let repo = &self.body.repo;
        self.body.blocks = comments
            .iter()
            .map(|comment| CommentBlock::new(comment, repo))
            .collect();
        self.body.none_loaded = comments.is_empty();
        self.body.comments = comments;
        self.body.stop = None;
        self.body.clear();
        self.state = State::Loaded;
    }

    /// The comment of the focused block, if a block has focus.
    pub(super) fn focused_comment(&self) -> Option<&IssueComment> {
        self.body.comments.get(self.body.stop?)
    }

    /// A posted comment or reply: a block below the others. Focus stays where it is.
    pub(super) fn append(&mut self, comment: IssueComment) {
        self.body.blocks.push(CommentBlock::new(&comment, &self.body.repo));
        self.body.comments.push(comment);
        self.body.clear();
    }

    /// The tab is going away: cancels the request if it's still out.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let State::Loading { id, .. } = self.state {
            cx.effect(Effect::Cancel(id));
        }
    }

    // Scrolling.

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    // Focus.

    /// Moves to `stop`, and scrolls a block into view at the next draw.
    fn focus(&mut self, stop: Option<usize>) {
        self.body.set_stop(stop);
        self.reveal = stop.is_some();
    }

    /// One stop `forward` or back: the scroll view, then each block. `false` past either end.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        let len = self.body.blocks.len();
        let next = match (self.body.stop, forward) {
            (None, true) if len > 0 => Some(Some(0)),
            (Some(at), true) if at + 1 < len => Some(Some(at + 1)),
            (Some(0), false) => Some(None),
            (Some(at), false) => Some(Some(at - 1)),
            _ => None,
        };
        match next {
            Some(stop) => {
                self.focus(stop);
                true
            }
            None => false,
        }
    }

    /// Focus enters from before: the scroll view.
    pub(super) fn focus_first(&mut self) {
        self.focus(None);
    }

    /// Focus enters from after: the last block, or the scroll view without any.
    pub(super) fn focus_last(&mut self) {
        self.focus(self.body.blocks.len().checked_sub(1));
    }

    /// Whether a block has focus, which lets `M` maximize the details tab area: Textual maximizes the block's
    /// nearest maximizable ancestor, the `TabbedContent`.
    pub(super) fn block_focused(&self) -> bool {
        self.body.stop.is_some()
    }

    /// The chain links in front of the content's scroll link: the focused block's, then the tab's.
    pub(super) fn chain(&self) -> Vec<ChainLink> {
        let mut chain = match self.body.stop {
            Some(_) => CommentBlock::chain(CommentStop::Block),
            None => Vec::new(),
        };
        chain.push(ChainLink::new(Owner::IssueComments, ISSUE_COMMENTS_BINDINGS));
        chain
    }

    /// A left press at a screen cell, against the last frame: a block focuses it, anything else the scroll view.
    pub(super) fn click(&mut self, position: Position) {
        let stop = self.block_at(position);
        self.focus(stop);
    }

    /// The block drawn at a screen cell.
    fn block_at(&mut self, position: Position) -> Option<usize> {
        let area = self.area;
        let width = self.body.last_width;
        if !area.contains(position) || position.x >= area.x.saturating_add(width) {
            return None;
        }
        let row = self.scroll.offset() + usize::from(position.y - area.y);
        let column = position.x - area.x;
        let placements = self.body.placements(width)?;
        let block = placements.iter().position(|placed| placed.rows.contains(&row))?;
        let placed = &placements[block];
        placed.layout.hit(row - placed.rows.start, column).map(|_| block)
    }

    // Drawing.

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        self.area = area;
        if let State::Loading { since, .. } = &mut self.state {
            let since = *since.get_or_insert(cx.now);
            let elapsed = cx.now.checked_duration_since(since).unwrap_or(Duration::ZERO);
            loading::render(frame.buffer_mut(), area, cx.theme, cx.animations, elapsed);
            return;
        }
        self.body.set_focused(cx.focused);
        if std::mem::take(&mut self.reveal) {
            let width = self.scroll.measure(area, cx.theme, &mut self.body);
            if let Some(rows) = self.body.focused_rows(width) {
                self.scroll.reveal(rows);
            }
        }
        self.scroll.render(frame, area, cx.theme, &mut self.body);
        if let Some(links) = cx.links {
            self.scroll.record_links(area, cx.theme, &mut self.body, links);
        }
    }

    /// When the loading overlay's next frame is due.
    pub(super) fn next_deadline(&self, now: Instant, animations: bool) -> Option<Instant> {
        (animations && matches!(self.state, State::Loading { .. })).then_some(now + loading::FRAME_INTERVAL)
    }
}

impl Body {
    /// Forgets the layouts, for when what they show changes.
    fn clear(&mut self) {
        self.layouts.clear();
        self.placements.clear();
    }

    fn set_stop(&mut self, stop: Option<usize>) {
        if self.stop != stop {
            self.stop = stop;
            self.clear();
        }
    }

    /// Focus moved in or out of the content. Only a focused block looks different.
    fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if self.stop.is_some() {
                self.clear();
            }
        }
    }

    fn placements(&self, width: u16) -> Option<&[Placed]> {
        self.placements
            .iter()
            .find(|(w, _)| *w == width)
            .map(|(_, placements)| placements.as_slice())
    }

    /// The focused block's rows in the layout `width` wide, without the margin row below it.
    fn focused_rows(&self, width: u16) -> Option<Range<usize>> {
        let rows = &self.placements(width)?.get(self.stop?)?.rows;
        Some(rows.start..rows.end.saturating_sub(1).max(rows.start))
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        self.last_width = width;
        let focused = self.focused.then_some(self.stop).flatten();
        let Body {
            blocks,
            none_loaded,
            layouts,
            placements,
            ..
        } = self;
        layouts.get(width, theme, || {
            let mut out = Rendered::default();
            let mut placed = Vec::with_capacity(blocks.len());
            if *none_loaded || blocks.is_empty() {
                let HeaderStyles { text, .. } = HeaderStyles::new(theme);
                push_label(&mut out, &[TextRun::new(NO_COMMENTS, text)], width);
            }
            for (index, block) in blocks.iter_mut().enumerate() {
                let focus = (focused == Some(index)).then_some(CommentStop::Block);
                let layout = block.layout(width, focus, theme.background(), theme);
                let start = out.lines.len();
                append(&mut out, layout.rendered.clone());
                placed.push(Placed {
                    rows: start..out.lines.len(),
                    layout,
                });
            }
            placements.retain(|(w, _)| *w != width);
            if placements.len() == 2 {
                placements.remove(0);
            }
            placements.push((width, placed));
            out
        })
    }
}
