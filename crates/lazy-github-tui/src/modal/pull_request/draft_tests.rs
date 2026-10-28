//! Create PR's draft: the debounced save, the saves on Cancel and `modal.close`, the clear after a create, the
//! restore on open, and Edit PR having none. Against the `pr-details` and `pr-create` fixtures and the `pr-draft`
//! seed's draft.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lazy_github_core::config::Config;
use lazy_github_core::drafts::{self, PrDraft};
use lazy_github_core::github::FakeGithub;

use super::tests::{
    created, fixture_body, fixture_repo, gh, is_open, messages, opened_in, plain, refused, selected, settle, type_text,
};
use super::*;
use crate::modal::ModalKind;
use crate::test_util::{GitAnswers, TestApp};

const REPO: &str = "parity-bot/pr-details";

/// The seed the `pr-drafts-restore` scenario starts from.
fn seed(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/seeds")
        .join(path)
}

fn seeded_draft_text() -> String {
    std::fs::read_to_string(seed(
        "pr-draft/home/.config/lazy-github/.cache/pr_drafts/parity-bot_pr-details.json",
    ))
    .unwrap()
}

fn config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = PathBuf::from("cache-dir");
    config
}

fn draft_path() -> PathBuf {
    drafts::draft_path(Path::new("cache-dir"), REPO)
}

fn app_with(gh: FakeGithub) -> TestApp {
    TestApp::with_github(config(), gh)
}

/// Create PR opened, with no draft saved.
fn opened() -> TestApp {
    opened_in(app_with(gh()))
}

/// Create PR opened with `text` as the saved draft.
fn opened_with_draft(config: Config, text: &str) -> TestApp {
    opened_in(TestApp::with_github(config, gh()).with_cache_file(draft_path(), text))
}

fn save_timeouts(app: &TestApp) -> usize {
    app.effects()
        .iter()
        .filter(|effect| {
            **effect
                == Effect::StartTimeout {
                    id: TimerId::PrDraftSave,
                    after: Duration::from_secs(2),
                }
        })
        .count()
}

fn draft_writes(app: &TestApp) -> Vec<&Effect> {
    app.effects()
        .iter()
        .filter(|effect| match effect {
            Effect::WriteCache { path, .. } | Effect::RemoveCache { path } => *path == draft_path(),
            _ => false,
        })
        .collect()
}

/// The saved draft, as the fake cache file holds it.
fn saved(app: &TestApp) -> Option<PrDraft> {
    app.cache_file(&draft_path())
        .map(|text| drafts::try_parse(&draft_path(), text).unwrap())
}

fn restored_draft() -> PrDraft {
    PrDraft {
        repo_full_name: REPO.into(),
        title: "Restored draft title".into(),
        description: "Restored draft body.\n\nSecond paragraph.".into(),
        base_ref: "main".into(),
        head_ref: "feature/restored".into(),
        is_draft: true,
        reviewers: vec!["parity-friend".into()],
    }
}

// Saving.

#[test]
fn an_edit_starts_the_save_timer_and_the_next_one_restarts_it() {
    let mut app = opened();
    assert_eq!(save_timeouts(&app), 0, "nothing restored, nothing scheduled");
    type_text(&mut app, "A");
    assert_eq!(save_timeouts(&app), 1);
    type_text(&mut app, "B");
    assert_eq!(save_timeouts(&app), 2);
    // Base, Compare and the description count too.
    app.press(&[
        "shift+tab",
        "shift+tab",
        "x",
        "shift+tab",
        "y",
        "tab",
        "tab",
        "tab",
        "z",
    ]);
    assert_eq!(save_timeouts(&app), 5);
    assert!(draft_writes(&app).is_empty(), "nothing saved yet");
}

