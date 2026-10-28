//! A pull request's Overview tab: the state, title and author, the change summary and the dates at the left edge,
//! then the reactions, status checks and reviews sections, a rule and the description, scrolling as one. Python's
//! `PrOverviewTabPane`.
//!
//! The sections load after the pull request: the reactions and both checks requests start with the tab, and the
//! reviews arrive with the Conversation tab's. Inside the tab, focus walks the body itself, then each shown
//! section's title and, when it's open, its list.
//!
//! `react.add` opens the Add reaction dialog once the current user is known, then posts the checked reactions one
//! after another and redraws the reactions summary.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::review_summary;
use lazy_github_core::models::{CheckRow, CheckRun, CheckState, CombinedStatus, FullPullRequest, Reactions};
use lazy_github_core::models::{ReactionKind, User};
use lazy_github_core::models::{Review, ReviewState, ReviewSummary, ReviewVerdict, check_rows, overall_state};
use lazy_github_core::{references, time};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};

use crate::action::Action;
use crate::component::{ChainLink, Cx, Owner};
use crate::components::collapsible::{CONTENT_INDENT, Collapsible, Placement, SectionFocus};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::list_view::ListView;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::reactions::ReactionsSummary;
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::modal::AddReactionDialog;
use crate::screens::main::MainAction;
use crate::theme::{Accent, Theme};

use super::super::details::{BodyRenderCx, SectionLayout};
use super::super::overview_text::{HeaderStyles, accent, append, push_label, push_rule};
use super::PR_OVERVIEW_BINDINGS;

/// A focused section title's bindings: Textual's `CollapsibleTitle`.
const TITLE_BINDINGS: &[BindingId] = &[BindingId::TextualCollapsibleToggle];

/// A focused section list's bindings: Textual's `ListView`.
const LIST_BINDINGS: &[BindingId] = &[
    BindingId::TextualListSelect,
    BindingId::TextualListCursorUp,
    BindingId::TextualListCursorDown,
];

/// The Overview's sections, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::screens::main) enum SectionKind {
    /// The reactions on the pull request.
    Reactions,
    /// The head commit's statuses and check runs.
    Checks,
    /// The latest review of each reviewer.
    Reviews,
}

impl SectionKind {
    const ALL: [SectionKind; 3] = [SectionKind::Reactions, SectionKind::Checks, SectionKind::Reviews];

    fn index(self) -> usize {
        match self {
            SectionKind::Reactions => 0,
            SectionKind::Checks => 1,
            SectionKind::Reviews => 2,
        }
    }
}

/// A focus stop inside the Overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::screens::main) enum OverviewStop {
    /// The scrolling body itself.
    Body,
    /// A section's title.
    Title(SectionKind),
    /// An open section's list.
    List(SectionKind),
}

/// One section: whether it's open, what it holds, and its list's highlight.
#[derive(Debug, Default)]
struct Section {
    collapsible: Collapsible,
    state: SectionState,
    list: ListView,
}

/// Where a section's data is.
#[derive(Debug, Default)]
enum SectionState {
    /// Requested: the placeholder title under the loading overlay. Not a focus stop, as Textual skips loading
    /// widgets.
    #[default]
    Loading,
    /// Arrived, with something to show.
    Loaded(Loaded),
    /// The request failed: the title says so, over an empty list (Py bugs B16 and B58).
    Failed,
    /// Arrived with nothing to show: the section isn't drawn.
    Hidden,
}

/// A loaded section's contents.
#[derive(Debug)]
enum Loaded {
    Reactions(ReactionsSummary),
    Checks { overall: CheckState, rows: Vec<CheckRow> },
    Reviews(ReviewSummary),
}

impl Section {
    fn loading() -> Self {
        let mut collapsible = Collapsible::default();
        collapsible.set_loading(true);
        Self {
            collapsible,
            ..Self::default()
        }
    }

    /// Whether it's drawn and can take focus: loaded or failed.
    fn is_settled(&self) -> bool {
        matches!(self.state, SectionState::Loaded(_) | SectionState::Failed)
    }

    fn item_count(&self) -> usize {
        match &self.state {
            SectionState::Loaded(Loaded::Reactions(summary)) => summary.rows.len(),
            SectionState::Loaded(Loaded::Checks { rows, .. }) => rows.len(),
            SectionState::Loaded(Loaded::Reviews(summary)) => summary.rows.len(),
            SectionState::Loading | SectionState::Failed | SectionState::Hidden => 0,
        }
    }

    fn settle(&mut self, state: SectionState) {
        self.state = state;
        self.collapsible.set_loading(false);
    }
}

/// The requests the Overview is waiting on.
#[derive(Debug, Default)]
struct Requests {
    reactions: Option<RequestId>,
    checks: Option<ChecksRequest>,
}

