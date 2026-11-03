//! The Search modal: the prefill and the restore rules, the scopes on offer, submitting, the status line's states,
//! paging to the cap, stale answers, closing, opening a hit, and its drawing. Against the `search`, `pr-details` and
//! `issue-details` parity fixtures.

use std::path::Path;

use indexmap::IndexMap;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{FullPullRequest, Repository};
use rstest::rstest;
use serde_json::{Value, json};

use super::*;
use crate::event::Event;
use crate::screens::main::MainAction;
use crate::test_util::{LogCapture, TestApp};
use crate::toast::Severity;

/// The response body of the parity fixture at `path`.
fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// A `search` fixture page as the client decodes it.
fn search_page(file: &str) -> SearchPage {
    let body = fixture_body(&format!("search/search/issues/{file}"));
    page_of(&body)
}

fn page_of(body: &Value) -> SearchPage {
    SearchPage {
        total_count: body["total_count"].as_u64().unwrap(),
        incomplete_results: body["incomplete_results"].as_bool().unwrap(),
        items: body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| SearchHit::from_api_item(item.clone()).unwrap())
            .collect(),
    }
}

/// `repo:parity-bot/pr-details (is:pr is:open)`: PRs 9, 5 and 2.
fn pr_details_open() -> SearchPage {
    search_page("GET.advanced_search-true_page-1_per_page-50_q-repo-parity-bot-pr-details--is-pr-is-o-e84c.json")
}

/// `is:pr author:@me` page 1: 50 of 1,234.
fn mine_first() -> SearchPage {
    search_page("GET.advanced_search-true_page-1_per_page-50_q-is-pr-author--me.json")
}

/// A hit in `repo`, numbered `number`.
fn hit(repo: &str, number: u64, title: &str) -> SearchHit {
    SearchHit::from_api_item(json!({
        "repository_url": format!("https://api.github.com/repos/{repo}"),
        "number": number,
        "title": title,
        "state": "open",
        "user": {"login": "parity-bot"},
        "updated_at": "2026-10-28T23:00:00Z",
        "html_url": format!("https://github.com/{repo}/issues/{number}"),
    }))
    .unwrap()
}

/// A page of `count` hits in `parity-bot/pr-details` numbered from `from` down, of `total`.
fn page(from: u64, count: u64, total: u64) -> SearchPage {
    SearchPage {
        total_count: total,
        incomplete_results: false,
        items: (0..count)
            .map(|i| hit("parity-bot/pr-details", from - i, &format!("Hit {}", from - i)))
            .collect(),
    }
}

/// `parity-bot/pr-details`, as its repos page lists it.
fn pr_details_repo() -> Repository {
    let page = fixture_body("pr-details/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

fn pr(number: u64) -> FullPullRequest {
    let body = fixture_body(&format!(
        "pr-details/repos/parity-bot/pr-details/pulls/{number}/GET.json"
    ));
    FullPullRequest::from_api(body, &pr_details_repo()).unwrap()
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

/// An app with `parity-bot/pr-details` selected, answering from `gh`.
async fn selected(config: Config, gh: FakeGithub) -> TestApp {
    let gh = gh.with_repositories(vec![pr_details_repo()]);
    let mut app = TestApp::with_github(config, gh);
    app.dispatch(Action::SelectRepo(Box::new(pr_details_repo())));
    app.settle().await;
    app
}

/// The modal opened from `origin`, drawn once so the table knows its height.
fn open(app: &mut TestApp, origin: SearchOrigin) {
    app.dispatch(Action::Main(MainAction::OpenSearch(origin)));
    assert_eq!(app.app().modals(), [ModalKind::Search]);
    app.render(160, 48);
}

fn dialog(app: &TestApp) -> &SearchDialog {
    app.app()
        .top_dialog::<SearchDialog>()
        .expect("the Search modal is open")
}

/// Presses `keys`, settles, and draws.
async fn press(app: &mut TestApp, keys: &[&str]) {
    app.press(keys);
    app.settle().await;
    app.render(160, 48);
}

/// Moves focus back to the input with `shift+tab`.
fn to_input(app: &mut TestApp) {
    for _ in 0..4 {
        if dialog(app).focus == Focus::Input {
            return;
        }
        app.press(&["shift+tab"]);
    }
    assert_eq!(dialog(app).focus, Focus::Input);
}

/// Replaces the query text with `text`.
fn retype(app: &mut TestApp, text: &str) {
    app.press(&["ctrl+u"]);
    app.send(Event::Paste(text.to_owned()));
}

/// The searches sent so far, as their `q`.
fn searches(app: &TestApp) -> Vec<(String, u32)> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::SearchIssues(query) => Some((query.q().unwrap(), query.page)),
            _ => None,
        })
        .collect()
}

fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

fn keys(app: &TestApp) -> Vec<(String, u64)> {
    dialog(app).hit_keys()
}

fn pr_keys(numbers: &[u64]) -> Vec<(String, u64)> {
    numbers
        .iter()
        .map(|&number| ("parity-bot/pr-details".to_owned(), number))
        .collect()
}

// Opening: the prefill and the restore rules.

#[rstest]
#[case::pull_requests(SearchOrigin::PullRequests, "is:pr is:open ")]
#[case::issues(SearchOrigin::Issues, "is:issue is:open ")]
#[case::palette(SearchOrigin::Palette, "")]
#[tokio::test]
async fn the_first_open_takes_the_origins_text(#[case] origin: SearchOrigin, #[case] text: &str) {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, origin);
    let dialog = dialog(&app);
    assert_eq!(dialog.text(), text);
    assert_eq!(dialog.session.input.cursor(), text.chars().count());
    assert_eq!(dialog.scope(), Some(ScopeChoice::Repo));
    assert_eq!(dialog.status_line(), "Type a query and press enter");
    assert!(!dialog.table_focused());
    // Nothing is searched until enter.
    assert!(searches(&app).is_empty());
}

#[tokio::test]
async fn later_opens_restore_the_session_and_the_prefill_waits_for_an_empty_query() {
    let mut app = selected(
        Config::default(),
        FakeGithub::new().with_search_page(Ok(pr_details_open())),
    )
    .await;
    // From the palette the query is empty, so the next open from the PR table still takes its text.
    open(&mut app, SearchOrigin::Palette);
    press(&mut app, &["escape"]).await;
    open(&mut app, SearchOrigin::PullRequests);
    assert_eq!(dialog(&app).text(), "is:pr is:open ");
    press(&mut app, &["enter"]).await;
    press(&mut app, &["j"]).await;
    press(&mut app, &["escape"]).await;
    assert!(app.app().modals().is_empty());
    // From the Issues table the text, results, cursor and focus come back as they were.
    open(&mut app, SearchOrigin::Issues);
    let dialog = dialog(&app);
    assert_eq!(dialog.text(), "is:pr is:open ");
    assert_eq!(dialog.hit_keys(), pr_keys(&[9, 5, 2]));
    assert_eq!(dialog.session.cursor(), 1);
    assert!(dialog.table_focused());
    assert_eq!(dialog.status_line(), "3 results");
    // Restoring searched nothing.
    assert_eq!(searches(&app).len(), 1);
}

// Scopes.

#[tokio::test]
async fn with_no_repo_this_repo_and_this_owner_are_not_offered() {
    let mut app = TestApp::with_github(Config::default(), FakeGithub::new());
    app.settle().await;
    open(&mut app, SearchOrigin::Palette);
    let dialog = dialog(&app);
    // No favorites or tracked repos either.
    assert_eq!(dialog.scope_labels(), ["All of GitHub"]);
    assert_eq!(dialog.scope(), Some(ScopeChoice::All));
}

#[tokio::test]
async fn with_a_repo_every_scope_is_offered() {
    let mut config = Config::default();
    config.repositories.favorites = vec!["parity-bot/dotfiles".into()];
    let mut app = selected(config, FakeGithub::new()).await;
    open(&mut app, SearchOrigin::Palette);
    assert_eq!(
        dialog(&app).scope_labels(),
        [
            "This repo (parity-bot/pr-details)",
            "This owner (parity-bot)",
            "Favorites and tracked repos",
            "All of GitHub",
        ]
    );
}

