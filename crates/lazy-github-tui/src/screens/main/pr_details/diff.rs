//! A pull request's Diff tab: Python's `PrDiffTabPane`. The diff is fetched with the pull request, whatever tab is
//! active; until it arrives the tab shows the loading overlay, then the diff viewer, or one line saying why there's
//! no diff.

use std::time::Duration;

use lazy_github_core::diff;
use lazy_github_core::github::DiffFetchError;
use lazy_github_core::models::FullPullRequest;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::diff_view::DiffView;
use crate::components::info::Scroll;
use crate::components::loading;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};

use super::super::details::BodyRenderCx;

/// Where the Diff tab is.
#[derive(Debug)]
pub(in crate::screens::main) enum PrDiff {
    /// Requested: the loading overlay, animated from when it was first drawn.
    Loading { id: RequestId, since: Option<Instant> },
    /// The fetch failed: the message, at the top left (#235's messages, Py bug B56).
    Failed(String),
    /// The viewer, or the filter and why the diff couldn't be parsed.
    Loaded(Box<DiffView>),
}

impl PrDiff {
    /// Requests `pr`'s diff.
    pub(super) fn new(pr: &FullPullRequest, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::PullRequestDiff {
            repo: Box::new(pr.pr.issue.repo.clone()),
            number: pr.number(),
        });
        PrDiff::Loading { id, since: None }
    }

    /// The viewer, once the diff is in.
    pub(in crate::screens::main) fn view(&self) -> Option<&DiffView> {
        match self {
            PrDiff::Loaded(view) => Some(view),
            PrDiff::Loading { .. } | PrDiff::Failed(_) => None,
        }
    }

    fn view_mut(&mut self) -> Option<&mut DiffView> {
        match self {
            PrDiff::Loaded(view) => Some(view),
            PrDiff::Loading { .. } | PrDiff::Failed(_) => None,
        }
    }

    /// Takes the answer to the diff request, and hands back any other response.
    pub(super) fn on_response(&mut self, id: RequestId, response: GithubResponse) -> Option<GithubResponse> {
        match (&*self, response) {
            (PrDiff::Loading { id: awaited, .. }, GithubResponse::PullRequestDiff(result)) if *awaited == id => {
                *self = Self::from_result(result);
                None
            }
            (_, other) => Some(other),
        }
    }

    fn from_result(result: Result<String, DiffFetchError>) -> Self {
        match result {
            Ok(text) => match diff::parse(&text) {
                Ok(diff) => PrDiff::Loaded(Box::new(DiffView::new(diff))),
                Err(error) => PrDiff::Loaded(Box::new(DiffView::invalid(&error.to_string()))),
            },
            Err(error) => {
                tracing::debug!("Fetching the diff failed: {error:?}");
                PrDiff::Failed(error.message())
            }
        }
    }

    /// The tab is going away: cancels the request if it's still out.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let PrDiff::Loading { id, .. } = self {
            cx.effect(Effect::Cancel(*id));
        }
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        if let Some(view) = self.view_mut() {
            view.scroll(scroll);
        }
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        if let Some(view) = self.view_mut() {
            view.wheel(wheel);
        }
    }

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        match self {
            PrDiff::Loading { since, .. } => {
                let since = *since.get_or_insert(cx.now);
                let elapsed = cx.now.checked_duration_since(since).unwrap_or(Duration::ZERO);
                loading::render(frame.buffer_mut(), area, cx.theme, cx.animations, elapsed);
            }
            PrDiff::Failed(message) => {
                let theme = cx.theme;
                let style = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
                frame.render_widget(Paragraph::new(Line::styled(message.as_str(), style)), area);
            }
            PrDiff::Loaded(view) => view.render(frame, area, cx.theme, cx.focused),
        }
    }

    /// When the loading overlay's next frame is due.
    pub(super) fn next_deadline(&self, now: Instant, animations: bool) -> Option<Instant> {
        (animations && matches!(self, PrDiff::Loading { .. })).then_some(now + loading::FRAME_INTERVAL)
    }

    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        self.view_mut().is_some_and(|view| view.focus_step(forward))
    }

    pub(super) fn focus_first(&mut self) {
        if let Some(view) = self.view_mut() {
            view.focus_first();
        }
    }

    pub(super) fn focus_last(&mut self) {
        if let Some(view) = self.view_mut() {
            view.focus_last();
        }
    }

    pub(super) fn click(&mut self, position: Position) {
        if let Some(view) = self.view_mut() {
            view.click(position);
        }
    }

    pub(super) fn chain(&self) -> Vec<ChainLink> {
        self.view().map(DiffView::chain).unwrap_or_default()
    }

    pub(super) fn binding_state(&self, owner: Owner, id: BindingId) -> BindingState {
        self.view()
            .map_or(BindingState::Enabled, |view| view.binding_state(owner, id))
    }

    pub(super) fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        self.view_mut()?.on_binding(owner, id)
    }

    pub(super) fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap) -> Option<Action> {
        self.view_mut()?.handle_key(key, keymap)
    }

    pub(super) fn handle_paste(&mut self, text: &str) -> bool {
        self.view_mut().is_some_and(|view| view.handle_paste(text))
    }
}

#[cfg(test)]
mod tests;
