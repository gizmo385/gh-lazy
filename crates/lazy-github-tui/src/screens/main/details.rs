//! Inside the Details box: a tab strip with the Introduction tab, and the intro under it. Python's
//! `SelectionDetailsContainer` with `LazyGithubInfoTabPane`.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::info::{InfoView, Scroll};
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::theme::Theme;

use super::FocusTarget;

/// The details pane's content.
#[derive(Debug)]
pub(super) struct DetailsPane {
    tabs: TabStrip,
    info: InfoView,
}

impl Default for DetailsPane {
    fn default() -> Self {
        Self {
            tabs: TabStrip::new(["Introduction"]),
            info: InfoView::default(),
        }
    }
}

impl DetailsPane {
    /// Whether the tab strip can take focus: Python's `action_focus_tabs` needs at least one tab.
    pub(super) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.info.scroll(scroll);
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
        self.info.render(frame, body, theme);
        [(tabs, FocusTarget::DetailsTabs), (body, FocusTarget::DetailsContent)]
    }
}
