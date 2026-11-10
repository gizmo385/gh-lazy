//! The New comment dialog (`conversation.comment.new`, `conversation.comment.reply` or `conversation.review.reply`
//! on a comment or review, and `diff.reply_to_thread` on a review thread in the diff), on the compose sheet (decision
//! compose-dialogs): a heading, the replied-to text quoted when replying, then the body under its `Write  Preview`
//! tabs. It posts the comment itself and closes with it. Python's `NewCommentModal` around a `NewCommentContainer`, whose fixed 100 x 50 layout with a live
//! preview the sheet replaces.

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::models::{IssueComment, Repository, Review, ReviewComment};
use lazy_github_core::time;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use super::form::FormScroll;
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::compose::{
    BodyMode, BodyMouse, COMPOSE_BINDINGS, ComposeAction, ComposeBody, ComposeLayout, ComposeParts, Quote,
    SheetGeometry, SheetPlacement, sheet_geometry,
};
use crate::components::markdown::LinkSpan;
use crate::components::text_editor::{EditorOutcome, EditorVariant};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};
use crate::toast::Toast;

/// The sheet's widest.
const MAX_WIDTH: u16 = 100;
/// A comment's body is at least this tall.
const BODY_MIN_ROWS: u16 = 3;
/// The heading's tail while the post is in flight.
const POSTING: &str = "Posting…";

/// What a reply answers, which decides the heading and the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyTarget {
    /// A new comment, not a reply.
    None,
    /// A reply to an issue comment: posted as a new issue comment, since GitHub doesn't thread them.
    IssueComment(Box<IssueComment>),
    /// A reply to a review comment: posted in its thread through the review-reply endpoint.
    ReviewComment(Box<ReviewComment>),
    /// A reply to a review: posted as a new issue comment (Py bug B18 keeps that). A review that wasn't submitted
    /// is answered like a new comment.
    Review(Box<Review>),
    /// A reply from the diff to a review thread (#385): quotes `quoted`, any comment of the thread, and posts through
    /// the review-reply endpoint to the thread's first comment `root`, since GitHub doesn't take replies to replies.
    Thread { quoted: Box<ReviewComment>, root: u64 },
}

impl ReplyTarget {
    /// The "Replying to ..." heading and the quoted text, when the target has them.
    fn heading(&self) -> Option<(String, &str)> {
        match self {
            ReplyTarget::None => None,
            ReplyTarget::IssueComment(comment) => Some(comment_heading(comment)),
            ReplyTarget::ReviewComment(reply) | ReplyTarget::Thread { quoted: reply, .. } => {
                Some(comment_heading(&reply.comment))
            }
            ReplyTarget::Review(review) => review.submitted_at.map(|at| {
                let line = format!(
                    "Replying to comment from {} at {}",
                    review.user.login,
                    time::c_format(at)
                );
                (line, review.body.as_str())
            }),
        }
    }
}

/// The heading for a reply to `comment`: its author, `Unknown` when the account is gone, and when it was written.
fn comment_heading(comment: &IssueComment) -> (String, &str) {
    let author = comment.user.as_ref().map_or("Unknown", |user| user.login.as_str());
    let line = format!(
        "Replying to comment from {author} at {}",
        time::c_format(comment.created_at)
    );
    (line, comment.body.as_str())
}

/// What the dialog posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostedComment {
    /// A comment in the issue's or pull request's conversation.
    Issue(Box<IssueComment>),
    /// A reply in a review comment's thread.
    Review(Box<ReviewComment>),
}

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewCommentAction {
    /// One of the sheet's shared keys.
    Compose(ComposeAction),
}

/// Where the dialog's focus is: the body, its only field, or the sheet itself after `escape` left the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Body,
    Sheet,
}

