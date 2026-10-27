//! The main screen: the status box on top, the four selection sections on the left, details (and the optional
//! command log) on the right, and the footer row. Python's `LazyGithubMainScreen`.

mod details;
mod issue_details;
mod issue_lists;
mod issue_lookup;
mod issues;
pub mod layout;
mod list;
mod overview_text;
mod pr_details;
mod pr_lookup;
mod profile;
mod pull_requests;
mod repo_overview;
mod repos;
mod run_details;
mod section_table;
mod sections;
mod workflows;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::AppearanceSettings;
use lazy_github_core::gh::GhError;
use lazy_github_core::models::{FullPullRequest, Issue, IssueListItem, Repository, WorkflowRun};
use ratatui::Frame;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear};
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, Owner, RenderCx, ScreenKind};
use crate::components::command_log::{CommandLog, Wheel};
use crate::components::hatch::Hatch;
use crate::components::info::{INFO_BINDINGS, Scroll};
use crate::components::table::{Handled, SEARCHABLE_BINDINGS, TABLE_BINDINGS, TableFocus};
use crate::components::text_input::{self, INPUT_BINDINGS};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::{LookupDialog, RepoLookup};
use crate::options::AppOptions;
use crate::theme::Theme;
use crate::toast::Toast;

pub use details::TabSwitch;
use details::{DetailsPane, details_title};
use issue_lists::IssueLists;
use issue_lookup::IssueLookup;
use layout::{LayoutInput, layout};
pub use pr_details::PullRequestTarget;
use pr_lookup::{PrLookup, select_repository_first};
use profile::ProfileFetch;
pub use profile::ProfileOrigin;
pub use repos::{RepoCells, ReposSection};
use section_table::{SectionEvent, SectionTable, earliest_deadline};
use sections::{EMPTY_SUFFIX, titled_box};
use workflows::WorkflowsPane;

/// The four selection sections, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionId {
    Repos,
    PullRequests,
    Issues,
    Workflows,
}

impl SectionId {
    /// Every section, in screen order.
    pub const ALL: [SectionId; 4] = [
        SectionId::Repos,
        SectionId::PullRequests,
        SectionId::Issues,
        SectionId::Workflows,
    ];

    /// The position in [`Self::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The binding that focuses it, whose key goes in its title.
    pub fn focus_binding(self) -> BindingId {
        match self {
            SectionId::Repos => BindingId::MainReposFocus,
            SectionId::PullRequests => BindingId::MainPullRequestsFocus,
            SectionId::Issues => BindingId::MainIssuesFocus,
            SectionId::Workflows => BindingId::MainWorkflowsFocus,
        }
    }

    /// The bindings of the section's container, in Python's `BINDINGS` order.
    fn bindings(self) -> &'static [BindingId] {
        match self {
            SectionId::Repos => &[
                BindingId::RepositoriesFavorite,
                BindingId::RepositoriesLookup,
                BindingId::UserViewProfile,
            ],
            SectionId::PullRequests => &[
                BindingId::PullRequestLookup,
                BindingId::PullRequestEdit,
                BindingId::UserViewProfile,
            ],
            SectionId::Issues => &[BindingId::IssueLookup, BindingId::IssueEdit, BindingId::UserViewProfile],
            SectionId::Workflows => &[BindingId::WorkflowsTrigger],
        }
    }

    /// The title after the key.
    pub fn label(self) -> &'static str {
        match self {
            SectionId::Repos => "Repositories",
            SectionId::PullRequests => "Pull Requests",
            SectionId::Issues => "Issues",
            SectionId::Workflows => "Workflows",
        }
    }
}

/// What has focus on the main screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    /// A section's table.
    Table(SectionId),
    /// A section's search input (#78).
    Search(SectionId),
    /// The details tab strip.
    DetailsTabs,
    /// The content of the active details tab.
    DetailsContent,
    /// The command log.
    CommandLog,
}

impl FocusTarget {
    /// The section focus is within, if any.
    pub fn section(self) -> Option<SectionId> {
        match self {
            FocusTarget::Table(id) | FocusTarget::Search(id) => Some(id),
            FocusTarget::DetailsTabs | FocusTarget::DetailsContent | FocusTarget::CommandLog => None,
        }
    }

    /// Whether `M` maximizes anything with `self` focused. Textual's `Screen.maximize` decides by the focused
    /// widget's `ALLOW_MAXIMIZE`, then maximizes its nearest maximizable ancestor, else the widget itself:
    ///
    /// - a table (`DataTable`), the search box (`Input`) and the command log (`Log`) are `ScrollView`s, which allow
    ///   it, and have no maximizable ancestor. The search box types `M`, so only a maximize rebound to a key it
    ///   doesn't type reaches it (Py bug B38, #156), and that maximizes the box alone;
    /// - the details tab strip (`Tabs`) is focusable, so allowed, and maximizes its `TabbedContent`;
    /// - the details content is a `VerticalScroll`, and `ScrollableContainer` sets `ALLOW_MAXIMIZE = False`, so `M`
    ///   there does nothing.
    pub fn maximizable(self) -> bool {
        !matches!(self, FocusTarget::DetailsContent)
    }

    fn in_details(self) -> bool {
        matches!(self, FocusTarget::DetailsTabs | FocusTarget::DetailsContent)
    }

    /// Whether `self` is inside the widget that maximizing `maximized` shows: the table or the search box itself
    /// (the box is outside the `DataTable`, and the table outside the `Input`), the details tab area, or the
    /// command log.
    fn within_maximized(self, maximized: FocusTarget) -> bool {
        match (self.section(), maximized.section()) {
            (Some(_), Some(_)) => self == maximized,
            (None, None) => self.in_details() == maximized.in_details(),
            _ => false,
        }
    }
}

/// How one section takes part in the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionState {
    /// From `appearance.show_*` at startup.
    pub displayed: bool,
    /// Hidden by a runtime toggle (M4; see #75).
    pub hidden_by_toggle: bool,
    /// Empty, drawn three rows high with ` (empty)` in the title (M2).
    pub compact: bool,
}

impl SectionState {
    /// Whether the section has a box on screen.
    pub fn shown(self) -> bool {
        self.displayed && !self.hidden_by_toggle
    }
}

/// `MainViewPane.BINDINGS`: the focus keys.
pub const MAIN_VIEW_BINDINGS: &[BindingId] = &[
    BindingId::MainReposFocus,
    BindingId::MainPullRequestsFocus,
    BindingId::MainIssuesFocus,
    BindingId::MainWorkflowsFocus,
    BindingId::MainDetailsFocus,
    BindingId::MainCommandLogFocus,
];

/// `SelectionsPane.BINDINGS`.
pub const SELECTIONS_BINDINGS: &[BindingId] = &[BindingId::IssueNew, BindingId::PullRequestNew];

