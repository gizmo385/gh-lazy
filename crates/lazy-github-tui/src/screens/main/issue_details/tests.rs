//! Selecting an issue, its tabs and its Overview in the app, against the `issue-details` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{Issue, IssueComment, IssueListItem, Repository};
use serde_json::Value;

use crate::action::Action;
use crate::app::Maximized;
use crate::component::Owner;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::{FocusTarget, MainAction, SectionId};
use crate::test_util::{TestApp, UNICODE_EXTRAS, logs, unicode_fixture};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/issue-details")
}

/// The response body of the fixture at `path` in the `issue-details` set.
fn fixture_body(path: &str) -> Value {
    let path = fixtures().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot/issue-fixture`, first on the set's repos page.
fn fixture_repo() -> Repository {
    let page = fixture_body("user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json");
    serde_json::from_value(page[0].clone()).unwrap()
}

/// The repo's first issues page: issues 12 (open, with a body), 11 (closed, no body), 10 and 9, and PR 8.
fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body("repos/parity-bot/issue-fixture/issues/GET.page-1_per_page-30_state-all.json");
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

/// Issue `number` of `parity-bot/issue-fixture`, as the Issues table holds it.
pub(in crate::screens::main) fn fixture_issue(number: u64) -> Issue {
    issues_page()
        .into_iter()
        .find_map(|item| match item {
            IssueListItem::Issue(issue) if issue.number == number => Some(issue),
            _ => None,
        })
        .unwrap()
}

/// Issue `number`'s comments, from the set's fixture.
fn fixture_comments(number: u64) -> Vec<IssueComment> {
    serde_json::from_value(fixture_body(&format!(
        "repos/parity-bot/issue-fixture/issues/{number}/comments/GET.json"
    )))
    .unwrap()
}

fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page("parity-bot/issue-fixture", 1, Ok(issues_page()))
        .with_issue_comments("parity-bot/issue-fixture", 12, Ok(fixture_comments(12)))
        .with_issue_comments("parity-bot/issue-fixture", 10, Ok(fixture_comments(10)))
        .with_readme(
            "parity-bot/issue-fixture",
            Ok(Some("Issues with bodies and comments.\n".into())),
        )
}

fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The app with `parity-bot/issue-fixture` selected and its issues listed, the Issues table focused on issue 12.
fn app() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3"]);
    app
}

/// [`app`] with issue `12` selected after `moves` cursor moves down.
fn selected(moves: usize) -> TestApp {
    let mut app = app();
    for _ in 0..moves {
        app.press(&["j"]);
    }
    app.press(&["enter"]);
    settle(&mut app);
    app
}

fn main(app: &TestApp) -> &crate::screens::main::MainScreen {
    app.app().main_screen().unwrap()
}

fn focus(app: &TestApp) -> FocusTarget {
    main(app).focus()
}

fn title(app: &TestApp) -> &str {
    main(app).details_title()
}

fn tabs(app: &TestApp) -> (Vec<String>, usize) {
    let main = main(app);
    (main.details_tabs().to_vec(), main.details.tabs().active())
}

fn shown_issue(app: &TestApp) -> Option<u64> {
    main(app).details_issue().map(|issue| issue.number)
}

fn issues_cursor(app: &TestApp) -> Option<u64> {
    main(app)
        .issue_lists
        .issues_table()
        .selected()
        .map(|issue| issue.number)
}

fn footer(app: &mut TestApp) -> String {
    app.screen(160, 48).lines().last().unwrap().to_owned()
}

/// The row of the screen whose text contains `needle`.
fn row_of(app: &mut TestApp, needle: &str) -> u16 {
    let screen = app.screen(160, 48);
    let row = screen.lines().position(|line| line.contains(needle)).unwrap();
    u16::try_from(row).unwrap()
}

#[test]
fn enter_space_and_a_click_each_select_the_issue() {
    let mut app = app();
    let out = logs(|| {
        app.press(&["enter"]);
    });
    assert!(out.contains("INFO Selected Issue: #12"), "{out}");
    assert_eq!(shown_issue(&app), Some(12));

    let mut app = self::app();
    app.press(&["j", "space"]);
    assert_eq!(shown_issue(&app), Some(11));

    // A click on the cursor's cell: the press focuses the table, the release selects. Elsewhere on the row it only
    // moves the cursor.
    let mut app = self::app();
    let row = row_of(&mut app, "│ 12  open");
    app.click(6, row);
    assert_eq!(shown_issue(&app), None);
    app.render(160, 48);
    app.click(2, row);
    assert_eq!(shown_issue(&app), None);
    app.render(160, 48);
    let out = logs(|| {
        app.click(2, row);
    });
    assert!(out.contains("INFO Selected Issue: #12"), "{out}");
    assert_eq!(shown_issue(&app), Some(12));
    assert!(!out.contains("ERROR"), "{out}");
}

#[test]
fn selecting_shows_the_two_tabs_and_focuses_the_strip() {
    let mut app = selected(0);
    assert_eq!(title(&app), "[5] Issue #12 Details");
    assert_eq!(tabs(&app), (vec!["Overview".into(), "Comments".into()], 0));
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    // The Issues table's cursor doesn't move.
    assert_eq!(issues_cursor(&app), Some(12));
    let screen = app.screen(160, 48);
    assert!(screen.contains("┌─ [5] Issue #12 Details ─"), "{screen}");
    assert!(screen.contains("│ Overview  Comments "), "{screen}");
    assert!(
        screen.contains("│Open Crash when the config file is empty (#12) by parity-friend "),
        "{screen}"
    );
}

#[test]
fn the_title_key_follows_the_overrides() {
    let mut config = Config::default();
    config
        .bindings
        .overrides
        .insert("main.details.focus".into(), "9".into());
    let mut app = TestApp::with_github(config, gh());
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    assert_eq!(title(&app), "[9] Issue #12 Details");
}

#[test]
fn selecting_the_same_issue_again_rebuilds_the_tabs() {
    let mut app = selected(0);
    app.render(100, 20);
    app.press(&["tab", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.issue().unwrap().overview().offset();
    assert_eq!(offset(&app), 2);
    app.press(&["5", "right"]);
    assert_eq!(tabs(&app).1, 1);
    app.press(&["3", "enter"]);
    assert_eq!(tabs(&app).1, 0);
    assert_eq!(offset(&app), 0);
    assert_eq!(shown_issue(&app), Some(12));
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn a_repo_selection_replaces_the_issue_tabs() {
    let mut app = selected(0);
    app.press(&["tab"]);
    app.press(&["1", "enter"]);
    assert_eq!(tabs(&app), (vec!["Repo Overview".into()], 0));
    assert_eq!(shown_issue(&app), None);
    assert_eq!(title(&app), "[5] parity-bot/issue-fixture Details");
}

#[test]
fn the_open_issue_action_shows_an_issue_at_once() {
    let mut app = app();
    app.dispatch(Action::Main(MainAction::OpenIssue(Box::new(fixture_issue(10)))));
    assert_eq!(shown_issue(&app), Some(10));
    assert_eq!(title(&app), "[5] Issue #10 Details");
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn an_issue_selection_cancels_a_pending_pull_request() {
    let mut app = app();
    app.press(&["2", "enter"]);
    let pending = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            crate::effect::Effect::Github {
                id,
                request: crate::effect::GithubRequest::PullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.press(&["3", "enter"]);
    assert!(app.effects().contains(&crate::effect::Effect::Cancel(pending)));
    assert_eq!(shown_issue(&app), Some(12));
}

#[test]
fn edit_shows_only_with_the_body_focused_and_opens_the_dialog() {
    let mut app = selected(0);
    assert!(!footer(&mut app).contains("Edit Issue"));
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    let line = footer(&mut app);
    assert!(
        line.starts_with(" E Edit Issue  ^v Paste link  f Follow link"),
        "{line}"
    );
    app.press(&["E"]);
    assert_eq!(app.app().modals(), [crate::modal::ModalKind::EditIssue]);
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    // `tab` again leaves the body: there are no stops inside the Overview.
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
    // The Comments tab's body doesn't have the Overview's binding.
    app.press(&["5", "right", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert!(!footer(&mut app).contains("Edit Issue"));
}

#[test]
fn switching_tabs_keeps_the_overview_scroll_position() {
    let mut app = selected(0);
    app.render(100, 20);
    app.press(&["tab", "down", "down", "down"]);
    let offset = |app: &TestApp| main(app).details.issue().unwrap().overview().offset();
    assert_eq!(offset(&app), 3);
    app.press(&["5", "right", "left"]);
    app.render(100, 20);
    assert_eq!(offset(&app), 3);
}

#[test]
fn maximize_works_on_the_strip_but_not_the_body() {
    let mut app = selected(0);
    app.press(&["tab", "M"]);
    assert_eq!(app.app().maximized(), None);
    app.press(&["5", "M"]);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(FocusTarget::DetailsTabs)));
}

#[test]
fn the_issues_section_cursor_stays_when_focus_returns() {
    let mut app = selected(1);
    app.press(&["3"]);
    assert_eq!(focus(&app), FocusTarget::Table(SectionId::Issues));
    assert_eq!(issues_cursor(&app), Some(11));
}

#[test]
fn issue_overview_160x48() {
    let mut app = selected(0);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_100x30() {
    let mut app = selected(0);
    insta::assert_snapshot!(app.screen(100, 30));
}

/// Issue 12 titled and described with the `unicode` fixture set's PR 5 title and body (#327), its title also holding
/// what the parity scenario leaves out: a ZWJ sequence and control characters.
#[test]
fn issue_overview_unicode_160x48() {
    let pr = unicode_fixture("repos/parity-bot/pr-details/pulls/5/GET.json");
    let repo = fixture_repo();
    let page = fixture_body("repos/parity-bot/issue-fixture/issues/GET.page-1_per_page-30_state-all.json");
    let items = page
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if item["number"] == 12 {
                item["title"] = format!("{}{UNICODE_EXTRAS}", pr["title"].as_str().unwrap()).into();
                item["body"] = pr["body"].clone();
            }
            IssueListItem::from_api_item(item, &repo).unwrap()
        })
        .collect();
    let mut app = TestApp::with_github(
        Config::default(),
        gh().with_issue_page("parity-bot/issue-fixture", 1, Ok(items)),
    );
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    settle(&mut app);
    let screen = app.screen(160, 48);
    assert!(screen.contains("�[31mred�[0m�2J��"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn issue_overview_closed_no_body() {
    let mut app = selected(1);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_body_focused() {
    let mut app = selected(0);
    app.press(&["tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_overview_scrolled_to_the_end_100x30() {
    // Issue 12's body just fits at 100x30: a longer one overflows, with a scrollbar.
    let mut issue = fixture_issue(12);
    let body = issue.body.take().unwrap();
    issue.body = Some(format!("{body}\n{}\nThe last line.\n", "More detail.\n\n".repeat(6)));
    let mut app = app();
    app.dispatch(Action::Main(MainAction::OpenIssue(Box::new(issue))));
    app.render(100, 30);
    app.press(&["tab", "end"]);
    let screen = app.screen(100, 30);
    assert!(screen.contains("The last line."), "{screen}");
    assert!(!screen.contains("Open Crash"), "{screen}");
    insta::assert_snapshot!(screen);
}

// The Comments tab.

fn comments(app: &TestApp) -> &super::IssueComments {
    main(app).details.issue().unwrap().comments()
}

/// The ids of the comments requests sent so far, oldest first.
fn comments_requests(app: &TestApp) -> Vec<RequestId> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::IssueComments { .. },
            } => Some(*id),
            _ => None,
        })
        .collect()
}

/// [`selected`] on the Comments tab, the strip focused.
fn on_comments(moves: usize) -> TestApp {
    let mut app = selected(moves);
    app.press(&["right"]);
    app
}

#[test]
fn one_comments_request_per_selection_before_the_tab_shows() {
    let mut app = app();
    app.press(&["enter"]);
    assert_eq!(comments_requests(&app).len(), 1);
    // The Overview shows, and the comments are on their way.
    assert_eq!(tabs(&app).1, 0);
    assert!(comments(&app).is_loading());
    settle(&mut app);
    assert!(!comments(&app).is_loading());
    assert_eq!(comments(&app).len(), 3);
    // Showing the tab sends nothing more.
    app.press(&["right"]);
    assert_eq!(comments_requests(&app).len(), 1);
}

#[test]
fn a_new_selection_cancels_the_comments_request_and_drops_its_answer() {
    let mut app = app();
    app.press(&["enter"]);
    let first = comments_requests(&app)[0];
    app.press(&["3", "j", "enter"]);
    assert!(app.effects().contains(&Effect::Cancel(first)));
    assert_eq!(comments_requests(&app).len(), 2);
    assert_eq!(shown_issue(&app), Some(11));
    // The first issue's comments, answered late, don't land on the second's tab.
    let out = logs(|| {
        app.send(Event::Github {
            id: first,
            response: GithubResponse::IssueComments(Ok(fixture_comments(12))),
        });
    });
    assert!(out.contains("Dropping the comments response"), "{out}");
    assert!(comments(&app).is_loading());
    settle(&mut app);
    assert_eq!(comments(&app).len(), 0);
}

#[test]
fn a_failed_request_logs_an_error_and_shows_no_comments() {
    let gh = gh().with_issue_comments(
        "parity-bot/issue-fixture",
        9,
        Err(GhError::Http {
            status: 500,
            message: "Server Error".into(),
            body: Vec::new(),
            headers: Vec::new(),
        }),
    );
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "j", "j", "j", "enter"]);
    assert_eq!(shown_issue(&app), Some(9));
    let out = settle(&mut app);
    assert_eq!(out.matches("ERROR Error retrieving issue comments").count(), 1, "{out}");
    assert!(out.contains("HTTP 500"), "{out}");
    assert!(!comments(&app).is_loading());
    app.press(&["right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│No comments available"), "{screen}");
    assert!(!screen.contains("Server Error"), "{screen}");
    assert!(app.app().toasts().is_empty());
}

#[test]
fn focus_walks_the_body_then_each_block_and_back() {
    let mut app = on_comments(0);
    app.render(160, 48);
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, None)
    );
    for block in 0..3 {
        app.press(&["tab"]);
        assert_eq!(
            (focus(&app), comments(&app).stop()),
            (FocusTarget::DetailsContent, Some(block))
        );
    }
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["shift+tab"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, Some(2))
    );
    app.press(&["shift+tab", "shift+tab", "shift+tab"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, None)
    );
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    // `5` goes back to the strip from a block.
    app.press(&["tab", "tab", "5"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

#[test]
fn without_comments_the_body_is_the_only_stop() {
    let mut app = on_comments(1);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["shift+tab"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, None)
    );
}

#[test]
fn the_last_block_scrolls_into_view_at_100x30() {
    // Issue 12's three comments fit at 100x30: twice as many, the last one changed, overflow.
    let mut many = fixture_comments(12);
    many.extend(fixture_comments(12));
    many.last_mut().unwrap().body = "The last comment.".into();
    let gh = gh().with_issue_comments("parity-bot/issue-fixture", 12, Ok(many));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["3", "enter"]);
    settle(&mut app);
    app.press(&["right"]);
    let screen = app.screen(100, 30);
    assert!(!screen.contains("The last comment."), "{screen}");
    app.press(&["tab"]);
    for _ in 0..6 {
        app.press(&["tab"]);
        app.render(100, 30);
    }
    assert_eq!(comments(&app).stop(), Some(5));
    let screen = app.screen(100, 30);
    assert!(comments(&app).offset() > 0);
    assert!(screen.contains("The last comment."), "{screen}");
    assert!(screen.contains("┗╍"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn a_click_on_a_block_focuses_it() {
    let mut app = on_comments(0);
    let row = row_of(&mut app, "A comment from a deleted account.");
    app.click(70, row);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, Some(1))
    );
    // The margin row below a block, which focus has boxed, is the body's.
    let row = row_of(&mut app, "Unknown • Wed Feb  4");
    app.click(70, row + 3);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, None)
    );
}

#[test]
fn n_and_r_open_the_new_comment_dialog_and_focus_comes_back() {
    let mut app = on_comments(0);
    app.press(&["tab"]);
    let main = app.app_mut().main_screen_mut().unwrap();
    assert!(matches!(
        main.details
            .on_binding(Owner::IssueComments, BindingId::ConversationCommentNew),
        Some(Action::Main(MainAction::DetailsCommand(Owner::IssueComments, _)))
    ));
    app.press(&["n"]);
    assert_eq!(app.app().modals(), [crate::modal::ModalKind::NewComment]);
    app.press(&["escape"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, None)
    );
    app.press(&["tab", "r"]);
    assert_eq!(app.app().modals(), [crate::modal::ModalKind::NewComment]);
    assert!(app.screen(160, 48).contains("Replying to comment from"));
    app.press(&["escape"]);
    assert_eq!(
        (focus(&app), comments(&app).stop()),
        (FocusTarget::DetailsContent, Some(0))
    );
}

#[test]
fn a_posted_comment_and_a_reply_are_appended() {
    let mut app = on_comments(0);
    let before = comments(&app).len();
    let mut posted = fixture_comments(12)[0].clone();
    posted.id = 7001;
    posted.body = "Posted later".into();
    app.dispatch(Action::Main(MainAction::CommentClosed {
        posted: Some(crate::modal::PostedComment::Issue(Box::new(posted.clone()))),
        reply: false,
    }));
    posted.id = 7002;
    posted.body = "A reply".into();
    app.dispatch(Action::Main(MainAction::CommentClosed {
        posted: Some(crate::modal::PostedComment::Issue(Box::new(posted))),
        reply: false,
    }));
    assert_eq!(comments(&app).len(), before + 2);
    let screen = app.screen(160, 48);
    let (later, reply) = (screen.find("Posted later").unwrap(), screen.find("A reply").unwrap());
    assert!(later < reply, "{screen}");
}

#[test]
fn the_footer_with_the_body_and_with_a_block_focused() {
    let mut app = on_comments(0);
    app.press(&["tab"]);
    let line = footer(&mut app);
    assert!(
        line.starts_with(" n New Comment  ^v Paste link  f Follow link"),
        "{line}"
    );
    app.press(&["tab"]);
    let line = footer(&mut app);
    assert!(
        line.starts_with(" r Reply to Comment  n New Comment  ^v Paste link  f Follow link"),
        "{line}"
    );
}

#[test]
fn maximize_works_from_a_block_but_not_the_comments_body() {
    let mut app = on_comments(0);
    app.press(&["tab", "M"]);
    assert_eq!(app.app().maximized(), None);
    app.press(&["tab", "M"]);
    assert!(app.app().maximized().is_some());
    let screen = app.screen(160, 48);
    assert!(!screen.contains("[3] Issues"), "{screen}");
    assert!(screen.contains("Thanks @parity-friend"), "{screen}");
}

#[test]
fn switching_tabs_keeps_the_comments_scroll_position() {
    let mut app = on_comments(0);
    app.render(100, 20);
    app.press(&["tab", "down", "down", "down"]);
    assert_eq!(comments(&app).offset(), 3);
    app.press(&["5", "left", "right"]);
    app.render(100, 20);
    assert_eq!(comments(&app).offset(), 3);
}

#[test]
fn issue_comments_loading() {
    let mut app = app();
    app.press(&["enter", "right"]);
    assert!(comments(&app).is_loading());
    let screen = app.screen(160, 48);
    assert!(screen.contains("Loading..."), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn issue_comments_160x48() {
    let mut app = on_comments(0);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_comments_100x30() {
    let mut app = on_comments(0);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn issue_comments_first_block_focused() {
    let mut app = on_comments(0);
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn issue_comments_none() {
    let mut app = on_comments(1);
    let screen = app.screen(160, 48);
    assert!(screen.contains("│No comments available"), "{screen}");
    insta::assert_snapshot!(screen);
}

/// Python mounts the posted comment below "No comments available", which stays.
#[test]
fn a_comment_posted_where_there_were_none_goes_below_the_empty_line() {
    let mut app = on_comments(1);
    assert_eq!(shown_issue(&app), Some(11));
    let mut posted = fixture_comments(12)[0].clone();
    posted.body = "The first comment".into();
    app.dispatch(Action::Main(MainAction::CommentClosed {
        posted: Some(crate::modal::PostedComment::Issue(Box::new(posted))),
        reply: false,
    }));
    let screen = app.screen(160, 48);
    let (empty, first) = (
        screen.find("No comments available").unwrap(),
        screen.find("The first comment").unwrap(),
    );
    assert!(empty < first, "{screen}");
}

// `gh` failing mid-session (#323): the comments.

mod gh_failures {
    use super::*;
    use crate::test_util::assert_gh_failures_surface;

    #[test]
    fn the_comments() {
        assert_gh_failures_surface(
            app,
            |app| {
                app.press(&["enter"]);
            },
            |app| {
                if comments(app).is_loading() {
                    return Err("still loading".into());
                }
                app.press(&["right"]);
                let screen = app.screen(160, 48);
                if screen.contains("│No comments available") {
                    Ok(())
                } else {
                    Err(format!("no fallback\n{screen}"))
                }
            },
        );
    }
}
