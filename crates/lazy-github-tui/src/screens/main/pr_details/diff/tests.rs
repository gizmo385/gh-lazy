//! The Diff tab in the app, against the `pr-details` fixtures: loading, errors, the viewer's keys and footer, and its
//! state across tabs and selections.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::{DiffFetchError, DraftReviewComment, FakeGithub, GithubCall, NewReview, ReviewEvent};
use lazy_github_core::models::{FullPullRequest, IssueListItem, Review, User};
use serde_json::Value;

use crate::backend::tests::VtTerminal;
use crate::components::diff_view::{DiffStop, DiffView};
use crossterm::event::{MouseButton, MouseEventKind};

use crate::effect::{Effect, GithubRequest, GithubResponse};
use crate::event::Event;
use crate::modal::{CommandPaletteDialog, ModalKind};
use crate::screens::main::details::{MaximizeScope, SectionKey, SectionRef};
use crate::screens::main::pr_details::tests::{fixture_pr, fixture_repo};
use crate::screens::main::{FocusTarget, Maximized};
use crate::test_util::{TestApp, UNICODE_EXTRAS, logs, unicode_fixture};

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

/// [`gh`] with the `unicode` fixture set's PR titles, PR 5 and PR 5's diff (#327), plus what the parity scenario
/// leaves out: PR 5's title and a diff line also hold [`UNICODE_EXTRAS`], and the README hunk gains a full-width line
/// too long for the hunk pane.
fn unicode_gh() -> FakeGithub {
    let repo = fixture_repo();
    let items: Vec<IssueListItem> = unicode_fixture(&format!("{ISSUES}/GET.page-1_per_page-30_state-all.json"))
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if item["number"] == 5 {
                item["title"] = with_extras(&item["title"]);
            }
            IssueListItem::from_api_item(item, &repo).unwrap()
        })
        .collect();
    let mut pr = unicode_fixture("repos/parity-bot/pr-details/pulls/5/GET.json");
    pr["title"] = with_extras(&pr["title"]);
    let pr = FullPullRequest::from_api(pr, &repo).unwrap();
    let diff = unicode_fixture("repos/parity-bot/pr-details/pulls/5/GET.diff.json")
        .as_str()
        .unwrap()
        .replace("@@ -1,4 +1,7 @@", "@@ -1,4 +1,9 @@")
        .replace(
            " 終わり\n",
            &format!(
                "+全角文字の行がとても長い場合でも、ペインの幅で文字が半分に切れてはいけません。ここから先は切り取られます。\n\
                 +ZWJ and controls:{UNICODE_EXTRAS}\n 終わり\n"
            ),
        );
    gh().with_issue_page(NAME, 1, Ok(items))
        .with_pull_request(NAME, 5, Ok(pr))
        .with_pull_request_diff(NAME, 5, Ok(diff))
}

const ISSUES: &str = "repos/parity-bot/pr-details/issues";

fn with_extras(title: &Value) -> Value {
    format!("{}{UNICODE_EXTRAS}", title.as_str().unwrap()).into()
}