/// Textual's `Screen` built-ins, then `LazyGithubMainScreen.BINDINGS`: Textual merges a subclass's bindings after
/// the ones it inherits, and the help panel lists them in that order.
pub const SCREEN_BINDINGS: &[BindingId] = &[
    BindingId::TextualFocusNext,
    BindingId::TextualFocusPrevious,
    BindingId::TextualCopyText,
    BindingId::NotificationsOpen,
    BindingId::LinkPaste,
    BindingId::LinkFollow,
    BindingId::LinkFollowExternal,
];

/// Textual's `Tabs` bindings.
pub const TABS_BINDINGS: &[BindingId] = &[BindingId::TextualTabsPrevious, BindingId::TextualTabsNext];

/// Textual's `ScrollableContainer` bindings, which the command log's `Log` has.
pub const LOG_BINDINGS: &[BindingId] = &[
    BindingId::TextualScrollUp,
    BindingId::TextualScrollDown,
    BindingId::TextualScrollLeft,
    BindingId::TextualScrollRight,
    BindingId::TextualScrollHome,
    BindingId::TextualScrollEnd,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollPageDown,
    BindingId::TextualScrollPageLeft,
    BindingId::TextualScrollPageRight,
];

/// The main screen.
#[derive(Debug)]
pub struct MainScreen {
    sections: [SectionState; 4],
    focus: FocusTarget,
    // Built once, as Python does: a Settings rebind doesn't refresh them.
    section_titles: [String; 4],
    details_title: String,
    command_log_title: String,
    details: DetailsPane,
    command_log: CommandLog,
    repos: ReposSection,
    issue_lists: IssueLists,
    workflows: WorkflowsPane,
    profile: ProfileFetch,
    /// Where each section's table component was drawn in the last frame, by [`SectionId::index`]: the box's
    /// interior, or the maximized rect. `None` when it wasn't drawn.
    table_areas: [Option<Rect>; 4],
    /// Where each focus target was drawn last, for clicks: the interiors of the boxes, not their borders.
    hits: Vec<(Rect, FocusTarget)>,
    /// The selected pull request's request, while it's in flight. Only the latest selection is ever shown.
    pending_pr: Option<PendingPullRequest>,
}

/// A pull request selection waiting for the full pull request.
#[derive(Debug, Clone, Copy)]
struct PendingPullRequest {
    id: RequestId,
    /// Whether the tab strip takes focus once it shows.
    focus: bool,
}

/// The main screen's own actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainAction {
    /// Scroll the details content (`textual.scroll.*` while it has focus).
    ScrollDetails(Scroll),
    /// Scroll the command log (`textual.scroll.*` while it has focus).
    ScrollLog(Scroll),
    /// A wheel notch over the command log, focused or not.
    WheelLog(Wheel),
    /// A wheel notch over the details content, focused or not.
    WheelDetails(Wheel),
    /// A click on the repos table's cursor cell: focus the table, then select the repo, in that order.
    ClickSelect(Box<Repository>),
    /// Favorite or unfavorite the repo under the repos table's cursor (`repositories.favorite`).
    ToggleFavorite,
    /// Open the Lookup repository dialog (`repositories.lookup`).
    LookupRepository,
    /// The Lookup repository dialog closed: with the repo it found, or `None` when it was cancelled.
    RepoLookedUp(Option<Box<Repository>>),
    /// Open the Lookup pull request dialog (`pull_request.lookup`), or toast without a selected repo.
    LookupPullRequest,
    /// The Lookup pull request dialog found a pull request. A cancelled lookup sends nothing.
    PullRequestLookedUp(Box<FullPullRequest>),
    /// Open the Lookup issue dialog (`issue.lookup`), or toast without a selected repo.
    LookupIssue,
    /// The Lookup issue dialog found an issue, or a pull request by its number. A cancelled lookup sends nothing.
    IssueLookedUp(Box<IssueListItem>),
    /// `user.view_profile` in a table: open the profile of the highlighted row's user.
    ViewProfile(ProfileOrigin),
    /// The User profile dialog closed, with the repo picked there, if any.
    ProfilePicked(ProfileOrigin, Option<Box<Repository>>),
    /// Show a pull request's details: fetch it first if `target` is a table row. `focus` moves focus to the tab
    /// strip once they show (Python's `focus_pr_details`, false only for M4's preload, #198).
    OpenPullRequest {
        /// The pull request.
        target: PullRequestTarget,
        /// Whether the tab strip takes focus.
        focus: bool,
    },
    /// Show an issue's details, focusing the tab strip: Python's `IssueSelected`, from the Issues table, the issue
    /// lookup (#247) and, later, links.
    OpenIssue(Box<Issue>),
    /// Show a workflow run's details and request its jobs, focusing the tab strip: Python's
    /// `handle_workflow_run_selection`.
    OpenWorkflowRun(Box<WorkflowRun>),
    /// A binding inside the details content whose action makes requests (`workflows.view_job_logs`): the body runs
    /// it here, with the context.
    DetailsCommand(Owner, BindingId),
    /// Switch the details tabs (`textual.tabs.*` with the strip focused).
    SwitchDetailsTab(TabSwitch),
    /// A click on a details tab: focus the strip, then activate the tab.
    ClickDetailsTab(usize),
    /// A section's lazy table asked for `page` (a cursor key, a click or a search submit): its section requests it.
    LoadMore {
        /// The section whose table asked.
        section: SectionId,
        /// The table's generation when it asked.
        generation: u64,
        /// The page to fetch, from 1.
        page: u32,
        /// Items per page.
        per_page: u32,
    },
}

impl MainScreen {
    /// The main screen for the `appearance` settings read at startup, with titles from `keymap`, the repos table's
    /// header colored by `theme` and the startup `options`. The repos table has focus, as Textual focuses the first
    /// focusable widget. [`Self::start`] starts loading it.
    pub fn new(appearance: &AppearanceSettings, keymap: &Keymap, theme: &Theme, options: AppOptions) -> Self {
        let displayed = [
            true,
            appearance.show_pull_requests,
            appearance.show_issues,
            appearance.show_workflows,
        ];
        let title = |id: BindingId, label: &str| format!("[{}] {label}", keymap.title_key(id));
        Self {
            sections: displayed.map(|displayed| SectionState {
                displayed,
                hidden_by_toggle: false,
                compact: false,
            }),
            focus: FocusTarget::Table(SectionId::Repos),
            section_titles: SectionId::ALL.map(|id| title(id.focus_binding(), id.label())),
            details_title: title(BindingId::MainDetailsFocus, "Details"),
            command_log_title: title(BindingId::MainCommandLogFocus, "Command Log"),
            details: DetailsPane::default().with_options(options),
            command_log: CommandLog::new(appearance.show_command_log),
            repos: ReposSection::new(theme, options),
            issue_lists: IssueLists::new(options),
            workflows: WorkflowsPane::new(options),
            profile: ProfileFetch::default(),
            table_areas: [None; 4],
            hits: Vec::new(),
            pending_pr: None,
        }
    }