/// The New comment dialog for issue or pull request `number` of `repo`. Its output is what it posted, or `None` when
/// it was closed.
#[derive(Debug)]
pub struct NewCommentDialog {
    repo: Repository,
    number: u64,
    target: ReplyTarget,
    heading: String,
    /// The replied-to text, when replying.
    quote: Option<Quote>,
    body: ComposeBody,
    focus: Focus,
    /// `compose.expand`: the sheet fills the screen above the footer.
    expanded: bool,
    placement: SheetPlacement,
    /// The interior, which scrolls (without a bar) only on a screen too short for even the body's minimum.
    scroll: FormScroll,
    /// Scroll the body into view at the next draw, after an edit or a focus change.
    reveal_body: bool,
    /// The theme of the last draw, which [`Dialog::frame`] measures the quote and the preview in.
    theme: Option<Theme>,
    /// The post request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<PostedComment>>,
}

impl NewCommentDialog {
    /// The dialog for a comment on issue or pull request `number` of `repo`, answering `target`, with an empty body
    /// focused.
    pub fn new(repo: Repository, number: u64, target: ReplyTarget) -> Self {
        let (heading, quote) = match target.heading() {
            Some((line, quote)) => (line, Some(Quote::new(quote))),
            None => ("New comment".to_owned(), None),
        };
        let placeholder = if quote.is_some() {
            "Write a reply (markdown)"
        } else {
            "Write a comment (markdown)"
        };
        Self {
            repo,
            number,
            target,
            heading,
            quote,
            body: ComposeBody::new(EditorVariant::Plain, placeholder, BODY_MIN_ROWS),
            focus: Focus::Body,
            expanded: false,
            placement: SheetPlacement::default(),
            scroll: FormScroll::without_scrollbar(),
            reveal_body: false,
            theme: None,
            pending: None,
            output: None,
        }
    }

    /// The text as typed.
    pub fn text(&self) -> String {
        self.body.text()
    }

    /// Whether a post request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: ComposeAction) -> Action {
        Action::Dialog(DialogAction::NewComment(NewCommentAction::Compose(action)))
    }

    /// The sheet's background, which its text and markdown sit on.
    fn under(theme: &Theme) -> Paint {
        theme.roles().surface.dialog.over(theme.background())
    }

    /// The sheet's limits on `screen` and its parts' rows, measured in `theme`.
    fn sheet(&self, screen: Rect, theme: &Theme) -> (SheetGeometry, ComposeLayout) {
        let geometry = sheet_geometry(screen, MAX_WIDTH, self.expanded);
        let under = Self::under(theme);
        let width = geometry.content_width;
        let parts = ComposeParts {
            quote: self
                .quote
                .as_ref()
                .map_or(0, |quote| quote.rows(width, geometry.quote_cap, theme, under)),
            body_needed: self.body.rows_needed(width, theme, under),
            body_min: self.body.min_rows(),
            ..ComposeParts::default()
        };
        (geometry, ComposeLayout::new(&parts, geometry.room(), self.expanded))
    }

    /// The frame's outer size on `screen`: as tall as the parts, up to the most the screen allows.
    fn sheet_size(&self, screen: Rect) -> (u16, u16) {
        let theme = self.theme.unwrap_or_default();
        let (geometry, layout) = self.sheet(screen, &theme);
        (geometry.width, layout.height.min(geometry.room()) + 2)
    }

    /// Focuses the body: its next edit starts a new undo batch, and it's scrolled into view.
    fn focus_body(&mut self) {
        if self.focus != Focus::Body {
            self.focus = Focus::Body;
            self.body.focus_gained();
        }
        self.reveal_body = true;
    }

    /// Python's `post_comment`: the text as it is, through the review-reply endpoint for a reply to a review comment
    /// and the issue comments endpoint otherwise. A second submit waits for the first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring a New comment submit: one is in flight");
            return;
        }
        let (repo, body) = (Box::new(self.repo.clone()), self.body.text());
        let request = match &self.target {
            ReplyTarget::ReviewComment(reply) => GithubRequest::ReplyToReviewComment {
                repo,
                pr_number: self.number,
                comment_id: reply.comment.id,
                body,
            },
            ReplyTarget::Thread { root, .. } => GithubRequest::ReplyToReviewComment {
                repo,
                pr_number: self.number,
                comment_id: *root,
                body,
            },
            ReplyTarget::None | ReplyTarget::IssueComment(_) | ReplyTarget::Review(_) => {
                GithubRequest::CreateIssueComment {
                    repo,
                    number: self.number,
                    body,
                }
            }
        };
        self.pending = Some(cx.request(request));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

