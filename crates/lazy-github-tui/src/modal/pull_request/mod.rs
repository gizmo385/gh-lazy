//! Create PR and Edit PR, on the compose sheet (decision compose-dialogs): a heading naming the repo, the upstream
//! warning when it applies, the one-row Base / Compare / Draft row, the Title, the description under its
//! `Description  Write  Preview` tabs, and the Reviewers section, which is one row until it has focus. Python's
//! `CreateOrEditPullRequestModal` around a `CreateOrEditPullRequestContainer`, whose fixed 30-row editor, tall inputs,
//! rules and button row the sheet replaces. Edit PR prefills everything from the pull request and disables the branch
//! row.
//!
//! The sheet is as tall as its parts and grows with the description up to the screen less the footer and a margin
//! row; only then does the description scroll. The dialog shows its own footer: submit (`ctrl+s`, `shift+enter`),
//! Preview, Expand, and `escape`, which leaves a field, then closes from the sheet.

mod draft;
mod layout;
mod reviewers;
mod submit;

use std::ops::Range;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{Branch, FullPullRequest, Repository};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use self::draft::DraftFile;
use self::layout::{BranchColumns, SheetRows};
use self::submit::{BranchFollowUp, Submit};
use super::form::FormScroll;
use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind, mark};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::compose::{
    BodyMode, BodyMouse, COMPOSE_BINDINGS, ComposeAction, ComposeBody, ComposeLayout, ComposeParts, LabelState,
    SheetGeometry, SheetPlacement, field_area, render_heading, render_label, sheet_geometry,
};
use crate::components::disabled;
use crate::components::switch::Switch;
use crate::components::text_editor::{EditorOutcome, EditorVariant};
use crate::components::text_input::{InputOutcome, TextInput};
use crate::effect::{Effect, GitRequest, GitResponse, GithubRequest, GithubResponse, RequestId, TimerId};
use crate::gh_health::report_unhandled;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

pub use reviewers::{ReviewersField, ReviewersFocus, ReviewersOutcome, removing_toast};

/// The dialog's screen bindings: the compose sheet's.
pub(crate) const PR_BINDINGS: &[BindingId] = COMPOSE_BINDINGS;

/// The sheet's widest.
const MAX_WIDTH: u16 = 100;
/// A description's body is at least this tall.
const BODY_MIN_ROWS: u16 = 5;

const CREATE_HEADING: &str = "New Pull Request";
const BASE_PLACEHOLDER: &str = "Choose a base ref";
const COMPARE_PLACEHOLDER: &str = "Choose a head ref";
const TITLE_PLACEHOLDER: &str = "Title";
const DESCRIPTION_PLACEHOLDER: &str = "Describe the change (markdown)";
const WARNING: &str = "Warning:";
const NO_UPSTREAM: &str = " Current local branch has no configured upstream";

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestAction {
    /// One of the sheet's shared keys. Its submit checks the fields and creates or updates the pull request.
    Compose(ComposeAction),
    /// `enter` in the Reviewers input: look the login up.
    AddReviewer,
    /// Title, Base, Compare or the description changed: Python's `Input.Changed` / `TextArea.Changed` handlers,
    /// where drafts (#186) schedule their save.
    Edited,
    /// The push prompt's answer: push the branch the refused create named (`true`), or not.
    PushAnswer(bool),
}

/// What the dialog is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// A new pull request.
    Create,
    /// This pull request, as it was when the dialog opened.
    Edit(Box<FullPullRequest>),
}

/// What Create PR opens with, read by the opener.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreatePrefill {
    /// Base: the selected repo's default branch.
    pub base: String,
    /// Compare: the current-directory branch, when the current-directory repo is the selected repo.
    pub compare: Option<String>,
    /// The description: `pull_requests.pull_request_template`'s text, when it names a readable file.
    pub description: String,
}

/// The form's values, as drafts (#186) save and restore them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestForm {
    /// The title, as typed.
    pub title: String,
    /// The description, as typed.
    pub description: String,
    /// Base, as typed.
    pub base_ref: String,
    /// Compare, as typed.
    pub head_ref: String,
    /// The Draft switch.
    pub is_draft: bool,
    /// The listed reviewers, in list order.
    pub reviewers: Vec<String>,
}

/// Where the dialog's focus is: one of its fields, in screen order, or the sheet itself after `escape` left one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrField {
    Sheet,
    Base,
    Compare,
    Draft,
    Title,
    Description,
    Reviewers(ReviewersFocus),
}

/// Where the branch row and the title were last drawn, in the interior's content coordinates, for presses.
#[derive(Debug, Clone, Copy, Default)]
struct Drawn {
    branches: Option<BranchColumns>,
    title_row: Rect,
    title: Rect,
}

