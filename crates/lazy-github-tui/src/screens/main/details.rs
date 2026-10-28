//! Inside the Details box: a tab strip, and the active tab's body under it. Before any selection that's the
//! Introduction tab; afterwards the selection's tabs: Repo Overview for a repo, or Overview, Diff and Conversation for
//! a pull request. Python's `SelectionDetailsContainer` and its `TabbedContent`.
//!
//! Each selection's tabs are one [`DetailsBody`]. The pane owns the strip (switching, clicks) and hands everything
//! else to the body, with the active tab's index. A new kind of selection (an issue, a workflow run) adds a [`Body`]
//! variant, a `DetailsPane::show_*` one-liner, and a [`DetailsBody`] impl; nothing else here changes.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::{FullPullRequest, Issue, Repository, WorkflowRun};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, Owner, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::info::{InfoView, Scroll};
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::effect::{GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::PostedComment;
use crate::options::AppOptions;
use crate::theme::Theme;

use super::FocusTarget;
use super::issue_details::IssueDetails;
use super::pr_details::PullRequestDetails;
use super::repo_overview::RepoOverview;
use super::run_details::RunDetails;

/// One selection's tabs, as the details pane drives them. Every method that concerns one tab gets the active tab's
/// index, in [`Self::tab_names`]' order; the body keeps every tab's state (scroll position, open sections) while
/// another one shows.
///
/// The defaults suit a body with no requests, no focus stops inside it and no bindings of its own.
pub(super) trait DetailsBody {
    /// The tab names, in order. The pane builds its strip from them when the body is shown, with the first active.
    fn tab_names(&self) -> &'static [&'static str];

    /// Tab `tab` became the active one: the first tab right after the body is shown, then each tab a switch shows,
    /// the one already showing excepted. A tab that only loads once it's seen (the run's Logs tab, #244) starts its
    /// requests here.
    fn on_tab_shown(&mut self, _tab: usize, _cx: &mut Cx<'_>) {}

    /// A response to one of the body's requests, or `Some(response)` handed back when it isn't one.
    fn on_response(&mut self, _id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        Some(response)
    }

    /// The body is being replaced: it cancels whatever it still has in flight.
    fn close(&self, _cx: &mut Cx<'_>) {}

    /// A `textual.scroll.*` binding with tab `tab`'s content focused.
    fn scroll(&mut self, tab: usize, scroll: Scroll);

    /// A wheel notch over tab `tab`'s content, focused or not.
    fn wheel(&mut self, tab: usize, wheel: Wheel);

    /// Draws tab `tab`'s content into `area`. `focused` says whether focus is inside it ([`FocusTarget::DetailsContent`]).
    fn render(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool);

    /// The inner focus ring: the stops inside tab `tab`'s content (a collapsible's title, a button), which
    /// `tab` / `shift+tab` walk before leaving the content. Moves one stop `forward` or back and says whether it
    /// moved; `false` at either end lets focus leave the content, to the next target of the main ring. The content
    /// itself is the first stop, as Textual focuses a scrolling container before what's inside it.
    fn focus_step(&mut self, _tab: usize, _forward: bool) -> bool {
        false
    }

    /// Focus enters tab `tab`'s content from before it (`tab`): the first stop.
    fn focus_first(&mut self, _tab: usize) {}

    /// Focus enters tab `tab`'s content from after it (`shift+tab`): the last stop.
    fn focus_last(&mut self, _tab: usize) {}

    /// A left press at `position` inside tab `tab`'s content, which then takes focus: the stop under it gets focus,
    /// else the content itself, as Textual focuses the clicked widget or its nearest focusable ancestor.
    fn click(&mut self, _tab: usize, _position: Position) {}

    /// Whether `M` maximizes the details tab area with focus inside tab `tab`'s content: only from a focused stop
    /// that allows it (a button), as the scrolling content itself doesn't.
    fn maximizable(&self, _tab: usize) -> bool {
        false
    }

    /// The binding chain links of tab `tab`'s content with focus inside it, innermost first, in front of the
    /// content's scroll link: Textual's widgets from the focused stop up to the tab pane.
    fn chain(&self, _tab: usize) -> Vec<ChainLink> {
        Vec::new()
    }

    /// A binding matched at one of [`Self::chain`]'s links. `None` lets the key fall through.
    fn on_binding(&mut self, _tab: usize, _owner: Owner, id: BindingId) -> Option<Action> {
        Some(Action::Unimplemented(id))
    }

    /// A binding's action that needs the context (it makes requests), as [`Self::on_binding`] asked for with
    /// [`MainAction::DetailsCommand`](super::MainAction::DetailsCommand).
    fn on_command(&mut self, _tab: usize, _owner: Owner, _id: BindingId, _cx: &mut Cx<'_>) {}

    /// The New comment dialog opened from this body closed, with what it posted, if anything: the body shows it, if
    /// it shows comments.
    /// `reply` says whether it answered a comment or a review.
    fn comment_closed(&mut self, _posted: Option<PostedComment>, _reply: bool, _cx: &mut Cx<'_>) {}

    /// A mouse event over tab `tab`'s content other than a press (a release, a move, the wheel), for a widget inside
    /// it that takes its own (a tree). Says whether it took the event; the pane handles any other.
    fn mouse(&mut self, _tab: usize, _mouse: MouseEvent) -> bool {
        false
    }

    /// How the footer and help panel show binding `id` of [`Self::chain`]'s link `owner` in tab `tab`.
    fn binding_state(&self, _tab: usize, _owner: Owner, _id: BindingId) -> BindingState {
        BindingState::Enabled
    }

    /// A key before the binding chain, with focus inside tab `tab`'s content: a text input inside it (the diff
    /// viewer's file filter) takes the keys it types and its editing keys, as the table's search box does, pasting
    /// `clipboard`. `None` passes the key on.
    fn handle_focused_key(
        &mut self,
        _tab: usize,
        _key: &KeyChord,
        _keymap: &Keymap,
        _clipboard: &str,
    ) -> Option<Action> {
        None
    }

    /// A bracketed paste with focus inside tab `tab`'s content. Says whether a text input inside it took it.
    fn handle_paste(&mut self, _tab: usize, _text: &str) -> bool {
        false
    }

    /// [`Self::render`] with the frame's time and the animation switch, which a body with loading overlays (the PR
    /// Overview's sections) needs. The pane always draws through this; the default hands over to [`Self::render`].
    fn render_timed(&mut self, tab: usize, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        self.render(tab, frame, area, cx.theme, cx.focused);
    }

    /// When tab `tab`'s next animation frame is due, while something in it animates (a loading overlay).
    fn next_deadline(&self, _tab: usize, _now: Instant, _animations: bool) -> Option<Instant> {
        None
    }
}

/// What a body's draw knows, for [`DetailsBody::render_timed`].
#[derive(Debug, Clone, Copy)]
pub(super) struct BodyRenderCx<'a> {
    /// The theme.
    pub(super) theme: &'a Theme,
    /// The time this frame is drawn at.
    pub(super) now: Instant,
    /// Whether loading overlays animate.
    pub(super) animations: bool,
    /// Whether focus is inside the content ([`FocusTarget::DetailsContent`]).
    pub(super) focused: bool,
}

/// A tab switch: `textual.tabs.next_tab`, `textual.tabs.previous_tab` or a click on a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabSwitch {
    /// The next tab, wrapping.
    Next,
    /// The previous tab, wrapping.
    Previous,
    /// The tab at this index.
    To(usize),
}

/// The details pane's content.
#[derive(Debug)]
pub(super) struct DetailsPane {
    tabs: TabStrip,
    body: Body,
    /// Where the strip was drawn last, for clicks.
    tabs_area: Rect,
    /// Whether the bodies' loading overlays animate.
    animations: bool,
}

/// The selection whose tabs show.
#[derive(Debug)]
enum Body {
    /// The Introduction tab, before any selection.
    Intro(InfoView),
    /// The selected repo's Repo Overview tab.
    Repo(Box<RepoOverview>),
    /// The selected pull request's Overview, Diff and Conversation tabs.
    PullRequest(Box<PullRequestDetails>),
    /// The selected issue's Overview and Comments tabs.
    Issue(Box<IssueDetails>),
    /// The selected workflow run's Overview, Jobs and Logs tabs.
    Run(Box<RunDetails>),
    /// A body for testing the pane's hooks.
    #[cfg(test)]
    Test(Box<tests::TestBody>),
}

impl Body {
    fn get(&self) -> &dyn DetailsBody {
        match self {
            Body::Intro(intro) => intro,
            Body::Repo(overview) => overview.as_ref(),
            Body::PullRequest(details) => details.as_ref(),
            Body::Issue(details) => details.as_ref(),
            Body::Run(details) => details.as_ref(),
            #[cfg(test)]
            Body::Test(body) => body.as_ref(),
        }
    }

    fn get_mut(&mut self) -> &mut dyn DetailsBody {
        match self {
            Body::Intro(intro) => intro,
            Body::Repo(overview) => overview.as_mut(),
            Body::PullRequest(details) => details.as_mut(),
            Body::Issue(details) => details.as_mut(),
            Body::Run(details) => details.as_mut(),
            #[cfg(test)]
            Body::Test(body) => body.as_mut(),
        }
    }
}

impl Default for DetailsPane {
    fn default() -> Self {
        let body = Body::Intro(InfoView::default());
        Self {
            tabs: TabStrip::new(body.get().tab_names().iter().copied()),
            body,
            tabs_area: Rect::default(),
            animations: true,
        }
    }
}

/// The Details box's title for a selection labelled `label` (`parity-bot/dotfiles`, `PR #5`, `Issue #12`), with the
/// `main.details.focus` key as the other titles have theirs: `[5] PR #5 Details`.
pub(super) fn details_title(keymap: &Keymap, label: &str) -> String {
    format!("[{}] {label} Details", keymap.title_key(BindingId::MainDetailsFocus))
}

impl DetailsPane {
    /// The pane with the startup options applied: whether loading overlays animate.
    #[must_use]
    pub(super) fn with_options(mut self, options: AppOptions) -> Self {
        self.animations = options.animations;
        self
    }

    /// When the active tab's next animation frame is due, while something in it animates.
    pub(super) fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.body.get().next_deadline(self.tabs.active(), now, self.animations)
    }

    /// Whether the tab strip can take focus: Python's `action_focus_tabs` needs at least one tab.
    pub(super) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// Replaces the tabs with `body`'s, the first active, closing the old body. Python clears the panes and adds new
    /// ones on every selection, the same one included, so every tab starts afresh, scrolled to the top.
    fn show(&mut self, body: Body, cx: &mut Cx<'_>) {
        self.body.get().close(cx);
        self.tabs = TabStrip::new(body.get().tab_names().iter().copied());
        self.body = body;
        self.body.get_mut().on_tab_shown(0, cx);
    }

    /// Shows `repo`'s Repo Overview tab.
    pub(super) fn show_repo(&mut self, repo: &Repository, cx: &mut Cx<'_>) {
        let overview = RepoOverview::new(repo, cx);
        self.show(Body::Repo(Box::new(overview)), cx);
    }

    /// Shows `pr`'s Overview, Diff and Conversation tabs, which all start loading now, as Textual mounts every pane
    /// at once.
    pub(super) fn show_pull_request(&mut self, pr: FullPullRequest, cx: &mut Cx<'_>) {
        let details = PullRequestDetails::new(pr, cx);
        self.show(Body::PullRequest(Box::new(details)), cx);
    }

    /// Shows `issue`'s Overview and Comments tabs, which both start loading now, as Textual mounts every pane at once.
    pub(super) fn show_issue(&mut self, issue: Issue, cx: &mut Cx<'_>) {
        let details = IssueDetails::new(issue, cx);
        self.show(Body::Issue(Box::new(details)), cx);
    }

    /// Shows `run`'s Overview, Jobs and Logs tabs, and requests its jobs, which the three share.
    pub(super) fn show_run(&mut self, run: WorkflowRun, cx: &mut Cx<'_>) {
        let details = RunDetails::new(run, cx);
        self.show(Body::Run(Box::new(details)), cx);
    }

    /// Switches tabs, and tells the body about a tab that became active. Says whether the active tab changed: with
    /// one tab, `left` and `right` do nothing.
    pub(super) fn switch_tab(&mut self, switch: TabSwitch, cx: &mut Cx<'_>) -> bool {
        let changed = match switch {
            TabSwitch::Next => self.tabs.select_next(),
            TabSwitch::Previous => self.tabs.select_previous(),
            TabSwitch::To(index) => self.tabs.select(index),
        };
        if changed {
            self.body.get_mut().on_tab_shown(self.tabs.active(), cx);
        }
        changed
    }

    /// The tab under a left press, if the press is on the strip's label row, against the last frame.
    pub(super) fn tab_at(&self, mouse: MouseEvent) -> Option<usize> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let labels = Rect {
            height: self.tabs_area.height.min(1),
            ..self.tabs_area
        };
        if !labels.contains(Position::new(mouse.column, mouse.row)) {
            return None;
        }
        self.tabs.hit(mouse.column - labels.x)
    }

    /// The tab strip.
    pub(super) fn tabs(&self) -> &TabStrip {
        &self.tabs
    }

    /// The repo whose overview is showing, if any.
    pub(super) fn repo(&self) -> Option<&Repository> {
        match &self.body {
            Body::Repo(overview) => Some(overview.repo()),
            _ => None,
        }
    }

    /// The pull request whose tabs are showing, if any.
    pub(super) fn pull_request(&self) -> Option<&PullRequestDetails> {
        match &self.body {
            Body::PullRequest(details) => Some(details),
            _ => None,
        }
    }

    /// The issue whose tabs are showing, if any.
    pub(super) fn issue(&self) -> Option<&IssueDetails> {
        match &self.body {
            Body::Issue(details) => Some(details),
            _ => None,
        }
    }

    /// The workflow run whose tabs are showing, if any.
    pub(super) fn run(&self) -> Option<&RunDetails> {
        match &self.body {
            Body::Run(details) => Some(details),
            _ => None,
        }
    }

    /// Takes a response to one of the body's requests, and hands back any other. A README response for an overview
    /// that's gone is dropped.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        match self.body.get_mut().on_response(id, response, cx)? {
            GithubResponse::RepoReadme(_) => {
                tracing::debug!("Dropping the README response {id} of a repo no longer shown");
                None
            }
            GithubResponse::WorkflowRunJobs(_) => {
                tracing::debug!("Dropping the jobs response {id} of a workflow run no longer shown");
                None
            }
            GithubResponse::JobLogs(_) => {
                tracing::debug!("Dropping the job logs response {id} no longer awaited");
                None
            }
            GithubResponse::CommitStatus(_) | GithubResponse::CheckRuns(_) | GithubResponse::IssueReactions(_) => {
                tracing::debug!("Dropping the Overview section response {id} no longer awaited");
                None
            }
            GithubResponse::IssueComments(_) | GithubResponse::CommentReactions(_) => {
                tracing::debug!("Dropping the comments response {id} no longer awaited");
                None
            }
            GithubResponse::PullRequestDiff(_) => {
                tracing::debug!("Dropping the diff response {id} of a pull request no longer shown");
                None
            }
            GithubResponse::PullRequestReviews(_) => {
                tracing::debug!("Dropping the reviews response {id} of a pull request no longer shown");
                None
            }
            other => Some(other),
        }
    }

    /// The active tab's links in front of the content's scroll link, with focus inside the content.
    pub(super) fn content_chain(&self) -> Vec<ChainLink> {
        self.body.get().chain(self.tabs.active())
    }

    /// A binding matched at one of [`Self::content_chain`]'s links.
    pub(super) fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        self.body.get_mut().on_binding(self.tabs.active(), owner, id)
    }

    /// [`DetailsBody::on_command`] in the active tab.
    pub(super) fn on_command(&mut self, owner: Owner, id: BindingId, cx: &mut Cx<'_>) {
        self.body.get_mut().on_command(self.tabs.active(), owner, id, cx);
    }

    /// [`DetailsBody::comment_closed`].
    pub(super) fn comment_closed(&mut self, posted: Option<PostedComment>, reply: bool, cx: &mut Cx<'_>) {
        self.body.get_mut().comment_closed(posted, reply, cx);
    }

    /// [`DetailsBody::mouse`] in the active tab.
    pub(super) fn mouse(&mut self, mouse: MouseEvent) -> bool {
        self.body.get_mut().mouse(self.tabs.active(), mouse)
    }

    /// [`DetailsBody::binding_state`] in the active tab.
    pub(super) fn binding_state(&self, owner: Owner, id: BindingId) -> BindingState {
        self.body.get().binding_state(self.tabs.active(), owner, id)
    }

    /// [`DetailsBody::handle_focused_key`] in the active tab.
    pub(super) fn handle_focused_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> Option<Action> {
        self.body
            .get_mut()
            .handle_focused_key(self.tabs.active(), key, keymap, clipboard)
    }

    /// [`DetailsBody::handle_paste`] in the active tab.
    pub(super) fn handle_paste(&mut self, text: &str) -> bool {
        self.body.get_mut().handle_paste(self.tabs.active(), text)
    }

    /// [`DetailsBody::focus_step`] in the active tab.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        self.body.get_mut().focus_step(self.tabs.active(), forward)
    }

    /// [`DetailsBody::focus_first`] or [`DetailsBody::focus_last`] in the active tab, as focus comes in `forward`
    /// or back.
    pub(super) fn focus_enter(&mut self, forward: bool) {
        let tab = self.tabs.active();
        if forward {
            self.body.get_mut().focus_first(tab);
        } else {
            self.body.get_mut().focus_last(tab);
        }
    }

    /// [`DetailsBody::click`] in the active tab.
    pub(super) fn click(&mut self, position: Position) {
        self.body.get_mut().click(self.tabs.active(), position);
    }

    /// [`DetailsBody::maximizable`] in the active tab.
    pub(super) fn maximizable(&self) -> bool {
        self.body.get().maximizable(self.tabs.active())
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.body.get_mut().scroll(self.tabs.active(), scroll);
    }

    /// A wheel notch over the active tab's body.
    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.body.get_mut().wheel(self.tabs.active(), wheel);
    }

    /// The overview showing, if any.
    #[cfg(test)]
    pub(super) fn overview(&self) -> Option<&RepoOverview> {
        match &self.body {
            Body::Repo(overview) => Some(overview),
            _ => None,
        }
    }

    /// Draws the tabs and the active tab's content into `area`, and returns where each focus target landed, for
    /// clicks.
    pub(super) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        cx: &RenderCx<'_>,
        focus: FocusTarget,
    ) -> [(Rect, FocusTarget); 2] {
        let theme = cx.theme;
        let tabs_height = area.height.min(TAB_STRIP_HEIGHT);
        let tabs = Rect {
            height: tabs_height,
            ..area
        };
        self.tabs.render(frame, tabs, theme, focus == FocusTarget::DetailsTabs);
        self.tabs_area = tabs;
        let body = Rect {
            y: area.y + tabs_height,
            height: area.height - tabs_height,
            ..area
        };
        let tab = self.tabs.active();
        let body_cx = BodyRenderCx {
            theme,
            now: cx.now,
            animations: self.animations,
            focused: focus == FocusTarget::DetailsContent,
        };
        self.body.get_mut().render_timed(tab, frame, body, &body_cx);
        [(tabs, FocusTarget::DetailsTabs), (body, FocusTarget::DetailsContent)]
    }
}

