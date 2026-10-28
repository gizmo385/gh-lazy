//! Create PR: opening it, focus and keys, prefills, the requests on open, the upstream warning, branch suggestions,
//! closing and the drawing, opened with `P` against the `pr-details` and `pr-create` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall, NewPullRequest};
use lazy_github_core::models::{IssueListItem, RepoName};
use serde_json::Value;

use super::*;
use crate::modal::ModalKind;
use crate::test_util::{GitAnswers, TestApp, logs};
use crate::toast::Toast;

const REPO: &str = "parity-bot/pr-details";

pub(super) fn fixture_body(set: &str, path: &str) -> Value {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(set)
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

pub(super) fn fixture_repo() -> Repository {
    let page = fixture_body(
        "pr-details",
        "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json",
    );
    serde_json::from_value(page[0].clone()).unwrap()
}

/// `main`, `new-feature` and `next-release`.
fn branches() -> Vec<Branch> {
    serde_json::from_value(fixture_body(
        "pr-create",
        &format!("repos/{REPO}/branches/GET.page-1_per_page-30.json"),
    ))
    .unwrap()
}

fn issues_page() -> Vec<IssueListItem> {
    let page = fixture_body(
        "pr-details",
        &format!("repos/{REPO}/issues/GET.page-1_per_page-30_state-all.json"),
    );
    let repo = fixture_repo();
    page.as_array()
        .unwrap()
        .iter()
        .map(|item| IssueListItem::from_api_item(item.clone(), &repo).unwrap())
        .collect()
}

pub(super) fn gh() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![fixture_repo()])
        .with_issue_page(REPO, 1, Ok(issues_page()))
        .with_branches(REPO, Ok(branches()))
        .with_collaborators(REPO, Ok(vec!["parity-friend".into(), "parity-reviewer".into()]))
}

pub(super) fn settle(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

/// The repo selected and its PRs listed, the PR table focused.
pub(super) fn selected(app: TestApp) -> TestApp {
    let mut app = app;
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["2"]);
    app
}

/// [`selected`] with `P` pressed and the requests answered.
pub(super) fn opened_in(app: TestApp) -> TestApp {
    let mut app = selected(app);
    app.press(&["P"]);
    assert!(is_open(&app), "P opens Create PR");
    settle(&mut app);
    // Drawn once, as the app is after every event, so the scroll area knows its size.
    let _ = app.render(160, 48);
    app
}

pub(super) fn opened() -> TestApp {
    opened_in(TestApp::with_github(Config::default(), gh()))
}

pub(super) fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::PullRequest]
}

/// Types `text` into the focused field, a key at a time; `\n` is `enter`.
pub(super) fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            ' ' => "space".to_owned(),
            '\n' => "enter".to_owned(),
            '-' => "minus".to_owned(),
            c => c.to_string(),
        };
        app.press(&[key.as_str()]);
    }
}

/// The open dialog, read through its focus and fields.
pub(super) fn dialog_view(app: &mut TestApp) -> DialogView {
    // The dialog's state shows in its drawing; the focused field is the one whose border is drawn focused. Reading
    // it through the screen keeps the tests on what the user sees.
    let screen = app.screen(160, 48);
    DialogView { screen }
}

pub(super) struct DialogView {
    pub(super) screen: String,
}

impl DialogView {
    pub(super) fn row(&self, n: usize) -> &str {
        self.screen.lines().nth(n).unwrap_or_default()
    }
}

pub(super) fn toasts(app: &TestApp) -> Vec<Toast> {
    app.app().toasts().iter().cloned().collect()
}

/// Create PR built directly, for its focus and fields, with `prefill`.
fn direct(prefill: CreatePrefill) -> PullRequestDialog {
    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| PullRequestDialog::create(fixture_repo(), prefill, cx))
}

/// The fields `direction` visits from the title, `steps` times.
pub(super) fn walk(dialog: &mut PullRequestDialog, direction: FocusMove, steps: usize) -> Vec<PrField> {
    (0..steps)
        .map(|_| {
            dialog.move_focus(direction);
            dialog.ring.focused().unwrap()
        })
        .collect()
}

