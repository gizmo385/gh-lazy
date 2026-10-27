//! A pull request's details: the Overview, Diff and Conversation tabs. Python's `PrOverviewTabPane`, `PrDiffTabPane`
//! and `PrConversationTabPane`, which `MainViewPane.load_pull_request` puts in the details pane.

mod overview;

use std::sync::Arc;

use lazy_github_core::models::{FullPullRequest, PartialPullRequest};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::details::DetailsBody;
pub(super) use overview::PrOverview;

/// `PrOverviewTabPane.BINDINGS`: merge (#183), edit (#185) and add a reaction, in the footer while the Overview's
/// content has focus.
pub const PR_OVERVIEW_BINDINGS: &[BindingId] = &[
    BindingId::PullRequestMerge,
    BindingId::PullRequestEdit,
    BindingId::ReactAdd,
];

/// A pull request to show in the details pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestTarget {
    /// A Pull Requests table row: the full pull request is fetched, and shown once it arrives.
    Partial(Box<PartialPullRequest>),
    /// One already fetched (#238's lookup, M4's links and notifications): shown at once.
    Full(Box<FullPullRequest>),
}

impl PullRequestTarget {
    /// Its number, as in `#5`.
    pub fn number(&self) -> u64 {
        match self {
            PullRequestTarget::Partial(pr) => pr.issue.number,
            PullRequestTarget::Full(pr) => pr.number(),
        }
    }
}

/// The three tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrTab {
    Overview,
    Diff,
    Conversation,
}

impl PrTab {
    const NAMES: &'static [&'static str] = &["Overview", "Diff", "Conversation"];

    fn at(index: usize) -> PrTab {
        match index {
            0 => PrTab::Overview,
            1 => PrTab::Diff,
            _ => PrTab::Conversation,
        }
    }
}

/// One pull request's tabs. The Diff tab (#235, #236) and the Conversation tab (#233, #234) are empty until they
/// land, and add their bodies here.
#[derive(Debug)]
pub(in crate::screens::main) struct PullRequestDetails {
    pr: Arc<FullPullRequest>,
    overview: PrOverview,
}

impl PullRequestDetails {
    /// The tabs of `pr`. Each tab starts its requests here, as Textual mounts every pane at once.
    pub(super) fn new(pr: FullPullRequest, _cx: &mut Cx<'_>) -> Self {
        let pr = Arc::new(pr);
        Self {
            overview: PrOverview::new(Arc::clone(&pr)),
            pr,
        }
    }

    /// The pull request shown.
    pub(in crate::screens::main) fn pr(&self) -> &FullPullRequest {
        &self.pr
    }

    /// The Overview tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn overview(&self) -> &PrOverview {
        &self.overview
    }
}

impl DetailsBody for PullRequestDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        PrTab::NAMES
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.scroll(scroll),
            PrTab::Diff | PrTab::Conversation => {}
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.wheel(wheel),
            PrTab::Diff | PrTab::Conversation => {}
        }
    }

    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, _focused: bool) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.render(frame, area, theme),
            PrTab::Diff | PrTab::Conversation => {}
        }
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match PrTab::at(tab) {
            PrTab::Overview => vec![ChainLink::new(Owner::PrOverview, PR_OVERVIEW_BINDINGS)],
            PrTab::Diff | PrTab::Conversation => Vec::new(),
        }
    }

    /// Merge (#183), edit (#185) and add a reaction aren't built yet: the key is consumed, as in Python.
    fn on_binding(&mut self, _tab: usize, _owner: Owner, id: BindingId) -> Option<Action> {
        Some(Action::Unimplemented(id))
    }
}

#[cfg(test)]
mod tests;
