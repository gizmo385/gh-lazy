//! Create PR (and, with #283, Edit PR): one dialog with a heading, the Base / Compare / Draft row, a title, a code
//! editor for the description, the Reviewers field and Create / Cancel, all in a scroll area. Python's
//! `CreateOrEditPullRequestModal` around a `CreateOrEditPullRequestContainer`.
//!
//! The whole interior scrolls (the container is the `VerticalScroll`, with `padding: 1`), so the buttons are below
//! the fold until focus or the wheel brings them up. The dialog has no footer of its own.

mod layout;
mod reviewers;

use std::ops::Range;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{Branch, FullPullRequest, Repository};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use self::layout::{BranchColumns, FormRows, Part};
use super::form::FormScroll;
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::info::Scroll;
use crate::components::switch::Switch;
use crate::components::text_editor::{EditorOutcome, EditorVariant, TextEditor};
use crate::components::text_input::{InputOutcome, TextInput};
use crate::effect::{Effect, GitRequest, GitResponse, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Accent, Paint, Theme};

pub use reviewers::{ReviewersField, ReviewersFocus, ReviewersOutcome, removing_toast};

/// `CreateOrEditPullRequestContainer { width: 100 }`, border included.
const WIDTH: u16 = 100;

/// `height: 80%` of the screen less the margins, rounded down as Textual does.
const HEIGHT_PERCENT: u16 = 80;

/// The container's `margin: 1`.
const MARGIN: u16 = 1;

/// The container's `padding: 1`, inside which the content scrolls and the scrollbar sits.
const PADDING: u16 = 1;

const CREATE_HEADING: &str = "New Pull Request";
const WARNING: &str = "Warning:";
const NO_UPSTREAM: &str = " Current local branch has no configured upstream";

/// The dialog's screen bindings: the container's `modal.submit`, then the modal's `modal.close`.
const PR_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestAction {
    /// `modal.submit` or Create: check the fields and create the pull request (#279).
    Submit,
    /// `modal.close` or Cancel: close with nothing.
    Close,
    /// `enter` in the Reviewers input: look the login up.
    AddReviewer,
    /// Title, Base, Compare or the description changed: Python's `Input.Changed` / `TextArea.Changed` handlers,
    /// where drafts (#186) schedule their save.
    Edited,
}

/// What the dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A new pull request.
    Create,
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

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrField {
    /// The scroll area itself, which Textual's `VerticalScroll` makes focusable.
    Scroll,
    Base,
    Compare,
    Draft,
    Title,
    Description,
    Reviewers(ReviewersFocus),
    /// Create (Update in Edit PR).
    Submit,
    Cancel,
}

impl PrField {
    fn button(self) -> Option<Pressed> {
        match self {
            PrField::Submit => Some(Pressed::Submit),
            PrField::Cancel => Some(Pressed::Cancel),
            _ => None,
        }
    }
}

/// The Create PR dialog for one repo. Its output is the created pull request, or `None` when it was closed.
#[derive(Debug)]
pub struct PullRequestDialog {
    repo: Repository,
    mode: Mode,
    ring: FocusRing<PrField>,
    base: TextInput,
    compare: TextInput,
    draft: Switch,
    title: TextInput,
    description: TextEditor,
    reviewers: ReviewersField,
    buttons: ButtonRow,
    scroll: FormScroll,
    /// Whether the current-directory branch has no upstream: the warning row shows.
    upstream_warning: bool,
    /// The branches request, until it answers.
    branches_request: Option<RequestId>,
    /// The upstream check, until it answers.
    upstream_request: Option<RequestId>,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<Box<FullPullRequest>>>,
}