/// The Create PR or Edit PR dialog for one repo. Its output is the created or updated pull request, or `None` when
/// it was closed.
#[derive(Debug)]
pub struct PullRequestDialog {
    repo: Repository,
    mode: Mode,
    focus: PrField,
    base: TextInput,
    compare: TextInput,
    draft: Switch,
    title: TextInput,
    body: ComposeBody,
    reviewers: ReviewersField,
    /// `compose.expand`: the sheet fills the screen above the footer.
    expanded: bool,
    placement: SheetPlacement,
    /// The interior, which scrolls (without a bar) only on a screen too short for even the parts' minimum.
    scroll: FormScroll,
    /// Bring the focused part into view at the next draw, after a focus change or an edit.
    reveal: bool,
    /// The theme of the last draw, which [`Dialog::frame`] measures the description in.
    theme: Option<Theme>,
    drawn: Drawn,
    /// Whether the current-directory branch has no upstream: the warning row shows.
    upstream_warning: bool,
    /// The branches request, until it answers.
    branches_request: Option<RequestId>,
    /// The upstream check, until it answers.
    upstream_request: Option<RequestId>,
    /// Edit PR's request for the reviewers already requested, until it answers.
    requested_request: Option<RequestId>,
    /// The submit in flight.
    submit: Option<Submit>,
    /// The remote check and the push prompt after a refused create.
    follow_up: BranchFollowUp,
    /// Create PR's draft (#186); Edit PR has none.
    draft_file: Option<DraftFile>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<FullPullRequest>>>,
}

impl PullRequestDialog {
    /// Create PR for `repo`, filled in from `prefill`, with the title focused. It asks at once for the repo's
    /// branches (uncached, so a branch pushed a moment ago is offered; owner decision on #278) and its collaborators,
    /// and whether the current-directory branch, if git named one, has an upstream. It also reads the repo's saved
    /// draft, which fills the form in over `prefill` when it loads.
    pub fn create(repo: Repository, prefill: CreatePrefill, cx: &mut Cx<'_>) -> Self {
        let mut base = TextInput::new(BASE_PLACEHOLDER).one_row();
        base.set_value(prefill.base);
        let mut compare = TextInput::new(COMPARE_PLACEHOLDER).one_row();
        compare.set_value(prefill.compare.unwrap_or_default());
        let branches_request = Some(cx.request(GithubRequest::ListBranchesUncached {
            repo: Box::new(repo.clone()),
        }));
        let upstream_request = cx
            .current_branch()
            .map(str::to_owned)
            .map(|branch| cx.git(GitRequest::HasUpstream { branch }));
        let draft_file = Some(DraftFile::open(&repo.full_name, cx));
        let mut dialog = Self::new(
            repo,
            Mode::Create,
            base,
            compare,
            Switch::new(false),
            &prefill.description,
            cx,
        );
        dialog.branches_request = branches_request;
        dialog.upstream_request = upstream_request;
        dialog.draft_file = draft_file;
        dialog
    }

    /// Edit PR for `pr`: its title and body (empty when it has none) filled in with the title focused, its base, head
    /// and draft flag shown in the disabled branch row. It asks at once for the repo's collaborators and the reviewers
    /// already requested, who are listed when they arrive.
    pub fn edit(pr: Box<FullPullRequest>, cx: &mut Cx<'_>) -> Self {
        let repo = pr.pr.issue.repo.clone();
        let mut base = TextInput::new(BASE_PLACEHOLDER).one_row();
        base.set_value(pr.base.name.clone());
        let mut compare = TextInput::new(COMPARE_PLACEHOLDER).one_row();
        compare.set_value(pr.head.name.clone());
        let draft = Switch::new(pr.pr.draft);
        let requested_request = Some(cx.request(GithubRequest::RequestedReviewers {
            repo: Box::new(repo.clone()),
            number: pr.number(),
        }));
        let (title, body) = (pr.pr.issue.title.clone(), pr.pr.issue.body.clone());
        let mut dialog = Self::new(
            repo,
            Mode::Edit(pr),
            base,
            compare,
            draft,
            body.as_deref().unwrap_or_default(),
            cx,
        );
        dialog.title.set_value(title);
        dialog.requested_request = requested_request;
        // The prefilled title is selected, as focusing it selects it.
        dialog.title.select_all();
        dialog
    }