/// The two checks requests, and what each has answered so far: the section shows once both have.
#[derive(Debug)]
struct ChecksRequest {
    status_id: RequestId,
    runs_id: RequestId,
    status: Option<CombinedStatus>,
    runs: Option<Vec<CheckRun>>,
}

/// Add reaction's state: the current user it waits for, then the checked reactions it posts one at a time.
#[derive(Debug, Default)]
struct Reacting {
    /// The `CurrentUser` request `react.add` made; the dialog opens with its answer.
    user_request: Option<RequestId>,
    /// Who the reactions are posted as, once known.
    user: Option<User>,
    /// The checked reactions not posted yet, in the order they were checked.
    queue: VecDeque<ReactionKind>,
    /// The post in flight, and its reaction.
    posting: Option<(RequestId, ReactionKind)>,
    /// Whether a post since the summary was last drawn created a reaction.
    added: bool,
}

/// What the next draw scrolls into view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    Stop(OverviewStop),
    Section(SectionKind),
}

/// The Overview of one pull request: its sections, its scroll position and its inner focus.
#[derive(Debug)]
pub(in crate::screens::main) struct PrOverview {
    scroll: ScrollView,
    body: Body,
    requests: Requests,
    /// The reactions as loaded, with the ones added since. `None` until they load, or when they failed to.
    reactions: Option<Reactions>,
    reacting: Reacting,
    reveal: Option<Reveal>,
    /// Where the body was drawn last, for clicks.
    area: Rect,
    /// Where a section laid out alone (maximized) landed, by section and width: the latest two.
    alone: Vec<(SectionKind, u16, Placed)>,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    pr: Arc<FullPullRequest>,
    /// The description with its references expanded, parsed once. `None` when there's none.
    description: Option<Markdown>,
    sections: [Section; 3],
    stop: OverviewStop,
    /// Whether focus is inside the content, so the stop is drawn focused.
    focused: bool,
    layouts: LayoutCache,
    /// Where the sections landed in each cached layout, by width.
    placements: Vec<(u16, Placements)>,
    /// The width of the last layout asked for: the one the last draw used.
    last_width: u16,
}

/// Where each shown section landed, by [`SectionKind::index`], and the rows of its list items.
type Placements = [Option<Placed>; 3];

#[derive(Debug, Clone, Default)]
struct Placed {
    section: Placement,
    items: Vec<Range<usize>>,
}

impl PrOverview {
    /// The Overview of `pr`, scrolled to the top, with the checks and reactions requested: Python's `on_mount`.
    pub(super) fn new(pr: Arc<FullPullRequest>, cx: &mut Cx<'_>) -> Self {
        let issue = &pr.pr.issue;
        let description = issue
            .body
            .as_deref()
            .filter(|body| !body.is_empty())
            .map(|body| Markdown::parse(&references::expand(body, &issue.repo.full_name)));
        let repo = || Box::new(issue.repo.clone());
        let sha = &pr.head.sha;
        let status_id = cx.request(GithubRequest::CommitStatus {
            repo: repo(),
            sha: sha.clone(),
        });
        let runs_id = cx.request(GithubRequest::CheckRuns {
            repo: repo(),
            sha: sha.clone(),
        });
        let reactions_id = cx.request(GithubRequest::IssueReactions {
            repo: repo(),
            number: issue.number,
        });
        Self {
            scroll: ScrollView::default(),
            requests: Requests {
                reactions: Some(reactions_id),
                checks: Some(ChecksRequest {
                    status_id,
                    runs_id,
                    status: None,
                    runs: None,
                }),
            },
            body: Body {
                pr,
                description,
                sections: [Section::loading(), Section::loading(), Section::loading()],
                stop: OverviewStop::Body,
                focused: false,
                layouts: LayoutCache::default(),
                placements: Vec::new(),
                last_width: 0,
            },
            reactions: None,
            reacting: Reacting::default(),
            reveal: None,
            area: Rect::default(),
            alone: Vec::new(),
        }
    }

    /// The first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// The focused stop, if focus is inside the content.
    #[cfg(test)]
    pub(in crate::screens::main) fn stop(&self) -> OverviewStop {
        self.body.stop
    }

    /// Whether section `kind` is open.
    #[cfg(test)]
    pub(in crate::screens::main) fn is_open(&self, kind: SectionKind) -> bool {
        self.body.sections[kind.index()].collapsible.is_open()
    }