impl PullRequestDialog {
    /// Create PR for `repo`, filled in from `prefill`, with the title focused. It asks at once for the repo's
    /// branches (uncached, so a branch pushed a moment ago is offered; owner decision on #278) and its collaborators,
    /// and whether the current-directory branch, if git named one, has an upstream.
    pub fn create(repo: Repository, prefill: CreatePrefill, cx: &mut Cx<'_>) -> Self {
        let mut base = TextInput::new("Choose a base ref");
        base.set_value(prefill.base);
        let mut compare = TextInput::new("Choose a head ref");
        compare.set_value(prefill.compare.unwrap_or_default());
        let mut reviewers = ReviewersField::new(&cx.config().pull_requests.additional_suggested_pr_reviewers);
        reviewers.load(&repo, cx);
        let branches_request = Some(cx.request(GithubRequest::ListBranchesUncached {
            repo: Box::new(repo.clone()),
        }));
        let upstream_request = cx
            .current_branch()
            .map(str::to_owned)
            .map(|branch| cx.git(GitRequest::HasUpstream { branch }));
        let mut dialog = Self {
            repo,
            mode: Mode::Create,
            ring: FocusRing::new([]),
            base,
            compare,
            draft: Switch::new(false),
            title: TextInput::new("Title"),
            description: TextEditor::new(EditorVariant::Code, &prefill.description),
            reviewers,
            buttons: ButtonRow::new("Create"),
            scroll: FormScroll::default(),
            upstream_warning: false,
            branches_request,
            upstream_request,
            output: None,
        };
        dialog.sync_ring();
        dialog.focus(PrField::Title);
        dialog
    }

    /// The repo the pull request is for.
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// The form as typed.
    pub fn form(&self) -> PullRequestForm {
        PullRequestForm {
            title: self.title.value().to_owned(),
            description: self.description.text(),
            base_ref: self.base.value().to_owned(),
            head_ref: self.compare.value().to_owned(),
            is_draft: self.draft.value(),
            reviewers: self.reviewers.reviewers(),
        }
    }

    /// Fills the form in from `form` (a restored draft, #186), over the prefills. Reviewers already listed stay.
    pub fn restore(&mut self, form: PullRequestForm) {
        self.title.set_value(form.title);
        self.description.set_text(&form.description);
        self.base.set_value(form.base_ref);
        self.compare.set_value(form.head_ref);
        if self.draft.value() != form.is_draft {
            self.draft.toggle();
        }
        self.reviewers.add_existing(form.reviewers);
        self.sync_ring();
    }

    /// Whether the upstream warning shows.
    pub fn shows_upstream_warning(&self) -> bool {
        self.upstream_warning
    }

    fn action(action: PullRequestAction) -> Action {
        Action::Dialog(DialogAction::PullRequest(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => PullRequestAction::Submit,
            Pressed::Cancel => PullRequestAction::Close,
        })
    }

