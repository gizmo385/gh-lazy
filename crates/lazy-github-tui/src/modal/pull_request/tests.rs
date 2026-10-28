//! Create PR: opening it, focus and keys, prefills, the requests on open, the upstream warning, branch suggestions,
//! closing and the drawing, opened with `P` against the `pr-details` and `pr-create` fixtures.

use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall};
use lazy_github_core::models::{IssueListItem, RepoName};
use serde_json::Value;

use super::*;
use crate::modal::ModalKind;
use crate::test_util::{GitAnswers, TestApp, logs};
use crate::toast::Toast;

const REPO: &str = "parity-bot/pr-details";

fn fixture_body(set: &str, path: &str) -> Value {
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

fn toasts(app: &TestApp) -> Vec<Toast> {
    app.app().toasts().iter().cloned().collect()
}

/// Create PR built directly, for its focus and fields, with `prefill`.
fn direct(prefill: CreatePrefill) -> PullRequestDialog {
    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| PullRequestDialog::create(fixture_repo(), prefill, cx))
}

/// The fields `direction` visits from the title, `steps` times.
fn walk(dialog: &mut PullRequestDialog, direction: FocusMove, steps: usize) -> Vec<PrField> {
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
    let (_, changed) = dialog.edit(PrField::Base, |input| {
        input.handle_paste("");
        input.set_value("");
        InputOutcome::Changed
    });
    assert!(changed);
    assert!(dialog.base.is_invalid());
    // Typing into the title clears its mark.
    dialog.focus(PrField::Title);
    let _ = dialog.edit(PrField::Title, |input| {
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
