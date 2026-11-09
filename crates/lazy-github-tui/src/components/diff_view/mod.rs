//! The Diff tab's viewer (#344, decision `diff-continuous-view`): the file filter at the top, then one scroll holding
//! the continuous diff body ([`DiffBody`]) and its position row, the pending review comments ([`PendingComment`],
//! #272), which Add Comment adds, and once the owner shows it the Submit review form (#273), which the owner submits.
//!
//! The viewer doesn't know about pull requests: it takes a parsed [`Diff`]. Inside the details content it has an inner
//! focus ring ([`DiffStop`]): the filter, the body, then the Remove buttons and the form's fields. The filter is a
//! text input that takes its keys before the binding chain, as the table's search box does.
//!
//! The body draws only the rows on screen, so the scroll isn't a laid-out text: [`Window`] maps its rows to the body's
//! rows, the position row and the lines of the pending comments and the form (the "tail", laid out each frame, as
//! it's short). While the body's rows fill the view, its position row stays at the view's bottom, and the body may
//! draw its cursor's file header over its top row (the sticky header), which clicks and the wheel account for.

mod body;
mod highlight;
mod review;
mod threads;

use std::ops::Range;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::diff::Diff;
use lazy_github_core::github::{NewReview, ReviewEvent};
use lazy_github_core::review_anchor::DiffThread;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub use body::{Cursor, CursorPosition, DIFF_BODY_BINDINGS, DiffBody, Row};
pub use highlight::code_spans;
pub use review::{DiffLineRef, MissingStatus, PendingComment, line_text};

use review::{ButtonPlaced, FormFocus, FormPlaced, ReviewForm};

use super::button::BUTTON_BINDINGS;
use super::command_log::Wheel;
use super::info::Scroll;
use super::link_map::ScreenLink;
use super::markdown::{self, Rendered, TextRun};
use super::scroll_view::ScrollView;
use super::scrollbar::{self, Axis, ScrollbarState};
use super::select::{SELECT_BINDINGS, SELECT_OVERLAY_BINDINGS, SelectOutcome};
use super::text_input::{self, INPUT_BINDINGS, InputOutcome, TextInput};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Owner};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Theme;
use tokio::time::Instant;

/// The filter's placeholder. It names the default key even with `diff.filter_files` rebound, as Python's does.
const FILTER_PLACEHOLDER: &str = "Filter files (press '/' to focus, ESC to clear)";

/// How many rows a wheel notch scrolls the view, as Textual's containers and the app's other scrolls do.
const WHEEL_ROWS: usize = 2;

/// `SplitDiffViewer.BINDINGS`: next and previous hunk, and the filter key, with focus on the filter, a Remove button
/// or the review form. The body's own link ([`DIFF_BODY_BINDINGS`]) has them too.
pub const DIFF_VIEWER_BINDINGS: &[BindingId] = &[
    BindingId::DiffNextHunk,
    BindingId::DiffPreviousHunk,
    BindingId::DiffFilterFiles,
];

/// The focused filter's bindings: `FileFilterInput`'s `escape`, which it takes before anything else, then
/// [`text_input::INPUT_BINDINGS`].
const FILTER_BINDINGS: &[BindingId] = &[
    BindingId::TextualFileFilterClear,
    BindingId::TextualInputCursorLeft,
    BindingId::TextualInputCursorRight,
    BindingId::TextualInputHome,
    BindingId::TextualInputEnd,
    BindingId::TextualInputCursorLeftSelect,
    BindingId::TextualInputCursorLeftWord,
    BindingId::TextualInputCursorLeftWordSelect,
    BindingId::TextualInputCursorRightSelect,
    BindingId::TextualInputCursorRightWord,
    BindingId::TextualInputCursorRightWordSelect,
    BindingId::TextualInputDeleteLeft,
    BindingId::TextualInputSelectAll,
    BindingId::TextualInputHomeSelect,
    BindingId::TextualInputEndSelect,
    BindingId::TextualInputDeleteRight,
    BindingId::TextualInputSubmit,
    BindingId::TextualInputDeleteLeftWord,
    BindingId::TextualInputDeleteLeftAll,
    BindingId::TextualInputDeleteRightWord,
    BindingId::TextualInputDeleteRightAll,
    BindingId::TextualInputCut,
    BindingId::TextualInputCopy,
    BindingId::TextualInputPaste,
];

/// A focus stop inside the viewer, in ring order: the filter, the body (while it has rows), then each pending
/// comment's Remove button, then the review form's status (not on the author's form), summary and button. Comments
/// are by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStop {
    /// The file filter.
    Filter,
    /// The continuous diff body and its line cursor.
    Body,
    /// A pending comment's Remove button.
    RemoveComment(usize),
    /// The review form's status select, its list open or closed.
    ReviewStatus,
    /// The review form's summary input.
    ReviewSummary,
    /// The review form's button: "Submit Review", or the author's "Add Comments".
    ReviewSubmit,
}

/// What a mouse event other than a press did to the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMouse {
    /// Not the viewer's.
    Ignored,
    /// Taken (a click on Remove dropped its comment).
    Consumed,
    /// A click on the review form's button: the owner submits the review.
    SubmitReview,
}