    /// What the screen does once it's shown: load the repos, under the loading overlay.
    pub fn start(&mut self, cx: &mut Cx<'_>) {
        self.repos.start(cx);
    }

    /// A repo was selected (`Action::SelectRepo`, applied by `App`): every pane shows it, in Python's order (the
    /// sections, then the details). A pane that loads per repo adds one line here; see #170's "What a selection
    /// publishes" for the contract of its `load_repo`.
    pub fn on_repo_selected(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        self.cancel_pending_selection(cx);
        if self.section(SectionId::PullRequests).shown() || self.section(SectionId::Issues).shown() {
            self.issue_lists.load_repo(repo, cx);
        }
        if self.section(SectionId::Workflows).shown() {
            self.workflows.load_repo(repo, cx);
        }
        self.details.show_repo(repo, cx);
        self.details_shown(&repo.full_name, false, cx);
    }

    /// The details pane shows a new selection labelled `label` (`PR #5`): the title follows, and focus moves to the
    /// tab strip when `focus` says so, or when it was in the content that just went away (Python removes the focused
    /// content along with the old tab, and Textual moves focus to the strip). Every selection that replaces the
    /// details tabs ends here.
    fn details_shown(&mut self, label: &str, focus: bool, cx: &Cx<'_>) {
        self.details_title = details_title(cx.keymap(), label);
        if focus || self.focus == FocusTarget::DetailsContent {
            self.move_focus(FocusTarget::DetailsTabs);
        }
    }

    /// A new selection replaces any still loading: the pending pull request request is cancelled, and its answer, if
    /// it comes anyway, is dropped. Every selection calls this first.
    fn cancel_pending_selection(&mut self, cx: &mut Cx<'_>) {
        if let Some(pending) = self.pending_pr.take() {
            cx.effect(Effect::Cancel(pending.id));
        }
    }

    /// `MainAction::OpenPullRequest`: a table row is fetched first, and nothing changes until it arrives; a full pull
    /// request shows at once. Python's `MainViewPane.load_pull_request`.
    fn open_pull_request(&mut self, target: PullRequestTarget, focus: bool, cx: &mut Cx<'_>) {
        self.cancel_pending_selection(cx);
        match target {
            PullRequestTarget::Partial(pr) => {
                let id = cx.request(GithubRequest::PullRequest {
                    repo: Box::new(pr.issue.repo),
                    number: pr.issue.number,
                });
                self.pending_pr = Some(PendingPullRequest { id, focus });
            }
            PullRequestTarget::Full(pr) => self.show_pull_request(*pr, focus, cx),
        }
    }

    fn show_pull_request(&mut self, pr: FullPullRequest, focus: bool, cx: &mut Cx<'_>) {
        let label = format!("PR #{}", pr.number());
        self.details.show_pull_request(pr, cx);
        self.details_shown(&label, focus, cx);
    }

    /// `MainAction::OpenIssue`: the issue comes whole from where it was picked, so it shows at once. Python's
    /// `MainViewPane.load_issue`.
    fn open_issue(&mut self, issue: Issue, cx: &mut Cx<'_>) {
        self.cancel_pending_selection(cx);
        let label = format!("Issue #{}", issue.number);
        self.details.show_issue(issue, cx);
        self.details_shown(&label, true, cx);
    }

    /// `MainAction::OpenWorkflowRun`: the run's tabs show at once, and its jobs load. Python's
    /// `MainViewPane.load_workflow_run`.
    fn open_workflow_run(&mut self, run: WorkflowRun, cx: &mut Cx<'_>) {
        self.cancel_pending_selection(cx);
        let label = format!("Workflow Run #{}", run.run_number);
        self.details.show_run(run, cx);
        self.details_shown(&label, true, cx);
    }

    /// Takes the answer to the pending pull request selection, and hands back any other response. A failure leaves
    /// everything as it was, with Python's log line and toast; an answer to a selection since replaced is dropped.
    fn on_pull_request(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let GithubResponse::PullRequest(result) = response else {
            return Some(response);
        };
        let Some(pending) = self.pending_pr.filter(|pending| pending.id == id) else {
            tracing::debug!("Dropping the pull request response {id} of a selection no longer current");
            return None;
        };
        self.pending_pr = None;
        match result {
            Ok(pr) => self.show_pull_request(*pr, pending.focus, cx),
            Err(error) => pull_request_failed(&error, cx),
        }
        None
    }

    /// Git said what the current-directory repo is: the repos section may select it.
    pub fn on_local_repo(&mut self, cx: &mut Cx<'_>) {
        self.repos.check_current_directory_repo(cx);
    }

    /// The details box's title.
    pub fn details_title(&self) -> &str {
        &self.details_title
    }

    /// The details tab names, in order.
    pub fn details_tabs(&self) -> &[String] {
        self.details.tabs().names()
    }

    /// The repo the details pane shows, if any.
    pub fn details_repo(&self) -> Option<&Repository> {
        self.details.repo()
    }

    /// The pull request the details pane shows, if any.
    pub fn details_pull_request(&self) -> Option<&FullPullRequest> {
        self.details.pull_request().map(pr_details::PullRequestDetails::pr)
    }

    /// The issue the details pane shows, if any.
    pub fn details_issue(&self) -> Option<&Issue> {
        self.details.issue().map(issue_details::IssueDetails::issue)
    }

    /// The workflow run the details pane shows, if any.
    pub fn details_run(&self) -> Option<&WorkflowRun> {
        self.details.run().map(run_details::RunDetails::run)
    }

    /// Whether `M` maximizes something with the current focus: see [`FocusTarget::maximizable`]. Inside the details
    /// content, a focused button allows it, and maximizes the details tab area.
    pub fn can_maximize(&self) -> bool {
        self.focus.maximizable() || (self.focus == FocusTarget::DetailsContent && self.details.maximizable())
    }