    /// The highlighted item of section `kind`'s list.
    #[cfg(test)]
    pub(in crate::screens::main) fn highlight(&self, kind: SectionKind) -> Option<usize> {
        self.body.sections[kind.index()].list.highlight()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    // Requests.

    /// Takes a response to one of the Overview's requests, and hands back any other.
    pub(super) fn on_response(
        &mut self,
        id: RequestId,
        response: GithubResponse,
        cx: &mut Cx<'_>,
    ) -> Option<GithubResponse> {
        let checks = self.requests.checks.as_ref();
        match response {
            GithubResponse::CurrentUser(result) if self.reacting.user_request == Some(id) => {
                self.reacting.user_request = None;
                self.open_add_reaction(result, cx);
            }
            GithubResponse::AddIssueReaction(result)
                if self.reacting.posting.is_some_and(|(posting, _)| posting == id) =>
            {
                self.on_reaction_posted(result, cx);
            }
            GithubResponse::IssueReactions(result) if self.requests.reactions == Some(id) => {
                self.requests.reactions = None;
                self.on_reactions(result);
            }
            GithubResponse::CommitStatus(result) if checks.is_some_and(|c| c.status_id == id) => {
                self.on_checks(result.map(ChecksPart::Status));
            }
            GithubResponse::CheckRuns(result) if checks.is_some_and(|c| c.runs_id == id) => {
                self.on_checks(result.map(ChecksPart::Runs));
            }
            other => return Some(other),
        }
        None
    }

    /// The Overview is going away: cancels whatever it still waits on.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        let checks = self.requests.checks.iter().flat_map(|checks| {
            [
                checks.status.is_none().then_some(checks.status_id),
                checks.runs.is_none().then_some(checks.runs_id),
            ]
        });
        let reacting = self.reacting.user_request.into_iter();
        for id in self
            .requests
            .reactions
            .into_iter()
            .chain(checks.flatten())
            .chain(reacting)
        {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// Python's `load_reactions`: a failure is logged and, after B16, shown in the title.
    fn on_reactions(&mut self, result: Result<Reactions, GhError>) {
        match result {
            Ok(reactions) => {
                self.reactions = Some(reactions);
                self.show_reactions();
            }
            Err(error) => {
                tracing::error!("Error loading reactions data: {error}");
                self.body.settle(SectionKind::Reactions, SectionState::Failed);
            }
        }
    }

    /// Draws the reactions section from [`Self::reactions`]: Python's `set_reactions`.
    fn show_reactions(&mut self) {
        let summary = self.reactions.as_ref().and_then(ReactionsSummary::new);
        let state = summary.map_or(SectionState::Hidden, |summary| {
            SectionState::Loaded(Loaded::Reactions(summary))
        });
        self.body.settle(SectionKind::Reactions, state);
    }

    // Add reaction.

    /// `react.add`: asks for the current user, whose reactions the dialog leaves out (`gh` remembers it after the
    /// first time). A second press while it's asked waits for the same answer.
    pub(super) fn add_reaction(&mut self, cx: &mut Cx<'_>) {
        if self.reacting.user_request.is_none() {
            self.reacting.user_request = Some(cx.request(GithubRequest::CurrentUser));
        }
    }

    /// The current user arrived: opens the dialog with every reaction they haven't given on the pull request, all
    /// of them when the reactions didn't load. Without the user there's nothing to compare, so no dialog opens.
    fn open_add_reaction(&mut self, result: Result<User, GhError>, cx: &mut Cx<'_>) {
        let user = match result {
            Ok(user) => user,
            Err(error) => {
                tracing::error!("Error fetching the current user for a reaction: {error}");
                return;
            }
        };
        let given: Vec<ReactionKind> = self
            .reactions
            .iter()
            .flat_map(Reactions::by_kind)
            .filter(|(_, users)| users.contains(&user))
            .map(|(kind, _)| kind)
            .collect();
        self.reacting.user = Some(user);
        cx.open_dialog(AddReactionDialog::new(&given), |kinds| {
            Action::Main(MainAction::ReactionsChosen(kinds))
        });
    }

    /// The dialog closed with `kinds` checked, in the order they were checked: they're posted one at a time, in that
    /// order, after any still being posted.
    pub(super) fn post_reactions(&mut self, kinds: Vec<ReactionKind>, cx: &mut Cx<'_>) {
        self.reacting.queue.extend(kinds);
        if self.reacting.posting.is_none() {
            self.post_next_reaction(cx);
        }
    }

    /// Posts the next queued reaction, or, with none left, redraws the summary if any was created.
    fn post_next_reaction(&mut self, cx: &mut Cx<'_>) {
        let Some(kind) = self.reacting.queue.pop_front() else {
            if std::mem::take(&mut self.reacting.added) {
                self.show_reactions();
            }
            return;
        };
        let issue = &self.body.pr.pr.issue;
        let id = cx.request(GithubRequest::AddIssueReaction {
            repo: Box::new(issue.repo.clone()),
            number: issue.number,
            kind,
        });
        self.reacting.posting = Some((id, kind));
    }

    /// A post answered. A created reaction adds the user to its kind, a new kind included (Py bug B66's
    /// recommendation; Python leaves a new kind out); one they already had changes nothing, and a failure is logged
    /// and the rest go on.
    fn on_reaction_posted(&mut self, result: Result<bool, GhError>, cx: &mut Cx<'_>) {
        let Some((_, kind)) = self.reacting.posting.take() else {
            return;
        };
        match result {
            Ok(true) => {
                if let Some(user) = self.reacting.user.clone() {
                    self.reactions.get_or_insert_default().add(kind, user);
                    self.reacting.added = true;
                }
            }
            Ok(false) => {}
            Err(error) => tracing::error!("Error adding reaction {} - {error}", kind.api_name()),
        }
        self.post_next_reaction(cx);
    }

    /// One of the two checks requests answered. The section shows once both have; either failing fails it, without
    /// waiting for the other (Py bug B58: Python's worker fails and the app exits).
    fn on_checks(&mut self, part: Result<ChecksPart, GhError>) {
        let Some(checks) = self.requests.checks.as_mut() else {
            return;
        };
        match part {
            Ok(ChecksPart::Status(status)) => checks.status = Some(status),
            Ok(ChecksPart::Runs(runs)) => checks.runs = Some(runs),
            Err(error) => {
                tracing::error!("Error loading status checks");
                tracing::debug!("Fetching the status checks failed: {error}");
                // The other request may still answer; it's dropped as no longer awaited.
                self.requests.checks = None;
                self.body.settle(SectionKind::Checks, SectionState::Failed);
                return;
            }
        }
        let (Some(status), Some(runs)) = (&checks.status, &checks.runs) else {
            return;
        };
        let rows = check_rows(status, runs);
        let state = if rows.is_empty() {
            SectionState::Hidden
        } else {
            SectionState::Loaded(Loaded::Checks {
                overall: overall_state(status, runs),
                rows,
            })
        };
        self.requests.checks = None;
        self.body.settle(SectionKind::Checks, state);
    }

    /// The Conversation tab's reviews arrived: Python's `add_reviews`. Each author's latest review shows as a row,
    /// and without any reviews the section isn't shown.
    pub(super) fn set_reviews(&mut self, reviews: &[Review]) {
        let state = review_summary(reviews).map_or(SectionState::Hidden, |summary| {
            SectionState::Loaded(Loaded::Reviews(summary))
        });
        self.body.settle(SectionKind::Reviews, state);
    }

    /// The Conversation tab's requests failed, which it logs: the section says so (Py bug B16).
    pub(super) fn reviews_failed(&mut self) {
        self.body.settle(SectionKind::Reviews, SectionState::Failed);
    }

    // Focus.

    /// The inner focus ring: the body, then each shown section's title and, when it's open with something in it,
    /// its list.
    fn stops(&self) -> Vec<OverviewStop> {
        let mut stops = vec![OverviewStop::Body];
        for kind in SectionKind::ALL {
            let section = &self.body.sections[kind.index()];
            if !section.is_settled() {
                continue;
            }
            stops.push(OverviewStop::Title(kind));
            if section.collapsible.is_open() && section.item_count() > 0 {
                stops.push(OverviewStop::List(kind));
            }
        }
        stops
    }

    /// Moves to `stop`, and scrolls it into view at the next draw.
    fn focus(&mut self, stop: OverviewStop) {
        self.body.set_stop(stop);
        if stop != OverviewStop::Body {
            self.reveal = Some(Reveal::Stop(stop));
        }
    }

    /// One stop `forward` or back; `false` past either end.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        let stops = self.stops();
        let at = stops.iter().position(|&stop| stop == self.body.stop).unwrap_or(0);
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

    /// Focus enters from before: the body.
    pub(super) fn focus_first(&mut self) {
        self.focus(OverviewStop::Body);
    }

    /// Focus enters from after: the last stop.
    pub(super) fn focus_last(&mut self) {
        let last = self.stops().last().copied().unwrap_or(OverviewStop::Body);
        self.focus(last);
    }

    // Maximize.

    /// The section `M` maximizes from the focused stop: the one its title or list is in. The body is a scrolling
    /// container, which doesn't allow it.
    pub(super) fn maximize_scope(&self) -> Option<SectionKind> {
        match self.body.stop {
            OverviewStop::Body => None,
            OverviewStop::Title(kind) | OverviewStop::List(kind) => Some(kind),
        }
    }

    /// Whether section `kind` is drawn: loaded or failed.
    pub(super) fn shows_section(&self, kind: SectionKind) -> bool {
        self.body.sections[kind.index()].is_settled()
    }

    /// Section `kind` laid out alone `width` wide, focused as in the pane, or `None` while it isn't drawn.
    pub(super) fn layout_section(
        &mut self,
        kind: SectionKind,
        width: u16,
        theme: &Theme,
        focused: bool,
    ) -> Option<SectionLayout> {
        if !self.shows_section(kind) {
            return None;
        }
        let (focus, list_focused) = self.body.section_focus(kind, focused);
        let section = &self.body.sections[kind.index()];
        let mut rendered = Rendered::default();
        let placed = place_section(&mut rendered, kind, section, focus, list_focused, width, theme);
        let focus_rows = focused
            .then(|| stop_rows(self.body.stop, kind, &placed, section.list.highlight()))
            .flatten();
        self.alone.retain(|(k, w, _)| (*k, *w) != (kind, width));
        if self.alone.len() == 2 {
            self.alone.remove(0);
        }
        self.alone.push((kind, width, placed));
        Some(SectionLayout { rendered, focus_rows })
    }

    /// Section `kind`'s own stops: its title and, while it's open with something in it, its list.
    fn section_stops(&self, kind: SectionKind) -> Vec<OverviewStop> {
        self.stops()
            .into_iter()
            .filter(|&stop| matches!(stop, OverviewStop::Title(k) | OverviewStop::List(k) if k == kind))
            .collect()
    }

    /// One stop `forward` or back among section `kind`'s stops, wrapping; from a stop outside it, its first.
    pub(super) fn focus_step_in(&mut self, kind: SectionKind, forward: bool) {
        let stops = self.section_stops(kind);
        let Some(&first) = stops.first() else { return };
        let next = match stops.iter().position(|&stop| stop == self.body.stop) {
            Some(at) if forward => stops[(at + 1) % stops.len()],
            Some(at) => stops[(at + stops.len() - 1) % stops.len()],
            None => first,
        };
        self.focus(next);
    }

    /// Focus enters maximized section `kind`: its title.
    pub(super) fn focus_section(&mut self, kind: SectionKind) {
        if self.shows_section(kind) {
            self.focus(OverviewStop::Title(kind));
        }
    }

    /// A left press on maximized section `kind` at `row` and `column` of its layout `width` wide, as of the last
    /// frame: the title focuses and toggles it, a list item focuses the list and highlights the item, and anything
    /// else focuses the body, the nearest focusable widget around the section.
    pub(super) fn click_section(&mut self, kind: SectionKind, row: usize, column: u16, width: u16) {
        let placed = self
            .alone
            .iter()
            .find(|(k, w, _)| (*k, *w) == (kind, width))
            .map(|(_, _, placed)| placed.clone());
        let stop = match placed {
            Some(placed) if column < width => self.stop_in(kind, &placed, row, column),
            _ => OverviewStop::Body,
        };
        if let OverviewStop::Title(kind) = stop {
            self.toggle(kind);
        }
        self.focus(stop);
    }

    /// The stop of section `kind`, placed at `placed`, drawn at `row` and `column`, highlighting the list item there.
    fn stop_in(&mut self, kind: SectionKind, placed: &Placed, row: usize, column: u16) -> OverviewStop {
        let at = &placed.section;
        if row == at.title_row && at.title_columns.contains(&column) {
            return OverviewStop::Title(kind);
        }
        if column >= CONTENT_INDENT
            && let Some(item) = placed.items.iter().position(|rows| rows.contains(&row))
        {
            self.body.sections[kind.index()].list.set_highlight(Some(item));
            self.body.layouts.clear();
            return OverviewStop::List(kind);
        }
        OverviewStop::Body
    }

    /// The chain links in front of the content's scroll link, for the focused stop.
    pub(super) fn chain(&self) -> Vec<ChainLink> {
        let overview = ChainLink::new(Owner::PrOverview, PR_OVERVIEW_BINDINGS);
        match self.body.stop {
            OverviewStop::Body => vec![overview],
            OverviewStop::Title(_) => vec![ChainLink::new(Owner::CollapsibleTitle, TITLE_BINDINGS), overview],
            OverviewStop::List(_) => vec![ChainLink::new(Owner::ListView, LIST_BINDINGS), overview],
        }
    }

    /// A binding at the title's or the list's link. Says whether it applied.
    pub(super) fn on_binding(&mut self, id: BindingId) -> bool {
        match (self.body.stop, id) {
            (OverviewStop::Title(kind), BindingId::TextualCollapsibleToggle) => self.toggle(kind),
            (OverviewStop::List(kind), BindingId::TextualListCursorUp | BindingId::TextualListCursorDown) => {
                let section = &mut self.body.sections[kind.index()];
                let len = section.item_count();
                if id == BindingId::TextualListCursorUp {
                    section.list.cursor_up(len);
                } else {
                    section.list.cursor_down(len);
                }
                self.body.layouts.clear();
                self.reveal = Some(Reveal::Stop(OverviewStop::List(kind)));
            }
            // Textual posts `Selected`, which nothing handles.
            (OverviewStop::List(_), BindingId::TextualListSelect) => {}
            _ => return false,
        }
        true
    }

    /// Opens or closes section `kind`, and scrolls it into view, as Textual's `Collapsible` does after a toggle.
    fn toggle(&mut self, kind: SectionKind) {
        self.body.sections[kind.index()].collapsible.toggle();
        self.body.layouts.clear();
        self.reveal = Some(Reveal::Section(kind));
    }

    /// A left press at a screen cell, against the last frame: a section's title focuses and toggles it, a list item
    /// focuses its list and highlights it, and anything else focuses the body.
    pub(super) fn click(&mut self, position: Position) {
        let stop = self.stop_at(position);
        match stop {
            OverviewStop::Title(kind) => self.toggle(kind),
            OverviewStop::List(_) | OverviewStop::Body => {}
        }
        self.focus(stop);
    }

    /// The stop drawn at a screen cell, highlighting the list item there.
    fn stop_at(&mut self, position: Position) -> OverviewStop {
        let area = self.area;
        let width = self.body.last_width;
        if !area.contains(position) || position.x >= area.x.saturating_add(width) {
            return OverviewStop::Body;
        }
        let content_row = self.scroll.offset() + usize::from(position.y - area.y);
        let x = position.x - area.x;
        let Some(placements) = self.body.placements(width).cloned() else {
            return OverviewStop::Body;
        };
        for kind in SectionKind::ALL {
            let Some(placed) = &placements[kind.index()] else {
                continue;
            };
            if !self.body.sections[kind.index()].is_settled() {
                continue;
            }
            let stop = self.stop_in(kind, placed, content_row, x);
            if stop != OverviewStop::Body {
                return stop;
            }
        }
        OverviewStop::Body
    }

    /// When the next frame of a loading overlay is due.
    pub(super) fn next_deadline(&self, now: tokio::time::Instant, animations: bool) -> Option<tokio::time::Instant> {
        self.body
            .sections
            .iter()
            .filter_map(|section| section.collapsible.next_deadline(now, animations))
            .min()
    }

    // Drawing.

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &BodyRenderCx<'_>) {
        self.body.set_focused(cx.focused);
        if let Some(reveal) = self.reveal.take() {
            let width = self.scroll.measure(area, cx.theme, &mut self.body);
            if let Some(rows) = self.body.rows_to_reveal(reveal, width) {
                self.scroll.reveal(rows);
            }
        }
        self.scroll.render(frame, area, cx.theme, &mut self.body);
        self.area = area;

        // The loading overlays, over the rows each loading section takes on screen.
        let width = self.body.last_width;
        let offset = self.scroll.offset();
        let window = offset..offset + usize::from(area.height);
        let Some(placements) = self.body.placements(width).cloned() else {
            return;
        };
        for kind in SectionKind::ALL {
            let Some(placed) = &placements[kind.index()] else {
                continue;
            };
            let rows = &placed.section.rows;
            let (start, end) = (rows.start.max(window.start), rows.end.min(window.end));
            if start >= end {
                continue;
            }
            // Both are within the window, which is at most `area.height` rows.
            let to_screen = |row: usize| u16::try_from(row - offset).unwrap_or(u16::MAX);
            let rect = Rect {
                x: area.x,
                y: area.y + to_screen(start),
                width,
                height: to_screen(end) - to_screen(start),
            };
            self.body.sections[kind.index()].collapsible.render_loading(
                frame.buffer_mut(),
                rect,
                cx.theme,
                cx.now,
                cx.animations,
            );
        }
    }
}

