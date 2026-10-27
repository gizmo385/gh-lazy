use std::path::{Path, PathBuf};

use jiff::Timestamp;
use lazy_github_core::config::Config;
use lazy_github_core::github::IssueListQuery;
use lazy_github_core::models::{Issue, IssueListItem, IssueState, User};
use tokio::time::Instant;

use super::*;
use crate::components::loading::FRAME_INTERVAL;
use crate::components::table::{Cell, Handled};
use crate::keymap::BindingId;
use crate::screens::main::repos::repo;
use crate::screens::main::section_table::{SectionTable, earliest_deadline};
use crate::test_util::{TestApp, logs};

/// A row keyed by a number, cached as `{"number": n}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "NumberedJson", into = "NumberedJson")]
struct Numbered(u64);

/// [`Numbered`] in its cache file: an object, as every cached model is.
#[derive(serde::Serialize, serde::Deserialize)]
struct NumberedJson {
    number: u64,
}

impl From<NumberedJson> for Numbered {
    fn from(json: NumberedJson) -> Self {
        Self(json.number)
    }
}

impl From<Numbered> for NumberedJson {
    fn from(numbered: Numbered) -> Self {
        Self { number: numbered.0 }
    }
}

impl TableItem for Numbered {
    type Key = u64;
    type Ctx = ();

    fn key(&self) -> u64 {
        self.0
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        vec![Cell::int(i64::try_from(self.0).unwrap())]
    }
}

/// A source over the issues endpoint that keeps every item, whose failed pages end paging when `END`.
struct TestSource<const END: bool>;

impl<const END: bool> ListSource for TestSource<END> {
    type Item = Numbered;
    const CACHE_NAME: &'static str = "numbers";
    const TYPE_NAME: &'static str = "Numbered";

    fn columns() -> Vec<Column> {
        vec![Column::new("#")]
    }

    fn sort() -> SortSpec {
        SortSpec::descending(0)
    }

    fn page_request(repo: &Repository, page: u32, per_page: u32, config: &Config) -> GithubRequest {
        GithubRequest::ListIssues {
            repo: Box::new(repo.clone()),
            query: IssueListQuery {
                state: config.issues.state_filter,
                owner: config.issues.owner_filter,
                page,
                per_page,
            },
        }
    }

    fn page_items(response: GithubResponse) -> Result<Result<Page<Numbered>, GhError>, GithubResponse> {
        match response {
            GithubResponse::ListIssues(result) => Ok(result.map(|items| {
                items
                    .iter()
                    .map(|item| Numbered(item.issue().number))
                    .collect::<Vec<_>>()
                    .into()
            })),
            other => Err(other),
        }
    }

    fn page_failed(_error: &GhError) -> PageFailure {
        tracing::error!("Test page failed");
        if END { PageFailure::End } else { PageFailure::Retry }
    }
}

type Section = ListSection<TestSource<false>>;

fn section() -> Section {
    ListSection::new(AppOptions { animations: false })
}

fn numbered(range: std::ops::RangeInclusive<u64>) -> Vec<Numbered> {
    range.map(Numbered).collect()
}

fn issue(number: u64) -> IssueListItem {
    IssueListItem::Issue(Issue {
        id: number,
        number,
        locked: false,
        state: IssueState::Open,
        title: format!("Issue {number}"),
        body: None,
        user: User {
            login: "octo".into(),
            id: 1,
            name: None,
            html_url: "https://github.com/octo".into(),
        },
        created_at: Timestamp::UNIX_EPOCH,
        updated_at: Timestamp::UNIX_EPOCH,
        closed_at: None,
        closed_by: None,
        assignee: None,
        assignees: None,
        comments_url: String::new(),
        html_url: String::new(),
        repo: repo("octo", "lists", false),
    })
}

fn page_response(numbers: std::ops::RangeInclusive<u64>) -> GithubResponse {
    GithubResponse::ListIssues(Ok(numbers.map(issue).collect()))
}

