//! The Reviewers field Create PR and Edit PR share: a bold "Reviewers" label, an input that completes logins, and,
//! once anyone is listed, "Current Reviewers - Click/Select them to remove" over a list of checked logins. Enter in
//! the input looks the login up and lists it; toggling a listed login removes it. Python's
//! `ReviewerSelectionContainer`.
//!
//! The field has no actions, bindings or effects of its own. The dialog that embeds it puts
//! [`ReviewersField::focus_stops`] into its focus ring, hands it the keys, bindings, clicks and responses for those
//! stops, and turns what it reports ([`ReviewersOutcome`]) into actions: Enter becomes the dialog's own action, whose
//! `update` calls [`ReviewersField::submit`] with a mutable context.

use std::ops::Range;
use std::sync::Arc;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{FullUser, Repository};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::component::Cx;
use crate::components::selection_list::{ListOutcome, SelectionList};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::FieldKind;
use crate::theme::{Accent, Paint, Theme};
use crate::toast::Toast;

const LABEL: &str = "Reviewers";
const PLACEHOLDER: &str = "Reviewer to add";
const LIST_LABEL: &str = "Current Reviewers - Click/Select them to remove";

/// The field's focusable parts, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewersFocus {
    /// The "Reviewer to add" input.
    Input,
    /// The list of reviewers, while anyone is listed.
    List,
}

/// What a key, a binding, a paste or a click did to the field, for the dialog to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum ReviewersOutcome {
    /// Taken; nothing for the dialog to do.
    Handled,
    /// Not the field's: the dialog passes it on.
    Ignored,
    /// `enter` in the input: the dialog dispatches its own action, whose `update` calls
    /// [`ReviewersField::submit`].
    Submitted,
    /// The input copied or cut this text: the dialog hands it to [`crate::action::Action::Copy`].
    Copied(String),
    /// A left press on `focus`, which the dialog focuses.
    Focus(ReviewersFocus),
    /// A toggle removed `login`: the dialog toasts "Removing <login>" ([`removing_toast`]). When the list emptied
    /// (`emptied`) it's hidden and the dialog moves focus to the input; a click on the list otherwise focuses it.
    Removed {
        /// The login taken off the list.
        login: String,
        /// Whether that was the last one.
        emptied: bool,
    },
}

/// The toast a removal raises: "Removing <login>", information.
pub fn removing_toast(login: &str) -> Toast {
    Toast::info(format!("Removing {login}"))
}

/// The Reviewers field. See the module docs.
#[derive(Debug)]
pub struct ReviewersField {
    input: TextInput,
    /// Every listed login, checked, in the order it was added.
    list: SelectionList<String>,
    /// `pull_requests.additional_suggested_pr_reviewers`, which the suggestions end with.
    configured: Arc<[String]>,
    /// The user lookups in flight, with the login each is for.
    lookups: Vec<(RequestId, String)>,
    /// The collaborators request, until it answers.
    collaborators: Option<RequestId>,
}

impl ReviewersField {
    /// An empty field whose input suggests from `configured` until the collaborators arrive.
    pub fn new(configured: &[String]) -> Self {
        let configured: Arc<[String]> = configured.into();
        Self {
            input: TextInput::new(PLACEHOLDER).with_suggestions(Arc::clone(&configured)),
            list: SelectionList::new(),
            configured,
            lookups: Vec::new(),
            collaborators: None,
        }
    }

    /// Asks for `repo`'s collaborators, for the suggestions: Python's `_fetch_collaborators`, when the dialog opens.
    pub fn load(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        if let Some(old) = self.collaborators.take() {
            cx.effect(Effect::Cancel(old));
        }
        self.collaborators = Some(cx.request(GithubRequest::Collaborators(Box::new(repo.clone()))));
    }

