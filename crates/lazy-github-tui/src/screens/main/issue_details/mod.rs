//! An issue's details: the Overview and Comments tabs. Python's `IssueOverviewTabPane` and
//! `IssueConversationTabPane`, which `MainViewPane.load_issue` puts in the details pane.

mod overview;

use std::sync::Arc;

use lazy_github_core::models::Issue;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::keymap::BindingId;
use crate::theme::Theme;

use super::details::DetailsBody;
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

/// One issue's tabs. The Comments tab (#246) is empty until it lands, and adds its body here.
#[derive(Debug)]
pub(in crate::screens::main) struct IssueDetails {
    issue: Arc<Issue>,
    overview: IssueOverview,
}

impl IssueDetails {
    /// The tabs of `issue`. Each tab starts its requests here, as Textual mounts every pane at once.
    pub(super) fn new(issue: Issue, _cx: &mut Cx<'_>) -> Self {
        let issue = Arc::new(issue);
        Self {
            overview: IssueOverview::new(Arc::clone(&issue)),
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
}

impl DetailsBody for IssueDetails {
    fn tab_names(&self) -> &'static [&'static str] {
        IssueTab::NAMES
    }

    fn scroll(&mut self, tab: usize, scroll: Scroll) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.scroll(scroll),
            IssueTab::Comments => {}
        }
    }

    fn wheel(&mut self, tab: usize, wheel: Wheel) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.wheel(wheel),
            IssueTab::Comments => {}
        }
    }

    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, _focused: bool) {
        match IssueTab::at(tab) {
            IssueTab::Overview => self.overview.render(frame, area, theme),
            IssueTab::Comments => {}
        }
    }

    fn chain(&self, tab: usize) -> Vec<ChainLink> {
        match IssueTab::at(tab) {
            IssueTab::Overview => vec![ChainLink::new(Owner::IssueOverview, ISSUE_OVERVIEW_BINDINGS)],
            IssueTab::Comments => Vec::new(),
        }
    }

    /// Editing an issue (#181) isn't built yet: the key is consumed, as in Python.
    fn on_binding(&mut self, _tab: usize, _owner: Owner, id: BindingId) -> Option<Action> {
        Some(Action::Unimplemented(id))
    }
}

#[cfg(test)]
mod tests;
