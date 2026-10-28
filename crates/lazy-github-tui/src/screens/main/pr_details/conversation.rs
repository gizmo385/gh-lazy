//! A pull request's Conversation tab: its reviews, each a collapsible section holding the review's body and the
//! threads of inline comments it starts, then the pull request's comments, all in one scroll area. Python's
//! `PrConversationTabPane`, `ReviewContainer` and `ReviewConversation`.
//!
//! The reviews and the comments are requested with the pull request, whatever tab is active; the tab shows the loading
//! overlay until both have answered. Then each shown comment's reactions are requested, and added to its block as
//! they arrive. The same reviews fill the Overview's reviews section.
//!
//! Inside the tab, focus walks the scroll area itself, then each review's title and, while the review is open, the
//! comment blocks of its threads (and each block's reactions), then the comment blocks after the reviews.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::{CommentRef, FullPullRequest, IssueComment, Review, ReviewState, review_threads};
use lazy_github_core::references;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tokio::time::Instant;

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::collapsible::{CONTENT_INDENT, Collapsible, ContentCx, SectionFocus};
use crate::components::command_log::Wheel;
use crate::components::comment::{CommentBlock, CommentHit, CommentLayout, CommentStop};
use crate::components::info::Scroll;
use crate::components::list_view::{append, pad_line};
use crate::components::loading;
use crate::components::markdown::{self, Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::modal::{NewCommentDialog, ReplyTarget};
use crate::theme::{Accent, Paint, Theme};

use super::super::MainAction;
use super::super::details::{BodyRenderCx, MaximizeScope, SectionKey, SectionLayout, SectionRef};
use super::super::overview_text::accent;
use super::overview::PrOverview;

/// `PrConversationTabPane.BINDINGS`: a new comment (#182), in the footer while focus is anywhere in the tab.
const CONVERSATION_BINDINGS: &[BindingId] = &[BindingId::ConversationCommentNew];

/// `ReviewContainer.BINDINGS`: reply to the review (#182), while focus is on its title or inside it.
const REVIEW_BINDINGS: &[BindingId] = &[BindingId::ConversationReviewReply];

/// A focused review title's bindings: Textual's `CollapsibleTitle`.
const TITLE_BINDINGS: &[BindingId] = &[BindingId::TextualCollapsibleToggle];

/// What shows with neither reviews nor comments.
const EMPTY: &str = "No reviews or comments available";

/// What shows when the reviews or the comments couldn't be fetched (Py bug B16).
const FAILED: &str = "Failed to load reviews and comments";

/// What a review with neither a body nor comments holds.
const NO_CONTENT: &str = "No additional review content";

/// A focus stop inside the Conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::screens::main) enum ConversationStop {
    /// The scroll area itself.
    Scroll,
    /// Item `n`'s title: a review.
    Review(usize),
    /// A stop in comment block `n`, by its index in the order the blocks were built.
    Comment(usize, CommentStop),
}

/// A focus stop that survives a reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeptStop {
    Scroll,
    Review(usize),
    Comment(CommentRef),
}

/// Where the tab is.
#[derive(Debug)]
enum State {
    /// Waiting on the reviews and the comments: the loading overlay, animated from when it was first drawn.
    Loading { requests: Requests, since: Option<Instant> },
    /// Both answered.
    Loaded(Box<Body>),
    /// Either failed: one line says so.
    Failed,
}

/// The two requests the tab waits on, and what each has answered so far.
#[derive(Debug)]
struct Requests {
    reviews_id: RequestId,
    comments_id: RequestId,
    reviews: Option<Vec<Review>>,
    comments: Option<Vec<IssueComment>>,
}

/// The Conversation of one pull request.
#[derive(Debug)]
pub(in crate::screens::main) struct PrConversation {
    pr: Arc<FullPullRequest>,
    state: State,
    scroll: ScrollView,
    /// The comment reactions still out, by request, with the block each is for.
    reactions: HashMap<RequestId, usize>,
    /// A reload's requests while the loaded content stays up (after a comment was posted).
    reloading: Option<Requests>,
    /// Whether the reload puts focus back on the same review or comment (after a reply), rather than on the scroll
    /// area (after `n`, as Python's remounted conversation loses its focused widget).
    keep_focus: bool,
    reveal: Option<Reveal>,
    /// Where the content was drawn last, for clicks.
    area: Rect,
    /// Where a section laid out alone (maximized) landed, by section and width: the latest two.
    alone: Vec<(SectionRef, u16, Placements)>,
}

/// What the next draw scrolls into view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    Stop(ConversationStop),
    /// Item `n`, whole: a review after a toggle.
    Item(usize),
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    items: Vec<Item>,
    /// Every shown comment's block, in the order they're shown, with the comment they're of.
    blocks: Vec<(CommentRef, CommentBlock)>,
    /// What a reply to each block answers, in the blocks' order.
    targets: Vec<ReplyTarget>,
    stop: ConversationStop,
    /// Whether focus is inside the content, so the stop is drawn focused.
    focused: bool,
    layouts: LayoutCache,
    /// Where things landed in each cached layout, by width.
    placements: Vec<(u16, Placements)>,
    /// The width of the last layout asked for: the one the last draw used.
    last_width: u16,
}

/// One thing in the scroll area, top to bottom.
#[derive(Debug)]
enum Item {
    Review(ReviewSection),
    /// A comment of the pull request: block `n`.
    Comment(usize),
}

/// A review, laid out as a collapsible section.
#[derive(Debug)]
struct ReviewSection {
    /// What a reply to the review answers.
    target: ReplyTarget,
    login: String,
    state: ReviewState,
    collapsible: Collapsible,
    contents: ReviewContents,
}

