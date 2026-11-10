//! A pull request's Diff tab: Python's `PrDiffTabPane`. The diff is fetched with the pull request, whatever tab is
//! active; until it arrives the tab shows the loading overlay, then the diff viewer, or one line saying why there's
//! no diff. `c` on a line of the diff body opens Add Comment, whose comment joins the viewer's pending comments, shown
//! under its line; `x` on one removes it, which is the viewer's own. `r` on a review thread's row opens New comment to
//! reply to the thread (#385), which the Conversation then reloads.
//!
//! Once the diff is in, the tab asks for the current user, to show the Submit review form the viewer may use (the
//! author's, or a reviewer's), and submits the review from it.
//!
//! The review threads under the diff's lines (#390) come from the reviews the Conversation loads, handed over with
//! [`PrDiff::set_reviews`] whenever it loads them, before or after the diff is in.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::MouseEvent;
use lazy_github_core::diff;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::DiffFetchError;
use lazy_github_core::models::{FullPullRequest, Review, User};
use lazy_github_core::review_anchor::{DiffThread, diff_threads};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Cx, Owner};
use crate::components::command_log::Wheel;
use crate::components::diff_view::{DiffStop, DiffView, MissingStatus, PendingComment, ThreadReply, ViewMouse};
use crate::components::info::Scroll;
use crate::components::loading;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::{AddCommentDialog, NewCommentDialog, ReplyTarget};
use crate::theme::Theme;
use crate::toast::Toast;

use super::super::MainAction;
use super::super::details::{BodyRenderCx, MaximizeScope};
use super::PullRequestTarget;

/// The Diff tab: its pull request, where the diff is, and the requests the review form waits on.
#[derive(Debug)]
pub(in crate::screens::main) struct PrDiff {
    pr: Arc<FullPullRequest>,
    state: DiffState,
    /// The current user, asked for once the diff is in, to pick the review form.
    user: Option<RequestId>,
    /// The submitted review, while it's in flight.
    review: Option<RequestId>,
    /// The review threads, once the Conversation has loaded the reviews: the viewer gets them when the diff is in.
    threads: Option<Vec<DiffThread>>,
}

/// Where the diff is.
#[derive(Debug)]
enum DiffState {
    /// Requested: the loading overlay, animated from when it was first drawn.
    Loading { id: RequestId, since: Option<Instant> },
    /// The fetch failed: the message, at the top left (#235's messages, Py bug B56).
    Failed(String),
    /// The viewer, or the filter and why the diff couldn't be parsed.
    Loaded(Box<DiffView>),
}

