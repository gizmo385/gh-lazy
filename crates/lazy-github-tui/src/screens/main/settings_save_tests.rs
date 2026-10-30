//! Settings' Save, applied by `App`: the one config write, the toast, the theme, the keymap, the shown sections and
//! their loads (Py bugs B91, B92), focus and the maximized layout (Py bug B88), the notifications poll and the `gh`
//! options.

use std::time::Duration;

use futures_util::FutureExt as _;
use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GhCliOptions};
use lazy_github_core::models::Repository;

use super::pull_requests::{Kind, item};
use super::repos::repo;
use super::workflows::run;
use super::*;
use crate::components::status_bar::UnreadState;
use crate::effect::TimerId;
use crate::modal::SettingsOutcome;
use crate::notifications::REFRESH_INTERVAL;
use crate::test_util::TestApp;
use crate::theme::{ColorDepth, ColorMode, Theme};
use crate::toast::{Severity, Toast};

const REPOS: FocusTarget = FocusTarget::Table(SectionId::Repos);
const ISSUES: FocusTarget = FocusTarget::Table(SectionId::Issues);

fn main(app: &TestApp) -> &MainScreen {
    app.app().main_screen().expect("on the main screen")
}

fn shown(app: &TestApp, id: SectionId) -> bool {
    main(app).section(id).shown()
}

/// Saves `config` from the Settings modal, as its Save button closes it.
fn save(app: &mut TestApp, config: &Config) {
    app.dispatch(Action::SettingsClosed(SettingsOutcome::Saved(Box::new(config.clone()))));
}

/// The app's config with `f` applied, as the modal would hand it back.
fn edited(app: &TestApp, f: impl FnOnce(&mut Config)) -> Config {
    let mut config = app.app().config().clone();
    f(&mut config);
    config
}

fn saves(app: &TestApp, from: usize) -> Vec<Config> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::SaveConfig(config) => Some((**config).clone()),
            _ => None,
        })
        .collect()
}

fn reconfigures(app: &TestApp, from: usize) -> Vec<GhCliOptions> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::ReconfigureGithub(options) => Some(options.clone()),
            _ => None,
        })
        .collect()
}

fn intervals(app: &TestApp, from: usize) -> usize {
    app.effects()[from..]
        .iter()
        .filter(|effect| {
            **effect
                == Effect::StartInterval {
                    id: TimerId::Notifications,
                    every: REFRESH_INTERVAL,
                }
        })
        .count()
}

fn auth_checks(app: &TestApp, from: usize) -> usize {
    app.effects()[from..]
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Github {
                    request: GithubRequest::CheckAuth,
                    ..
                }
            )
        })
        .count()
}

/// The requests and cache reads in the effects from `from` on, as short names.
fn loads(app: &TestApp, from: usize) -> Vec<&'static str> {
    app.effects()[from..]
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. } => match request {
                GithubRequest::ListIssues { .. } => Some("issues"),
                GithubRequest::ListWorkflows { .. } => Some("workflows"),
                GithubRequest::ListWorkflowRuns { .. } => Some("runs"),
                _ => None,
            },
            Effect::ReadCache { path, .. } => Some(if path.to_string_lossy().ends_with("_workflow_runs.json") {
                "runs cache"
            } else {
                "other cache"
            }),
            _ => None,
        })
        .collect()
}

fn settle(app: &mut TestApp) {
    app.settle().now_or_never().expect("the fake answers at once");
}

fn one() -> Repository {
    repo("octo", "one", false)
}

