//! `repos.json`: the repos table's cache, restored under the loading overlay and rewritten when the rows change.

use std::path::PathBuf;

use futures_util::FutureExt as _;
use lazy_github_core::cache;
use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::{RepoName, Repository};

use super::{ReposSection, repo};
use crate::effect::{Effect, GitResponse, GithubRequest};
use crate::event::Event;
use crate::options::AppOptions;
use crate::test_util::{TestApp, logs};
use crate::theme::Theme;

fn config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = PathBuf::from("cache-dir");
    config
}

fn cache_path() -> PathBuf {
    PathBuf::from("cache-dir").join("repos.json")
}

/// What the cache holds: one repo the list no longer returns, and one it does.
fn cached() -> Vec<Repository> {
    vec![
        repo("parity-bot", "cache-fixture", false),
        repo("parity-bot", "dotfiles", true),
    ]
}

/// What the list returns.
fn listed() -> Vec<Repository> {
    ["dotfiles", "hello-world", "lazy-demo"]
        .map(|name| repo("parity-bot", name, false))
        .into()
}

/// The repos table's items, by name, in insertion order.
fn rows(app: &TestApp) -> Vec<String> {
    let table = app.app().main_screen().unwrap().repos().table();
    table.items().map(|repo| repo.full_name.clone()).collect()
}

fn loading(app: &TestApp) -> bool {
    app.app().main_screen().unwrap().repos().table().is_loading()
}