impl PrDiff {
    /// Requests `pr`'s diff.
    pub(super) fn new(pr: Arc<FullPullRequest>, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::PullRequestDiff {
            repo: Box::new(pr.pr.issue.repo.clone()),
            number: pr.number(),
        });
        Self {
            pr,
            state: DiffState::Loading { id, since: None },
            user: None,
            review: None,
            threads: None,
        }
    }

    /// The Conversation loaded `reviews`: their threads show under their lines, now or once the diff is in. A reload
    /// keeps the expanded threads expanded.
    pub(super) fn set_reviews(&mut self, reviews: &[Review]) {
        let threads = diff_threads(reviews);
        self.threads = Some(threads.clone());
        let repo = self.pr.pr.issue.repo.full_name.clone();
        if let Some(view) = self.view_mut() {
            view.set_threads(threads, &repo);
        }
    }

    /// The Conversation couldn't load the reviews: no threads show, and nothing says so here (the Conversation does).
    pub(super) fn reviews_failed(&mut self) {
        self.threads = None;
        if let Some(view) = self.view_mut() {
            view.set_threads(Vec::new(), "");
        }
    }

    /// The viewer, once the diff is in.
    pub(in crate::screens::main) fn view(&self) -> Option<&DiffView> {
        match &self.state {
            DiffState::Loaded(view) => Some(view),
            DiffState::Loading { .. } | DiffState::Failed(_) => None,
        }
    }

    /// Whether the viewer holds review comments that haven't been sent, which a refresh mustn't drop.
    pub(super) fn has_unsent_comments(&self) -> bool {
        self.view().is_some_and(|view| !view.pending().is_empty())
    }

    fn view_mut(&mut self) -> Option<&mut DiffView> {
        match &mut self.state {
            DiffState::Loaded(view) => Some(view),
            DiffState::Loading { .. } | DiffState::Failed(_) => None,
        }
    }

    /// Takes the answers to the tab's requests (the diff, the current user, the review), and hands back any other
    /// response.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        match response {
            GithubResponse::PullRequestDiff(result) if matches!(self.state, DiffState::Loading { id: awaited, .. } if awaited == id) =>
            {
                self.state = DiffState::from_result(result);
                if self.view().is_some() {
                    self.user = Some(cx.request(GithubRequest::CurrentUser));
                }
                if let Some(threads) = self.threads.clone() {
                    let repo = self.pr.pr.issue.repo.full_name.clone();
                    if let Some(view) = self.view_mut() {
                        view.set_threads(threads, &repo);
                    }
                }
                None
            }
            GithubResponse::CurrentUser(result) if self.user == Some(id) => {
                self.user = None;
                self.on_user(result);
                None
            }
            GithubResponse::CreateReview(result) if self.review == Some(id) => {
                self.review = None;
                self.on_review(result, cx);
                None
            }
            other => Some(other),
        }
    }

    /// Python's `reviewer_is_author`: the form for the viewer, once both the diff and the user are in. Without the
    /// user there's no form, as Python shows no viewer at all then.
    fn on_user(&mut self, result: Result<User, GhError>) {
        let author = self.pr.pr.issue.user.login.clone();
        match result {
            Ok(user) => {
                if let Some(view) = self.view_mut() {
                    view.show_review_form(user.login == author);
                }
            }
            // Python fetches the user with the diff and shows "Error fetching diff" when it fails; here the diff
            // still shows, and the log says why there's no form.
            Err(error) => tracing::error!("Error fetching the current user for the review form: {error}"),
        }
    }

    /// The review's answer: the button comes back either way. A new review reloads the pull request as a fresh
    /// selection does (Py bug B15's recommendation), which drops the pending comments; a refused one keeps them, with
    /// a toast (Py bug B71's recommendation: Python says nothing).
    fn on_review(&mut self, result: Result<Box<Review>, GhError>, cx: &mut Cx<'_>) {
        if let Some(view) = self.view_mut() {
            view.set_submitting(false);
        }
        match result {
            Ok(_) => {
                cx.notify(Toast::info("New review created!"));
                cx.dispatch(Action::Main(MainAction::OpenPullRequest {
                    target: PullRequestTarget::Partial(Box::new(self.pr.pr.clone())),
                    focus: true,
                }));
            }
            Err(error) => {
                tracing::error!("Error submitting review for PR #{}: {error}", self.pr.number());
                cx.notify(Toast::error("Could not submit the review").with_title("Error submitting review"));
            }
        }
    }

    /// The tab is going away: cancels whatever is still out.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let DiffState::Loading { id, .. } = self.state {
            cx.effect(Effect::Cancel(id));
        }
        for id in [self.user, self.review].into_iter().flatten() {
            cx.effect(Effect::Cancel(id));
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
        match &mut self.state {
            DiffState::Loading { since, .. } => {
                let since = *since.get_or_insert(cx.now);
                let elapsed = cx.now.checked_duration_since(since).unwrap_or(Duration::ZERO);
                loading::render(frame.buffer_mut(), area, cx.theme, cx.animations, elapsed);
            }
            DiffState::Failed(message) => {
                let theme = cx.theme;
                let style = Style::new().fg(theme.color(theme.roles().text.plain, theme.background()));
                frame.render_widget(Paragraph::new(Line::styled(message.as_str(), style)), area);
            }
            DiffState::Loaded(view) => {
                view.render(frame, area, cx.theme, cx.focused);
                if let Some(links) = cx.links {
                    let mut links = links.borrow_mut();
                    for link in view.links() {
                        links.push(link.clone());
                    }
                }
            }
        }
    }

    /// [`DiffView::render_popups`], once the diff is in.
    pub(super) fn render_popups(&mut self, frame: &mut Frame<'_>, theme: &Theme) {
        if let Some(view) = self.view_mut() {
            view.render_popups(frame, theme);
        }
    }

    /// [`DiffView::popup_contains`], once the diff is in.
    pub(super) fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.view().is_some_and(|view| view.popup_contains(column, row))
    }

    /// When the loading overlay's next frame is due.
    pub(super) fn next_deadline(&self, now: Instant, animations: bool) -> Option<Instant> {
        (animations && matches!(self.state, DiffState::Loading { .. })).then_some(now + loading::FRAME_INTERVAL)
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

    /// Every stop of the viewer maximizes the details tab area: none sits in a section of its own.
    pub(super) fn maximize_scope(&self) -> MaximizeScope {
        if self.view().is_some() {
            MaximizeScope::TabArea
        } else {
            MaximizeScope::Nothing
        }
    }

    pub(super) fn binding_state(&self, owner: Owner, id: BindingId) -> BindingState {
        self.view()
            .map_or(BindingState::Enabled, |view| view.binding_state(owner, id))
    }

    /// Add Comment, Reply and the review's submit make requests or open a dialog, which needs the context: they run in
    /// [`Self::command`]. The viewer takes everything else.
    pub(super) fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        let view = self.view_mut()?;
        let command = match (owner, id) {
            // Add Comment off a diff line (a header, a `file was removed` row) and Reply off a thread's rows are hidden
            // and do nothing.
            (Owner::DiffBody, BindingId::DiffAddComment) if view.comment_target().is_none() => {
                return Some(Action::Consumed);
            }
            (Owner::DiffBody, BindingId::DiffReplyToThread) if view.reply_target().is_none() => {
                return Some(Action::Consumed);
            }
            (Owner::DiffBody, BindingId::DiffAddComment | BindingId::DiffReplyToThread) => true,
            (Owner::DetailsButton, BindingId::TextualButtonPress) => view.stop() == DiffStop::ReviewSubmit,
            _ => false,
        };
        if command {
            return Some(Action::Main(MainAction::DetailsCommand(owner, id)));
        }
        view.on_binding(owner, id)
    }

    /// Python's `action_add_comment` and `on_trigger_add_comment` (Add Comment for the body's cursor line,
    /// whose comment comes back as [`MainAction::DiffCommentAdded`]), Reply to the cursor's review thread, and
    /// `on_trigger_review_submission`.
    pub(super) fn command(&mut self, owner: Owner, id: BindingId, cx: &mut Cx<'_>) {
        match (owner, id) {
            (Owner::DiffBody, BindingId::DiffAddComment) => self.add_comment_dialog(cx),
            (Owner::DiffBody, BindingId::DiffReplyToThread) => self.reply_dialog(cx),
            (Owner::DetailsButton, BindingId::TextualButtonPress) => self.submit_review(cx),
            _ => {}
        }
    }

    fn add_comment_dialog(&self, cx: &mut Cx<'_>) {
        let Some(target) = self.view().and_then(DiffView::comment_target) else {
            return;
        };
        let dialog = AddCommentDialog::new(target.line_text.clone());
        cx.open_dialog(dialog, move |body| {
            body.map_or(Action::Consumed, |body| {
                Action::Main(MainAction::DiffCommentAdded(Box::new(PendingComment {
                    body,
                    ..target
                })))
            })
        });
    }

    /// New comment quoting the comment under the cursor, posting to its thread's first comment. A posted reply closes
    /// it as the Conversation's replies do ([`MainAction::CommentClosed`]), which reloads the Conversation and with it
    /// the threads; closing without posting changes nothing.
    fn reply_dialog(&self, cx: &mut Cx<'_>) {
        let Some(ThreadReply { root, quoted }) = self.view().and_then(DiffView::reply_target) else {
            return;
        };
        let target = ReplyTarget::Thread {
            quoted: Box::new(quoted),
            root,
        };
        let dialog = NewCommentDialog::new(self.pr.pr.issue.repo.clone(), self.pr.number(), target);
        cx.open_dialog(dialog, |posted| {
            posted.map_or(Action::Consumed, |posted| {
                Action::Main(MainAction::CommentClosed {
                    posted: Some(posted),
                    reply: true,
                })
            })
        });
    }

    /// A reply was posted in the thread whose first comment is `root` (from here or the Conversation): the thread
    /// expands, so the reply shows once the reload's threads arrive.
    pub(super) fn reply_posted(&mut self, root: u64) {
        if let Some(view) = self.view_mut() {
            view.expand_root(root);
        }
    }

    /// Sends the review with every pending comment, on the pull request's head commit, with the button disabled until
    /// the answer comes. A press while one is in flight is ignored; the blank status is refused with Python's toast.
    fn submit_review(&mut self, cx: &mut Cx<'_>) {
        let (repo, number) = (self.pr.pr.issue.repo.clone(), self.pr.number());
        let commit = self.pr.head.sha.clone();
        let Some(view) = self.view_mut() else {
            return;
        };
        if view.is_submitting() {
            tracing::debug!("Ignoring a review submit: one is in flight");
            return;
        }
        match view.review_request(&commit) {
            None => {}
            Some(Err(MissingStatus)) => cx.notify(Toast::error("Please select a status for the new review!")),
            Some(Ok(review)) => {
                view.set_submitting(true);
                self.review = Some(cx.request(GithubRequest::CreateReview {
                    repo: Box::new(repo),
                    number,
                    review,
                }));
            }
        }
    }

    /// Add Comment closed with `comment`: it joins the pending comments, with Python's toast.
    pub(super) fn add_comment(&mut self, comment: PendingComment, cx: &mut Cx<'_>) {
        if let Some(view) = self.view_mut() {
            view.add_comment(comment);
            cx.notify(Toast::info("New comment added"));
        }
    }

    /// A mouse event other than a press: the release of a click on a button. A click on the review's button submits
    /// it, as `enter` does.
    pub(super) fn captures_mouse(&self) -> bool {
        self.view().is_some_and(DiffView::captures_mouse)
    }

    pub(super) fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        self.view_mut().is_some_and(|view| view.captured_mouse(event))
    }

    pub(super) fn mouse(&mut self, event: MouseEvent) -> Option<Action> {
        match self.view_mut()?.mouse(event) {
            ViewMouse::Ignored => None,
            ViewMouse::SubmitReview => Some(Action::Main(MainAction::DetailsCommand(
                Owner::DetailsButton,
                BindingId::TextualButtonPress,
            ))),
        }
    }

    pub(super) fn handle_key(
        &mut self,
        key: &KeyChord,
        keymap: &Keymap,
        clipboard: &str,
        now: Instant,
    ) -> Option<Action> {
        self.view_mut()?.handle_key(key, keymap, clipboard, now)
    }

    pub(super) fn handle_paste(&mut self, text: &str) -> bool {
        self.view_mut().is_some_and(|view| view.handle_paste(text))
    }
}

impl DiffState {
    fn from_result(result: Result<String, DiffFetchError>) -> Self {
        match result {
            Ok(text) => match diff::parse(&text) {
                Ok(diff) => DiffState::Loaded(Box::new(DiffView::new(diff))),
                Err(error) => DiffState::Loaded(Box::new(DiffView::invalid(&error.to_string()))),
            },
            Err(error) => {
                tracing::debug!("Fetching the diff failed: {error:?}");
                DiffState::Failed(error.message())
            }
        }
    }
}

#[cfg(test)]
mod tests;