#[tokio::test]
async fn favorites_and_tracked_repos_are_deduplicated_and_cut_to_twenty() {
    let mut config = Config::default();
    // 25 distinct repos: five favorites, then 21 tracked of which one repeats a favorite in another case.
    config.repositories.favorites = (0..5).map(|i| format!("parity-bot/fav-{i}")).collect();
    config.repositories.additional_repos_to_track = std::iter::once("Parity-Bot/Fav-0".to_owned())
        .chain((0..20).map(|i| format!("parity-bot/tracked-{i}")))
        .collect();
    let mut app = selected(config, FakeGithub::new().with_search_page(Ok(page(9, 1, 1)))).await;
    open(&mut app, SearchOrigin::Palette);
    // The third option, through the keys: open, down twice, pick.
    press(&mut app, &["tab", "enter", "down", "down", "enter"]).await;
    assert_eq!(dialog(&app).scope(), Some(ScopeChoice::Tracked));
    assert_eq!(
        dialog(&app).status_line(),
        "Type a query and press enter · first 20 of 25 repos"
    );
    // Nothing was submitted, so the pick sent nothing; enter in the input sends the first 20.
    assert!(searches(&app).is_empty());
    press(&mut app, &["shift+tab"]).await;
    retype(&mut app, "bug");
    press(&mut app, &["enter"]).await;
    let sent = app
        .gh()
        .calls()
        .into_iter()
        .find_map(|call| match call {
            GithubCall::SearchIssues(query) => Some(query.scope),
            _ => None,
        })
        .unwrap();
    let SearchScope::Repos(repos) = sent else {
        panic!("{sent:?}")
    };
    assert_eq!(repos.len(), 20);
    assert_eq!(
        repos[..2],
        ["parity-bot/fav-0".to_owned(), "parity-bot/fav-1".to_owned()]
    );
    assert_eq!(repos[19], "parity-bot/tracked-14");
    assert!(dialog(&app).status_line().ends_with(" · first 20 of 25 repos"));
}

#[tokio::test]
async fn a_scope_change_searches_again_only_after_a_submit() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_search_page(Ok(mine_first()));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    // Before a submit: picking "This owner" sends nothing.
    press(&mut app, &["tab", "enter", "down", "enter"]).await;
    assert_eq!(dialog(&app).scope(), Some(ScopeChoice::Owner));
    assert!(searches(&app).is_empty());
    // Submit, then pick "All of GitHub": it searches again at once.
    press(&mut app, &["shift+tab", "enter"]).await;
    assert_eq!(searches(&app), [("user:parity-bot (is:pr is:open)".to_owned(), 1)]);
    press(&mut app, &["shift+tab", "enter", "down", "enter"]).await;
    assert_eq!(dialog(&app).scope(), Some(ScopeChoice::All));
    assert_eq!(searches(&app)[1], ("is:pr is:open".to_owned(), 1));
    // A scope that can span repos shows the Repo column.
    assert!(dialog(&app).session.columns.repo);
}

#[tokio::test]
async fn a_scope_in_the_query_disables_the_select_and_takes_it_out_of_the_ring() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, SearchOrigin::PullRequests);
    retype(&mut app, "is:pr repo:octocat/hello");
    app.render(160, 48);
    let dialog_ref = dialog(&app);
    assert!(dialog_ref.scope_disabled());
    assert_eq!(
        dialog_ref.status_line(),
        "Type a query and press enter · Scope from the query"
    );
    // tab goes from the input straight to the table.
    press(&mut app, &["tab"]).await;
    assert!(dialog(&app).table_focused());
    press(&mut app, &["tab"]).await;
    assert_eq!(dialog(&app).focus, Focus::Input);
}

// Submitting and the status line.

#[tokio::test]
async fn enter_searches_and_moves_focus_to_the_table_only_with_hits() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_search_page(Ok(page(1, 0, 0)));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    assert_eq!(
        searches(&app),
        [("repo:parity-bot/pr-details (is:pr is:open)".to_owned(), 1)]
    );
    let found = dialog(&app);
    assert_eq!(found.hit_keys(), pr_keys(&[9, 5, 2]));
    assert_eq!(found.status_line(), "3 results");
    assert!(found.table_focused());
    assert_eq!(found.session.cursor(), 0);
    // "This repo": no Repo column.
    assert!(!found.session.columns.repo);
    // No hits: focus stays in the input.
    to_input(&mut app);
    press(&mut app, &["enter"]).await;
    let none = dialog(&app);
    assert_eq!(none.status_line(), "No issues or pull requests match");
    assert!(none.hit_keys().is_empty());
    assert_eq!(none.focus, Focus::Input);
}