/// What a review's section holds.
#[derive(Debug)]
enum ReviewContents {
    /// Neither a body nor comments.
    Nothing,
    /// The body, when it has one, then its threads, each the blocks of its comments in order.
    Content {
        body: Option<Markdown>,
        threads: Vec<Vec<usize>>,
    },
}

impl ReviewSection {
    fn threads(&self) -> &[Vec<usize>] {
        match &self.contents {
            ReviewContents::Content { threads, .. } => threads,
            ReviewContents::Nothing => &[],
        }
    }
}

/// Where everything landed in one layout.
#[derive(Debug, Clone, Default)]
struct Placements {
    /// Every item's rows, by item.
    items: Vec<Range<usize>>,
    /// Each shown review's title: its row, and the columns a click toggles it on. `None` for a comment.
    titles: Vec<Option<(usize, Range<u16>)>>,
    /// Each block's layout and where it landed, while it's shown (a closed review hides its blocks).
    blocks: Vec<Option<PlacedBlock>>,
}

#[derive(Debug, Clone)]
struct PlacedBlock {
    /// Its first row.
    top: usize,
    /// Its left column.
    left: u16,
    layout: CommentLayout,
}

impl PlacedBlock {
    fn rows(&self) -> Range<usize> {
        self.top..self.top + self.layout.rendered.lines.len()
    }
}

impl PrConversation {
    /// The Conversation of `pr`, with its reviews and comments requested: Python's `on_mount`.
    pub(super) fn new(pr: Arc<FullPullRequest>, cx: &mut Cx<'_>) -> Self {
        let reviews_id = cx.request(GithubRequest::PullRequestReviews {
            pr: Box::new((*pr).clone()),
        });
        let comments_id = cx.request(GithubRequest::IssueComments {
            issue: Box::new(pr.pr.issue.clone()),
        });
        Self {
            pr,
            state: State::Loading {
                requests: Requests {
                    reviews_id,
                    comments_id,
                    reviews: None,
                    comments: None,
                },
                since: None,
            },
            scroll: ScrollView::default(),
            reactions: HashMap::new(),
            reloading: None,
            keep_focus: false,
            reveal: None,
            area: Rect::default(),
            alone: Vec::new(),
        }
    }

    fn body(&self) -> Option<&Body> {
        match &self.state {
            State::Loaded(body) => Some(body),
            State::Loading { .. } | State::Failed => None,
        }
    }

    fn body_mut(&mut self) -> Option<&mut Body> {
        match &mut self.state {
            State::Loaded(body) => Some(body),
            State::Loading { .. } | State::Failed => None,
        }
    }

    /// The focused stop, if focus is inside the content.
    #[cfg(test)]
    pub(in crate::screens::main) fn stop(&self) -> ConversationStop {
        self.body().map_or(ConversationStop::Scroll, |body| body.stop)
    }