    fn heading(&self) -> &'static str {
        match self.mode {
            Mode::Create => CREATE_HEADING,
        }
    }

    /// The fields in screen order: the Reviewers field's list joins while anyone is listed.
    fn order(&self) -> Vec<PrField> {
        let mut order = vec![
            PrField::Scroll,
            PrField::Base,
            PrField::Compare,
            PrField::Draft,
            PrField::Title,
            PrField::Description,
        ];
        order.extend(
            self.reviewers
                .focus_stops()
                .iter()
                .map(|&focus| PrField::Reviewers(focus)),
        );
        order.extend([PrField::Submit, PrField::Cancel]);
        order
    }

    fn sync_ring(&mut self) {
        let order = self.order();
        self.ring.set_order(order);
    }

    /// The form as it is now.
    fn rows(&self) -> FormRows {
        FormRows::new(self.upstream_warning, self.reviewers.height())
    }

    /// The rows `field` takes, and with its margins, for Textual's scroll when it gains focus.
    fn field_rows(&self, field: PrField, rows: &FormRows) -> Option<(Range<u16>, Range<u16>)> {
        let with_bottom = |range: Range<u16>, below: u16| (range.clone(), range.start..range.end + below);
        match field {
            PrField::Scroll => None,
            PrField::Base | PrField::Compare | PrField::Draft => rows.rows(Part::Branches).map(|r| with_bottom(r, 0)),
            // `#pr_title` and `#pr_description` have `margin-bottom: 1`.
            PrField::Title => rows.rows(Part::Title).map(|r| with_bottom(r, 1)),
            PrField::Description => rows.rows(Part::Description).map(|r| with_bottom(r, 1)),
            PrField::Reviewers(focus) => {
                let top = rows.rows(Part::Reviewers)?.start;
                let inner = self.reviewers.rows(focus);
                Some(with_bottom(top + inner.start..top + inner.end, 0))
            }
            // A button's `margin: 1` fills its row's block.
            PrField::Submit | PrField::Cancel => rows.rows(Part::Buttons).map(|r| (r.start + 1..r.end - 1, r)),
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

    /// Textual's `Length(minimum=1)` on `field`'s input: the red border follows the latest check.
    fn validate(&mut self, field: PrField) -> bool {
        self.input_mut(field).is_none_or(|input| {
            let valid = !input.value().is_empty();
            input.set_invalid(!valid);
            valid
        })
    }

    /// Moves focus with `f`. An input that loses focus is checked (Textual validates on blur); a field that gains it
    /// is scrolled into view, an input selects its text (`select_on_focus`) and the editor starts a new undo batch.
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<PrField>)) {
        self.sync_ring();
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(field) = self.ring.focused().filter(|&field| Some(field) != before) else {
            return;
        };
        if let Some(before) = before {
            self.validate(before);
        }
        match field {
            PrField::Description => self.description.focus_gained(),
            PrField::Reviewers(focus) => self.reviewers.focus_gained(focus),
            other => {
                if let Some(input) = self.input_mut(other) {
                    input.select_all();
                }
            }
        }
        if let Some((rows, with_margin)) = self.field_rows(field, &self.rows()) {
            self.scroll.center(rows, with_margin);
        }
    }

    fn focus(&mut self, field: PrField) {
        self.refocus(|ring| {
            ring.focus(field);
        });
    }

    /// Edits `field`'s input with `edit`, checking it when the text changed (Textual's `Input.Changed`) or was
    /// submitted. Returns the outcome and whether the text changed.
    fn edit(&mut self, field: PrField, edit: impl FnOnce(&mut TextInput) -> InputOutcome) -> (InputOutcome, bool) {
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
                self.focus(PrField::Reviewers(focus));
                Some(Action::Consumed)
            }
            ReviewersOutcome::Removed { login, emptied } => {
                self.sync_ring();
                if emptied {
                    self.focus(PrField::Reviewers(ReviewersFocus::Input));
                } else if clicked {
                    self.focus(PrField::Reviewers(ReviewersFocus::List));
                }
                Some(Action::Notify(removing_toast(&login)))
            }
        }
    }

    /// The one way the dialog closes without a pull request: `modal.close` or Cancel. Requests in flight are
    /// cancelled. Drafts (#186) save here.
    fn close(&mut self, cx: &mut Cx<'_>) {
        for id in self
            .branches_request
            .take()
            .into_iter()
            .chain(self.upstream_request.take())
        {
            cx.effect(Effect::Cancel(id));
        }
        self.reviewers.cancel(cx);
        self.output = Some(None);
    }

    fn on_branches(&mut self, result: Result<Vec<Branch>, GhError>) {
        match result {
            Ok(branches) => {
                let names: Vec<String> = branches.into_iter().map(|branch| branch.name).collect();
                self.base.set_suggestions(names.clone());
                self.compare.set_suggestions(names);
            }
            // Python's worker raises and Textual logs it; the inputs just get no suggestions.
            Err(error) => tracing::debug!("Fetching the branches for Create PR failed: {error}"),
        }
    }

    /// A wheel notch at `(column, row)`: the editor scrolls if it's under the pointer and can, else the form does.
    fn wheel(&mut self, wheel: Wheel, column: u16, row: u16) {
        let rows = match wheel {
            Wheel::Up => -WHEEL_ROWS.cast_signed(),
            Wheel::Down => WHEEL_ROWS.cast_signed(),
            Wheel::Left | Wheel::Right => return,
        };
        let over_editor = self
            .rows()
            .rows(Part::Description)
            .is_some_and(|editor| self.scroll.shows_at(editor, column, row));
        if !(over_editor && self.description.scroll(rows)) {
            self.scroll.wheel(wheel);
        }
    }

    /// A left press in the interior: the field under it takes focus (and the press, for the text fields and the
    /// switch), or the scroll area itself does.
    fn press(&mut self, event: MouseEvent) -> Option<Action> {
        let content = self.scroll.content();
        let row = self
            .scroll
            .content_row(event.row)
            .filter(|_| (content.x..content.right()).contains(&event.column));
        let rows = self.rows();
        let moved = self.scroll.to_content(event);
        let at = Position::new(moved.column, moved.row);
        match row.and_then(|row| rows.part_at(row)) {
            Some(Part::Branches) => {
                let columns = BranchColumns::new(rows.area(Part::Branches, content));
                if columns.base.contains(at) {
                    self.focus(PrField::Base);
                    self.base.press(columns.base, moved.column);
                } else if columns.compare.contains(at) {
                    self.focus(PrField::Compare);
                    self.compare.press(columns.compare, moved.column);
                } else if self.draft.handle_mouse(&moved, columns.draft) {
                    self.focus(PrField::Draft);
                } else {
                    self.focus(PrField::Scroll);
                }
            }
            Some(Part::Title) => {
                self.focus(PrField::Title);
                self.title.press(rows.area(Part::Title, content), moved.column);
            }
            Some(Part::Description) => {
                self.focus(PrField::Description);
                self.description.press(moved.column, moved.row);
            }
            Some(Part::Reviewers) => {
                let outcome = self.reviewers.handle_mouse(&moved, rows.area(Part::Reviewers, content));
                if outcome == ReviewersOutcome::Ignored {
                    self.focus(PrField::Scroll);
                } else {
                    return self.on_reviewers(outcome, true);
                }
            }
            _ => self.focus(PrField::Scroll),
        }
        Some(Action::Consumed)
    }
}

