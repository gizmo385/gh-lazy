//! The New comment dialog (`conversation.comment.new`, and `conversation.comment.reply` on a comment): an optional
//! "Replying to ..." heading with the replied-to text, a "New comment" editor, a live markdown "Preview", then Post
//! Comment and Cancel. It posts the comment itself and closes with it. Python's `NewCommentModal` around a
//! `NewCommentContainer`.
//!
//! The dialog is 100 x 50 with a cell of margin, taller than most screens: its frame runs past the screen's bottom, as
//! in Python, and the interior scrolls (without a bar) to keep the focused field in view, where Python scrolls the
//! modal screen.

use std::ops::Range;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::{IssueComment, Repository};
use lazy_github_core::time;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

use super::form::FormScroll;
use super::{
    Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings, frame_rect_with_margin,
};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::markdown::Markdown;
use crate::components::text_editor::{EditorBorder, EditorOutcome, EditorPadding, EditorVariant, TextEditor};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Theme;
use crate::toast::Toast;

/// The dialog's screen bindings: `modal.submit` (Py bug B17's recommendation; Python binds only `modal.close`), then
/// `modal.close`.
const NEW_COMMENT_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// `NewCommentContainer { width: 100; height: 50; margin: 1 }`, border included.
const SIZE: (u16, u16) = (100, 50);
const MARGIN: u16 = 1;

/// `min-height: 40%` of the editor and the preview, of the interior's height.
const BOX_SHARE: f64 = 0.4;

/// Rows of the editor's and the preview's `border: blank` and `padding-top: 1` above their text.
const BOX_TOP: u16 = 2;

/// What a reply answers, which decides the heading and the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyTarget {
    /// A new comment, not a reply: no heading.
    None,
    /// A reply to an issue comment: posted as a new issue comment, since GitHub doesn't thread them.
    IssueComment(Box<IssueComment>),
}

impl ReplyTarget {
    /// The heading line and the quoted text, when the target has a heading.
    fn heading(&self) -> Option<(String, &str)> {
        match self {
            ReplyTarget::None => None,
            ReplyTarget::IssueComment(comment) => {
                let author = comment.user.as_ref().map_or("Unknown", |user| user.login.as_str());
                let line = format!(
                    "Replying to comment from {author} at {}",
                    time::c_format(comment.created_at)
                );
                Some((line, comment.body.as_str()))
            }
        }
    }
}

/// What the dialog posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostedComment {
    /// A comment in the issue's or pull request's conversation.
    Issue(Box<IssueComment>),
}

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewCommentAction {
    /// `modal.submit` or Post Comment: post the text as it is.
    Submit,
    /// `modal.close` or Cancel: close with nothing, cancelling a request in flight.
    Close,
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentField {
    Editor,
    Post,
    Cancel,
}

impl CommentField {
    fn button(self) -> Option<Pressed> {
        match self {
            CommentField::Post => Some(Pressed::Submit),
            CommentField::Cancel => Some(Pressed::Cancel),
            CommentField::Editor => None,
        }
    }
}

/// Where each part goes, in rows from the interior's top.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rows {
    /// The heading, the quote and the rule (Python's `ReplyingToContainer`), when replying.
    reply: Option<Range<u16>>,
    editor_label: u16,
    editor: Range<u16>,
    preview_label: u16,
    preview: Range<u16>,
    /// The buttons themselves, centered in their holder.
    buttons: Range<u16>,
}

impl Rows {
    /// Textual's vertical layout of the container, `height` rows inside its border: the editor and the preview at
    /// 40% each, the labels a row each, and what's left shared by the `1fr` reply container (when there is one) and
    /// the button holder. Each part ends where the running total of the exact sizes, rounded down, does, as Textual
    /// places fractional sizes.
    fn new(height: u16, reply: bool) -> Self {
        let total = f64::from(height);
        let boxed = BOX_SHARE * total;
        let shares = if reply { 2.0 } else { 1.0 };
        let fr = (total - 2.0 - 2.0 * boxed).max(0.0) / shares;
        let sizes = [if reply { fr } else { 0.0 }, 1.0, boxed, 1.0, boxed, fr];
        let mut edges = [0u16; 7];
        let mut sum = 0.0;
        for (index, size) in sizes.iter().enumerate() {
            sum += size;
            // Rows of a screen: small, non-negative, and floored as Textual floors them.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let edge = (sum.floor() as u16).min(height);
            edges[index + 1] = edge;
        }
        let holder = edges[5]..edges[6];
        let holder_rows = holder.end - holder.start;
        let top = holder.start + holder_rows.saturating_sub(ButtonRow::FLUSH_HEIGHT) / 2;
        Self {
            reply: reply.then_some(edges[0]..edges[1]),
            editor_label: edges[1],
            editor: edges[2]..edges[3],
            preview_label: edges[3],
            preview: edges[4]..edges[5],
            buttons: top..top + ButtonRow::FLUSH_HEIGHT,
        }
    }