/// An app with `octo/one` selected and every request answered: two PRs and an issue, and a run.
fn selected(config: Config) -> TestApp {
    let repo = one();
    let gh = FakeGithub::new()
        .with_repositories(vec![repo.clone()])
        .with_issue_page(
            "octo/one",
            1,
            Ok(vec![
                item(&repo, 3, Kind::OpenPr),
                item(&repo, 2, Kind::Issue),
                item(&repo, 1, Kind::OpenPr),
            ]),
        )
        .with_run_page(
            "octo/one",
            1,
            Ok(vec![run(
                &repo,
                1,
                "CI",
                1,
                "2026-10-20T09:00:00Z",
                Some("success"),
                "Run",
            )]),
        );
    let mut app = TestApp::with_github(config, gh);
    app.dispatch(Action::SelectRepo(Box::new(repo)));
    settle(&mut app);
    app
}

fn appearance(f: impl FnOnce(&mut AppearanceSettings)) -> Config {
    let mut config = Config::default();
    f(&mut config.appearance);
    config
}

#[test]
fn a_save_writes_the_edited_config_once_and_toasts() {
    let mut app = TestApp::new(Config::default());
    let config = edited(&app, |c| {
        c.appearance.show_issues = false;
        c.pull_requests.preload_pull_request_for_current_commit = false;
    });
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(saves(&app, from), std::slice::from_ref(&config));
    assert_eq!(app.app().config(), &config);
    let expected = Toast::new(Severity::Information, "Settings updated");
    assert!(app.app().toasts().iter().eq([&expected]));
}

#[test]
fn an_unchanged_save_still_writes_and_toasts() {
    let mut app = TestApp::new(Config::default());
    let config = app.app().config().clone();
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(saves(&app, from), [config]);
    assert!(
        app.app()
            .toasts()
            .iter()
            .any(|toast| *toast == Toast::new(Severity::Information, "Settings updated"))
    );
}

#[test]
fn cancel_applies_nothing() {
    let mut app = TestApp::new(Config::default());
    let from = app.effects().len();
    app.dispatch(Action::SettingsClosed(SettingsOutcome::Cancelled));
    assert_eq!(&app.effects()[from..], []);
    assert_eq!(app.app().toasts().iter().count(), 0);
}

#[test]
fn the_theme_follows_keeping_the_color_mode_and_replacing_a_palette_pick() {
    let mode = ColorMode {
        depth: ColorDepth::EightBit,
        no_color: false,
    };
    let mut app = TestApp::with_theme(Config::default(), Theme::default().with_color_mode(mode));
    app.dispatch(Action::SetTheme("dracula"));
    assert_eq!(app.app().theme().name, "dracula");
    let config = edited(&app, |c| "nord".clone_into(&mut c.appearance.theme));
    save(&mut app, &config);
    assert_eq!(app.app().theme().name, "nord");
    assert_eq!(app.app().theme().color_mode(), mode);

    // An unchanged theme still replaces the palette's.
    app.dispatch(Action::SetTheme("dracula"));
    save(&mut app, &config);
    assert_eq!(app.app().theme().name, "nord");
}

#[test]
fn a_removed_override_restores_the_default_key_and_titles_keep_their_text() {
    let mut config = Config::default();
    config.bindings.overrides.insert("main.issues.focus".into(), "i".into());
    let mut app = TestApp::new(config);
    assert!(app.screen(160, 48).contains("[i] Issues"));
    app.press(&["i"]);
    assert_eq!(main(&app).focus(), ISSUES);
    app.press(&["1"]);

    let config = edited(&app, |c| {
        c.bindings.overrides.shift_remove("main.issues.focus");
    });
    save(&mut app, &config);
    app.press(&["i"]);
    assert_eq!(main(&app).focus(), REPOS);
    app.press(&["3"]);
    assert_eq!(main(&app).focus(), ISSUES);
    // The title was built with the old key and keeps it until restart, as in Python.
    assert!(app.screen(160, 48).contains("[i] Issues"));
}

#[test]
fn a_section_the_palette_hid_comes_back_fully_and_takes_its_key() {
    let mut app = TestApp::new(Config::default());
    app.dispatch(Action::Toggle(Toggle::Issues));
    assert!(main(&app).section(SectionId::Issues).hidden_by_toggle);
    let config = app.app().config().clone();
    save(&mut app, &config);
    let state = main(&app).section(SectionId::Issues);
    assert!(state.shown());
    assert!(!state.hidden_by_toggle);
    assert!(main(&app).focus_ring(None).contains(&ISSUES));
    app.press(&["3"]);
    assert_eq!(main(&app).focus(), ISSUES);
}