/// One checks request's answer.
enum ChecksPart {
    Status(CombinedStatus),
    Runs(Vec<CheckRun>),
}

impl Body {
    fn settle(&mut self, kind: SectionKind, state: SectionState) {
        self.sections[kind.index()].settle(state);
        self.layouts.clear();
    }

    fn set_stop(&mut self, stop: OverviewStop) {
        if self.stop != stop {
            self.stop = stop;
            self.layouts.clear();
        }
    }

    /// Focus moved in or out of the content. Only a stop other than the body looks different.
    fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if self.stop != OverviewStop::Body {
                self.layouts.clear();
            }
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
        let placements = self.placements(width)?;
        let placed = |kind: SectionKind| placements[kind.index()].as_ref();
        match reveal {
            Reveal::Section(kind) => placed(kind).map(|p| p.section.rows.clone()),
            Reveal::Stop(stop @ (OverviewStop::Title(kind) | OverviewStop::List(kind))) => {
                stop_rows(stop, kind, placed(kind)?, self.sections[kind.index()].list.highlight())
            }
            Reveal::Stop(OverviewStop::Body) => None,
        }
    }

    /// How section `kind` looks with focus where it is, `focused` saying whether focus is inside the content.
    fn section_focus(&self, kind: SectionKind, focused: bool) -> (SectionFocus, bool) {
        let focused = |stop| focused && self.stop == stop;
        let title = focused(OverviewStop::Title(kind));
        let list = focused(OverviewStop::List(kind));
        (
            SectionFocus {
                title,
                within: title || list,
            },
            list,
        )
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        self.last_width = width;
        let focus = SectionKind::ALL.map(|kind| self.section_focus(kind, self.focused));
        let Body {
            pr,
            description,
            sections,
            layouts,
            placements,
            ..
        } = self;
        layouts.get(width, theme, || {
            let (rendered, placed) = overview(pr, description.as_ref(), sections, &focus, width, theme);
            placements.retain(|(w, _)| *w != width);
            if placements.len() == 2 {
                placements.remove(0);
            }
            placements.push((width, placed));
            rendered
        })
    }
}