#[test]
fn the_status_line_words_each_state() {
    let results = |total, incomplete, more_failed| {
        status_text(&Status::Results {
            total,
            incomplete,
            more_failed,
        })
        .0
    };
    assert_eq!(
        status_text(&Status::Prompt),
        ("Type a query and press enter".into(), false)
    );
    assert_eq!(status_text(&Status::Searching), ("Searching…".into(), false));
    assert_eq!(results(71, false, false), "71 results");
    assert_eq!(results(1, false, false), "1 result");
    assert_eq!(results(1000, false, false), "1,000 results");
    assert_eq!(
        results(885_203, false, false),
        "885,203 results · GitHub returns the first 1,000"
    );
    assert_eq!(
        results(12, true, false),
        "12 results · GitHub timed out, results may be incomplete"
    );
    assert_eq!(
        results(120, false, true),
        "120 results · loading more failed, move down to retry"
    );
    assert_eq!(
        status_text(&Status::NoHits),
        ("No issues or pull requests match".into(), false)
    );
    assert_eq!(status_text(&Status::Error("Nope".into())), ("Nope".into(), true));
}

#[tokio::test]
async fn an_invalid_query_shows_githubs_messages_and_clears_the_results() {
    let invalid = SearchError::Invalid {
        messages: vec!["The listed users and repositories cannot be searched.".into()],
    };
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_search_page(Err(invalid));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    to_input(&mut app);
    press(&mut app, &["enter"]).await;
    let dialog = dialog(&app);
    assert_eq!(
        dialog.status(),
        ("The listed users and repositories cannot be searched.".to_owned(), true)
    );
    assert!(dialog.hit_keys().is_empty());
}

#[tokio::test]
async fn any_other_failure_keeps_the_results_and_logs() {
    let logs = LogCapture::start();
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_search_page(Err(SearchError::Gh(server_error())));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    to_input(&mut app);
    press(&mut app, &["enter"]).await;
    let dialog = dialog(&app);
    let (line, error) = dialog.status();
    assert!(line.starts_with("Search failed: "), "{line}");
    assert!(error);
    assert_eq!(dialog.hit_keys(), pr_keys(&[9, 5, 2]));
    assert!(!dialog.session.table.is_loading());
    assert!(logs.text().contains("ERROR Search failed"), "{}", logs.text());
}

#[tokio::test]
async fn a_search_rate_limit_says_when_to_try_again_and_toasts() {
    // The fake clock starts at 2026-10-25 12:00 UTC; the limit lifts five minutes later.
    let limited = GhError::Http {
        status: 403,
        message: "API rate limit exceeded for user ID 1000001.".into(),
        body: Vec::new(),
        headers: vec![
            ("X-RateLimit-Remaining".into(), "0".into()),
            ("X-RateLimit-Reset".into(), (1_792_929_600 + 300).to_string()),
            ("X-RateLimit-Resource".into(), "search".into()),
        ],
    };
    let gh = FakeGithub::new().with_search_page(Err(SearchError::Gh(limited)));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    assert_eq!(
        dialog(&app).status(),
        ("Search rate limit reached. Try again at 12:05".to_owned(), true)
    );
    assert!(toasts(&app).contains(&(
        Severity::Error,
        "GitHub search rate limit reached. Try again at 12:05".to_owned()
    )));
}

#[tokio::test]
async fn an_empty_query_with_no_scope_sends_nothing() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, SearchOrigin::Palette);
    press(&mut app, &["tab", "enter", "end", "enter", "shift+tab"]).await;
    assert_eq!(dialog(&app).scope(), Some(ScopeChoice::All));
    press(&mut app, &["enter"]).await;
    assert_eq!(dialog(&app).status(), ("Type a query to search".to_owned(), true));
    assert!(searches(&app).is_empty());
}

// Paging.

