//! Tiny terminals and resizes (#326): every screen and dialog draws at any size without panicking, and a resize
//! keeps what's open as it was.

use std::path::Path;

use lazy_github_core::config::Config;
use lazy_github_core::debug::DebugEnv;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{FullUser, Issue, Repository, Workflow};
use rstest::rstest;
use serde_json::Value;

use super::issue_lookup::IssueLookup;
use super::pr_details::tests::{comments, fixture_pr, fixture_repo, gh, reviews, settle, shown};
use super::pr_lookup::PrLookup;
use crate::action::Action;
use crate::component::Cx;
use crate::event::Event;
use crate::modal::{
    AddCommentDialog, AddReactionDialog, ConfirmDialog, CreatePrefill, DebugInfoDialog, EditIssueDialog, LookupDialog,
    NewCommentDialog, NewIssueDialog, NotificationsDialog, PullRequestDialog, ReplyTarget, RepoLookup,
    TriggerWorkflowDialog, UserProfileDialog,
};
use crate::modal::{Dialog, ModalKind};
use crate::test_util::TestApp;

/// The response body of the fixture at `path` in fixture set `set`.
fn fixture_body(set: &str, path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn issue() -> Issue {
    let body = fixture_body("issue-write", "repos/parity-bot/issue-fixture/issues/12/PATCH.json");
    Issue::from_api(body, &fixture_repo()).unwrap()
}

fn user() -> FullUser {
    serde_json::from_value(fixture_body("user-profile", "users/parity-bot/GET.json")).unwrap()
}

fn user_repos() -> Vec<Repository> {
    serde_json::from_value(fixture_body(
        "user-profile",
        "users/parity-bot/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
    ))
    .unwrap()
}

fn workflows() -> Vec<Workflow> {
    let body = fixture_body(
        "workflow-write",
        "repos/parity-bot/run-fixture/actions/workflows/GET.page-1_per_page-30.json",
    );
    serde_json::from_value(body["workflows"].clone()).unwrap()
}

fn debug_env() -> DebugEnv {
    DebugEnv {
        version: "0.1.0",
        rustc: "1.97.0",
        gh: Some("2.80.0".into()),
        os: "linux x86_64".into(),
        terminal: "xterm-256color".into(),
    }
}

/// Pull request 5 of `parity-bot/pr-details` in the details pane, with every request answered.
fn pr_shown() -> TestApp {
    shown(&["enter"])
}

/// [`pr_shown`] with the dialog `make` builds open over it, its reply doing nothing.
fn with_dialog<D: Dialog>(make: impl FnOnce(&mut Cx<'_>) -> D) -> TestApp {
    let mut app = pr_shown();
    app.with_cx(|cx| {
        let dialog = make(cx);
        cx.open_dialog(dialog, |_| Action::Consumed);
    });
    app
}

/// Every screen and dialog, in a state with something in it.
fn scene(name: &str) -> TestApp {
    screen_scene(name).unwrap_or_else(|| dialog_scene(name))
}

/// The base screens, and what the main screen opens itself.
fn screen_scene(name: &str) -> Option<TestApp> {
    let shown_after = |keys: &[&str]| {
        let mut app = pr_shown();
        app.press(keys);
        settle(&mut app);
        app
    };
    Some(match name {
        "startup" => TestApp::on_startup(Config::default(), FakeGithub::new()),
        "auth" => {
            let error = lazy_github_core::gh::GhError::NotAuthenticated {
                hostname: None,
                detail: String::new(),
            };
            let mut app = TestApp::on_startup(Config::default(), FakeGithub::new().with_auth_error(error));
            settle(&mut app);
            app
        }
        "main-empty" => TestApp::new(Config::default()),
        "main-repo" => {
            let mut app = TestApp::with_github(Config::default(), gh());
            app.press(&["enter"]);
            settle(&mut app);
            app
        }
        "pr-overview" => pr_shown(),
        "pr-diff" => shown_after(&["right"]),
        "pr-conversation" => shown_after(&["right", "right"]),
        "help" => shown_after(&["question_mark"]),
        "maximized" => shown_after(&["M"]),
        "command-log" => shown_after(&["6"]),
        "command-palette" => shown_after(&["ctrl+p"]),
        "settings" => {
            let mut app = pr_shown();
            app.dispatch(Action::OpenSettings);
            app
        }
        _ => return None,
    })
}

/// Each dialog the main screen opens, with data, over pull request 5.
fn dialog_scene(name: &str) -> TestApp {
    match name {
        "confirm" => with_dialog(|_| ConfirmDialog::new("Are you sure you want to merge?")),
        "lookup-repo" => with_dialog(|_| LookupDialog::<RepoLookup>::new()),
        "lookup-pr" => with_dialog(|_| LookupDialog::<PrLookup>::new()),
        "lookup-issue" => with_dialog(|_| LookupDialog::<IssueLookup>::new()),
        "user-profile" => with_dialog(|_| UserProfileDialog::new(user(), user_repos())),
        "new-issue" => with_dialog(|_| NewIssueDialog::new(fixture_repo(), None)),
        "edit-issue" => with_dialog(|_| EditIssueDialog::new(Box::new(issue()))),
        "new-comment" => with_dialog(|_| {
            let target = ReplyTarget::IssueComment(Box::new(comments(5).remove(0)));
            NewCommentDialog::new(fixture_repo(), 5, target)
        }),
        "review-reply" => with_dialog(|_| {
            let target = ReplyTarget::Review(Box::new(reviews(5).remove(0)));
            NewCommentDialog::new(fixture_repo(), 5, target)
        }),
        "add-comment" => with_dialog(|_| AddCommentDialog::new("+    let cursor = 0;")),
        "add-reaction" => with_dialog(|_| AddReactionDialog::new(&[])),
        "trigger-workflow" => with_dialog(|cx| TriggerWorkflowDialog::new(fixture_repo(), &workflows(), cx)),
        "create-pr" => with_dialog(|cx| {
            let prefill = CreatePrefill {
                base: "main".into(),
                compare: Some("cursor".into()),
                description: "A description\nover two lines".into(),
            };
            PullRequestDialog::create(fixture_repo(), prefill, cx)
        }),
        "edit-pr" => with_dialog(|cx| PullRequestDialog::edit(Box::new(fixture_pr(5)), cx)),
        "notifications" => with_dialog(NotificationsDialog::new),
        "debug-info" => with_dialog(|_| DebugInfoDialog::new(&debug_env(), &Config::default())),
        other => panic!("no scene {other}"),
    }
}

/// A small deterministic generator (xorshift), so a failing size is the same on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u16) -> u16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        u16::try_from(self.0 % u64::from(bound)).unwrap()
    }
}