/// The heading row: the heading, and while posting a dim `Posting…` at the row's end.
fn render_heading(buf: &mut Buffer, area: Rect, heading: &str, posting: bool, text: Style, dim: Style) {
    let tail = if posting {
        u16::try_from(POSTING.chars().count()).unwrap_or(u16::MAX)
    } else {
        0
    };
    let room = area.width.saturating_sub(if posting { tail + 1 } else { 0 });
    buf.set_span(area.x, area.y, &Span::styled(heading, text), room);
    if posting {
        buf.set_span(
            area.right().saturating_sub(tail),
            area.y,
            &Span::styled(POSTING, dim),
            tail,
        );
    }
}

/// `spans` drawn from (`x`, `y`) of the buffer, moved to rows from the buffer's top and columns from `left`.
fn moved(spans: Vec<LinkSpan>, x: u16, y: u16, left: u16) -> impl Iterator<Item = LinkSpan> {
    spans.into_iter().map(move |mut span| {
        span.row += usize::from(y);
        let shift = x.saturating_sub(left);
        span.columns = span.columns.start + shift..span.columns.end + shift;
        span
    })
}

impl Component for NewCommentDialog {
    type Action = NewCommentAction;

    fn bindings(&self) -> &'static [BindingId] {
        COMPOSE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        let action = match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit => ComposeAction::Submit,
            BindingId::ComposeTogglePreview => ComposeAction::TogglePreview,
            BindingId::ComposeExpand => ComposeAction::ToggleExpand,
            BindingId::ModalClose if self.focus == Focus::Body => ComposeAction::Leave,
            BindingId::ModalClose => ComposeAction::Close,
            _ => return None,
        };
        Some(Self::action(action))
    }

    fn binding_state(&self, id: BindingId, _cx: &Cx<'_>) -> BindingState {
        match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit if self.pending.is_some() => BindingState::Disabled,
            BindingId::ComposeTogglePreview if self.body.mode() == BodyMode::Preview => {
                BindingState::Relabeled("Write")
            }
            BindingId::ComposeExpand if self.expanded => BindingState::Relabeled("Collapse"),
            BindingId::ModalClose if self.focus == Focus::Body => BindingState::Relabeled("Leave"),
            _ => BindingState::Enabled,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.focus == Focus::Body && self.body.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        if self.focus != Focus::Body {
            return None;
        }
        Some(
            match self.body.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
                EditorOutcome::Changed => {
                    self.reveal_body = true;
                    Action::Consumed
                }
                EditorOutcome::Copied { text, changed } => {
                    self.reveal_body |= changed;
                    Action::Copy(text)
                }
                EditorOutcome::Moved | EditorOutcome::Ignored => Action::Consumed,
            },
        )
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        // The parts are drawn offscreen and copied with the interior's scroll: their coordinates are the content's.
        match self.body.handle_mouse(self.scroll.to_content(event)) {
            BodyMouse::Focused => self.focus_body(),
            BodyMouse::Handled => {}
            BodyMouse::Missed => {
                if let Some(wheel) = Wheel::from_mouse(&event) {
                    self.scroll.wheel(wheel);
                }
            }
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: NewCommentAction, cx: &mut Cx<'_>) {
        let NewCommentAction::Compose(action) = action;
        match action {
            ComposeAction::Submit => self.submit(cx),
            ComposeAction::Close => self.close(cx),
            ComposeAction::Leave => self.focus = Focus::Sheet,
            ComposeAction::TogglePreview => {
                self.body.toggle_mode();
                self.focus_body();
            }
            ComposeAction::ToggleExpand => {
                self.expanded = !self.expanded;
                self.reveal_body = true;
            }
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        self.theme = Some(*theme);
        let screen = frame.area();
        let outer = Rect {
            x: area.x.saturating_sub(1),
            y: area.y.saturating_sub(1),
            width: area.width + 2,
            height: area.height + 2,
        };
        self.placement.record(screen, outer, self.expanded);
        let (_, layout) = self.sheet(screen, theme);
        let roles = theme.roles();
        let under = Self::under(theme);
        let text = Style::new().fg(theme.color(roles.text.plain, under));
        let dim = Style::new().fg(theme.color(roles.text.muted, under));
        if std::mem::take(&mut self.reveal_body) {
            self.scroll.reveal(layout.body.clone());
        }
        let focused = self.focus == Focus::Body;
        let posting = self.pending.is_some();
        let (heading, quote, body) = (&self.heading, &self.quote, &mut self.body);
        // The links drawn, in rows from the interior's top and columns from its left edge.
        let mut links = Vec::new();
        self.scroll.render(
            frame.buffer_mut(),
            area,
            theme,
            text.bg(theme.paint(under)),
            layout.height,
            |buf, content| {
                let row = |y: u16, height: u16| Rect {
                    x: content.x + 1,
                    y: content.y + y,
                    width: content.width.saturating_sub(2),
                    height,
                };
                render_heading(buf, row(layout.heading, 1), heading, posting, text, dim);
                if let Some(quote) = quote {
                    let rows = layout.quote.end - layout.quote.start;
                    let rect = row(layout.quote.start, rows);
                    links.extend(moved(quote.render(buf, rect, theme, under), rect.x, rect.y, content.x));
                }
                let rect = row(layout.body.start, layout.body_rows());
                let spans = body.render(buf, row(layout.tabs, 1), rect, theme, under, focused);
                links.extend(moved(spans, rect.x, rect.y, content.x));
            },
        );
        // Textual's `Markdown` keeps `open_links` in a dialog, so a click on the quote's or the preview's links opens
        // them.
        if let Some(sink) = cx.links {
            self.scroll.record_links(sink, &links);
        }
    }
}