fn http_error() -> GithubResponse {
    GithubResponse::ListIssues(Err(GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
    }))
}

/// The ids, repos and page numbers of every page request made so far, oldest first.
fn page_requests(app: &TestApp) -> Vec<(RequestId, String, u32, u32)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::ListIssues { repo, query },
            } => Some((*id, repo.full_name.clone(), query.page, query.per_page)),
            _ => None,
        })
        .collect()
}

fn cancels(app: &TestApp) -> Vec<RequestId> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// A section begun for `octo/lists` with a first page of `first` items.
fn loaded<S: ListSource<Item = Numbered>>(app: &mut TestApp, first: Vec<Numbered>) -> ListSection<S> {
    let mut section = ListSection::new(AppOptions { animations: false });
    app.with_cx(|cx| {
        section.begin(&repo("octo", "lists", false), cx);
        section.first_page(Some(first.into()), cx);
    });
    section.refresh_compact();
    section
}

/// Moves the cursor of `section`'s table with `binding`, following a page ask as `MainScreen::update` does.
fn cursor<S: ListSource>(app: &mut TestApp, section: &mut ListSection<S>, binding: BindingId) {
    if let Handled::Event(TableEvent::LoadMore {
        generation,
        page,
        per_page,
    }) = section.table_mut().on_binding(binding)
    {
        app.with_cx(|cx| section.load_more(generation, page, per_page, cx));
    }
}

fn lazy<S: ListSource>(section: &ListSection<S>) -> (u32, bool, Option<u32>) {
    let lazy = section.table().lazy_load().expect("paging is on");
    (lazy.current_batch(), lazy.can_load_more(), lazy.in_flight())
}

#[test]
fn before_any_repo_the_table_is_empty_and_not_compact() {
    let section = section();
    assert_eq!(section.table().row_count(), 0);
    assert!(!section.table().is_loading());
    assert!(!section.compact());
    assert_eq!(section.repo(), None);
}

#[test]
fn begin_clears_shows_the_overlay_and_keeps_compact() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, Vec::new());
    assert!(section.compact());
    app.with_cx(|cx| section.begin(&repo("octo", "other", false), cx));
    assert!(section.table().is_loading());
    assert!(section.compact(), "compact stays until the new first page lands");
    assert_eq!(section.table().lazy_load(), None);
    assert_eq!(section.repo().map(|repo| repo.full_name.as_str()), Some("octo/other"));
}

#[test]
fn begin_cancels_a_page_in_flight_and_clears_the_rows() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let [(page_2, ..)] = page_requests(&app)[..] else {
        panic!("one page request: {:?}", page_requests(&app));
    };
    let read = app.effects().iter().find_map(|effect| match effect {
        Effect::ReadCache { id, .. } => Some(*id),
        _ => None,
    });
    app.with_cx(|cx| section.begin(&repo("octo", "other", false), cx));
    // The first repo's cache read was never answered, so it's cancelled too.
    assert_eq!(
        cancels(&app),
        [Some(page_2), read].into_iter().flatten().collect::<Vec<_>>()
    );
    assert_eq!(section.table().row_count(), 0);
    assert_eq!((section.table().cursor(), section.table().scroll()), (0, (0, 0)));
    assert!(section.table().is_loading());
    assert!(!section.compact());
}

#[test]
fn a_short_first_page_turns_paging_on_and_asks_for_page_two_at_once() {
    let mut app = TestApp::new(Config::default());
    let section: Section = loaded(&mut app, numbered(1..=3));
    assert!(!section.table().is_loading());
    assert_eq!(lazy(&section), (1, true, Some(2)));
    let requests = page_requests(&app);
    assert_eq!(
        requests
            .iter()
            .map(|(_, repo, page, per_page)| (repo.as_str(), *page, *per_page))
            .collect::<Vec<_>>(),
        [("octo/lists", 2, 30)]
    );
    assert!(!section.compact());
}