#[tokio::test]
async fn paging_stops_at_the_last_page() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(page(500, 50, 120)))
        .with_search_page(Ok(page(450, 50, 120)))
        .with_search_page(Ok(page(400, 20, 120)));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    for _ in 0..4 {
        press(&mut app, &["G"]).await;
    }
    assert_eq!(dialog(&app).hit_keys().len(), 120);
    let pages: Vec<u32> = searches(&app).into_iter().map(|(_, page)| page).collect();
    assert_eq!(pages, [1, 2, 3]);
}

#[tokio::test]
async fn paging_stops_at_githubs_cap() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(page(500, 50, 5000)))
        .with_search_page(Err(SearchError::PastLimit));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    for _ in 0..3 {
        press(&mut app, &["G"]).await;
    }
    assert_eq!(dialog(&app).hit_keys().len(), 50);
    let pages: Vec<u32> = searches(&app).into_iter().map(|(_, page)| page).collect();
    assert_eq!(pages, [1, 2]);
    assert_eq!(
        dialog(&app).status_line(),
        "5,000 results · GitHub returns the first 1,000"
    );
}

#[tokio::test]
async fn a_failed_later_page_is_asked_for_again_on_the_next_move() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(page(500, 50, 120)))
        .with_search_page(Err(SearchError::Gh(server_error())))
        .with_search_page(Ok(page(450, 50, 120)));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    press(&mut app, &["G"]).await;
    assert_eq!(
        dialog(&app).status_line(),
        "120 results · loading more failed, move down to retry"
    );
    assert_eq!(dialog(&app).hit_keys().len(), 50);
    press(&mut app, &["k"]).await;
    assert_eq!(dialog(&app).status_line(), "120 results");
    assert_eq!(dialog(&app).hit_keys().len(), 100);
    let pages: Vec<u32> = searches(&app).into_iter().map(|(_, page)| page).collect();
    assert_eq!(pages, [1, 2, 2]);
}

// Requests in flight.

/// The id of the latest search request.
fn last_search_id(app: &TestApp) -> RequestId {
    app.effects()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::SearchIssues(_),
            } => Some(*id),
            _ => None,
        })
        .expect("a search was sent")
}

#[tokio::test]
async fn a_stale_answer_is_dropped_after_a_new_submit() {
    let gh = FakeGithub::new().with_search_page(Ok(pr_details_open()));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    app.press(&["enter"]);
    let first = last_search_id(&app);
    app.press(&["enter"]);
    let second = last_search_id(&app);
    assert_ne!(first, second);
    assert!(app.effects().contains(&Effect::Cancel(first)));
    // The first answer arrives anyway: dropped.
    app.send(Event::Github {
        id: first,
        response: GithubResponse::SearchIssues(Ok(page(99, 1, 1))),
    });
    assert!(keys(&app).is_empty());
    assert_eq!(dialog(&app).status_line(), "Searching…");
    app.settle().await;
    assert_eq!(keys(&app), pr_keys(&[9, 5, 2]));
}

#[tokio::test]
async fn escape_cancels_the_search_in_flight_and_keeps_the_session() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, SearchOrigin::PullRequests);
    app.press(&["enter"]);
    let id = last_search_id(&app);
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(app.effects().contains(&Effect::Cancel(id)));
    open(&mut app, SearchOrigin::PullRequests);
    // The status from before the search, not a search that will never answer.
    assert_eq!(dialog(&app).status_line(), "Type a query and press enter");
    assert!(!dialog(&app).session.table.is_loading());
}

#[test]
fn rows_are_keyed_by_repo_and_number() {
    let mut table: SearchableTable<SearchHit> = SearchableTable::new(Columns { repo: true }.headers(), None);
    let _ = table.insert_many(
        [
            hit("parity-bot/pr-details", 5, "One"),
            hit("parity-bot/issue-fixture", 5, "Other"),
            hit("parity-bot/pr-details", 4, "Two"),
        ],
        &Columns { repo: true },
    );
    let keys: Vec<(String, u64)> = table
        .row_keys()
        .map(|(repo, number)| (repo.to_string(), *number))
        .collect();
    assert_eq!(
        keys,
        [
            ("parity-bot/pr-details".to_owned(), 5),
            ("parity-bot/issue-fixture".to_owned(), 5),
            ("parity-bot/pr-details".to_owned(), 4),
        ]
    );
}