    /// Takes a response to one of the screen's requests, and hands back any other: the repos section is offered it
    /// first, then each section's pane.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let response = self.profile.on_response(id, response, cx)?;
        let response = self.repos.on_response(id, response, cx)?;
        let response = self.issue_lists.on_response(id, response, cx)?;
        let response = self.workflows.on_response(id, response, cx)?;
        let response = self.on_pull_request(id, response, cx)?;
        self.details.on_response(id, response, cx)
    }

    /// Takes the result of one of the screen's cache reads, and hands back any other.
    pub fn on_cache_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        let result = self.repos.on_cache_read(id, result, cx)?;
        let result = self.issue_lists.on_cache_read(id, result, cx)?;
        self.workflows.on_cache_read(id, result, cx)
    }

    /// The repos section.
    pub fn repos(&self) -> &ReposSection {
        &self.repos
    }

    /// The next animation frame on the screen, if anything animates: the earliest over every table and the details.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let tables = earliest_deadline(SectionId::ALL.into_iter().map(|id| self.table(id)), now);
        [tables, self.details.next_deadline(now)].into_iter().flatten().min()
    }

    /// What has focus.
    pub fn focus(&self) -> FocusTarget {
        self.focus
    }

    /// Moves focus, without the rules of [`Self::try_focus`]. A hidden search box still can't take it.
    pub fn set_focus(&mut self, focus: FocusTarget) {
        self.move_focus(focus);
    }

    /// The one place focus changes, which keeps the table's own [`TableFocus`] in step with [`FocusTarget`]: focus
    /// leaving `Search(id)` blurs the box (a blank one hides), and focus entering it focuses the box, selecting its
    /// text. A hidden box takes no focus, so the move doesn't happen and this returns false.
    fn move_focus(&mut self, target: FocusTarget) -> bool {
        if target == self.focus {
            return true;
        }
        if let FocusTarget::Search(id) = target
            && !self.table_mut(id).focus_search()
        {
            return false;
        }
        if let FocusTarget::Search(id) = self.focus {
            self.table_mut(id).focus_table();
        }
        self.focus = target;
        true
    }

    /// The section's table.
    fn table_mut(&mut self, id: SectionId) -> &mut dyn SectionTable {
        match id {
            SectionId::Repos => &mut self.repos.table,
            SectionId::PullRequests => self.issue_lists.prs_table_mut(),
            SectionId::Issues => self.issue_lists.issues_table_mut(),
            SectionId::Workflows => self.workflows.runs_table_mut(),
        }
    }

    fn table(&self, id: SectionId) -> &dyn SectionTable {
        match id {
            SectionId::Repos => &self.repos.table,
            SectionId::PullRequests => self.issue_lists.prs_table(),
            SectionId::Issues => self.issue_lists.issues_table(),
            SectionId::Workflows => self.workflows.runs_table(),
        }
    }

    /// Whether a section is compact: a list section's own flag, as of its last `refresh_compact`, else the stored
    /// one (the repos section's).
    fn compact(&self, id: SectionId) -> bool {
        match id {
            SectionId::PullRequests => self.issue_lists.prs_compact(),
            SectionId::Issues => self.issue_lists.issues_compact(),
            SectionId::Workflows => self.workflows.compact(),
            SectionId::Repos => self.sections[id.index()].compact,
        }
    }

    /// Requests the page a section's table asked for.
    fn load_more(&mut self, section: SectionId, generation: u64, page: u32, per_page: u32, cx: &mut Cx<'_>) {
        match section {
            SectionId::PullRequests => self.issue_lists.load_more_prs(generation, page, per_page, cx),
            SectionId::Issues => self.issue_lists.load_more_issues(generation, page, per_page, cx),
            SectionId::Workflows => self.workflows.load_more(generation, page, per_page, cx),
            // The repos table isn't lazy.
            SectionId::Repos => {}
        }
    }

    /// Where the table itself (not its search box) was drawn in the last frame, which takes clicks.
    pub(crate) fn drawn_table_rect(&self, id: SectionId) -> Option<Rect> {
        let area = self.table_areas[id.index()]?;
        Some(self.table(id).drawn_table_rect(area))
    }

    /// `common.table.search`: shows the section's search box and focuses it, keeping its text.
    fn open_search(&mut self, id: SectionId) {
        self.table_mut(id).open_search();
        self.move_focus(FocusTarget::Search(id));
    }

    /// Follows a change the table made to its own focus (a submit, a click), through [`Self::move_focus`].
    fn sync_table_focus(&mut self, id: SectionId) {
        let target = match self.table(id).focus() {
            TableFocus::Search => FocusTarget::Search(id),
            TableFocus::Table if self.focus == FocusTarget::Search(id) => FocusTarget::Table(id),
            TableFocus::Table => return,
        };
        self.move_focus(target);
    }

    /// Step 4 of key dispatch: a key the focused search box takes before the binding chain. `None` passes it on.
    pub fn handle_focused_key(&mut self, key: &KeyChord, keymap: &Keymap) -> Option<Action> {
        // An input inside the details content (the diff viewer's file filter) takes its keys the same way.
        if self.focus == FocusTarget::DetailsContent {
            return self.details.handle_focused_key(key, keymap);
        }
        let FocusTarget::Search(id) = self.focus else {
            return None;
        };
        let table = self.table_mut(id);
        if !table.consumes_key(key, keymap) {
            return None;
        }
        let handled = table.handle_key(key, keymap);
        // A submit moved the table's focus to the table.
        self.sync_table_focus(id);
        Some(table_outcome(id, handled).unwrap_or(Action::Consumed))
    }

    /// A bracketed paste, which only a focused text input takes: a search box, or the diff viewer's file filter.
    pub fn handle_paste(&mut self, text: &str) -> Option<Action> {
        if self.focus == FocusTarget::DetailsContent {
            return self.details.handle_paste(text).then_some(Action::Consumed);
        }
        let FocusTarget::Search(id) = self.focus else {
            return None;
        };
        let handled = self.table_mut(id).handle_paste(text);
        handled.is_handled().then_some(Action::Consumed)
    }

    /// The state of one section.
    pub fn section(&self, id: SectionId) -> SectionState {
        SectionState {
            compact: self.compact(id),
            ..self.sections[id.index()]
        }
    }

    /// Every section's state, by [`SectionId::index`].
    fn section_states(&self) -> [SectionState; 4] {
        SectionId::ALL.map(|id| self.section(id))
    }

    /// Hides or shows a section by a runtime toggle. M4's command palette drives this.
    pub fn set_hidden_by_toggle(&mut self, id: SectionId, hidden: bool) {
        self.sections[id.index()].hidden_by_toggle = hidden;
    }

    /// The command log, which keeps every line pushed since the screen was built, shown or not.
    pub fn command_log(&self) -> &CommandLog {
        &self.command_log
    }

    /// The command log, to push lines and to show or hide it (the palette's toggle, M4).
    pub fn command_log_mut(&mut self) -> &mut CommandLog {
        &mut self.command_log
    }

    /// Moves focus to `target` if Python would, and says whether it did.
    ///
    /// Python's `action_focus_section` checks `visible`, which only a runtime toggle (and the command log's
    /// `show_command_log`) clears; config hides sections with `display` alone, so `2` still focuses a PR section
    /// that config hides (Py bug B31). `action_focus_workflow_tabs` doesn't check at all, and `action_focus_tabs`
    /// needs a tab.
    pub fn try_focus(&mut self, target: FocusTarget) -> bool {
        let allowed = match target {
            FocusTarget::Table(SectionId::Workflows)
            | FocusTarget::Search(SectionId::Workflows)
            | FocusTarget::DetailsContent => true,
            FocusTarget::Table(id) | FocusTarget::Search(id) => !self.sections[id.index()].hidden_by_toggle,
            FocusTarget::DetailsTabs => self.details.has_tabs(),
            FocusTarget::CommandLog => self.command_log.is_visible(),
        };
        allowed && self.move_focus(target)
    }

    /// What Tab and Shift+Tab walk through, in order: the shown sections' tables, each after its search box while
    /// the box is shown, the details tabs and content, and the command log when it's displayed. While something is
    /// maximized, only the targets inside it.
    pub fn focus_ring(&self, maximized: Option<FocusTarget>) -> Vec<FocusTarget> {
        let tables = SectionId::ALL
            .into_iter()
            .filter(|id| self.sections[id.index()].shown())
            .flat_map(|id| {
                let search = self.table(id).search_visible().then_some(FocusTarget::Search(id));
                search.into_iter().chain([FocusTarget::Table(id)])
            });
        let details = self
            .details
            .has_tabs()
            .then_some(FocusTarget::DetailsTabs)
            .into_iter()
            .chain([FocusTarget::DetailsContent]);
        let log = self.command_log.is_visible().then_some(FocusTarget::CommandLog);
        tables
            .chain(details)
            .chain(log)
            .filter(|target| maximized.is_none_or(|maximized| target.within_maximized(maximized)))
            .collect()
    }

    /// Moves focus one step around [`Self::focus_ring`], wrapping. From a target outside the ring (a section
    /// hidden by config, focused with its key), both directions go to the ring's first entry, as in Textual.
    pub fn focus_step(&mut self, forward: bool, maximized: Option<FocusTarget>) {
        // The stops inside the details content come first, and focus leaves the content past either end.
        if self.focus == FocusTarget::DetailsContent && self.details.focus_step(forward) {
            return;
        }
        let ring = self.focus_ring(maximized);
        let Some(&first) = ring.first() else { return };
        let target = match ring.iter().position(|&target| target == self.focus) {
            Some(i) if forward => ring[(i + 1) % ring.len()],
            Some(i) => ring[(i + ring.len() - 1) % ring.len()],
            None => first,
        };
        if self.move_focus(target) && target == FocusTarget::DetailsContent {
            self.details.focus_enter(forward);
        }
    }

    /// The target drawn at a cell in the last frame, if any. Borders, titles, the status box and the footer have
    /// none.
    pub fn hit(&self, column: u16, row: u16) -> Option<FocusTarget> {
        let position = Position::new(column, row);
        self.hits
            .iter()
            .find(|(rect, _)| rect.contains(position))
            .map(|&(_, target)| target)
    }

    /// A mouse event, against the last frame. A left press focuses what it lands on, and the wheel scrolls the widget
    /// under the pointer, focused or not, as in Textual. The tables also take moves (their hover) and the click that
    /// moves their cursor.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        let at = Position::new(mouse.column, mouse.row);
        let over = SectionId::ALL
            .into_iter()
            .find(|&id| self.table_areas[id.index()].is_some_and(|area| area.contains(at)));
        // Moves elsewhere still reach the other tables, so their hover clears when the pointer leaves.
        if mouse.kind == MouseEventKind::Moved {
            for id in SectionId::ALL.into_iter().filter(|&id| Some(id) != over) {
                let area = self.table_areas[id.index()].unwrap_or_default();
                let _ = self.table_mut(id).handle_mouse(mouse, area);
            }
        }
        if let Some(id) = over {
            return self.table_mouse(id, mouse, at);
        }
        // Only a strip drawn in the last frame takes a click.
        if self.hit(mouse.column, mouse.row) == Some(FocusTarget::DetailsTabs)
            && let Some(tab) = self.details.tab_at(mouse)
        {
            return Some(Action::Main(MainAction::ClickDetailsTab(tab)));
        }
        // A widget inside the content that takes its own wheel and clicks (a tree) gets them first.
        if !matches!(mouse.kind, MouseEventKind::Down(_))
            && self.hit(mouse.column, mouse.row) == Some(FocusTarget::DetailsContent)
            && self.details.mouse(mouse)
        {
            return Some(Action::Consumed);
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let target = self.hit(mouse.column, mouse.row);
                if target == Some(FocusTarget::DetailsContent) {
                    self.details.click(at);
                }
                target.map(Action::Focus)
            }
            _ => self.wheel(&mouse),
        }
    }

    /// A mouse event over a section's interior. A left press on the search box or the table focuses it: the
    /// table moves its own focus, and the screen follows it.
    fn table_mouse(&mut self, id: SectionId, mouse: MouseEvent, at: Position) -> Option<Action> {
        let on_table = self.drawn_table_rect(id).is_some_and(|rect| rect.contains(at));
        // Below the table's rows is its container, which neither scrolls nor takes focus.
        if Wheel::from_mouse(&mouse).is_some() && !on_table {
            return None;
        }
        let area = self.table_areas[id.index()].unwrap_or_default();
        let table = self.table_mut(id);
        let overlay = table.overlay_shown();
        let handled = table.handle_mouse(mouse, area);
        let focus = table.focus();
        match mouse.kind {
            // The loading overlay took it.
            _ if overlay => None,
            MouseEventKind::Down(MouseButton::Left) => {
                if focus == TableFocus::Search {
                    Some(Action::Focus(FocusTarget::Search(id)))
                } else if on_table {
                    // A press on the table blurred the box, if it had focus.
                    self.sync_table_focus(id);
                    Some(Action::Focus(FocusTarget::Table(id)))
                } else {
                    None
                }
            }
            // A click on the cursor's cell: the press focused the table, and this release selects.
            _ => match handled {
                Handled::Event(SectionEvent::Selected) if id == SectionId::Repos => self
                    .repos
                    .table
                    .selected()
                    .map(|repo| Action::Main(MainAction::ClickSelect(Box::new(repo.clone()))))
                    // The event is for the cursor's row, so it's always there.
                    .or(Some(Action::Consumed)),
                Handled::Event(SectionEvent::Selected) if id == SectionId::PullRequests => {
                    Some(self.select_pull_request())
                }
                Handled::Event(SectionEvent::Selected) if id == SectionId::Issues => Some(self.select_issue()),
                Handled::Event(SectionEvent::Selected) if id == SectionId::Workflows => {
                    Some(self.select_workflow_run())
                }
                handled => table_outcome(id, handled),
            },
        }
    }

    /// `enter`, `space` or a click on the cursor row of the Pull Requests table: Python's `pr_selected`.
    fn select_pull_request(&self) -> Action {
        let Some(pr) = self.issue_lists.prs_table().selected() else {
            // The event is for the cursor's row, so it's always there.
            return Action::Consumed;
        };
        tracing::info!("Selected PR: #{}", pr.issue.number);
        Action::Main(MainAction::OpenPullRequest {
            target: PullRequestTarget::Partial(Box::new(pr.clone())),
            focus: true,
        })
    }

    /// `enter`, `space` or a click on the cursor row of the Issues table: Python's `issue_selected`.
    fn select_issue(&self) -> Action {
        let Some(issue) = self.issue_lists.issues_table().selected() else {
            // The event is for the cursor's row, so it's always there.
            return Action::Consumed;
        };
        tracing::info!("Selected Issue: #{}", issue.number);
        Action::Main(MainAction::OpenIssue(Box::new(issue.clone())))
    }

    /// `enter`, `space` or a click on the cursor row of the Workflows table: Python's `workflow_run_selected`.
    fn select_workflow_run(&self) -> Action {
        let Some(run) = self.workflows.runs_table().selected() else {
            // The event is for the cursor's row, so it's always there.
            return Action::Consumed;
        };
        tracing::info!("Selected workflow run #{}", run.run_number);
        Action::Main(MainAction::OpenWorkflowRun(Box::new(run.clone())))
    }

    /// What a mouse wheel event means: Textual scrolls the widget under the pointer, focused or not. The repos table
    /// takes its own in [`Self::handle_mouse`]; here it's the command log or the details content.
    pub fn wheel(&self, mouse: &MouseEvent) -> Option<Action> {
        let wheel = Wheel::from_mouse(mouse)?;
        match self.hit(mouse.column, mouse.row)? {
            FocusTarget::CommandLog => Some(Action::Main(MainAction::WheelLog(wheel))),
            FocusTarget::DetailsContent => Some(Action::Main(MainAction::WheelDetails(wheel))),
            _ => None,
        }
    }

    /// The binding chain for the current focus, innermost first, up to and including the screen. Python's widget
    /// tree from the focused widget up.
    pub fn binding_chain(&self) -> Vec<ChainLink> {
        let link = ChainLink::new;
        let mut chain = match self.focus {
            FocusTarget::Table(id) => vec![
                link(Owner::Table(id), TABLE_BINDINGS),
                link(Owner::SearchableTable(id), SEARCHABLE_BINDINGS),
                link(Owner::Section(id), id.bindings()),
                link(Owner::SelectionsPane, SELECTIONS_BINDINGS),
            ],
            // `App` marks every link after the box's own as behind it, which drops the keys the box types.
            FocusTarget::Search(id) => vec![
                link(Owner::SearchInput(id), INPUT_BINDINGS),
                link(Owner::SearchableTable(id), SEARCHABLE_BINDINGS),
                link(Owner::Section(id), id.bindings()),
                link(Owner::SelectionsPane, SELECTIONS_BINDINGS),
            ],
            FocusTarget::DetailsTabs => vec![link(Owner::DetailsTabs, TABS_BINDINGS)],
            FocusTarget::DetailsContent => {
                let mut links = self.details.content_chain();
                links.push(link(Owner::DetailsContent, INFO_BINDINGS));
                links
            }
            FocusTarget::CommandLog => vec![link(Owner::CommandLog, LOG_BINDINGS)],
        };
        chain.push(link(Owner::MainView, MAIN_VIEW_BINDINGS));
        chain.push(link(Owner::Screen(ScreenKind::Main), self.bindings()));
        chain
    }

    /// What a binding matched at one of [`Self::binding_chain`]'s links means right now. `None` lets the key fall
    /// through to the next link.
    pub fn on_link_binding(&mut self, owner: Owner, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match owner {
            Owner::MainView => focus_key_target(id).map(Action::Focus),
            Owner::Screen(ScreenKind::Main) => self.on_binding(id, cx),
            Owner::DetailsContent => {
                Scroll::from_binding(id).map(|scroll| Action::Main(MainAction::ScrollDetails(scroll)))
            }
            Owner::CommandLog => Scroll::from_binding(id).map(|scroll| Action::Main(MainAction::ScrollLog(scroll))),
            // The table is plain state, so it applies the binding now, as Textual runs the binding's action.
            Owner::Table(section) => match self.table_mut(section).on_binding(id) {
                Handled::Event(SectionEvent::Selected) if section == SectionId::Repos => self
                    .repos
                    .table
                    .selected()
                    .map(|repo| Action::SelectRepo(Box::new(repo.clone())))
                    // The event is for the cursor's row, so it's always there.
                    .or(Some(Action::Consumed)),
                Handled::Event(SectionEvent::Selected) if section == SectionId::PullRequests => {
                    Some(self.select_pull_request())
                }
                Handled::Event(SectionEvent::Selected) if section == SectionId::Issues => Some(self.select_issue()),
                Handled::Event(SectionEvent::Selected) if section == SectionId::Workflows => {
                    Some(self.select_workflow_run())
                }
                handled => table_outcome(section, handled),
            },
            Owner::DetailsTabs => {
                let switch = match id {
                    BindingId::TextualTabsNext => TabSwitch::Next,
                    BindingId::TextualTabsPrevious => TabSwitch::Previous,
                    _ => return None,
                };
                Some(Action::Main(MainAction::SwitchDetailsTab(switch)))
            }
            Owner::PrOverview
            | Owner::IssueOverview
            | Owner::DetailsButton
            | Owner::CollapsibleTitle
            | Owner::ListView
            | Owner::CommentBlock
            | Owner::Tree
            | Owner::RunJobs
            | Owner::DiffViewer
            | Owner::HunkPane
            | Owner::DiffFilter => self.details.on_binding(owner, id),
            Owner::SearchableTable(section) if id == BindingId::TableSearch => {
                self.open_search(section);
                Some(Action::Consumed)
            }
            // `handle_focused_key` already gave the box every key it binds.
            Owner::SearchInput(_) => Some(Action::Consumed),
            Owner::Section(SectionId::Repos) if id == BindingId::RepositoriesFavorite => {
                Some(Action::Main(MainAction::ToggleFavorite))
            }
            Owner::Section(SectionId::Repos) if id == BindingId::RepositoriesLookup => {
                Some(Action::Main(MainAction::LookupRepository))
            }
            Owner::Section(SectionId::PullRequests) if id == BindingId::PullRequestLookup => {
                Some(Action::Main(MainAction::LookupPullRequest))
            }
            Owner::Section(SectionId::Issues) if id == BindingId::IssueLookup => {
                Some(Action::Main(MainAction::LookupIssue))
            }
            Owner::Section(SectionId::Repos) if id == BindingId::UserViewProfile => {
                Some(Action::Main(MainAction::ViewProfile(ProfileOrigin::Repos)))
            }
            Owner::Section(SectionId::PullRequests) if id == BindingId::UserViewProfile => {
                Some(Action::Main(MainAction::ViewProfile(ProfileOrigin::PullRequests)))
            }
            Owner::Section(SectionId::Issues) if id == BindingId::UserViewProfile => {
                Some(Action::Main(MainAction::ViewProfile(ProfileOrigin::Issues)))
            }
            // Not built yet: the key is still consumed, as in Python.
            Owner::SearchableTable(_) | Owner::Section(_) | Owner::SelectionsPane => Some(Action::Unimplemented(id)),
            Owner::App | Owner::Screen(ScreenKind::Auth | ScreenKind::Modal(_)) | Owner::DialogField(..) => None,
        }
    }

    /// How the footer and help panel show a binding matched at one of [`Self::binding_chain`]'s links.
    pub fn link_binding_state(&self, owner: Owner, id: BindingId, cx: &Cx<'_>) -> BindingState {
        match owner {
            Owner::Screen(ScreenKind::Main) => self.binding_state(id, cx),
            Owner::DiffViewer => self.details.binding_state(owner, id),
            _ => BindingState::Enabled,
        }
    }

    /// The footer row for a screen drawn in `area`: its last row, full width. There's none while maximized.
    pub fn footer_area(&self, area: Rect) -> Rect {
        self.layout(area).footer
    }

    /// The status box for a screen drawn in `area`: its top three rows, full width. There's none while maximized.
    pub fn status_area(&self, area: Rect) -> Rect {
        self.layout(area).status
    }

    fn layout(&self, area: Rect) -> layout::MainLayout {
        layout(&LayoutInput {
            area,
            sections: self.section_states(),
            focus: Some(self.focus),
            command_log: self.command_log.is_visible(),
        })
    }

    /// Draws only the maximized widget for `maximized`, over the whole `area`, as Textual's `Screen.maximize`: at
    /// full width, and either filling the height or centered on a hatch.
    pub fn render_maximized(&mut self, frame: &mut Frame<'_>, area: Rect, maximized: FocusTarget, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let natural_height = match maximized {
            FocusTarget::Table(id) => Some(self.table(id).bare_table_rect(area).height),
            FocusTarget::Search(_) => Some(text_input::HEIGHT),
            FocusTarget::DetailsTabs | FocusTarget::DetailsContent | FocusTarget::CommandLog => None,
        };
        let rect = match natural_height {
            Some(height) if height < area.height => Rect {
                y: area.y + (area.height - height) / 2,
                height,
                ..area
            },
            _ => area,
        };
        if rect != area {
            frame.render_widget(Hatch::new(theme), area);
        }
        let screen = theme.background();
        frame.render_widget(Clear, rect);
        frame.render_widget(Block::new().style(Style::new().bg(theme.paint(screen))), rect);

        self.hits.clear();
        self.table_areas = [None; 4];
        match maximized {
            FocusTarget::Table(id) | FocusTarget::Search(id) => {
                let within = self.focus.section() == Some(id);
                let table_cx = RenderCx {
                    focused: within,
                    focus_within: within,
                    ..*cx
                };
                let table = self.table_mut(id);
                if matches!(maximized, FocusTarget::Table(_)) {
                    // Only the `DataTable`: the search box is outside it, even while it has focus. No loading
                    // overlay either: Python's belongs to the container around it.
                    table.render_table(frame, rect, &table_cx);
                    self.table_areas[id.index()] = Some(rect);
                } else {
                    // Only the `Input` (#156).
                    table.render_search(frame, rect, &table_cx);
                }
                self.hits.push((rect, maximized));
            }
            // `DetailsContent` is maximized from a button inside it (see `Self::can_maximize`): the tab area shows.
            FocusTarget::DetailsTabs | FocusTarget::DetailsContent => {
                let hits = self.details.render(frame, rect, cx, self.focus);
                self.hits.extend(hits);
            }
            FocusTarget::CommandLog => {
                let focused = self.focus == FocusTarget::CommandLog;
                self.command_log.render(frame, rect, theme, focused);
                self.hits.push((rect, FocusTarget::CommandLog));
            }
        }
    }
}