    /// Whether review item `index` is open.
    #[cfg(test)]
    pub(in crate::screens::main) fn is_open(&self, index: usize) -> bool {
        self.body()
            .and_then(|body| match body.items.get(index) {
                Some(Item::Review(review)) => Some(review.collapsible.is_open()),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// The block of `comment`, once loaded.
    #[cfg(test)]
    pub(in crate::screens::main) fn block(&self, comment: CommentRef) -> Option<&CommentBlock> {
        self.body()?
            .blocks
            .iter()
            .find(|(of, _)| *of == comment)
            .map(|(_, block)| block)
    }

    /// The block index of `comment`, once loaded.
    #[cfg(test)]
    pub(in crate::screens::main) fn block_index(&self, comment: CommentRef) -> Option<usize> {
        self.body()?.blocks.iter().position(|(of, _)| *of == comment)
    }

    /// The first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    // Requests.

    /// Takes a response to one of the tab's requests, and hands back any other. Once the reviews and the comments
    /// are both in, the Overview gets the reviews too.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        overview: &mut PrOverview,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        if let GithubResponse::CommentReactions(result) = response {
            let Some(block) = self.reactions.remove(&id) else {
                return Some(GithubResponse::CommentReactions(result));
            };
            if let Some(body) = self.body_mut() {
                body.blocks[block].1.set_reactions(result);
                body.layouts.clear();
            }
            return None;
        }
        let requests = match (&mut self.state, &mut self.reloading) {
            (_, Some(requests)) | (State::Loading { requests, .. }, None) => requests,
            (State::Loaded(_) | State::Failed, None) => return Some(response),
        };
        let failed = match response {
            GithubResponse::PullRequestReviews(result) if id == requests.reviews_id => match result {
                Ok(reviews) => {
                    requests.reviews = Some(reviews);
                    None
                }
                Err(error) => Some(error),
            },
            GithubResponse::IssueComments(result) if id == requests.comments_id => match result {
                Ok(comments) => {
                    requests.comments = Some(comments);
                    None
                }
                Err(error) => Some(error),
            },
            other => return Some(other),
        };
        if let Some(error) = failed {
            self.fail(&error, overview, cx);
            return None;
        }
        // The tab loads once both have answered.
        let (Some(reviews), Some(comments)) = (&mut requests.reviews, &mut requests.comments) else {
            return None;
        };
        let (reviews, comments) = (std::mem::take(reviews), std::mem::take(comments));
        overview.set_reviews(&reviews);
        let kept = self
            .reloading
            .take()
            .and(std::mem::take(&mut self.keep_focus).then(|| self.kept_stop()))
            .flatten();
        self.load(&reviews, &comments, cx);
        if let Some(kept) = kept {
            self.restore(kept);
        }
        None
    }

    /// Where focus is in the loaded content, in terms that outlive a reload: a review by its place, a comment by
    /// which comment it is.
    fn kept_stop(&self) -> Option<KeptStop> {
        let body = self.body()?;
        Some(match body.stop {
            ConversationStop::Scroll => KeptStop::Scroll,
            ConversationStop::Review(index) => KeptStop::Review(index),
            ConversationStop::Comment(block, _) => KeptStop::Comment(body.blocks.get(block)?.0),
        })
    }

    /// Focus back where it was before a reload, if that's still there; the scroll position stays.
    fn restore(&mut self, kept: KeptStop) {
        let Some(body) = self.body_mut() else {
            return;
        };
        let stop = match kept {
            KeptStop::Review(index) if matches!(body.items.get(index), Some(Item::Review(_))) => {
                Some(ConversationStop::Review(index))
            }
            KeptStop::Comment(comment) => body
                .blocks
                .iter()
                .position(|(of, _)| *of == comment)
                .map(|block| ConversationStop::Comment(block, CommentStop::Block)),
            KeptStop::Scroll | KeptStop::Review(_) => None,
        };
        body.set_stop(stop.unwrap_or(ConversationStop::Scroll));
    }

    /// Either request failed: Python logs this and leaves the spinner up (Py bug B16); the tab and the Overview's
    /// section say so instead. The other request, if it's still out, is cancelled.
    fn fail(&mut self, error: &GhError, overview: &mut PrOverview, cx: &mut Cx<'_>) {
        tracing::error!("Error retrieving PR reviews or comments");
        tracing::debug!("Fetching the conversation failed: {error}");
        let reloading = self.reloading.take();
        if let Some(requests) = reloading.as_ref().or(match &self.state {
            State::Loading { requests, .. } => Some(requests),
            State::Loaded(_) | State::Failed => None,
        }) {
            if requests.reviews.is_none() {
                cx.effect(Effect::Cancel(requests.reviews_id));
            }
            if requests.comments.is_none() {
                cx.effect(Effect::Cancel(requests.comments_id));
            }
        }
        self.state = State::Failed;
        overview.reviews_failed();
    }

    /// Builds the content from the answers, and requests the reactions of every comment it shows: Python's
    /// `handle_reviews_loaded`.
    fn load(&mut self, reviews: &[Review], comments: &[IssueComment], cx: &mut Cx<'_>) {
        let repo = &self.pr.pr.issue.repo;
        let mut blocks = Vec::new();
        let mut targets = Vec::new();
        let mut items = Vec::new();
        let threads = review_threads(reviews);
        for (review, threads) in reviews.iter().zip(threads) {
            // Python hides every comment review without a body (Py bug B10); one that starts threads shows.
            if review.body.is_empty() && review.state == ReviewState::Commented && threads.is_empty() {
                continue;
            }
            let contents = if review.body.is_empty() && review.comments.is_empty() {
                ReviewContents::Nothing
            } else {
                let body = (!review.body.is_empty())
                    .then(|| Markdown::parse(&references::expand(&review.body, &repo.full_name)));
                let threads = threads
                    .iter()
                    .map(|thread| {
                        thread
                            .iter()
                            .map(|comment| {
                                blocks.push((
                                    CommentRef::Review(comment.comment.id),
                                    CommentBlock::new(&comment.comment, &repo.full_name),
                                ));
                                targets.push(ReplyTarget::ReviewComment(Box::new((*comment).clone())));
                                blocks.len() - 1
                            })
                            .collect()
                    })
                    .collect();
                ReviewContents::Content { body, threads }
            };
            let open = review.state != ReviewState::Dismissed && !matches!(contents, ReviewContents::Nothing);
            items.push(Item::Review(ReviewSection {
                target: ReplyTarget::Review(Box::new(review.clone())),
                login: review.user.login.clone(),
                state: review.state,
                collapsible: Collapsible::new(open),
                contents,
            }));
        }
        for comment in comments.iter().filter(|comment| !comment.body.is_empty()) {
            blocks.push((
                CommentRef::Issue(comment.id),
                CommentBlock::new(comment, &repo.full_name),
            ));
            targets.push(ReplyTarget::IssueComment(Box::new(comment.clone())));
            items.push(Item::Comment(blocks.len() - 1));
        }

        for (index, (comment, _)) in blocks.iter().enumerate() {
            let id = cx.request(GithubRequest::CommentReactions {
                repo: Box::new(repo.clone()),
                comment: *comment,
            });
            self.reactions.insert(id, index);
        }
        self.state = State::Loaded(Box::new(Body {
            items,
            blocks,
            targets,
            stop: ConversationStop::Scroll,
            // The next draw says.
            focused: false,
            layouts: LayoutCache::default(),
            placements: Vec::new(),
            last_width: 0,
        }));
    }

    /// The tab is going away: cancels whatever it still waits on.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let Some(requests) = &self.reloading {
            for (id, answered) in [
                (requests.reviews_id, requests.reviews.is_some()),
                (requests.comments_id, requests.comments.is_some()),
            ] {
                if !answered {
                    cx.effect(Effect::Cancel(id));
                }
            }
        }
        if let State::Loading { requests, .. } = &self.state {
            if requests.reviews.is_none() {
                cx.effect(Effect::Cancel(requests.reviews_id));
            }
            if requests.comments.is_none() {
                cx.effect(Effect::Cancel(requests.comments_id));
            }
        }
        for id in self.reactions.keys() {
            cx.effect(Effect::Cancel(*id));
        }
    }

    // Focus.

    /// The inner focus ring: the scroll area, each review's title then, while it's open, its blocks' stops, then the
    /// comments' blocks.
    fn stops(&self) -> Vec<ConversationStop> {
        let mut stops = vec![ConversationStop::Scroll];
        let Some(body) = self.body() else {
            return stops;
        };
        let block_stops = |stops: &mut Vec<ConversationStop>, index: usize| {
            stops.extend(
                body.blocks[index]
                    .1
                    .stops()
                    .into_iter()
                    .map(|stop| ConversationStop::Comment(index, stop)),
            );
        };
        for (index, item) in body.items.iter().enumerate() {
            match item {
                Item::Review(review) => {
                    stops.push(ConversationStop::Review(index));
                    if review.collapsible.is_open() {
                        for &block in review.threads().iter().flatten() {
                            block_stops(&mut stops, block);
                        }
                    }
                }
                Item::Comment(block) => block_stops(&mut stops, *block),
            }
        }
        stops
    }

    fn current(&self) -> ConversationStop {
        self.body().map_or(ConversationStop::Scroll, |body| body.stop)
    }

    /// Moves to `stop`, and scrolls it into view at the next draw.
    fn focus(&mut self, stop: ConversationStop) {
        if let Some(body) = self.body_mut() {
            body.set_stop(stop);
        }
        if stop != ConversationStop::Scroll {
            self.reveal = Some(Reveal::Stop(stop));
        }
    }

    /// One stop `forward` or back; `false` past either end.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        let stops = self.stops();
        let current = self.current();
        let at = stops.iter().position(|&stop| stop == current).unwrap_or(0);
        let next = if forward {
            at.checked_add(1).filter(|&next| next < stops.len())
        } else {
            at.checked_sub(1)
        };
        match next {
            Some(next) => {
                self.focus(stops[next]);
                true
            }
            None => false,
        }
    }

    /// Focus enters from before: the scroll area.
    pub(super) fn focus_first(&mut self) {
        self.focus(ConversationStop::Scroll);
    }

    /// Focus enters from after: the last stop.
    pub(super) fn focus_last(&mut self) {
        let last = self.stops().last().copied().unwrap_or(ConversationStop::Scroll);
        self.focus(last);
    }

    /// The review item whose title or contents hold `stop`, if any.
    fn review_of(&self, stop: ConversationStop) -> Option<usize> {
        let body = self.body()?;
        body.review_of(stop)
    }

    /// The chain links in front of the content's scroll link, innermost first: the block's (#233), the review's,
    /// then the tab's, so `r` on a comment in a review replies to the comment.
    pub(super) fn chain(&self) -> Vec<ChainLink> {
        let stop = self.current();
        let mut chain = match stop {
            ConversationStop::Scroll => Vec::new(),
            ConversationStop::Review(_) => vec![ChainLink::new(Owner::CollapsibleTitle, TITLE_BINDINGS)],
            ConversationStop::Comment(_, stop) => CommentBlock::chain(stop),
        };
        if self.review_of(stop).is_some() {
            chain.push(ChainLink::new(Owner::ConversationReview, REVIEW_BINDINGS));
        }
        chain.push(ChainLink::new(Owner::PrConversation, CONVERSATION_BINDINGS));
        chain
    }

    /// A binding at one of [`Self::chain`]'s links. A new comment and the replies open the New comment dialog, which
    /// needs the context: [`Self::command`].
    pub(super) fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        let stop = self.current();
        match (owner, stop) {
            (Owner::CollapsibleTitle, ConversationStop::Review(index)) if id == BindingId::TextualCollapsibleToggle => {
                self.toggle(index);
                Some(Action::Consumed)
            }
            (Owner::CollapsibleTitle | Owner::ListView, ConversationStop::Comment(block, at)) => {
                let body = self.body_mut()?;
                if body.blocks[block].1.on_binding(at, id) {
                    body.layouts.clear();
                    self.reveal = Some(Reveal::Stop(stop));
                    Some(Action::Consumed)
                } else {
                    None
                }
            }
            (Owner::CommentBlock, ConversationStop::Comment(..)) if id == BindingId::ConversationCommentReply => {
                Some(Action::Main(MainAction::DetailsCommand(owner, id)))
            }
            (Owner::ConversationReview, _) if id == BindingId::ConversationReviewReply => {
                Some(Action::Main(MainAction::DetailsCommand(owner, id)))
            }
            (Owner::PrConversation, _) if id == BindingId::ConversationCommentNew => {
                Some(Action::Main(MainAction::DetailsCommand(owner, id)))
            }
            (Owner::CommentBlock | Owner::ConversationReview | Owner::PrConversation, _) => {
                Some(Action::Unimplemented(id))
            }
            _ => None,
        }
    }

    /// What `r` at the current stop answers: the focused block's comment for `conversation.comment.reply`, the review
    /// holding the stop for `conversation.review.reply`.
    fn reply_target(&self, id: BindingId) -> Option<ReplyTarget> {
        let body = self.body()?;
        match (id, body.stop) {
            (BindingId::ConversationCommentReply, ConversationStop::Comment(block, _)) => {
                body.targets.get(block).cloned()
            }
            (BindingId::ConversationReviewReply, stop) => match body.items.get(body.review_of(stop)?)? {
                Item::Review(review) => Some(review.target.clone()),
                Item::Comment(_) => None,
            },
            _ => None,
        }
    }

    /// Opens New comment for the pull request: Python's `action_new_comment`, `reply_to_comment_flow` and
    /// `action_reply_to_review`. After `n` the conversation reloads whether or not something was posted; after a
    /// reply, only when it was (Py bug B18's recommendation: Python never reloads after a reply).
    pub(super) fn command(&mut self, owner: Owner, id: BindingId, cx: &mut Cx<'_>) {
        let (repo, number) = (self.pr.pr.issue.repo.clone(), self.pr.number());
        match (owner, id) {
            (Owner::PrConversation, BindingId::ConversationCommentNew) => {
                let dialog = NewCommentDialog::new(repo, number, ReplyTarget::None);
                cx.open_dialog(dialog, |posted| {
                    Action::Main(MainAction::CommentClosed { posted, reply: false })
                });
            }
            (Owner::CommentBlock | Owner::ConversationReview, _) => {
                let Some(target) = self.reply_target(id) else {
                    return;
                };
                let dialog = NewCommentDialog::new(repo, number, target);
                cx.open_dialog(dialog, |posted| {
                    posted.map_or(Action::Consumed, |posted| {
                        Action::Main(MainAction::CommentClosed {
                            posted: Some(posted),
                            reply: true,
                        })
                    })
                });
            }
            _ => {}
        }
    }

    /// Requests the reviews and the comments again, as opening the tab does: Python's `fetch_conversation`. Loaded
    /// content stays up until the answers replace it; with `keep_focus`, focus then goes back to the same review or
    /// comment. What was still out is cancelled.
    pub(super) fn reload(&mut self, keep_focus: bool, cx: &mut Cx<'_>) {
        self.close(cx);
        self.reactions.clear();
        let requests = Requests {
            reviews_id: cx.request(GithubRequest::PullRequestReviews {
                pr: Box::new((*self.pr).clone()),
            }),
            comments_id: cx.request(GithubRequest::IssueComments {
                issue: Box::new(self.pr.pr.issue.clone()),
            }),
            reviews: None,
            comments: None,
        };
        match self.state {
            // The content stays up, with focus where it is, until the new one replaces it.
            State::Loaded(_) => {
                self.reloading = Some(requests);
                self.keep_focus = keep_focus;
            }
            State::Loading { .. } | State::Failed => {
                self.reloading = None;
                self.state = State::Loading { requests, since: None };
            }
        }
    }

    /// Opens or closes review item `index`, and scrolls it into view, as Textual's `Collapsible` does after a toggle.
    fn toggle(&mut self, index: usize) {
        let Some(body) = self.body_mut() else {
            return;
        };
        if let Some(Item::Review(review)) = body.items.get_mut(index) {
            review.collapsible.toggle();
            body.layouts.clear();
            self.reveal = Some(Reveal::Item(index));
        }
    }

    /// A left press at a screen cell, against the last frame: a review's title focuses and toggles it, a block focuses
    /// it (its reactions title toggling, a reaction highlighted), and anything else focuses the scroll area.
    pub(super) fn click(&mut self, position: Position) {
        let stop = self.stop_at(position);
        self.press(stop);
    }

    /// A left press landed on `stop`: a title toggles its section, and the stop takes focus.
    fn press(&mut self, stop: ConversationStop) {
        match stop {
            ConversationStop::Review(index) => self.toggle(index),
            ConversationStop::Comment(block, CommentStop::ReactionsTitle) => {
                if let Some(body) = self.body_mut() {
                    body.blocks[block].1.toggle_reactions();
                    body.layouts.clear();
                }
            }
            ConversationStop::Comment(..) | ConversationStop::Scroll => {}
        }
        self.focus(stop);
    }

    /// The stop drawn at a screen cell, highlighting the reaction there.
    fn stop_at(&mut self, position: Position) -> ConversationStop {
        let area = self.area;
        let offset = self.scroll.offset();
        let Some(body) = self.body_mut() else {
            return ConversationStop::Scroll;
        };
        let width = body.last_width;
        if !area.contains(position) || position.x >= area.x.saturating_add(width) {
            return ConversationStop::Scroll;
        }
        let row = offset + usize::from(position.y - area.y);
        let x = position.x - area.x;
        let Some(placements) = body.placements(width).cloned() else {
            return ConversationStop::Scroll;
        };
        body.hit(&placements, row, x)
    }

    // Maximize.

    /// What `M` maximizes from the focused stop: a review's title, or a comment block in it, its review's section; a
    /// reactions title or list, its section; a comment outside any review, the details tab area. The scroll area
    /// doesn't allow it.
    pub(super) fn maximize_scope(&self) -> MaximizeScope {
        let Some(body) = self.body() else {
            return MaximizeScope::Nothing;
        };
        let section = match body.stop {
            ConversationStop::Scroll => return MaximizeScope::Nothing,
            ConversationStop::Comment(block, CommentStop::ReactionsTitle | CommentStop::ReactionsList) => {
                SectionRef::CommentReactions(body.blocks[block].0)
            }
            stop => match body.review_of(stop) {
                Some(review) => SectionRef::Review(review),
                None => return MaximizeScope::TabArea,
            },
        };
        MaximizeScope::Section(SectionKey(section))
    }

    /// Whether `section` is shown: a review, or a comment's reactions section.
    pub(super) fn shows_section(&self, section: SectionRef) -> bool {
        let Some(body) = self.body() else {
            return false;
        };
        match section {
            SectionRef::Review(index) => matches!(body.items.get(index), Some(Item::Review(_))),
            SectionRef::CommentReactions(comment) => body
                .blocks
                .iter()
                .any(|(of, block)| *of == comment && block.has_reactions()),
            SectionRef::Overview(_) | SectionRef::File(_) | SectionRef::Hunk(..) => false,
        }
    }

    /// `section`'s own stops: a review's title and, while it's open, its blocks' stops; a reactions section's title
    /// and open list.
    fn section_stops(&self, section: SectionRef) -> Vec<ConversationStop> {
        let Some(body) = self.body() else {
            return Vec::new();
        };
        let inside = |stop: ConversationStop| match (section, stop) {
            (SectionRef::Review(index), stop) => body.review_of(stop) == Some(index),
            (
                SectionRef::CommentReactions(comment),
                ConversationStop::Comment(block, CommentStop::ReactionsTitle | CommentStop::ReactionsList),
            ) => body.blocks[block].0 == comment,
            _ => false,
        };
        self.stops().into_iter().filter(|&stop| inside(stop)).collect()
    }

    /// One stop `forward` or back among `section`'s stops, wrapping; from a stop outside it, its first.
    pub(super) fn focus_step_in(&mut self, section: SectionRef, forward: bool) {
        let stops = self.section_stops(section);
        let Some(&first) = stops.first() else { return };
        let current = self.current();
        let next = match stops.iter().position(|&stop| stop == current) {
            Some(at) if forward => stops[(at + 1) % stops.len()],
            Some(at) => stops[(at + stops.len() - 1) % stops.len()],
            None => first,
        };
        self.focus(next);
    }

    /// Focus enters maximized `section`: its first stop.
    pub(super) fn focus_section(&mut self, section: SectionRef) {
        if let Some(&first) = self.section_stops(section).first() {
            self.focus(first);
        }
    }

    /// `section` laid out alone `width` wide, focused as in the pane: a review without the focused review's box, as
    /// Python draws it maximized. `None` while it isn't shown.
    pub(super) fn layout_section(
        &mut self,
        section: SectionRef,
        width: u16,
        theme: &Theme,
        focused: bool,
    ) -> Option<SectionLayout> {
        let body = self.body_mut()?;
        let mut placements = body.no_placements();
        let mut rendered = Rendered::default();
        match section {
            SectionRef::Review(index) => {
                let Some(Item::Review(review)) = body.items.get(index) else {
                    return None;
                };
                let (focused_review, title, block_focus) = body.focus_state(focused);
                let focus = Focus {
                    review: focused_review,
                    title,
                    blocks: &block_focus,
                };
                place_review(
                    &mut rendered,
                    index,
                    review,
                    &mut body.blocks,
                    &focus,
                    width,
                    theme,
                    false,
                    &mut placements,
                );
            }
            SectionRef::CommentReactions(comment) => {
                let block = body.blocks.iter().position(|(of, _)| *of == comment)?;
                let focus = body.block_focus(block, focused);
                let layout = body.blocks[block].1.layout_reactions(width, focus, theme)?;
                rendered = layout.rendered.clone();
                placements.blocks[block] = Some(PlacedBlock {
                    top: 0,
                    left: 0,
                    layout,
                });
            }
            SectionRef::Overview(_) | SectionRef::File(_) | SectionRef::Hunk(..) => return None,
        }
        let focus_rows = focused
            .then(|| body.rows_in(&placements, Reveal::Stop(body.stop)))
            .flatten();
        self.alone.retain(|(s, w, _)| (*s, *w) != (section, width));
        if self.alone.len() == 2 {
            self.alone.remove(0);
        }
        self.alone.push((section, width, placements));
        Some(SectionLayout { rendered, focus_rows })
    }

    /// A left press on maximized `section` at `row` and `column` of its layout `width` wide, as of the last frame:
    /// as in the pane. Elsewhere in a reactions section, its block takes focus, the nearest focusable widget around
    /// it; elsewhere in a review, the scroll area.
    pub(super) fn click_section(&mut self, section: SectionRef, row: usize, column: u16, width: u16) {
        let placements = self
            .alone
            .iter()
            .find(|(s, w, _)| (*s, *w) == (section, width))
            .map(|(_, _, placements)| placements.clone());
        let stop = match (placements, self.body_mut()) {
            (Some(placements), Some(body)) if column < width => body.hit(&placements, row, column),
            _ => ConversationStop::Scroll,
        };
        let stop = match (section, stop) {
            (SectionRef::CommentReactions(comment), ConversationStop::Scroll) => self
                .body()
                .and_then(|body| body.blocks.iter().position(|(of, _)| *of == comment))
                .map_or(ConversationStop::Scroll, |block| {
                    ConversationStop::Comment(block, CommentStop::Block)
                }),
            (_, stop) => stop,
        };
        self.press(stop);
    }
}