    fn of(&self, field: CommentField) -> Range<u16> {
        match field {
            CommentField::Editor => self.editor.clone(),
            CommentField::Post | CommentField::Cancel => self.buttons.clone(),
        }
    }
}

/// The New comment dialog for issue or pull request `number` of `repo`. Its output is what it posted, or `None` when
/// it was closed.
#[derive(Debug)]
pub struct NewCommentDialog {
    repo: Repository,
    number: u64,
    target: ReplyTarget,
    /// The heading's line and its quote, parsed once.
    heading: Option<(String, Markdown)>,
    ring: FocusRing<CommentField>,
    editor: TextEditor,
    /// The preview, re-parsed at the next draw after the text changed.
    preview: Markdown,
    preview_stale: bool,
    buttons: ButtonRow,
    scroll: FormScroll,
    /// The post request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<PostedComment>>,
}

impl NewCommentDialog {
    /// The dialog for a comment on issue or pull request `number` of `repo`, answering `target`, with an empty
    /// editor focused.
    pub fn new(repo: Repository, number: u64, target: ReplyTarget) -> Self {
        let heading = target.heading().map(|(line, quote)| (line, Markdown::parse(quote)));
        Self {
            repo,
            number,
            target,
            heading,
            ring: FocusRing::new([CommentField::Editor, CommentField::Post, CommentField::Cancel]),
            editor: TextEditor::new(EditorVariant::Plain, "")
                .with_border(EditorBorder::Blank)
                .with_padding(EditorPadding { top: 1, horizontal: 0 }),
            preview: Markdown::default(),
            preview_stale: false,
            buttons: ButtonRow::flush("Post Comment"),
            scroll: FormScroll::without_scrollbar(),
            pending: None,
            output: None,
        }
    }

    /// The text as typed.
    pub fn text(&self) -> String {
        self.editor.text()
    }

    /// Whether a post request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: NewCommentAction) -> Action {
        Action::Dialog(DialogAction::NewComment(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => NewCommentAction::Submit,
            Pressed::Cancel => NewCommentAction::Close,
        })
    }

    /// The layout as of the last draw.
    fn rows(&self) -> Rows {
        Rows::new(self.scroll.content().height, self.target != ReplyTarget::None)
    }

    /// Moves focus with `f`: a field that gains it is scrolled into view, and the editor starts a new undo batch.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<CommentField>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(field) = self.ring.focused().filter(|&field| Some(field) != before) else {
            return;
        };
        if field == CommentField::Editor {
            self.editor.focus_gained();
        }
        self.scroll.reveal(self.rows().of(field));
    }

    /// Python's `post_comment`: the text as it is, through the issue comments endpoint. A second submit waits for the
    /// first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring a New comment submit: one is in flight");
            return;
        }
        let request = match &self.target {
            ReplyTarget::None | ReplyTarget::IssueComment(_) => GithubRequest::CreateIssueComment {
                repo: Box::new(self.repo.clone()),
                number: self.number,
                body: self.editor.text(),
            },
        };
        self.pending = Some(cx.request(request));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }

    /// A wheel notch at `(column, row)`: the editor scrolls if it's under the pointer and can, else the interior does.
    fn wheel(&mut self, wheel: Wheel, column: u16, row: u16) {
        let rows = match wheel {
            Wheel::Up => -WHEEL_ROWS.cast_signed(),
            Wheel::Down => WHEEL_ROWS.cast_signed(),
            Wheel::Left | Wheel::Right => return,
        };
        let over_editor = self.scroll.shows_at(self.rows().editor, column, row);
        if !(over_editor && self.editor.scroll(rows)) {
            self.scroll.wheel(wheel);
        }
    }

    /// The reply heading, its quote and its rule, `width` cells wide, as lines; the reply's rows cut them off.
    fn reply_lines(&self, width: u16, theme: &Theme, text: Style) -> Vec<Line<'static>> {
        let Some((line, quote)) = &self.heading else {
            return Vec::new();
        };
        let mut lines = vec![Line::styled(line.clone(), text)];
        lines.extend(quote.layout(width, theme).lines);
        // The quote's paragraph margin, then the `Rule` with its `margin: 1 0`.
        lines.push(Line::default());
        lines.push(Line::styled(
            "─".repeat(usize::from(width)),
            Style::new().fg(theme.color(theme.palette.secondary, theme.background())),
        ));
        lines
    }
}