    /// The parts both modes share, the description holding `description`, with the title focused and the
    /// collaborators asked for.
    fn new(
        repo: Repository,
        mode: Mode,
        base: TextInput,
        compare: TextInput,
        draft: Switch,
        description: &str,
        cx: &mut Cx<'_>,
    ) -> Self {
        let mut reviewers = ReviewersField::new(&cx.config().pull_requests.additional_suggested_pr_reviewers);
        reviewers.load(&repo, cx);
        Self {
            repo,
            mode,
            focus: PrField::Title,
            base,
            compare,
            draft,
            title: TextInput::new(TITLE_PLACEHOLDER).one_row(),
            body: ComposeBody::new(EditorVariant::Code, DESCRIPTION_PLACEHOLDER, BODY_MIN_ROWS).with_text(description),
            reviewers,
            expanded: false,
            placement: SheetPlacement::default(),
            scroll: FormScroll::without_scrollbar(),
            reveal: true,
            theme: None,
            drawn: Drawn::default(),
            upstream_warning: false,
            branches_request: None,
            upstream_request: None,
            requested_request: None,
            submit: None,
            follow_up: BranchFollowUp::default(),
            draft_file: None,
            output: None,
        }
    }

    /// The repo the pull request is for.
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// The form as typed.
    pub fn form(&self) -> PullRequestForm {
        PullRequestForm {
            title: self.title.value().to_owned(),
            description: self.body.text(),
            base_ref: self.base.value().to_owned(),
            head_ref: self.compare.value().to_owned(),
            is_draft: self.draft.value(),
            reviewers: self.reviewers.reviewers(),
        }
    }

    /// Fills the form in from `form` (a restored draft, #186), over the prefills. Reviewers already listed stay.
    pub fn restore(&mut self, form: PullRequestForm) {
        self.title.set_value(form.title);
        self.body.set_text(&form.description);
        self.base.set_value(form.base_ref);
        self.compare.set_value(form.head_ref);
        if self.draft.value() != form.is_draft {
            self.draft.toggle();
        }
        self.reviewers.add_existing(form.reviewers);
        self.reveal = true;
    }

    /// Whether the upstream warning shows.
    pub fn shows_upstream_warning(&self) -> bool {
        self.upstream_warning
    }

    fn action(action: PullRequestAction) -> Action {
        Action::Dialog(DialogAction::PullRequest(action))
    }

    /// The heading: what's being written, and in which repo.
    fn heading(&self) -> String {
        match &self.mode {
            Mode::Create => format!("{CREATE_HEADING} · {}", self.repo.full_name),
            Mode::Edit(pr) => format!("Editing PR {} · {}", pr.number(), self.repo.full_name),
        }
    }

    /// What submitting does, which names the submit keys and, in flight, ends the heading.
    fn verb(&self) -> (&'static str, &'static str) {
        match self.mode {
            Mode::Create => ("Create", "Creating…"),
            Mode::Edit(_) => ("Update", "Updating…"),
        }
    }

    /// The fields in screen order: the branch row's only while it's enabled (Create PR), and the Reviewers field's
    /// list while anyone is listed.
    fn order(&self) -> Vec<PrField> {
        let mut order = Vec::with_capacity(7);
        if matches!(self.mode, Mode::Create) {
            order.extend([PrField::Base, PrField::Compare, PrField::Draft]);
        }
        order.extend([PrField::Title, PrField::Description]);
        order.extend(
            self.reviewers
                .focus_stops()
                .iter()
                .map(|&focus| PrField::Reviewers(focus)),
        );
        order
    }

    /// The Reviewers part with focus, while it has it: the section is expanded then.
    fn reviewers_focus(&self) -> Option<ReviewersFocus> {
        match self.focus {
            PrField::Reviewers(focus) => Some(focus),
            _ => None,
        }
    }

    /// The sheet's background, which its text and markdown sit on.
    fn under(theme: &Theme) -> Paint {
        theme.roles().surface.dialog.over(theme.background())
    }

    /// The sheet's limits on `screen` and its parts' rows, measured in `theme`.
    fn sheet(&self, screen: Rect, theme: &Theme) -> (SheetGeometry, SheetRows) {
        let geometry = sheet_geometry(screen, MAX_WIDTH, self.expanded);
        let width = geometry.content_width;
        let parts = ComposeParts {
            quote: u16::from(self.upstream_warning),
            fields: 2,
            body_needed: self.body.rows_needed(width, theme, Self::under(theme)),
            body_min: self.body.min_rows(),
            sections: self
                .reviewers
                .height(self.reviewers_focus().is_some(), reviewer_items_width(width)),
        };
        let layout = ComposeLayout::new(&parts, geometry.room(), self.expanded);
        (geometry, SheetRows { layout })
    }

