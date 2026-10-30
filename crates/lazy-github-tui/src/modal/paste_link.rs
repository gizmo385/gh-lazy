//! Paste link (`link.paste`): "Paste a web GitHub link:", a "Link:" input checked against the GitHub link patterns,
//! and Open/Cancel. Open looks the link up and hands what it points at to the main screen. Python's `PasteLinkModal`.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::links::{LinkSubject, looks_like_github_url, parse_github_url};
use ratatui::Frame;
use ratatui::layout::Rect;

use super::single_input::{Areas, CheckedInput};
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::text_input::{InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The frame's outer size, border included: `width: 80; max-height: 20` on a container that's `1fr` tall.
const SIZE: (u16, u16) = (80, 20);

const HEADING: &str = "Paste a web GitHub link:";
const LABEL: &str = "Link:";
const PLACEHOLDER: &str = "https://github.com/...";

/// Paste link's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteLinkAction {
    /// `modal.submit` or Open: look the link up.
    Submit,
    /// `modal.close` or Cancel: close with nothing, cancelling a lookup in flight.
    Close,
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteLinkField {
    Input,
    Open,
    Cancel,
}

impl PasteLinkField {
    fn button(self) -> Option<Pressed> {
        match self {
            PasteLinkField::Open => Some(Pressed::Submit),
            PasteLinkField::Cancel => Some(Pressed::Cancel),
            PasteLinkField::Input => None,
        }
    }
}

/// The Paste link dialog. Its output is what the link points at, or `None` when it was closed.
#[derive(Debug)]
pub struct PasteLinkDialog {
    ring: FocusRing<PasteLinkField>,
    input: CheckedInput,
    buttons: ButtonRow,
    /// The lookup in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<LinkSubject>>,
}

impl Default for PasteLinkDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl PasteLinkDialog {
    /// An empty dialog with its input focused.
    pub fn new() -> Self {
        Self {
            ring: FocusRing::new([PasteLinkField::Input, PasteLinkField::Open, PasteLinkField::Cancel]),
            input: CheckedInput::new(TextInput::new(PLACEHOLDER), looks_like_github_url),
            buttons: ButtonRow::new("Open"),
            pending: None,
            output: None,
        }
    }

    /// The dialog opened by a paste on the main screen (decision `paste-opens-link-modal`): `text` goes into the
    /// input as the input's own paste would put it there (its first line), selected, so typing replaces it, and
    /// checked at once. Nothing is submitted.
    pub fn with_text(text: &str) -> Self {
        let mut dialog = Self::new();
        dialog.input.paste(text);
        dialog.input.validate();
        dialog.input.input_mut().select_all();
        dialog
    }

    /// The input's text.
    pub fn text(&self) -> &str {
        self.input.input().value()
    }

    /// Whether the input shows the red border of a failed check.
    pub fn is_invalid(&self) -> bool {
        self.input.input().is_invalid()
    }

    /// Whether a lookup is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether Open and Cancel are disabled.
    pub fn buttons_disabled(&self) -> bool {
        self.buttons.button(Pressed::Submit).disabled()
    }

    fn action(action: PasteLinkAction) -> Action {
        Action::Dialog(DialogAction::PasteLink(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => PasteLinkAction::Submit,
            Pressed::Cancel => PasteLinkAction::Close,
        })
    }

    /// Moves focus by `f`, checking the input when it loses focus and selecting its text when it gains it.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<PasteLinkField>)) {
        let had_input = self.ring.is_focused(PasteLinkField::Input);
        f(&mut self.ring);
        self.input
            .focus_moved(had_input, self.ring.is_focused(PasteLinkField::Input));
    }

    /// Disables or enables Open and Cancel. Disabling a focused button blurs it, and enabling doesn't give focus
    /// back, so nothing in the dialog has focus until `tab` or a click, as in Textual.
    fn set_buttons_disabled(&mut self, disabled: bool) {
        for (field, button) in [
            (PasteLinkField::Open, Pressed::Submit),
            (PasteLinkField::Cancel, Pressed::Cancel),
        ] {
            self.refocus(|ring| ring.set_disabled(field, disabled));
            self.buttons.button_mut(button).set_disabled(disabled);
        }
    }

    fn submit(&mut self, cx: &mut Cx<'_>) {
        // A second submit waits for the first (Py bug B104: Python queues every key until GitHub answers).
        if self.pending.is_some() {
            tracing::debug!("Ignoring a paste-link submit: a lookup is in flight");
            return;
        }
        // Python disables the buttons before it parses the link, so even an unknown link blurs a focused Open.
        self.set_buttons_disabled(true);
        match parse_github_url(self.text(), true) {
            None => {
                self.set_buttons_disabled(false);
                cx.notify(Toast::info("Unknown link"));
            }
            Some(link) => self.pending = Some(cx.request(GithubRequest::ResolveLink(link))),
        }
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

impl Component for PasteLinkDialog {
    type Action = PasteLinkAction;

    fn bindings(&self) -> &'static [BindingId] {
        dialog_bindings![ModalSubmit, ModalClose]
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(PasteLinkAction::Submit)),
            BindingId::ModalClose => Some(Self::action(PasteLinkAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.ring.is_focused(PasteLinkField::Input) && self.input.input().consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        // `enter` in the input only checks it (Python doesn't handle `Input.Submitted`); opening is `modal.submit`
        // or Open.
        match self
            .input
            .edit(|input| input.handle_key(&chord, cx.keymap(), cx.clipboard()))
        {
            InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
            InputOutcome::Copied(text) => Some(Action::Copy(text)),
            InputOutcome::Ignored => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // A drag that started on the input, wherever the pointer is now.
        if self.input.input_mut().captured_mouse(event) {
            return Some(Action::Consumed);
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    let field = match button {
                        Pressed::Submit => PasteLinkField::Open,
                        Pressed::Cancel => PasteLinkField::Cancel,
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
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Some(Action::Consumed);
        }
        let areas = Areas::new(area, false);
        if areas.input.contains((event.column, event.row).into()) {
            self.refocus(|ring| {
                ring.focus(PasteLinkField::Input);
            });
            self.input.input_mut().press(areas.input, event.column);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: PasteLinkAction, cx: &mut Cx<'_>) {
        match action {
            PasteLinkAction::Submit => self.submit(cx),
            PasteLinkAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        // A dialog taller than the terminal runs off its bottom edge, which nothing may be drawn past.
        let areas = Areas::new(area, false).clipped(frame.area());
        let buf = frame.buffer_mut();
        areas.render_text(buf, HEADING, LABEL, theme, under);
        self.input.input_mut().render(
            buf,
            areas.input,
            theme,
            under,
            self.ring.is_focused(PasteLinkField::Input),
        );
        let focused = self.ring.focused().and_then(PasteLinkField::button);
        self.buttons.render(buf, areas.buttons, theme, focused);
    }
}

impl Dialog for PasteLinkDialog {
    type Output = Option<LinkSubject>;

    fn kind(&self) -> ModalKind {
        ModalKind::PasteLink
    }

    fn own_action(&self, action: DialogAction) -> Option<PasteLinkAction> {
        match action {
            DialogAction::PasteLink(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<Option<LinkSubject>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(PasteLinkField::Input) => FieldKind::TextInput,
            Some(PasteLinkField::Open | PasteLinkField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(PasteLinkField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            // `consumes_key` already gave the input every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.input.input().is_selecting()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        self.ring.is_focused(PasteLinkField::Input).then(|| {
            self.input.paste(text);
            Action::Consumed
        })
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::ResolveLink(result) = response else {
            return Some(response);
        };
        self.pending = None;
        self.set_buttons_disabled(false);
        match result {
            Ok(Some(subject)) => self.output = Some(Some(subject)),
            Ok(None) => cx.notify(lookup_failed()),
            Err(error) => {
                tracing::debug!("Resolving the pasted link failed: {error}");
                cx.notify(lookup_failed());
            }
        }
        None
    }
}

/// The toast for a link whose lookup failed, however it failed.
fn lookup_failed() -> Toast {
    Toast::error("Could not resolve link!").with_title("GitHub lookup failed")
}

#[cfg(test)]
mod tests;