/// What a section table's answer to an input means for the screen. `None` lets the input fall through.
fn table_outcome(section: SectionId, handled: Handled<SectionEvent>) -> Option<Action> {
    match handled {
        Handled::Ignored => None,
        Handled::Consumed => Some(Action::Consumed),
        Handled::Event(SectionEvent::LoadMore {
            generation,
            page,
            per_page,
        }) => Some(Action::Main(MainAction::LoadMore {
            section,
            generation,
            page,
            per_page,
        })),
        // The repos table's selections are taken before this; the other tables' select issues take theirs.
        Handled::Event(SectionEvent::Selected) => Some(Action::Unimplemented(BindingId::TableSelect)),
    }
}

/// A pull request that couldn't be fetched: Python's `load_pull_request` on `GithubApiRequestFailed`.
fn pull_request_failed(error: &GhError, cx: &mut Cx<'_>) {
    tracing::error!("API error loading PR!");
    tracing::debug!("Fetching the pull request failed: {error}");
    cx.notify(Toast::error("Error fetching pull request!").with_title("API Error"));
}

/// The target a focus key moves to.
fn focus_key_target(id: BindingId) -> Option<FocusTarget> {
    Some(match id {
        BindingId::MainReposFocus => FocusTarget::Table(SectionId::Repos),
        BindingId::MainPullRequestsFocus => FocusTarget::Table(SectionId::PullRequests),
        BindingId::MainIssuesFocus => FocusTarget::Table(SectionId::Issues),
        BindingId::MainWorkflowsFocus => FocusTarget::Table(SectionId::Workflows),
        BindingId::MainDetailsFocus => FocusTarget::DetailsTabs,
        BindingId::MainCommandLogFocus => FocusTarget::CommandLog,
        _ => return None,
    })
}

