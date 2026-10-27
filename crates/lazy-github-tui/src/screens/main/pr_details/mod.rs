//! A pull request's details: the Overview, Diff and Conversation tabs. Python's `PrOverviewTabPane`, `PrDiffTabPane`
//! and `PrConversationTabPane`, which `MainViewPane.load_pull_request` puts in the details pane.

mod diff;
mod overview;

use std::sync::Arc;

use lazy_github_core::models::{FullPullRequest, PartialPullRequest};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::effect::{GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Theme;

use super::details::{BodyRenderCx, DetailsBody};
pub(super) use diff::PrDiff;
pub(super) use overview::PrOverview;
#[cfg(test)]
pub(super) use overview::{OverviewStop, SectionKind};

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

/// One pull request's tabs. The Conversation tab (#233, #234) is empty until it lands, and adds its body here.
#[derive(Debug)]
pub(in crate::screens::main) struct PullRequestDetails {
    pr: Arc<FullPullRequest>,
    overview: PrOverview,
    diff: PrDiff,
}

impl PullRequestDetails {
    /// The tabs of `pr`. Each tab starts its requests here, as Textual mounts every pane at once.
    pub(super) fn new(pr: FullPullRequest, cx: &mut Cx<'_>) -> Self {
        let pr = Arc::new(pr);
        Self {
            overview: PrOverview::new(Arc::clone(&pr), cx),
            diff: PrDiff::new(&pr, cx),
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

    /// The Diff tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn diff(&self) -> &PrDiff {
        &self.diff
    }
}

impl DetailsBody for PullRequestDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        PrTab::NAMES
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        let response = self.overview.on_response(id, response)?;
        self.diff.on_response(id, response)
    }

    fn close(&self, cx: &mut Cx<'_>) {
        self.overview.close(cx);
        self.diff.close(cx);
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.scroll(scroll),
            PrTab::Diff => self.diff.scroll(scroll),
            PrTab::Conversation => {}
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.wheel(wheel),
            PrTab::Diff => self.diff.wheel(wheel),
            PrTab::Conversation => {}
        }
    }

    /// The pane draws through [`Self::render_timed`]; this fallback draws any loading overlay as static text.
    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        let cx = BodyRenderCx {
            theme,
            now: tokio::time::Instant::now(),
            animations: false,
            focused,
        };
        self.render_timed(tab, frame, area, &cx);
    }

    fn render_timed(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.render(frame, area, cx),
            PrTab::Diff => self.diff.render(frame, area, cx),
            PrTab::Conversation => {}
        }
    }

    fn next_deadline(&self, tab: usize, now: tokio::time::Instant, animations: bool) -> Option<tokio::time::Instant> {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.next_deadline(now, animations),
            PrTab::Diff => self.diff.next_deadline(now, animations),
            PrTab::Conversation => None,
        }
    }

    fn focus_step(&mut self, tab: usize, forward: bool) -> bool {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.focus_step(forward),
            PrTab::Diff => self.diff.focus_step(forward),
            PrTab::Conversation => false,
        }
    }

    fn focus_first(&mut self, tab: usize) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.focus_first(),
            PrTab::Diff => self.diff.focus_first(),
            PrTab::Conversation => {}
        }
    }

    fn focus_last(&mut self, tab: usize) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.focus_last(),
            PrTab::Diff => self.diff.focus_last(),
            PrTab::Conversation => {}
        }
    }

    fn click(&mut self, tab: usize, position: Position) {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.click(position),
            PrTab::Diff => self.diff.click(position),
            PrTab::Conversation => {}
        }
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match PrTab::at(tab) {
            PrTab::Overview => self.overview.chain(),
            PrTab::Diff => self.diff.chain(),
            PrTab::Conversation => Vec::new(),
        }
    }

    /// A section title's or list's key applies to the Overview; merge (#183), edit (#185) and add a reaction aren't
    /// built yet: the key is consumed, as in Python.
    fn on_binding(&mut self, tab: usize, owner: Owner, id: BindingId) -> Option<Action> {
        // The diff viewer's titles, pane, filter and keys.
        if PrTab::at(tab) == PrTab::Diff {
            return self.diff.on_binding(owner, id);
        }
        match owner {
            Owner::CollapsibleTitle | Owner::ListView => {
                (PrTab::at(tab) == PrTab::Overview && self.overview.on_binding(id)).then_some(Action::Consumed)
            }
            _ => Some(Action::Unimplemented(id)),
        }
    }

    fn binding_state(&self, tab: usize, owner: Owner, id: BindingId) -> BindingState {
        match PrTab::at(tab) {
            PrTab::Diff => self.diff.binding_state(owner, id),
            PrTab::Overview | PrTab::Conversation => BindingState::Enabled,
        }
    }

    fn handle_focused_key(&mut self, tab: usize, key: &KeyChord, keymap: &Keymap) -> Option<Action> {
        match PrTab::at(tab) {
            PrTab::Diff => self.diff.handle_key(key, keymap),
            PrTab::Overview | PrTab::Conversation => None,
        }
    }

    fn handle_paste(&mut self, tab: usize, text: &str) -> bool {
        PrTab::at(tab) == PrTab::Diff && self.diff.handle_paste(text)
    }
}

#[cfg(test)]
mod tests;
