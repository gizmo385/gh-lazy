//! Selecting a pull request, its tabs and its Overview in the app, against the `pr-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{
    CheckRun, CombinedStatus, CommentRef, FullPullRequest, IssueComment, IssueListItem, Reactions, Repository, Review,
    ReviewState,
};
use serde_json::Value;

use crate::action::Action;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, logs};
use crate::toast::Toast;

use super::{OverviewStop, PrOverview, SectionKind};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/pr-details")
}

/// The response body of the fixture at `path` in the `pr-details` set.
fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/pr-details`, first on the set's repos page.
pub(in crate::screens::main) fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// Pull request `number` of `parity-bot/pr-details`, as the pull request endpoint returns it.
pub(in crate::screens::main) fn fixture_pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!("repos/parity-bot/pr-details/pulls/{number}/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// The repo's first issues page: PRs 5 (open), 4 (closed), 3 (merged) and 2 (draft).
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body("repos/parity-bot/pr-details/issues/GET.page-1_per_page-30_state-all.json");
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

pub(super) fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
    }
}

/// The head commit's statuses of pull request `number`.
fn commit_status(number: u64) -> CombinedStatus {
    let sha = fixture_pr(number).head.sha;
    let path = format!("repos/parity-bot/pr-details/commits/{sha}/status/GET.page-1_per_page-100.json");
    serde_json::from_value(fixture_body(&path)).unwrap()
}

/// The head commit's check runs of pull request `number`.
fn check_runs(number: u64) -> Vec<CheckRun> {
    let sha = fixture_pr(number).head.sha;
    let path = format!("repos/parity-bot/pr-details/commits/{sha}/check-runs/GET.page-1_per_page-100.json");
    serde_json::from_value(fixture_body(&path)["check_runs"].clone()).unwrap()
}

/// The submitted reviews of pull request `number`, each with its comments, as `pull_request_reviews` gives them.
pub(super) fn reviews(number: u64) -> Vec<Review> {
    let path = format!("repos/parity-bot/pr-details/pulls/{number}/reviews/GET.json");
    let all: Vec<Review> = serde_json::from_value(fixture_body(&path)).unwrap();
    all.into_iter()
        .filter(|review| review.state != ReviewState::Pending)
        .map(|mut review| {
            let path = format!(
                "repos/parity-bot/pr-details/pulls/{number}/reviews/{}/comments/GET.json",
                review.id
            );
            review.comments = serde_json::from_value(fixture_body(&path)).unwrap();
            review
        })
        .collect()
}

/// The comments on pull request `number`.
pub(super) fn comments(number: u64) -> Vec<IssueComment> {
    let path = format!("repos/parity-bot/pr-details/issues/{number}/comments/GET.json");
    serde_json::from_value(fixture_body(&path)).unwrap()
}

/// The reactions on `comment`, from its own endpoint.
pub(super) fn comment_reactions(comment: CommentRef) -> Reactions {
    let kind = match comment {
        CommentRef::Issue(_) => "issues",
        CommentRef::Review(_) => "pulls",
    };
    let path = format!(
        "repos/parity-bot/pr-details/{kind}/comments/{}/reactions/GET.page-1_per_page-100.json",
        comment.id()
    );
    Reactions::from_api(fixture_body(&path)).unwrap()
}

/// The reactions on pull request `number`.
fn reactions(number: u64) -> Reactions {
    let path = format!("repos/parity-bot/pr-details/issues/{number}/reactions/GET.json");
    Reactions::from_api(fixture_body(&path)).unwrap()
}

pub(super) const NAME: &str = "parity-bot/pr-details";

/// The fake with the `pr-details` repo, its PRs, their checks, reactions, reviews and comments, and the comments'
/// reactions.
pub(super) fn gh() -> FakeGithub {
    let mut gh = FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(NAME, 1, Ok(issues_page()))
        .with_pull_request(NAME, 2, Err(server_error()));
    for number in [5, 4, 3, 9] {
        let sha = fixture_pr(number).head.sha;
        gh = gh
            .with_pull_request(NAME, number, Ok(fixture_pr(number)))
            .with_commit_status(NAME, &sha, Ok(commit_status(number)))
            .with_check_runs(NAME, &sha, Ok(check_runs(number)))
            .with_issue_reactions(NAME, number, Ok(reactions(number)))
            .with_pull_request_reviews(NAME, number, Ok(reviews(number)))
            .with_issue_comments(NAME, number, Ok(comments(number)));
    }
    for comment in [7001, 7002, 7003]
        .map(CommentRef::Issue)
        .into_iter()
        .chain([9001, 9002, 9003].map(CommentRef::Review))
    {
        gh = gh.with_comment_reactions(NAME, comment, Ok(comment_reactions(comment)));
    }
    gh
}

/// The `pr-details` scenario's config: Issues and Workflows hidden.
fn config() -> Config {
    let mut config = Config::default();
    config.appearance.show_issues = false;
    config.appearance.show_workflows = false;
    config
}

pub(super) fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The app with `parity-bot/pr-details` selected and its PRs listed, the PR table focused on PR 5.
pub(super) fn app() -> TestApp {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app
}

/// The pull request requests among the app's effects, by number.
pub(super) fn pr_requests(app: &TestApp) -> Vec<(RequestId, u64)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequest { number, .. },
            } => Some((*id, *number)),
            _ => None,
        })
        .collect()
}

pub(super) fn cancels(app: &TestApp) -> Vec<RequestId> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) => Some(*id),
            _ => None,
        })
        .collect()
}

pub(super) fn main(app: &TestApp) -> &crate::screens::main::MainScreen {
    app.app().main_screen().unwrap()
}

pub(super) fn focus(app: &TestApp) -> FocusTarget {
    main(app).focus()
}

fn title(app: &TestApp) -> &str {
    main(app).details_title()
}

fn tabs(app: &TestApp) -> (Vec<String>, usize) {
    let main = main(app);
    (main.details_tabs().to_vec(), main.details.tabs().active())
}

fn shown_pr(app: &TestApp) -> Option<u64> {
    main(app).details_pull_request().map(FullPullRequest::number)
}

/// The row of the screen whose text contains `needle`.
pub(super) fn row_of(app: &mut TestApp, needle: &str) -> u16 {
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains(needle)).unwrap();
    u16::try_from(row).unwrap()
}

#[test]
fn enter_space_and_a_click_each_request_the_pull_request_once() {
    let mut app = app();
    app.press(&["enter"]);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [5]);

    let mut app = self::app();
    app.press(&["j", "space"]);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [4]);

    // A click on the cursor's cell (the number column of the first row): the press focuses the table, the release
    // selects. Elsewhere on the row it only moves the cursor.
    let mut app = self::app();
    let row = row_of(&mut app, "│ 5  open");
    app.click(6, row);
    assert!(pr_requests(&app).is_empty());
    app.render(160, 48);
    app.click(2, row);
    assert!(pr_requests(&app).is_empty());
    app.render(160, 48);
    app.click(2, row);
    assert_eq!(pr_requests(&app).iter().map(|&(_, n)| n).collect::<Vec<_>>(), [5]);
    let out = settle(&mut app);
    assert_eq!(
        app.gh()
            .calls()
            .into_iter()
            .filter(|call| matches!(call, GithubCall::PullRequest { .. }))
            .collect::<Vec<_>>(),
        [GithubCall::PullRequest {
            full_name: "parity-bot/pr-details".into(),
            number: 5
        }]
    );
    assert_eq!(shown_pr(&app), Some(5));
    assert!(!out.contains("ERROR"), "{out}");
}

#[test]
fn selecting_logs_and_nothing_changes_until_the_answer() {
    let mut app = app();
    let before = app.screen(160, 48);
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("INFO Selected PR: #5"), "{out}");
    assert_eq!(app.screen(160, 48), before);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
    assert_eq!(title(&app), "[5] parity-bot/pr-details Details");
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn the_answer_shows_the_three_tabs_and_focuses_the_strip() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    assert_eq!(title(&app), "[5] PR #5 Details");
    assert_eq!(
        tabs(&app),
        (vec!["Overview".into(), "Diff".into(), "Conversation".into()], 0)
    );
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    let screen = app.screen(160, 48);
    assert!(screen.contains("┌─ [5] PR #5 Details ─"), "{screen}");
    assert!(screen.contains("│ Overview  Diff  Conversation "), "{screen}");
    assert!(
        screen.contains("│Open Add a cursor to the viewer (#5) by parity-friend "),
        "{screen}"
    );
}

#[test]
fn the_title_key_follows_the_overrides() {
    let mut config = config();
    config
        .bindings
        .overrides
        .insert("main.details.focus".into(), "9".into());
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert_eq!(title(&app), "[9] PR #5 Details");
}

#[test]
fn a_failure_toasts_and_changes_nothing_else() {
    let mut app = app();
    app.press(&["j", "enter"]);
    settle(&mut app);
    assert_eq!(shown_pr(&app), Some(4));
    app.press(&["2", "j", "j", "enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR API error loading PR!"), "{out}");
    assert!(
        app.app()
            .toasts()
            .iter()
            .eq([&Toast::error("Error fetching pull request!").with_title("API Error")])
    );
    assert_eq!(shown_pr(&app), Some(4));
    assert_eq!(title(&app), "[5] PR #4 Details");
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
}

#[test]
fn a_missing_pull_request_fails_the_same_way() {
    let mut app = TestApp::with_github(
        config(),
        gh().with_pull_request("parity-bot/pr-details", 5, Err(GhError::NotInstalled)),
    );
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR API error loading PR!"), "{out}");
    assert_eq!(app.app().toasts().len(), 1);
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn a_second_selection_cancels_the_first_and_drops_its_late_answer() {
    let mut app = app();
    app.press(&["enter"]);
    let (first, _) = pr_requests(&app)[0];
    app.press(&["j", "enter"]);
    assert!(cancels(&app).contains(&first));
    // The first answer arrives anyway, as one already sent can.
    let out = logs(|| {
        app.send(Event::Github {
            id: first,
            response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
        });
    });
    assert!(out.contains("DEBUG Dropping the pull request response"), "{out}");
    assert_eq!(shown_pr(&app), None);
    settle(&mut app);
    assert_eq!(shown_pr(&app), Some(4));
    assert_eq!(title(&app), "[5] PR #4 Details");
}

#[test]
fn selecting_a_repo_drops_a_pending_pull_request() {
    let mut app = app();
    app.press(&["enter"]);
    let (pending, _) = pr_requests(&app)[0];
    app.press(&["1", "enter"]);
    assert!(cancels(&app).contains(&pending));
    app.send(Event::Github {
        id: pending,
        response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
    });
    settle(&mut app);
    assert_eq!(shown_pr(&app), None);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(title(&app), "[5] parity-bot/pr-details Details");
}

#[test]
fn selecting_a_repo_replaces_the_pull_request_tabs() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["1", "enter"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(shown_pr(&app), None);
}

#[test]
fn selecting_the_same_pull_request_again_fetches_and_rebuilds() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["right", "right"]);
    assert_eq!(tabs(&app).1, 2);
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert_eq!(pr_requests(&app).len(), 2);
    assert_eq!(tabs(&app).1, 0);
    assert_eq!(shown_pr(&app), Some(5));
}

#[test]
fn right_and_left_switch_tabs_and_wrap() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    let active = |app: &TestApp| tabs(app).1;
    app.press(&["right"]);
    assert_eq!(active(&app), 1);
    app.press(&["right"]);
    assert_eq!(active(&app), 2);
    app.press(&["right"]);
    assert_eq!(active(&app), 0);
    app.press(&["left"]);
    assert_eq!(active(&app), 2);
    app.press(&["left", "left"]);
    assert_eq!(active(&app), 0);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn with_one_tab_the_keys_do_nothing() {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["5", "right"]);
    assert_eq!(tabs(&app), (vec!["Introduction".into()], 0));
    app.press(&["1", "enter", "5", "left", "right"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
}

#[test]
fn a_click_on_a_tab_activates_it_and_focuses_the_strip() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    let screen = app.screen(160, 48);
    let row = row_of(&mut app, "Overview  Diff  Conversation");
    let line = screen.lines().nth(usize::from(row)).unwrap();
    let column = |name: &str| u16::try_from(line[..line.find(name).unwrap()].chars().count()).unwrap();
    app.click(column("Conversation") + 3, row);
    assert_eq!(tabs(&app).1, 2);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.click(column("Diff"), row);
    assert_eq!(tabs(&app).1, 1);
    // The underline row only focuses.
    app.click(column("Overview"), row + 1);
    assert_eq!(tabs(&app).1, 1);
}

#[test]
fn each_tab_keeps_its_scroll_position() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.render(100, 20);
    app.press(&["tab", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.pull_request().unwrap().overview().offset();
    assert_eq!(offset(&app), 2);
    app.press(&["5", "right", "left"]);
    app.render(100, 20);
    assert_eq!(offset(&app), 2);
}

#[test]
fn tab_from_the_strip_focuses_the_body_and_its_footer() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    let footer = |app: &mut TestApp| app.screen(160, 48).lines().last().unwrap().to_owned();
    assert!(!footer(&mut app).contains("Merge PR"));
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let footer = footer(&mut app);
    assert!(
        footer.starts_with(" ^m Merge PR  E Edit PR  R Add reaction  ^v Paste link"),
        "{footer}"
    );
    // Not built yet: consumed and logged. `R` is Add reaction's.
    let out = logs(|| {
        app.press(&["E"]);
    });
    assert!(out.contains("is not implemented yet"), "{out}");
    // Merge, edit and react belong to the Overview: the other tabs' bodies don't have them.
    app.press(&["5", "right", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let screen = app.screen(160, 48);
    assert!(!screen.lines().last().unwrap().contains("Merge PR"), "{screen}");
}

#[test]
fn the_pull_request_bindings_answer_unimplemented() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["tab"]);
    let main = app.app_mut().main_screen_mut().unwrap();
    assert!(matches!(
        main.details
            .on_binding(crate::component::Owner::PrOverview, BindingId::PullRequestEdit),
        Some(Action::Unimplemented(BindingId::PullRequestEdit))
    ));
    // Merge goes to the main screen, which holds the merges in flight.
    assert!(matches!(
        main.details
            .on_binding(crate::component::Owner::PrOverview, BindingId::PullRequestMerge),
        Some(Action::Main(crate::screens::main::MainAction::MergePullRequest))
    ));
    // Add reaction runs as a command, which makes a request.
    assert!(matches!(
        main.details
            .on_binding(crate::component::Owner::PrOverview, BindingId::ReactAdd),
        Some(Action::Main(crate::screens::main::MainAction::DetailsCommand(
            crate::component::Owner::PrOverview,
            BindingId::ReactAdd
        )))
    ));
}

#[test]
fn a_full_pull_request_shows_at_once() {
    let mut app = app();
    app.dispatch(Action::Main(crate::screens::main::MainAction::OpenPullRequest {
        target: super::PullRequestTarget::Full(Box::new(fixture_pr(9))),
        focus: false,
    }));
    assert!(pr_requests(&app).is_empty());
    assert_eq!(shown_pr(&app), Some(9));
    assert_eq!(title(&app), "[5] PR #9 Details");
    // Without `focus`, focus stays where it was.
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::PullRequests));
}

#[test]
fn pr_overview_sections_collapsed_160x48() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_overview_merged_160x48() {
    let mut app = app();
    app.press(&["j", "j", "enter"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("Opened on Sat Jan 10 14:00:00 2026 • Merged on Mon Jan 12 16:20:00 2026"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_overview_closed_100x30() {
    let mut app = app();
    app.press(&["j", "enter"]);
    settle(&mut app);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn pr_details_tabs_switched() {
    let mut app = app();
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["left"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

// The Overview's sections (#232).

/// The app on `gh` with the PR table focused on PR 5.
pub(super) fn app_on(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(config(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app
}

/// The app showing the PR `keys` select from the table, every request answered, the tab strip focused.
pub(super) fn shown(keys: &[&str]) -> TestApp {
    let mut app = app();
    app.press(keys);
    settle(&mut app);
    app
}

pub(super) fn overview(app: &TestApp) -> &PrOverview {
    main(app).details.pull_request().unwrap().overview()
}

fn stop(app: &TestApp) -> OverviewStop {
    overview(app).stop()
}

/// The Overview requests among the app's effects.
fn section_requests(app: &TestApp) -> Vec<(RequestId, GithubRequest)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .filter(|(_, request)| {
            matches!(
                request,
                GithubRequest::CommitStatus { .. }
                    | GithubRequest::CheckRuns { .. }
                    | GithubRequest::IssueReactions { .. }
            )
        })
        .collect()
}

/// Selects PR 5 and answers only its pull request request, so the sections are still loading.
pub(super) fn loading_sections() -> TestApp {
    let mut app = app();
    app.press(&["enter"]);
    let (id, _) = pr_requests(&app)[0];
    app.send(Event::Github {
        id,
        response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
    });
    app
}

#[test]
fn both_sections_start_loading_with_the_pull_request() {
    let mut app = loading_sections();
    let sha = fixture_pr(5).head.sha;
    let repo = Box::new(fixture_repo());
    let requests: Vec<GithubRequest> = section_requests(&app).into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        requests,
        [
            GithubRequest::CommitStatus {
                repo: repo.clone(),
                sha: sha.clone()
            },
            GithubRequest::CheckRuns {
                repo: repo.clone(),
                sha
            },
            GithubRequest::IssueReactions { repo, number: 5 },
        ]
    );
    // The spinner over each section: reactions, checks and reviews.
    let screen = app.screen(160, 48);
    assert_eq!(screen.matches("Loading...").count(), 3, "{screen}");
    // Loading sections aren't focus stops.
    app.press(&["tab", "tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn pr_overview_sections_loading() {
    let mut app = loading_sections();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn the_loading_spinner_animates_until_the_sections_load() {
    let mut app = loading_sections();
    let now = crate::clock::Clock::now(app.clock());
    // The test app doesn't animate.
    assert_eq!(main(&app).next_deadline(now), None);
    let mut pane = super::super::details::DetailsPane::default();
    app.with_cx(|cx| pane.show_pull_request(fixture_pr(5), cx));
    assert_eq!(
        pane.next_deadline(now),
        Some(now + crate::components::loading::FRAME_INTERVAL)
    );
}

#[test]
fn sections_with_nothing_are_not_shown() {
    let mut app = shown(&["j", "enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Drop the legacy renderer (#4)"), "{screen}");
    for absent in ["Status checks", "Reactions", "👍", "Reviews"] {
        assert!(!screen.contains(absent), "{absent}: {screen}");
    }
    assert!(!screen.contains("Loading..."), "{screen}");
    // The body is the only stop.
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, OverviewStop::Body)
    );
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn loaded_sections_show_their_titles() {
    let mut app = shown(&["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│  ▶ 👍  2 | 🎉  1 | 👀  4 "), "{screen}");
    assert!(screen.contains("│  ▶ Status checks: ✘ Failed "), "{screen}");
    let mut merged = shown(&["j", "j", "enter"]);
    let screen = merged.screen(160, 48);
    assert!(screen.contains("▶ 🎉  2 "), "{screen}");
    assert!(screen.contains("▶ Status checks: ✓ Passed "), "{screen}");
}

#[test]
fn a_failed_checks_request_says_so_and_leaves_the_rest() {
    let sha = fixture_pr(5).head.sha;
    let mut app = app_on(gh().with_check_runs(NAME, &sha, Err(server_error())));
    app.press(&["enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR Error loading status checks"), "{out}");
    let screen = app.screen(160, 48);
    assert!(screen.contains("▶ Status checks: failed to load "), "{screen}");
    assert!(screen.contains("▶ 👍  2 | 🎉  1 | 👀  4"), "{screen}");
    assert!(app.app().toasts().is_empty());
    insta::assert_snapshot!("pr_overview_checks_failed", screen);

    // The other request failing does the same.
    let mut app = app_on(gh().with_commit_status(NAME, &sha, Err(server_error())));
    app.press(&["enter"]);
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR Error loading status checks").count(), 1, "{out}");
    assert!(app.screen(160, 48).contains("▶ Status checks: failed to load "));
    // The failed section's title is a stop; its empty list isn't, even open: the reviews title is next.
    app.press(&["tab", "tab", "tab"]);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Checks));
    app.press(&["enter", "tab"]);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Reviews));
}

#[test]
fn a_failed_reactions_request_says_so_and_leaves_the_rest() {
    let mut app = app_on(gh().with_issue_reactions(NAME, 5, Err(server_error())));
    app.press(&["enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR Error loading reactions data: "), "{out}");
    let screen = app.screen(160, 48);
    assert!(screen.contains("▶ Reactions: failed to load "), "{screen}");
    assert!(screen.contains("▶ Status checks: ✘ Failed "), "{screen}");
}

#[test]
fn enter_on_a_title_toggles_its_section() {
    let mut app = shown(&["enter"]);
    app.press(&["tab", "tab"]);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Reactions));
    let footer = app.screen(160, 48).lines().last().unwrap().to_owned();
    assert!(
        footer.starts_with(" ^m Merge PR  E Edit PR  R Add reaction"),
        "{footer}"
    );
    app.press(&["enter"]);
    assert!(overview(&app).is_open(SectionKind::Reactions));
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Reactions));
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("👀 : parity-bot, parity-reviewer, parity-carol, and 1 more"),
        "{screen}"
    );
    insta::assert_snapshot!("pr_overview_reactions_open", screen);
    app.press(&["enter"]);
    assert!(!overview(&app).is_open(SectionKind::Reactions));
}

#[test]
fn pr_overview_checks_open() {
    let mut app = shown(&["enter"]);
    app.press(&["tab", "tab", "tab", "enter"]);
    assert!(overview(&app).is_open(SectionKind::Checks));
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn a_click_on_a_title_focuses_and_toggles_it() {
    let mut app = shown(&["enter"]);
    let row = row_of(&mut app, "▶ Status checks");
    let screen = app.screen(160, 48);
    let line = screen.lines().nth(usize::from(row)).unwrap();
    let column = u16::try_from(line[..line.find('▶').unwrap()].chars().count()).unwrap();
    app.click(column, row);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Checks));
    assert!(overview(&app).is_open(SectionKind::Checks));
    // A click on a list item focuses the list and highlights it.
    let row = row_of(&mut app, "... Pending deploy");
    app.click(column + 4, row);
    assert_eq!(stop(&app), OverviewStop::List(SectionKind::Checks));
    assert_eq!(overview(&app).highlight(SectionKind::Checks), Some(2));
    // Past the title's end on its row, and elsewhere in the body, the body takes focus.
    let row = row_of(&mut app, "▼ Status checks");
    app.click(column + 60, row);
    assert_eq!(stop(&app), OverviewStop::Body);
    assert!(overview(&app).is_open(SectionKind::Checks));
    app.click(column, row);
    assert!(!overview(&app).is_open(SectionKind::Checks));
}

#[test]
fn the_focus_ring_walks_the_titles_and_open_lists() {
    let mut app = shown(&["enter"]);
    app.press(&["tab"]);
    assert_eq!(stop(&app), OverviewStop::Body);
    app.press(&["tab", "enter", "tab"]);
    assert_eq!(stop(&app), OverviewStop::List(SectionKind::Reactions));
    app.press(&["tab"]);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Checks));
    app.press(&["tab", "enter", "tab"]);
    assert_eq!(stop(&app), OverviewStop::List(SectionKind::Reviews));
    // The reviews list is the last stop: focus moves on, round to the repos table as the command log is hidden.
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
    // And back, from the last stop.
    app.press(&["shift+tab"]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, OverviewStop::List(SectionKind::Reviews))
    );
    app.press(&[
        "shift+tab",
        "shift+tab",
        "shift+tab",
        "shift+tab",
        "shift+tab",
        "shift+tab",
    ]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn up_and_down_move_the_highlight_in_a_focused_list() {
    let mut app = shown(&["enter"]);
    app.press(&["tab", "tab", "enter", "tab"]);
    assert_eq!(overview(&app).highlight(SectionKind::Reactions), None);
    app.press(&["down"]);
    assert_eq!(overview(&app).highlight(SectionKind::Reactions), Some(0));
    app.press(&["down", "down", "down"]);
    assert_eq!(overview(&app).highlight(SectionKind::Reactions), Some(2));
    app.press(&["up"]);
    assert_eq!(overview(&app).highlight(SectionKind::Reactions), Some(1));
    // `enter` in the list does nothing.
    app.press(&["enter"]);
    assert!(overview(&app).is_open(SectionKind::Reactions));
    assert_eq!(stop(&app), OverviewStop::List(SectionKind::Reactions));
}

#[test]
fn a_section_stays_open_across_a_tab_switch() {
    let mut app = shown(&["enter"]);
    app.press(&["tab", "tab", "enter", "5", "right", "left"]);
    assert_eq!(tabs(&app).1, 0);
    assert!(overview(&app).is_open(SectionKind::Reactions));
    assert!(app.screen(160, 48).contains("▼ 👍  2 | 🎉  1 | 👀  4"));
}

#[test]
fn the_focused_stop_scrolls_into_view() {
    let mut app = shown(&["enter"]);
    app.render(100, 20);
    app.press(&["tab", "tab", "enter", "tab", "tab"]);
    assert_eq!(stop(&app), OverviewStop::Title(SectionKind::Checks));
    app.render(100, 20);
    let offset = overview(&app).offset();
    assert!(offset > 0, "{offset}");
    let screen = app.screen(100, 20);
    assert!(screen.contains("▶ Status checks"), "{screen}");
}

#[test]
fn replacing_the_pull_request_cancels_its_section_requests() {
    let mut app = loading_sections();
    let ids: Vec<RequestId> = section_requests(&app).into_iter().map(|(id, _)| id).collect();
    app.dispatch(Action::Main(crate::screens::main::MainAction::OpenPullRequest {
        target: super::PullRequestTarget::Full(Box::new(fixture_pr(4))),
        focus: true,
    }));
    assert_eq!(shown_pr(&app), Some(4));
    for id in &ids {
        assert!(cancels(&app).contains(id), "{id}");
    }
    // A late answer for the old pull request is dropped.
    let out = logs(|| {
        app.send(Event::Github {
            id: ids[2],
            response: GithubResponse::IssueReactions(Ok(reactions(5))),
        });
    });
    assert!(out.contains("Dropping the Overview section response"), "{out}");
    assert!(!app.screen(160, 48).contains("👍"));
}

// Add reaction.

/// parity-bot, who gave PR 5 a 👍, a 🎉 and a 👀, as `/user` returns them (with a name, unlike the reactions' users).
fn parity_bot() -> lazy_github_core::models::User {
    lazy_github_core::models::User {
        login: "parity-bot".into(),
        id: 1_000_001,
        name: Some("Parity Bot".into()),
        html_url: "https://github.com/parity-bot".into(),
    }
}

/// PR 5's Overview open with focus in its content, against `gh` with parity-bot logged in.
fn overview_focused(gh: FakeGithub) -> TestApp {
    let mut app = TestApp::with_github(config(), gh.with_user(parity_bot()));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app
}

fn modals(app: &TestApp) -> Vec<crate::modal::ModalKind> {
    app.app().modals()
}

/// The reactions posted, in order.
fn posted(app: &TestApp) -> Vec<lazy_github_core::models::ReactionKind> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::AddIssueReaction {
                full_name,
                number,
                kind,
            } => {
                assert_eq!((full_name.as_str(), number), (NAME, 5));
                Some(kind)
            }
            _ => None,
        })
        .collect()
}

fn reactions_title(app: &mut TestApp) -> String {
    let screen = app.screen(160, 48);
    let line = screen.lines().find(|line| line.contains("▶ 👍")).unwrap_or_default();
    let title = line.split("││").nth(1).unwrap_or_default().trim_matches(['│', ' ']);
    // A wide emoji leaves a blank in its second cell.
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn r_asks_for_the_user_and_lists_the_reactions_they_havent_given() {
    use lazy_github_core::models::ReactionKind::{Confused, Heart, Laugh, Rocket, ThumbsDown};

    let mut app = overview_focused(gh());
    app.press(&["R"]);
    assert!(modals(&app).is_empty(), "the dialog waits for the user");
    settle(&mut app);
    assert_eq!(modals(&app), [crate::modal::ModalKind::AddReaction]);
    let screen = app.screen(160, 48);
    for kind in [ThumbsDown, Laugh, Confused, Heart, Rocket] {
        assert!(screen.contains(&format!("▐X▌ {}", kind.emoji())), "{screen}");
    }
    for emoji in ["👍", "🎉", "👀"] {
        assert!(!screen.contains(&format!("▐X▌ {emoji}")), "{screen}");
    }
    assert_eq!(screen.lines().last().unwrap().trim_end(), " q Close");
}

#[test]
fn every_reaction_is_listed_when_the_reactions_didnt_load() {
    let gh = gh().with_issue_reactions(NAME, 5, Err(server_error()));
    let mut app = overview_focused(gh);
    app.press(&["R"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    for kind in lazy_github_core::models::ReactionKind::ALL {
        assert!(screen.contains(&format!("▐X▌ {}", kind.emoji())), "{screen}");
    }
}

#[test]
fn submit_posts_in_the_order_checked_and_shows_new_kinds_with_one() {
    use lazy_github_core::models::ReactionKind::{Heart, Laugh, Rocket, ThumbsDown};

    let gh = gh()
        .with_added_reaction(Laugh, Ok(true))
        .with_added_reaction(Rocket, Ok(true))
        .with_added_reaction(ThumbsDown, Ok(true))
        .with_added_reaction(Heart, Ok(false));
    let mut app = overview_focused(gh);
    app.press(&["R"]);
    settle(&mut app);
    // 🚀, then 👎 checked, unchecked and checked again after ❤️ and 😄.
    app.press(&[
        "end", "space", "home", "space", "space", "up", "up", "space", "home", "down", "space",
    ]);
    app.press(&["home", "space", "tab", "enter"]);
    assert!(modals(&app).is_empty());
    let out = settle(&mut app);
    assert!(!out.contains("ERROR"), "{out}");
    assert_eq!(posted(&app), [Rocket, Heart, Laugh, ThumbsDown]);
    // ❤️ was already there (a 200): nothing changes for it. The new kinds show with 1 (B66's recommendation).
    assert_eq!(reactions_title(&mut app), "▶ 👍 2 | 🎉 1 | 👀 4 | 🚀 1 | 😄 1 | 👎 1");
}

#[test]
fn a_failed_post_is_logged_and_the_rest_go_on() {
    use lazy_github_core::models::ReactionKind::{Laugh, Rocket};

    let gh = gh().with_added_reaction(Rocket, Ok(true));
    let mut app = overview_focused(gh);
    app.press(&["R"]);
    settle(&mut app);
    // 😄 isn't canned: the fake answers it with a 422.
    app.press(&["down", "space", "end", "space", "tab", "enter"]);
    let out = settle(&mut app);
    assert!(out.contains("ERROR Error adding reaction laugh - "), "{out}");
    assert_eq!(posted(&app), [Laugh, Rocket]);
    assert_eq!(reactions_title(&mut app), "▶ 👍 2 | 🎉 1 | 👀 4 | 🚀 1");
    assert!(app.app().toasts().is_empty());
}

#[test]
fn cancel_and_q_post_nothing() {
    for keys in [&["space", "tab", "tab", "enter"][..], &["space", "q"]] {
        let mut app = overview_focused(gh());
        app.press(&["R"]);
        settle(&mut app);
        app.press(keys);
        settle(&mut app);
        assert!(modals(&app).is_empty(), "{keys:?}");
        assert!(posted(&app).is_empty(), "{keys:?}");
        assert_eq!(reactions_title(&mut app), "▶ 👍 2 | 🎉 1 | 👀 4", "{keys:?}");
        assert_eq!(focus(&app), FocusTarget::DetailsContent, "{keys:?}");
    }
}

#[test]
fn without_the_user_no_dialog_opens() {
    let mut app = TestApp::with_github(config(), gh().with_user_error(server_error()));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    app.press(&["tab", "R"]);
    let out = settle(&mut app);
    assert!(
        out.contains("ERROR Error fetching the current user for a reaction"),
        "{out}"
    );
    assert!(modals(&app).is_empty());
}

// Merge (#262).

/// The Overview of the PR `keys` select from the table, with focus in its content, on `config` and `gh`.
fn merge_ready(config: Config, gh: FakeGithub, keys: &[&str]) -> TestApp {
    let mut app = TestApp::with_github(config, gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app.press(keys);
    settle(&mut app);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app
}

/// The merges sent, by number, with what each sent.
fn merges(app: &TestApp) -> Vec<(u64, lazy_github_core::github::MergeRequest)> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::MergePullRequest {
                full_name,
                number,
                request,
            } => {
                assert_eq!(full_name, NAME);
                Some((number, request))
            }
            _ => None,
        })
        .collect()
}

/// The pull request fetches made, by number.
fn fetched(app: &TestApp) -> Vec<u64> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::PullRequest { number, .. } => Some(number),
            _ => None,
        })
        .collect()
}

fn merge_result(merged: bool) -> lazy_github_core::github::MergeResult {
    lazy_github_core::github::MergeResult {
        merged,
        message: "Pull Request successfully merged".into(),
        sha: merged.then(|| "5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed".into()),
    }
}

fn toasts(app: &TestApp) -> Vec<Toast> {
    app.app().toasts().iter().cloned().collect()
}

#[test]
fn the_overview_footer_starts_with_merge() {
    let mut app = merge_ready(config(), gh(), &["enter"]);
    let screen = app.screen(160, 48);
    assert!(
        screen
            .lines()
            .last()
            .unwrap()
            .starts_with(" ^m Merge PR  E Edit PR  R Add reaction"),
        "{screen}"
    );
}

#[test]
fn an_already_merged_pull_request_warns_and_sends_nothing() {
    let mut app = merge_ready(config(), gh(), &["j", "j", "enter"]);
    app.press(&["ctrl+m"]);
    settle(&mut app);
    assert!(merges(&app).is_empty());
    assert_eq!(
        toasts(&app),
        [Toast::warning("PR has already been merged!").with_title("Already Merged")]
    );
}

#[test]
fn a_merge_toasts_and_shows_the_pull_request_fetched_again() {
    let gh = gh().with_merge(5, Ok(merge_result(true)));
    let mut app = merge_ready(config(), gh, &["enter"]);
    assert_eq!(fetched(&app), [5]);
    // Once merged, GitHub answers with the merged pull request.
    let mut merged = fixture_pr(5);
    merged.merged_at = fixture_pr(3).merged_at;
    let _ = app.gh().clone().with_pull_request(NAME, 5, Ok(merged));
    app.press(&["ctrl+m"]);
    let out = settle(&mut app);
    assert!(out.contains("INFO Merged PR 5 in repo parity-bot/pr-details"), "{out}");
    assert_eq!(
        merges(&app),
        [(
            5,
            lazy_github_core::github::MergeRequest {
                merge_method: lazy_github_core::config::MergeMethod::Squash,
                sha: fixture_pr(5).head.sha,
            }
        )]
    );
    assert_eq!(
        toasts(&app),
        [
            Toast::info("Pull request 5 merged. Note some cached information on the UI may be out of date.")
                .with_title("PR Merged")
        ]
    );
    // Fetched again and shown as a fresh selection: the tabs rebuilt and the strip focused.
    assert_eq!(fetched(&app), [5, 5]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    assert_eq!(
        tabs(&app),
        (vec!["Overview".into(), "Diff".into(), "Conversation".into()], 0)
    );
    assert!(main(&app).details_pull_request().unwrap().merged_at.is_some());
    assert!(app.screen(160, 48).contains("Merged Add a cursor to the viewer (#5)"));
}

#[test]
fn a_failed_refetch_toasts_as_a_failed_selection() {
    let gh = gh().with_merge(5, Ok(merge_result(true)));
    let mut app = merge_ready(config(), gh, &["enter"]);
    let _ = app.gh().clone().with_pull_request(NAME, 5, Err(server_error()));
    app.press(&["ctrl+m"]);
    settle(&mut app);
    assert_eq!(fetched(&app), [5, 5]);
    let toasts = toasts(&app);
    assert_eq!(toasts.len(), 2, "{toasts:?}");
    assert_eq!(
        toasts[1],
        Toast::error("Error fetching pull request!").with_title("API Error")
    );
    // The pull request still shows as it was.
    assert_eq!(shown_pr(&app), Some(5));
    assert!(main(&app).details_pull_request().unwrap().merged_at.is_none());
}

#[test]
fn a_refused_or_unmerged_merge_toasts_and_changes_nothing() {
    for result in [Ok(merge_result(false)), Err(server_error())] {
        let failed = result.is_err();
        let gh = gh().with_merge(4, result);
        let mut app = merge_ready(config(), gh, &["j", "enter"]);
        app.press(&["ctrl+m"]);
        let out = settle(&mut app);
        let line = if failed {
            "ERROR Failed to merge PR 4"
        } else {
            "WARN Failed to merge PR 4"
        };
        assert!(out.contains(line), "{out}");
        assert_eq!(merges(&app).len(), 1);
        assert_eq!(
            toasts(&app),
            [Toast::error("Pull request 4 could not be merged").with_title("Error Merging PR")]
        );
        assert_eq!(fetched(&app), [4], "no refetch");
        assert_eq!(focus(&app), FocusTarget::DetailsContent);
    }
}

#[test]
fn the_merge_method_follows_the_config() {
    use lazy_github_core::config::MergeMethod;

    for method in [MergeMethod::Merge, MergeMethod::Rebase] {
        let mut config = config();
        config.pull_requests.preferred_merge_method = method;
        let mut app = merge_ready(config, gh().with_merge(5, Ok(merge_result(true))), &["enter"]);
        app.press(&["ctrl+m"]);
        settle(&mut app);
        assert_eq!(merges(&app)[0].1.merge_method, method);
    }
}

#[test]
fn enter_in_the_overview_never_merges() {
    let mut app = merge_ready(config(), gh().with_merge(5, Ok(merge_result(true))), &["enter"]);
    app.press(&["enter"]);
    settle(&mut app);
    // Down onto a list and enter again.
    app.press(&["tab", "enter"]);
    settle(&mut app);
    assert!(merges(&app).is_empty());
    assert!(app.app().toasts().is_empty());
}