#[test]
fn a_section_the_palette_showed_follows_the_config() {
    let mut app = TestApp::new(appearance(|a| a.show_workflows = false));
    app.dispatch(Action::Toggle(Toggle::Workflows));
    assert!(shown(&app, SectionId::Workflows));
    let config = app.app().config().clone();
    save(&mut app, &config);
    assert!(!shown(&app, SectionId::Workflows));
}

#[test]
fn a_section_shown_with_a_repo_selected_loads_once() {
    let mut app = selected(appearance(|a| {
        a.show_workflows = false;
        a.show_issues = false;
        a.show_pull_requests = false;
    }));
    let config = edited(&app, |c| {
        c.appearance.show_workflows = true;
        c.appearance.show_issues = true;
        c.appearance.show_pull_requests = true;
    });
    let from = app.effects().len();
    save(&mut app, &config);
    // One page for both tables, as the palette toggle loads them.
    assert_eq!(loads(&app, from), ["issues", "runs cache", "workflows", "runs"]);
    settle(&mut app);
    assert_eq!(main(&app).issue_lists.issues_table().row_count(), 1);
    assert_eq!(main(&app).issue_lists.prs_table().row_count(), 2);
    assert_eq!(main(&app).workflows.runs_table().row_count(), 1);

    // Already shown: nothing more.
    let from = app.effects().len();
    save(&mut app, &config);
    assert!(loads(&app, from).is_empty());
}

#[test]
fn a_section_shown_without_a_repo_requests_nothing() {
    let mut app = TestApp::new(appearance(|a| {
        a.show_workflows = false;
        a.show_issues = false;
    }));
    let config = edited(&app, |c| {
        c.appearance.show_workflows = true;
        c.appearance.show_issues = true;
    });
    let from = app.effects().len();
    save(&mut app, &config);
    assert!(shown(&app, SectionId::Workflows));
    assert!(shown(&app, SectionId::Issues));
    assert!(loads(&app, from).is_empty());
}

#[test]
fn hiding_the_focused_section_focuses_the_first_ring_stop() {
    let mut app = TestApp::new(Config::default());
    app.press(&["3"]);
    let config = edited(&app, |c| c.appearance.show_issues = false);
    save(&mut app, &config);
    assert!(!shown(&app, SectionId::Issues));
    assert_eq!(main(&app).focus(), REPOS);

    // An unfocused section hidden leaves focus alone.
    let mut app = TestApp::new(Config::default());
    app.press(&["2"]);
    save(&mut app, &config);
    assert_eq!(main(&app).focus(), FocusTarget::Table(SectionId::PullRequests));
}

#[test]
fn hiding_the_maximized_section_restores_the_layout() {
    let mut app = TestApp::new(Config::default());
    app.press(&["3", "M"]);
    assert!(app.app().maximized().is_some());
    let config = edited(&app, |c| c.appearance.show_issues = false);
    save(&mut app, &config);
    assert!(app.app().maximized().is_none());
    assert_eq!(main(&app).focus(), REPOS);

    // Another section maximized stays so.
    let mut app = TestApp::new(Config::default());
    app.press(&["2", "M"]);
    save(&mut app, &config);
    assert_eq!(
        app.app().maximized().map(|m| m.target),
        Some(FocusTarget::Table(SectionId::PullRequests))
    );
}

#[test]
fn the_command_log_is_untouched() {
    let mut app = TestApp::new(Config::default());
    app.dispatch(Action::Toggle(Toggle::CommandLog));
    assert!(main(&app).command_log().is_visible());
    let config = app.app().config().clone();
    save(&mut app, &config);
    assert!(main(&app).command_log().is_visible());
}