#[test]
fn a_hits_cells_follow_the_column_set() {
    let first = mine_first();
    // Result 4: a merged PR in issue-fixture; result 5: a draft.
    let merged = &first.items[3];
    let cells = |hit: &SearchHit, columns| -> Vec<String> {
        hit.cells(&columns)
            .into_iter()
            .map(|cell| cell.display.to_string())
            .collect()
    };
    assert_eq!(
        cells(merged, Columns { repo: true }),
        [
            "parity-bot/issue-fixture",
            "1997",
            "PR",
            "merged",
            "parity-bot",
            "Search result 4",
            "2026-10-28"
        ]
    );
    assert_eq!(cells(&first.items[4], Columns { repo: false })[1..3], ["PR", "draft"]);
    assert_eq!(
        cells(&hit("o/r", 1, "x"), Columns { repo: false })[1..3],
        ["Issue", "open"]
    );
}

// Opening a hit.

#[tokio::test]
async fn enter_on_a_hit_closes_and_opens_it_reselecting_its_repo() {
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_pull_request("parity-bot/pr-details", 5, Ok(pr(5)));
    let mut app = selected(Config::default(), gh).await;
    let readmes_before = app
        .gh()
        .calls()
        .iter()
        .filter(|call| matches!(call, GithubCall::RepoReadme(_)))
        .count();
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    press(&mut app, &["j", "enter"]).await;
    assert!(app.app().modals().is_empty());
    let main = app.app().main_screen().unwrap();
    assert!(main.details_title().contains("PR #5"), "{}", main.details_title());
    assert_eq!(main.details_pull_request().map(FullPullRequest::number), Some(5));
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
    // The selected repo is selected again (#405 D2 a), without a lookup: its readme is asked for once more.
    let calls = app.gh().calls();
    assert!(!calls.iter().any(|call| matches!(call, GithubCall::Repository(_))));
    let readmes = calls
        .iter()
        .filter(|call| matches!(call, GithubCall::RepoReadme(_)))
        .count();
    assert_eq!(readmes, readmes_before + 1);
    assert!(calls.contains(&GithubCall::PullRequest {
        full_name: "parity-bot/pr-details".into(),
        number: 5,
    }));
    // Back to the results, the cursor on PR 5.
    open(&mut app, SearchOrigin::PullRequests);
    assert_eq!(dialog(&app).session.cursor(), 1);
    assert!(dialog(&app).table_focused());
}

#[tokio::test]
async fn a_hit_in_a_repo_that_cannot_be_found_toasts_and_changes_nothing() {
    let gh = FakeGithub::new().with_search_page(Ok(mine_first()));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::Palette);
    retype(&mut app, "is:pr author:@me");
    // Result 2 is in parity-bot/issue-fixture, which the fake doesn't know.
    press(&mut app, &["enter"]).await;
    press(&mut app, &["j", "enter"]).await;
    assert!(
        app.gh()
            .calls()
            .contains(&GithubCall::Repository("parity-bot/issue-fixture".into()))
    );
    assert_eq!(
        toasts(&app),
        [(
            Severity::Error,
            "Could not open parity-bot/issue-fixture#1999".to_owned()
        )]
    );
    assert_eq!(
        app.app().current_repo().map(|repo| repo.full_name.as_str()),
        Some("parity-bot/pr-details")
    );
}

// Keys.

#[tokio::test]
async fn s_on_the_pr_and_issues_tables_opens_it_and_the_footer_shows_it() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    app.press(&["2"]);
    let screen = app.screen(160, 48);
    assert!(screen.lines().last().unwrap().contains("s Search GitHub"), "{screen}");
    app.press(&["s"]);
    assert_eq!(app.app().modals(), [ModalKind::Search]);
    assert_eq!(dialog(&app).text(), "is:pr is:open ");
    app.press(&["escape", "3"]);
    let screen = app.screen(160, 48);
    assert!(screen.lines().last().unwrap().contains("s Search GitHub"), "{screen}");
    app.press(&["s"]);
    assert_eq!(app.app().modals(), [ModalKind::Search]);
}

