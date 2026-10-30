//! The lookup dialog: a heading, a labeled input, an optional "Continue tracking this repo?" switch and a
//! Lookup/Cancel row, which looks one thing up on GitHub and hands it to the opener. Python's
//! `LookupRepositoryModal`, `LookupIssueModal` and `LookupPullRequestModal`, which differ only in what a
//! [`LookupKind`] says.

use std::marker::PhantomData;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::Repository;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::single_input::{Areas, CheckedInput};
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::switch::Switch;
use crate::components::text_input::{InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The frame's outer size, border included: `width: 60; max-height: 25` on a container that's `1fr` tall.
const SIZE: (u16, u16) = (60, 25);

/// The lookup dialog's screen bindings.
const LOOKUP_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// Which lookup a dialog is, for [`ModalKind::Lookup`] and for routing [`DialogAction::Lookup`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LookupTarget {
    /// Lookup repository (`repositories.lookup`).
    Repository,
    /// Lookup issue (`issue.lookup`).
    Issue,
    /// Lookup pull request (`pull_request.lookup`).
    PullRequest,
}

/// A lookup dialog's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupAction {
    /// `modal.submit` or the Lookup button: look the text up.
    Submit,
    /// `modal.close` or Cancel: close with nothing, cancelling a lookup in flight.
    Close,
}

/// What one lookup looks up: its texts, how it checks the input, its request and its result.
pub trait LookupKind: Send + 'static {
    /// What the opener gets: `Repository`, `Issue`, `FullPullRequest`.
    type Found: Send + 'static;
    /// Which lookup this is.
    const TARGET: LookupTarget;
    /// The markdown H1 at the top: "Search for a repository by name:".
    const HEADING: &'static str;
    /// The bold label over the input: "Repository (owner/name):".
    const LABEL: &'static str;
    /// The input's placeholder: "Repository name".
    const PLACEHOLDER: &'static str;
    /// Whether the dialog has the "Continue tracking this repo?" switch.
    const TRACK_SWITCH: bool = false;
    /// What the input may hold, if it's restricted: [`number::can_become_number`] for a `type="number"` input.
    const FILTER: Option<fn(&str) -> bool> = None;

    /// The live check behind the red border.
    fn is_valid(text: &str) -> bool;

    /// The request for `text`, or the toast for text that can't be looked up.
    fn request(text: &str, cx: &Cx<'_>) -> Result<GithubRequest, Toast>;

    /// Its answer, or the toast for "not found"; any other response is handed back.
    fn found(response: GithubResponse) -> Result<Result<Self::Found, Toast>, GithubResponse>;

    /// After a successful lookup, before the dialog closes (Repository: the tracking write).
    fn on_found(_found: &Self::Found, _submitted: &str, _track: bool, _cx: &mut Cx<'_>) {}
}

/// A lookup dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupField {
    Input,
    /// The "Continue tracking this repo?" switch, with [`LookupKind::TRACK_SWITCH`].
    Track,
    Submit,
    Cancel,
}

impl LookupField {
    fn button(self) -> Option<Pressed> {
        match self {
            LookupField::Submit => Some(Pressed::Submit),
            LookupField::Cancel => Some(Pressed::Cancel),
            LookupField::Input | LookupField::Track => None,
        }
    }
}

/// The lookup dialog for kind `K`. Its output is what was found, or `None` when it was closed.
#[derive(Debug)]
pub struct LookupDialog<K: LookupKind> {
    ring: FocusRing<LookupField>,
    input: CheckedInput,
    track: Option<Switch>,
    buttons: ButtonRow,
    /// The request in flight and the text it was for.
    pending: Option<(RequestId, String)>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<K::Found>>,
    _kind: PhantomData<fn() -> K>,
}