/// What the next draw scrolls into view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// The body's cursor, inside its margin.
    Cursor,
    /// A stop after the body: a Remove button or a form field.
    Stop(DiffStop),
}

/// The rows of the scroll under the filter: the body's rows, its position row (while it has rows), then the tail's
/// lines; and how many show at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Window {
    body: usize,
    tail: usize,
    height: usize,
}

/// What a window shows at one scroll offset.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Split {
    /// The body's rows shown, from the view's top.
    body: Range<usize>,
    /// The view row of the position row, when it shows.
    position: Option<usize>,
    /// The tail's lines shown.
    tail: Range<usize>,
    /// The view row of the first of them.
    tail_top: usize,
}

impl Window {
    fn position_rows(self) -> usize {
        usize::from(self.body > 0)
    }

    /// Every row, in scroll order.
    fn content(self) -> usize {
        self.body + self.position_rows() + self.tail
    }

    /// Tail line `line`'s row in the scroll.
    fn tail_row(self, line: usize) -> usize {
        self.body + self.position_rows() + line
    }

    /// What shows from row `offset` on. Rows follow one another in scroll order, but while the body's rows fill the
    /// view the position row takes its last row.
    fn split(self, offset: usize) -> Split {
        let height = self.height;
        let tail_from = |start: usize, top: usize| {
            let start = start.min(self.tail);
            start..(start + height.saturating_sub(top)).min(self.tail)
        };
        if self.body == 0 || offset > self.body {
            let start = offset.saturating_sub(self.body + self.position_rows());
            return Split {
                body: offset..offset,
                position: None,
                tail: tail_from(start, 0),
                tail_top: 0,
            };
        }
        let shown = (self.body - offset).min(height.saturating_sub(1));
        Split {
            body: offset..offset + shown,
            position: (height > 0).then_some(shown),
            tail: tail_from(0, shown + 1),
            tail_top: shown + 1,
        }
    }
}

/// The pending comments and the form laid out, for one draw: their lines, and where their stops landed in them.
#[derive(Debug, Default)]
struct Tail {
    rendered: Rendered,
    placed: TailPlaced,
}

/// Where the tail's stops landed, by line.
#[derive(Debug, Clone, Default)]
struct TailPlaced {
    /// Each pending comment's Remove button.
    removes: Vec<ButtonPlaced>,
    /// The review form's fields, while it shows.
    form: Option<FormPlaced>,
}

impl TailPlaced {
    /// The lines of a stop after the body.
    fn lines_of(&self, stop: DiffStop) -> Option<Range<usize>> {
        match stop {
            DiffStop::RemoveComment(i) => self.removes.get(i).map(|p| p.rows.clone()),
            DiffStop::ReviewStatus => self.form.as_ref()?.status.clone(),
            DiffStop::ReviewSummary => self.form.as_ref().map(|p| p.summary.clone()),
            DiffStop::ReviewSubmit => self.form.as_ref().map(|p| p.submit.rows.clone()),
            DiffStop::Filter | DiffStop::Body => None,
        }
    }

    /// The stop drawn at `line` and column `x` of the tail.
    fn hit(&self, line: usize, x: u16) -> Option<DiffStop> {
        if let Some(index) = self.removes.iter().position(|placed| placed.contains(line, x)) {
            return Some(DiffStop::RemoveComment(index));
        }
        let form = self.form.as_ref()?;
        if form.status.as_ref().is_some_and(|rows| rows.contains(&line)) {
            Some(DiffStop::ReviewStatus)
        } else if form.summary.contains(&line) {
            Some(DiffStop::ReviewSummary)
        } else if form.submit.contains(line, x) {
            Some(DiffStop::ReviewSubmit)
        } else {
            None
        }
    }
}

/// Where the last draw put things, for clicks and the status's open list.
#[derive(Debug, Clone, Default)]
struct Drawn {
    /// The whole view.
    area: Rect,
    /// The scroll under the filter, its scrollbar included.
    region: Rect,
    /// The columns the scroll's rows got, the scrollbar left out.
    width: u16,
    window: Window,
    tail: TailPlaced,
}

/// What a screen cell of the last draw is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Filter,
    /// A body row, by its row number, and the cells from the body's left edge.
    Row(usize, usize),
    /// The sticky header over the body's top row: the header of this file, scrolled out above.
    Sticky(usize),
    /// The body's position row.
    Position,
    /// A stop after the body.
    Stop(DiffStop),
    Nothing,
}

/// The diff viewer: the filter, the body, the pending comments and the form, the scroll they share, and the inner
/// focus.
#[derive(Debug)]
pub struct DiffView {
    filter: TextInput,
    body: DiffBody,
    /// A line under the filter in place of the body: why the diff couldn't be parsed.
    notice: Option<String>,
    /// The comments added with Add Comment, in the order they were added.
    pending: Vec<PendingComment>,
    /// The Submit review form, once the owner shows it.
    form: Option<ReviewForm>,
    stop: DiffStop,
    /// Whether focus is inside the content, so the stop is drawn focused.
    focused: bool,
    /// The scroll under the filter, in [`Window`] rows. The body scrolls with it: the body's own scroll is set from
    /// it before the body moves or is drawn, and read back after a move.
    scroll: ScrollView,
    reveal: Option<Reveal>,
    drawn: Drawn,
    /// A left press landed on this button and its release hasn't come yet: the two make a click.
    armed: Option<DiffStop>,
}