#[test]
fn an_empty_first_page_makes_the_section_compact() {
    let mut app = TestApp::new(Config::default());
    let section: Section = loaded(&mut app, Vec::new());
    assert!(!section.table().is_loading());
    assert!(section.compact());
    assert_eq!(lazy(&section), (1, true, None));
    assert_eq!(page_requests(&app), []);
}

#[test]
fn a_failed_first_page_is_an_empty_one_and_the_section_logs_nothing() {
    let mut app = TestApp::new(Config::default());
    let mut section = section();
    let log = logs(|| {
        app.with_cx(|cx| {
            section.begin(&repo("octo", "lists", false), cx);
            section.first_page(None, cx);
        });
    });
    section.refresh_compact();
    assert_eq!(log, "");
    assert!(!section.table().is_loading());
    assert!(section.compact());
    assert_eq!(lazy(&section), (1, true, None));
    assert_eq!(page_requests(&app), []);
}

#[test]
fn a_load_more_issues_one_request_and_its_response_appends_rows() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    // Python's PR table sorts newest first, so the bottom row is the lowest number.
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    // More moves while the page is in flight ask for nothing more.
    cursor(&mut app, &mut section, BindingId::TableCursorUp);
    cursor(&mut app, &mut section, BindingId::TableCursorDown);
    let [(id, ref repo, 2, 30)] = page_requests(&app)[..] else {
        panic!("one request for page 2: {:?}", page_requests(&app));
    };
    assert_eq!(repo, "octo/lists");
    let unclaimed = app.with_cx(|cx| section.on_response(id, page_response(31..=33), cx));
    assert!(unclaimed.is_none());
    assert_eq!(section.table().row_count(), 33);
    assert_eq!(section.table().cursor(), 29, "the cursor doesn't move");
    // The moves seen while page 2 was in flight are checked again: row 29 of 33 asks for page 3.
    assert_eq!(lazy(&section), (2, true, Some(3)));
    let pages: Vec<u32> = page_requests(&app).iter().map(|request| request.2).collect();
    assert_eq!(pages, [2, 3]);
}

#[test]
fn a_later_page_is_for_the_loaded_repo_not_the_current_one() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    // Nothing is selected app-wide: the section asks for the repo it was begun with.
    assert!(app.with_cx(|cx| cx.current_repo().is_none()));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    assert_eq!(page_requests(&app)[0].1, "octo/lists");
}

#[test]
fn other_responses_are_handed_back() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    let other = app.with_cx(|cx| {
        section.on_response(
            id,
            GithubResponse::CurrentUser(Err(GhError::Http {
                status: 500,
                message: String::new(),
                body: Vec::new(),
            })),
            cx,
        )
    });
    assert!(matches!(other, Some(GithubResponse::CurrentUser(_))));
    let stranger = app.with_cx(|cx| {
        let stranger = cx.request(GithubRequest::CurrentUser);
        section.on_response(stranger, page_response(31..=33), cx)
    });
    assert!(stranger.is_some());
    assert_eq!(section.table().row_count(), 30);
}

#[test]
fn a_page_that_arrives_after_begin_is_dropped() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (old, ..) = page_requests(&app)[0];
    app.with_cx(|cx| {
        section.begin(&repo("octo", "other", false), cx);
        section.first_page(Some(numbered(101..=130).into()), cx);
    });
    let late = app.with_cx(|cx| section.on_response(old, page_response(31..=60), cx));
    assert!(late.is_some(), "handed back, so the app drops it");
    assert_eq!(section.table().row_count(), 30);
    assert!(section.table().get(&31).is_none());
    assert_eq!(lazy(&section), (1, true, None));
}

#[test]
fn an_empty_last_page_ends_paging() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    assert!(
        app.with_cx(|cx| section.on_response(id, GithubResponse::ListIssues(Ok(Vec::new())), cx))
            .is_none()
    );
    assert_eq!(lazy(&section), (2, false, None));
    cursor(&mut app, &mut section, BindingId::TableCursorUp);
    assert_eq!(page_requests(&app).len(), 1);
}

