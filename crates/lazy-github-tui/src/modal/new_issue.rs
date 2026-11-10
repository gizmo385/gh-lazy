//! The New issue dialog (`issue.new`), on the compose sheet (decision compose-dialogs): a heading, a one-row title and
//! the description under its `Description  Write  Preview` tabs, with the footer's keys for Save and Close. It
//! creates the issue itself and closes with it. Python's `NewIssueModal` around a `NewIssueContainer`.

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::github::NewIssue;
use lazy_github_core::models::{Issue, Repository};
use ratatui::Frame;
use ratatui::layout::Rect;

use super::issue_form::IssueForm;
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::compose::{COMPOSE_BINDINGS, ComposeAction};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewIssueAction {
    /// One of the sheet's shared keys.
    Compose(ComposeAction),
}

/// The New issue dialog for one repo. Its output is the created issue, or `None` when it was closed.
#[derive(Debug)]
pub struct NewIssueDialog {
    repo: Repository,
    form: IssueForm,
    /// The create request in flight.
    pending: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<Issue>>>,
}

impl NewIssueDialog {
    /// An empty dialog for `repo`, its description holding `prefilled` (the bug-report flow passes a template), with
    /// the title focused.
    pub fn new(repo: Repository, prefilled: Option<&str>) -> Self {
        let heading = format!("New issue · {}", repo.full_name);
        Self {
            form: IssueForm::new(ModalKind::NewIssue, heading, "", prefilled.unwrap_or_default()),
            repo,
            pending: None,
            output: None,
        }
    }

    /// The title as typed.
    pub fn title(&self) -> &str {
        self.form.title()
    }

    /// The description as typed.
    pub fn description(&self) -> String {
        self.form.body()
    }

    /// Whether a create request is in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn action(action: ComposeAction) -> Action {
        Action::Dialog(DialogAction::NewIssue(NewIssueAction::Compose(action)))
    }

    /// Python's `create_issue`: an empty title is refused (a blank one isn't), an empty description is allowed (Py
    /// bug B25's recommendation), and a second submit waits for the first.
    fn submit(&mut self, cx: &mut Cx<'_>) {
        if self.pending.is_some() {
            tracing::debug!("Ignoring a New issue submit: one is in flight");
            return;
        }
        let title = self.form.title();
        if title.is_empty() {
            cx.notify(Toast::error("Must have a non-empty issue title!"));
            self.form.refuse_title();
            return;
        }
        cx.notify(Toast::info("Creating new issue..."));
        let issue = NewIssue {
            title: title.to_owned(),
            body: self.form.body(),
        };
        self.form.set_saving(true);
        self.pending = Some(cx.request(GithubRequest::CreateIssue {
            repo: Box::new(self.repo.clone()),
            issue,
        }));
    }

    fn close(&mut self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending.take() {
            cx.effect(Effect::Cancel(id));
        }
        self.output = Some(None);
    }
}

impl Component for NewIssueDialog {
    type Action = NewIssueAction;

    fn bindings(&self) -> &'static [BindingId] {
        COMPOSE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        self.form.compose_action(id).map(Self::action)
    }

    fn binding_state(&self, id: BindingId, _cx: &Cx<'_>) -> BindingState {
        self.form.binding_state(id)
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.form.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        self.form.handle_key(key, cx)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        Some(self.form.handle_mouse(event))
    }

    fn update(&mut self, action: NewIssueAction, cx: &mut Cx<'_>) {
        let NewIssueAction::Compose(action) = action;
        match action {
            ComposeAction::Submit => self.submit(cx),
            ComposeAction::Close => self.close(cx),
            other => self.form.update(other),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        self.form.render(frame, area, cx);
    }
}

impl Dialog for NewIssueDialog {
    type Output = Option<Box<Issue>>;

    fn kind(&self) -> ModalKind {
        ModalKind::NewIssue
    }

    fn own_action(&self, action: DialogAction) -> Option<NewIssueAction> {
        match action {
            DialogAction::NewIssue(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.form.move_focus(direction);
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        self.form.size(available)
    }

    fn frame(&self, area: Rect) -> Rect {
        self.form.frame(area)
    }

    fn take_output(&mut self) -> Option<Option<Box<Issue>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        self.form.field_chain()
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        self.form.on_field_binding(field, id)
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
        let GithubResponse::CreateIssue(result) = response else {
            return Some(response);
        };
        self.pending = None;
        self.form.set_saving(false);
        match result {
            Ok(issue) => {
                // Raised before the dialog closes, as Python's is.
                cx.notify(Toast::info(format!("Successfully created issue #{}", issue.number)));
                self.output = Some(Some(issue));
            }
            // Python has no handling and crashes (Py bug B70): the dialog stays open with everything typed.
            Err(error) => {
                tracing::error!("Error creating issue: {error}");
                cx.notify(Toast::error("Could not create the issue").with_title("Error creating issue"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