impl DiffView {
    /// The viewer of `diff`: every file unfolded but deleted ones, the cursor on the first file's header.
    pub fn new(diff: Diff) -> Self {
        Self::build(DiffBody::new(diff), None)
    }

    /// A diff that couldn't be parsed: the filter, then `error parsing diff - please view on github: <reason>`.
    pub fn invalid(reason: &str) -> Self {
        Self::build(
            DiffBody::new(Diff { files: Vec::new() }),
            Some(format!("error parsing diff - please view on github: {reason}")),
        )
    }

    fn build(body: DiffBody, notice: Option<String>) -> Self {
        let stop = if body.rows() > 0 {
            DiffStop::Body
        } else {
            DiffStop::Filter
        };
        Self {
            filter: TextInput::new(FILTER_PLACEHOLDER),
            body,
            notice,
            pending: Vec::new(),
            form: None,
            stop,
            focused: false,
            scroll: ScrollView::default(),
            reveal: None,
            drawn: Drawn::default(),
            armed: None,
        }
    }

    /// The focused stop, if focus is inside the viewer.
    pub fn stop(&self) -> DiffStop {
        self.stop
    }

    /// The first visible row of the scroll under the filter.
    pub fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// The continuous diff body.
    pub fn body(&self) -> &DiffBody {
        &self.body
    }

    /// The filter's text.
    pub fn filter_text(&self) -> &str {
        self.filter.value()
    }

    /// Whether the filter keeps file `file`.
    pub fn is_file_shown(&self, file: usize) -> bool {
        self.body.is_shown(file)
    }

    /// Shows the pull request's review `threads` under their lines ([`DiffBody::set_threads`]), keeping the cursor on
    /// its row. `repo_full_name` is where the bodies' `#N` references point.
    pub fn set_threads(&mut self, threads: Vec<DiffThread>, repo_full_name: &str) {
        self.with_body(|body| body.set_threads(threads, repo_full_name));
    }

    /// The links the last draw put on screen, in the body's expanded threads.
    pub fn links(&self) -> &[ScreenLink] {
        self.body.links()
    }

    /// The pending comments, in the order they were added.
    pub fn pending(&self) -> &[PendingComment] {
        &self.pending
    }

    /// Shows the Submit review form after the body and the pending comments: the author's (`author`, only a summary
    /// and "Add Comments") or a reviewer's. A diff that couldn't be parsed shows no form, as in Python.
    pub fn show_review_form(&mut self, author: bool) {
        if self.notice.is_none() && self.form.is_none() {
            self.form = Some(ReviewForm::new(author));
        }
    }

    /// Whether the review form shows.
    pub fn has_review_form(&self) -> bool {
        self.form.is_some()
    }

    /// The review status chosen: `None` without a form or on the author's form, `Some(None)` for the blank option.
    pub fn review_status(&self) -> Option<Option<ReviewEvent>> {
        let status = self.form.as_ref()?.status.as_ref()?;
        Some(status.value().copied())
    }

    /// The review summary as typed, while the form shows.
    pub fn review_summary(&self) -> Option<&str> {
        self.form.as_ref().map(|form| form.summary.value())
    }

    /// The review form's button label, while the form shows.
    pub fn review_button(&self) -> Option<&str> {
        self.form.as_ref().map(|form| form.submit.label())
    }

    /// The review the form would send for commit `commit_id`, with every pending comment in the order they were
    /// added. `None` without a form.
    pub fn review_request(&self, commit_id: &str) -> Option<Result<NewReview, MissingStatus>> {
        let form = self.form.as_ref()?;
        Some(form.review(commit_id, &self.pending))
    }

    /// Whether a submitted review is in flight: the button is disabled until the answer comes.
    pub fn is_submitting(&self) -> bool {
        self.form.as_ref().is_some_and(|form| form.submit.disabled())
    }

    /// Disables the review form's button while a review is in flight, or enables it again.
    pub fn set_submitting(&mut self, submitting: bool) {
        if let Some(form) = &mut self.form {
            form.submit.set_disabled(submitting);
        }
    }

    /// What Add Comment comments on with the body focused: the diff line under the cursor, with an empty body. `None`
    /// on a file header or a `file was removed` row.
    pub fn comment_target(&self) -> Option<PendingComment> {
        if self.stop != DiffStop::Body {
            return None;
        }
        let line = self.body.cursor_line()?;
        let file = self.body.file(line.file)?;
        PendingComment::on(&file.path, file.hunks.get(line.hunk)?, line)
    }

    /// Adds `comment` after the others, showing the section with the first one. Focus stays where it is.
    pub fn add_comment(&mut self, comment: PendingComment) {
        self.pending.push(comment);
    }