impl Body {
    /// The stop at `row` and column `x` of the layout `placements` describe, highlighting the reaction there; the
    /// scroll area when it's nothing else.
    fn hit(&mut self, placements: &Placements, row: usize, x: u16) -> ConversationStop {
        let body = self;
        for (index, title) in placements.titles.iter().enumerate() {
            if let Some((title_row, columns)) = title
                && row == *title_row
                && columns.contains(&x)
            {
                return ConversationStop::Review(index);
            }
        }
        for (index, placed) in placements.blocks.iter().enumerate() {
            let Some(placed) = placed else {
                continue;
            };
            if !placed.rows().contains(&row) || x < placed.left {
                continue;
            }
            match placed.layout.hit(row - placed.top, x - placed.left) {
                Some(CommentHit::Block) => return ConversationStop::Comment(index, CommentStop::Block),
                Some(CommentHit::ReactionsTitle) => {
                    return ConversationStop::Comment(index, CommentStop::ReactionsTitle);
                }
                Some(CommentHit::ReactionsItem(item)) => {
                    body.blocks[index].1.set_reaction_highlight(Some(item));
                    body.layouts.clear();
                    return ConversationStop::Comment(index, CommentStop::ReactionsList);
                }
                None => {}
            }
        }
        ConversationStop::Scroll
    }
}

