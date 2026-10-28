//! One open dialog on the stack, with its type erased so dialogs of every kind share one `Vec`.

use std::fmt;

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use super::{Dialog, DialogAction, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, Owner, RenderCx, ScreenKind};
use crate::effect::{GitResponse, GithubResponse, RequestId, TimerId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Palette, ThemeColor};

/// An open dialog and what to do with its output. Made by [`Cx::open_dialog`](crate::component::Cx::open_dialog).
pub struct ModalEntry {
    dialog: Box<dyn AnyDialog>,
    /// The frame's interior in the last frame drawn, for mouse events. Empty until then.
    pub(crate) interior: Rect,
}

impl ModalEntry {
    pub(crate) fn new<D: Dialog>(dialog: D, reply: impl FnOnce(D::Output) -> Action + Send + 'static) -> Self {
        debug_assert!(
            dialog.bindings().starts_with(super::MODAL_SCREEN_BINDINGS),
            "a dialog's bindings start with MODAL_SCREEN_BINDINGS"
        );
        Self {
            dialog: Box::new(Open {
                dialog,
                reply: Some(Box::new(reply)),
            }),
            interior: Rect::default(),
        }
    }

    /// Which modal it is.
    pub fn kind(&self) -> ModalKind {
        self.dialog.kind()
    }

    /// The dialog's links: the focused field's, then its screen link.
    pub(crate) fn chain(&self) -> Vec<ChainLink> {
        let kind = self.kind();
        let mut chain = self.dialog.field_chain();
        chain.push(ChainLink::new(
            Owner::Screen(ScreenKind::Modal(kind)),
            self.dialog.bindings(),
        ));
        chain
    }

    pub(crate) fn on_binding(&mut self, owner: Owner, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match owner {
            Owner::DialogField(_, field) => self.dialog.on_field_binding(field, id, cx),
            // Nothing in a dialog can be selected yet, so the copy never applies; with the app link cut off, `ctrl+c`
            // then does nothing at all.
            Owner::Screen(_) if id == BindingId::TextualCopyText => None,
            Owner::Screen(_) if id == BindingId::TextualFocusNext => {
                self.dialog.move_focus(FocusMove::Next);
                Some(Action::Consumed)
            }
            Owner::Screen(_) if id == BindingId::TextualFocusPrevious => {
                self.dialog.move_focus(FocusMove::Previous);
                Some(Action::Consumed)
            }
            _ => self.dialog.on_binding(id, cx),
        }
    }

    pub(crate) fn binding_state(&self, owner: Owner, id: BindingId, cx: &Cx<'_>) -> BindingState {
        match owner {
            Owner::DialogField(_, field) => self.dialog.field_binding_state(field, id, cx),
            _ => self.dialog.binding_state(id, cx),
        }
    }

    pub(crate) fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.dialog.consumes_key(key, keymap)
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_key(key, cx)
    }

    /// A mouse event, which only reaches the dialog inside its interior.
    pub(crate) fn handle_mouse(&mut self, event: MouseEvent, cx: &Cx<'_>) -> Option<Action> {
        let interior = self.interior;
        interior
            .contains((event.column, event.row).into())
            .then(|| self.dialog.handle_mouse(event, interior, cx))
            .flatten()
    }

    /// Whether a field of the dialog has the mouse ([`Dialog::captures_mouse`]).
    pub(crate) fn captures_mouse(&self) -> bool {
        self.dialog.captures_mouse()
    }

    /// A mouse event while a field of the dialog has the mouse: it reaches the dialog wherever it is, even outside
    /// the interior, as Textual routes every mouse event to the widget that captured the mouse.
    pub(crate) fn captured_mouse(&mut self, event: MouseEvent, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_mouse(event, self.interior, cx)
    }

    pub(crate) fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_paste(text, cx)
    }

    pub(crate) fn update(&mut self, action: DialogAction, cx: &mut Cx<'_>) {
        self.dialog.update(action, cx);
    }

    pub(crate) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        self.dialog.on_response(id, response, cx)
    }

    pub(crate) fn on_git_response(
        &mut self,
        id: RequestId,
        response: GitResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GitResponse> {
        self.dialog.on_git_response(id, response, cx)
    }

    pub(crate) fn on_timer(&mut self, id: TimerId, cx: &mut Cx<'_>) -> bool {
        self.dialog.on_timer(id, cx)
    }

    pub(crate) fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        self.dialog.on_cache_read(id, result, cx)
    }

    pub(crate) fn frame(&self, area: Rect) -> Rect {
        self.dialog.frame(area)
    }

    pub(crate) fn background(&self, palette: &Palette) -> ThemeColor {
        self.dialog.background(palette)
    }

    pub(crate) fn border(&self, palette: &Palette) -> ThemeColor {
        self.dialog.border(palette)
    }

    pub(crate) fn shows_footer(&self) -> bool {
        self.dialog.shows_footer()
    }

    pub(crate) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.dialog.render(frame, area, cx);
    }

    /// The opener's reply, once the dialog has an output. The entry is spent after that.
    pub(crate) fn take_reply(&mut self) -> Option<Action> {
        self.dialog.take_reply()
    }
}

