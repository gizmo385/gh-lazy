//! Inside the Details box: a tab strip, and the active tab's body under it. Before a repo is selected that's the
//! Introduction tab; afterwards the selected repo's Repo Overview. Python's `SelectionDetailsContainer`, with
//! `LazyGithubInfoTabPane` or `RepoOverviewTabPane`.

use lazy_github_core::models::Repository;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::component::Cx;
use crate::components::info::{InfoView, Scroll};
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::theme::Theme;

use super::FocusTarget;

/// The details pane's content.
#[derive(Debug)]
pub(super) struct DetailsPane {
    tabs: TabStrip,
    body: Body,
}

/// The active tab's body.
#[derive(Debug)]
enum Body {
    /// The Introduction tab, before any selection.
    Intro(InfoView),
    /// The selected repo's Repo Overview tab.
    Repo(RepoOverview),
}

impl Default for DetailsPane {
    fn default() -> Self {
        Self {
            tabs: TabStrip::new(["Introduction"]),
            body: Body::Intro(InfoView::default()),
        }
    }
}

/// The Repo Overview tab of the selected repo. Its body is #174's; until then it draws nothing.
#[derive(Debug)]
pub(super) struct RepoOverview {
    repo: Repository,
}

impl RepoOverview {
    /// The overview of `repo`. Takes the context so it can start its own requests (the README, #219).
    pub(super) fn new(repo: &Repository, _cx: &mut Cx<'_>) -> Self {
        Self { repo: repo.clone() }
    }

    /// The repo shown.
    pub(super) fn repo(&self) -> &Repository {
        &self.repo
    }
}

impl DetailsPane {
    /// Whether the tab strip can take focus: Python's `action_focus_tabs` needs at least one tab.
    pub(super) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// Replaces the tabs with the one active Repo Overview tab of `repo`, scrolled to the top: Python clears the
    /// panes and adds a new one on every selection, the same repo included.
    pub(super) fn show_repo(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        self.tabs = TabStrip::new(["Repo Overview"]);
        self.body = Body::Repo(RepoOverview::new(repo, cx));
    }

    /// The tab strip.
    pub(super) fn tabs(&self) -> &TabStrip {
        &self.tabs
    }

    /// The repo whose overview is showing, if any.
    pub(super) fn repo(&self) -> Option<&Repository> {
        match &self.body {
            Body::Intro(_) => None,
            Body::Repo(overview) => Some(overview.repo()),
        }
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        match &mut self.body {
            Body::Intro(info) => info.scroll(scroll),
            // Nothing to scroll until #174 draws the overview.
            Body::Repo(_) => {}
        }
    }

    /// Draws the tabs and the active tab's content into `area`, and returns where each focus target landed, for
    /// clicks.
    pub(super) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: &Theme,
        focus: FocusTarget,
    ) -> [(Rect, FocusTarget); 2] {
        let tabs_height = area.height.min(TAB_STRIP_HEIGHT);
        let tabs = Rect {
            height: tabs_height,
            ..area
        };
        self.tabs.render(frame, tabs, theme, focus == FocusTarget::DetailsTabs);
        let body = Rect {
            y: area.y + tabs_height,
            height: area.height - tabs_height,
            ..area
        };
        match &mut self.body {
            Body::Intro(info) => info.render(frame, body, theme),
            Body::Repo(_) => {}
        }
        [(tabs, FocusTarget::DetailsTabs), (body, FocusTarget::DetailsContent)]
    }
}