/// The whole tab laid out `width` cells wide, and where its sections landed.
fn overview(
    pr: &FullPullRequest,
    description: Option<&Markdown>,
    sections: &[Section; 3],
    focus: &[(SectionFocus, bool); 3],
    width: u16,
    theme: &Theme,
) -> (Rendered, Placements) {
    let mut out = Rendered::default();
    let mut placements = Placements::default();
    if width == 0 {
        return (out, placements);
    }
    let HeaderStyles { text, link } = HeaderStyles::new(theme);
    let issue = &pr.pr.issue;

    let state = if pr.merged_at.is_some() {
        TextRun::new("Merged", accent(theme, Accent::Orchid))
    } else if issue.closed_at.is_some() {
        TextRun::new("closed", accent(theme, Accent::Red))
    } else {
        TextRun::new("Open", accent(theme, Accent::GreenYellow))
    };
    // The title as typed: Python passes it as a markup variable, so brackets in it are text.
    let title = [
        state,
        TextRun::new(" ", text),
        TextRun::new(issue.title.as_str(), text.add_modifier(Modifier::BOLD)),
        TextRun::new(" ", text),
        TextRun::link(format!("(#{})", issue.number), link, issue.html_url.as_str()),
        TextRun::new(" by ", text),
        TextRun::link(issue.user.login.as_str(), link, issue.user.html_url.as_str()),
    ];
    push_label(&mut out, &title, width);

    let bold = text.add_modifier(Modifier::BOLD);
    let separator = || TextRun::new(" • ", text);
    let summary = [
        TextRun::new(plural(pr.commits, "commit", "commits"), text),
        separator(),
        TextRun::new(plural(pr.changed_files, "file changed", "files changed"), text),
        separator(),
        TextRun::new(format!("+{}", pr.additions), accent(theme, Accent::GreenYellow)),
        separator(),
        TextRun::new(format!("-{}", pr.deletions), accent(theme, Accent::Red)),
        separator(),
        TextRun::new(format!("{}:{}", pr.head.user.login, pr.head.name), bold),
        TextRun::new(" → ", text),
        TextRun::new(format!("{}:{}", pr.base.user.login, pr.base.name), bold),
    ];
    push_label(&mut out, &summary, width);

    // Python's label starts with a newline, which draws as a blank row.
    let merged = pr
        .merged_at
        .map(|at| format!(" • Merged on {}", time::c_format(at)))
        .unwrap_or_default();
    let dates = format!("\nOpened on {}{merged}", time::c_format(issue.created_at));
    push_label(&mut out, &[TextRun::new(dates, text)], width);

    for kind in SectionKind::ALL {
        let section = &sections[kind.index()];
        if matches!(section.state, SectionState::Hidden) {
            continue;
        }
        let (section_focus, list_focused) = focus[kind.index()];
        placements[kind.index()] = Some(place_section(
            &mut out,
            kind,
            section,
            section_focus,
            list_focused,
            width,
            theme,
        ));
    }

    push_rule(&mut out, width, theme);
    if let Some(description) = description {
        append(&mut out, description.layout(width, theme));
    }
    (out, placements)
}