impl Component for PullRequestDialog {
    type Action = PullRequestAction;

    fn bindings(&self) -> &'static [BindingId] {
        PR_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(PullRequestAction::Submit)),
            BindingId::ModalClose => Some(Self::action(PullRequestAction::Close)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        match self.ring.focused() {
            Some(PrField::Base) => self.base.consumes_key(key, keymap),
            Some(PrField::Compare) => self.compare.consumes_key(key, keymap),
            Some(PrField::Title) => self.title.consumes_key(key, keymap),
            Some(PrField::Description) => self.description.consumes_key(key, keymap),
            Some(PrField::Reviewers(focus)) => self.reviewers.consumes_key(focus, key, keymap),
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        let field = self.ring.focused()?;
        match field {
            PrField::Base | PrField::Compare | PrField::Title => {
                let (outcome, changed) =
                    self.edit(field, |input| input.handle_key(&chord, cx.keymap(), cx.clipboard()));
                match outcome {
                    InputOutcome::Copied(text) => Some(Action::Copy(text)),
                    InputOutcome::Ignored => None,
                    // `enter` only checks the input; submitting is `modal.submit` or Create.
                    InputOutcome::Changed | InputOutcome::Submitted if changed => {
                        Some(Self::action(PullRequestAction::Edited))
                    }
                    InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
                }
            }
            PrField::Description => match self
                .description
                .handle_key(&chord, cx.keymap(), cx.clipboard(), cx.now())
            {
                EditorOutcome::Copied { text, .. } => Some(Action::Copy(text)),
                EditorOutcome::Changed => Some(Self::action(PullRequestAction::Edited)),
                EditorOutcome::Moved | EditorOutcome::Ignored => Some(Action::Consumed),
            },
            PrField::Reviewers(focus) => {
                let outcome = self.reviewers.handle_key(focus, &chord, cx.keymap(), cx.clipboard());
                self.on_reviewers(outcome, false)
            }
            PrField::Scroll | PrField::Draft | PrField::Submit | PrField::Cancel => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // A drag that started in a text field, wherever the pointer is now.
        let moved = self.scroll.to_content(event);
        if self.base.captured_mouse(moved)
            || self.compare.captured_mouse(moved)
            || self.title.captured_mouse(moved)
            || self.description.captured_mouse(moved)
            || self.reviewers.captured_mouse(moved)
        {
            return Some(Action::Consumed);
        }
        if self.scroll.content_row(event.row).is_some()
            && let Some((button, outcome)) = self.buttons.handle_mouse(&moved, cx.now())
        {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    self.focus(match button {
                        Pressed::Submit => PrField::Submit,
                        Pressed::Cancel => PrField::Cancel,
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            self.wheel(wheel, event.column, event.row);
        } else if event.kind == MouseEventKind::Down(MouseButton::Left) {
            return self.press(event);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: PullRequestAction, cx: &mut Cx<'_>) {
        match action {
            PullRequestAction::Submit => tracing::debug!("Create PR's submit comes with #279"),
            PullRequestAction::Close => self.close(cx),
            PullRequestAction::AddReviewer => self.reviewers.submit(cx),
            // Drafts (#186) schedule their save here.
            PullRequestAction::Edited => {}
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let fill = Style::new()
            .fg(theme.color(theme.palette.foreground, under))
            .bg(theme.paint(under));
        let inner = area.inner(Margin::new(PADDING, PADDING));
        let rows = self.rows();
        let heading = self.heading();
        let warning = self.upstream_warning;
        let focused = self.ring.focused();
        let Self {
            scroll,
            base,
            compare,
            draft,
            title,
            description,
            reviewers,
            buttons,
            ..
        } = self;
        let mut form = Form {
            base,
            compare,
            draft,
            title,
            description,
            reviewers,
            buttons,
        };
        scroll.render(frame.buffer_mut(), inner, theme, fill, rows.height(), |buf, content| {
            draw_static(buf, content, &rows, theme, under, heading, warning);
            form.render(buf, content, &rows, theme, under, focused);
        });
    }
}

/// The fields, borrowed for drawing into the scroll area's content.
struct Form<'a> {
    base: &'a mut TextInput,
    compare: &'a mut TextInput,
    draft: &'a Switch,
    title: &'a mut TextInput,
    description: &'a mut TextEditor,
    reviewers: &'a mut ReviewersField,
    buttons: &'a mut ButtonRow,
}

impl Form<'_> {
    fn render(
        &mut self,
        buf: &mut Buffer,
        content: Rect,
        rows: &FormRows,
        theme: &Theme,
        under: Paint,
        focused: Option<PrField>,
    ) {
        let is = |field: PrField| focused == Some(field);
        let columns = BranchColumns::new(rows.area(Part::Branches, content));
        self.base.render(buf, columns.base, theme, under, is(PrField::Base));
        self.compare
            .render(buf, columns.compare, theme, under, is(PrField::Compare));
        self.draft.render(buf, columns.draft, theme, under, is(PrField::Draft));
        self.title
            .render(buf, rows.area(Part::Title, content), theme, under, is(PrField::Title));
        self.description.render(
            buf,
            rows.area(Part::Description, content),
            theme,
            under,
            is(PrField::Description),
        );
        let reviewers_focus = match focused {
            Some(PrField::Reviewers(focus)) => Some(focus),
            _ => None,
        };
        self.reviewers
            .render(buf, rows.area(Part::Reviewers, content), theme, under, reviewers_focus);
        self.buttons.render(
            buf,
            rows.area(Part::Buttons, content),
            theme,
            focused.and_then(PrField::button),
        );
    }
}

/// Draws the heading, the warning, the labels and the rules into `content`.
fn draw_static(
    buf: &mut Buffer,
    content: Rect,
    rows: &FormRows,
    theme: &Theme,
    under: Paint,
    heading: &str,
    warning: bool,
) {
    let p = theme.palette;
    let text = Style::new().fg(theme.color(p.foreground, under));
    let bold = text.add_modifier(Modifier::BOLD);

    // `MarkdownH1`: centered inside the `Markdown` widget's padding, in `$markdown-h1-color` and its text style.
    let block = rows.area(Part::Heading, content);
    let h1 = p
        .markdown_h1_text_style
        .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
    let width = usize::from(content.width.saturating_sub(2 * layout::HEADING_PADDING));
    buf.set_stringn(
        content.x + layout::HEADING_PADDING,
        block.y + layout::HEADING_ROW,
        format!("{heading:^width$}"),
        width,
        h1,
    );

    if warning {
        // `[yellow]Warning:[/yellow] ...`, centered across the content (`content-align: center middle`).
        let area = rows.area(Part::Warning, content);
        let line = Line::from(vec![
            Span::styled(WARNING, Style::new().fg(theme.accent(Accent::Yellow))),
            Span::styled(NO_UPSTREAM, text),
        ]);
        let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
        let x = content.x + content.width.saturating_sub(width) / 2;
        buf.set_line(x, area.y, &line, content.width);
    }

    // The branch row's labels sit on its middle row (`padding-top: 1`).
    let columns = BranchColumns::new(rows.area(Part::Branches, content));
    let middle = columns.base_label.y + 1;
    buf.set_stringn(columns.base_label.x, middle, layout::BASE_LABEL, 4, bold);
    buf.set_line(
        columns.compare_label.x,
        middle,
        &Line::from(vec![
            Span::styled(layout::COMPARE_ARROW, text),
            Span::styled(layout::COMPARE_LABEL, bold),
        ]),
        columns.compare_label.width,
    );
    buf.set_stringn(
        columns.draft_label.x,
        middle,
        layout::DRAFT_LABEL,
        usize::from(columns.draft_label.width),
        text,
    );

    let width = usize::from(content.width);
    for (part, label) in [(Part::TitleLabel, "Title"), (Part::DescriptionLabel, "Description")] {
        buf.set_stringn(content.x, rows.area(part, content).y, label, width, bold);
    }
    // `Rule { color: $secondary }`, with its `margin: 1 0`.
    let rule = Style::new().fg(theme.color(p.secondary, under));
    for part in [Part::FirstRule, Part::SecondRule] {
        let y = rows.area(part, content).y + 1;
        buf.set_stringn(content.x, y, "─".repeat(width), width, rule);
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

    fn move_focus(&mut self, direction: FocusMove) {
        self.refocus(|ring| ring.step(direction));
    }

    /// `height: 80%` of the whole screen less the margins: the modal screen has no footer, so it's as tall as the
    /// terminal, while the main screen's footer shows through on the bottom row.
    fn size(&self, available: Rect) -> (u16, u16) {
        (
            WIDTH,
            available.height.saturating_sub(2 * MARGIN) * HEIGHT_PERCENT / 100,
        )
    }

    /// Centered with its margins on the whole screen, as Textual's `align: center middle` places it on a modal screen
    /// without a footer. A frame wider than the screen starts `MARGIN` in and runs past the right edge.
    fn frame(&self, area: Rect) -> Rect {
        let (width, height) = self.size(area);
        Rect {
            x: area.x + area.width.saturating_sub(width + 2 * MARGIN) / 2 + MARGIN,
            y: area.y + area.height.saturating_sub(height + 2 * MARGIN) / 2 + MARGIN,
            width,
            height,
        }
    }

    fn shows_footer(&self) -> bool {
        false
    }

    fn take_output(&mut self) -> Option<Option<Box<FullPullRequest>>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(PrField::Scroll) => FieldKind::Scroll,
            Some(PrField::Base | PrField::Compare | PrField::Title) => FieldKind::TextInput,
            Some(PrField::Draft) => FieldKind::Switch,
            Some(PrField::Description) => FieldKind::TextEditor,
            Some(PrField::Reviewers(focus)) => ReviewersField::field_kind(focus),
            Some(PrField::Submit | PrField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(PrField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            (FieldKind::Scroll, id) => {
                let scroll = Scroll::from_binding(id)?;
                self.scroll.scroll(scroll);
                Some(Action::Consumed)
            }
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
            || self.description.is_selecting()
            || self.reviewers.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, cx: &Cx<'_>) -> Option<Action> {
        let field = self.ring.focused()?;
        match field {
            PrField::Base | PrField::Compare | PrField::Title => {
                let (_, changed) = self.edit(field, |input| {
                    input.handle_paste(text);
                    InputOutcome::Changed
                });
                Some(if changed {
                    Self::action(PullRequestAction::Edited)
                } else {
                    Action::Consumed
                })
            }
            PrField::Description => Some(match self.description.handle_paste(text, cx.now()) {
                EditorOutcome::Changed => Self::action(PullRequestAction::Edited),
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
                self.on_branches(result);
                None
            }
            other => {
                let unclaimed = self.reviewers.on_response(id, other, cx);
                // A found reviewer shows the list, which joins the focus order.
                self.sync_ring();
                unclaimed
            }
        }
    }

    fn on_git_response(&mut self, id: RequestId, response: GitResponse, _cx: &mut Cx<'_>) -> Option<GitResponse> {
        match response {
            GitResponse::HasUpstream(has_upstream) if self.upstream_request == Some(id) => {
                self.upstream_request = None;
                self.upstream_warning = !has_upstream;
                None
            }
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests;