impl Component for MainScreen {
    type Action = MainAction;

    fn bindings(&self) -> &'static [BindingId] {
        SCREEN_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match id {
            // Py bug B7: inactive while notifications are off, so the key falls through.
            BindingId::NotificationsOpen => cx.config().notifications.enabled.then_some(Action::Unimplemented(id)),
            BindingId::TextualFocusNext => Some(Action::FocusNext),
            BindingId::TextualFocusPrevious => Some(Action::FocusPrevious),
            // Nothing can be selected yet, so Textual's `check_action` lets ctrl+c through to `help_quit`.
            BindingId::TextualCopyText => None,
            other => Some(Action::Unimplemented(other)),
        }
    }

    fn binding_state(&self, id: BindingId, cx: &Cx<'_>) -> BindingState {
        match id {
            BindingId::NotificationsOpen if !cx.config().notifications.enabled => BindingState::Hidden,
            _ => BindingState::Enabled,
        }
    }

    fn update(&mut self, action: MainAction, cx: &mut Cx<'_>) {
        match action {
            MainAction::ScrollDetails(scroll) => self.details.scroll(scroll),
            MainAction::ScrollLog(scroll) => self.command_log.scroll(scroll),
            MainAction::WheelLog(wheel) => self.command_log.wheel(wheel),
            MainAction::WheelDetails(wheel) => self.details.wheel(wheel),
            MainAction::DetailsCommand(owner, id) => self.details.on_command(owner, id, cx),
            MainAction::ClickSelect(repo) => {
                self.try_focus(FocusTarget::Table(SectionId::Repos));
                cx.dispatch(Action::SelectRepo(repo));
            }
            MainAction::ToggleFavorite => self.repos.toggle_favorite(cx),
            MainAction::LookupRepository => cx.open_dialog(LookupDialog::<RepoLookup>::new(), |found| {
                Action::Main(MainAction::RepoLookedUp(found.map(Box::new)))
            }),
            // Python's `action_lookup_repository`: `add_item`, then `RepoSelected` as for enter. Focus and the cursor
            // stay where they are.
            MainAction::RepoLookedUp(Some(repo)) => {
                self.repos.add((*repo).clone(), cx);
                cx.dispatch(Action::SelectRepo(repo));
            }
            // A cancelled lookup or a profile closed with nothing picked.
            MainAction::RepoLookedUp(None) | MainAction::ProfilePicked(_, None) => {}
            MainAction::ViewProfile(ProfileOrigin::Repos) => match self.repos.table.selected() {
                Some(repo) => self.profile.open(repo.owner.login.clone(), ProfileOrigin::Repos, cx),
                None => cx.notify(Toast::error("No repository currently selected")),
            },
            MainAction::ViewProfile(ProfileOrigin::PullRequests) => match self.issue_lists.prs_table().selected() {
                Some(pr) => self
                    .profile
                    .open(pr.issue.user.login.clone(), ProfileOrigin::PullRequests, cx),
                None => cx.notify(Toast::error("No pull request currently selected")),
            },
            MainAction::ViewProfile(ProfileOrigin::Issues) => match self.issue_lists.issues_table().selected() {
                Some(issue) => self.profile.open(issue.user.login.clone(), ProfileOrigin::Issues, cx),
                None => cx.notify(Toast::error("No issue currently selected")),
            },
            MainAction::ProfilePicked(ProfileOrigin::Repos, Some(repo)) => {
                if self.repos.table.get(&repo.full_name).is_none() {
                    self.repos.add((*repo).clone(), cx);
                }
                cx.dispatch(Action::SelectRepo(repo));
            }
            // Python's PR and issue containers post `RepoSelected` without `add_item`.
            MainAction::ProfilePicked(ProfileOrigin::PullRequests | ProfileOrigin::Issues, Some(repo)) => {
                cx.dispatch(Action::SelectRepo(repo));
            }
            // Python opens the dialog and fails an `assert` at submit (Py bug B12).
            MainAction::LookupPullRequest | MainAction::LookupIssue if cx.current_repo().is_none() => {
                cx.notify(select_repository_first());
            }
            MainAction::LookupPullRequest => cx.open_dialog(LookupDialog::<PrLookup>::new(), |found| {
                found.map_or(Action::Consumed, |pr| {
                    Action::Main(MainAction::PullRequestLookedUp(Box::new(pr)))
                })
            }),
            // Python's `action_lookup_pull_request`: added unless its number is in the table, then selected as from the
            // table, but with the pull request already fetched.
            MainAction::PullRequestLookedUp(pr) => {
                self.issue_lists.insert_pr_if_missing(pr.pr.clone(), cx);
                tracing::info!("Looked up PR #{}", pr.number());
                cx.dispatch(Action::Main(MainAction::OpenPullRequest {
                    target: PullRequestTarget::Full(pr),
                    focus: true,
                }));
            }
            MainAction::LookupIssue => cx.open_dialog(LookupDialog::<IssueLookup>::new(), |found| {
                found.map_or(Action::Consumed, |item| {
                    Action::Main(MainAction::IssueLookedUp(Box::new(item)))
                })
            }),
            // Python's `action_lookup_issue`: added unless its number is in the table, then selected as from the
            // table. A pull request is selected as an issue but kept out of the Issues table (Py bug B61).
            MainAction::IssueLookedUp(item) => {
                let issue = match *item {
                    IssueListItem::Issue(issue) => {
                        self.issue_lists.insert_issue_if_missing(issue.clone(), cx);
                        issue
                    }
                    IssueListItem::PullRequest(pr) => pr.issue,
                };
                tracing::info!("Looked up Issue #{}", issue.number);
                cx.dispatch(Action::Main(MainAction::OpenIssue(Box::new(issue))));
            }
            MainAction::OpenPullRequest { target, focus } => self.open_pull_request(target, focus, cx),
            MainAction::OpenIssue(issue) => self.open_issue(*issue, cx),
            MainAction::OpenWorkflowRun(run) => self.open_workflow_run(*run, cx),
            MainAction::SwitchDetailsTab(switch) => {
                self.details.switch_tab(switch, cx);
            }
            MainAction::ClickDetailsTab(tab) => {
                self.try_focus(FocusTarget::DetailsTabs);
                self.details.switch_tab(TabSwitch::To(tab), cx);
            }
            MainAction::LoadMore {
                section,
                generation,
                page,
                per_page,
            } => self.load_more(section, generation, page, per_page, cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let boxes = self.layout(area);
        self.hits.clear();
        self.table_areas = [None; 4];
        let interior = |rect: Rect| rect.inner(Margin::new(1, 1));

        // `App` draws the status box into `Self::status_area`, from the current repo and unread count it holds.
        for (id, rect) in SectionId::ALL.into_iter().zip(boxes.sections) {
            let Some(rect) = rect else { continue };
            let focused = self.focus.section() == Some(id);
            let suffix = self.compact(id).then_some(EMPTY_SUFFIX);
            let title = titled_box(&self.section_titles[id.index()], suffix, rect.width, focused, theme);
            frame.render_widget(title, rect);
            // Only the rows the `DataTable` draws take a click: below them is its unfocusable container.
            let inner = interior(rect);
            // The table draws the box or itself focused, by its own `TableFocus`.
            let table_cx = RenderCx {
                focused,
                focus_within: focused,
                ..*cx
            };
            let table = self.table_mut(id);
            table.render(frame, inner, &table_cx);
            let table_rect = table.table_rect(inner);
            self.table_areas[id.index()] = Some(inner);
            self.hits.push((table_rect, FocusTarget::Table(id)));
        }

        let details = titled_box(
            &self.details_title,
            None,
            boxes.details.width,
            self.focus.in_details(),
            theme,
        );
        let inner = details.inner(boxes.details);
        frame.render_widget(details, boxes.details);
        let hits = self.details.render(frame, inner, cx, self.focus);
        self.hits.extend(hits);

        if let Some(rect) = boxes.command_log {
            let focused = self.focus == FocusTarget::CommandLog;
            frame.render_widget(
                titled_box(&self.command_log_title, None, rect.width, focused, theme),
                rect,
            );
            self.command_log.render(frame, interior(rect), theme, focused);
            self.hits.push((interior(rect), FocusTarget::CommandLog));
        }
        // `App` draws the footer into `Self::footer_area`, from the chain's active bindings.
    }
}

#[cfg(test)]
mod tests;