impl<K: LookupKind> Default for LookupDialog<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: LookupKind> LookupDialog<K> {
    /// An empty dialog with its input focused and the switch, if it has one, off.
    pub fn new() -> Self {
        let ring = if K::TRACK_SWITCH {
            FocusRing::new([
                LookupField::Input,
                LookupField::Track,
                LookupField::Submit,
                LookupField::Cancel,
            ])
        } else {
            FocusRing::new([LookupField::Input, LookupField::Submit, LookupField::Cancel])
        };
        Self {
            ring,
            input: CheckedInput::new(
                match K::FILTER {
                    Some(filter) => TextInput::new(K::PLACEHOLDER).with_filter(filter),
                    None => TextInput::new(K::PLACEHOLDER),
                },
                K::is_valid,
            ),
            track: K::TRACK_SWITCH.then(|| Switch::new(false)),
            buttons: ButtonRow::new("Lookup"),
            pending: None,
            output: None,
            _kind: PhantomData,
        }
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

    fn action(action: LookupAction) -> Action {
        Action::Dialog(DialogAction::Lookup(K::TARGET, action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => LookupAction::Submit,
            Pressed::Cancel => LookupAction::Close,
        })
    }

    /// Moves focus by `f`, checking the input when it loses focus and selecting its text when it gains it.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<LookupField>)) {
        let had_input = self.ring.is_focused(LookupField::Input);
        f(&mut self.ring);
        self.input
            .focus_moved(had_input, self.ring.is_focused(LookupField::Input));
    }