impl PrConversation {
    // Drawing.

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        self.area = area;
        let theme = cx.theme;
        match &mut self.state {
            State::Loading { since, .. } => {
                let since = *since.get_or_insert(cx.now);
                let elapsed = cx.now.checked_duration_since(since).unwrap_or(Duration::ZERO);
                loading::render(frame.buffer_mut(), area, theme, cx.animations, elapsed);
            }
            State::Failed => {
                let style = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
                frame.render_widget(Paragraph::new(Line::styled(FAILED, style)), area);
            }
            State::Loaded(body) => {
                body.set_focused(cx.focused);
                if let Some(reveal) = self.reveal.take() {
                    let width = self.scroll.measure(area, theme, body.as_mut());
                    if let Some(rows) = body.rows_to_reveal(reveal, width) {
                        self.scroll.reveal(rows);
                    }
                }
                self.scroll.render(frame, area, theme, body.as_mut());
            }
        }
    }

    /// When the loading overlay's next frame is due.
    pub(super) fn next_deadline(&self, now: Instant, animations: bool) -> Option<Instant> {
        (animations && matches!(self.state, State::Loading { .. })).then_some(now + loading::FRAME_INTERVAL)
    }
}

impl Body {
    fn set_stop(&mut self, stop: ConversationStop) {
        if self.stop != stop {
            self.stop = stop;
            self.layouts.clear();
        }
    }

