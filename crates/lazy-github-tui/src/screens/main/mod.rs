//! The main screen: the status box on top, the four selection sections on the left, details (and the optional
//! command log) on the right, and the footer row. Python's `LazyGithubMainScreen`.

mod details;
pub mod layout;
mod sections;
mod status;

use lazy_github_core::config::AppearanceSettings;
use ratatui::Frame;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear};

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, Owner, RenderCx, ScreenKind};
use crate::components::hatch::Hatch;
use crate::components::info::{INFO_BINDINGS, Scroll};
use crate::keymap::{BindingId, Keymap};

use details::DetailsPane;
use layout::{LayoutInput, layout};
use sections::titled_box;
use status::StatusBar;

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

    fn in_details(self) -> bool {
        matches!(self, FocusTarget::DetailsTabs | FocusTarget::DetailsContent)
    }

    /// Whether `self` is inside the widget that maximizing `maximized` shows: the same section's table, the details
    /// tab area, or the command log.
    fn within_maximized(self, maximized: FocusTarget) -> bool {
        match (self.section(), maximized.section()) {
            (Some(a), Some(b)) => a == b,
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
const MAIN_VIEW_BINDINGS: &[BindingId] = &[
    BindingId::MainReposFocus,
    BindingId::MainPullRequestsFocus,
    BindingId::MainIssuesFocus,
    BindingId::MainWorkflowsFocus,
    BindingId::MainDetailsFocus,
    BindingId::MainCommandLogFocus,
];

/// `SelectionsPane.BINDINGS`.
const SELECTIONS_BINDINGS: &[BindingId] = &[BindingId::IssueNew, BindingId::PullRequestNew];

/// `LazyGithubMainScreen.BINDINGS`, then Textual's `Screen` built-ins.
const SCREEN_BINDINGS: &[BindingId] = &[
    BindingId::NotificationsOpen,
    BindingId::LinkPaste,
    BindingId::LinkFollow,
    BindingId::LinkFollowExternal,
    BindingId::TextualFocusNext,
    BindingId::TextualFocusPrevious,
    BindingId::TextualCopyText,
];

/// Textual's `Tabs` bindings.
const TABS_BINDINGS: &[BindingId] = &[BindingId::TextualTabsPrevious, BindingId::TextualTabsNext];

/// Textual's `ScrollableContainer` bindings, which the command log's `Log` has. #80 makes them scroll.
const LOG_BINDINGS: &[BindingId] = &[
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

/// The rows a section's table draws: an empty `DataTable` is its header row. Clicks hit only these rows, and a
/// maximized table is this tall. #78 replaces it with the table's real natural height, in both places.
const TABLE_NATURAL_HEIGHT: u16 = 1;

/// The main screen.
#[derive(Debug)]
pub struct MainScreen {
    sections: [SectionState; 4],
    command_log_displayed: bool,
    focus: FocusTarget,
    // Built once, as Python does: a Settings rebind doesn't refresh them.
    section_titles: [String; 4],
    details_title: String,
    command_log_title: String,
    status: StatusBar,
    details: DetailsPane,
    /// Where each focus target was drawn last, for clicks: the interiors of the boxes, not their borders.
    hits: Vec<(Rect, FocusTarget)>,
}

/// The main screen's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainAction {
    /// Scroll the details content (`textual.scroll.*` while it has focus).
    ScrollDetails(Scroll),
}

impl MainScreen {
    /// The main screen for the `appearance` settings read at startup, with titles from `keymap`. The repos table has
    /// focus, as Textual focuses the first focusable widget.
    pub fn new(appearance: &AppearanceSettings, keymap: &Keymap) -> Self {
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
            command_log_displayed: appearance.show_command_log,
            focus: FocusTarget::Table(SectionId::Repos),
            section_titles: SectionId::ALL.map(|id| title(id.focus_binding(), id.label())),
            details_title: title(BindingId::MainDetailsFocus, "Details"),
            command_log_title: title(BindingId::MainCommandLogFocus, "Command Log"),
            status: StatusBar::default(),
            details: DetailsPane::default(),
            hits: Vec::new(),
        }
    }

    /// What has focus.
    pub fn focus(&self) -> FocusTarget {
        self.focus
    }

    /// Moves focus, without the rules of [`Self::try_focus`].
    pub fn set_focus(&mut self, focus: FocusTarget) {
        self.focus = focus;
    }

    /// The state of one section.
    pub fn section(&self, id: SectionId) -> SectionState {
        self.sections[id.index()]
    }

    /// Hides or shows a section by a runtime toggle. M4's command palette drives this.
    pub fn set_hidden_by_toggle(&mut self, id: SectionId, hidden: bool) {
        self.sections[id.index()].hidden_by_toggle = hidden;
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
            FocusTarget::CommandLog => self.command_log_displayed,
        };
        if allowed {
            self.focus = target;
        }
        allowed
    }

    /// What Tab and Shift+Tab walk through, in order: the shown sections' tables, the details tabs and content, and
    /// the command log when it's displayed. While something is maximized, only the targets inside it.
    pub fn focus_ring(&self, maximized: Option<FocusTarget>) -> Vec<FocusTarget> {
        let tables = SectionId::ALL
            .into_iter()
            .filter(|id| self.sections[id.index()].shown())
            .map(FocusTarget::Table);
        let details = self
            .details
            .has_tabs()
            .then_some(FocusTarget::DetailsTabs)
            .into_iter()
            .chain([FocusTarget::DetailsContent]);
        let log = self.command_log_displayed.then_some(FocusTarget::CommandLog);
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
        self.focus = match ring.iter().position(|&target| target == self.focus) {
            Some(i) if forward => ring[(i + 1) % ring.len()],
            Some(i) => ring[(i + ring.len() - 1) % ring.len()],
            None => first,
        };
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

    /// The binding chain for the current focus, innermost first, up to and including the screen. Python's widget
    /// tree from the focused widget up.
    pub fn binding_chain(&self) -> Vec<ChainLink> {
        let link = |owner: Owner, bindings: &'static [BindingId]| ChainLink { owner, bindings };
        let mut chain = match self.focus {
            // #78 adds the table's own bindings and the search input's link.
            FocusTarget::Table(id) | FocusTarget::Search(id) => vec![
                link(Owner::Table(id), &[]),
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
            // Not built yet: the key is still consumed, as in Python. With one tab, switching tabs is a no-op.
            Owner::Table(_) | Owner::Section(_) | Owner::SelectionsPane | Owner::DetailsTabs | Owner::CommandLog => {
                Some(Action::Unimplemented(id))
            }
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

    /// Draws only the maximized widget for `maximized`, over the whole `area`, as Textual's `Screen.maximize`: at
    /// full width, and either filling the height or centered on a hatch.
    pub fn render_maximized(&mut self, frame: &mut Frame<'_>, area: Rect, maximized: FocusTarget, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let natural_height = maximized.section().map(|_| TABLE_NATURAL_HEIGHT);
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
        match maximized {
            FocusTarget::Table(id) | FocusTarget::Search(id) => self.hits.push((rect, FocusTarget::Table(id))),
            FocusTarget::DetailsTabs | FocusTarget::DetailsContent => {
                let hits = self.details.render(frame, rect, theme, self.focus);
                self.hits.extend(hits);
            }
            // #80 draws the log's lines.
            FocusTarget::CommandLog => self.hits.push((rect, FocusTarget::CommandLog)),
        }
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

    fn update(&mut self, action: MainAction, _cx: &mut Cx<'_>) {
        match action {
            MainAction::ScrollDetails(scroll) => self.details.scroll(scroll),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let boxes = layout(&LayoutInput {
            area,
            sections: self.sections,
            focus: Some(self.focus),
            command_log: self.command_log_displayed,
        });
        self.hits.clear();
        let interior = |rect: Rect| rect.inner(Margin::new(1, 1));

        self.status.render(frame, boxes.status, theme);
        for ((id, rect), title) in SectionId::ALL.into_iter().zip(boxes.sections).zip(&self.section_titles) {
            if let Some(rect) = rect {
                let focused = self.focus.section() == Some(id);
                frame.render_widget(titled_box(title, focused, theme), rect);
                // Only the rows the `DataTable` draws take a click: below them is its unfocusable container.
                let inner = interior(rect);
                let table = Rect {
                    height: inner.height.min(TABLE_NATURAL_HEIGHT),
                    ..inner
                };
                self.hits.push((table, FocusTarget::Table(id)));
            }
        }

        let details = titled_box(&self.details_title, self.focus.in_details(), theme);
        let inner = details.inner(boxes.details);
        frame.render_widget(details, boxes.details);
        let hits = self.details.render(frame, inner, theme, self.focus);
        self.hits.extend(hits);

        if let Some(rect) = boxes.command_log {
            let focused = self.focus == FocusTarget::CommandLog;
            frame.render_widget(titled_box(&self.command_log_title, focused, theme), rect);
            self.hits.push((interior(rect), FocusTarget::CommandLog));
        }

        // Footer content arrives with #76.
        let footer_bg = theme.color(theme.palette.footer_background, theme.background());
        frame.render_widget(Block::new().style(Style::new().bg(footer_bg)), boxes.footer);
    }
}

#[cfg(test)]
mod tests;