    /// Drops pending comment `index`, hiding the section with the last one. A focused Remove button that goes takes
    /// focus to the stop before it, as Textual does when the focused widget is removed.
    fn remove_comment(&mut self, index: usize) {
        if index >= self.pending.len() {
            return;
        }
        let stop = match self.stop {
            DiffStop::RemoveComment(i) if i == index => {
                let stops = self.stops();
                let at = stops.iter().position(|&stop| stop == self.stop).unwrap_or(0);
                Some(stops[at.saturating_sub(1)])
            }
            DiffStop::RemoveComment(i) if i > index => Some(DiffStop::RemoveComment(i - 1)),
            _ => None,
        };
        self.pending.remove(index);
        if let Some(stop) = stop {
            self.focus(stop);
        }
    }

    /// A `textual.scroll.*` binding the focused stop doesn't take (the body takes them all, moving its cursor): up,
    /// down and the page keys scroll the view, and left and right pan the body's code.
    pub fn scroll(&mut self, scroll: Scroll) {
        match scroll {
            Scroll::Left | Scroll::PageLeft => self.pan(BindingId::TextualScrollLeft),
            Scroll::Right | Scroll::PageRight => self.pan(BindingId::TextualScrollRight),
            Scroll::Up | Scroll::Down | Scroll::Home | Scroll::End | Scroll::PageUp | Scroll::PageDown => {
                self.scroll.scroll(scroll);
                self.keep_cursor_in_view();
            }
        }
    }

    /// A wheel notch over the viewer: up and down scroll the view two rows, and the cursor stays on a row in view;
    /// sideways pans the body's code.
    pub fn wheel(&mut self, wheel: Wheel) {
        match wheel {
            Wheel::Up => {
                self.scroll.scroll_to(self.scroll.offset().saturating_sub(WHEEL_ROWS));
                self.keep_cursor_in_view();
            }
            Wheel::Down => {
                self.scroll.scroll_to(self.scroll.offset() + WHEEL_ROWS);
                self.keep_cursor_in_view();
            }
            Wheel::Left => self.pan(BindingId::TextualScrollLeft),
            Wheel::Right => self.pan(BindingId::TextualScrollRight),
        }
    }

    fn pan(&mut self, id: BindingId) {
        // Panning is one of the body's own bindings; it always takes it.
        let _ = self.body.handle(id);
    }

    /// After the view scrolled: a cursor whose row went out of view moves to the nearest row still in it, and off the
    /// top row when the sticky header covers it.
    fn keep_cursor_in_view(&mut self) {
        let rows = self.drawn.window.split(self.scroll.offset()).body;
        self.body.keep_cursor_in(rows.clone());
        if self.body.sticky_file(rows.clone()).is_some() && self.body.cursor_row() == Some(rows.start) {
            self.body.keep_cursor_in(rows.start + 1..rows.end);
        }
    }

    /// Runs `f` on the body scrolled to the view's offset, then scrolls the view to wherever the body went.
    fn with_body<T>(&mut self, f: impl FnOnce(&mut DiffBody) -> T) -> T {
        self.body.set_scroll(self.scroll.offset());
        let out = f(&mut self.body);
        self.scroll.scroll_to(self.body.scroll());
        out
    }

    // Focus.

    /// Whether the body is a stop: it has rows to show.
    fn has_body(&self) -> bool {
        self.body.rows() > 0
    }

    /// The inner focus ring, from what's shown.
    fn stops(&self) -> Vec<DiffStop> {
        let mut stops = vec![DiffStop::Filter];
        if self.has_body() {
            stops.push(DiffStop::Body);
        }
        stops.extend((0..self.pending.len()).map(DiffStop::RemoveComment));
        if let Some(form) = &self.form {
            if form.status.is_some() {
                stops.push(DiffStop::ReviewStatus);
            }
            stops.extend([DiffStop::ReviewSummary, DiffStop::ReviewSubmit]);
        }
        stops
    }

    /// Moves to `stop` and scrolls it into view at the next draw: the body's cursor, or a stop after the body (the
    /// filter is always in view). Focus moving onto the filter selects its text, as Textual's `Input` does on focus.
    fn focus(&mut self, stop: DiffStop) {
        let before = self.stop;
        if stop == DiffStop::Filter && before != DiffStop::Filter {
            self.filter.select_all();
        }
        if let Some(form) = &mut self.form {
            if stop == DiffStop::ReviewSummary && before != DiffStop::ReviewSummary {
                form.summary.select_all();
            }
            // The open list goes with the focus.
            if stop != DiffStop::ReviewStatus
                && let Some(status) = &mut form.status
            {
                status.close();
            }
        }
        self.stop = stop;
        self.reveal = match stop {
            DiffStop::Filter => None,
            DiffStop::Body => Some(Reveal::Cursor),
            other => Some(Reveal::Stop(other)),
        };
    }

    /// One stop `forward` or back; `false` past either end, where focus leaves the viewer.
    pub fn focus_step(&mut self, forward: bool) -> bool {
        let stops = self.stops();
        let at = stops.iter().position(|&stop| stop == self.stop).unwrap_or(0);
        let next = if forward {
            at.checked_add(1).filter(|&next| next < stops.len())
        } else {
            at.checked_sub(1)
        };
        match next {
            Some(next) => {
                self.focus(stops[next]);
                true
            }
            None => false,
        }
    }

