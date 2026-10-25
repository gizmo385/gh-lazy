//! The main screen. A placeholder until #74 builds the real layout: one bordered box titled `LazyGithub` above a
//! blank footer row.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::component::{Component, Cx, RenderCx};

/// The main screen.
#[derive(Debug, Default)]
pub struct MainScreen;

/// The main screen's actions. None yet.
#[derive(Debug)]
pub enum MainAction {}

impl Component for MainScreen {
    type Action = MainAction;

    fn update(&mut self, action: MainAction, _cx: &mut Cx<'_>) {
        match action {}
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let [body, _footer] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        let block = Block::bordered()
            .title("LazyGithub")
            .border_style(Style::new().fg(cx.theme.color(cx.theme.palette.primary, cx.theme.background())));
        frame.render_widget(block, body);
    }
}