    /// Focus moved in or out of the content. Only a stop other than the scroll area looks different.
    fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if self.stop != ConversationStop::Scroll {
                self.layouts.clear();
            }
        }
    }

    /// The review item whose title or contents hold `stop`, if any.
    fn review_of(&self, stop: ConversationStop) -> Option<usize> {
        match stop {
            ConversationStop::Scroll => None,
            ConversationStop::Review(index) => Some(index),
            ConversationStop::Comment(block, _) => self.items.iter().position(|item| match item {
                Item::Review(review) => review.threads().iter().flatten().any(|&b| b == block),
                Item::Comment(_) => false,
            }),
        }
    }

    fn placements(&self, width: u16) -> Option<&Placements> {
        self.placements
            .iter()
            .find(|(w, _)| *w == width)
            .map(|(_, placements)| placements)
    }

    /// The content rows to scroll into view for `reveal`, in the layout `width` wide.
    fn rows_to_reveal(&self, reveal: Reveal, width: u16) -> Option<Range<usize>> {
        self.rows_in(self.placements(width)?, reveal)
    }

    /// The rows to scroll into view for `reveal`, by `placements`.
    fn rows_in(&self, placements: &Placements, reveal: Reveal) -> Option<Range<usize>> {
        match reveal {
            Reveal::Item(index) => placements.items.get(index).cloned(),
            Reveal::Stop(ConversationStop::Scroll) => None,
            Reveal::Stop(ConversationStop::Review(index)) => {
                let (row, _) = placements.titles.get(index)?.as_ref()?;
                Some(*row..row + 1)
            }
            Reveal::Stop(ConversationStop::Comment(block, stop)) => {
                let placed = placements.blocks.get(block)?.as_ref()?;
                let shift = |rows: Range<usize>| rows.start + placed.top..rows.end + placed.top;
                let reactions = placed.layout.reactions.as_ref();
                match stop {
                    CommentStop::Block => Some(placed.rows()),
                    CommentStop::ReactionsTitle => reactions.map(|at| shift(at.title_row..at.title_row + 1)),
                    CommentStop::ReactionsList => {
                        let highlight = self.blocks[block].1.reaction_highlight();
                        match highlight.and_then(|item| placed.layout.reaction_items.get(item)) {
                            Some(rows) => Some(shift(rows.clone())),
                            None => reactions.map(|at| shift(at.content.clone())),
                        }
                    }
                }
            }
        }
    }

    /// The stop inside block `index` that has focus, if focus is on it or inside it, `focused` saying whether focus
    /// is inside the content.
    fn block_focus(&self, index: usize, focused: bool) -> Option<CommentStop> {
        match self.stop {
            ConversationStop::Comment(block, stop) if focused && block == index => Some(stop),
            _ => None,
        }
    }

    /// Where focus is, as far as the layout goes, `focused` saying whether focus is inside the content: the review
    /// holding it, the review whose title has it, and each block's focused stop.
    fn focus_state(&self, focused: bool) -> (Option<usize>, Option<usize>, Vec<Option<CommentStop>>) {
        let review = if focused { self.review_of(self.stop) } else { None };
        let title = match self.stop {
            ConversationStop::Review(index) if focused => Some(index),
            _ => None,
        };
        let blocks = (0..self.blocks.len()).map(|i| self.block_focus(i, focused)).collect();
        (review, title, blocks)
    }

    /// An empty set of placements for the items and blocks.
    fn no_placements(&self) -> Placements {
        Placements {
            items: Vec::new(),
            titles: vec![None; self.items.len()],
            blocks: vec![None; self.blocks.len()],
        }
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        self.last_width = width;
        let (focused_review, title_focus, block_focus) = self.focus_state(self.focused);
        let Body {
            items,
            blocks,
            layouts,
            placements,
            ..
        } = self;
        layouts.get(width, theme, || {
            let focus = Focus {
                review: focused_review,
                title: title_focus,
                blocks: &block_focus,
            };
            let (rendered, placed) = conversation(items, blocks, &focus, width, theme);
            placements.retain(|(w, _)| *w != width);
            if placements.len() == 2 {
                placements.remove(0);
            }
            placements.push((width, placed));
            rendered
        })
    }
}