    /// Focus enters from before: the body, so `j` moves the cursor at once; the filter when there's no body.
    pub fn focus_first(&mut self) {
        let first = if self.has_body() {
            DiffStop::Body
        } else {
            DiffStop::Filter
        };
        self.focus(first);
    }

    /// Focus enters from after: the last stop.
    pub fn focus_last(&mut self) {
        let last = self.stops().last().copied().unwrap_or(DiffStop::Filter);
        self.focus(last);
    }

    /// The chain links in front of the content's scroll link, for the focused stop.
    pub fn chain(&self) -> Vec<ChainLink> {
        let viewer = ChainLink::new(Owner::DiffViewer, DIFF_VIEWER_BINDINGS);
        match self.stop {
            DiffStop::Filter => vec![ChainLink::new(Owner::DiffFilter, FILTER_BINDINGS), viewer],
            DiffStop::Body => vec![ChainLink::new(Owner::DiffBody, DIFF_BODY_BINDINGS)],
            DiffStop::RemoveComment(_) | DiffStop::ReviewSubmit => {
                vec![ChainLink::new(Owner::DetailsButton, BUTTON_BINDINGS), viewer]
            }
            DiffStop::ReviewSummary => vec![ChainLink::new(Owner::DetailsInput, INPUT_BINDINGS), viewer],
            DiffStop::ReviewStatus if self.status_open() => {
                vec![
                    ChainLink::new(Owner::DetailsSelectOverlay, SELECT_OVERLAY_BINDINGS),
                    viewer,
                ]
            }
            DiffStop::ReviewStatus => vec![ChainLink::new(Owner::DetailsSelect, SELECT_BINDINGS), viewer],
        }
    }

    /// Whether the review status's list is open.
    pub(crate) fn status_open(&self) -> bool {
        self.form
            .as_ref()
            .and_then(|form| form.status.as_ref())
            .is_some_and(super::select::Select::is_open)
    }

    /// How the footer and help panel show a binding of [`Self::chain`]: Add Comment is hidden while the cursor isn't
    /// on a diff line; `enter` reads Fold on a file header, Expand or Collapse on a review thread or a line with
    /// threads, and is hidden elsewhere; Next and Previous Comment are hidden without threads; and Fold Files reads
    /// Unfold Files once every shown file is folded.
    pub fn binding_state(&self, owner: Owner, id: BindingId) -> BindingState {
        if owner != Owner::DiffBody {
            return BindingState::Enabled;
        }
        let on_header = matches!(self.body.cursor(), Some(Cursor::File(_)));
        match id {
            BindingId::DiffAddComment if self.body.cursor_line().is_none() => BindingState::Hidden,
            BindingId::DiffToggle if !on_header => match self.body.toggle_expands() {
                Some(true) => BindingState::Relabeled("Expand"),
                Some(false) => BindingState::Relabeled("Collapse"),
                None => BindingState::Hidden,
            },
            BindingId::DiffNextComment | BindingId::DiffPreviousComment if self.body.thread_count() == 0 => {
                BindingState::Hidden
            }
            BindingId::DiffFoldAll if self.body.all_folded() => BindingState::Relabeled("Unfold Files"),
            _ => BindingState::Enabled,
        }
    }

