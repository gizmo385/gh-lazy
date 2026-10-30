//! Timing tests for large data (#325): a 1,000-repo account, a 300-file diff, long job logs, 5,000 notifications, a
//! 1 MB issue body and a search over 1,000 rows. Each case drives the app as a user would and times every frame:
//! the update a key press or a response causes, plus a render of the whole screen to a 160x48 `TestBackend`.
//!
//! They're `#[ignore]`d, so the required CI job skips them; `cargo xtask perf` runs them in a release build, one
//! process per case so its peak RSS is the case's own. The 50 ms budget is asserted only in release builds: a debug
//! build (`cargo test -- --ignored`) still runs every case, as a check that nothing panics on large data.
//!
//! The data is generated here, from one item of an existing parity fixture set used as a template.

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, NotificationQuery};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Notification, Repository, WorkflowJob, WorkflowRun};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::Value;

use crate::test_util::{TestApp, large_markdown};

/// The longest any frame may take to build in a release build.
const BUDGET: Duration = Duration::from_millis(50);

const COLS: u16 = 160;
const ROWS: u16 = 48;

/// The response body of a parity fixture.
fn fixture_body(set: &str, path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// The frames of one case, with how long each took to build.
struct Timing {
    case: &'static str,
    frames: Vec<(String, Duration)>,
    /// One terminal for the whole case, as the app draws to one: each frame is drawn over the last.
    terminal: Terminal<TestBackend>,
}

impl Timing {
    fn new(case: &'static str) -> Self {
        Self {
            case,
            frames: Vec::new(),
            terminal: Terminal::new(TestBackend::new(COLS, ROWS)).unwrap(),
        }
    }

    /// Runs `step` (a key press, a response) and draws the screen, timing both as one frame.
    fn frame(&mut self, label: impl Into<String>, app: &mut TestApp, step: impl FnOnce(&mut TestApp)) {
        let start = Instant::now();
        step(app);
        self.terminal.draw(|frame| app.app_mut().render(frame)).unwrap();
        self.frames.push((label.into(), start.elapsed()));
    }

    /// [`Self::frame`] for pressing `key`, its requests answered.
    fn press(&mut self, app: &mut TestApp, key: &str) {
        self.frame(format!("press {key}"), app, |app| {
            app.press(&[key]);
            settle(app);
        });
    }

    /// Prints the slowest frames and a `PERF` summary line (which `cargo xtask perf` collects), then fails if a frame
    /// went over [`BUDGET`] in a release build.
    fn finish(self) {
        let ms = |took: Duration| took.as_secs_f64() * 1000.0;
        let mut sorted: Vec<&(String, Duration)> = self.frames.iter().collect();
        sorted.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
        let median = sorted.get(sorted.len() / 2).map_or(0.0, |(_, took)| ms(*took));
        let slowest = sorted.first().map_or(0.0, |(_, took)| ms(*took));
        let over = sorted.iter().filter(|(_, took)| *took > BUDGET).count();
        let mut report = format!("{}: {} frames, slowest:\n", self.case, self.frames.len());
        for (label, took) in sorted.iter().take(5) {
            let _ = writeln!(report, "  {:>9.3} ms  {label}", ms(*took));
        }
        let rss = peak_rss_kib().map_or_else(|| "?".to_owned(), |kib| format!("{:.1}", f64::from(kib) / 1024.0));
        let _ = writeln!(
            report,
            "PERF case={} frames={} median_ms={median:.3} slowest_ms={slowest:.3} over_budget={over} peak_rss_mib={rss}",
            self.case,
            self.frames.len(),
        );
        println!("{report}");
        if cfg!(debug_assertions) {
            println!("(a debug build: the {} ms budget isn't checked)", BUDGET.as_millis());
            return;
        }
        assert_eq!(over, 0, "{}: {over} frames over {} ms", self.case, BUDGET.as_millis());
    }
}

/// The process's peak resident set size, where the platform reports it (Linux's `VmHWM`).
fn peak_rss_kib() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Fails unless the screen shows `text`, so a case can't pass by timing the wrong screen.
fn shows(app: &mut TestApp, text: &str) {
    let screen = app.screen(COLS, ROWS);
    assert!(screen.contains(text), "expected {text:?} on screen:\n{screen}");
}

/// Fails if the screen shows `text`.
fn hides(app: &mut TestApp, text: &str) {
    let screen = app.screen(COLS, ROWS);
    assert!(!screen.contains(text), "expected no {text:?} on screen:\n{screen}");
}

fn settle(app: &mut TestApp) {
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
}

// Data.

/// `count` repos named so they sort and filter apart: `parity-bot/service-0000` and on.
fn repos(count: usize) -> Vec<Repository> {
    let template = fixture_body(
        "repos-60",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    )[0]
    .clone();
    (0..count)
        .map(|i| {
            let mut repo = template.clone();
            let name = format!("service-{i:04}-with-a-deliberately-long-repository-name");
            repo["id"] = (4_000_000 + i).into();
            repo["full_name"] = format!("parity-bot/{name}").into();
            repo["html_url"] = format!("https://github.com/parity-bot/{name}").into();
            repo["url"] = format!("https://api.github.com/repos/parity-bot/{name}").into();
            repo["description"] = format!("Large account repository {} of {count}", i + 1).into();
            repo["name"] = name.into();
            serde_json::from_value(repo).unwrap()
        })
        .collect()
}

const PR_REPO: &str = "parity-bot/pr-details";

fn pr_repo() -> Repository {
    let page = fixture_body(
        "pr-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

/// A unified diff of `files` files, each one hunk of `lines` changed lines: half removed, half added, with a little
/// context, in Rust so the hunk panes highlight it.
fn large_diff(files: usize, lines: usize) -> String {
    let mut diff = String::new();
    for file in 0..files {
        let path = format!("src/module_{file:03}/generated_{file:03}.rs");
        let removed = lines / 2;
        let added = lines - removed;
        let _ = writeln!(diff, "diff --git a/{path} b/{path}");
        let _ = writeln!(diff, "index 1111111..2222222 100644");
        let _ = writeln!(diff, "--- a/{path}\n+++ b/{path}");
        let _ = writeln!(diff, "@@ -1,{} +1,{} @@", removed + 2, added + 2);
        diff.push_str(" // Generated for the large-diff timing test.\n");
        for line in 0..removed {
            let _ = writeln!(
                diff,
                "-fn old_{line}(value: u32) -> u32 {{ value.wrapping_mul({line}) + 1 }}"
            );
        }
        for line in 0..added {
            let _ = writeln!(
                diff,
                "+fn new_{line}(value: u32) -> u32 {{ value.wrapping_add({line}) * 2 }}"
            );
        }
        diff.push_str(" // End.\n");
    }
    diff
}

/// `count` notifications, unread and read in turn, newest first, numbered from `first`.
fn notifications(first: usize, count: usize) -> Vec<Notification> {
    let template = fixture_body(
        "notifications-modal",
        "notifications/GET.all-true_page-1_per_page-50.json",
    )[0]
    .clone();
    (first..first + count)
        .map(|i| {
            let mut notification = template.clone();
            notification["id"] = (9_500_000 + i).to_string().into();
            notification["unread"] = (i % 2 == 0).into();
            notification["subject"]["title"] = format!("Notification {i:04} about a pull request").into();
            let minutes = 1_000_000 - i64::try_from(i).unwrap();
            let at = jiff::Timestamp::from_second(minutes * 60 + 1_700_000_000).unwrap();
            notification["updated_at"] = at.to_string().into();
            serde_json::from_value(notification).unwrap()
        })
        .collect()
}

// Cases.

fn repos_app(count: usize) -> (TestApp, Timing) {
    let gh = FakeGithub::new().with_repositories(repos(count));
    let mut app = TestApp::loading(Config::default(), gh);
    let mut frames = Timing::new("repos");
    frames.frame("repos arrive", &mut app, settle);
    (app, frames)
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_repos_1000() {
    let (mut app, mut frames) = repos_app(1000);
    shows(&mut app, "service-0000");
    for key in ["j", "j", "J", "J", "G"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "service-0999");
    for key in ["k", "K", "g", "l", "h", "$", "^", "enter"] {
        frames.press(&mut app, key);
    }
    frames.finish();
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_table_search_1000() {
    let (mut app, mut frames) = repos_app(1000);
    frames.case = "table search";
    frames.press(&mut app, "/");
    for key in "service-09".chars() {
        frames.press(&mut app, &key.to_string());
    }
    // As in Python, the filter applies on `enter`.
    frames.press(&mut app, "enter");
    shows(&mut app, "service-0900");
    assert!(!app.screen(COLS, ROWS).contains("service-0000"));
    for key in ["j", "G", "/"] {
        frames.press(&mut app, key);
    }
    for _ in 0.."service-09".len() {
        frames.press(&mut app, "backspace");
    }
    frames.press(&mut app, "enter");
    shows(&mut app, "service-0000");
    frames.finish();
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_pr_diff_300_files() {
    let repo = pr_repo();
    let page = fixture_body(
        "pr-details",
        "repos/parity-bot/pr-details/issues/GET.page-1_per_page-30_state-all.json",
    );
    let items: Vec<IssueListItem> = page
        .as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect();
    let pr = FullPullRequest::from_api(
        fixture_body("pr-details", "repos/parity-bot/pr-details/pulls/5/GET.json"),
        &repo,
    )
    .unwrap();
    // 300 files of 67 lines each: 20,100 changed lines.
    let diff = large_diff(300, 67);
    let gh = FakeGithub::new()
        .with_repositories(vec![repo])
        .with_issue_page(PR_REPO, 1, Ok(items))
        .with_pull_request(PR_REPO, 5, Ok(pr))
        .with_pull_request_diff(PR_REPO, 5, Ok(diff));
    let mut config = Config::default();
    config.appearance.show_issues = false;
    config.appearance.show_workflows = false;
    let mut app = TestApp::with_github(config, gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    let mut frames = Timing::new("pr diff");
    // The Diff tab: the diff is parsed and laid out for the first time here.
    frames.press(&mut app, "right");
    shows(&mut app, "generated_000.rs");
    frames.press(&mut app, "tab");
    for _ in 0..5 {
        frames.press(&mut app, "tab");
    }
    // `J` opens each hunk in turn, so by the end every hunk is open and highlighted.
    for _ in 0..300 {
        frames.press(&mut app, "J");
    }
    shows(&mut app, "generated_299.rs");
    // The last pane's cursor, with every hunk open.
    for _ in 0..10 {
        frames.press(&mut app, "j");
        frames.press(&mut app, "k");
    }
    for key in ["K", "end", "home", "pagedown", "down", "up"] {
        frames.press(&mut app, key);
    }
    frames.finish();
}

const RUN_REPO: &str = "parity-bot/run-fixture";
const ACTIONS: &str = "repos/parity-bot/run-fixture/actions";

/// Run 42 of `run-details` with two jobs: one whose log is a single 50,000-character line, one whose log is 200,000
/// lines.
fn run_gh() -> FakeGithub {
    let repo: Repository = serde_json::from_value(
        fixture_body(
            "run-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )[0]
        .clone(),
    )
    .unwrap();
    let runs: Vec<WorkflowRun> = serde_json::from_value(
        fixture_body("run-details", &format!("{ACTIONS}/runs/GET.page-1_per_page-30.json"))["workflow_runs"].clone(),
    )
    .unwrap();
    let run = runs.iter().find(|run| run.run_number == 42).unwrap().clone();
    let template = fixture_body("run-details", &format!("{ACTIONS}/runs/{}/jobs/GET.json", run.id))["jobs"][0].clone();
    let job = |id: u64, name: &str| -> WorkflowJob {
        let mut job = template.clone();
        job["id"] = id.into();
        job["name"] = name.into();
        serde_json::from_value(job).unwrap()
    };
    let long_line = format!("2026-10-24T12:00:00.0000000Z {}\n", "x".repeat(50_000));
    let mut many_lines = String::with_capacity(200_000 * 60);
    for line in 0..200_000 {
        let _ = writeln!(many_lines, "2026-10-24T12:00:00.0000000Z step output line {line}");
    }
    FakeGithub::new()
        .with_repositories(vec![repo])
        .with_readme(RUN_REPO, Ok(Some("Workflow runs.\n".into())))
        .with_workflow_page(RUN_REPO, 1, Ok(Vec::new()))
        .with_run_page(RUN_REPO, 1, Ok(vec![run.clone()]))
        .with_run_jobs(
            run.id,
            Ok(vec![job(7_000_001, "long line"), job(7_000_002, "many lines")]),
        )
        .with_job_logs(7_000_001, Ok(Some(long_line)))
        .with_job_logs(7_000_002, Ok(Some(many_lines)))
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_job_logs() {
    let mut app = TestApp::with_github(Config::default(), run_gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["4", "enter"]);
    settle(&mut app);
    let mut frames = Timing::new("job logs");
    // The Jobs tab: `l` on each job shows its log in the tab's body.
    frames.press(&mut app, "right");
    for key in ["tab", "tab", "down", "l"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "xxxxxxxxxxxxxxxxxxxx");
    for key in ["down", "l"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "step output line 0");
    // The body, scrolled.
    for key in ["shift+tab", "end", "home", "pagedown", "end"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "(logs truncated, too long to display)");
    // The Logs tab: every job's log, one after the other.
    for key in [
        "shift+tab",
        "right",
        "tab",
        "end",
        "home",
        "pagedown",
        "down",
        "up",
        "end",
    ] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "(logs truncated, too long to display)");
    frames.finish();
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_notifications_100_pages() {
    let mut gh = FakeGithub::new().with_unread_count(Ok(2500));
    for page in 1..=100u32 {
        let first = (page as usize - 1) * 50;
        gh = gh.with_notification_page(NotificationQuery { all: true, page }, Ok(notifications(first, 50)));
        let unread = notifications(first, 50)
            .into_iter()
            .map(|n| Notification { unread: true, ..n })
            .collect();
        gh = gh.with_notification_page(NotificationQuery { all: false, page }, Ok(unread));
    }
    for page in [101, 102] {
        for all in [true, false] {
            gh = gh.with_notification_page(NotificationQuery { all, page }, Ok(Vec::new()));
        }
    }
    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::with_github(config, gh);
    settle(&mut app);
    let mut frames = Timing::new("notifications");
    frames.press(&mut app, "ctrl+n");
    // Each `G` reaches the end of the loaded rows, which asks for the next page.
    for _ in 0..110 {
        frames.press(&mut app, "G");
    }
    shows(&mut app, "Notification 4998");
    for key in ["g", "j", "J", "G", "k"] {
        frames.press(&mut app, key);
    }
    // The Read tab pages on its own.
    frames.press(&mut app, "r");
    for _ in 0..110 {
        frames.press(&mut app, "G");
    }
    shows(&mut app, "Notification 4999");
    frames.finish();
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_markdown_1mb_body() {
    let repo: Repository = serde_json::from_value(
        fixture_body(
            "issue-details",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )[0]
        .clone(),
    )
    .unwrap();
    let mut item = fixture_body(
        "issue-details",
        "repos/parity-bot/issue-fixture/issues/GET.page-1_per_page-30_state-all.json",
    )[0]
    .clone();
    item["body"] = large_markdown(1 << 20, 500).into();
    let issue = IssueListItem::from_api_item(item, &repo).unwrap();
    let gh = FakeGithub::new()
        .with_repositories(vec![repo])
        .with_issue_page("parity-bot/issue-fixture", 1, Ok(vec![issue]))
        .with_issue_comments("parity-bot/issue-fixture", 12, Ok(Vec::new()))
        .with_readme("parity-bot/issue-fixture", Ok(Some("Issues.\n".into())));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    // The parse runs on a blocking thread in the app, not in a frame: held back here, and run between frames.
    app.hold_parses();
    let mut frames = Timing::new("markdown body");
    frames.press(&mut app, "3");
    // The issue's Overview opens at once, with nothing below the rule.
    frames.press(&mut app, "enter");
    shows(&mut app, "Crash when the config file is empty");
    hides(&mut app, "A very large issue");
    let parsed = app.run_held_parses();
    // The body lands and is laid out once, at the width beside the scrollbar.
    frames.frame("body parsed", &mut app, |app| {
        for event in parsed {
            app.send(event);
        }
    });
    shows(&mut app, "A very large issue");
    for key in ["tab", "pagedown", "down", "end"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "item-499");
    for key in ["up", "home", "pagedown"] {
        frames.press(&mut app, key);
    }
    frames.finish();
}

#[test]
#[ignore = "timing test: run with `cargo xtask perf`"]
fn perf_markdown_1mb_readme() {
    let repo: Repository = serde_json::from_value(
        fixture_body(
            "repo-overview",
            "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
        )[0]
        .clone(),
    )
    .unwrap();
    let gh = FakeGithub::new()
        .with_readme(&repo.full_name, Ok(Some(large_markdown(1 << 20, 500))))
        .with_repositories(vec![repo]);
    let mut app = TestApp::with_github(Config::default(), gh);
    app.hold_parses();
    // Selecting the repo requests its README.
    app.press(&["enter"]);
    let mut frames = Timing::new("markdown readme");
    // The README lands; it's parsed off the frame, so the Repo Overview shows nothing below the rule.
    frames.frame("readme arrives", &mut app, settle);
    hides(&mut app, "A very large issue");
    let parsed = app.run_held_parses();
    assert_eq!(parsed.len(), 1);
    frames.frame("readme parsed", &mut app, |app| {
        for event in parsed {
            app.send(event);
        }
    });
    shows(&mut app, "A very large issue");
    frames.press(&mut app, "5");
    for key in ["tab", "end"] {
        frames.press(&mut app, key);
    }
    shows(&mut app, "item-499");
    frames.press(&mut app, "home");
    shows(&mut app, "A very large issue");
    frames.finish();
}