#[test]
fn a_failed_page_that_retries_keeps_the_paging_state() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    let log = logs(|| assert!(app.with_cx(|cx| section.on_response(id, http_error(), cx)).is_none()));
    assert!(log.contains("ERROR Test page failed"), "{log}");
    assert_eq!(lazy(&section), (1, true, None));
    assert_eq!(section.table().row_count(), 30);
    // The next cursor move asks for the same page again.
    cursor(&mut app, &mut section, BindingId::TableCursorUp);
    let pages: Vec<u32> = page_requests(&app).iter().map(|request| request.2).collect();
    assert_eq!(pages, [2, 2]);
}

#[test]
fn a_failed_page_that_ends_ends_paging() {
    let mut app = TestApp::new(Config::default());
    let mut section: ListSection<TestSource<true>> = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    let log = logs(|| assert!(app.with_cx(|cx| section.on_response(id, http_error(), cx)).is_none()));
    assert!(log.contains("ERROR Test page failed"), "{log}");
    assert_eq!(lazy(&section), (2, false, None));
    assert_eq!(section.table().row_count(), 30);
    cursor(&mut app, &mut section, BindingId::TableCursorUp);
    assert_eq!(page_requests(&app).len(), 1);
}

#[test]
fn a_short_later_page_asks_for_the_next_at_once_when_the_cursor_is_near_the_end() {
    let mut app = TestApp::new(Config::default());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    assert!(
        app.with_cx(|cx| section.on_response(id, page_response(31..=31), cx))
            .is_none()
    );
    // Without a move while page 2 was in flight, nothing re-checks.
    assert_eq!(page_requests(&app).len(), 1);
    cursor(&mut app, &mut section, BindingId::TableCursorDown);
    let pages: Vec<u32> = page_requests(&app).iter().map(|request| request.2).collect();
    assert_eq!(pages, [2, 3]);
}

#[test]
fn the_screen_deadline_includes_a_loading_section_table() {
    let mut app = TestApp::new(Config::default());
    let mut animated: Section = ListSection::new(AppOptions { animations: true });
    let mut still = section();
    let now = Instant::now();
    let tables = |a: &Section, b: &Section| -> Option<Instant> {
        earliest_deadline([a.table() as &dyn SectionTable, b.table() as &dyn SectionTable], now)
    };
    assert_eq!(tables(&animated, &still), None);
    app.with_cx(|cx| {
        animated.begin(&repo("octo", "lists", false), cx);
        still.begin(&repo("octo", "lists", false), cx);
    });
    assert_eq!(tables(&animated, &still), Some(now + FRAME_INTERVAL));
    // The screen's own deadline goes through the same function, over every mounted table.
    let screen = app.app().main_screen().expect("on the main screen");
    assert_eq!(screen.next_deadline(now), None);
}

// The repo's cache file.

fn cache_config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = PathBuf::from("cache-dir");
    config
}

fn cache_path(full_name: &str) -> PathBuf {
    Path::new("cache-dir").join(format!("{}_numbers.json", full_name.replace('/', "_")))
}

/// The cache reads among `app`'s effects, oldest first.
fn cache_reads(app: &TestApp) -> Vec<(RequestId, PathBuf)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::ReadCache { id, path } => Some((*id, path.clone())),
            _ => None,
        })
        .collect()
}

/// The cache writes among `app`'s effects, oldest first, with the numbers each wrote.
fn cache_writes(app: &TestApp) -> Vec<(PathBuf, Vec<u64>)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::WriteCache { path, text } => {
                let items: Vec<Numbered> = serde_json::from_str(text).unwrap();
                Some((path.clone(), items.into_iter().map(|item| item.0).collect()))
            }
            _ => None,
        })
        .collect()
}