    /// A binding matched at one of [`Self::chain`]'s links. `None` lets the key fall through. Add Comment
    /// (`diff.add_comment`) isn't the viewer's: it opens a dialog, which the Diff tab does.
    pub fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        match (owner, id) {
            (Owner::DiffBody | Owner::DiffViewer, BindingId::DiffFilterFiles) => self.focus(DiffStop::Filter),
            (Owner::DiffBody, BindingId::DiffAddComment) => return None,
            (Owner::DiffBody, _) => {
                if !self.with_body(|body| body.handle(id)) {
                    return None;
                }
            }
            // `J` / `K` from the Remove buttons or the form: the body takes focus and its cursor jumps.
            (Owner::DiffViewer, BindingId::DiffNextHunk | BindingId::DiffPreviousHunk) => {
                if !self.has_body() {
                    return None;
                }
                self.focus(DiffStop::Body);
                self.with_body(|body| body.handle(id));
            }
            (Owner::DetailsButton, BindingId::TextualButtonPress) => match self.stop {
                DiffStop::RemoveComment(index) => self.remove_comment(index),
                _ => return None,
            },
            (Owner::DetailsSelect | Owner::DetailsSelectOverlay, _) => {
                let status = self.form.as_mut()?.status.as_mut()?;
                if status.on_binding(id) == SelectOutcome::Ignored {
                    return None;
                }
                // The view may have scrolled it out (the wheel): a list opened or moved in shows with its box.
                self.reveal = Some(Reveal::Stop(DiffStop::ReviewStatus));
            }
            (Owner::DiffFilter, BindingId::TextualFileFilterClear) => self.clear_filter(),
            // The filter and the summary took their editing keys before the chain.
            (Owner::DiffFilter | Owner::DetailsInput, _) => {}
            _ => return None,
        }
        Some(Action::Consumed)
    }

    /// A key before the binding chain, at `now`: the focused filter or review summary takes printable keys and its
    /// editing keys, and the review status's open list its search keys; a key in the summary or the list scrolls it
    /// back into view. `enter` leaves the filter for the body, the
    /// cursor on the first shown file's header, and does nothing in the summary. Paste types `clipboard`, and copy
    /// and cut hand the text to the app's clipboard.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str, now: Instant) -> Option<Action> {
        match (self.stop, &mut self.form) {
            (DiffStop::ReviewSummary, Some(form)) if form.summary.consumes_key(key, keymap) => {
                let action = match form.summary.handle_key(key, keymap, clipboard) {
                    InputOutcome::Copied(text) => Action::Copy(text),
                    InputOutcome::Changed | InputOutcome::Submitted | InputOutcome::Ignored => Action::Consumed,
                };
                self.reveal = Some(Reveal::Stop(DiffStop::ReviewSummary));
                return Some(action);
            }
            (
                DiffStop::ReviewStatus,
                Some(ReviewForm {
                    status: Some(status), ..
                }),
            ) if status.consumes_key(key, keymap) => {
                let _ = status.handle_key(key, keymap, now);
                self.reveal = Some(Reveal::Stop(DiffStop::ReviewStatus));
                return Some(Action::Consumed);
            }
            _ => {}
        }
        if self.stop != DiffStop::Filter || !self.filter.consumes_key(key, keymap) {
            return None;
        }
        let mut action = Action::Consumed;
        match self.filter.handle_key(key, keymap, clipboard) {
            InputOutcome::Changed => self.apply_filter(),
            InputOutcome::Submitted => self.leave_filter(),
            // A cut changes the text, so the filter applies again.
            InputOutcome::Copied(text) => {
                self.apply_filter();
                action = Action::Copy(text);
            }
            InputOutcome::Ignored => {}
        }
        Some(action)
    }

    /// A bracketed paste: its first line goes into the filter or the review summary while it has focus. Says whether
    /// it did.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        if let (DiffStop::ReviewSummary, Some(form)) = (self.stop, &mut self.form) {
            form.summary.handle_paste(text);
            return true;
        }
        if self.stop != DiffStop::Filter {
            return false;
        }
        self.filter.handle_paste(text);
        self.apply_filter();
        true
    }

    /// Shows the files whose full path contains the filter's text, ignoring case; all of them when it's empty. A
    /// cursor in a file it hides moves to the first shown file's header.
    fn apply_filter(&mut self) {
        let needle = self.filter.value().to_owned();
        self.with_body(|body| body.set_filter(&needle));
    }

    /// `enter` in the filter: the body, the cursor on the first shown file's header. Python focuses the first pane
    /// whatever its hunk (Py bug B59); there are no panes here. With no file shown, focus stays.
    fn leave_filter(&mut self) {
        if self.has_body() {
            self.focus(DiffStop::Body);
            self.with_body(DiffBody::cursor_to_first_file);
        }
    }

    /// `escape` in the filter: it clears, and the body takes focus with the cursor where it was.
    fn clear_filter(&mut self) {
        self.filter.set_value("");
        self.apply_filter();
        if self.has_body() {
            self.focus(DiffStop::Body);
        }
    }

    // The mouse.

    /// A left press at a screen cell, against the last frame: a body row focuses the body and takes the cursor (a
    /// hunk separator gives it to the hunk's first line), the filter or the summary takes focus with its cursor at the
    /// press, the status select opens or closes, and a button is armed for the release.
    pub fn click(&mut self, position: Position) {
        self.armed = None;
        // The status's open list is drawn over everything: a press on it picks, and one anywhere else closes it.
        if let Some(ReviewForm {
            status: Some(status), ..
        }) = &mut self.form
            && status.is_open()
        {
            let event = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: position.x,
                row: position.y,
                modifiers: crossterm::event::KeyModifiers::NONE,
            };
            if status.handle_overlay_mouse(&event) != SelectOutcome::Ignored {
                return;
            }
            status.close();
        }
        // Only the columns matter to an input's press.
        let input = Rect {
            height: text_input::HEIGHT,
            ..self.drawn.area
        };
        match self.hit(position) {
            Hit::Filter => {
                self.focus(DiffStop::Filter);
                self.filter.press(input, position.x);
            }
            Hit::Row(row, x) => {
                if self.body.click_row(row, x) {
                    // The row is in view already: the click doesn't scroll.
                    self.focus(DiffStop::Body);
                    self.reveal = None;
                }
            }
            // The real header is above the view: focusing the body reveals the cursor on it.
            Hit::Sticky(file) => {
                self.body.cursor_to_file(file);
                self.focus(DiffStop::Body);
            }
            Hit::Position => {
                self.focus(DiffStop::Body);
                self.reveal = None;
            }
            Hit::Stop(stop @ (DiffStop::RemoveComment(_) | DiffStop::ReviewSubmit)) => {
                self.armed = Some(stop);
                self.focus(stop);
            }
            Hit::Stop(DiffStop::ReviewStatus) => {
                self.focus(DiffStop::ReviewStatus);
                if let Some(ReviewForm {
                    status: Some(status), ..
                }) = &mut self.form
                {
                    let _ = status.toggle();
                }
            }
            Hit::Stop(DiffStop::ReviewSummary) => {
                self.focus(DiffStop::ReviewSummary);
                let input = Rect {
                    width: self.drawn.width,
                    ..input
                };
                if let Some(form) = &mut self.form {
                    form.summary.press(input, position.x);
                }
            }
            Hit::Stop(DiffStop::Filter | DiffStop::Body) | Hit::Nothing => {}
        }
    }

    /// Whether screen cell `(column, row)` is on the review status's open list, as last drawn by
    /// [`Self::render_popups`]. The list takes every mouse event there, ahead of whatever is drawn under it.
    pub fn popup_contains(&self, column: u16, row: u16) -> bool {
        matches!(
            &self.form,
            Some(ReviewForm { status: Some(status), .. }) if status.overlay_contains(column, row)
        )
    }

    /// Whether a press on the filter or the review summary started a drag that hasn't been released: the owner keeps
    /// the mouse for the viewer until then and hands it [`Self::captured_mouse`].
    pub fn captures_mouse(&self) -> bool {
        self.filter.is_selecting() || self.form.as_ref().is_some_and(|form| form.summary.is_selecting())
    }

    /// A mouse event while the viewer has the mouse, wherever the pointer is: the input being dragged over takes the
    /// left drags and the release ([`TextInput::captured_mouse`]). Says whether it took the event.
    pub fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        let summary = self.form.as_mut().map(|form| &mut form.summary);
        self.filter.captured_mouse(event) || summary.is_some_and(|input| input.captured_mouse(event))
    }

    /// A mouse event other than a press: the left release that completes a click on a button presses it.
    pub fn mouse(&mut self, event: MouseEvent) -> ViewMouse {
        if event.kind != MouseEventKind::Up(MouseButton::Left) {
            return ViewMouse::Ignored;
        }
        let Some(armed) = self.armed.take() else {
            return ViewMouse::Ignored;
        };
        if self.hit(Position::new(event.column, event.row)) != Hit::Stop(armed) {
            return ViewMouse::Ignored;
        }
        match armed {
            DiffStop::RemoveComment(index) => {
                self.remove_comment(index);
                ViewMouse::Consumed
            }
            DiffStop::ReviewSubmit => ViewMouse::SubmitReview,
            _ => ViewMouse::Ignored,
        }
    }

    /// What the last frame drew at a screen cell.
    fn hit(&self, position: Position) -> Hit {
        let drawn = &self.drawn;
        if !drawn.area.contains(position) {
            return Hit::Nothing;
        }
        if position.y < drawn.region.y {
            return Hit::Filter;
        }
        if self.notice.is_some() || position.x >= drawn.region.x.saturating_add(drawn.width) {
            return Hit::Nothing;
        }
        let dy = usize::from(position.y - drawn.region.y);
        let split = drawn.window.split(self.scroll.offset());
        if dy == 0
            && let Some(file) = self.body.sticky_file(split.body.clone())
        {
            return Hit::Sticky(file);
        }
        if dy < split.body.len() {
            return Hit::Row(split.body.start + dy, usize::from(position.x - drawn.region.x));
        }
        if split.position == Some(dy) {
            return Hit::Position;
        }
        let line = dy
            .checked_sub(split.tail_top)
            .map(|below| split.tail.start + below)
            .filter(|line| split.tail.contains(line));
        line.and_then(|line| drawn.tail.hit(line, position.x - drawn.region.x))
            .map_or(Hit::Nothing, Hit::Stop)
    }

    // Drawing.

    /// The pending comments and the form laid out `width` cells wide, focused as the view is.
    fn layout_tail(&mut self, width: u16, theme: &Theme) -> Tail {
        let remove_focused = match self.stop {
            DiffStop::RemoveComment(index) if self.focused => Some(index),
            _ => None,
        };
        let form_focused = match self.stop {
            DiffStop::ReviewStatus if self.focused => Some(FormFocus::Status),
            DiffStop::ReviewSummary if self.focused => Some(FormFocus::Summary),
            DiffStop::ReviewSubmit if self.focused => Some(FormFocus::Submit),
            _ => None,
        };
        let mut rendered = Rendered::default();
        let removes = review::layout_pending(&mut rendered, &self.pending, remove_focused, width, theme);
        let form = self
            .form
            .as_mut()
            .map(|form| review::layout_form(&mut rendered, form, form_focused, width, theme));
        Tail {
            rendered,
            placed: TailPlaced { removes, form },
        }
    }

    /// Draws the viewer into `area`, `focused` when focus is inside the details content.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        self.focused = focused;
        let resized = self.drawn.area != area;
        let buf = frame.buffer_mut();
        let filter_rows = text_input::HEIGHT.min(area.height);
        let filter_area = Rect {
            height: filter_rows,
            ..area
        };
        let filter_focused = focused && self.stop == DiffStop::Filter;
        self.filter
            .render(buf, filter_area, theme, theme.background(), filter_focused);
        let region = Rect {
            y: area.y + filter_rows,
            height: area.height - filter_rows,
            ..area
        };
        self.drawn.area = area;
        self.drawn.region = region;
        if let Some(notice) = &self.notice {
            let text = Style::new().fg(theme.color(theme.roles().text.plain, theme.background()));
            let lines = markdown::wrap_text(&[TextRun::new(notice.as_str(), text)], region.width).lines;
            for (y, line) in (region.y..region.bottom()).zip(&lines) {
                buf.set_line(region.x, y, line, region.width);
            }
            self.drawn.width = region.width;
            self.drawn.window = Window::default();
            self.drawn.tail = TailPlaced::default();
            return;
        }
        let height = usize::from(region.height);
        // The tail at the full width, then again beside the scrollbar when everything doesn't fit.
        let mut width = region.width;
        let mut tail = self.layout_tail(width, theme);
        let mut window = Window {
            body: self.body.rows(),
            tail: tail.rendered.lines.len(),
            height,
        };
        let overflows = window.content() > height;
        if overflows {
            width = region.width.saturating_sub(scrollbar::VERTICAL_WIDTH);
            tail = self.layout_tail(width, theme);
            window.tail = tail.rendered.lines.len();
        }
        self.scroll.resize(window.content(), height);
        self.body.fit(width, height.saturating_sub(1), window.tail);
        let reveal = self
            .reveal
            .take()
            .or_else(|| (resized && self.stop == DiffStop::Body).then_some(Reveal::Cursor));
        match reveal {
            Some(Reveal::Cursor) => self.with_body(DiffBody::reveal_cursor),
            Some(Reveal::Stop(stop)) => {
                if let Some(lines) = tail.placed.lines_of(stop) {
                    self.scroll
                        .reveal(window.tail_row(lines.start)..window.tail_row(lines.end));
                }
            }
            None => {}
        }
        let offset = self.scroll.offset();
        self.body.set_scroll(offset);
        let split = window.split(offset);
        // Rows are bounded by the region's height, a u16.
        let row = |dy: usize| region.y.saturating_add(u16::try_from(dy).unwrap_or(u16::MAX));
        if let Some(position) = split.position {
            let rows = Rect {
                width,
                height: u16::try_from(position + 1).unwrap_or(u16::MAX),
                ..region
            };
            let body_focused = focused && self.stop == DiffStop::Body;
            self.body.draw(buf, rows, theme, body_focused);
        }
        for (dy, line) in (split.tail_top..).zip(&tail.rendered.lines[split.tail.clone()]) {
            buf.set_line(region.x, row(dy), line, width);
        }
        if overflows {
            let bar = Rect {
                x: region.x + width,
                width: region.width - width,
                ..region
            };
            let state = ScrollbarState {
                content: window.content(),
                window: height,
                position: offset,
            };
            scrollbar::render(buf, bar, Axis::Vertical, state, DiffBody::scrollbar_colors(theme));
        }
        self.drawn.width = width;
        self.drawn.window = window;
        self.drawn.tail = tail.placed;
    }

    /// What the viewer draws over the whole screen, after everything else on it (the command log, the status bar,
    /// the footer): the review status's open list, as Textual's `overlay: screen` draws it, right below the box and
    /// moved up to fit the screen. Placed against the last [`Self::render`].
    pub fn render_popups(&mut self, frame: &mut Frame<'_>, theme: &Theme) {
        let drawn = &self.drawn;
        let Some(lines) = drawn.tail.form.as_ref().and_then(|form| form.status.clone()) else {
            return;
        };
        let split = drawn.window.split(self.scroll.offset());
        let region = drawn.region;
        let width = drawn.width;
        let Some(ReviewForm {
            status: Some(status), ..
        }) = &mut self.form
        else {
            return;
        };
        if !status.is_open() {
            return;
        }
        // The box's rows on screen, kept inside the view when it's scrolled out of it.
        let as_i64 = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        let top = i64::from(region.y) + as_i64(split.tail_top) + as_i64(lines.start) - as_i64(split.tail.start);
        let top = top.clamp(i64::from(region.y), i64::from(region.bottom().saturating_sub(1)));
        let below = Rect {
            x: region.x,
            y: u16::try_from(top).unwrap_or(region.y),
            width,
            height: crate::components::select::HEIGHT,
        };
        let screen = frame.area();
        status.render_overlay(frame.buffer_mut(), below, screen, theme, theme.background());
    }
}

/// Row `y` of `buf` as a line of styled spans, a span per run of cells with the same style.
fn buffer_line(buf: &Buffer, y: u16) -> Line<'static> {
    let area = buf.area;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut skip = 0;
    for x in area.left()..area.right() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let cell = &buf[(x, y)];
        let symbol = cell.symbol();
        // A wide character covers the cells after it.
        skip = Span::raw(symbol).width().saturating_sub(1);
        let style = cell.style();
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(symbol),
            _ => spans.push(Span::styled(symbol.to_owned(), style)),
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests;
