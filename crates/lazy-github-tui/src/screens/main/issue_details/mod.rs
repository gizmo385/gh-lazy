//! An issue's details: the Overview and Comments tabs. Python's `IssueOverviewTabPane` and
//! `IssueConversationTabPane`, which `MainViewPane.load_issue` puts in the details pane.

mod comments;
mod overview;

use std::sync::Arc;

use lazy_github_core::models::Issue;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::effect::{GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::details::{BodyRenderCx, DetailsBody};
pub(super) use comments::IssueComments;
pub(super) use overview::IssueOverview;

/// `IssueOverviewTabPane.BINDINGS`: edit (#181), in the footer while the Overview's content has focus.
pub const ISSUE_OVERVIEW_BINDINGS: &[BindingId] = &[BindingId::IssueEdit];

/// The two tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueTab {
    Overview,
    Comments,
}

impl IssueTab {
    const NAMES: &'static [&'static str] = &["Overview", "Comments"];

    fn at(index: usize) -> IssueTab {
        match index {
            0 => IssueTab::Overview,
            _ => IssueTab::Comments,
        }
    }
}

/// One issue's tabs.
#[derive(Debug)]
pub(in crate::screens::main) struct IssueDetails {
    issue: Arc<Issue>,
    overview: IssueOverview,
    comments: IssueComments,
}

impl IssueDetails {
    /// The tabs of `issue`. Each tab starts its requests here, as Textual mounts every pane at once.
    pub(super) fn new(issue: Issue, cx: &mut Cx<'_>) -> Self {
        let issue = Arc::new(issue);
        Self {
            overview: IssueOverview::new(Arc::clone(&issue)),
            comments: IssueComments::new(&issue, cx),
            issue,
        }
    }

    /// The issue shown.
    pub(in crate::screens::main) fn issue(&self) -> &Issue {
        &self.issue
    }

    /// The Overview tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn overview(&self) -> &IssueOverview {
        &self.overview
    }

    /// The Comments tab.
    #[cfg(test)]
    pub(in crate::screens::main) fn comments(&self) -> &IssueComments {
        &self.comments
    }
}

impl DetailsBody for IssueDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        IssueTab::NAMES
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        self.comments.on_response(id, response)
    }

    fn close(&self, cx: &mut Cx<'_>) {
        self.comments.close(cx);
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.scroll(scroll),
            IssueTab::Comments => self.comments.scroll(scroll),
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.wheel(wheel),
            IssueTab::Comments => self.comments.wheel(wheel),
        }
    }

    /// The pane draws through [`Self::render_timed`]; this fallback draws the loading overlay as static text.
    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        let cx = BodyRenderCx {
            theme,
            now: Instant::now(),
            animations: false,
            focused,
        };
        self.render_timed(tab, frame, area, &cx);
    }

    fn render_timed(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.render(frame, area, cx.theme),
            IssueTab::Comments => self.comments.render(frame, area, cx),
        }
    }

    fn next_deadline(&self, tab: usize, now: Instant, animations: bool) -> Option<Instant> {
        match IssueTab::at(tab) {
            IssueTab::Overview => None,
            IssueTab::Comments => self.comments.next_deadline(now, animations),
        }
    }

    fn focus_step(&mut self, tab: usize, forward: bool) -> bool {
        IssueTab::at(tab) == IssueTab::Comments && self.comments.focus_step(forward)
    }

    fn focus_first(&mut self, tab: usize) {
        if IssueTab::at(tab) == IssueTab::Comments {
            self.comments.focus_first();
        }
    }

    fn focus_last(&mut self, tab: usize) {
        if IssueTab::at(tab) == IssueTab::Comments {
            self.comments.focus_last();
        }
    }

    fn click(&mut self, tab: usize, position: Position) {
        if IssueTab::at(tab) == IssueTab::Comments {
            self.comments.click(position);
        }
    }

    fn maximizable(&self, tab: usize) -> bool {
        IssueTab::at(tab) == IssueTab::Comments && self.comments.block_focused()
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match IssueTab::at(tab) {
            IssueTab::Overview => vec![ChainLink::new(Owner::IssueOverview, ISSUE_OVERVIEW_BINDINGS)],
            IssueTab::Comments => self.comments.chain(),
        }
    }

    /// Editing an issue (#181), a new comment and a reply (#182) aren't built yet: the key is consumed, as in
    /// Python.
    fn on_binding(&mut self, _tab: usize, _owner: Owner, id: BindingId) -> Option<Action> {
        Some(Action::Unimplemented(id))
    }
}

#[cfg(test)]
mod tests;