/// Draws `lines` from the top of `area`, one per row, cut off at its edges. Their styles go over what's there.
fn draw_lines(buf: &mut Buffer, area: Rect, lines: &[Line<'_>]) {
    for (line, y) in lines.iter().zip(area.top()..area.bottom()) {
        buf.set_line(area.x, y, line, area.width);
    }
}

impl Component for NewCommentDialog {
    type Action = NewCommentAction;

    fn bindings(&self) -> &'static [BindingId] {
        NEW_COMMENT_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(NewCommentAction::Submit)),
            BindingId::ModalClose => Some(Self::action(NewCommentAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.ring.is_focused(CommentField::Editor) && self.editor.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        if !self.ring.is_focused(CommentField::Editor) {
            return None;
        }
        match self.editor.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
            EditorOutcome::Changed => {
                self.preview_stale = true;
                Some(Action::Consumed)
            }
            EditorOutcome::Copied { text, changed } => {
                self.preview_stale |= changed;
                Some(Action::Copy(text))
            }
            EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // The fields are drawn offscreen and copied with the scroll offset: their mouse coordinates are the form's.
        let moved = self.scroll.to_content(event);
        if self.editor.captured_mouse(moved) {
            return Some(Action::Consumed);
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&moved, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    let field = match button {
                        Pressed::Submit => CommentField::Post,
                        Pressed::Cancel => CommentField::Cancel,
                    };
                    self.refocus(|ring| {
                        ring.focus(field);
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.wheel(wheel, event.column, event.row);
        } else if event.kind == MouseEventKind::Down(MouseButton::Left)
            && self.scroll.shows_at(self.rows().editor, event.column, event.row)
        {
            self.refocus(|ring| {
                ring.focus(CommentField::Editor);
            });
            self.editor.press(moved.column, moved.row);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: NewCommentAction, cx: &mut Cx<'_>) {
        match action {
            NewCommentAction::Submit => self.submit(cx),
            NewCommentAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let text = Style::new().fg(theme.color(theme.palette.foreground, under));
        let fill = text.bg(theme.paint(under));
        if std::mem::take(&mut self.preview_stale) {
            self.preview = Markdown::parse(&self.editor.text());
        }
        // The interior runs past the screen's bottom: what shows is above the footer row.
        let screen = frame.area();
        let window = area.intersection(Rect {
            height: screen.height.saturating_sub(1),
            ..screen
        });
        let rows = Rows::new(area.height, self.target != ReplyTarget::None);
        let reply = self.reply_lines(window.width, theme, text);
        let focused = self.ring.focused();
        let (editor, preview, buttons) = (&mut self.editor, &self.preview, &mut self.buttons);
        self.scroll
            .render(frame.buffer_mut(), window, theme, fill, area.height, |buf, content| {
                let rect = |range: Range<u16>| Rect {
                    y: content.y + range.start,
                    height: range.end - range.start,
                    ..content
                };
                if let Some(range) = rows.reply.clone() {
                    draw_lines(buf, rect(range), &reply);
                }
                let label = |y: u16| rect(y..y + 1);
                draw_lines(buf, label(rows.editor_label), &[Line::styled("New comment", text)]);
                editor.render(
                    buf,
                    rect(rows.editor.clone()),
                    theme,
                    under,
                    focused == Some(CommentField::Editor),
                );
                draw_lines(buf, label(rows.preview_label), &[Line::styled("Preview", text)]);
                render_preview(buf, content, rows.preview.clone(), preview, theme);
                buttons.render(
                    buf,
                    rect(rows.buttons.clone()),
                    theme,
                    focused.and_then(CommentField::button),
                );
            });
    }
}

/// The preview: the text as markdown, inside the box's blank border and below its top padding, with no side padding
/// (`padding-top: 1` leaves Textual's other edges at 0).
fn render_preview(buf: &mut Buffer, content: Rect, rows: Range<u16>, preview: &Markdown, theme: &Theme) {
    let area = Rect {
        x: content.x + 1,
        y: content.y + rows.start + BOX_TOP,
        width: content.width.saturating_sub(2),
        height: (rows.end - rows.start).saturating_sub(BOX_TOP + 1),
    };
    draw_lines(buf, area, &preview.layout_unpadded(area.width, theme).lines);
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

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn frame(&self, area: Rect) -> Rect {
        frame_rect_with_margin(area, SIZE, MARGIN)
    }

    fn take_output(&mut self) -> Option<Option<PostedComment>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(CommentField::Editor) => FieldKind::TextEditor,
            Some(CommentField::Post | CommentField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(CommentField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            // `consumes_key` already gave the editor every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.editor.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.ring.is_focused(CommentField::Editor).then(|| {
            if self.editor.handle_paste(text, cx.now()) == EditorOutcome::Changed {
                self.preview_stale = true;
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