/// The rows' numbers, top to bottom.
fn rows<S: ListSource<Item = Numbered>>(section: &ListSection<S>) -> Vec<u64> {
    section.table().row_keys().copied().collect()
}

/// The cache file's text for `numbers`.
fn cached_text(numbers: std::ops::RangeInclusive<u64>) -> String {
    cache::to_json(numbered(numbers))
}

/// A section begun for `octo/lists`, its cache read answered with `cached`.
fn restored(app: &mut TestApp, cached: std::ops::RangeInclusive<u64>) -> Section {
    let mut section = section();
    app.with_cx(|cx| section.begin(&repo("octo", "lists", false), cx));
    let (id, _) = cache_reads(app).pop().expect("begin reads the cache");
    let unclaimed = app.with_cx(|cx| section.on_cache_read(id, Ok(Some(cached_text(cached))), cx));
    assert!(unclaimed.is_none());
    section
}

#[test]
fn begin_reads_the_repos_cache_file() {
    let mut app = TestApp::new(cache_config());
    let mut section = section();
    app.with_cx(|cx| section.begin(&repo("octo", "lists", false), cx));
    let [(_, ref path)] = cache_reads(&app)[..] else {
        panic!("one read: {:?}", cache_reads(&app));
    };
    assert_eq!(path, &cache_path("octo/lists"));
    assert_eq!(cache_writes(&app), []);
}

#[test]
fn cached_rows_go_in_under_the_overlay_and_ask_for_nothing() {
    let mut app = TestApp::new(cache_config());
    let section = restored(&mut app, 1..=3);
    assert_eq!(rows(&section), [3, 2, 1]);
    assert!(section.table().is_loading(), "the overlay stays until the first page");
    assert_eq!(section.table().lazy_load(), None);
    assert_eq!(page_requests(&app), []);
    assert_eq!(cache_writes(&app), [], "restoring writes nothing");
}

#[test]
fn a_missing_cache_file_restores_nothing() {
    let mut app = TestApp::new(cache_config());
    let mut section = section();
    app.with_cx(|cx| section.begin(&repo("octo", "lists", false), cx));
    let (id, _) = cache_reads(&app)[0];
    let log = logs(|| assert!(app.with_cx(|cx| section.on_cache_read(id, Ok(None), cx)).is_none()));
    assert_eq!(log, "");
    assert_eq!(section.table().row_count(), 0);
    assert!(section.table().is_loading());
}

#[test]
fn a_cached_read_after_the_first_page_is_dropped() {
    let mut app = TestApp::new(cache_config());
    let mut section = section();
    app.with_cx(|cx| {
        section.begin(&repo("octo", "lists", false), cx);
        section.first_page(Some(numbered(10..=12).into()), cx);
    });
    let (id, _) = cache_reads(&app)[0];
    let unclaimed = app.with_cx(|cx| section.on_cache_read(id, Ok(Some(cached_text(1..=3))), cx));
    assert!(unclaimed.is_none(), "claimed and dropped");
    assert_eq!(rows(&section), [12, 11, 10]);
}

#[test]
fn a_successful_first_page_replaces_the_cached_rows_and_writes_them() {
    let mut app = TestApp::new(cache_config());
    let mut section = restored(&mut app, 1..=3);
    app.with_cx(|cx| section.first_page(Some(numbered(2..=4).into()), cx));
    section.refresh_compact();
    assert_eq!(rows(&section), [4, 3, 2], "row 1 is gone (Py bug B53)");
    assert!(!section.table().is_loading());
    assert_eq!(cache_writes(&app), [(cache_path("octo/lists"), vec![2, 3, 4])]);
    // A short first page asks for page 2 at once, as without a cache.
    assert_eq!(lazy(&section), (1, true, Some(2)));
}