/// Adds section `kind` to `out`, `width` wide, and returns where it and its list items landed. The pane and the
/// maximize view both lay sections out through this.
fn place_section(
    out: &mut Rendered,
    kind: SectionKind,
    section: &Section,
    focus: SectionFocus,
    list_focused: bool,
    width: u16,
    theme: &Theme,
) -> Placed {
    let mut items = Vec::new();
    let placement = section.collapsible.layout(
        out,
        &section_title(kind, &section.state, theme),
        focus,
        width,
        theme,
        |content| {
            let rows = section_items(&section.state, theme);
            let list = section
                .list
                .layout(&rows, content.width, theme, list_focused, content.under);
            items = list.items;
            list.rendered
        },
    );
    let items = items
        .into_iter()
        .map(|rows| rows.start + placement.content.start..rows.end + placement.content.start)
        .collect();
    Placed {
        section: placement,
        items,
    }
}

/// The rows `stop` takes when it's in section `kind`, placed at `placed`: the title's row, or the list's highlighted
/// item (all of its contents without one).
fn stop_rows(stop: OverviewStop, kind: SectionKind, placed: &Placed, highlight: Option<usize>) -> Option<Range<usize>> {
    match stop {
        OverviewStop::Title(k) if k == kind => Some(placed.section.title_row..placed.section.title_row + 1),
        OverviewStop::List(k) if k == kind => match highlight.and_then(|item| placed.items.get(item)) {
            Some(rows) => Some(rows.clone()),
            None => Some(placed.section.content.clone()),
        },
        _ => None,
    }
}