/// Every size up to 56×20, which covers each side of the minimum, and 256 more of up to 300×100, in a shuffled
/// order so each size follows a different one, as in a run of resizes.
fn sizes(seed: u64) -> Vec<(u16, u16)> {
    let mut rng = Rng(seed | 1);
    let mut sizes: Vec<(u16, u16)> = (0..=56).flat_map(|w| (0..=20).map(move |h| (w, h))).collect();
    sizes.extend((0..256).map(|_| (rng.below(301), rng.below(101))));
    for i in (1..sizes.len()).rev() {
        let j = usize::from(rng.below(u16::try_from(i + 1).unwrap()));
        sizes.swap(i, j);
    }
    sizes
}

#[rstest]
fn every_size_draws_without_panicking(
    #[values(
        "startup",
        "auth",
        "main-empty",
        "main-repo",
        "pr-overview",
        "pr-diff",
        "pr-conversation",
        "help",
        "maximized",
        "command-log",
        "command-palette",
        "settings",
        "confirm",
        "lookup-repo",
        "lookup-pr",
        "lookup-issue",
        "user-profile",
        "new-issue",
        "edit-issue",
        "new-comment",
        "review-reply",
        "add-comment",
        "add-reaction",
        "trigger-workflow",
        "create-pr",
        "edit-pr",
        "notifications",
        "debug-info"
    )]
    name: &str,
) {
    let mut app = scene(name);
    let seed = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    for (width, height) in sizes(seed) {
        app.send(Event::Resize(width, height));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.render(width, height)));
        assert!(result.is_ok(), "{name} panicked at {width}×{height}");
    }
}

// The "Terminal too small" screen.

#[test]
fn below_the_minimum_only_the_message_is_drawn() {
    let mut app = scene("main-repo");
    insta::assert_snapshot!("too_small_39x20", app.screen(39, 20));
    insta::assert_snapshot!("too_small_80x14", app.screen(80, 14));
    // Too narrow for one line, the size goes on a second.
    insta::assert_snapshot!("too_small_20x6", app.screen(20, 6));
    assert!(app.screen(40, 15).contains("[1] Repos"));
}

#[test]
fn while_too_small_keys_clicks_and_pastes_do_nothing_but_quit() {
    let mut app = scene("main-repo");
    let before = app.screen(160, 48);
    app.render(30, 10);
    app.press(&["ctrl+p", "2", "question_mark", "M", "tab", "slash", "enter"]);
    app.click(5, 5);
    app.send(Event::Paste("pasted".into()));
    assert!(app.app().modals().is_empty());
    assert!(!app.app().should_quit());
    assert_eq!(app.screen(160, 48), before);

    app.render(30, 10);
    app.press(&["q"]);
    assert!(app.app().should_quit());
}