    /// The frame's outer size on `screen`: as tall as the parts, up to the most the screen allows.
    fn sheet_size(&self, screen: Rect) -> (u16, u16) {
        let theme = self.theme.unwrap_or_default();
        let (geometry, rows) = self.sheet(screen, &theme);
        (geometry.width, rows.layout.height.min(geometry.room()) + 2)
    }

    /// The rows the focused part takes, in a sheet laid out as `rows` with its parts `width` columns wide.
    fn focused_rows(&self, rows: &SheetRows, width: u16) -> Option<Range<u16>> {
        let row = |y: u16| y..y + 1;
        match self.focus {
            PrField::Sheet => None,
            PrField::Base | PrField::Compare | PrField::Draft => Some(row(rows.branches())),
            PrField::Title => Some(row(rows.title())),
            PrField::Description => Some(rows.layout.tabs..rows.layout.body.end),
            PrField::Reviewers(focus) => {
                let top = rows.reviewers().start;
                let inner = self.reviewers.rows(focus, reviewer_items_width(width));
                Some(top + inner.start..top + inner.end)
            }
        }
    }

    /// The text input `field` is, if it's one of the dialog's own.
    fn input_mut(&mut self, field: PrField) -> Option<&mut TextInput> {
        match field {
            PrField::Base => Some(&mut self.base),
            PrField::Compare => Some(&mut self.compare),
            PrField::Title => Some(&mut self.title),
            _ => None,
        }
    }

    /// Textual's `Length(minimum=1)` on `field`'s input: the label's error color follows the latest check.
    fn validate(&mut self, field: PrField) -> bool {
        self.input_mut(field).is_none_or(|input| {
            let valid = !input.value().is_empty();
            input.set_invalid(!valid);
            valid
        })
    }

    /// Moves focus to `field`. An input that loses focus is checked (Textual validates on blur); a field that gains it
    /// is brought into view, an input selects its text (`select_on_focus`) and the editor starts a new undo batch.
    fn focus_field(&mut self, field: PrField) {
        self.reveal = true;
        let before = self.focus;
        if field == before {
            return;
        }
        self.validate(before);
        self.focus = field;
        match field {
            PrField::Description => self.body.focus_gained(),
            PrField::Reviewers(focus) => self.reviewers.focus_gained(focus),
            other => {
                if let Some(input) = self.input_mut(other) {
                    input.select_all();
                }
            }
        }
    }

    /// Edits `field`'s input with `edit`, checking it when the text changed (Textual's `Input.Changed`) or was
    /// submitted. Returns the outcome and whether the text changed.
    fn edit_input(
        &mut self,
        field: PrField,
        edit: impl FnOnce(&mut TextInput) -> InputOutcome,
    ) -> (InputOutcome, bool) {
        let Some(input) = self.input_mut(field) else {
            return (InputOutcome::Ignored, false);
        };
        let before = input.value().to_owned();
        let outcome = edit(input);
        let changed = input.value() != before;
        if changed || outcome == InputOutcome::Submitted {
            self.validate(field);
        }
        (outcome, changed)
    }

    /// What the dialog does with a Reviewers outcome. `clicked` is a press on the list, which focuses it.
    fn on_reviewers(&mut self, outcome: ReviewersOutcome, clicked: bool) -> Option<Action> {
        match outcome {
            ReviewersOutcome::Handled => Some(Action::Consumed),
            ReviewersOutcome::Ignored => None,
            ReviewersOutcome::Submitted => Some(Self::action(PullRequestAction::AddReviewer)),
            ReviewersOutcome::Copied(text) => Some(Action::Copy(text)),
            ReviewersOutcome::Focus(focus) => {
                self.focus_field(PrField::Reviewers(focus));
                Some(Action::Consumed)
            }
            ReviewersOutcome::Removed { login, emptied } => {
                if emptied {
                    self.focus_field(PrField::Reviewers(ReviewersFocus::Input));
                } else if clicked {
                    self.focus_field(PrField::Reviewers(ReviewersFocus::List));
                }
                Some(Action::Notify(removing_toast(&login)))
            }
        }
    }

    /// The one way the dialog closes without a pull request: `modal.close` from the sheet. Requests in flight are
    /// cancelled. Create PR saves its draft first.
    fn close(&mut self, cx: &mut Cx<'_>) {
        self.save_draft(cx);
        for id in self
            .branches_request
            .take()
            .into_iter()
            .chain(self.upstream_request.take())
            .chain(self.requested_request.take())
        {
            cx.effect(Effect::Cancel(id));
        }
        self.reviewers.cancel(cx);
        self.cancel_submit(cx);
        self.output = Some(None);
    }