/// A section's title: Python's placeholder while it loads, then what it holds or that it failed.
fn section_title(kind: SectionKind, state: &SectionState, theme: &Theme) -> Vec<TextRun> {
    let plain = |text: &str| vec![TextRun::new(text, Style::new())];
    match (kind, state) {
        (SectionKind::Reactions, SectionState::Loaded(Loaded::Reactions(summary))) => plain(&summary.title),
        (SectionKind::Checks, SectionState::Loaded(Loaded::Checks { overall, .. })) => {
            let mut title = plain("Status checks: ");
            title.push(check_state(*overall, theme));
            title
        }
        (SectionKind::Reviews, SectionState::Loaded(Loaded::Reviews(summary))) => match summary.verdict {
            ReviewVerdict::Mixed => plain("PR Reviews Summary"),
            ReviewVerdict::Approved | ReviewVerdict::ChangesRequested => {
                let state = match summary.verdict {
                    ReviewVerdict::Approved => ReviewState::Approved,
                    _ => ReviewState::ChangesRequested,
                };
                let mut title = plain("PR Reviews: ");
                title.push(review_state(state, theme));
                title
            }
        },
        (SectionKind::Reactions, SectionState::Failed) => plain("Reactions: failed to load"),
        (SectionKind::Checks, SectionState::Failed) => plain("Status checks: failed to load"),
        (SectionKind::Reviews, SectionState::Failed) => plain("PR Reviews: failed to load"),
        (SectionKind::Reactions, _) => plain("Reactions..."),
        (SectionKind::Checks, _) => plain("Status Checks: ..."),
        (SectionKind::Reviews, _) => plain("Reviews: ..."),
    }
}