#[test]
fn toggling_draft_and_adding_a_reviewer_start_no_timer() {
    let friend = serde_json::from_value(fixture_body("pr-edit", "users/parity-friend/GET.json")).unwrap();
    let mut app = opened_in(app_with(gh().with_profile("parity-friend", Ok(friend))));
    // Draft, from the title.
    app.press(&["shift+tab", "space"]);
    // The Reviewers input, from Draft: title, description, reviewers.
    app.press(&["tab", "tab", "tab"]);
    type_text(&mut app, "parity-friend");
    app.press(&["enter"]);
    settle(&mut app);
    assert_eq!(save_timeouts(&app), 0);
    // Both are still saved by the next save.
    app.press(&["escape"]);
    let draft = saved(&app).unwrap();
    assert!(draft.is_draft);
    assert_eq!(draft.reviewers, ["parity-friend"]);
}

#[test]
fn the_timer_saves_the_form_with_the_title_and_description_stripped() {
    let mut app = opened();
    type_text(&mut app, " Spaced title ");
    app.press(&["tab"]);
    type_text(&mut app, "\nBody\n");
    app.advance(Duration::from_millis(1999));
    assert!(saved(&app).is_none());
    app.advance(Duration::from_millis(1));
    assert_eq!(
        saved(&app),
        Some(PrDraft {
            repo_full_name: REPO.into(),
            title: "Spaced title".into(),
            description: "Body".into(),
            base_ref: "main".into(),
            head_ref: String::new(),
            is_draft: false,
            reviewers: Vec::new(),
        })
    );
    assert!(is_open(&app), "saving doesn't close it");
    let text = app.cache_file(&draft_path()).unwrap();
    assert!(text.starts_with("{\n  \"repo_full_name\": "), "{text}");
}

#[test]
fn cancel_and_modal_close_save_at_once_and_drop_the_timer() {
    for keys in [&["escape"][..], &["tab", "tab", "tab", "tab", "enter"][..]] {
        let mut app = opened();
        type_text(&mut app, "Kept");
        app.press(keys);
        assert!(!is_open(&app), "{keys:?}");
        assert_eq!(saved(&app).map(|draft| draft.title), Some("Kept".into()), "{keys:?}");
        assert!(app.effects().contains(&Effect::CancelTimer(TimerId::PrDraftSave)));
        // The dropped timer never fires, and a stale tick does nothing.
        let writes = draft_writes(&app).len();
        app.advance(Duration::from_secs(5));
        app.dispatch(Action::Timer(TimerId::PrDraftSave));
        assert_eq!(draft_writes(&app).len(), writes, "{keys:?}");
    }
}

#[test]
fn a_save_with_nothing_in_it_deletes_the_draft() {
    // Whitespace counts as nothing once stripped.
    for typed in ["", "  "] {
        let mut app = opened_in(app_with(gh()).with_cache_file(draft_path(), "{}"));
        type_text(&mut app, typed);
        app.press(&["escape"]);
        assert!(!is_open(&app));
        assert_eq!(
            draft_writes(&app),
            [&Effect::RemoveCache { path: draft_path() }],
            "{typed:?}"
        );
        assert_eq!(app.cache_file(&draft_path()), None);
    }
}

#[test]
fn a_created_pull_request_deletes_the_draft_and_a_refused_one_keeps_it() {
    for ok in [true, false] {
        let result = if ok { Ok(created()) } else { Err(refused()) };
        // The refused head is on the remote, so no push prompt opens over the dialog.
        let app = app_with(gh().with_created_pull_request(result)).with_git_answers(GitAnswers {
            on_remote: true,
            ..GitAnswers::default()
        });
        let mut app = opened_in(app);
        type_text(&mut app, "Parity PR");
        app.press(&["shift+tab", "shift+tab"]);
        type_text(&mut app, "new-feature");
        app.press(&["shift+enter"]);
        settle(&mut app);
        let cancelled = app.effects().contains(&Effect::CancelTimer(TimerId::PrDraftSave));
        let removed = app.effects().contains(&Effect::RemoveCache { path: draft_path() });
        assert_eq!((cancelled, removed), (ok, ok), "created: {ok}");
        if ok {
            assert_eq!(
                messages(&app),
                [plain("Creating PR..."), plain("Successfully created PR!")]
            );
        } else {
            assert!(is_open(&app));
            // The timer is still running and saves as usual.
            app.advance(Duration::from_secs(2));
            assert_eq!(saved(&app).map(|draft| draft.title), Some("Parity PR".into()));
        }
    }
}