impl Dialog for NewCommentDialog {
    type Output = Option<PostedComment>;

    fn kind(&self) -> ModalKind {
        ModalKind::NewComment
    }

    fn own_action(&self, action: DialogAction) -> Option<NewCommentAction> {
        match action {
            DialogAction::NewComment(action) => Some(action),
            _ => None,
        }
    }

    /// `tab` and `shift+tab` go back to the body from the sheet; in the body, its only field, they do nothing.
    fn move_focus(&mut self, _direction: FocusMove) {
        self.focus_body();
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        self.sheet_size(Rect {
            height: available.height + 1,
            ..available
        })
    }

    fn frame(&self, area: Rect) -> Rect {
        self.placement.place(area, self.sheet_size(area), self.expanded)
    }

    fn take_output(&mut self) -> Option<Option<PostedComment>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        match self.focus {
            // The preview keeps the editor's link: the keys it would type do nothing there instead of reaching the
            // sheet's bindings, and the footer stays as it was.
            Focus::Body => vec![FieldKind::TextEditor.link(self.kind())],
            Focus::Sheet => Vec::new(),
        }
    }

    fn on_field_binding(&mut self, _field: FieldKind, _id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        // `consumes_key` already gave the body every key the editor binds.
        Some(Action::Consumed)
    }

    fn captures_mouse(&self) -> bool {
        self.body.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        (self.focus == Focus::Body).then(|| {
            if self.body.handle_paste(text, cx.now()) == EditorOutcome::Changed {
                self.reveal_body = true;
            }
            Action::Consumed
        })
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let number = self.number;
        let result = match response {
            GithubResponse::CreateIssueComment(result) => result.map(PostedComment::Issue),
            GithubResponse::ReplyToReviewComment(result) => result.map(PostedComment::Review),
            other => return Some(other),
        };
        self.pending = None;
        match result {
            Ok(posted) => {
                tracing::info!("Successfully posted new comment for issue #{number}");
                self.output = Some(Some(posted));
            }
            // Python crashes with the gh backend (Py bug B17): the dialog stays open with the text.
            Err(error) => {
                tracing::error!("Error while posting comment for issue #{number}: {error}");
                cx.notify(Toast::error("Could not post the comment").with_title("Error posting comment"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
