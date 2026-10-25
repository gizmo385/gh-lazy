//! Inside the Details box: a tab strip with the Introduction tab, and the intro under it. Python's
//! `SelectionDetailsContainer` with `LazyGithubInfoTabPane`.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::info::InfoView;
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::theme::Theme;

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
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let tabs_height = area.height.min(TAB_STRIP_HEIGHT);
        self.tabs.render(
            frame,
            Rect {
                height: tabs_height,
                ..area
            },
            theme,
        );
        let body = Rect {
            y: area.y + tabs_height,
            height: area.height - tabs_height,
            ..area
        };
        self.info.render(frame, body, theme);
    }
}