#[test]
fn edit_pr_reads_schedules_and_writes_no_draft() {
    let pr: FullPullRequest = FullPullRequest::from_api(
        fixture_body("pr-details", &format!("repos/{REPO}/pulls/5/GET.json")),
        &fixture_repo(),
    )
    .unwrap();
    let gh = gh()
        .with_pull_request(REPO, 5, Ok(pr))
        .with_pull_request_update(5, Ok(()));
    let mut app = selected(app_with(gh).with_cache_file(draft_path(), seeded_draft_text()));
    app.press(&["E"]);
    settle(&mut app);
    assert!(is_open(&app));
    type_text(&mut app, "x");
    app.press(&["escape"]);
    let touched = app.effects().iter().any(|effect| match effect {
        Effect::ReadCache { path, .. } | Effect::WriteCache { path, .. } | Effect::RemoveCache { path } => {
            *path == draft_path()
        }
        Effect::StartTimeout { id, .. } | Effect::CancelTimer(id) => *id == TimerId::PrDraftSave,
        _ => false,
    });
    assert!(!touched, "{:?}", app.effects());
    assert!(!messages(&app).contains(&plain("Restored saved PR draft")));
}

// Restoring.

/// A config whose template is the seed's `pr_template.md`.
fn templated(seed_name: &str) -> Config {
    let mut config = config();
    config.pull_requests.pull_request_template = Some(seed(&format!("{seed_name}/work/pr_template.md")));
    config
}

#[test]
fn a_saved_draft_fills_every_field_in_schedules_a_save_and_toasts() {
    let mut app = opened_with_draft(templated("pr-draft"), &seeded_draft_text());
    let screen = app.screen(160, 48);
    for shown in [
        "Restored draft title",
        "Restored draft body.",
        "Second paragraph.",
        "feature/restored",
    ] {
        assert!(screen.contains(shown), "{shown}:\n{screen}");
    }
    assert!(!screen.contains("Template text"), "{screen}");
    assert_eq!(messages(&app), [plain("Restored saved PR draft")]);
    assert_eq!(save_timeouts(&app), 1, "restoring counts as a change");
    // The reviewers are listed below the fold: the list's focus brings them up (title, description, list).
    app.press(&["tab", "tab"]);
    let screen = app.screen(160, 48);
    for shown in ["Current Reviewers - Click/Select them to remove", "parity-friend"] {
        assert!(screen.contains(shown), "{shown}:\n{screen}");
    }
    // The 2 s save writes the same draft back, every field included.
    app.advance(Duration::from_secs(2));
    assert_eq!(saved(&app), Some(restored_draft()));
}

#[test]
fn a_blank_draft_replaces_the_template_silently() {
    let blank = PrDraft {
        repo_full_name: REPO.into(),
        title: "  ".into(),
        base_ref: "main".into(),
        ..PrDraft::default()
    };
    let mut app = opened_with_draft(templated("pr-draft"), &drafts::to_json(&blank));
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Template text"), "{screen}");
    assert!(messages(&app).is_empty(), "{:?}", messages(&app));
    assert_eq!(save_timeouts(&app), 1);
    // Nothing in it once stripped, so its save deletes it.
    app.advance(Duration::from_secs(2));
    assert_eq!(app.cache_file(&draft_path()), None);
}

