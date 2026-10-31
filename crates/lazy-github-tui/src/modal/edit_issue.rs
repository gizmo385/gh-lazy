//! The Edit issue dialog (`issue.edit`): the issue's title, a Status select and its description in a scroll area,
//! then Save and Cancel. It saves the issue itself and closes with the updated one. Python's `EditIssueModal` around
//! an `EditIssueContainer`.

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::github::IssueUpdate;
use lazy_github_core::models::Issue;
use ratatui::Frame;
use ratatui::layout::Rect;

use super::issue_form::{EditorHeight, IssueForm};
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::Pressed;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The dialog's screen bindings, in Python's order: the modal's `modal.close`, then `modal.submit`.
const EDIT_ISSUE_BINDINGS: &[BindingId] = dialog_bindings![ModalClose, ModalSubmit];

/// `EditIssueModal > EditIssueContainer { width: 100; height: 30 }`, border included.
const SIZE: (u16, u16) = (100, 30);

/// `#updated_issue_body { height: 15 }`.
const EDITOR_ROWS: u16 = 15;

/// How long "Updating issue #N..." shows: Python's `timeout=1`.
const UPDATING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditIssueAction {
    /// `modal.submit` or Save: save the issue.
    Submit,
    /// `modal.close` or Cancel: close with nothing, cancelling a request in flight.
    Close,
}

/// The Edit issue dialog for one issue. Its output is the updated issue, or `None` when it was closed.
#[derive(Debug)]
pub struct EditIssueDialog {
    issue: Box<Issue>,
    form: IssueForm,
    /// The update request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<Issue>>>,
}

impl EditIssueDialog {
    /// A dialog for `issue`, its title, state and body filled in (an issue without a body gives an empty editor),
    /// with the scroll area focused.
    pub fn new(issue: Box<Issue>) -> Self {
        let form = IssueForm::new(
            ModalKind::EditIssue,
            Self::pressed,
            &issue.title,
            issue.body.as_deref().unwrap_or_default(),
            EditorHeight::Rows(EDITOR_ROWS),
        )
        .with_status(issue.state);
        Self {
            issue,
            form,
            pending: None,
            output: None,
        }
    }

    /// Whether an update request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: EditIssueAction) -> Action {
        Action::Dialog(DialogAction::EditIssue(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => EditIssueAction::Submit,
            Pressed::Cancel => EditIssueAction::Close,
        })
    }

    /// Python's `submit_updated_issue`, with no checks: the toast, Save as "Saving..." and disabled (Py bug B69's
    /// recommendation), and the request. A second submit waits for the first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring an Edit issue submit: one is in flight");
            return;
        }
        let number = self.issue.number;
        cx.notify(Toast::info(format!("Updating issue #{number}...")).with_timeout(UPDATING_TIMEOUT));
        self.form.set_saving(true);
        let update = IssueUpdate {
            title: self.form.title().to_owned(),
            body: self.form.body(),
            state: self.form.status(),
        };
        self.pending = Some(cx.request(GithubRequest::UpdateIssue {
            repo: Box::new(self.issue.repo.clone()),
            number,
            update,
        }));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

impl Component for EditIssueDialog {
    type Action = EditIssueAction;

    fn bindings(&self) -> &'static [BindingId] {
        EDIT_ISSUE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(EditIssueAction::Submit)),
            BindingId::ModalClose => Some(Self::action(EditIssueAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.form.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        self.form.handle_key(key, cx)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        Some(self.form.handle_mouse(event, cx))
    }

    fn update(&mut self, action: EditIssueAction, cx: &mut Cx<'_>) {
        match action {
            EditIssueAction::Submit => self.submit(cx),
            EditIssueAction::Close => self.close(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        self.form.render(frame, area, cx, under);
    }
}

impl Dialog for EditIssueDialog {
    type Output = Option<Box<Issue>>;

    fn kind(&self) -> ModalKind {
        ModalKind::EditIssue
    }

    fn own_action(&self, action: DialogAction) -> Option<EditIssueAction> {
        match action {
            DialogAction::EditIssue(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.form.move_focus(direction);
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<Option<Box<Issue>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        self.form.field_chain()
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        self.form.on_field_binding(field, id, cx)
    }

    fn captures_mouse(&self) -> bool {
        self.form.captures_mouse()
    }

    fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.form.popup_contains(column, row)
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        self.form.handle_paste(text, cx)
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::UpdateIssue(result) = response else {
            return Some(response);
        };
        self.pending = None;
        let number = self.issue.number;
        match result {
            Ok(issue) => {
                cx.notify(Toast::info(format!("Successfully updated issue #{number}")));
                self.output = Some(Some(issue));
            }
            // Python has no handling and crashes (Py bug B70): Save comes back and the dialog keeps the edits.
            Err(error) => {
                tracing::error!("Error updating issue #{number}: {error}");
                self.form.set_saving(false);
                cx.notify(Toast::error(format!("Could not update issue #{number}")).with_title("Error updating issue"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
