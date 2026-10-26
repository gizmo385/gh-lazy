//! Inside the Details box: a tab strip, and the active tab's body under it. Before a repo is selected that's the
//! Introduction tab; afterwards the selected repo's Repo Overview. Python's `SelectionDetailsContainer`, with
//! `LazyGithubInfoTabPane` or `RepoOverviewTabPane`.

use lazy_github_core::models::Repository;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::component::Cx;
use crate::components::command_log::Wheel;
use crate::components::info::{InfoView, Scroll};
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::effect::{GithubResponse, RequestId};
use crate::theme::Theme;

use super::FocusTarget;
use super::repo_overview::RepoOverview;

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
    Repo(Box<RepoOverview>),
}

impl Default for DetailsPane {
    fn default() -> Self {
        Self {
            tabs: TabStrip::new(["Introduction"]),
            body: Body::Intro(InfoView::default()),
        }
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
        if let Body::Repo(old) = &self.body {
            old.close(cx);
        }
        self.tabs = TabStrip::new(["Repo Overview"]);
        self.body = Body::Repo(Box::new(RepoOverview::new(repo, cx)));
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

    /// Takes the overview's README response, and hands back any other. A README response for an overview that's
    /// gone is dropped.
    pub(super) fn on_response(&mut self, id: RequestId, response: GithubResponse) -> Option<GithubResponse> {
        let GithubResponse::RepoReadme(result) = response else {
            return Some(response);
        };
        match &mut self.body {
            Body::Repo(overview) if overview.pending() == Some(id) => overview.on_readme(result),
            _ => tracing::debug!("Dropping the README response {id} of a repo no longer shown"),
        }
        None
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        match &mut self.body {
            Body::Intro(info) => info.scroll(scroll),
            Body::Repo(overview) => overview.scroll(scroll),
        }
    }

    /// A wheel notch over the active tab's body.
    pub(super) fn wheel(&mut self, wheel: Wheel) {
        match &mut self.body {
            Body::Intro(info) => info.wheel(wheel),
            Body::Repo(overview) => overview.wheel(wheel),
        }
    }

    /// The overview showing, if any.
    #[cfg(test)]
    pub(super) fn overview(&self) -> Option<&RepoOverview> {
        match &self.body {
            Body::Intro(_) => None,
            Body::Repo(overview) => Some(overview),
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
            Body::Repo(overview) => overview.render(frame, body, theme),
        }
        [(tabs, FocusTarget::DetailsTabs), (body, FocusTarget::DetailsContent)]
    }
}