/// The cache writes among `app`'s effects.
fn writes(app: &TestApp) -> Vec<(PathBuf, String)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::WriteCache { path, text } => Some((path.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

fn local(full_name: &str) -> Option<RepoName> {
    RepoName::from_remote_url(&format!("git@github.com:{full_name}.git"))
}

fn current_repo(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

/// Answers every queued request, and returns what that logged.
fn settle(app: &mut TestApp) -> String {
    logs(|| app.settle().now_or_never().expect("the fake answers at once"))
}

/// The app at startup, `repos.json` holding [`cached`] and the list request still out.
fn with_cache(gh: FakeGithub) -> TestApp {
    TestApp::loading(config(), gh).with_cache_file(cache_path(), cache::to_json(cached()))
}

#[test]
fn start_reads_the_cache_file_alongside_the_list() {
    let mut app = TestApp::new(config());
    let mut section = ReposSection::new(&Theme::default(), AppOptions::default());
    app.with_cx(|cx| section.start(cx));
    let [
        Effect::Github {
            request: GithubRequest::ListRepositories(_),
            ..
        },
        Effect::ReadCache { path, .. },
    ] = app.effects()
    else {
        panic!("{:?}", app.effects());
    };
    assert_eq!(path, &cache_path());
}

#[test]
fn cached_rows_show_under_the_overlay_and_select_the_current_directory_repo() {
    // Git answers first, so it's the cached rows that let the check find the repo.
    let mut app = with_cache(FakeGithub::new().with_repositories(listed()));
    let git = app
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Git { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.send(Event::Git {
        id: git,
        response: GitResponse::OriginRepo(local("parity-bot/cache-fixture")),
    });
    assert_eq!(current_repo(&app), None);
    let out = logs(|| {
        app.answer_setup_cache();
    });
    assert_eq!(rows(&app), ["parity-bot/cache-fixture", "parity-bot/dotfiles"]);
    assert!(loading(&app));
    assert!(out.contains("INFO Selected repo parity-bot/cache-fixture"), "{out}");
    assert_eq!(current_repo(&app), Some("parity-bot/cache-fixture"));
    // The selection's lists start loading before the repos list arrives.
    assert!(
        app.effects().iter().any(|effect| matches!(
            effect,
            Effect::Github {
                request: GithubRequest::ListIssues { repo, .. },
                ..
            } if repo.full_name == "parity-bot/cache-fixture"
        )),
        "{:?}",
        app.effects()
    );
    assert_eq!(writes(&app), []);
}

#[test]
fn the_list_replaces_the_cached_rows_and_writes_the_file() {
    let mut app = with_cache(FakeGithub::new().with_repositories(listed()));
    app.answer_setup_cache();
    settle(&mut app);
    assert!(!loading(&app));
    assert_eq!(
        rows(&app),
        ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
    );
    let text = cache::to_json(listed());
    assert_eq!(writes(&app), [(cache_path(), text.clone())]);
    assert_eq!(app.cache_file(&cache_path()), Some(text.as_str()));
}

#[test]
fn the_cache_the_list_and_git_select_once() {
    let mut app =
        with_cache(FakeGithub::new().with_repositories(listed())).with_local_repo(local("parity-bot/dotfiles"));
    let out = settle(&mut app);
    let selected: Vec<&str> = out.lines().filter(|line| line.contains("Selected repo")).collect();
    assert_eq!(selected, ["INFO Selected repo parity-bot/dotfiles"]);
    assert_eq!(current_repo(&app), Some("parity-bot/dotfiles"));
}

#[test]
fn a_failed_list_keeps_the_cached_rows_and_writes_nothing() {
    let error = GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    };
    let mut app = with_cache(FakeGithub::new().with_repositories_error(error));
    let before = cache::to_json(cached());
    settle(&mut app);
    assert!(!loading(&app));
    assert_eq!(rows(&app), ["parity-bot/cache-fixture", "parity-bot/dotfiles"]);
    assert_eq!(writes(&app), []);
    assert_eq!(app.cache_file(&cache_path()), Some(before.as_str()));
}

#[test]
fn a_read_after_the_list_is_dropped() {
    let mut app = with_cache(FakeGithub::new().with_repositories(listed()));
    app.answer_repos();
    let out = logs(|| {
        app.answer_setup_cache();
    });
    assert_eq!(out, "DEBUG Dropping the cached repos: the list already arrived\n");
    assert_eq!(
        rows(&app),
        ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
    );
}

#[test]
fn a_missing_file_shows_nothing_and_a_bad_one_warns() {
    let mut app = TestApp::loading(config(), FakeGithub::new().with_repositories(listed()));
    let out = logs(|| {
        app.answer_setup_cache();
    });
    assert_eq!(out, "");
    assert_eq!(rows(&app), Vec::<String>::new());

    let mut app = TestApp::loading(config(), FakeGithub::new()).with_cache_file(cache_path(), "[{\"name\": 1}]");
    let out = logs(|| {
        app.answer_setup_cache();
    });
    let prefix = format!(
        "WARN Cache file '{}' contains data that doesn't match the expected schema for Repository.",
        cache_path().display()
    );
    assert!(out.starts_with(&prefix), "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
    assert_eq!(rows(&app), Vec::<String>::new());
    assert!(loading(&app));
}

#[test]
fn add_writes_the_table_with_the_new_repo() {
    let mut app = TestApp::new(config());
    let mut section = ReposSection::new(&Theme::default(), AppOptions::default());
    let added = repo("someone", "else", false);
    app.with_cx(|cx| {
        section.table.replace_all(listed(), &super::RepoCells::from_cx(cx));
        section.add(added.clone(), cx);
    });
    let mut expected = listed();
    expected.push(added);
    assert_eq!(writes(&app), [(cache_path(), cache::to_json(&expected))]);
    assert_eq!(section.table().row_count(), 4);
}

/// The app at startup with two tracked repos, one found and one not, `repos.json` holding [`cached`] and the list
/// request still out.
fn tracking() -> TestApp {
    let mut config = config();
    config.repositories.additional_repos_to_track = vec!["octo/tracked".into(), "octo/gone".into()];
    let gh = FakeGithub::new()
        .with_repositories(listed())
        .with_repository("octo/tracked", Ok(Some(repo("octo", "tracked", false))))
        .with_repository("octo/gone", Ok(None));
    TestApp::loading(config, gh).with_cache_file(cache_path(), cache::to_json(cached()))
}

#[test]
fn the_file_is_written_once_the_tracked_repos_are_in() {
    let mut app = tracking();
    app.answer_setup_cache();
    // The list is in, the tracked repos aren't: the cached rows and the file stay.
    app.answer_repos();
    assert!(loading(&app));
    assert_eq!(rows(&app), ["parity-bot/cache-fixture", "parity-bot/dotfiles"]);
    assert_eq!(writes(&app), []);
    settle(&mut app);
    let mut expected = listed();
    expected.push(repo("octo", "tracked", false));
    assert_eq!(writes(&app), [(cache_path(), cache::to_json(&expected))]);
    assert!(!loading(&app));
}

#[test]
fn a_read_while_the_tracked_repos_are_out_still_shows() {
    let mut app = tracking();
    app.answer_repos();
    app.answer_setup_cache();
    assert_eq!(rows(&app), ["parity-bot/cache-fixture", "parity-bot/dotfiles"]);
    assert!(loading(&app));
    settle(&mut app);
    assert_eq!(
        rows(&app),
        [
            "parity-bot/dotfiles",
            "parity-bot/hello-world",
            "parity-bot/lazy-demo",
            "octo/tracked"
        ]
    );
}

#[test]
fn a_read_after_a_failed_list_still_shows() {
    let error = GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: Vec::new(),
        headers: Vec::new(),
    };
    let mut app = with_cache(FakeGithub::new().with_repositories_error(error));
    app.answer_repos();
    assert!(!loading(&app));
    app.answer_setup_cache();
    assert_eq!(rows(&app), ["parity-bot/cache-fixture", "parity-bot/dotfiles"]);
    assert_eq!(writes(&app), []);
}

/// Python's `action_toggle_favorite_repo` saves the config and re-sorts, but never writes the cache.
#[test]
fn toggling_a_favorite_writes_nothing() {
    let mut app = with_cache(FakeGithub::new().with_repositories(listed()));
    settle(&mut app);
    let before = writes(&app).len();
    app.press(&["ctrl+f"]);
    let favorites = app.with_cx(|cx| cx.config().repositories.favorites.clone());
    assert_eq!(favorites, ["parity-bot/dotfiles"]);
    assert_eq!(writes(&app).len(), before);
}