    fn submit(&mut self, cx: &mut Cx<'_>) {
        // A second submit waits for the first (Py bug B51: Python queues it behind the request instead).
        if self.pending.is_some() {
            tracing::debug!("Ignoring a lookup submit: one is in flight");
            return;
        }
        let text = self.input.input().value().to_owned();
        match K::request(&text, cx) {
            Ok(request) => {
                let id = cx.request(request);
                self.pending = Some((id, text));
            }
            Err(toast) => cx.notify(toast),
        }
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some((id, _)) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

impl<K: LookupKind> Component for LookupDialog<K> {
    type Action = LookupAction;

    fn bindings(&self) -> &'static [BindingId] {
        LOOKUP_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(LookupAction::Submit)),
            BindingId::ModalClose => Some(Self::action(LookupAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.ring.is_focused(LookupField::Input) && self.input.input().consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        // `enter` in the input only checks it; looking up is `modal.submit` or the Lookup button.
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
                        Pressed::Submit => LookupField::Submit,
                        Pressed::Cancel => LookupField::Cancel,
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
        let areas = Areas::new(area, self.track.is_some());
        if let (Some(track), Some(track_area)) = (self.track.as_mut(), areas.track)
            && track.handle_mouse(&event, track_area)
        {
            self.refocus(|ring| {
                ring.focus(LookupField::Track);
            });
            return Some(Action::Consumed);
        }
        if areas.input.contains((event.column, event.row).into()) {
            self.refocus(|ring| {
                ring.focus(LookupField::Input);
            });
            self.input.input_mut().press(areas.input, event.column);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: LookupAction, cx: &mut Cx<'_>) {
        match action {
            LookupAction::Submit => self.submit(cx),
            LookupAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let under = self.background(p).over(theme.background());
        let text = Style::new().fg(theme.color(p.foreground, under));
        // A dialog taller than the terminal runs off its bottom edge, which nothing may be drawn past.
        let areas = Areas::new(area, self.track.is_some()).clipped(frame.area());
        let buf = frame.buffer_mut();
        areas.render_text(buf, K::HEADING, K::LABEL, theme, under);
        self.input
            .input_mut()
            .render(buf, areas.input, theme, under, self.ring.is_focused(LookupField::Input));

        if let (Some(track), Some(label_area), Some(track_area)) = (self.track, areas.track_label, areas.track) {
            buf.set_stringn(
                label_area.x,
                label_area.y,
                "Continue tracking this repo?",
                usize::from(label_area.width),
                text,
            );
            track.render(buf, track_area, theme, under, self.ring.is_focused(LookupField::Track));
        }

        let focused = self.ring.focused().and_then(LookupField::button);
        self.buttons.render(buf, areas.buttons, theme, focused);
    }
}

impl<K: LookupKind> Dialog for LookupDialog<K> {
    type Output = Option<K::Found>;

    fn kind(&self) -> ModalKind {
        ModalKind::Lookup(K::TARGET)
    }

    fn own_action(&self, action: DialogAction) -> Option<LookupAction> {
        match action {
            DialogAction::Lookup(target, action) if target == K::TARGET => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<Option<K::Found>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(LookupField::Input) => FieldKind::TextInput,
            Some(LookupField::Track) => FieldKind::Switch,
            Some(LookupField::Submit | LookupField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(LookupField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            (FieldKind::Switch, BindingId::TextualSwitchToggle) => {
                if let Some(track) = &mut self.track {
                    track.toggle();
                }
                Some(Action::Consumed)
            }
            // `consumes_key` already gave the input every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.input.input().is_selecting()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        self.ring.is_focused(LookupField::Input).then(|| {
            self.input.paste(text);
            Action::Consumed
        })
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let Some((pending, submitted)) = self.pending.take() else {
            return Some(response);
        };
        if pending != id {
            self.pending = Some((pending, submitted));
            return Some(response);
        }
        match K::found(response) {
            Err(other) => {
                self.pending = Some((pending, submitted));
                Some(other)
            }
            Ok(Err(toast)) => {
                cx.notify(toast);
                None
            }
            Ok(Ok(found)) => {
                let track = self.track.is_some_and(|track| track.value());
                K::on_found(&found, &submitted, track, cx);
                self.output = Some(Some(found));
                None
            }
        }
    }
}

/// Lookup repository (`repositories.lookup`): an `owner/name`, looked up with `GET /repos/<owner>/<name>`, and
/// optionally added to `repositories.additional_repos_to_track`.
#[derive(Debug)]
pub enum RepoLookup {}

impl LookupKind for RepoLookup {
    type Found = Repository;
    const TARGET: LookupTarget = LookupTarget::Repository;
    const HEADING: &'static str = "Search for a repository by name:";
    const LABEL: &'static str = "Repository (owner/name):";
    const PLACEHOLDER: &'static str = "Repository name";
    const TRACK_SWITCH: bool = true;

    /// Python's `Regex(r"^[^/]+/[^/]+$")`: one `/` with something on both sides. Spaces are allowed.
    fn is_valid(text: &str) -> bool {
        text.split_once('/')
            .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
    }

    /// Checked at submit time, so an empty lookup never sends `GET /repos/` (Py bug B52).
    fn request(text: &str, _cx: &Cx<'_>) -> Result<GithubRequest, Toast> {
        if Self::is_valid(text) {
            Ok(GithubRequest::Repository(text.to_owned()))
        } else {
            Err(validation_error("You must enter a repo name!"))
        }
    }

    fn found(response: GithubResponse) -> Result<Result<Repository, Toast>, GithubResponse> {
        match response {
            GithubResponse::Repository(Ok(Some(repo))) => Ok(Ok(*repo)),
            GithubResponse::Repository(result) => {
                if let Err(error) = result {
                    tracing::debug!("Looking up a repository failed: {error}");
                }
                Ok(Err(validation_error("Could not find repo!")))
            }
            other => Err(other),
        }
    }

    /// Python's `_continue_tracking_repo`, run only once the repo is found (Py bug B19): the name as typed, once.
    fn on_found(_found: &Repository, submitted: &str, track: bool, cx: &mut Cx<'_>) {
        if !track
            || cx
                .config()
                .repositories
                .additional_repos_to_track
                .iter()
                .any(|tracked| tracked == submitted)
        {
            return;
        }
        cx.config_mut()
            .repositories
            .additional_repos_to_track
            .push(submitted.to_owned());
    }
}

/// The error toast a lookup raises: titled "Validation Error", as Python's are.
fn validation_error(message: &'static str) -> Toast {
    Toast::error(message).with_title("Validation Error")
}

pub mod number;

#[cfg(test)]
mod tests;
