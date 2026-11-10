//! Add Comment (`diff.add_comment` on a line of the diff body), on the compose sheet (decision compose-dialogs): an
//! `Add Comment` heading, the diff line being commented on quoted on one row, then the comment under its
//! `Write  Preview` tabs. It sends nothing: it closes with the comment's text, which the diff viewer keeps as a pending
//! comment until the review is submitted. Python's `AddCommentModal`, whose fixed 80-wide layout with a read-only
//! input for the line and Add Comment / Cancel buttons the sheet replaces.
//!
//! The sheet is 80 wide and keeps Python's `thick $accent` border on `$surface`.

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use super::form::FormScroll;
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::compose::{
    BodyMode, BodyMouse, COMPOSE_BINDINGS, ComposeAction, ComposeBody, ComposeLayout, ComposeParts, SheetGeometry,
    SheetPlacement, quote_line, sheet_geometry,
};
use crate::components::text_editor::{EditorOutcome, EditorVariant};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Roles, Theme, ThemeColor};
use crate::toast::Toast;

/// Python's `width: 80`, border included: the sheet's widest.
const MAX_WIDTH: u16 = 80;
/// A comment's body is at least this tall.
const BODY_MIN_ROWS: u16 = 3;
/// Python's title.
const HEADING: &str = "Add Comment";
/// The text a whitespace-only comment toasts.
const EMPTY_COMMENT: &str = "Comment cannot be empty!";

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddCommentAction {
    /// One of the sheet's shared keys.
    Compose(ComposeAction),
}

/// Where the dialog's focus is: the body, its only field, or the sheet itself after `escape` left the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Body,
    Sheet,
}

/// Add Comment for one diff line. Its output is the comment as typed, or `None` when it was closed.
#[derive(Debug)]
pub struct AddCommentDialog {
    /// The line commented on, as the diff shows it: its `+`, `-` or space marker first, trimmed.
    line: String,
    body: ComposeBody,
    focus: Focus,
    /// `compose.expand`: the sheet fills the screen above the footer.
    expanded: bool,
    placement: SheetPlacement,
    /// The interior, which scrolls (without a bar) only on a screen too short for even the body's minimum.
    scroll: FormScroll,
    /// Scroll the body into view at the next draw, after an edit or a focus change.
    reveal_body: bool,
    /// The theme of the last draw, which [`Dialog::frame`] measures the preview in.
    theme: Option<Theme>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<String>>,
}

impl AddCommentDialog {
    /// The dialog for the line whose diff text (its `+`, `-` or space marker first, trimmed) is `line_text`, with an
    /// empty body focused.
    pub fn new(line_text: impl Into<String>) -> Self {
        Self {
            line: line_text.into(),
            body: ComposeBody::new(EditorVariant::Plain, "Write a review comment (markdown)", BODY_MIN_ROWS),
            focus: Focus::Body,
            expanded: false,
            placement: SheetPlacement::default(),
            scroll: FormScroll::without_scrollbar(),
            reveal_body: false,
            theme: None,
            output: None,
        }
    }

    /// The line quoted.
    pub fn line_text(&self) -> &str {
        &self.line
    }

    /// The comment as typed.
    pub fn text(&self) -> String {
        self.body.text()
    }

    fn action(action: ComposeAction) -> Action {
        Action::Dialog(DialogAction::AddComment(AddCommentAction::Compose(action)))
    }

    /// The sheet's background, which its text and markdown sit on.
    fn under(&self, theme: &Theme) -> Paint {
        self.background(theme.roles()).over(theme.background())
    }

    /// The sheet's limits on `screen` and its parts' rows, measured in `theme`. The quote is always one row.
    fn sheet(&self, screen: Rect, theme: &Theme) -> (SheetGeometry, ComposeLayout) {
        let geometry = sheet_geometry(screen, MAX_WIDTH, self.expanded);
        let parts = ComposeParts {
            quote: 1,
            body_needed: self.body.rows_needed(geometry.content_width, theme, self.under(theme)),
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

    /// Python's `_submit_comment`: a comment that's only whitespace is refused with a toast, and the sheet stays.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        let text = self.body.text();
        if text.trim().is_empty() {
            cx.notify(Toast::warning(EMPTY_COMMENT));
            return;
        }
        self.output = Some(Some(text));
    }
}

impl Component for AddCommentDialog {
    type Action = AddCommentAction;

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

    fn update(&mut self, action: AddCommentAction, cx: &mut Cx<'_>) {
        let AddCommentAction::Compose(action) = action;
        match action {
            ComposeAction::Submit => self.submit(cx),
            ComposeAction::Close => self.output = Some(None),
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
        let under = self.under(theme);
        let text = Style::new().fg(theme.color(theme.roles().text.plain, under));
        if std::mem::take(&mut self.reveal_body) {
            self.scroll.reveal(layout.body.clone());
        }
        let focused = self.focus == Focus::Body;
        let (line, body) = (&self.line, &mut self.body);
        // The preview's links drawn, in rows from the interior's top and columns from its left edge.
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
                let heading = row(layout.heading, 1);
                buf.set_span(heading.x, heading.y, &Span::styled(HEADING, text), heading.width);
                quote_line(buf, row(layout.quote.start, 1), line, theme, under);
                let rect = row(layout.body.start, layout.body_rows());
                let spans = body.render(buf, row(layout.tabs, 1), rect, theme, under, focused);
                links.extend(spans.into_iter().map(|mut span| {
                    span.row += usize::from(rect.y - content.y);
                    let shift = rect.x - content.x;
                    span.columns = span.columns.start + shift..span.columns.end + shift;
                    span
                }));
            },
        );
        // As New comment's, the preview's links open on a click.
        if let Some(sink) = cx.links {
            self.scroll.record_links(sink, &links);
        }
    }
}

impl Dialog for AddCommentDialog {
    type Output = Option<String>;

    fn kind(&self) -> ModalKind {
        ModalKind::AddComment
    }

    fn own_action(&self, action: DialogAction) -> Option<AddCommentAction> {
        match action {
            DialogAction::AddComment(action) => Some(action),
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

    /// `background: $surface`.
    fn background(&self, roles: &Roles) -> ThemeColor {
        roles.surface.pane
    }

    /// `border: thick $accent`.
    fn border(&self, roles: &Roles) -> ThemeColor {
        roles.border.emphasis
    }

    fn take_output(&mut self) -> Option<Option<String>> {
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
}

#[cfg(test)]
mod tests;