#[test]
fn an_empty_first_page_replaces_the_cached_rows_and_the_section_is_compact() {
    let mut app = TestApp::new(cache_config());
    let mut section = restored(&mut app, 1..=3);
    app.with_cx(|cx| section.first_page(Some(Vec::new().into()), cx));
    section.refresh_compact();
    assert_eq!(section.table().row_count(), 0);
    assert!(section.compact());
    assert_eq!(cache_writes(&app), [(cache_path("octo/lists"), vec![])]);
}

#[test]
fn a_failed_first_page_keeps_the_cached_rows_and_rewrites_them() {
    let mut app = TestApp::new(cache_config());
    let mut section = restored(&mut app, 1..=3);
    app.with_cx(|cx| section.first_page(None, cx));
    section.refresh_compact();
    assert_eq!(rows(&section), [3, 2, 1]);
    assert!(!section.table().is_loading());
    assert!(!section.compact());
    assert_eq!(cache_writes(&app), [(cache_path("octo/lists"), vec![1, 2, 3])]);
    // Paging is on and counts the cached rows: the next cursor move asks for page 2.
    assert_eq!(lazy(&section), (1, true, None));
    cursor(&mut app, &mut section, BindingId::TableCursorDown);
    let pages: Vec<u32> = page_requests(&app).iter().map(|request| request.2).collect();
    assert_eq!(pages, [2]);
}

#[test]
fn each_later_page_writes_every_row() {
    let mut app = TestApp::new(cache_config());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    assert!(
        app.with_cx(|cx| section.on_response(id, page_response(31..=33), cx))
            .is_none()
    );
    let writes = cache_writes(&app);
    let [(ref first_path, ref first), (ref path, ref second)] = writes[..] else {
        panic!("one write per page: {writes:?}");
    };
    assert_eq!((first_path, first.len()), (&cache_path("octo/lists"), 30));
    assert_eq!(path, &cache_path("octo/lists"));
    assert_eq!(second, &(1..=33).collect::<Vec<_>>());
}

#[test]
fn a_failed_later_page_that_retries_writes_nothing() {
    let mut app = TestApp::new(cache_config());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (id, ..) = page_requests(&app)[0];
    logs(|| assert!(app.with_cx(|cx| section.on_response(id, http_error(), cx)).is_none()));
    assert_eq!(cache_writes(&app).len(), 1, "the first page's only");
}

#[test]
fn a_read_or_page_for_a_repo_no_longer_selected_is_ignored() {
    let mut app = TestApp::new(cache_config());
    let mut section: Section = loaded(&mut app, numbered(1..=30));
    cursor(&mut app, &mut section, BindingId::TableScrollBottom);
    let (page_2, ..) = page_requests(&app)[0];
    app.with_cx(|cx| section.begin(&repo("octo", "other", false), cx));
    // A third repo before the second's read answers: that read is cancelled, and handed back if it still arrives.
    let (old_read, _) = cache_reads(&app)[1];
    app.with_cx(|cx| section.begin(&repo("octo", "third", false), cx));
    assert!(cancels(&app).contains(&old_read));
    let late = app.with_cx(|cx| section.on_cache_read(old_read, Ok(Some(cached_text(1..=3))), cx));
    assert!(late.is_some(), "handed back, so the app drops it");
    assert_eq!(section.table().row_count(), 0);
    // The first repo's page 2 was cancelled; if it still arrives it writes nothing.
    let writes = cache_writes(&app).len();
    assert!(
        app.with_cx(|cx| section.on_response(page_2, page_response(31..=33), cx))
            .is_some()
    );
    assert_eq!(cache_writes(&app).len(), writes);
    // The current repo's own read and first page go to its file.
    let (read, ref path) = cache_reads(&app)[2];
    assert_eq!(path, &cache_path("octo/third"));
    assert!(app.with_cx(|cx| section.on_cache_read(read, Ok(None), cx)).is_none());
    app.with_cx(|cx| section.first_page(Some(numbered(7..=7).into()), cx));
    assert_eq!(cache_writes(&app).last(), Some(&(cache_path("octo/third"), vec![7])));
}