#[test]
fn pr_table_and_overview_unicode_160x48() {
    let mut app = TestApp::with_github(config(), unicode_gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2", "enter"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    // Control characters are drawn as U+FFFD, and nothing is drawn raw.
    assert!(screen.contains("�[31mred�[0m�2J�� (#5)"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_diff_unicode_160x48() {
    let mut app = diff_tab_with(config(), unicode_gh(), &["enter"]);
    // The scroll area, the filter, README.md's title, its hunk's title; enter opens the hunk.
    app.press(&["tab", "tab", "tab", "tab", "enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Line: 1/10"), "{screen}");
    insta::assert_snapshot!(screen);
}

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
        (
            DiffFetchError::from(GhError::Http {
                status: 502,
                message: String::new(),
                body: Vec::new(),
                headers: Vec::new(),
            }),
            "Error fetching diff (502)",
        ),
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

/// The screen row and column where `text` is first drawn.
fn find(screen: &str, text: &str) -> (u16, u16) {
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(text))
        .unwrap_or_else(|| panic!("{text:?} isn't on screen:\n{screen}"));
    let column = line[..line.find(text).unwrap()].chars().count();
    (u16::try_from(column).unwrap(), u16::try_from(row).unwrap())
}

#[test]
fn a_drag_from_the_filter_keeps_the_mouse_until_the_release() {
    let mut app = diff_tab();
    app.press(&["tab", "/"]);
    app.send(Event::Paste("foo-bar baz".into()));
    let (column, row) = find(&app.screen(160, 48), "foo-bar baz");
    let bar = column + 4;
    // Pressed on the `b` of `bar`, dragged left out of the details over the Pull Requests table and back right over
    // the diff below the filter, and released there: `bar` is selected, and the release clicks nothing.
    app.drag((bar, row), &[(10, 20), (bar + 3, row + 6)]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, DiffStop::Filter)
    );
    app.press(&["x"]);
    assert_eq!(view(&app).filter_text(), "foo-x baz");
    // The release ended the drag: moves with the button down select nothing until the next press.
    app.mouse(MouseEventKind::Drag(MouseButton::Left), column, row);
    app.press(&["y"]);
    assert_eq!(view(&app).filter_text(), "foo-xy baz");
    // A drag past the filter's right edge selects to the end.
    app.render(160, 48);
    app.drag((bar, row), &[(159, 2)]);
    app.press(&["z"]);
    assert_eq!(view(&app).filter_text(), "foo-z");
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
    reviewing_with(config())
}

/// [`reviewing`] on `config`.
fn reviewing_with(config: Config) -> TestApp {
    let mut app = diff_tab_with(config, reviewer_gh(), &["enter"]);
    settle(&mut app);
    app.press(&["tab", "tab", "tab", "tab", "enter", "tab", "j", "j", "j"]);
    add(&mut app, "Mention the arrow keys too.");
    // From the repos table, back to the end of the screen's ring (past the command log, when it's shown).
    app.press(&["1"]);
    for _ in 0..4 {
        app.press(&["shift+tab"]);
        if focus(&app) == FocusTarget::DetailsContent && stop(&app) == DiffStop::ReviewStatus {
            break;
        }
    }
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
fn a_drag_from_the_summary_selects_wherever_the_pointer_goes() {
    let mut app = diff_tab_with(config(), reviewer_gh(), &["j", "j", "enter"]);
    settle(&mut app);
    app.press(&["1", "shift+tab", "shift+tab"]);
    assert_eq!(stop(&app), DiffStop::ReviewSummary);
    app.send(Event::Paste("foo-bar baz".into()));
    let (column, row) = find(&app.screen(160, 48), "foo-bar baz");
    let bar = column + 4;
    // Out over the tables and back, released below the summary.
    app.drag((bar, row), &[(10, 20), (bar + 3, row + 3)]);
    assert_eq!(stop(&app), DiffStop::ReviewSummary);
    type_text(&mut app, "x");
    assert_eq!(view(&app).review_summary(), Some("foo-x baz"));
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

// The review status's open list near the screen's bottom (#364).

/// [`reviewing_with`] at 160x48, scrolled up three notches as the `pr-review` scenario does, with the review status's
/// list open: it doesn't fit below the box, so it's moved up until its bottom border is on the footer row.
fn status_list_near_bottom(config: Config) -> TestApp {
    let mut app = reviewing_with(config);
    // The comment's toast would cover part of the list.
    app.advance(std::time::Duration::from_secs(30));
    open_near_bottom(&mut app);
    app
}

/// Scrolls the focused review status's tab up three notches at 160x48 and opens the list.
fn open_near_bottom(app: &mut TestApp) {
    app.render(160, 48);
    for _ in 0..3 {
        app.mouse(MouseEventKind::ScrollUp, 110, 30);
        app.render(160, 48);
    }
    app.press(&["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Changes Requested"), "{screen}");
}

/// The open list's cells, from the last frame: from its left edge `▊` to its right edge `▎` on the footer row, and up
/// six rows (the border and the four rows).
fn status_list_cells(app: &mut TestApp) -> (std::ops::RangeInclusive<u16>, std::ops::RangeInclusive<u16>) {
    let screen = app.screen(160, 48);
    let last = screen.lines().last().unwrap();
    let left = last
        .chars()
        .position(|c| c == '▊')
        .expect("the list's bottom-left corner");
    let right = last
        .chars()
        .position(|c| c == '▎')
        .expect("the list's bottom-right corner");
    let columns = u16::try_from(left).unwrap()..=u16::try_from(right).unwrap();
    (columns, 42..=47)
}

fn status_open(app: &TestApp) -> bool {
    view(app).status_open()
}

#[test]
fn pr_diff_status_list_over_footer() {
    let mut app = status_list_near_bottom(config());
    let screen = app.screen(160, 48);
    let last = screen.lines().last().unwrap();
    assert!(last.starts_with(" esc Dismiss menu"), "{last}");
    assert!(last.contains("▊▁▁▁") && last.ends_with("▁▎"), "{last}");
    insta::assert_snapshot!(screen);
}

#[test]
fn pr_diff_status_list_over_command_log() {
    let mut config = config();
    config.appearance.show_command_log = true;
    let mut app = status_list_near_bottom(config);
    let screen = app.screen(160, 48);
    // The log's box is drawn first, so its border doesn't cut through the list.
    for option in ["▊  Select", "▊  Approved", "▊  Changes Requested", "▊  Commented"] {
        assert!(screen.contains(option), "{option}: {screen}");
    }
    insta::assert_snapshot!(screen);
}

#[test]
fn a_press_on_the_list_on_the_footer_row_runs_no_footer_binding() {
    let mut app = status_list_near_bottom(config());
    let (columns, _) = status_list_cells(&mut app);
    // Inside the list's bottom border, under where the footer's `^p Command Palette` item would reach.
    app.click(*columns.start() + 1, 47);
    assert!(modals(&app).is_empty(), "{:?}", modals(&app));
    assert!(status_open(&app));
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(stop(&app), DiffStop::ReviewStatus);
}

#[test]
fn a_press_on_the_option_over_the_details_border_picks_it() {
    let mut app = reviewing();
    // Approved first, so the pick shows.
    app.press(&["enter", "up", "up", "enter"]);
    assert_eq!(view(&app).review_status(), Some(Some(ReviewEvent::Approve)));
    open_near_bottom(&mut app);
    let screen = app.screen(160, 48);
    let row = screen.lines().nth(46).unwrap();
    assert!(row.contains("▊  Commented") && row.starts_with('└'), "{row}");
    app.click(70, 46);
    assert!(!status_open(&app));
    assert_eq!(view(&app).review_status(), Some(Some(ReviewEvent::Comment)));
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(stop(&app), DiffStop::ReviewStatus);
}

#[test]
fn toasts_still_draw_over_the_open_list() {
    let mut app = reviewing();
    open_near_bottom(&mut app);
    let screen = app.screen(160, 48);
    let row = screen
        .lines()
        .find(|line| line.contains("▊  Changes Requested"))
        .expect("the list's row");
    assert!(row.contains("New comment added"), "{screen}");
}

#[test]
fn with_the_list_closed_the_footer_row_press_runs_the_footer_item() {
    let mut app = status_list_near_bottom(config());
    let (columns, _) = status_list_cells(&mut app);
    app.press(&["escape"]);
    assert!(!status_open(&app));
    let screen = app.screen(160, 48);
    let last = screen.lines().last().unwrap();
    let palette = last.find("^p Command").expect("the palette's footer item");
    let column = u16::try_from(last[..palette].chars().count()).unwrap();
    // The footer's items changed with the focus; the row the list covered is the footer's again.
    let main = app.app().main_screen().unwrap();
    assert!(columns.clone().all(|column| !main.popup_contains(column, 47)));
    app.click(column + 1, 47);
    assert_eq!(modals(&app), [ModalKind::CommandPalette]);
}

#[test]
fn the_screen_s_popup_is_the_open_list_s_cells() {
    let mut app = status_list_near_bottom(config());
    let (columns, rows) = status_list_cells(&mut app);
    let main = app.app().main_screen().unwrap();
    for row in rows.clone() {
        for column in columns.clone() {
            assert!(main.popup_contains(column, row), "({column}, {row})");
        }
        assert!(!main.popup_contains(*columns.start() - 1, row));
        assert!(!main.popup_contains(*columns.end() + 1, row));
    }
    for column in columns.clone() {
        assert!(!main.popup_contains(column, *rows.start() - 1));
    }
    app.press(&["escape"]);
    app.render(160, 48);
    let main = app.app().main_screen().unwrap();
    for row in rows {
        for column in columns.clone() {
            assert!(!main.popup_contains(column, row), "({column}, {row})");
        }
    }
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
    assert_eq!(pr_diff(&app).maximize_scope(), MaximizeScope::TabArea);
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

// Maximize from the stops outside any file or hunk (#295).

/// The details tab area, maximized from the Diff content.
fn tab_area() -> Maximized {
    Maximized::widget(FocusTarget::DetailsContent)
}

/// How many "View Maximized" toasts are up.
fn maximize_toasts(app: &TestApp) -> usize {
    app.app()
        .toasts()
        .iter()
        .filter(|toast| toast.title.as_deref() == Some("View Maximized"))
        .count()
}

/// [`config`] with maximize rebound to `f6`, a key neither input types.
fn f6_config() -> Config {
    let mut config = config();
    config
        .bindings
        .overrides
        .insert("app.maximize_widget".into(), "f6".into());
    config
}

#[test]
fn a_rebound_maximize_key_on_the_filter_maximizes_the_tab_area_and_m_still_types() {
    let mut app = diff_tab_with(f6_config(), gh(), &["enter"]);
    app.press(&["tab", "tab", "r", "e"]);
    assert_eq!(stop(&app), DiffStop::Filter);
    app.press(&["f6"]);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert_eq!(maximize_toasts(&app), 1);
    assert_eq!(view(&app).filter_text(), "re");
    assert_eq!(stop(&app), DiffStop::Filter);
    app.press(&["M"]);
    assert_eq!(view(&app).filter_text(), "reM");
    // Escape restores first; the filter keeps its focus and text, and a second one is the filter's own.
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(stop(&app), DiffStop::Filter);
    assert_eq!(view(&app).filter_text(), "reM");
    app.press(&["escape"]);
    assert_eq!(view(&app).filter_text(), "");
    assert_eq!(stop(&app), DiffStop::Scroll);
}

#[test]
fn a_rebound_maximize_key_on_the_summary_maximizes_the_tab_area_and_m_still_types() {
    let mut app = reviewing_with(f6_config());
    app.press(&["tab", "o", "k"]);
    assert_eq!(stop(&app), DiffStop::ReviewSummary);
    app.press(&["f6"]);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert_eq!(maximize_toasts(&app), 1);
    assert_eq!(view(&app).review_summary(), Some("ok"));
    app.press(&["M"]);
    assert_eq!(view(&app).review_summary(), Some("okM"));
    assert_eq!(app.app().maximized(), Some(tab_area()));
}

#[test]
fn m_on_a_remove_button_maximizes_the_tab_area_and_escape_keeps_its_focus() {
    let mut app = reviewing();
    app.press(&["shift+tab"]);
    assert_eq!(stop(&app), DiffStop::RemoveComment(0));
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert_eq!(maximize_toasts(&app), 1);
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(stop(&app), DiffStop::RemoveComment(0));
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(pending(&app).len(), 1);
}

#[test]
fn m_on_the_review_status_and_submit_maximizes_the_tab_area() {
    let mut app = reviewing();
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert_eq!(stop(&app), DiffStop::ReviewStatus);
    app.press(&["escape", "tab", "tab", "M"]);
    assert_eq!(stop(&app), DiffStop::ReviewSubmit);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert!(reviews_sent(&app).is_empty());
}

#[test]
fn the_palette_offers_maximize_on_the_filter_and_runs_it_without_the_toast() {
    let mut app = diff_tab();
    app.press(&["tab", "tab"]);
    assert_eq!(stop(&app), DiffStop::Filter);
    app.press(&["ctrl+p"]);
    let palette = app
        .app()
        .top_dialog::<CommandPaletteDialog>()
        .expect("the palette is open");
    assert!(palette.hit_names().contains(&"Maximize"), "{:?}", palette.hit_names());
    type_text(&mut app, "max");
    app.press(&["enter"]);
    assert_eq!(app.app().maximized(), Some(tab_area()));
    assert_eq!(maximize_toasts(&app), 0);
    assert_eq!(stop(&app), DiffStop::Filter);
    assert_eq!(view(&app).filter_text(), "");
}

#[test]
fn pr_diff_tab_area_maximized_from_submit_160x48() {
    let mut app = reviewing();
    app.press(&["tab", "tab", "M"]);
    assert_eq!(stop(&app), DiffStop::ReviewSubmit);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(160, 48);
    // The form sits at the bottom of the diff, so a maximize that scrolled back to the top would lose it.
    assert!(screen.contains("Submit Review"), "{screen}");
    assert!(screen.contains("Overview  Diff  Conversation"), "{screen}");
    assert!(!screen.contains("[1] Repositories"), "{screen}");
    insta::assert_snapshot!(screen);
}

// `gh` failing mid-session (#323): the diff, the viewer's user and the review.

mod gh_failures {
    use super::*;
    use crate::action::Action;
    use crate::screens::main::MainAction;
    use crate::screens::main::pr_details::PullRequestTarget;
    use crate::test_util::{assert_gh_failures_surface, gh_failures};

    /// The PR table focused on PR 5, nothing shown yet.
    fn pr_table(gh: FakeGithub) -> TestApp {
        let mut app = TestApp::with_github(config(), gh);
        app.press(&["enter"]);
        settle(&mut app);
        app.press(&["2"]);
        app
    }

    fn show_pr_5(app: &mut TestApp) {
        app.dispatch(Action::Main(MainAction::OpenPullRequest {
            target: PullRequestTarget::Full(Box::new(fixture_pr(5))),
            focus: true,
        }));
    }

    #[test]
    fn the_diff_says_why_in_the_tab() {
        for (name, error) in gh_failures() {
            let mut app = pr_table(reviewer_gh());
            app.gh().fail_from_now(error);
            show_pr_5(&mut app);
            settle(&mut app);
            app.press(&["right"]);
            let screen = app.screen(160, 48);
            assert!(screen.contains("││Error fetching diff ("), "{name}: {screen}");
            assert!(!screen.contains("Loading"), "{name}: {screen}");
            // Nothing else is asked for: the form needs the diff.
            assert!(!app.gh().calls().contains(&GithubCall::CurrentUser), "{name}");
        }
    }

    #[test]
    fn without_the_viewer_the_diff_shows_with_no_form() {
        for (name, error) in gh_failures() {
            let mut app = pr_table(gh().with_user_error(error));
            show_pr_5(&mut app);
            let out = logs(|| futures_util::FutureExt::now_or_never(app.settle()).unwrap());
            assert!(app.gh().calls().contains(&GithubCall::CurrentUser), "{name}");
            assert!(
                out.contains("ERROR Error fetching the current user for the review form"),
                "{name}: {out}"
            );
            app.press(&["right"]);
            let screen = app.screen(160, 48);
            assert!(screen.contains("Filter files"), "{name}: {screen}");
            assert_eq!(view(&app).review_button(), None, "{name}");
        }
    }

    #[test]
    fn submitting_a_review() {
        assert_gh_failures_surface(
            reviewing,
            |app| {
                app.press(&["tab", "tab", "enter"]);
            },
            |app| {
                if view(app).is_submitting() {
                    return Err("still submitting".into());
                }
                if view(app).pending().len() != 1 {
                    return Err("the pending comment is gone".into());
                }
                // The button works again.
                app.press(&["enter"]);
                if view(app).is_submitting() {
                    Ok(())
                } else {
                    Err("the button is stuck".into())
                }
            },
        );
    }
}