    /// Cancels the requests in flight, for a dialog that closes.
    pub fn cancel(&mut self, cx: &mut Cx<'_>) {
        for id in self
            .collaborators
            .take()
            .into_iter()
            .chain(self.lookups.drain(..).map(|(id, _)| id))
        {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// Takes the answer to one of the field's requests, and hands back any other.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let lookup = self.lookups.iter().position(|&(lookup, _)| lookup == id);
        match (response, lookup) {
            (GithubResponse::Collaborators(result), _) if self.collaborators == Some(id) => {
                self.collaborators = None;
                self.on_collaborators(result);
            }
            (GithubResponse::User(result), Some(index)) => {
                let (_, login) = self.lookups.remove(index);
                self.on_user(login, result, cx);
            }
            (other, _) => return Some(other),
        }
        None
    }

    /// The collaborators, followed by the configured logins, become the suggestions. A failure (GitHub answers 403 to
    /// anyone without push access) keeps the configured ones, as Python's empty list does.
    fn on_collaborators(&mut self, result: Result<Vec<String>, GhError>) {
        match result {
            Ok(mut logins) => {
                logins.extend(self.configured.iter().cloned());
                self.input.set_suggestions(logins);
            }
            Err(error) => tracing::debug!("Fetching the collaborators for the reviewer suggestions failed: {error}"),
        }
    }

    /// Python's `_validate_new_reviewer` once the user lookup answers: any failure is "not found"; a found user is
    /// listed (unless an earlier answer listed it already) and the input cleared.
    fn on_user(&mut self, login: String, result: Result<Box<FullUser>, GhError>, cx: &mut Cx<'_>) {
        match result {
            Ok(_) => {
                tracing::info!("Adding {login} to PR reviewer list");
                if !self.list.contains(&login) {
                    self.list.add(login.clone(), login, true);
                }
                self.input.set_value("");
            }
            Err(error) => {
                tracing::debug!("Looking up the reviewer {login} failed: {error}");
                let yellow = Style::new().fg(cx.theme().accent(Accent::Yellow));
                cx.notify(Toast::error(Line::from(vec![
                    Span::raw("Could not find user "),
                    Span::styled(login, yellow),
                    Span::raw(" - are you sure they exist?"),
                ])));
            }
        }
    }

    /// `enter` in the input: the text, trimmed and lower-cased, is looked up unless it's blank (Py bug B76's
    /// recommendation) or listed already.
    pub fn submit(&mut self, cx: &mut Cx<'_>) {
        let login = self.input.value().trim().to_lowercase();
        if login.is_empty() {
            return;
        }
        if self.list.contains(&login) {
            cx.notify(Toast::error("Already a reviewer!"));
            return;
        }
        let id = cx.request(GithubRequest::User(login.clone()));
        self.lookups.push((id, login));
    }

    /// Lists `logins`, checked, in order, skipping any listed already: Edit PR's requested reviewers and a restored
    /// draft's. They keep the case they come in.
    pub fn add_existing(&mut self, logins: impl IntoIterator<Item = String>) {
        for login in logins {
            if !self.list.contains(&login) {
                self.list.add(login.clone(), login, true);
            }
        }
    }

    /// The listed logins, in list order.
    pub fn reviewers(&self) -> Vec<String> {
        self.list.labels().map(str::to_owned).collect()
    }

    /// The input's text, for tests and drafts.
    pub fn input_value(&self) -> &str {
        self.input.value()
    }

    /// The input's suggestion for its current text, if any.
    pub fn suggestion(&self) -> Option<&str> {
        self.input.suggestion()
    }

    /// The highlighted row of the list, if any.
    pub fn highlighted(&self) -> Option<usize> {
        self.list.highlighted()
    }

    /// The parts that take focus: the input, and the list while anyone is listed.
    pub fn focus_stops(&self) -> &'static [ReviewersFocus] {
        if self.list.is_empty() {
            &[ReviewersFocus::Input]
        } else {
            &[ReviewersFocus::Input, ReviewersFocus::List]
        }
    }

    /// The kind of field `focus` is, for the dialog's chain.
    pub fn field_kind(focus: ReviewersFocus) -> FieldKind {
        match focus {
            ReviewersFocus::Input => FieldKind::TextInput,
            ReviewersFocus::List => FieldKind::SelectionList,
        }
    }

    /// `focus` gained focus: the input selects its text, as Textual's `select_on_focus` does. The list keeps its
    /// highlight, which a list filled after it was built doesn't have until a movement key.
    pub fn focus_gained(&mut self, focus: ReviewersFocus) {
        if focus == ReviewersFocus::Input {
            self.input.select_all();
        }
    }

    /// Whether the focused part takes `key` before any binding: the input's printable keys and bindings. The list's
    /// bindings come through the chain ([`Self::on_binding`]).
    pub fn consumes_key(&self, focus: ReviewersFocus, key: &KeyChord, keymap: &Keymap) -> bool {
        focus == ReviewersFocus::Input && self.input.consumes_key(key, keymap)
    }

    /// A key while `focus` has focus.
    pub fn handle_key(
        &mut self,
        focus: ReviewersFocus,
        key: &KeyChord,
        keymap: &Keymap,
        clipboard: &str,
    ) -> ReviewersOutcome {
        match focus {
            ReviewersFocus::Input => match self.input.handle_key(key, keymap, clipboard) {
                InputOutcome::Changed => ReviewersOutcome::Handled,
                InputOutcome::Submitted => ReviewersOutcome::Submitted,
                InputOutcome::Copied(text) => ReviewersOutcome::Copied(text),
                InputOutcome::Ignored => ReviewersOutcome::Ignored,
            },
            ReviewersFocus::List => match self.list.handle_key(key, keymap) {
                ListOutcome::Ignored => ReviewersOutcome::Ignored,
                outcome => self.on_list(outcome),
            },
        }
    }

