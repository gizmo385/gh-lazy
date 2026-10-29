use std::path::{Path, PathBuf};

use jiff::civil::DateTime;
use rstest::rstest;
use serde_json::Value;

use super::*;
use crate::config::ConfigPaths;

fn paths() -> ConfigPaths {
    ConfigPaths::in_dir(Path::new("/home/u/.config/lazy-github"))
}

fn config() -> Config {
    Config::default_in(&paths())
}

fn field(section: &str, name: &str) -> &'static Field {
    SECTIONS
        .iter()
        .flat_map(|s| s.fields)
        .find(|f| f.section == section && f.name == name)
        .unwrap_or_else(|| panic!("no field {section}.{name}"))
}

fn text(s: &str) -> FieldValue {
    FieldValue::Text(s.to_owned())
}

#[test]
fn sections_are_in_config_order_with_python_titles_and_docstrings() {
    let sections: Vec<_> = SECTIONS.iter().map(|s| (s.key, s.title, s.doc)).collect();
    assert_eq!(
        sections,
        [
            (
                "appearance",
                "Appearance",
                Some(
                    "Settings focused on altering the appearance of LazyGithub, including hiding or showing different sections."
                )
            ),
            (
                "notifications",
                "Notifications",
                Some(
                    "Controls the settings for the optional notification feature, which relies on the standard GitHub CLI."
                )
            ),
            (
                "bindings",
                "Key Bindings",
                Some(
                    "Custom keybinding overrides for LazyGithub. When rebinding, pressing ESCAPE will reset to the default binding."
                )
            ),
            ("repositories", "Repositories", Some("Repository-specific settings")),
            (
                "pull_requests",
                "Pull Requests",
                Some("Changes how pull requests are retrieved from the Github API")
            ),
            (
                "issues",
                "Issues",
                Some("Changes how issues are retrieved from the Github API")
            ),
            (
                "cache",
                "Cache",
                Some("Settings that control how long data will be cached from the GitHub API")
            ),
            ("core", "Core", None),
            (
                "api",
                "Api",
                Some("Controlling how the GitHub API is accessed in LazyGithub")
            ),
        ]
    );
}

#[test]
fn tab_titles_are_the_title_cased_keys_except_bindings() {
    for section in SECTIONS.iter().filter(|s| s.key != "bindings") {
        assert_eq!(
            section.title,
            py_title(&section.key.replace('_', " ")),
            "{}",
            section.key
        );
    }
}

#[test]
fn fields_are_in_file_order_with_their_kinds() {
    use FieldKind as K;
    use TextKind as T;

    let fields: Vec<_> = SECTIONS
        .iter()
        .flat_map(|s| s.fields)
        .map(|f| (format!("{}.{}", f.section, f.name), f.kind))
        .collect();
    let expected = [
        ("appearance.theme", K::Theme),
        ("appearance.show_command_log", K::Bool),
        ("appearance.show_workflows", K::Bool),
        ("appearance.show_issues", K::Bool),
        ("appearance.show_pull_requests", K::Bool),
        ("notifications.enabled", K::Bool),
        ("notifications.mark_notification_as_read_when_selected", K::Bool),
        ("repositories.additional_repos_to_track", K::List),
        ("repositories.favorites", K::List),
        ("pull_requests.state_filter", K::Choice(STATE_CHOICES)),
        ("pull_requests.owner_filter", K::Choice(OWNER_CHOICES)),
        ("pull_requests.preferred_merge_method", K::Choice(MERGE_CHOICES)),
        ("pull_requests.additional_suggested_pr_reviewers", K::List),
        ("pull_requests.preload_pull_request_for_current_commit", K::Bool),
        ("pull_requests.pull_request_template", K::OptionalPath),
        ("issues.state_filter", K::Choice(STATE_CHOICES)),
        ("issues.owner_filter", K::Choice(OWNER_CHOICES)),
        ("cache.auth_cache_duration", K::Text(T::Integer)),
        ("cache.auth_last_checked", K::Text(T::Timestamp)),
        ("cache.cache_directory", K::Text(T::Path)),
        ("cache.default_ttl", K::Text(T::Integer)),
        ("cache.list_repos_ttl", K::Text(T::Integer)),
        ("cache.list_issues_ttl", K::Text(T::Integer)),
        ("core.first_start", K::Bool),
        ("core.logfile_path", K::Text(T::Path)),
        ("core.logfile_max_bytes", K::Text(T::Integer)),
        ("core.logfile_count", K::Text(T::Integer)),
        ("api.base_url", K::Text(T::Plain)),
        ("api.client_type", K::ReadOnlyChoice(CLIENT_TYPE_CHOICES)),
    ];
    let expected: Vec<_> = expected.into_iter().map(|(key, kind)| (key.to_owned(), kind)).collect();
    assert_eq!(fields, expected);
    for section in SECTIONS {
        assert!(
            section.fields.iter().all(|f| f.section == section.key),
            "{}",
            section.key
        );
    }
}