/// Where focus is, as far as the layout goes.
struct Focus<'a> {
    /// The review item with focus on its title or inside it: drawn in a box.
    review: Option<usize>,
    /// The review item whose title has focus.
    title: Option<usize>,
    /// Each block's focused stop.
    blocks: &'a [Option<CommentStop>],
}

/// The whole tab laid out `width` cells wide, and where everything landed.
fn conversation(
    items: &[Item],
    blocks: &mut [(CommentRef, CommentBlock)],
    focus: &Focus<'_>,
    width: u16,
    theme: &Theme,
) -> (Rendered, Placements) {
    let mut out = Rendered::default();
    let mut placements = Placements {
        items: Vec::with_capacity(items.len()),
        titles: vec![None; items.len()],
        blocks: vec![None; blocks.len()],
    };
    if width == 0 {
        return (out, placements);
    }
    if items.is_empty() {
        let style = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
        append(&mut out, markdown::wrap_text(&[TextRun::new(EMPTY, style)], width), 0);
        return (out, placements);
    }
    for (index, item) in items.iter().enumerate() {
        let start = out.lines.len();
        match item {
            Item::Review(review) => {
                let boxed = focus.review == Some(index) && width > 2;
                place_review(
                    &mut out,
                    index,
                    review,
                    blocks,
                    focus,
                    width,
                    theme,
                    boxed,
                    &mut placements,
                );
            }
            Item::Comment(block) => {
                let layout = blocks[*block]
                    .1
                    .layout(width, focus.blocks[*block], theme.background(), theme);
                let top = out.lines.len();
                append(&mut out, layout.rendered.clone(), 0);
                placements.blocks[*block] = Some(PlacedBlock { top, left: 0, layout });
            }
        }
        placements.items.push(start..out.lines.len());
    }
    (out, placements)
}

