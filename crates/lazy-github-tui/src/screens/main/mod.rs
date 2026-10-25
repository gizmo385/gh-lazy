//! The main screen: the status box on top, the four selection sections on the left, details (and the optional
//! command log) on the right, and the footer row. Python's `LazyGithubMainScreen`.

mod details;
pub mod layout;
mod sections;
mod status;

use lazy_github_core::config::AppearanceSettings;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::component::{Component, Cx, RenderCx};
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
}

/// The main screen's actions. None yet.
#[derive(Debug)]
pub enum MainAction {}

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
        }
    }

    /// What has focus.
    pub fn focus(&self) -> FocusTarget {
        self.focus
    }

    /// Moves focus. #75 drives this from keys and clicks.
    pub fn set_focus(&mut self, focus: FocusTarget) {
        self.focus = focus;
    }
}

impl Component for MainScreen {
    type Action = MainAction;

    fn update(&mut self, action: MainAction, _cx: &mut Cx<'_>) {
        match action {}
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let boxes = layout(&LayoutInput {
            area,
            sections: self.sections,
            focus: Some(self.focus),
            command_log: self.command_log_displayed,
        });

        self.status.render(frame, boxes.status, theme);
        for ((id, rect), title) in SectionId::ALL.into_iter().zip(boxes.sections).zip(&self.section_titles) {
            if let Some(rect) = rect {
                let focused = self.focus.section() == Some(id);
                frame.render_widget(titled_box(title, focused, theme), rect);
            }
        }

        let details = titled_box(&self.details_title, self.focus.in_details(), theme);
        let inner = details.inner(boxes.details);
        frame.render_widget(details, boxes.details);
        self.details.render(frame, inner, theme);

        if let Some(rect) = boxes.command_log {
            let focused = self.focus == FocusTarget::CommandLog;
            frame.render_widget(titled_box(&self.command_log_title, focused, theme), rect);
        }

        // Footer content arrives with #76.
        let footer_bg = theme.color(theme.palette.footer_background, theme.background());
        frame.render_widget(Block::new().style(Style::new().bg(footer_bg)), boxes.footer);
    }
}

#[cfg(test)]
mod tests;