/// A section's list items.
fn section_items(state: &SectionState, theme: &Theme) -> Vec<Vec<TextRun>> {
    match state {
        SectionState::Loaded(Loaded::Reactions(summary)) => summary
            .rows
            .iter()
            .map(|row| vec![TextRun::new(row.as_str(), Style::new())])
            .collect(),
        SectionState::Loaded(Loaded::Checks { rows, .. }) => rows.iter().map(|row| check_row(row, theme)).collect(),
        SectionState::Loaded(Loaded::Reviews(summary)) => summary
            .rows
            .iter()
            .map(|(login, state)| {
                vec![
                    TextRun::new(format!("{login}: "), Style::new()),
                    review_state(*state, theme),
                ]
            })
            .collect(),
        SectionState::Loading | SectionState::Failed | SectionState::Hidden => Vec::new(),
    }
}

/// `<state> <context> - <description>` for a commit status, `<state> <name>` for a check run (or a status without a
/// description), linking to its page when it has one.
fn check_row(row: &CheckRow, theme: &Theme) -> Vec<TextRun> {
    let rest = match &row.description {
        Some(description) => format!(" {} - {description}", row.name),
        None => format!(" {}", row.name),
    };
    let state = check_state(row.state, theme);
    let runs = [state, TextRun::new(rest, Style::new())];
    match &row.url {
        Some(url) => runs
            .into_iter()
            .map(|run| TextRun::link(run.text, run.style.add_modifier(Modifier::UNDERLINED), url.as_str()))
            .collect(),
        None => runs.into(),
    }
}

/// Python's `CheckStatusState.to_display`.
fn check_state(state: CheckState, theme: &Theme) -> TextRun {
    let (text, color) = match state {
        CheckState::Success => ("✓ Passed", Accent::GreenYellow),
        CheckState::Pending => ("... Pending", Accent::Yellow),
        CheckState::Failure => ("✘ Failed", Accent::Red),
        CheckState::Error => ("✘ Errored", Accent::Red),
    };
    TextRun::new(text, accent(theme, color))
}

/// Python's `ReviewState.to_display`: approved and changes requested with a mark in their color, the rest
/// title-cased.
fn review_state(state: ReviewState, theme: &Theme) -> TextRun {
    match state {
        ReviewState::Approved => TextRun::new("✓ Approved", accent(theme, Accent::GreenYellow)),
        ReviewState::ChangesRequested => TextRun::new("✘ Changes Requested", accent(theme, Accent::Red)),
        ReviewState::Commented | ReviewState::Dismissed | ReviewState::Pending => {
            TextRun::new(state.title(), Style::new())
        }
    }
}

/// `1 commit`, `3 commits`.
fn plural(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

#[cfg(test)]
mod tests;