/// The Introduction tab.
impl DetailsBody for InfoView {
    fn tab_names(&self) -> &'static [&'static str] {
        &["Introduction"]
    }

    fn scroll(&mut self, _tab: usize, scroll: Scroll) {
        InfoView::scroll(self, scroll);
    }

    fn wheel(&mut self, _tab: usize, wheel: Wheel) {
        InfoView::wheel(self, wheel);
    }

    fn render(&mut self, _tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, _focused: bool) {
        InfoView::render(self, frame, area, theme);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use lazy_github_core::config::Config;

    use super::*;
    use crate::screens::main::MainAction;
    use crate::test_util::TestApp;

    /// Two tabs; the first has two focus stops inside its content, after the content itself. It records the tabs
    /// the pane shows it.
    #[derive(Debug, Default)]
    pub(in crate::screens::main) struct TestBody {
        /// The stop focused inside the first tab's content: 0 for the content itself.
        stop: usize,
        /// Every tab shown, in order.
        shown: Vec<usize>,
    }

    impl DetailsBody for TestBody {
        fn tab_names(&self) -> &'static [&'static str] {
            &["One", "Two"]
        }

        fn on_tab_shown(&mut self, tab: usize, _cx: &mut Cx<'_>) {
            self.shown.push(tab);
        }

        fn scroll(&mut self, _tab: usize, _scroll: Scroll) {}

        fn wheel(&mut self, _tab: usize, _wheel: Wheel) {}

        fn render(&mut self, _tab: usize, _frame: &mut Frame<'_>, _area: Rect, _theme: &Theme, _focused: bool) {}

        fn focus_step(&mut self, tab: usize, forward: bool) -> bool {
            self.stop = match (tab, forward) {
                (0, true) if self.stop < 2 => self.stop + 1,
                (0, false) if self.stop > 0 => self.stop - 1,
                _ => return false,
            };
            true
        }

        fn focus_first(&mut self, _tab: usize) {
            self.stop = 0;
        }

        fn focus_last(&mut self, tab: usize) {
            self.stop = if tab == 0 { 2 } else { 0 };
        }
    }

    /// The app's details pane showing a fresh [`TestBody`].
    fn app() -> TestApp {
        let mut app = TestApp::new(Config::default());
        let mut pane = std::mem::take(&mut app.app_mut().main_screen_mut().unwrap().details);
        app.with_cx(|cx| pane.show(Body::Test(Box::default()), cx));
        app.app_mut().main_screen_mut().unwrap().details = pane;
        app
    }

    fn body(app: &TestApp) -> &TestBody {
        match &app.app().main_screen().unwrap().details.body {
            Body::Test(body) => body,
            other => panic!("not the test body: {other:?}"),
        }
    }

    fn focus(app: &TestApp) -> FocusTarget {
        app.app().main_screen().unwrap().focus()
    }

    #[test]
    fn the_first_tab_is_shown_at_once_and_each_switch_after() {
        let mut app = app();
        assert_eq!(body(&app).shown, [0]);
        app.press(&["5", "right", "right", "left"]);
        assert_eq!(body(&app).shown, [0, 1, 0, 1]);
        // The tab already showing isn't shown again.
        app.dispatch(Action::Main(MainAction::ClickDetailsTab(1)));
        assert_eq!(body(&app).shown, [0, 1, 0, 1]);
    }

    #[test]
    fn tab_walks_the_inner_stops_before_leaving_the_content() {
        let mut app = app();
        app.press(&["5", "tab"]);
        assert_eq!((focus(&app), body(&app).stop), (FocusTarget::DetailsContent, 0));
        app.press(&["tab"]);
        assert_eq!((focus(&app), body(&app).stop), (FocusTarget::DetailsContent, 1));
        app.press(&["tab"]);
        assert_eq!((focus(&app), body(&app).stop), (FocusTarget::DetailsContent, 2));
        // Past the last stop, focus leaves the content.
        app.press(&["tab"]);
        assert_ne!(focus(&app), FocusTarget::DetailsContent);
        // Coming back from after it starts at the last stop, and leaves past the first.
        app.press(&["shift+tab"]);
        assert_eq!((focus(&app), body(&app).stop), (FocusTarget::DetailsContent, 2));
        app.press(&["shift+tab", "shift+tab"]);
        assert_eq!((focus(&app), body(&app).stop), (FocusTarget::DetailsContent, 0));
        app.press(&["shift+tab"]);
        assert_eq!(focus(&app), FocusTarget::DetailsTabs);
        // A tab without inner stops is one stop.
        app.press(&["right", "tab"]);
        assert_eq!(focus(&app), FocusTarget::DetailsContent);
        app.press(&["tab"]);
        assert_ne!(focus(&app), FocusTarget::DetailsContent);
    }

    #[test]
    fn details_titles_take_the_key_from_the_keymap() {
        let mut app = TestApp::new(Config::default());
        let titles = app.with_cx(|cx| {
            [
                details_title(cx.keymap(), "PR #5"),
                details_title(cx.keymap(), "Workflow Run #42"),
            ]
        });
        assert_eq!(titles, ["[5] PR #5 Details", "[5] Workflow Run #42 Details"]);
    }
}