#[test]
fn turning_notifications_on_refreshes_now_and_starts_the_interval_once() {
    let mut app = TestApp::with_github(Config::default(), FakeGithub::new().with_unread_count(Ok(3)));
    assert_eq!(intervals(&app, 0), 0);
    let config = edited(&app, |c| c.notifications.enabled = true);
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(auth_checks(&app, from), 1);
    assert_eq!(intervals(&app, from), 1);
    settle(&mut app);
    assert_eq!(app.app().unread(), UnreadState::Count(3));

    // A second save refreshes again, but the interval is already running.
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(auth_checks(&app, from), 1);
    assert_eq!(intervals(&app, from), 0);
    settle(&mut app);

    // The interval ticks.
    let from = app.effects().len();
    app.advance(Duration::from_mins(1));
    assert_eq!(auth_checks(&app, from), 1);
}

#[test]
fn notifications_on_at_startup_keep_the_one_interval() {
    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::with_github(config, FakeGithub::new().with_unread_count(Ok(3)));
    settle(&mut app);
    assert_eq!(intervals(&app, 0), 1);
    let config = app.app().config().clone();
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(auth_checks(&app, from), 1);
    assert_eq!(intervals(&app, from), 0);
}

#[test]
fn turning_notifications_off_blanks_the_count() {
    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::with_github(config, FakeGithub::new().with_unread_count(Ok(3)));
    settle(&mut app);
    assert_eq!(app.app().unread(), UnreadState::Count(3));
    let config = edited(&app, |c| c.notifications.enabled = false);
    let from = app.effects().len();
    save(&mut app, &config);
    assert_eq!(auth_checks(&app, from), 0);
    assert_eq!(app.app().unread(), UnreadState::Hidden);
}

#[test]
fn the_gh_options_are_rebuilt_only_when_a_ttl_or_the_host_changes() {
    let mut app = TestApp::new(Config::default());
    // Fields `gh` doesn't use.
    let config = edited(&app, |c| {
        c.appearance.show_issues = false;
        c.cache.auth_cache_duration = 5;
        c.cache.cache_directory = "/elsewhere".into();
    });
    let from = app.effects().len();
    save(&mut app, &config);
    assert!(reconfigures(&app, from).is_empty());

    for change in [
        (|c: &mut Config| c.cache.default_ttl = 7) as fn(&mut Config),
        |c| c.cache.list_repos_ttl = 7,
        |c| c.cache.list_issues_ttl = 7,
        |c| "https://ghe.example.com/api/v3".clone_into(&mut c.api.base_url),
    ] {
        let config = edited(&app, change);
        let from = app.effects().len();
        save(&mut app, &config);
        assert_eq!(reconfigures(&app, from), [GhCliOptions::from(&config)]);
    }
}

#[test]
fn the_gh_options_change_before_the_requests_the_save_makes() {
    let mut app = selected(appearance(|a| a.show_issues = false));
    let config = edited(&app, |c| {
        c.appearance.show_issues = true;
        c.notifications.enabled = true;
        c.cache.default_ttl = 7;
    });
    let from = app.effects().len();
    save(&mut app, &config);
    let effects = &app.effects()[from..];
    let reconfigure = effects
        .iter()
        .position(|effect| matches!(effect, Effect::ReconfigureGithub(_)))
        .expect("reconfigured");
    let first_request = effects
        .iter()
        .position(|effect| matches!(effect, Effect::Github { .. }))
        .expect("requested");
    assert!(reconfigure < first_request);
}

#[test]
fn saved_nord_without_issues_160x48() {
    let mut app = TestApp::new(Config::default());
    let config = edited(&app, |c| {
        "nord".clone_into(&mut c.appearance.theme);
        c.appearance.show_issues = false;
    });
    save(&mut app, &config);
    insta::assert_snapshot!(crate::test_util::buffer_to_styled_string(&app.render(160, 48)));
}
