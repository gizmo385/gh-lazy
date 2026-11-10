//! The Reviewers field Create PR and Edit PR share, Python's `ReviewerSelectionContainer` as a compose sheet section
//! (decision compose-dialogs, D5 a). While focus is elsewhere it's one row, "Reviewers", the listed logins (or
//! `none`) and a dim "(tab to edit)". With focus it expands in place: a one-row input that completes logins, then the
//! listed logins as `<login> ✕` items, wrapping. Enter in the input looks the login up and lists it; toggling a listed
//! login (`enter` or `space` on the highlighted one, or a click) removes it.
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
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::component::Cx;
use crate::components::compose::{LabelState, field_area, render_label};
use crate::components::selection_list::{ListOutcome, SelectionList};
use crate::components::text_input::{InputOutcome, TextInput};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::{FieldKind, mark};
use crate::theme::{Paint, Theme};
use crate::toast::Toast;

/// The field's label, which the dialog's label column is sized for.
pub(super) const LABEL: &str = "Reviewers";
const PLACEHOLDER: &str = "Reviewer to add";
/// The collapsed summary with nobody listed, and its hint.
const NONE: &str = "none";
const HINT: &str = "(tab to edit)";
/// What follows each listed login's item, and the columns between two items.
const REMOVE: &str = " ✕";
const ITEM_GAP: u16 = 3;

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
    /// Where it was last drawn.
    drawn: Drawn,
}

impl ReviewersField {
    /// An empty field whose input suggests from `configured` until the collaborators arrive.
    pub fn new(configured: &[String]) -> Self {
        let configured: Arc<[String]> = configured.into();
        Self {
            input: TextInput::new(PLACEHOLDER)
                .one_row()
                .with_suggestions(Arc::clone(&configured)),
            list: SelectionList::new(),
            configured,
            lookups: Vec::new(),
            collaborators: None,
            drawn: Drawn::default(),
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
                let yellow = Style::new().fg(mark(cx.theme(), |m| m.warning));
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

    /// A left press at `event`, in the coordinates the field was last drawn in: on the input it moves the cursor there
    /// and asks for focus, as a press on the collapsed summary does; on a listed login's item it removes the login.
    pub fn handle_mouse(&mut self, event: &MouseEvent) -> ReviewersOutcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return ReviewersOutcome::Ignored;
        }
        let at = Position::new(event.column, event.row);
        if self.drawn.input.contains(at) {
            self.input.press(self.drawn.input, event.column);
            return ReviewersOutcome::Focus(ReviewersFocus::Input);
        }
        if let Some(index) = self.drawn.items.iter().position(|item| item.contains(at)) {
            return self.on_list(ListOutcome::Toggled { index, selected: false });
        }
        if self.drawn.summary.contains(at) {
            return ReviewersOutcome::Focus(ReviewersFocus::Input);
        }
        ReviewersOutcome::Ignored
    }

    /// The collapsed field's text after its label: the listed logins, comma-separated, or `none`.
    pub fn summary(&self) -> String {
        if self.list.is_empty() {
            NONE.to_owned()
        } else {
            self.reviewers().join(", ")
        }
    }

    /// Where each listed login's item goes when the items are `width` columns wide: its row from the first item row,
    /// and its columns from the left. Items flow left to right with a gap between them, wrapping before one that
    /// doesn't fit.
    fn item_layout(&self, width: u16) -> Vec<Rect> {
        let mut items = Vec::with_capacity(self.list.len());
        let (mut x, mut y) = (0u16, 0u16);
        for login in self.list.labels() {
            let item = super::layout::text_width(login).saturating_add(super::layout::text_width(REMOVE));
            if x > 0 && x.saturating_add(item) > width {
                (x, y) = (0, y + 1);
            }
            items.push(Rect::new(x, y, item.min(width), 1));
            x = x.saturating_add(item + ITEM_GAP);
        }
        items
    }

    /// The rows the field takes with its items `width` columns wide: the summary row while collapsed; expanded (while
    /// it has focus), the input row and the rows of the items.
    pub fn height(&self, expanded: bool, width: u16) -> u16 {
        if !expanded {
            return 1;
        }
        1 + self.item_layout(width).last().map_or(0, |last| last.y + 1)
    }

    /// The rows `focus` takes from the expanded field's top, with its items `width` columns wide.
    pub fn rows(&self, focus: ReviewersFocus, width: u16) -> Range<u16> {
        match focus {
            ReviewersFocus::Input => 0..1,
            ReviewersFocus::List => 1..self.height(true, width).max(2),
        }
    }

    /// Draws the field into `area`, over the dialog's background `under`, its label in the first `label_column`
    /// columns. With `focus` (the part that has it) it's expanded: the input, then the listed logins' items, the
    /// highlighted one in the block cursor while the list has focus. Without, the one-row summary.
    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        label_column: u16,
        theme: &Theme,
        under: Paint,
        focus: Option<ReviewersFocus>,
    ) {
        self.drawn = Drawn::default();
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let roles = theme.roles();
        let text = Style::new().fg(theme.color(roles.text.plain, under));
        let dim = Style::new().fg(theme.color(roles.text.muted, under));
        let label = if focus.is_some() {
            LabelState::Focused
        } else {
            LabelState::Plain
        };
        render_label(buf, area, label_column, LABEL, label, theme, under);
        let rest = Rect {
            height: 1,
            ..field_area(area, label_column)
        };
        let Some(focus) = focus else {
            let line = Line::from(vec![
                Span::styled(self.summary(), text),
                Span::raw("  "),
                Span::styled(HINT, dim),
            ]);
            buf.set_line(rest.x, rest.y, &line, rest.width);
            self.drawn.summary = Rect { height: 1, ..area };
            return;
        };
        let input_focused = focus == ReviewersFocus::Input;
        self.input.render(buf, rest, theme, under, input_focused);
        self.drawn.input = rest;
        let cursor = roles.cursor.focused;
        let cursor_bg = cursor.bg.over(under);
        let highlighted = cursor.style.apply(
            Style::new()
                .fg(theme.color(cursor.fg, cursor_bg))
                .bg(theme.paint(cursor_bg)),
        );
        let highlight = self.list.highlighted().filter(|_| focus == ReviewersFocus::List);
        let layout = self.item_layout(rest.width);
        for (index, (item, login)) in layout.into_iter().zip(self.list.labels()).enumerate() {
            let item = Rect {
                x: rest.x + item.x,
                y: area.y + 1 + item.y,
                ..item
            }
            .intersection(area);
            if item.is_empty() {
                continue;
            }
            let (name, remove) = if highlight == Some(index) {
                (highlighted, highlighted)
            } else {
                (text, dim)
            };
            let line = Line::from(vec![Span::styled(login, name), Span::styled(REMOVE, remove)]);
            buf.set_line(item.x, item.y, &line, item.width);
            self.drawn.items.push(item);
        }
    }
}

/// Where the field's parts were last drawn, for presses. Empty for a part that wasn't drawn.
#[derive(Debug, Default)]
struct Drawn {
    /// The collapsed summary row.
    summary: Rect,
    /// The input's box, while expanded.
    input: Rect,
    /// Each listed login's item, in list order, while expanded.
    items: Vec<Rect>,
}

#[cfg(test)]
mod tests;