    /// A binding of the focused list's link (`FieldKind::SelectionList`): the `OptionList` keys move the highlight,
    /// and `space` or `enter` toggle, which removes the reviewer.
    pub fn on_binding(&mut self, id: BindingId) -> ReviewersOutcome {
        match self.list.on_binding(id) {
            ListOutcome::Ignored => ReviewersOutcome::Ignored,
            outcome => self.on_list(outcome),
        }
    }

    /// Every listed login is checked, so any toggle takes it off the list, as Python's `handle_reviewer_deselected`
    /// does.
    fn on_list(&mut self, outcome: ListOutcome) -> ReviewersOutcome {
        match outcome {
            ListOutcome::Toggled { index, .. } => match self.list.remove_at(index) {
                Some(login) => ReviewersOutcome::Removed {
                    login,
                    emptied: self.list.is_empty(),
                },
                None => ReviewersOutcome::Handled,
            },
            ListOutcome::Moved | ListOutcome::Ignored => ReviewersOutcome::Handled,
        }
    }

    /// A bracketed paste into the input.
    pub fn handle_paste(&mut self, text: &str) {
        self.input.handle_paste(text);
    }

    /// Whether a press on the input started a drag that hasn't been released.
    pub fn is_selecting(&self) -> bool {
        self.input.is_selecting()
    }

    /// A drag or release while the input has the mouse; see [`TextInput::captured_mouse`].
    pub fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        self.input.captured_mouse(event)
    }

    /// A left press at `event` on a field drawn in `area` (both in the coordinates it was drawn in): on the input it
    /// moves the cursor there and asks for focus; on a listed login it toggles, and so removes, it.
    pub fn handle_mouse(&mut self, event: &MouseEvent, area: Rect) -> ReviewersOutcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return ReviewersOutcome::Ignored;
        }
        let at = Position::new(event.column, event.row);
        let input = self.area_of(ReviewersFocus::Input, area);
        if input.contains(at) {
            self.input.press(input, event.column);
            return ReviewersOutcome::Focus(ReviewersFocus::Input);
        }
        let list = self.area_of(ReviewersFocus::List, area);
        if !list.contains(at) {
            return ReviewersOutcome::Ignored;
        }
        match self.list.handle_mouse(event) {
            ListOutcome::Ignored => ReviewersOutcome::Focus(ReviewersFocus::List),
            outcome => self.on_list(outcome),
        }
    }

    /// The rows the field takes: the label and the input, then the list's label and the list while anyone is
    /// listed.
    pub fn height(&self) -> u16 {
        let input = 1 + text_input::HEIGHT;
        if self.list.is_empty() {
            input
        } else {
            input + 1 + self.list.height()
        }
    }

    /// The rows `focus` takes, from the field's top.
    pub fn rows(&self, focus: ReviewersFocus) -> Range<u16> {
        match focus {
            ReviewersFocus::Input => 1..1 + text_input::HEIGHT,
            ReviewersFocus::List => {
                let top = 2 + text_input::HEIGHT;
                top..top + self.list.height()
            }
        }
    }

    /// Where `focus` goes in a field drawn in `area`. Empty for a hidden list.
    fn area_of(&self, focus: ReviewersFocus, area: Rect) -> Rect {
        if focus == ReviewersFocus::List && self.list.is_empty() {
            return Rect::default();
        }
        let rows = self.rows(focus);
        Rect {
            y: area.y.saturating_add(rows.start),
            height: rows.end - rows.start,
            ..area
        }
        .intersection(area)
    }

    /// Draws the field into the top [`Self::height`] rows of `area`, over the dialog's background `under`, with
    /// `focus` drawn focused.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focus: Option<ReviewersFocus>) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let text = Style::new().fg(theme.color(theme.palette.foreground, under));
        let width = usize::from(area.width);
        buf.set_stringn(area.x, area.y, LABEL, width, text.add_modifier(Modifier::BOLD));
        let input = self.area_of(ReviewersFocus::Input, area);
        self.input
            .render(buf, input, theme, under, focus == Some(ReviewersFocus::Input));
        if self.list.is_empty() {
            return;
        }
        let list = self.area_of(ReviewersFocus::List, area);
        if let Some(y) = list.y.checked_sub(1).filter(|&y| y >= area.y) {
            buf.set_stringn(area.x, y, LIST_LABEL, width, text);
        }
        self.list
            .render(buf, list, theme, under, focus == Some(ReviewersFocus::List));
    }
}

#[cfg(test)]
mod tests;