#[test]
fn while_too_small_q_in_a_text_field_is_not_quit_but_ctrl_q_is() {
    let mut app = scene("new-issue");
    app.press(&["T", "i"]);
    let before = app.screen(160, 48);
    app.send(Event::Resize(30, 10));
    app.press(&["q", "x"]);
    assert!(!app.app().should_quit());
    assert_eq!(app.screen(160, 48), before);

    // The main screen's search box types `q` too.
    let mut main = scene("main-repo");
    main.press(&["slash"]);
    main.send(Event::Resize(30, 10));
    main.press(&["q"]);
    assert!(!main.app().should_quit());

    app.send(Event::Resize(30, 10));
    app.press(&["ctrl+q"]);
    assert!(app.app().should_quit());
}

#[test]
fn growing_back_takes_keys_again() {
    let mut app = scene("main-repo");
    app.send(Event::Resize(30, 10));
    app.press(&["ctrl+p"]);
    assert!(app.app().modals().is_empty());
    app.send(Event::Resize(40, 15));
    app.press(&["ctrl+p"]);
    assert_eq!(app.app().modals(), [ModalKind::CommandPalette]);
}

// Resizes with something open.

/// Resizes from 160×48 to 80×24, below the minimum, and back, drawing each, and checks the screen at 160×48 is as it
/// was: the dialog, its focus, its scroll position and its text are all kept.
fn assert_resize_keeps(app: &mut TestApp) {
    let before = app.screen(160, 48);
    for (width, height) in [(80, 24), (30, 10), (160, 48)] {
        app.send(Event::Resize(width, height));
        app.render(width, height);
    }
    assert_eq!(app.screen(160, 48), before);
}

#[rstest]
#[case::confirm("confirm", &["tab"])]
#[case::lookup_repo("lookup-repo", &["o", "w", "n", "e", "r", "tab"])]
#[case::lookup_pr("lookup-pr", &["4", "2"])]
#[case::lookup_issue("lookup-issue", &["1", "3", "tab"])]
#[case::user_profile("user-profile", &["down", "down"])]
#[case::new_issue("new-issue", &["T", "i", "t", "l", "e", "tab", "B", "o", "d", "y"])]
#[case::edit_issue("edit-issue", &["end", "!", "tab", "tab"])]
#[case::new_comment("new-comment", &["H", "i", "enter", "t", "h", "e", "r", "e"])]
#[case::review_reply("review-reply", &["O", "k"])]
#[case::add_comment("add-comment", &["tab", "N", "o", "t", "e"])]
#[case::add_reaction("add-reaction", &["down", "space", "down"])]
#[case::trigger_workflow("trigger-workflow", &["tab", "end", "-", "x"])]
#[case::create_pr("create-pr", &["tab", "tab", "T", "i", "t", "l", "e"])]
#[case::edit_pr("edit-pr", &["tab", "end", "!"])]
#[case::notifications("notifications", &["right"])]
#[case::command_palette("command-palette", &["t", "h", "e", "down"])]
#[case::settings("settings", &["right", "tab"])]
#[case::debug_info("debug-info", &["down", "down", "down"])]
fn a_resize_keeps_the_open_dialog_as_it_was(#[case] name: &str, #[case] keys: &[&str]) {
    let mut app = scene(name);
    app.press(keys);
    let open = app.app().modals();
    assert!(!open.is_empty());
    assert_resize_keeps(&mut app);
    assert_eq!(app.app().modals(), open);
}

#[test]
fn a_resize_recenters_the_dialog() {
    let mut app = scene("confirm");
    for (width, height) in [(160, 48), (80, 24), (60, 30)] {
        app.send(Event::Resize(width, height));
        app.render(width, height);
        let interior = app.app().top_dialog_interior().unwrap();
        let left = interior.x;
        let right = width - interior.right();
        assert!(left.abs_diff(right) <= 1, "{width}×{height}: {interior:?}");
        let above = interior.y;
        // The bottom row is the footer's.
        let below = height - 1 - interior.bottom();
        assert!(above.abs_diff(below) <= 1, "{width}×{height}: {interior:?}");
    }
}

#[test]
fn a_resize_keeps_the_maximized_view() {
    let mut app = scene("maximized");
    assert!(app.app().maximized().is_some());
    assert_resize_keeps(&mut app);
    assert!(app.app().maximized().is_some());
}

#[test]
fn a_resize_keeps_the_help_panel_and_its_scroll() {
    let mut app = scene("help");
    assert_resize_keeps(&mut app);
    assert!(app.app().help_open());
}
