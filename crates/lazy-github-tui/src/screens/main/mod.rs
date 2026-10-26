//! The main screen: the status box on top, the four selection sections on the left, details (and the optional
//! command log) on the right, and the footer row. Python's `LazyGithubMainScreen`.

mod details;
pub mod layout;
mod repos;
mod sections;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::AppearanceSettings;
use lazy_github_core::models::Repository;
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
use crate::components::table::{Handled, SEARCHABLE_BINDINGS, SearchableTable, TABLE_BINDINGS, TableEvent, TableFocus};
use crate::components::text_input::{self, INPUT_BINDINGS};
use crate::effect::{GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::options::AppOptions;
use crate::theme::Theme;

use details::DetailsPane;
use layout::{LayoutInput, layout};
pub use repos::{RepoCells, ReposSection};
use sections::titled_box;

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

/// The rows an empty `DataTable` draws: its header row. The sections without a table yet take clicks on this row
/// only, and maximized they're this tall. #171, #172 and #173 replace it with their tables' real height.
const EMPTY_TABLE_HEIGHT: u16 = 1;

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
    /// Where each focus target was drawn last, for clicks: the interiors of the boxes, not their borders.
    hits: Vec<(Rect, FocusTarget)>,
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
    /// A click on the repos table's cursor cell: focus the table, then select the repo, in that order.
    ClickSelect(Box<Repository>),
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
            details: DetailsPane::default(),
            command_log: CommandLog::new(appearance.show_command_log),
            repos: ReposSection::new(theme, options),
            hits: Vec::new(),
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
        if self.section(SectionId::PullRequests).shown() || self.section(SectionId::Issues).shown() {
            // #171: one shared first-page request feeding both tables; #172 adds the issues side.
        }
        if self.section(SectionId::Workflows).shown() {
            // #173: workflows and runs.
        }
        // #174 fills the Repo Overview body.
        self.details.show_repo(repo, cx);
        self.details_title = format!(
            "[{}] {} Details",
            cx.keymap().title_key(BindingId::MainDetailsFocus),
            repo.full_name
        );
        // Python removes the focused content along with the old tab, and Textual moves focus to the tab strip.
        if self.focus == FocusTarget::DetailsContent {
            self.move_focus(FocusTarget::DetailsTabs);
        }
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

    /// Takes a response to one of the screen's requests, and hands back any other.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        self.repos.on_response(id, response, cx)
    }

    /// The repos section.
    pub fn repos(&self) -> &ReposSection {
        &self.repos
    }

    /// The next animation frame on the screen, if anything animates.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.repos.table.next_deadline(now)
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
            && let Some(table) = self.table_mut(id)
            && !table.focus_search()
        {
            return false;
        }
        if let FocusTarget::Search(id) = self.focus
            && let Some(table) = self.table_mut(id)
        {
            table.focus_table();
        }
        self.focus = target;
        true
    }

    /// The section's table, once it's built. #171, #172 and #173 add theirs.
    fn table_mut(&mut self, id: SectionId) -> Option<&mut SearchableTable<Repository>> {
        match id {
            SectionId::Repos => Some(&mut self.repos.table),
            SectionId::PullRequests | SectionId::Issues | SectionId::Workflows => None,
        }
    }

    fn table(&self, id: SectionId) -> Option<&SearchableTable<Repository>> {
        match id {
            SectionId::Repos => Some(&self.repos.table),
            SectionId::PullRequests | SectionId::Issues | SectionId::Workflows => None,
        }
    }

    /// `common.table.search`: shows the section's search box and focuses it, keeping its text.
    fn open_search(&mut self, id: SectionId) {
        if let Some(table) = self.table_mut(id) {
            table.open_search();
        }
        self.move_focus(FocusTarget::Search(id));
    }

    /// Follows a change the table made to its own focus (a submit, a click), through [`Self::move_focus`].
    fn sync_table_focus(&mut self, id: SectionId) {
        let Some(table) = self.table(id) else { return };
        let target = match table.focus() {
            TableFocus::Search => FocusTarget::Search(id),
            TableFocus::Table if self.focus == FocusTarget::Search(id) => FocusTarget::Table(id),
            TableFocus::Table => return,
        };
        self.move_focus(target);
    }

    /// Step 4 of key dispatch: a key the focused search box takes before the binding chain. `None` passes it on.
    pub fn handle_focused_key(&mut self, key: &KeyChord, keymap: &Keymap) -> Option<Action> {
        let FocusTarget::Search(id) = self.focus else {
            return None;
        };
        let table = self.table_mut(id)?;
        if !table.consumes_key(key, keymap) {
            return None;
        }
        let handled = table.handle_key(key, keymap);
        // A submit moved the table's focus to the table.
        self.sync_table_focus(id);
        Some(table_outcome(&handled).unwrap_or(Action::Consumed))
    }

    /// A bracketed paste, which only the focused search box takes.
    pub fn handle_paste(&mut self, text: &str) -> Option<Action> {
        let FocusTarget::Search(id) = self.focus else {
            return None;
        };
        let handled = self.table_mut(id)?.handle_paste(text);
        handled.is_handled().then_some(Action::Consumed)
    }

    /// The state of one section.
    pub fn section(&self, id: SectionId) -> SectionState {
        self.sections[id.index()]
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
                let search = self
                    .table(id)
                    .is_some_and(SearchableTable::search_visible)
                    .then_some(FocusTarget::Search(id));
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
        let ring = self.focus_ring(maximized);
        let Some(&first) = ring.first() else { return };
        let target = match ring.iter().position(|&target| target == self.focus) {
            Some(i) if forward => ring[(i + 1) % ring.len()],
            Some(i) => ring[(i + ring.len() - 1) % ring.len()],
            None => first,
        };
        self.move_focus(target);
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
    /// under the pointer, focused or not, as in Textual. The repos table also takes moves (its hover) and the click
    /// that moves its cursor.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        let at = Position::new(mouse.column, mouse.row);
        if self.repos.area.is_some_and(|area| area.contains(at)) {
            return self.repos_mouse(mouse, at);
        }
        // Moves elsewhere still reach the table, so its hover clears when the pointer leaves.
        if mouse.kind == MouseEventKind::Moved {
            let _ = self
                .repos
                .table
                .handle_mouse(mouse, self.repos.area.unwrap_or_default());
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.hit(mouse.column, mouse.row).map(Action::Focus),
            _ => self.wheel(&mouse),
        }
    }

    /// A mouse event over the repos section's interior. A left press on the search box or the table focuses it: the
    /// table moves its own focus, and the screen follows it.
    fn repos_mouse(&mut self, mouse: MouseEvent, at: Position) -> Option<Action> {
        let on_table = self.repos.table_rect().is_some_and(|rect| rect.contains(at));
        // Below the table's rows is its container, which neither scrolls nor takes focus.
        if Wheel::from_mouse(&mouse).is_some() && !on_table {
            return None;
        }
        let area = self.repos.area.unwrap_or_default();
        let overlay = self.repos.table.overlay_shown();
        let handled = self.repos.table.handle_mouse(mouse, area);
        match mouse.kind {
            // The loading overlay took it.
            _ if overlay => None,
            MouseEventKind::Down(MouseButton::Left) => {
                let id = SectionId::Repos;
                if self.repos.table.focus() == TableFocus::Search {
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
                Handled::Event(TableEvent::Selected(key)) => self
                    .repos
                    .table
                    .get(&key)
                    .map(|repo| Action::Main(MainAction::ClickSelect(Box::new(repo.clone()))))
                    // The key is the cursor row's, so it's always there.
                    .or(Some(Action::Consumed)),
                handled => table_outcome(&handled),
            },
        }
    }

    /// What a mouse wheel event means: Textual scrolls the widget under the pointer, focused or not. The repos table
    /// takes its own in [`Self::handle_mouse`]; here it's the command log.
    pub fn wheel(&self, mouse: &MouseEvent) -> Option<Action> {
        let wheel = Wheel::from_mouse(mouse)?;
        (self.hit(mouse.column, mouse.row) == Some(FocusTarget::CommandLog))
            .then_some(Action::Main(MainAction::WheelLog(wheel)))
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
            FocusTarget::DetailsContent => vec![link(Owner::DetailsContent, INFO_BINDINGS)],
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
            Owner::Table(SectionId::Repos) => match self.repos.table.on_binding(id) {
                Handled::Event(TableEvent::Selected(key)) => self
                    .repos
                    .table
                    .get(&key)
                    .map(|repo| Action::SelectRepo(Box::new(repo.clone())))
                    // The key is the cursor row's, so it's always there.
                    .or(Some(Action::Consumed)),
                handled => table_outcome(&handled),
            },
            Owner::SearchableTable(SectionId::Repos) if id == BindingId::TableSearch => {
                self.open_search(SectionId::Repos);
                Some(Action::Consumed)
            }
            // `handle_focused_key` already gave the box every key it binds.
            Owner::SearchInput(_) => Some(Action::Consumed),
            // Not built yet: the key is still consumed, as in Python. With one tab, switching tabs is a no-op.
            Owner::Table(_)
            | Owner::SearchableTable(_)
            | Owner::Section(_)
            | Owner::SelectionsPane
            | Owner::DetailsTabs => Some(Action::Unimplemented(id)),
            Owner::App | Owner::Screen(ScreenKind::Auth) => None,
        }
    }

    /// How the footer and help panel show a binding matched at one of [`Self::binding_chain`]'s links.
    pub fn link_binding_state(&self, owner: Owner, id: BindingId, cx: &Cx<'_>) -> BindingState {
        match owner {
            Owner::Screen(ScreenKind::Main) => self.binding_state(id, cx),
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
            sections: self.sections,
            focus: Some(self.focus),
            command_log: self.command_log.is_visible(),
        })
    }

    /// Draws only the maximized widget for `maximized`, over the whole `area`, as Textual's `Screen.maximize`: at
    /// full width, and either filling the height or centered on a hatch.
    pub fn render_maximized(&mut self, frame: &mut Frame<'_>, area: Rect, maximized: FocusTarget, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let natural_height = match maximized {
            FocusTarget::Table(SectionId::Repos) => Some(self.repos.table.bare_table_rect(area).height),
            FocusTarget::Table(_) => Some(EMPTY_TABLE_HEIGHT),
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
        self.repos.area = None;
        let within = self.focus.section() == Some(SectionId::Repos);
        let repos_cx = RenderCx {
            focused: within,
            focus_within: within,
            ..*cx
        };
        match maximized {
            // Only the `DataTable`: the search box is outside it, even while it has focus.
            FocusTarget::Table(SectionId::Repos) => {
                self.repos.table.render_table(frame, rect, &repos_cx);
                self.repos.area = Some(rect);
                self.hits.push((rect, maximized));
            }
            // Only the `Input` (#156).
            FocusTarget::Search(SectionId::Repos) => {
                self.repos.table.render_search(frame, rect, &repos_cx);
                self.hits.push((rect, maximized));
            }
            FocusTarget::Table(_) | FocusTarget::Search(_) => self.hits.push((rect, maximized)),
            // `DetailsContent` is never maximized (see `FocusTarget::maximizable`); it's here to keep the match total.
            FocusTarget::DetailsTabs | FocusTarget::DetailsContent => {
                let hits = self.details.render(frame, rect, theme, self.focus);
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

/// What a table's answer to an input means for the screen. `None` lets the input fall through.
fn table_outcome<K>(handled: &Handled<TableEvent<K>>) -> Option<Action> {
    match handled {
        Handled::Ignored => None,
        // Only lazy tables ask for pages, and none is mounted yet (#171).
        Handled::Consumed | Handled::Event(TableEvent::LoadMore { .. }) => Some(Action::Consumed),
        // The repos table's selections are taken before this; the other tables' select issues take theirs.
        Handled::Event(TableEvent::Selected(_)) => Some(Action::Unimplemented(BindingId::TableSelect)),
    }
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
            MainAction::ClickSelect(repo) => {
                self.try_focus(FocusTarget::Table(SectionId::Repos));
                cx.dispatch(Action::SelectRepo(repo));
            }
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let boxes = self.layout(area);
        self.hits.clear();
        self.repos.area = None;
        let interior = |rect: Rect| rect.inner(Margin::new(1, 1));

        // `App` draws the status box into `Self::status_area`, from the current repo and unread count it holds.
        for ((id, rect), title) in SectionId::ALL.into_iter().zip(boxes.sections).zip(&self.section_titles) {
            if let Some(rect) = rect {
                let focused = self.focus.section() == Some(id);
                frame.render_widget(titled_box(title, rect.width, focused, theme), rect);
                // Only the rows the `DataTable` draws take a click: below them is its unfocusable container.
                let inner = interior(rect);
                let table = if id == SectionId::Repos {
                    // The table draws the box or itself focused, by its own `TableFocus`.
                    let cx = RenderCx {
                        focused,
                        focus_within: focused,
                        ..*cx
                    };
                    self.repos.table.render(frame, inner, &cx);
                    self.repos.area = Some(inner);
                    self.repos.table.table_rect(inner)
                } else {
                    Rect {
                        height: inner.height.min(EMPTY_TABLE_HEIGHT),
                        ..inner
                    }
                };
                self.hits.push((table, FocusTarget::Table(id)));
            }
        }

        let details = titled_box(&self.details_title, boxes.details.width, self.focus.in_details(), theme);
        let inner = details.inner(boxes.details);
        frame.render_widget(details, boxes.details);
        let hits = self.details.render(frame, inner, theme, self.focus);
        self.hits.extend(hits);

        if let Some(rect) = boxes.command_log {
            let focused = self.focus == FocusTarget::CommandLog;
            frame.render_widget(titled_box(&self.command_log_title, rect.width, focused, theme), rect);
            self.command_log.render(frame, interior(rect), theme, focused);
            self.hits.push((interior(rect), FocusTarget::CommandLog));
        }
        // `App` draws the footer into `Self::footer_area`, from the chain's active bindings.
    }
}

#[cfg(test)]
mod tests;