/// A field added to `config.rs` fails here until it has a descriptor.
#[test]
fn every_config_field_but_binding_overrides_has_exactly_one_descriptor() {
    let Value::Object(json) = serde_json::to_value(config()).unwrap() else {
        panic!("config serializes to an object")
    };
    let mut keys = Vec::new();
    for (section, value) in &json {
        let Value::Object(fields) = value else {
            panic!("{section} is a section")
        };
        keys.extend(fields.keys().map(|name| format!("{section}.{name}")));
    }
    keys.retain(|key| key != "bindings.overrides");
    keys.sort();

    let mut described: Vec<_> = SECTIONS
        .iter()
        .flat_map(|s| s.fields)
        .map(|f| format!("{}.{}", f.section, f.name))
        .collect();
    described.sort();
    // `client_type` lives in `api.extra`, and a default config carries it.
    assert_eq!(described, keys);

    let sections: Vec<_> = SECTIONS.iter().map(|s| s.key).collect();
    assert_eq!(sections, json.keys().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn labels_are_python_title_case_with_a_colon() {
    let labels: Vec<_> = SECTIONS.iter().flat_map(|s| s.fields).map(Field::label).collect();
    assert_eq!(
        labels,
        [
            "Theme:",
            "Show Command Log:",
            "Show Workflows:",
            "Show Issues:",
            "Show Pull Requests:",
            "Enabled:",
            "Mark Notification As Read When Selected:",
            "Additional Repos To Track:",
            "Favorites:",
            "State Filter:",
            "Owner Filter:",
            "Preferred Merge Method:",
            "Additional Suggested Pr Reviewers:",
            "Preload Pull Request For Current Commit:",
            "Pull Request Template:",
            "State Filter:",
            "Owner Filter:",
            "Auth Cache Duration:",
            "Auth Last Checked:",
            "Cache Directory:",
            "Default Ttl:",
            "List Repos Ttl:",
            "List Issues Ttl:",
            "First Start:",
            "Logfile Path:",
            "Logfile Max Bytes:",
            "Logfile Count:",
            "Base Url:",
            "Client Type:",
        ]
    );
}

#[rstest]
#[case("raw_http", "Raw_Http")]
#[case("GITHUB_CLI", "Github_Cli")]
#[case("a1b c-d", "A1B C-D")]
fn py_title_capitalizes_after_any_non_letter(#[case] text: &str, #[case] titled: &str) {
    assert_eq!(py_title(text), titled);
}

#[test]
fn choices_are_the_enums_in_order_labelled_by_title_case() {
    fn values(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.value).collect()
    }
    assert_eq!(
        values(STATE_CHOICES),
        IssueStateFilter::ALL.map(IssueStateFilter::as_str)
    );
    assert_eq!(
        values(OWNER_CHOICES),
        IssueOwnerFilter::ALL.map(IssueOwnerFilter::as_str)
    );
    assert_eq!(values(MERGE_CHOICES), MergeMethod::ALL.map(MergeMethod::as_str));
    assert_eq!(values(CLIENT_TYPE_CHOICES), ["RAW_HTTP", "GITHUB_CLI"]);
    for choice in [STATE_CHOICES, OWNER_CHOICES, MERGE_CHOICES, CLIENT_TYPE_CHOICES].concat() {
        assert_eq!(choice.label, py_title(choice.value));
    }
    let labels = |choices: &[Choice]| choices.iter().map(|c| c.label).collect::<Vec<_>>();
    assert_eq!(labels(STATE_CHOICES), ["All", "Open", "Closed"]);
    assert_eq!(labels(OWNER_CHOICES), ["Mine", "All"]);
    assert_eq!(labels(MERGE_CHOICES), ["Merge", "Squash", "Rebase"]);
    assert_eq!(labels(CLIENT_TYPE_CHOICES), ["Raw_Http", "Github_Cli"]);
}

#[test]
fn enum_values_round_trip_through_serde() {
    for filter in IssueStateFilter::ALL {
        assert_eq!(serde_json::to_value(filter).unwrap(), filter.as_str());
    }
    for filter in IssueOwnerFilter::ALL {
        assert_eq!(serde_json::to_value(filter).unwrap(), filter.as_str());
    }
    for method in MergeMethod::ALL {
        assert_eq!(serde_json::to_value(method).unwrap(), method.as_str());
    }
}

#[test]
fn get_shows_each_kind() {
    let c = config();
    assert_eq!(field("appearance", "theme").get(&c), FieldValue::Choice("textual-dark"));
    assert_eq!(field("appearance", "show_issues").get(&c), FieldValue::Bool(true));
    assert_eq!(field("repositories", "favorites").get(&c), FieldValue::List(vec![]));
    assert_eq!(
        field("pull_requests", "preferred_merge_method").get(&c),
        FieldValue::Choice("squash")
    );
    assert_eq!(field("pull_requests", "pull_request_template").get(&c), text(""));
    assert_eq!(field("cache", "auth_cache_duration").get(&c), text("86400"));
    assert_eq!(field("cache", "auth_last_checked").get(&c), text("None"));
    assert_eq!(
        field("cache", "cache_directory").get(&c),
        FieldValue::Text(paths().dir.join(".cache").to_string_lossy().into_owned())
    );
    assert_eq!(field("api", "base_url").get(&c), text("https://api.github.com"));
    assert_eq!(field("api", "client_type").get(&c), FieldValue::Choice("GITHUB_CLI"));

    let mut c = config();
    c.appearance.theme = "rose-pine-moon".to_owned();
    c.repositories.favorites = vec!["b/b".to_owned(), "a/a".to_owned()];
    c.pull_requests.pull_request_template = Some(PathBuf::from("/home/u/template.md"));
    c.cache.auth_last_checked = Some("2026-10-24T18:30:05".parse().unwrap());
    c.cache.cache_directory = PathBuf::from("/home/u/.config/lazy-github/.cache");
    c.core.logfile_count = -3;
    assert_eq!(
        field("appearance", "theme").get(&c),
        FieldValue::Choice("rose-pine-moon")
    );
    assert_eq!(
        field("repositories", "favorites").get(&c),
        FieldValue::List(vec!["b/b".to_owned(), "a/a".to_owned()])
    );
    assert_eq!(
        field("pull_requests", "pull_request_template").get(&c),
        text("/home/u/template.md")
    );
    assert_eq!(field("cache", "auth_last_checked").get(&c), text("2026-10-24 18:30:05"));
    assert_eq!(
        field("cache", "cache_directory").get(&c),
        text("/home/u/.config/lazy-github/.cache")
    );
    assert_eq!(field("core", "logfile_count").get(&c), text("-3"));

    c.cache.auth_last_checked = Some("2026-10-24T18:30:05.123456".parse().unwrap());
    assert_eq!(
        field("cache", "auth_last_checked").get(&c),
        text("2026-10-24 18:30:05.123456")
    );
}

#[test]
fn client_type_shows_github_cli_whatever_the_file_says() {
    let mut c = config();
    c.api
        .extra
        .insert("client_type".to_owned(), Value::String("RAW_HTTP".to_owned()));
    assert_eq!(field("api", "client_type").get(&c), FieldValue::Choice("GITHUB_CLI"));
}

#[test]
fn set_writes_bools_choices_themes_and_lists() {
    let mut c = config();
    field("appearance", "show_command_log")
        .set(&mut c, FieldValue::Bool(true))
        .unwrap();
    field("appearance", "theme")
        .set(&mut c, FieldValue::Choice("nord"))
        .unwrap();
    field("pull_requests", "preferred_merge_method")
        .set(&mut c, FieldValue::Choice("rebase"))
        .unwrap();
    field("pull_requests", "owner_filter")
        .set(&mut c, FieldValue::Choice("mine"))
        .unwrap();
    let items = vec!["z/z".to_owned(), "a/a".to_owned()];
    field("repositories", "additional_repos_to_track")
        .set(&mut c, FieldValue::List(items.clone()))
        .unwrap();
    assert!(c.appearance.show_command_log);
    assert_eq!(c.appearance.theme, "nord");
    assert_eq!(c.pull_requests.preferred_merge_method, MergeMethod::Rebase);
    assert_eq!(c.pull_requests.owner_filter, IssueOwnerFilter::Mine);
    assert_eq!(c.repositories.additional_repos_to_track, items);
}

#[rstest]
#[case("600", 600)]
#[case(" 600 ", 600)]
#[case("1_000", 1000)]
#[case("-5", -5)]
fn set_integer_parses_as_the_loader_does(#[case] input: &str, #[case] expected: i64) {
    let mut c = config();
    field("cache", "default_ttl").set(&mut c, text(input)).unwrap();
    assert_eq!(c.cache.default_ttl, expected);
}

#[rstest]
#[case("abc")]
#[case("")]
#[case("1.5")]
#[case("1e3")]
fn set_integer_refuses_other_text(#[case] input: &str) {
    let mut c = config();
    let before = c.clone();
    assert_eq!(
        field("core", "logfile_count").set(&mut c, text(input)),
        Err(FieldError::NotAWholeNumber)
    );
    assert_eq!(c, before);
}

#[rstest]
#[case("None", None)]
#[case("", None)]
#[case("  ", None)]
#[case("2026-10-24 18:30:05", Some("2026-10-24T18:30:05"))]
#[case("2026-10-24T18:30:05.123456", Some("2026-10-24T18:30:05.123456"))]
fn set_timestamp(#[case] input: &str, #[case] expected: Option<&str>) {
    let mut c = config();
    c.cache.auth_last_checked = Some("2020-01-01T00:00:00".parse().unwrap());
    field("cache", "auth_last_checked").set(&mut c, text(input)).unwrap();
    let expected = expected.map(|s| s.parse::<DateTime>().unwrap());
    assert_eq!(c.cache.auth_last_checked, expected);
}

#[test]
fn set_timestamp_refuses_other_text() {
    let mut c = config();
    assert_eq!(
        field("cache", "auth_last_checked").set(&mut c, text("yesterday")),
        Err(FieldError::NotATimestamp)
    );
    assert_eq!(c.cache.auth_last_checked, None);
}

#[test]
fn set_path_trims_and_refuses_blank() {
    let mut c = config();
    field("core", "logfile_path")
        .set(&mut c, text("  /tmp/lg.log "))
        .unwrap();
    assert_eq!(c.core.logfile_path, Path::new("/tmp/lg.log"));
    for blank in ["", "   "] {
        assert_eq!(
            field("cache", "cache_directory").set(&mut c, text(blank)),
            Err(FieldError::Empty)
        );
    }
    assert_eq!(c.cache.cache_directory, paths().dir.join(".cache"));
}

#[test]
fn set_template_trims_and_blank_is_unset() {
    let mut c = config();
    let template = field("pull_requests", "pull_request_template");
    template.set(&mut c, text(" docs/pr.md ")).unwrap();
    assert_eq!(c.pull_requests.pull_request_template, Some(PathBuf::from("docs/pr.md")));
    template.set(&mut c, text("  ")).unwrap();
    assert_eq!(c.pull_requests.pull_request_template, None);
}

#[test]
fn set_base_url_keeps_the_text_as_typed() {
    let mut c = config();
    field("api", "base_url")
        .set(&mut c, text(" https://ghe.example.com/api/v3 "))
        .unwrap();
    assert_eq!(c.api.base_url, " https://ghe.example.com/api/v3 ");
}

#[test]
fn set_client_type_changes_nothing() {
    let mut c = config();
    let before = c.clone();
    field("api", "client_type")
        .set(&mut c, FieldValue::Choice("GITHUB_CLI"))
        .unwrap();
    assert_eq!(c, before);
    let json: Value = serde_json::from_str(&c.to_json_pretty()).unwrap();
    assert_eq!(json["api"]["client_type"], "RAW_HTTP");
}

#[test]
fn set_refuses_a_value_of_the_wrong_kind() {
    let mut c = config();
    let before = c.clone();
    assert_eq!(
        field("appearance", "show_issues").set(&mut c, text("true")),
        Err(FieldError::Mismatch)
    );
    assert_eq!(
        field("appearance", "theme").set(&mut c, FieldValue::Choice("not-a-theme")),
        Err(FieldError::Mismatch)
    );
    assert_eq!(
        field("issues", "state_filter").set(&mut c, FieldValue::Choice("mine")),
        Err(FieldError::Mismatch)
    );
    assert_eq!(
        field("cache", "default_ttl").set(&mut c, FieldValue::Bool(true)),
        Err(FieldError::Mismatch)
    );
    assert_eq!(c, before);
}

/// Py bug B1: Python saves the Pull Requests filters into Issues too.
#[test]
fn each_field_writes_only_its_own_section() {
    let mut c = config();
    field("issues", "state_filter")
        .set(&mut c, FieldValue::Choice("closed"))
        .unwrap();
    field("issues", "owner_filter")
        .set(&mut c, FieldValue::Choice("mine"))
        .unwrap();
    assert_eq!(c.issues.state_filter, IssueStateFilter::Closed);
    assert_eq!(c.issues.owner_filter, IssueOwnerFilter::Mine);
    assert_eq!(c.pull_requests.state_filter, IssueStateFilter::All);
    assert_eq!(c.pull_requests.owner_filter, IssueOwnerFilter::All);
}

#[test]
fn error_messages_follow_the_label() {
    assert_eq!(FieldError::NotAWholeNumber.to_string(), "must be a whole number");
    assert_eq!(
        FieldError::NotATimestamp.to_string(),
        "must be a date and time like 2026-10-24 18:30:05, or None"
    );
    assert_eq!(FieldError::Empty.to_string(), "can't be empty");
}

/// Py bug B2: Python writes numbers and `None` as strings.
#[test]
fn saved_values_are_typed_and_load_back_equal() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::in_dir(dir.path().join("lazy-github"));
    let mut c = Config::default_in(&paths);
    c.api.extra.insert("future_key".to_owned(), Value::Bool(true));
    field("cache", "default_ttl").set(&mut c, text(" 600 ")).unwrap();
    field("core", "logfile_max_bytes").set(&mut c, text("1_000")).unwrap();
    field("cache", "auth_last_checked")
        .set(&mut c, text("2026-10-24 18:30:05"))
        .unwrap();
    field("cache", "auth_last_checked").set(&mut c, text("None")).unwrap();
    field("pull_requests", "pull_request_template")
        .set(&mut c, text(""))
        .unwrap();
    field("api", "client_type")
        .set(&mut c, FieldValue::Choice("GITHUB_CLI"))
        .unwrap();

    let json: Value = serde_json::from_str(&c.to_json_pretty()).unwrap();
    assert_eq!(json["cache"]["default_ttl"], Value::from(600));
    assert_eq!(json["core"]["logfile_max_bytes"], Value::from(1000));
    assert_eq!(json["cache"]["auth_last_checked"], Value::Null);
    assert_eq!(json["pull_requests"]["pull_request_template"], Value::Null);
    assert_eq!(json["api"]["client_type"], "RAW_HTTP");
    assert_eq!(json["api"]["future_key"], true);

    c.save(&paths).unwrap();
    assert_eq!(Config::load(&paths).unwrap(), c);

    field("cache", "auth_last_checked")
        .set(&mut c, text("2026-10-24 18:30:05.5"))
        .unwrap();
    c.save(&paths).unwrap();
    let json: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(json["cache"]["auth_last_checked"], "2026-10-24T18:30:05.500000");
    assert_eq!(Config::load(&paths).unwrap(), c);
}