/// Adds review item `index` to `out`, `width` wide, in the focused review's box when `boxed`, and records where its
/// title and its blocks landed in `placements`. The pane and the maximize view (unboxed, as Python draws it) both lay
/// reviews out through this.
#[allow(clippy::too_many_arguments)] // One review's layout: everything here is an input to it.
fn place_review(
    out: &mut Rendered,
    index: usize,
    review: &ReviewSection,
    blocks: &mut [(CommentRef, CommentBlock)],
    focus: &Focus<'_>,
    width: u16,
    theme: &Theme,
    boxed: bool,
    placements: &mut Placements,
) {
    let (left, inner) = if boxed { (1, width - 2) } else { (0, width) };
    let mut section = Rendered::default();
    let mut placed_blocks = Vec::new();
    let section_focus = SectionFocus {
        title: focus.title == Some(index),
        within: focus.review == Some(index),
    };
    let placement = review.collapsible.layout(
        &mut section,
        &review_title(review, theme),
        section_focus,
        inner,
        theme,
        |content| review_contents(review, blocks, focus, content, theme, &mut placed_blocks),
    );
    let border = boxed.then(|| {
        let under = theme.background();
        Style::new()
            .fg(theme.color(theme.palette.success_lighten_3, under))
            .bg(theme.paint(under))
    });
    let top = out.lines.len() + usize::from(boxed);
    match border {
        Some(border) => {
            let rule = "─".repeat(usize::from(inner));
            out.lines.push(Line::styled(format!("┌{rule}┐"), border));
            for line in &mut section.lines {
                line.spans.insert(0, Span::styled("│", border));
                line.spans.push(Span::styled("│", border));
            }
            append(out, section, left);
            out.lines.push(Line::styled(format!("└{rule}┘"), border));
        }
        None => append(out, section, 0),
    }
    placements.titles[index] = Some((
        top + placement.title_row,
        placement.title_columns.start + left..placement.title_columns.end + left,
    ));
    for (block, row, column, layout) in placed_blocks {
        placements.blocks[block] = Some(PlacedBlock {
            top: top + placement.content.start + row,
            left: left + CONTENT_INDENT + column,
            layout,
        });
    }
}

/// `Review from <login> (<state>)`, the state approved in greenyellow, changes requested in red, else title-cased.
fn review_title(review: &ReviewSection, theme: &Theme) -> Vec<TextRun> {
    let state = match review.state {
        ReviewState::Approved => TextRun::new("Approved", accent(theme, Accent::GreenYellow)),
        ReviewState::ChangesRequested => TextRun::new("Changes Requested", accent(theme, Accent::Red)),
        ReviewState::Commented | ReviewState::Dismissed | ReviewState::Pending => {
            TextRun::new(review.state.title(), Style::new())
        }
    };
    vec![
        TextRun::new(format!("Review from {} (", review.login), Style::new()),
        state,
        TextRun::new(")", Style::new()),
    ]
}

/// A review's section contents, `content.width` wide: its body, then each thread with its left border and a blank row
/// below, or the line saying there's nothing. Each block laid out goes to `placed`, with its row and column in the
/// contents.
fn review_contents(
    review: &ReviewSection,
    blocks: &mut [(CommentRef, CommentBlock)],
    focus: &Focus<'_>,
    content: ContentCx,
    theme: &Theme,
    placed: &mut Vec<(usize, usize, u16, CommentLayout)>,
) -> Rendered {
    let mut out = Rendered::default();
    let under = content.under;
    let fill = Style::new().bg(theme.paint(under));
    let text = Style::new().fg(theme.color(theme.palette.foreground, under));
    match &review.contents {
        ReviewContents::Nothing => {
            append(
                &mut out,
                markdown::wrap_text(&[TextRun::new(NO_CONTENT, text)], content.width),
                0,
            );
        }
        ReviewContents::Content { body, threads } => {
            if let Some(body) = body {
                append(&mut out, filled(body.layout(content.width, theme), under, theme), 0);
            }
            if content.width <= 1 {
                return out;
            }
            // `ReviewConversation { border-left: solid $secondary-background; margin-bottom: 1 }`.
            let border = Style::new()
                .fg(theme.color(theme.palette.secondary_background, under))
                .bg(theme.paint(under));
            for thread in threads {
                for &block in thread {
                    let layout = blocks[block]
                        .1
                        .layout(content.width - 1, focus.blocks[block], under, theme);
                    let mut rows = layout.rendered.clone();
                    for line in &mut rows.lines {
                        line.spans.insert(0, Span::styled("│", border));
                    }
                    placed.push((block, out.lines.len(), 1, layout));
                    append(&mut out, rows, 1);
                }
                let mut gap = Line::default();
                pad_line(&mut gap, content.width, fill);
                out.lines.push(gap);
            }
        }
    }
    out
}

/// `rendered` with its unstyled cells on `under`, the section's background.
fn filled(mut rendered: Rendered, under: Paint, theme: &Theme) -> Rendered {
    for line in &mut rendered.lines {
        for span in &mut line.spans {
            if span.style.bg.is_none() {
                span.style.bg = Some(theme.paint(under));
            }
        }
    }
    rendered
}

#[cfg(test)]
mod tests;