    fn on_branches(&mut self, result: Result<Vec<Branch>, GhError>, cx: &mut Cx<'_>) {
        match result {
            Ok(branches) => {
                let names: Vec<String> = branches.into_iter().map(|branch| branch.name).collect();
                self.base.set_suggestions(names.clone());
                self.compare.set_suggestions(names);
            }
            // Python's worker raises, which ends the app; here the inputs just get no suggestions (#323).
            Err(error) => report_unhandled("Error fetching branches for the pull request dialog", &error, cx),
        }
    }

    /// A left press, in the interior's content coordinates: the field under it takes focus, and the press, for the
    /// text fields and the switch. Edit PR's branch row is disabled, so a press there does nothing. Returns the
    /// action a press on the Reviewers section gives (a removal's toast).
    fn press(&mut self, event: MouseEvent) -> Option<Action> {
        let at = Position::new(event.column, event.row);
        if let Some(columns) = self.drawn.branches.filter(|_| matches!(self.mode, Mode::Create)) {
            if columns.base.contains(at) {
                self.focus_field(PrField::Base);
                self.base.press(columns.base, event.column);
                return None;
            }
            if columns.compare.contains(at) {
                self.focus_field(PrField::Compare);
                self.compare.press(columns.compare, event.column);
                return None;
            }
            if self.draft.handle_mouse(&event, columns.draft) {
                self.focus_field(PrField::Draft);
                return None;
            }
        }
        if self.drawn.title_row.contains(at) {
            self.focus_field(PrField::Title);
            if self.drawn.title.contains(at) {
                self.title.press(self.drawn.title, event.column);
            }
            return None;
        }
        match self.reviewers.handle_mouse(&event) {
            ReviewersOutcome::Ignored => None,
            outcome => self.on_reviewers(outcome, true),
        }
    }
}

/// The branch row's fields, borrowed for drawing.
struct BranchRow<'a> {
    base: &'a mut TextInput,
    compare: &'a mut TextInput,
    draft: &'a Switch,
    /// The focused field, unless the row is disabled.
    focus: Option<PrField>,
    /// Edit PR's: the branches can't change.
    disabled: bool,
}

impl BranchRow<'_> {
    /// Draws `Base <base> ← Compare <head>  Draft [ ]` into `area` over the sheet's background `under`, and returns
    /// where each part went.
    fn render(self, buf: &mut Buffer, area: Rect, label_column: u16, theme: &Theme, under: Paint) -> BranchColumns {
        let columns = BranchColumns::new(area, label_column);
        let is = |field: PrField| self.focus == Some(field);
        let base_label = label_state(is(PrField::Base), self.base.is_invalid());
        render_label(
            buf,
            columns.base_label,
            label_column,
            layout::BASE_LABEL,
            base_label,
            theme,
            under,
        );
        self.base.render(buf, columns.base, theme, under, is(PrField::Base));
        // ` ← Compare `: the arrow in the labels' plain color, then the label, which follows the field's state.
        let compare_label = label_state(is(PrField::Compare), self.compare.is_invalid());
        let arrow = columns.compare_label;
        render_label(
            buf,
            arrow,
            arrow.width,
            layout::COMPARE_ARROW,
            LabelState::Plain,
            theme,
            under,
        );
        let after = layout::text_width(layout::COMPARE_ARROW);
        let compare = field_area(arrow, after);
        render_label(
            buf,
            compare,
            compare.width,
            layout::COMPARE_LABEL,
            compare_label,
            theme,
            under,
        );
        self.compare
            .render(buf, columns.compare, theme, under, is(PrField::Compare));
        let draft = field_area(columns.draft_label, layout::DRAFT_GAP);
        let draft_label = label_state(is(PrField::Draft), false);
        render_label(buf, draft, draft.width, layout::DRAFT_LABEL, draft_label, theme, under);
        self.draft
            .render_compact(buf, columns.draft, theme, under, is(PrField::Draft));
        if self.disabled {
            // Drawn as Textual draws disabled fields.
            disabled::fade(buf, area, theme, under);
        }
        columns
    }
}

/// The Title row in `row`: its label, then the input, drawn focused while `focused`. Returns the input's area.
fn render_title(
    buf: &mut Buffer,
    row: Rect,
    label_column: u16,
    title: &mut TextInput,
    theme: &Theme,
    under: Paint,
    focused: bool,
) -> Rect {
    let state = label_state(focused, title.is_invalid());
    render_label(buf, row, label_column, layout::TITLE_LABEL, state, theme, under);
    let area = field_area(row, label_column);
    title.render(buf, area, theme, under, focused);
    area
}

