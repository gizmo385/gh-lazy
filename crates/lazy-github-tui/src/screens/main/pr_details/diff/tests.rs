//! The Diff tab in the app, against the `pr-details` fixtures: loading, errors, the viewer's keys and footer, and its
//! state across tabs and selections.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{DiffFetchError, DraftReviewComment, FakeGithub, GithubCall, NewReview, ReviewEvent};
use lazy_github_core::models::{IssueListItem, Review, User};
use serde_json::Value;

use crate::backend::tests::VtTerminal;
use crate::components::diff_view::{DiffStop, DiffView};
use crate::effect::{Effect, GithubRequest, GithubResponse};
use crate::event::Event;
use crate::modal::ModalKind;
use crate::screens::main::FocusTarget;
use crate::screens::main::details::{MaximizeScope, SectionKey, SectionRef};
use crate::screens::main::pr_details::tests::{fixture_pr, fixture_repo};
use crate::test_util::{TestApp, logs};

use super::PrDiff;

const NAME: &str = "parity-bot/pr-details";

fn fixture(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-details")
        .join(path);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Pull request `number`'s diff, from the fixtures.
fn diff_text(number: u64) -> String {
    let fixture = fixture(&format!("repos/parity-bot/pr-details/pulls/{number}/GET.diff.json"));
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

/// The fake with the `pr-details` repo, PRs 5, 4, 3 and 9, and their diffs (PR 4's too large, as in the fixtures).
fn gh() -> FakeGithub {
    let page = fixture("repos/parity-bot/pr-details/issues/GET.page-1_per_page-30_state-all.json");
    let repo = fixture_repo();
    let items: Vec<IssueListItem> = page["response"]["body"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect();
    let mut gh = FakeGithub::new()
        .with_repositories(vec![repo])
        .with_issue_page(NAME, 1, Ok(items))
        .with_pull_request_diff(NAME, 4, Err(DiffFetchError::TooLarge));
    for number in [5, 4, 3, 9] {
        gh = gh.with_pull_request(NAME, number, Ok(fixture_pr(number)));
    }
    for number in [5, 3, 9] {
        gh = gh.with_pull_request_diff(NAME, number, Ok(diff_text(number)));
    }
    gh
}

fn config() -> Config {
    let mut config = Config::default();
    config.appearance.show_issues = false;
    config.appearance.show_workflows = false;
    config
}

fn settle(app: &mut TestApp) {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"));
}

/// The app on `gh` and `config` with the PR `keys` pick from the table shown, its Diff tab active and the tab strip
/// focused.
fn diff_tab_with(config: Config, gh: FakeGithub, keys: &[&str]) -> TestApp {
    let mut app = TestApp::with_github(config, gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app.press(keys);
    settle(&mut app);
    app.press(&["right"]);
    app
}

/// PR 5's Diff tab, the tab strip focused.
fn diff_tab() -> TestApp {
    diff_tab_with(config(), gh(), &["enter"])
}

fn pr_diff(app: &TestApp) -> &PrDiff {
    app.app().main_screen().unwrap().details.pull_request().unwrap().diff()
}

fn view(app: &TestApp) -> &DiffView {
    pr_diff(app).view().expect("the diff is in")
}

fn stop(app: &TestApp) -> DiffStop {
    view(app).stop()
}

fn focus(app: &TestApp) -> FocusTarget {
    app.app().main_screen().unwrap().focus()
}

fn footer(app: &mut TestApp) -> String {
    app.screen(160, 48).lines().last().unwrap().to_owned()
}

const README: usize = 0;
const OLD: usize = 3;
const VIEWER: usize = 4;

#[test]
fn the_tab_shows_the_spinner_until_the_diff_arrives() {
    let mut app = TestApp::with_github(config(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    let pr_request = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::PullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.send(Event::Github {
        id: pr_request,
        response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
    });
    // The diff is requested with the pull request, whatever tab shows.
    let diff_request = app.effects().iter().find_map(|effect| match effect {
        Effect::Github {
            id,
            request: GithubRequest::PullRequestDiff { number, .. },
        } => Some((*id, *number)),
        _ => None,
    });
    let (id, number) = diff_request.unwrap();
    assert_eq!(number, 5);
    app.press(&["right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Loading..."), "{screen}");
    assert!(!screen.contains("Filter files"), "{screen}");
    // Nothing inside to focus yet.
    app.press(&["tab", "tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
    app.send(Event::Github {
        id,
        response: GithubResponse::PullRequestDiff(Ok(diff_text(5))),
    });
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Loading..."), "{screen}");
    assert!(
        screen.contains("Filter files (press '/' to focus, ESC to clear)"),
        "{screen}"
    );
}

#[test]
fn each_fetch_error_says_why() {
    let cases = [
        (DiffFetchError::NotFound, "No diff contents found"),
        (DiffFetchError::TooLarge, "Diff too large"),
        (DiffFetchError::Http(502), "Error fetching diff (502)"),
        (DiffFetchError::Gh(GhError::NotInstalled), "Error fetching diff (0)"),
    ];
    for (error, message) in cases {
        let gh = gh().with_pull_request_diff(NAME, 5, Err(error));
        let mut app = diff_tab_with(config(), gh, &["enter"]);
        let screen = app.screen(160, 48);
        assert!(screen.contains(&format!("││{message} ")), "{message}: {screen}");
        assert!(!screen.contains("Filter files"), "{screen}");
    }
}

#[test]
fn pr_diff_error_too_large() {
    let mut app = diff_tab_with(config(), gh(), &["j", "enter"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn pr_diff_parse_error() {
    let gh = gh().with_pull_request_diff(NAME, 5, Ok("diff --git a/x b/x\n--- a/x\n".into()));
    let mut app = diff_tab_with(config(), gh, &["enter"]);
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("error parsing diff - please view on github: "),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn an_empty_diff_shows_only_the_filter() {
    let gh = gh().with_pull_request_diff(NAME, 5, Ok(String::new()));
    let mut app = diff_tab_with(config(), gh, &["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Filter files"), "{screen}");
    assert!(!screen.contains('▼') && !screen.contains('▶'), "{screen}");
}

#[test]
fn pr_diff_files_160x48() {
    let mut app = diff_tab();
    let screen = app.screen(160, 48);
    // In diff order, deleted files closed, every hunk closed, paths as they are (Py bug B55). The test screen writes
    // a cell after each wide character.
    let titles: Vec<&str> = screen
        .lines()
        .filter_map(|line| line.find(['▼', '▶']).map(|at| line[at..].trim_end_matches([' ', '│'])))
        .collect();
    assert_eq!(
        titles,
        [
            "▼ 📄  README.md (+3 -1)",
            "▶ Lines 3-7 (+3 -1) • A tiny pager.",
            "▼ 📄  app.css (+3)",
            "▶ Lines 1-3 (+3)",
            "▼ 📄  build.rs (+1)",
            "▶ Lines 1-4 (+1)",
            "▶ 🗑️   old.md (deleted)",
            "▼ 📄  viewer.rs (+5 -2)",
            "▶ Lines 1-8 (+2)",
            "▶ Lines 22-28 (+3 -2) • impl Viewer {",
        ]
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_diff_hunk_focused_160x48() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "tab", "enter", "tab"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    assert!(
        footer(&mut app).starts_with(
            " j Line down  k Line up  c Add Comment  J Next Hunk  K Previous Hunk  / Filter Files  ^v Paste link"
        ),
        "{}",
        footer(&mut app)
    );
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_diff_hunk_focused_100x30() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "tab", "enter", "tab", "j"]);
    let screen = app.screen(100, 30);
    // The longest line is cut by the pane, over the horizontal scrollbar.
    assert!(
        screen.contains("│     5 │ Run `viewer FILE`, then press `j` and│"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn the_gutter_numbers_follow_the_old_file_for_removed_lines() {
    let mut app = diff_tab();
    app.press(&["tab", "J"]);
    let screen = app.screen(160, 48);
    for row in [
        "│►    3 │ ## Usage ",
        "│     4 │  ",
        "│     5 │ Run `viewer FILE`. ",
        "│     5 │ Run `viewer FILE`, then press `j` and `k` to move. ",
        "│     6 │  ",
        "│     7 │ The current line is marked with `>`. ",
    ] {
        assert!(screen.contains(row), "{row}: {screen}");
    }
}

#[test]
fn j_and_k_move_the_line_and_clamp() {
    let mut app = diff_tab();
    app.press(&["tab", "J"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    app.press(&["k"]);
    assert!(app.screen(160, 48).contains("Line: 1/6"));
    app.press(&["j", "j", "k"]);
    assert!(app.screen(160, 48).contains("Line: 2/6"));
    app.press(&["j", "j", "j", "j", "j", "j"]);
    assert!(app.screen(160, 48).contains("Line: 6/6"));
}

#[test]
fn a_rebound_cursor_key_moves_the_line_and_j_no_longer_does() {
    let mut config = config();
    config.bindings.overrides.insert("diff.cursor_down".into(), "n".into());
    let mut app = diff_tab_with(config, gh(), &["enter"]);
    app.press(&["tab", "J", "j"]);
    assert_eq!(view(&app).pane(README, 0).unwrap().cursor(), 0);
    app.press(&["n"]);
    assert_eq!(view(&app).pane(README, 0).unwrap().cursor(), 1);
    // An overridden key goes after the link's default ones, as in Textual.
    let footer = footer(&mut app);
    assert!(
        footer.starts_with(" k Line up  c Add Comment  n Line down  J Next Hunk"),
        "{footer}"
    );
}

#[test]
fn the_footer_follows_the_focused_stop() {
    let mut app = diff_tab();
    // The scroll area: the viewer's keys work but aren't listed, as in Python.
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, DiffStop::Scroll)
    );
    assert!(footer(&mut app).starts_with(" ^v Paste link"), "{}", footer(&mut app));
    // A title.
    app.press(&["tab", "tab"]);
    assert_eq!(stop(&app), DiffStop::FileTitle(README));
    assert!(
        footer(&mut app).starts_with(" J Next Hunk  K Previous Hunk  / Filter Files  ^v Paste link"),
        "{}",
        footer(&mut app)
    );
    // The filter: its key and the palette, as the table's search box.
    app.press(&["/"]);
    assert_eq!(stop(&app), DiffStop::Filter);
    assert_eq!(footer(&mut app).trim_end(), " / Filter Files  ^p Command Palette");
}

#[test]
fn slash_from_a_pane_focuses_the_filter_and_typing_filters() {
    let mut app = diff_tab();
    app.press(&["tab", "J", "/"]);
    assert_eq!(stop(&app), DiffStop::Filter);
    // Letters are typed, `J` and `j` included, not run.
    app.press(&["V", "J", "j", "backspace", "backspace", "I", "e", "w"]);
    assert_eq!(view(&app).filter_text(), "VIew");
    let screen = app.screen(160, 48);
    // The test screen writes a cell after each wide character.
    assert!(screen.contains("▼ 📄  viewer.rs (+5 -2)"), "{screen}");
    assert!(!screen.contains("README.md"), "{screen}");
    insta::assert_snapshot!("pr_diff_filtered", screen);
    // Escape clears it and leaves it for the scroll area.
    app.press(&["escape"]);
    assert_eq!(
        (focus(&app), stop(&app), view(&app).filter_text()),
        (FocusTarget::DetailsContent, DiffStop::Scroll, "")
    );
    assert!(app.screen(160, 48).contains("README.md"));
}

#[test]
fn enter_in_the_filter_focuses_the_first_shown_hunk() {
    let mut app = diff_tab();
    app.press(&["tab", "/", "v", "i", "e", "w", "enter"]);
    assert_eq!(stop(&app), DiffStop::HunkTitle(VIEWER, 0));
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn a_paste_goes_into_the_focused_filter() {
    let mut app = diff_tab();
    app.press(&["tab", "/"]);
    app.send(Event::Paste("old\nignored".into()));
    assert_eq!(view(&app).filter_text(), "old");
    assert!(view(&app).is_file_shown(OLD));
    assert!(!view(&app).is_file_shown(README));
}

#[test]
fn enter_and_a_click_toggle_sections() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab"]);
    assert_eq!(stop(&app), DiffStop::FileTitle(README));
    app.press(&["enter"]);
    assert!(!view(&app).is_file_open(README));
    app.press(&["enter"]);
    assert!(view(&app).is_file_open(README));
    // A click on the deleted file's title opens it to its notice.
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains("old.md")).unwrap();
    let line = screen.lines().nth(row).unwrap();
    let column = line[..line.find('▶').unwrap()].chars().count();
    app.click(u16::try_from(column).unwrap(), u16::try_from(row).unwrap());
    assert_eq!(stop(&app), DiffStop::FileTitle(OLD));
    assert!(view(&app).is_file_open(OLD));
    let screen = app.screen(160, 48);
    assert!(screen.contains("file was removed"), "{screen}");
    insta::assert_snapshot!("pr_diff_deleted_file_open", screen);
}

#[test]
fn the_tab_keeps_its_state_across_tabs_and_a_new_pr_resets_it() {
    let mut app = diff_tab();
    app.press(&["tab", "J", "j", "j"]);
    app.press(&["5", "left", "right"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    assert!(view(&app).is_hunk_open(README, 0));
    assert_eq!(view(&app).pane(README, 0).unwrap().cursor(), 2);
    assert!(app.screen(160, 48).contains("Line: 3/6"));
    // Selecting the PR again builds the tab afresh.
    app.press(&["2", "enter"]);
    settle(&mut app);
    app.press(&["right"]);
    assert_eq!(stop(&app), DiffStop::Scroll);
    assert!(!view(&app).is_hunk_open(README, 0));
}

#[test]
fn the_deleted_file_row_keeps_its_border_on_a_terminal_and_leaves_nothing_behind() {
    // The `pr-details` parity scenario's frames, each drawn over the last on a terminal that widens `🗑️` (#249):
    // `draw` checks every row against ratatui's buffer. With the reactions open, the Overview has body text where
    // the Diff tab puts the icon's second cell, so ratatui writes that cell again.
    let gh = crate::screens::main::pr_details::tests::gh().with_pull_request_diff(NAME, 5, Ok(diff_text(5)));
    let mut terminal = VtTerminal::new(160, 48);
    let mut app = TestApp::with_github(config(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    terminal.draw(|frame| app.app_mut().render(frame));
    app.press(&["tab", "tab", "enter"]);
    settle(&mut app);
    terminal.draw(|frame| app.app_mut().render(frame));
    app.press(&["5", "right"]);
    settle(&mut app);
    let rows = terminal.draw(|frame| app.app_mut().render(frame));
    let deleted = rows.iter().find(|row| row.contains("old.md (deleted)")).unwrap();
    assert!(
        deleted.ends_with(
            "││  ▶ 🗑️  old.md (deleted)                                                                      │"
        ),
        "{deleted}"
    );
    app.press(&["right"]);
    settle(&mut app);
    let rows = terminal.draw(|frame| app.app_mut().render(frame));
    assert!(
        rows.iter().any(|row| row.contains("Review from parity-bot")),
        "{rows:#?}"
    );
    assert!(rows.iter().all(|row| !row.contains("(deleted)")), "{rows:#?}");
}

// Add Comment and the pending comments (#272).

/// PR 5's Diff tab with README.md's hunk open and its pane focused on line `line` (from 1).
fn readme_pane_at(line: usize) -> TestApp {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "tab", "enter", "tab"]);
    for _ in 1..line {
        app.press(&["j"]);
    }
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    app
}

fn modals(app: &TestApp) -> Vec<ModalKind> {
    app.app().modals()
}

fn toasts(app: &TestApp) -> Vec<String> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

/// Types `text` into whatever has focus: letters, digits and spaces.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        app.press(&[key.as_str()]);
    }
}

/// Adds a comment `body` on the focused pane's current line with `c`.
fn add(app: &mut TestApp, body: &str) {
    app.press(&["c"]);
    type_text(app, body);
    app.press(&["shift+enter"]);
}

fn pending(app: &TestApp) -> Vec<(String, u32, String)> {
    view(app)
        .pending()
        .iter()
        .map(|c| (format!("{}:{}", c.path, c.line_number), c.position, c.body.clone()))
        .collect()
}

#[test]
fn c_opens_add_comment_on_the_current_line() {
    let mut app = readme_pane_at(4);
    app.press(&["c"]);
    assert_eq!(modals(&app), [ModalKind::AddComment]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Commenting on:"), "{screen}");
    assert!(
        screen.contains("+Run `viewer FILE`, then press `j` and `k` to move."),
        "{screen}"
    );
    // The screen's footer stays under the dim: the dialog has none of its own.
    assert!(screen.lines().last().unwrap().contains("c Add Comment"), "{screen}");
}

#[test]
fn pr_review_add_comment_160x48() {
    let mut app = readme_pane_at(4);
    app.press(&["c"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_review_add_comment_100x30() {
    let mut app = readme_pane_at(4);
    app.press(&["c"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn an_empty_comment_toasts_and_the_dialog_stays() {
    let mut app = readme_pane_at(4);
    app.press(&["c", "shift+enter"]);
    assert_eq!(modals(&app), [ModalKind::AddComment]);
    assert_eq!(toasts(&app), ["Comment cannot be empty!"]);
    assert!(view(&app).pending().is_empty());
}

#[test]
fn an_added_comment_is_pending_with_a_toast_and_focus_goes_back_to_the_pane() {
    let mut app = readme_pane_at(4);
    add(&mut app, "Mention the arrow keys");
    assert!(modals(&app).is_empty());
    assert_eq!(toasts(&app), ["New comment added"]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, DiffStop::Pane(README, 0))
    );
    // Line 4 of the hunk is the file's line 5 (B22: Python shows `README.md:3`, the line's index), at diff position 4.
    assert_eq!(
        pending(&app),
        [("README.md:5".to_owned(), 4, "Mention the arrow keys".to_owned())]
    );
}

#[test]
fn a_closed_dialog_adds_nothing() {
    let mut app = readme_pane_at(4);
    app.press(&["c", "x", "escape"]);
    assert!(modals(&app).is_empty());
    assert!(view(&app).pending().is_empty());
    assert!(!app.screen(160, 48).contains("Pending Comments:"));
}

#[test]
fn comments_stay_in_the_order_they_were_added() {
    let mut app = readme_pane_at(1);
    add(&mut app, "first");
    app.press(&["j", "j"]);
    add(&mut app, "second");
    app.press(&["k"]);
    add(&mut app, "third");
    let bodies: Vec<String> = pending(&app).into_iter().map(|(_, _, body)| body).collect();
    assert_eq!(bodies, ["first", "second", "third"]);
}

#[test]
fn the_section_shows_with_the_first_comment_and_hides_with_the_last() {
    let mut app = readme_pane_at(4);
    assert!(!app.screen(160, 48).contains("Pending Comments:"));
    add(&mut app, "only");
    // The Remove button is the ring's last stop.
    app.press(&["5", "tab"]);
    let mut ring = vec![stop(&app)];
    while focus(&app) == FocusTarget::DetailsContent {
        app.press(&["tab"]);
        if focus(&app) == FocusTarget::DetailsContent {
            ring.push(stop(&app));
        }
    }
    assert_eq!(ring.last(), Some(&DiffStop::RemoveComment(0)));
    app.press(&["5", "tab"]);
    while stop(&app) != DiffStop::RemoveComment(0) {
        app.press(&["tab"]);
    }
    let screen = app.screen(160, 48);
    assert!(screen.contains("Pending Comments:"), "{screen}");
    app.press(&["enter"]);
    assert!(view(&app).pending().is_empty());
    assert!(!app.screen(160, 48).contains("Pending Comments:"));
    // Focus went to the stop before the button: src/viewer.rs's second hunk title. No toast for a removal.
    assert_eq!(stop(&app), DiffStop::HunkTitle(VIEWER, 1));
    assert_eq!(toasts(&app), ["New comment added"]);
}

#[test]
fn a_reselected_pr_starts_with_no_pending_comments() {
    let mut app = readme_pane_at(4);
    add(&mut app, "gone");
    assert_eq!(view(&app).pending().len(), 1);
    app.press(&["2", "enter"]);
    settle(&mut app);
    assert!(view(&app).pending().is_empty());
}

// The Submit review form (#273).

/// The fake of [`gh`] logged in as parity-bot, who wrote PR 3 but not PR 5, with PR 5's scenario review answered.
fn reviewer_gh() -> FakeGithub {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/base/user/GET.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let user: User = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
    let created = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/pr-write/repos/parity-bot/pr-details/pulls/5/reviews/POST.json");
    let created: Value = serde_json::from_str(&std::fs::read_to_string(&created).unwrap()).unwrap();
    let created: Review = serde_json::from_value(created["response"]["body"].clone()).unwrap();
    gh().with_user(user).with_created_review(scenario_review(), Ok(created))
}

/// The review the `pr-review` scenario submits.
fn scenario_review() -> NewReview {
    NewReview {
        event: ReviewEvent::Approve,
        commit_id: "c0ffee5555555555555555555555555555555555".into(),
        body: "Looks good.".into(),
        comments: vec![DraftReviewComment {
            path: "README.md".into(),
            body: "Mention the arrow keys too.".into(),
            position: 4,
        }],
    }
}

/// PR 5's Diff tab viewed by parity-bot, with the arrow-keys comment on README.md's line 4 pending, and focus on the
/// review status.
fn reviewing() -> TestApp {
    let mut app = diff_tab_with(config(), reviewer_gh(), &["enter"]);
    settle(&mut app);
    app.press(&["tab", "tab", "tab", "tab", "enter", "tab", "j", "j", "j"]);
    add(&mut app, "Mention the arrow keys too.");
    // From the repos table, back to the end of the screen's ring.
    app.press(&["1", "shift+tab", "shift+tab", "shift+tab"]);
    assert_eq!(stop(&app), DiffStop::ReviewStatus);
    app
}

fn reviews_sent(app: &TestApp) -> Vec<NewReview> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::CreateReview { review, .. } => Some(review),
            _ => None,
        })
        .collect()
}

#[test]
fn the_form_shows_once_the_diff_and_the_user_are_in() {
    let mut app = diff_tab_with(config(), reviewer_gh(), &["enter"]);
    settle(&mut app);
    assert_eq!(view(&app).review_button(), Some("Submit Review"));
    assert_eq!(view(&app).review_status(), Some(Some(ReviewEvent::Comment)));
    // Without the user, no form, as Python shows no viewer then.
    let app = diff_tab();
    assert!(!view(&app).has_review_form());
}

#[test]
fn the_author_s_form_has_only_the_summary_and_add_comments() {
    let mut app = diff_tab_with(config(), reviewer_gh(), &["j", "j", "enter"]);
    settle(&mut app);
    assert_eq!(view(&app).review_button(), Some("Add Comments"));
    assert_eq!(view(&app).review_status(), None);
    // Its last stops: the summary, then the button.
    app.press(&["1", "shift+tab"]);
    assert_eq!(stop(&app), DiffStop::ReviewSubmit);
    app.press(&["shift+tab"]);
    assert_eq!(stop(&app), DiffStop::ReviewSummary);
    app.press(&["shift+tab"]);
    assert_ne!(stop(&app), DiffStop::ReviewStatus);
}

#[test]
fn the_scenario_s_review_is_sent_with_the_comment_s_diff_position() {
    let mut app = reviewing();
    app.press(&["enter", "up", "up", "enter"]);
    assert_eq!(view(&app).review_status(), Some(Some(ReviewEvent::Approve)));
    app.press(&["tab"]);
    type_text(&mut app, "Looks good");
    app.press(&["full_stop", "tab", "enter"]);
    assert!(view(&app).is_submitting());
    settle(&mut app);
    assert_eq!(reviews_sent(&app), [scenario_review()]);
}

#[test]
fn the_blank_status_toasts_and_sends_nothing() {
    let mut app = reviewing();
    // The list's first row is the blank.
    app.press(&["enter", "home", "enter"]);
    assert_eq!(view(&app).review_status(), Some(None));
    app.press(&["tab", "tab", "enter"]);
    settle(&mut app);
    assert!(reviews_sent(&app).is_empty());
    assert_eq!(
        toasts(&app).last().map(String::as_str),
        Some("Please select a status for the new review!")
    );
    assert!(!view(&app).is_submitting());
}

#[test]
fn the_button_is_disabled_while_in_flight_and_a_second_press_is_ignored() {
    let mut app = reviewing();
    app.press(&["tab", "tab", "enter"]);
    assert!(view(&app).is_submitting());
    app.advance(std::time::Duration::from_secs(1));
    app.press(&["enter"]);
    let requests = app
        .effects()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Github {
                    request: GithubRequest::CreateReview { .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(requests, 1);
}

#[test]
fn a_new_review_toasts_and_reloads_the_pr_without_the_pending_comments() {
    let mut app = reviewing();
    app.press(&["enter", "up", "up", "enter", "tab"]);
    type_text(&mut app, "Looks good");
    app.press(&["full_stop", "tab", "enter"]);
    settle(&mut app);
    assert_eq!(toasts(&app).last().map(String::as_str), Some("New review created!"));
    // The pull request was fetched again and shown as a fresh selection: the tab strip has focus, the Overview shows.
    let fetched = app
        .gh()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, GithubCall::PullRequest { number: 5, .. }))
        .count();
    assert_eq!(fetched, 2);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    assert!(app.screen(160, 48).contains("Opened on"));
    settle(&mut app);
    assert!(view(&app).pending().is_empty());
}

#[test]
fn a_refused_review_toasts_and_keeps_everything() {
    let mut app = reviewing();
    // Commented isn't the review the fake answers: it's refused with a 422.
    app.press(&["tab", "tab", "enter"]);
    settle(&mut app);
    assert_eq!(reviews_sent(&app).len(), 1);
    let toast = app.app().toasts().iter().last().unwrap();
    assert_eq!(
        (toast.title.as_deref(), toast.message.to_string().as_str()),
        (Some("Error submitting review"), "Could not submit the review")
    );
    assert_eq!(view(&app).pending().len(), 1);
    assert!(!view(&app).is_submitting());
    assert_eq!(stop(&app), DiffStop::ReviewSubmit);
}

#[test]
fn a_click_on_the_button_submits() {
    let mut app = reviewing();
    app.press(&["tab", "tab"]);
    let screen = app.screen(160, 48);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Submit Review"))
        .unwrap();
    let column = line.chars().position(|c| c == 'S').unwrap();
    app.click(u16::try_from(column).unwrap(), u16::try_from(row).unwrap());
    settle(&mut app);
    assert_eq!(reviews_sent(&app).len(), 1);
}

#[test]
fn pr_review_form_reviewer() {
    let mut app = reviewing();
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_review_form_author() {
    let mut app = diff_tab_with(config(), reviewer_gh(), &["j", "j", "enter"]);
    settle(&mut app);
    app.press(&["1", "shift+tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

// M in the Diff tab (#288).

fn section(section: SectionRef) -> MaximizeScope {
    MaximizeScope::Section(SectionKey(section))
}

fn maximized_section(app: &TestApp) -> Option<SectionRef> {
    app.app()
        .maximized()
        .and_then(|maximized| maximized.section)
        .map(|key| key.0)
}

fn tab_to(app: &mut TestApp, wanted: DiffStop) {
    for _ in 0..40 {
        app.press(&["tab"]);
        if stop(app) == wanted {
            return;
        }
    }
    panic!("never reached {wanted:?}");
}

#[test]
fn maximize_scope_is_the_section_of_each_title_or_pane() {
    let mut app = diff_tab();
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::Scroll);
    assert_eq!(pr_diff(&app).maximize_scope(), MaximizeScope::Nothing);
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::Filter);
    assert_eq!(pr_diff(&app).maximize_scope(), MaximizeScope::Nothing);
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::FileTitle(README));
    assert_eq!(pr_diff(&app).maximize_scope(), section(SectionRef::File(README)));
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::HunkTitle(README, 0));
    assert_eq!(pr_diff(&app).maximize_scope(), section(SectionRef::Hunk(README, 0)));
    app.press(&["enter", "tab"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    assert_eq!(pr_diff(&app).maximize_scope(), section(SectionRef::Hunk(README, 0)));
    // A deleted file's title.
    tab_to(&mut app, DiffStop::FileTitle(OLD));
    assert_eq!(pr_diff(&app).maximize_scope(), section(SectionRef::File(OLD)));
}

#[test]
fn m_in_the_filter_types_it() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "M"]);
    assert_eq!(view(&app).filter_text(), "M");
    assert_eq!(app.app().maximized(), None);
}

#[test]
fn keys_inside_a_maximized_file_stay_inside_it() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "M"]);
    assert_eq!(maximized_section(&app), Some(SectionRef::File(README)));
    // Its title and its hunk's, wrapping.
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::HunkTitle(README, 0));
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::FileTitle(README));
    // Enter on the hunk's title opens its pane, which j moves in.
    app.press(&["tab", "enter"]);
    assert!(view(&app).is_hunk_open(README, 0));
    app.press(&["tab", "j"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    assert_eq!(view(&app).pane(README, 0).unwrap().cursor(), 1);
    assert!(app.screen(160, 48).contains("Line: 2/6"));
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::FileTitle(README));
    app.press(&["shift+tab"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    assert_eq!(maximized_section(&app), Some(SectionRef::File(README)));
    app.press(&["escape"]);
    assert_eq!(maximized_section(&app), None);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
}

#[test]
fn j_and_k_move_the_line_in_a_maximized_hunk_pane() {
    let mut app = readme_pane_at(1);
    app.press(&["M"]);
    assert_eq!(maximized_section(&app), Some(SectionRef::Hunk(README, 0)));
    app.press(&["j", "j", "k"]);
    assert_eq!(view(&app).pane(README, 0).unwrap().cursor(), 1);
    assert!(app.screen(160, 48).contains("Line: 2/6"));
    // Its title and its pane, wrapping.
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::HunkTitle(README, 0));
    app.press(&["tab"]);
    assert_eq!(stop(&app), DiffStop::Pane(README, 0));
    // The pane keeps its line once restored.
    app.press(&["escape"]);
    assert!(app.screen(160, 48).contains("Line: 2/6"));
}

#[test]
fn a_click_on_a_maximized_file_hits_its_own_layout() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "M"]);
    let row = u16::try_from(
        app.screen(160, 48)
            .lines()
            .position(|line| line.contains("▶ Lines 3-7"))
            .unwrap(),
    )
    .unwrap();
    app.click(8, row);
    assert_eq!(stop(&app), DiffStop::HunkTitle(README, 0));
    assert!(view(&app).is_hunk_open(README, 0));
    assert_eq!(maximized_section(&app), Some(SectionRef::File(README)));
}

#[test]
fn pr_diff_file_maximized_160x48() {
    let mut app = diff_tab();
    app.press(&["tab", "tab", "tab", "M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(160, 48);
    assert!(screen.contains("  ▼ 📄  README.md (+3 -1)"), "{screen}");
    assert!(screen.contains("      ▶ Lines 3-7 (+3 -1) • A tiny pager."), "{screen}");
    assert!(!screen.contains("app.css"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_diff_hunk_pane_maximized_160x48() {
    let mut app = readme_pane_at(1);
    app.press(&["M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(160, 48);
    assert!(screen.contains("  ▼ Lines 3-7 (+3 -1) • A tiny pager."), "{screen}");
    assert!(screen.contains("Line: 1/6"), "{screen}");
    assert!(!screen.contains("README.md"), "{screen}");
    insta::assert_snapshot!(screen);
}