impl fmt::Debug for ModalEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModalEntry")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

/// A dialog with its reply, behind the object-safe interface the stack uses.
struct Open<D: Dialog> {
    dialog: D,
    /// Taken when the output is.
    reply: Option<Box<dyn FnOnce(D::Output) -> Action + Send>>,
}

/// [`Dialog`] and [`Component`] with the associated types erased.
trait AnyDialog: Send {
    fn kind(&self) -> ModalKind;
    fn bindings(&self) -> &'static [BindingId];
    fn field_chain(&self) -> Vec<ChainLink>;
    fn on_binding(&mut self, id: BindingId, cx: &Cx<'_>) -> Option<Action>;
    fn move_focus(&mut self, direction: FocusMove);
    fn on_field_binding(&mut self, field: super::FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action>;
    fn binding_state(&self, id: BindingId, cx: &Cx<'_>) -> BindingState;
    fn field_binding_state(&self, field: super::FieldKind, id: BindingId, cx: &Cx<'_>) -> BindingState;
    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool;
    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action>;
    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, cx: &Cx<'_>) -> Option<Action>;
    fn captures_mouse(&self) -> bool;
    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action>;
    fn update(&mut self, action: DialogAction, cx: &mut Cx<'_>);
    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse>;
    fn on_git_response(&mut self, id: RequestId, response: GitResponse, cx: &mut Cx<'_>) -> Option<GitResponse>;
    fn on_timer(&mut self, id: TimerId, cx: &mut Cx<'_>) -> bool;
    fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>>;
    fn frame(&self, area: Rect) -> Rect;
    fn background(&self, palette: &Palette) -> ThemeColor;
    fn border(&self, palette: &Palette) -> ThemeColor;
    fn shows_footer(&self) -> bool;
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>);
    fn take_reply(&mut self) -> Option<Action>;
}

impl<D: Dialog> AnyDialog for Open<D> {
    fn kind(&self) -> ModalKind {
        self.dialog.kind()
    }

    fn bindings(&self) -> &'static [BindingId] {
        self.dialog.bindings()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        self.dialog.field_chain()
    }

    fn on_binding(&mut self, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.on_binding(id, cx)
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.dialog.move_focus(direction);
    }

    fn on_field_binding(&mut self, field: super::FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.on_field_binding(field, id, cx)
    }

    fn binding_state(&self, id: BindingId, cx: &Cx<'_>) -> BindingState {
        self.dialog.binding_state(id, cx)
    }

    fn field_binding_state(&self, field: super::FieldKind, id: BindingId, cx: &Cx<'_>) -> BindingState {
        self.dialog.field_binding_state(field, id, cx)
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.dialog.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_key(key, cx)
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_mouse(event, area, cx)
    }

    fn captures_mouse(&self) -> bool {
        self.dialog.captures_mouse()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.dialog.handle_paste(text, cx)
    }

    fn update(&mut self, action: DialogAction, cx: &mut Cx<'_>) {
        if let Some(action) = self.dialog.own_action(action) {
            self.dialog.update(action, cx);
        } else {
            tracing::debug!("Dropping an action that {:?} doesn't take", self.dialog.kind());
        }
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        self.dialog.on_response(id, response, cx)
    }

    fn on_git_response(&mut self, id: RequestId, response: GitResponse, cx: &mut Cx<'_>) -> Option<GitResponse> {
        self.dialog.on_git_response(id, response, cx)
    }

    fn on_timer(&mut self, id: TimerId, cx: &mut Cx<'_>) -> bool {
        self.dialog.on_timer(id, cx)
    }

    fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        self.dialog.on_cache_read(id, result, cx)
    }

    fn frame(&self, area: Rect) -> Rect {
        self.dialog.frame(area)
    }

    fn background(&self, palette: &Palette) -> ThemeColor {
        self.dialog.background(palette)
    }

    fn border(&self, palette: &Palette) -> ThemeColor {
        self.dialog.border(palette)
    }

    fn shows_footer(&self) -> bool {
        self.dialog.shows_footer()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.dialog.render(frame, area, cx);
    }

    fn take_reply(&mut self) -> Option<Action> {
        let output = self.dialog.take_output()?;
        let Some(reply) = self.reply.take() else {
            tracing::debug!("{:?} finished twice; dropping its second output", self.dialog.kind());
            return None;
        };
        Some(reply(output))
    }
}