#[test]
fn a_draft_that_doesnt_load_leaves_the_form_as_it_opened() {
    let corrupt = std::fs::read_to_string(seed(
        "pr-draft-corrupt/home/.config/lazy-github/.cache/pr_drafts/parity-bot_pr-details.json",
    ))
    .unwrap();
    let mut app =
        selected(TestApp::with_github(templated("pr-draft-corrupt"), gh()).with_cache_file(draft_path(), corrupt));
    app.press(&["P"]);
    let logged = settle(&mut app);
    let warning = format!(
        "WARN PR draft file '{}' has invalid structure: ",
        draft_path().display()
    );
    assert!(logged.contains(&warning), "{logged}");
    let screen = app.screen(160, 48);
    assert!(screen.contains("Template text from pr_template.md"), "{screen}");
    assert!(!screen.contains("Half a draft"), "{screen}");
    assert!(messages(&app).is_empty());
    assert_eq!(save_timeouts(&app), 0);
}

#[test]
fn a_restored_draft_s_compare_wins_over_the_current_directory_branch() {
    // Py bug B72's recommendation: the prefill is in place before the draft's read answers, so the draft replaces it.
    let mut dialog_app = TestApp::new(config());
    let prefill = CreatePrefill {
        base: "main".into(),
        compare: Some("local-branch".into()),
        description: String::new(),
    };
    let mut dialog = dialog_app.with_cx(|cx| PullRequestDialog::create(fixture_repo(), prefill, cx));
    assert_eq!(dialog.form().head_ref, "local-branch");
    let id = dialog.draft_file.as_ref().and_then(|file| file.read).unwrap();
    let unclaimed = dialog_app.with_cx(|cx| dialog.on_cache_read(id, Ok(Some(seeded_draft_text())), cx));
    assert!(unclaimed.is_none());
    assert_eq!(dialog.form().head_ref, "feature/restored");
    // Another read's answer isn't the dialog's.
    let other = dialog_app.with_cx(|cx| dialog.on_cache_read(id, Ok(None), cx));
    assert_eq!(other, Some(Ok(None)));
}

#[test]
fn create_pr_restored_from_the_seeded_draft() {
    let mut app = opened_with_draft(templated("pr-draft"), &seeded_draft_text());
    assert_eq!(app.app().modals(), [ModalKind::PullRequest]);
    insta::assert_snapshot!(app.screen(160, 48));
}

// The app's routing.

#[test]
fn the_app_hands_a_timer_tick_to_the_open_dialog_and_drops_it_without_one() {
    let mut app = opened();
    type_text(&mut app, "Ticked");
    app.dispatch(Action::Timer(TimerId::PrDraftSave));
    assert_eq!(saved(&app).map(|draft| draft.title), Some("Ticked".into()));
    app.press(&["escape"]);
    let writes = draft_writes(&app).len();
    let logged = crate::test_util::logs(|| {
        app.dispatch(Action::Timer(TimerId::PrDraftSave));
    });
    assert_eq!(draft_writes(&app).len(), writes);
    assert!(
        logged.contains("Ignoring timer PrDraftSave: nothing owns it"),
        "{logged}"
    );
}

#[test]
fn the_app_hands_a_cache_read_to_the_dialog_that_asked_for_it() {
    let mut app = selected(app_with(gh()));
    app.press(&["P"]);
    let read = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::ReadCache { id, path } if *path == draft_path() => Some(*id),
            _ => None,
        })
        .expect("Create PR reads its draft");
    app.dispatch(Action::CacheRead(read, Ok(Some(seeded_draft_text()))));
    assert_eq!(messages(&app), [plain("Restored saved PR draft")]);
    // An answer nobody asked for is dropped.
    let logged = crate::test_util::logs(|| {
        app.dispatch(Action::CacheRead(read, Ok(Some(seeded_draft_text()))));
    });
    assert!(logged.contains("nothing is waiting for it"), "{logged}");
    assert_eq!(messages(&app).len(), 1);
}