/// How a field's label is drawn: invalid wins over focused.
fn label_state(focused: bool, invalid: bool) -> LabelState {
    if invalid {
        LabelState::Invalid
    } else if focused {
        LabelState::Focused
    } else {
        LabelState::Plain
    }
}

/// The upstream warning, Python's text, in `area`'s row.
fn render_warning(buf: &mut Buffer, area: Rect, theme: &Theme, text: Style) {
    let line = Line::from(vec![
        Span::styled(WARNING, Style::new().fg(mark(theme, |m| m.warning))),
        Span::styled(NO_UPSTREAM, text),
    ]);
    buf.set_line(area.x, area.y, &line, area.width);
}

/// The width of the Reviewers items in a sheet whose parts are `width` columns wide: right of the label column.
fn reviewer_items_width(width: u16) -> u16 {
    width.saturating_sub(layout::label_column())
}

impl Component for PullRequestDialog {
    type Action = PullRequestAction;

    fn bindings(&self) -> &'static [BindingId] {
        COMPOSE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        let action = match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit => ComposeAction::Submit,
            BindingId::ComposeTogglePreview => ComposeAction::TogglePreview,
            BindingId::ComposeExpand => ComposeAction::ToggleExpand,
            BindingId::ModalClose if self.focus == PrField::Sheet => ComposeAction::Close,
            BindingId::ModalClose => ComposeAction::Leave,
            _ => return None,
        };
        Some(Self::action(PullRequestAction::Compose(action)))
    }

    fn binding_state(&self, id: BindingId, _cx: &Cx<'_>) -> BindingState {
        let (verb, _) = self.verb();
        match id {
            BindingId::ComposeSubmit | BindingId::ModalSubmit if self.submit.is_some() => {
                BindingState::DisabledAs(verb)
            }
            BindingId::ComposeSubmit | BindingId::ModalSubmit => BindingState::Relabeled(verb),
            BindingId::ComposeTogglePreview if self.body.mode() == BodyMode::Preview => {
                BindingState::Relabeled("Write")
            }
            BindingId::ComposeExpand if self.expanded => BindingState::Relabeled("Collapse"),
            BindingId::ModalClose if self.focus != PrField::Sheet => BindingState::Relabeled("Leave"),
            _ => BindingState::Enabled,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match self.focus {
            PrField::Base => self.base.consumes_key(key, keymap),
            PrField::Compare => self.compare.consumes_key(key, keymap),
            PrField::Title => self.title.consumes_key(key, keymap),
            PrField::Description => self.body.consumes_key(key, keymap),
            PrField::Reviewers(focus) => self.reviewers.consumes_key(focus, key, keymap),
            PrField::Sheet | PrField::Draft => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        let field = self.focus;
        match field {
            PrField::Base | PrField::Compare | PrField::Title => {
                let (outcome, changed) =
                    self.edit_input(field, |input| input.handle_key(&chord, cx.keymap(), cx.clipboard()));
                match outcome {
                    InputOutcome::Copied(text) => Some(Action::Copy(text)),
                    InputOutcome::Ignored => None,
                    // `enter` only checks the input; submitting is the sheet's submit keys.
                    InputOutcome::Changed | InputOutcome::Submitted if changed => {
                        Some(Self::action(PullRequestAction::Edited))
                    }
                    InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
                }
            }
            PrField::Description => match self.body.handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now()) {
                EditorOutcome::Copied { text, changed } => {
                    self.reveal |= changed;
                    Some(Action::Copy(text))
                }
                EditorOutcome::Changed => {
                    self.reveal = true;
                    Some(Self::action(PullRequestAction::Edited))
                }
                EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
            },
            PrField::Reviewers(focus) => {
                let outcome = self.reviewers.handle_key(focus, &chord, cx.keymap(), cx.clipboard());
                self.on_reviewers(outcome, false)
            }
            PrField::Sheet | PrField::Draft => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        // The parts are drawn offscreen and copied with the interior's scroll: their coordinates are the content's.
        let moved = self.scroll.to_content(event);
        // A drag that started in a text field, wherever the pointer is now.
        if self.base.captured_mouse(moved)
            || self.compare.captured_mouse(moved)
            || self.title.captured_mouse(moved)
            || self.reviewers.captured_mouse(moved)
        {
            return Some(Action::Consumed);
        }
        match self.body.handle_mouse(moved) {
            BodyMouse::Focused => {
                self.focus_field(PrField::Description);
                return Some(Action::Consumed);
            }
            BodyMouse::Handled => return Some(Action::Consumed),
            BodyMouse::Missed => {}
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.scroll.wheel(wheel);
        } else if event.kind == MouseEventKind::Down(MouseButton::Left) && self.scroll.content_row(event.row).is_some()
        {
            return self.press(moved).or(Some(Action::Consumed));
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: PullRequestAction, cx: &mut Cx<'_>) {
        match action {
            PullRequestAction::Compose(ComposeAction::Submit) => self.submit(cx),
            PullRequestAction::Compose(ComposeAction::Close) => self.close(cx),
            PullRequestAction::Compose(ComposeAction::Leave) => {
                self.validate(self.focus);
                self.focus = PrField::Sheet;
            }
            PullRequestAction::Compose(ComposeAction::TogglePreview) => {
                self.body.toggle_mode();
                self.focus_field(PrField::Description);
            }
            PullRequestAction::Compose(ComposeAction::ToggleExpand) => {
                self.expanded = !self.expanded;
                self.reveal = true;
            }
            PullRequestAction::PushAnswer(yes) => self.on_push_answer(yes, cx),
            PullRequestAction::AddReviewer => self.reviewers.submit(cx),
            PullRequestAction::Edited => self.schedule_draft_save(cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        self.theme = Some(*theme);
        let screen = frame.area();
        // The frame around the interior, whose border is a cell wide.
        self.placement
            .record(screen, area.outer(Margin::new(1, 1)), self.expanded);
        let (geometry, rows) = self.sheet(screen, theme);
        if std::mem::take(&mut self.reveal)
            && let Some(focused) = self.focused_rows(&rows, geometry.content_width)
        {
            self.scroll.center_at_draw(focused.clone(), focused);
        }
        let roles = theme.roles();
        let under = Self::under(theme);
        let text = Style::new().fg(theme.color(roles.text.plain, under));
        let dim = Style::new().fg(theme.color(roles.text.muted, under));
        let heading = self.heading();
        let (_, in_flight) = self.verb();
        let tail = self.submit.is_some().then_some(in_flight);
        let warning = self.upstream_warning;
        let editing = matches!(self.mode, Mode::Edit(_));
        let focus = self.focus;
        let reviewers_focus = self.reviewers_focus();
        let label_column = layout::label_column();
        let on_title = focus == PrField::Title;
        let on_body = focus == PrField::Description;
        let description = label_state(on_body, false);
        let Self {
            scroll,
            base,
            compare,
            draft,
            title,
            body,
            reviewers,
            drawn,
            ..
        } = self;
        // The links drawn, in rows from the interior's top and columns from its left edge.
        let mut links = Vec::new();
        scroll.render(
            frame.buffer_mut(),
            area,
            theme,
            text.bg(theme.paint(under)),
            rows.layout.height,
            |buf, content| {
                let row = |y: u16, height: u16| Rect {
                    x: content.x + 1,
                    y: content.y + y,
                    width: content.width.saturating_sub(2),
                    height,
                };
                render_heading(buf, row(rows.layout.heading, 1), &heading, tail, text, dim);
                if let Some(y) = rows.warning().filter(|_| warning) {
                    render_warning(buf, row(y, 1), theme, text);
                }

                let branches = BranchRow {
                    base,
                    compare,
                    draft,
                    focus: (!editing).then_some(focus),
                    disabled: editing,
                };
                let columns = branches.render(buf, row(rows.branches(), 1), label_column, theme, under);

                let title_row = row(rows.title(), 1);
                let title_box = render_title(buf, title_row, label_column, title, theme, under, on_title);
                *drawn = Drawn {
                    branches: Some(columns),
                    title_row,
                    title: title_box,
                };

                let (tabs_row, column) = (row(rows.layout.tabs, 1), layout::TABS_COLUMN);
                render_label(
                    buf,
                    tabs_row,
                    column,
                    layout::DESCRIPTION_LABEL,
                    description,
                    theme,
                    under,
                );
                let tabs = field_area(tabs_row, column);
                let rect = row(rows.layout.body.start, rows.layout.body_rows());
                let spans = body.render(buf, tabs, rect, theme, under, on_body);
                let shift = rect.x - content.x;
                links.extend(spans.into_iter().map(|mut span| {
                    span.row += usize::from(rect.y);
                    span.columns = span.columns.start + shift..span.columns.end + shift;
                    span
                }));

                let section = rows.reviewers();
                let section = row(section.start, section.end - section.start);
                reviewers.render(buf, section, label_column, theme, under, reviewers_focus);
            },
        );
        // Textual's `Markdown` keeps `open_links` in a dialog, so a click on the preview's links opens them.
        if let Some(sink) = cx.links {
            self.scroll.record_links(sink, &links);
        }
    }
}

impl Dialog for PullRequestDialog {
    type Output = Option<Box<FullPullRequest>>;

    fn kind(&self) -> ModalKind {
        ModalKind::PullRequest
    }

    fn own_action(&self, action: DialogAction) -> Option<PullRequestAction> {
        match action {
            DialogAction::PullRequest(action) => Some(action),
            _ => None,
        }
    }

    /// `tab` and `shift+tab` cycle the fields, skipping Edit PR's disabled branch row; from the sheet they go to the
    /// first field and the last.
    fn move_focus(&mut self, direction: FocusMove) {
        let order = self.order();
        let (Some(&first), Some(&last)) = (order.first(), order.last()) else {
            return;
        };
        let next = match (order.iter().position(|&field| field == self.focus), direction) {
            (Some(index), FocusMove::Next) => order[(index + 1) % order.len()],
            (Some(index), FocusMove::Previous) => order[(index + order.len() - 1) % order.len()],
            (None, FocusMove::Next) => first,
            (None, FocusMove::Previous) => last,
        };
        self.focus_field(next);
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

    fn take_output(&mut self) -> Option<Option<Box<FullPullRequest>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.focus {
            PrField::Sheet => return Vec::new(),
            PrField::Base | PrField::Compare | PrField::Title => FieldKind::TextInput,
            PrField::Draft => FieldKind::Switch,
            // The preview keeps the editor's link: the keys it would type do nothing there instead of reaching the
            // sheet's bindings.
            PrField::Description => FieldKind::TextEditor,
            PrField::Reviewers(focus) => ReviewersField::field_kind(focus),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Switch, BindingId::TextualSwitchToggle) => {
                self.draft.toggle();
                Some(Action::Consumed)
            }
            // Every key of the list's link is the list's, whether or not it changed anything.
            (FieldKind::SelectionList, id) => {
                let outcome = self.reviewers.on_binding(id);
                self.on_reviewers(outcome, false).or(Some(Action::Consumed))
            }
            // `consumes_key` already gave the text fields every key they bind.
            _ => Some(Action::Consumed),
        }
    }

    fn captures_mouse(&self) -> bool {
        self.base.is_selecting()
            || self.compare.is_selecting()
            || self.title.is_selecting()
            || self.body.is_selecting()
            || self.reviewers.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        let field = self.focus;
        match field {
            PrField::Base | PrField::Compare | PrField::Title => {
                let (_, changed) = self.edit_input(field, |input| {
                    input.handle_paste(text);
                    InputOutcome::Changed
                });
                Some(if changed {
                    Self::action(PullRequestAction::Edited)
                } else {
                    Action::Consumed
                })
            }
            PrField::Description => Some(match self.body.handle_paste(text, cx.now()) {
                EditorOutcome::Changed => {
                    self.reveal = true;
                    Self::action(PullRequestAction::Edited)
                }
                _ => Action::Consumed,
            }),
            PrField::Reviewers(ReviewersFocus::Input) => {
                self.reviewers.handle_paste(text);
                Some(Action::Consumed)
            }
            _ => None,
        }
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match response {
            GithubResponse::ListBranches(result) if self.branches_request == Some(id) => {
                self.branches_request = None;
                self.on_branches(result, cx);
                None
            }
            GithubResponse::RequestedReviewers(result) if self.requested_request == Some(id) => {
                self.requested_request = None;
                match result {
                    Ok(logins) => {
                        tracing::debug!("Found existing reviewers: {logins:?}");
                        self.reviewers.add_existing(logins);
                    }
                    // Python's `list_requested_reviewers` logs (its message) and lists nobody.
                    Err(error) => {
                        tracing::error!("Error creating review requests");
                        tracing::debug!("Fetching the requested reviewers for Edit PR failed: {error}");
                    }
                }
                None
            }
            other => {
                let other = self.on_submit_response(id, other, cx)?;
                self.reviewers.on_response(id, other, cx)
            }
        }
    }

    fn on_git_response(&mut self, id: RequestId, response: GitResponse, cx: &mut Cx<'_>) -> Option<GitResponse> {
        match response {
            GitResponse::HasUpstream(has_upstream) if self.upstream_request == Some(id) => {
                self.upstream_request = None;
                self.upstream_warning = !has_upstream;
                None
            }
            other => self.on_branch_git(id, other, cx),
        }
    }

    fn on_timer(&mut self, id: TimerId, cx: &mut Cx<'_>) -> bool {
        id == TimerId::PrDraftSave && self.on_draft_timer(cx)
    }

    fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        self.on_draft_read(id, result, cx)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod edit_tests;

#[cfg(test)]
mod draft_tests;