// Opening.

#[test]
fn p_without_a_repo_toasts() {
    let mut app = TestApp::with_github(Config::default(), gh());
    app.press(&["P"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(
        toasts(&app),
        [Toast::error("Please select a repository first!").with_title("Cannot open new pull request")]
    );
}

#[test]
fn p_opens_from_the_pull_requests_and_issues_sections_but_not_the_details() {
    let mut app = selected(TestApp::with_github(Config::default(), gh()));
    app.press(&["3", "P"]);
    assert!(is_open(&app), "from the Issues section");
    app.press(&["escape", "5", "P"]);
    assert!(app.app().modals().is_empty(), "the details pane doesn't bind it");
    assert!(toasts(&app).is_empty());
}

#[test]
fn p_asks_for_the_branches_uncached_and_the_collaborators() {
    let app = opened();
    let calls = app.gh().calls();
    assert!(
        calls.contains(&GithubCall::ListBranchesUncached(REPO.into())),
        "{calls:?}"
    );
    assert!(calls.contains(&GithubCall::Collaborators(REPO.into())), "{calls:?}");
    assert!(!calls.contains(&GithubCall::ListBranches(REPO.into())));
}

#[test]
fn the_title_has_focus_and_the_base_is_the_default_branch() {
    let dialog = direct(CreatePrefill {
        base: "main".into(),
        ..CreatePrefill::default()
    });
    assert_eq!(dialog.ring.focused(), Some(PrField::Title));
    assert_eq!(dialog.form().base_ref, "main");
    assert_eq!(dialog.form().head_ref, "");
    let mut app = opened();
    type_text(&mut app, "Hello");
    let view = dialog_view(&mut app);
    assert!(view.screen.contains("▊  Hello"), "{}", view.screen);
    assert!(view.row(14).contains("▊  main "), "{}", view.row(14));
    assert!(view.row(14).contains("Choose a head ref"), "{}", view.row(14));
}

#[test]
fn no_upstream_check_without_a_current_directory_branch() {
    let app = opened();
    assert!(!app.effects().iter().any(|effect| matches!(
        effect,
        Effect::Git {
            request: GitRequest::HasUpstream { .. },
            ..
        }
    )));
}

fn local(app: TestApp, repo: Option<&str>, branch: Option<&str>) -> TestApp {
    app.with_local_branch(branch)
        .with_local_repo(repo.map(|name| RepoName::from_remote_url(&format!("git@github.com:{name}.git")).unwrap()))
}

#[test]
fn compare_is_the_current_branch_only_in_a_clone_of_the_selected_repo() {
    // The clone's name may differ in case.
    let app = local(
        TestApp::with_github(Config::default(), gh()),
        Some("Parity-Bot/PR-Details"),
        Some("new-feature"),
    );
    let mut app = opened_in(app);
    assert!(dialog_view(&mut app).screen.contains("▊  new-feature "));
    let app = local(
        TestApp::with_github(Config::default(), gh()),
        Some("parity-bot/dotfiles"),
        Some("new-feature"),
    );
    let mut app = opened_in(app);
    let view = dialog_view(&mut app);
    assert!(!view.screen.contains("new-feature"), "{}", view.screen);
    assert!(view.screen.contains("Choose a head ref"));
}

#[test]
fn the_warning_shows_for_a_branch_without_upstream_whatever_the_repo() {
    for repo in [Some("parity-bot/pr-details"), Some("parity-bot/dotfiles"), None] {
        let app = local(TestApp::with_github(Config::default(), gh()), repo, Some("new-feature"));
        let mut app = opened_in(app);
        assert!(
            app.effects().iter().any(|effect| matches!(
                effect,
                Effect::Git { request: GitRequest::HasUpstream { branch }, .. } if branch == "new-feature"
            )),
            "the upstream check goes out for {repo:?}"
        );
        assert!(
            dialog_view(&mut app)
                .screen
                .contains("Warning: Current local branch has no configured upstream"),
            "{repo:?}"
        );
    }
    // With an upstream, no warning.
    let app = local(
        TestApp::with_github(Config::default(), gh()),
        Some("parity-bot/pr-details"),
        Some("new-feature"),
    )
    .with_git_answers(GitAnswers {
        upstream: true,
        ..GitAnswers::default()
    });
    let mut app = opened_in(app);
    assert!(!dialog_view(&mut app).screen.contains("Warning:"));
}

#[test]
fn both_branch_inputs_suggest_the_branches() {
    let mut app = opened();
    app.press(&["shift+tab", "shift+tab"]);
    type_text(&mut app, "new");
    assert!(
        dialog_view(&mut app).row(14).contains("▊  new-feature "),
        "Compare suggests"
    );
    app.press(&["shift+tab"]);
    // Base's text is selected on focus; typing replaces it.
    type_text(&mut app, "next");
    assert!(
        dialog_view(&mut app).row(14).contains("▊  next-release "),
        "Base suggests"
    );
}

#[test]
fn no_suggestions_when_the_branches_fail() {
    let gh = gh().with_branches(
        REPO,
        Err(lazy_github_core::gh::GhError::Http {
            status: 500,
            message: "boom".into(),
            body: Vec::new(),
        }),
    );
    let mut app = opened_in(TestApp::with_github(Config::default(), gh));
    app.press(&["shift+tab", "shift+tab"]);
    type_text(&mut app, "new");
    let view = dialog_view(&mut app);
    assert!(!view.row(14).contains("new-feature"), "{}", view.row(14));
    assert!(toasts(&app).is_empty());
}

// Focus and keys.

#[test]
fn tab_walks_the_fields_and_wraps() {
    use PrField::*;
    let mut dialog = direct(CreatePrefill::default());
    assert_eq!(
        walk(&mut dialog, FocusMove::Next, 9),
        [
            Description,
            Reviewers(ReviewersFocus::Input),
            Submit,
            Cancel,
            Scroll,
            Base,
            Compare,
            Draft,
            Title,
        ]
    );
}

#[test]
fn shift_tab_from_the_title_goes_to_draft_then_compare() {
    let mut dialog = direct(CreatePrefill::default());
    assert_eq!(
        walk(&mut dialog, FocusMove::Previous, 2),
        [PrField::Draft, PrField::Compare]
    );
}

#[test]
fn the_reviewers_list_joins_the_order_while_anyone_is_listed() {
    let mut dialog = direct(CreatePrefill::default());
    dialog.reviewers.add_existing(["parity-friend".to_owned()]);
    dialog.move_focus(FocusMove::Next);
    assert_eq!(
        walk(&mut dialog, FocusMove::Next, 3),
        [
            PrField::Reviewers(ReviewersFocus::Input),
            PrField::Reviewers(ReviewersFocus::List),
            PrField::Submit
        ]
    );
}

#[test]
fn q_types_in_the_inputs_and_the_editor() {
    let mut app = opened();
    app.press(&["q"]);
    app.press(&["tab", "q"]);
    app.press(&["tab", "q"]);
    assert!(is_open(&app));
    let view = dialog_view(&mut app);
    assert!(view.screen.contains("▊  q "), "{}", view.screen);
}

#[test]
fn q_closes_from_the_switch_and_the_buttons() {
    let mut app = opened();
    app.press(&["shift+tab", "q"]);
    assert!(!is_open(&app), "from Draft");
    let mut app = opened();
    app.press(&["tab", "tab", "tab", "q"]);
    assert!(!is_open(&app), "from Create");
}

#[test]
fn escape_closes_from_the_inputs_and_the_editor() {
    let mut app = opened();
    app.press(&["escape"]);
    assert!(!is_open(&app), "from the title");
    let mut app = opened();
    app.press(&["tab", "escape"]);
    assert!(!is_open(&app), "from the editor");
}

#[test]
fn cancel_closes_with_nothing_and_cancels_the_requests() {
    let mut app = selected(TestApp::with_github(Config::default(), gh()));
    app.press(&["P"]);
    // Nothing answered yet.
    app.press(&["tab", "tab", "tab", "tab", "enter"]);
    assert!(!is_open(&app));
    let cancels = app
        .effects()
        .iter()
        .filter(|effect| matches!(effect, Effect::Cancel(_)))
        .count();
    assert_eq!(cancels, 2, "the branches and the collaborators");
    assert!(toasts(&app).is_empty());
}

#[test]
fn enter_and_space_toggle_draft() {
    let mut app = TestApp::new(Config::default());
    let mut dialog = app.with_cx(|cx| PullRequestDialog::create(fixture_repo(), CreatePrefill::default(), cx));
    dialog.focus(PrField::Draft);
    assert_eq!(dialog.field_chain(), [FieldKind::Switch.link(ModalKind::PullRequest)]);
    app.with_cx(|cx| {
        let _ = dialog.on_field_binding(FieldKind::Switch, BindingId::TextualSwitchToggle, cx);
    });
    assert!(dialog.form().is_draft);
}

#[test]
fn an_input_emptied_and_left_is_marked_invalid() {
    let mut dialog = direct(CreatePrefill {
        base: "main".into(),
        ..CreatePrefill::default()
    });
    // Leaving the empty title marks it.
    dialog.move_focus(FocusMove::Previous);
    assert!(dialog.title.is_invalid());
    assert!(!dialog.base.is_invalid());
    // Base emptied: marked as it changes.
    dialog.focus(PrField::Base);
    let (_, changed) = dialog.edit_input(PrField::Base, |input| {
        input.handle_paste("");
        input.set_value("");
        InputOutcome::Changed
    });
    assert!(changed);
    assert!(dialog.base.is_invalid());
    // Typing into the title clears its mark.
    dialog.focus(PrField::Title);
    let _ = dialog.edit_input(PrField::Title, |input| {
        input.handle_paste("x");
        InputOutcome::Changed
    });
    assert!(!dialog.title.is_invalid());
}

#[test]
fn reopening_starts_fresh() {
    let mut app = opened();
    type_text(&mut app, "Typed");
    app.press(&["escape"]);
    app.press(&["P"]);
    settle(&mut app);
    assert!(!dialog_view(&mut app).screen.contains("Typed"));
}

// Reviewers in the dialog.

#[test]
fn the_reviewers_input_suggests_collaborators_and_enter_adds() {
    let gh = gh().with_profile(
        "parity-friend",
        Ok(serde_json::from_value(fixture_body("pr-edit", "users/parity-friend/GET.json")).unwrap()),
    );
    let mut app = opened_in(TestApp::with_github(Config::default(), gh));
    app.press(&["tab", "tab"]);
    type_text(&mut app, "parity-fr");
    app.press(&["right", "enter"]);
    settle(&mut app);
    let view = dialog_view(&mut app);
    assert!(
        view.screen.contains("Current Reviewers - Click/Select them to remove"),
        "{}",
        view.screen
    );
    assert!(view.screen.contains("▐X▌ parity-friend"));
}

// Drawing.

#[test]
fn create_pr_opened() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn create_pr_opened_with_warning() {
    let app = local(
        TestApp::with_github(Config::default(), gh()),
        Some("parity-bot/pr-details"),
        Some("new-feature"),
    );
    let mut app = opened_in(app);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn create_pr_opened_100x30() {
    let mut app = opened();
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn create_pr_opened_with_warning_100x30() {
    let app = local(
        TestApp::with_github(Config::default(), gh()),
        Some("parity-bot/pr-details"),
        Some("new-feature"),
    );
    let mut app = opened_in(app);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn create_pr_scrolled_to_create() {
    let mut app = opened();
    app.press(&["tab", "tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn create_pr_compare_suggestion() {
    let mut app = opened();
    app.press(&["shift+tab", "shift+tab"]);
    type_text(&mut app, "new");
    insta::assert_snapshot!(app.screen(160, 48));
}

// Submitting.

/// PR 10, as `POST .../pulls` answers.
pub(super) fn created() -> FullPullRequest {
    let body = fixture_body("pr-create", &format!("repos/{REPO}/pulls/10/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

pub(super) fn refused() -> lazy_github_core::gh::GhError {
    lazy_github_core::gh::GhError::Http {
        status: 422,
        message: "Validation Failed".into(),
        body: Vec::new(),
    }
}

/// The create requests sent, oldest first.
fn creates(app: &TestApp) -> Vec<NewPullRequest> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::CreatePullRequest { full_name, pr } => {
                assert_eq!(full_name, REPO);
                Some(pr)
            }
            _ => None,
        })
        .collect()
}

/// The git requests sent, oldest first, without the upstream check.
fn git_requests(app: &TestApp) -> Vec<GitRequest> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Git { request, .. } if !matches!(request, GitRequest::HasUpstream { .. }) => Some(request.clone()),
            _ => None,
        })
        .filter(|request| !matches!(request, GitRequest::OriginRepo | GitRequest::CurrentBranch))
        .collect()
}

pub(super) fn messages(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

pub(super) fn titled(title: &str, message: &str) -> (Option<String>, String) {
    (Some(title.to_owned()), message.to_owned())
}

pub(super) fn plain(message: &str) -> (Option<String>, String) {
    (None, message.to_owned())
}

/// Create PR with the title, Compare and the description typed (`\n` in it is `enter`), from `app`.
fn filled_in(app: TestApp, title: &str, compare: &str, description: &str) -> TestApp {
    let mut app = opened_in(app);
    type_text(&mut app, title);
    app.press(&["shift+tab", "shift+tab"]);
    type_text(&mut app, compare);
    app.press(&["tab", "tab", "tab"]);
    type_text(&mut app, description);
    app
}

fn creating_gh(result: Result<FullPullRequest, lazy_github_core::gh::GhError>) -> FakeGithub {
    gh().with_created_pull_request(result)
}

#[test]
fn each_empty_required_field_is_refused_and_marked() {
    for (title, compare, base_emptied) in [("", "x", false), ("T", "", false), ("T", "x", true)] {
        let mut app = opened_in(TestApp::with_github(Config::default(), creating_gh(Ok(created()))));
        type_text(&mut app, title);
        app.press(&["shift+tab", "shift+tab"]);
        type_text(&mut app, compare);
        if base_emptied {
            app.press(&["shift+tab", "backspace"]);
        }
        app.press(&["shift+enter"]);
        settle(&mut app);
        assert!(creates(&app).is_empty(), "{title:?} {compare:?} {base_emptied}");
        assert_eq!(messages(&app), [titled("Invalid PR!", "Missing required fields!")]);
        assert!(is_open(&app));
    }
}

#[test]
fn every_empty_field_is_marked_on_submit() {
    let mut dialog = direct(CreatePrefill::default());
    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| dialog.submit(cx));
    assert!(dialog.title.is_invalid() && dialog.compare.is_invalid() && dialog.base.is_invalid());
    assert!(dialog.submit.is_none(), "nothing sent");
}

#[test]
fn spaces_pass_and_the_request_is_built_from_the_form() {
    let app = filled_in(
        TestApp::with_github(Config::default(), creating_gh(Ok(created()))),
        " ",
        " ",
        "",
    );
    let mut app = app;
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(
        creates(&app),
        [NewPullRequest {
            title: " ".into(),
            draft: false,
            base: "main".into(),
            head: "parity-bot: ".into(),
            body: None,
        }]
    );
}

#[test]
fn draft_and_the_description_are_sent() {
    let mut app = filled_in(
        TestApp::with_github(Config::default(), creating_gh(Ok(created()))),
        "Parity PR",
        "new-feature",
        "First line\nSecond line",
    );
    // Back to Draft: description, title, Draft.
    app.press(&["shift+tab", "shift+tab", "space", "shift+enter"]);
    settle(&mut app);
    assert_eq!(
        creates(&app),
        [NewPullRequest {
            title: "Parity PR".into(),
            draft: true,
            base: "main".into(),
            head: "parity-bot:new-feature".into(),
            body: Some("First line\nSecond line".into()),
        }]
    );
}

#[test]
fn a_second_submit_while_creating_is_ignored_and_closing_cancels_it() {
    let mut app = filled_in(
        TestApp::with_github(Config::default(), creating_gh(Ok(created()))),
        "T",
        "b",
        "",
    );
    app.press(&["shift+enter", "shift+enter"]);
    let creating: Vec<RequestId> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::CreatePullRequest { .. },
            } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(creating.len(), 1);
    assert_eq!(messages(&app), [plain("Creating PR...")]);
    app.press(&["escape"]);
    assert!(!is_open(&app));
    assert!(app.effects().contains(&Effect::Cancel(creating[0])));
}

#[test]
fn a_created_pull_request_closes_the_dialog_with_it() {
    let mut app = filled_in(
        TestApp::with_github(Config::default(), creating_gh(Ok(created()))),
        "Parity PR",
        "new-feature",
        "",
    );
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(
        messages(&app),
        [plain("Creating PR..."), plain("Successfully created PR!")]
    );
    // No reviewers listed: no review request.
    assert!(
        !app.gh()
            .calls()
            .iter()
            .any(|call| matches!(call, GithubCall::RequestReviewers { .. }))
    );
    let main = app.app().main_screen().unwrap();
    assert_eq!(main.details_title(), "[5] PR #10 Details");
}

#[test]
fn listed_reviewers_are_requested_and_a_refusal_is_reported_after_the_success() {
    for refuse in [false, true] {
        let friend = serde_json::from_value(fixture_body("pr-edit", "users/parity-friend/GET.json")).unwrap();
        let gh = creating_gh(Ok(created()))
            .with_review_request(10, if refuse { Err(refused()) } else { Ok(()) })
            .with_profile("parity-friend", Ok(friend));
        // From the description, `tab` to the Reviewers input, which lists the login it finds.
        let mut app = filled_in(TestApp::with_github(Config::default(), gh), "T", "b", "");
        app.press(&["tab"]);
        type_text(&mut app, "parity-friend");
        app.press(&["enter"]);
        settle(&mut app);
        app.press(&["shift+enter"]);
        settle(&mut app);
        assert!(app.gh().calls().contains(&GithubCall::RequestReviewers {
            full_name: REPO.into(),
            number: 10,
            reviewers: vec!["parity-friend".into()],
        }));
        assert!(!is_open(&app), "closed either way");
        let mut expected = vec![plain("Creating PR..."), plain("Successfully created PR!")];
        if refuse {
            expected.push(titled(
                "Error requesting reviews",
                "Could not request reviews from parity-friend",
            ));
        }
        assert_eq!(messages(&app), expected, "refused: {refuse}");
    }
}

#[test]
fn a_refused_create_keeps_the_form_and_checks_the_branch_as_typed() {
    let app = TestApp::with_github(Config::default(), creating_gh(Err(refused()))).with_git_answers(GitAnswers {
        on_remote: true,
        ..GitAnswers::default()
    });
    let mut app = filled_in(app, "Parity PR", " spaced ", "kept");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(
        messages(&app)[1],
        titled(
            "Error creating pull request",
            "Check that your branches are valid and that a PR does not already exist"
        )
    );
    assert_eq!(
        git_requests(&app),
        [GitRequest::BranchExistsOnRemote {
            branch: " spaced ".into()
        }]
    );
    assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
    assert!(dialog_view(&mut app).screen.contains("▊ 1  kept"));
}

/// A refused create in a directory whose `origin` is `local` (if any), with git answering from `answers`.
fn refused_in(local: Option<&str>, answers: GitAnswers) -> TestApp {
    let app = TestApp::with_github(Config::default(), creating_gh(Err(refused())))
        .with_local_repo(local.map(|name| RepoName::from_remote_url(&format!("git@github.com:{name}.git")).unwrap()))
        .with_git_answers(answers);
    let mut app = filled_in(app, "Parity PR", "missing-branch", "");
    app.press(&["shift+enter"]);
    settle(&mut app);
    app
}

#[test]
fn a_branch_on_the_remote_asks_nothing() {
    let app = refused_in(
        None,
        GitAnswers {
            on_remote: true,
            ..GitAnswers::default()
        },
    );
    assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
}

#[test]
fn a_missing_branch_asks_to_push_it() {
    let mut app = refused_in(None, GitAnswers::default());
    assert_eq!(app.app().modals(), [ModalKind::PullRequest, ModalKind::Confirm]);
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("Warning: Branch missing-branch may not exist"),
        "{screen}"
    );
    assert!(screen.contains("on remote. Do you want to create it?"));
    assert_eq!(screen.lines().last(), Some(" y Yes  n No  q Close"));
}

#[test]
fn yes_outside_a_clone_of_the_repo_fails_without_git() {
    for local in [None, Some("parity-bot/dotfiles")] {
        let mut app = refused_in(local, GitAnswers::default());
        app.press(&["y"]);
        settle(&mut app);
        assert_eq!(app.app().modals(), [ModalKind::PullRequest], "back to Create PR");
        assert!(
            !git_requests(&app)
                .iter()
                .any(|request| matches!(request, GitRequest::PushBranch { .. }))
        );
        assert_eq!(
            messages(&app).last(),
            Some(&plain("Error while pushing branch missing-branch to remote"))
        );
        let toast = app.app().toasts().iter().last().unwrap();
        assert_eq!(
            toast.message.spans[1].style.fg,
            Some(crate::theme::Theme::default().accent(crate::theme::Accent::Red))
        );
    }
}

#[test]
fn yes_in_a_clone_pushes_and_reports_either_way() {
    for push in [Ok(()), Err("rejected".to_owned())] {
        let failed = push.is_err();
        let mut app = refused_in(
            Some("parity-bot/pr-details"),
            GitAnswers {
                push,
                ..GitAnswers::default()
            },
        );
        app.press(&["y"]);
        settle(&mut app);
        assert!(git_requests(&app).contains(&GitRequest::PushBranch {
            branch: "missing-branch".into()
        }));
        let expected = if failed {
            "Error while pushing branch missing-branch to remote"
        } else {
            "Pushed branch missing-branch to remote"
        };
        assert_eq!(messages(&app).last(), Some(&plain(expected)));
        assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
        assert_eq!(creates(&app).len(), 1, "nothing is resubmitted");
    }
}

#[test]
fn no_pushes_nothing_and_focus_is_back_where_it_was() {
    for key in ["n", "escape", "q"] {
        let mut app = refused_in(Some("parity-bot/pr-details"), GitAnswers::default());
        app.press(&[key]);
        settle(&mut app);
        assert_eq!(app.app().modals(), [ModalKind::PullRequest], "{key}");
        assert!(
            !git_requests(&app)
                .iter()
                .any(|request| matches!(request, GitRequest::PushBranch { .. }))
        );
        // The description had focus: typing goes there.
        type_text(&mut app, "after");
        assert!(dialog_view(&mut app).screen.contains("after"), "{key}");
    }
}

#[test]
fn a_push_answer_after_the_dialog_closed_is_dropped() {
    let mut app = refused_in(
        Some("parity-bot/pr-details"),
        GitAnswers {
            push: Ok(()),
            ..GitAnswers::default()
        },
    );
    // Yes, then Create PR closed before git answers.
    app.press(&["y", "escape"]);
    settle(&mut app);
    assert!(app.app().modals().is_empty());
    assert!(!messages(&app).iter().any(|(_, message)| message.starts_with("Pushed")));
}

#[test]
fn create_pr_push_prompt() {
    let mut app = refused_in(None, GitAnswers::default());
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn create_pr_missing_fields() {
    let mut app = opened();
    app.press(&["shift+enter"]);
    let screen = app.render(160, 48);
    // The dialog's frame, with the invalid borders' colors.
    let frame = Rect::new(30, 6, 100, 36);
    let mut dialog = ratatui::buffer::Buffer::empty(frame);
    for (x, y) in frame.positions().map(|p| (p.x, p.y)) {
        dialog[(x, y)] = screen[(x, y)].clone();
    }
    insta::assert_snapshot!(crate::test_util::buffer_to_styled_string(&dialog));
}