#[tokio::test]
async fn search_open_is_rebindable() {
    let mut config = Config::default();
    config.bindings.overrides = IndexMap::from([("search.open".to_owned(), "ctrl+g".to_owned())]);
    let mut app = selected(config, FakeGithub::new()).await;
    app.press(&["2", "s"]);
    assert!(app.app().modals().is_empty());
    app.press(&["ctrl+g"]);
    assert_eq!(app.app().modals(), [ModalKind::Search]);
}

// Drawing.

#[tokio::test]
async fn the_pr_table_footer_offers_search() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    app.press(&["2"]);
    let screen = app.screen(160, 48);
    insta::assert_snapshot!(screen.lines().last().unwrap());
}

#[tokio::test]
async fn before_a_search() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, SearchOrigin::PullRequests);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[rstest]
#[case::wide(160, 48)]
#[case::narrow(100, 30)]
#[tokio::test]
async fn results(#[case] cols: u16, #[case] rows: u16) {
    let mut app = selected(
        Config::default(),
        FakeGithub::new().with_search_page(Ok(pr_details_open())),
    )
    .await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["enter"]).await;
    insta::assert_snapshot!(format!("results_{cols}x{rows}"), app.screen(cols, rows));
}

#[tokio::test]
async fn cross_repo_results() {
    let mut app = selected(Config::default(), FakeGithub::new().with_search_page(Ok(mine_first()))).await;
    open(&mut app, SearchOrigin::Palette);
    retype(&mut app, "is:pr author:@me");
    press(&mut app, &["tab", "enter", "end", "enter", "shift+tab", "enter"]).await;
    insta::assert_snapshot!(app.screen(160, 48));
}

#[tokio::test]
async fn the_select_open() {
    let mut config = Config::default();
    config.repositories.favorites = vec!["parity-bot/dotfiles".into()];
    let mut app = selected(config, FakeGithub::new()).await;
    open(&mut app, SearchOrigin::PullRequests);
    press(&mut app, &["tab", "enter"]).await;
    insta::assert_snapshot!(app.screen(160, 48));
}

#[tokio::test]
async fn the_scope_disabled_by_the_query() {
    let mut app = selected(Config::default(), FakeGithub::new()).await;
    open(&mut app, SearchOrigin::PullRequests);
    retype(&mut app, "is:pr repo:octocat/hello");
    insta::assert_snapshot!(app.screen(160, 48));
}

/// The status row of the modal at 160x48, for each state.
#[tokio::test]
async fn status_line_states() {
    let invalid = SearchError::Invalid {
        messages: vec!["The listed users and repositories cannot be searched.".into()],
    };
    let gh = FakeGithub::new()
        .with_search_page(Ok(pr_details_open()))
        .with_search_page(Ok(page(1, 0, 0)))
        .with_search_page(Err(invalid))
        .with_search_page(Err(SearchError::Gh(server_error())));
    let mut app = selected(Config::default(), gh).await;
    open(&mut app, SearchOrigin::PullRequests);
    // The status row: the heading block, the scope and the input, then the hint, inside the frame.
    let row = |app: &mut TestApp| -> String {
        let interior = app.app().top_dialog_interior().unwrap();
        let screen = app.screen(160, 48);
        let y = usize::from(interior.y + HEADING_ROWS + select::HEIGHT + text_input::HEIGHT + 1);
        let line: String = screen
            .lines()
            .nth(y)
            .unwrap()
            .chars()
            .skip(usize::from(interior.x))
            .take(usize::from(interior.width))
            .collect();
        line.trim_end().to_owned()
    };
    let mut lines = vec![row(&mut app)];
    app.press(&["enter"]);
    lines.push(row(&mut app));
    app.settle().await;
    lines.push(row(&mut app));
    for _ in 0..3 {
        to_input(&mut app);
        press(&mut app, &["enter"]).await;
        lines.push(row(&mut app));
    }
    to_input(&mut app);
    press(
        &mut app,
        &["ctrl+u", "tab", "enter", "end", "enter", "shift+tab", "enter"],
    )
    .await;
    lines.push(row(&mut app));
    insta::assert_snapshot!(lines.join("\n"));
}
