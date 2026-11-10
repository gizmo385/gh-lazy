//! Config load/save compatibility with the Python app. Fixtures in `tests/fixtures/config/` come from
//! `generate.py` there.
//!
//! Frozen from the Python app (lazy-github 0.6.6) by `crates/lazy-github-core/tests/fixtures/config/generate.py`,
//! removed at cutover (#398; `git log --diff-filter=D -- crates/lazy-github-core/tests/fixtures/config/generate.py`
//! finds its last version). Edit by hand when the Rust behavior changes on purpose.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use jiff::civil::DateTime;
use lazy_github_core::config::{
    ColorDepthSetting, Config, ConfigError, ConfigPaths, IssueOwnerFilter, IssueStateFilter, MergeMethod,
    SearchSettings,
};
use rstest::rstest;
use serde_json::Value;

/// A temporary directory removed on drop. Tests use their own so they can run in parallel.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lazy-github-config-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Config paths in a not-yet-created subfolder.
    fn paths(&self) -> ConfigPaths {
        ConfigPaths::in_dir(self.0.join("lazy-github"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// Writes `text` as the config file in a fresh temp dir and loads it.
fn load_text(text: &str) -> (TempDir, ConfigPaths, Result<Config, ConfigError>) {
    let tmp = TempDir::new();
    let paths = tmp.paths();
    std::fs::create_dir_all(&paths.dir).unwrap();
    std::fs::write(&paths.file, text).unwrap();
    let result = Config::load(&paths);
    (tmp, paths, result)
}

fn load_ok(text: &str) -> (TempDir, ConfigPaths, Config) {
    let (tmp, paths, result) = load_text(text);
    (tmp, paths, result.unwrap())
}

fn dt(s: &str) -> DateTime {
    s.parse().unwrap()
}

#[cfg(unix)]
#[test]
fn default_matches_python_byte_for_byte() {
    let config = Config::default_in(&ConfigPaths::in_dir("/home/lazy/.config/lazy-github"));
    // But for the default theme's name, renamed from Textual's (app-owned-themes D2), the cache TTLs, lowered
    // (cache-ttl-defaults-and-refresh D1 a), and the Rust-only `color_depth` (color-depth-detection).
    let python = with_rust_only_keys(&with_rust_ttls(
        &fixture("python-default.json").replace(r#""theme": "textual-dark""#, r#""theme": "dark""#),
    ));
    assert_eq!(config.to_json_pretty(), python);
}

/// `text`, a config Python saved, with the keys only Rust writes: `appearance.color_depth`'s default, after
/// `show_pull_requests`, the section's last key (decision `color-depth-detection`), and the default `search` section
/// after `api`, the last section (decision `search-issues-and-prs`). Every other byte is unchanged.
fn with_rust_only_keys(text: &str) -> String {
    let last = text
        .find("\n        \"show_pull_requests\": ")
        .expect("the file has show_pull_requests");
    let end = last + 1 + text[last + 1..].find('\n').expect("it isn't the file's last line");
    let text = format!("{},\n        \"color_depth\": \"auto\"{}", &text[..end], &text[end..]);
    let close = text.rfind("\n    }").expect("the file has a last section");
    format!(
        "{}\n    }},\n    \"search\": {{\n        \"saved_queries\": []{}",
        &text[..close],
        &text[close..]
    )
}

/// `text` with Python's default cache TTLs (600 / 86400 / 3600) swapped for Rust's (60 / 3600 / 60), every other
/// byte unchanged (decision `cache-ttl-defaults-and-refresh`).
fn with_rust_ttls(text: &str) -> String {
    let swaps = [
        (r#""default_ttl": 600,"#, r#""default_ttl": 60,"#),
        (r#""list_repos_ttl": 86400,"#, r#""list_repos_ttl": 3600,"#),
        (r#""list_issues_ttl": 3600"#, r#""list_issues_ttl": 60"#),
    ];
    swaps.into_iter().fold(text.to_owned(), |text, (python, rust)| {
        assert!(text.contains(python), "{python} not in {text}");
        text.replacen(python, rust, 1)
    })
}

/// Python's default file, as every Python-saved config holds the old TTL trio: it loads as the new defaults, and a
/// save writes them, every other byte as Python's (decision `cache-ttl-defaults-and-refresh`).
#[cfg(unix)]
#[test]
fn pythons_default_file_round_trips_with_the_new_ttls() {
    let (_tmp, paths, c) = load_ok(&fixture("python-default.json"));
    assert_eq!(
        (c.cache.default_ttl, c.cache.list_repos_ttl, c.cache.list_issues_ttl),
        (60, 3600, 60)
    );
    c.save(&paths).unwrap();
    // The file names the Python default's paths, so they load and save as written.
    assert_eq!(
        std::fs::read_to_string(&paths.file).unwrap(),
        with_rust_only_keys(&with_rust_ttls(&fixture("python-default.json")))
    );
}

/// Any string loads as written and a save writes it back, so a renamed theme's old name (an alias the UI resolves)
/// and a name the UI doesn't know both survive (app-owned-themes D2, D5). Other scalars load as the default, as in
/// Python; a list or an object is a field error (Py bug B39).
#[rstest]
#[case::alias(r#""textual-dark""#, "textual-dark")]
#[case::canonical(r#""dark""#, "dark")]
#[case::unknown(r#""no-such""#, "no-such")]
#[case::case_sensitive(r#""Nord""#, "Nord")]
#[case::number("5", "dark")]
#[case::bool("true", "dark")]
#[case::null("null", "dark")]
fn theme_names_load_as_written(#[case] input: &str, #[case] loaded: &str) {
    let (_tmp, paths, c) = load_ok(&format!(r#"{{"appearance": {{"theme": {input}}}}}"#));
    assert_eq!(c.appearance.theme, loaded);
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(saved["appearance"]["theme"], loaded);
}

#[rstest]
#[case::list(r#"["nord"]"#)]
#[case::object(r#"{"name": "nord"}"#)]
fn a_list_or_object_theme_is_a_field_error(#[case] input: &str) {
    let (_tmp, _paths, result) = load_text(&format!(r#"{{"appearance": {{"theme": {input}}}}}"#));
    assert!(
        matches!(&result, Err(ConfigError::Invalid { field, .. }) if field == "appearance.theme"),
        "{result:?}"
    );
}

/// `appearance.color_depth` (decision `color-depth-detection`): each value loads and saves as written.
#[rstest]
#[case::auto("auto", ColorDepthSetting::Auto)]
#[case::truecolor("truecolor", ColorDepthSetting::TrueColor)]
#[case::eight_bit("256", ColorDepthSetting::EightBit)]
#[case::standard("16", ColorDepthSetting::Standard)]
fn color_depths_load_and_save_as_written(#[case] input: &str, #[case] loaded: ColorDepthSetting) {
    let (_tmp, paths, result) = load_text(&format!(r#"{{"appearance": {{"color_depth": "{input}"}}}}"#));
    let (c, notes) = Config::load_with_notes(&paths).unwrap();
    assert_eq!(result.unwrap(), c);
    assert_eq!(c.appearance.color_depth, loaded);
    assert_eq!(notes.invalid_color_depth, None);
    assert!(notes.warnings().is_empty());
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(saved["appearance"]["color_depth"], input);
}

/// A value that isn't one of the four loads as `auto`, with a warning, where every other bad enum stops the load:
/// a wrong color depth costs only colors. The next save writes `auto`.
#[rstest]
#[case::unknown(r#""bogus""#)]
#[case::wrong_case(r#""Auto""#)]
#[case::empty(r#""""#)]
#[case::number("256")]
#[case::null("null")]
#[case::list(r#"["16"]"#)]
fn a_bad_color_depth_loads_as_auto_with_a_warning(#[case] input: &str) {
    let text = format!(
        r#"{{"appearance": {{"theme": "nord", "color_depth": {input}}}, "issues": {{"owner_filter": "mine"}}}}"#
    );
    let (_tmp, paths, result) = load_text(&text);
    let c = result.unwrap();
    assert_eq!(c.appearance.color_depth, ColorDepthSetting::Auto);
    // The rest of the file loads as usual.
    assert_eq!(c.appearance.theme, "nord");
    assert_eq!(c.issues.owner_filter, IssueOwnerFilter::Mine);
    let (_, notes) = Config::load_with_notes(&paths).unwrap();
    let found: Value = serde_json::from_str(input).unwrap();
    assert_eq!(notes.invalid_color_depth.as_ref(), Some(&found));
    assert_eq!(
        notes.warnings(),
        [format!(
            r#"appearance.color_depth: found {found}, expected one of "auto", "truecolor", "256", "16". Using "auto"."#
        )]
    );
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(saved["appearance"]["color_depth"], "auto");
}

/// Only the color depth gets that leniency: another bad value in the same file still stops the load, and names
/// itself, not the color depth.
#[test]
fn a_bad_color_depth_doesnt_hide_another_bad_value() {
    let (_tmp, _paths, result) =
        load_text(r#"{"appearance": {"color_depth": "bogus"}, "issues": {"state_filter": "bogus"}}"#);
    assert!(
        matches!(&result, Err(ConfigError::Invalid { field, .. }) if field == "issues.state_filter"),
        "{result:?}"
    );
}

/// The Rust-only key round-trips in Rust, and the Python app loads the file and drops the key
/// (`python-rust-only-keys.json` is what it saves), which Rust then loads as `auto`.
#[test]
fn rust_only_keys_round_trip_and_python_drops_them() {
    let text = fixture("rust-only-keys.json");
    let (_tmp, paths, c) = load_ok(&text);
    assert_eq!(c.appearance.color_depth, ColorDepthSetting::EightBit);
    assert_eq!(c.appearance.theme, "nord");
    assert!(c.appearance.extra.is_empty());
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_contains(&saved, &serde_json::from_str(&text).unwrap(), "");

    let python = fixture("python-rust-only-keys.json");
    let python_json: Value = serde_json::from_str(&python).unwrap();
    assert_eq!(python_json["appearance"].get("color_depth"), None);
    assert_eq!(python_json["appearance"]["theme"], "nord");
    assert_eq!(python_json["appearance"]["show_issues"], false);
    assert_eq!(python_json.get("search"), None);
    let (_tmp, _paths, c) = load_ok(&python);
    assert_eq!(c.appearance.color_depth, ColorDepthSetting::Auto);
    assert!(!c.appearance.show_issues);
    assert_eq!(c.search, SearchSettings::default());
}

/// `search.saved_queries` (decision `search-issues-and-prs`) loads and saves in order, and unknown keys in the
/// section are written back.
#[test]
fn saved_search_queries_round_trip_with_unknown_keys() {
    let (_tmp, paths, c) = load_ok(&fixture("rust-only-keys.json"));
    assert_eq!(c.search.saved_queries, ["is:pr is:open", "label:bug"]);
    assert_eq!(c.search.extra.get("zz_unknown"), Some(&Value::from(1)));
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(
        saved["search"],
        serde_json::json!({"saved_queries": ["is:pr is:open", "label:bug"], "zz_unknown": 1})
    );
    let (_tmp, _paths, again) = load_ok(&saved.to_string());
    assert_eq!(again.search, c.search);
}

/// A config without a `search` section, as every Python-saved file is, loads with no saved queries, and a save
/// writes the default section.
#[test]
fn a_config_without_a_search_section_loads_with_no_saved_queries() {
    let (_tmp, paths, c) = load_ok(r#"{"appearance": {"theme": "nord"}}"#);
    assert_eq!(c.search, SearchSettings::default());
    assert!(c.search.saved_queries.is_empty());
    c.save(&paths).unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(saved["search"], serde_json::json!({"saved_queries": []}));
    let (_tmp, _paths, c) = load_ok(r#"{"search": {}}"#);
    assert!(c.search.saved_queries.is_empty());
}

#[test]
fn the_default_color_depth_is_auto() {
    let config = Config::default_in(&ConfigPaths::in_dir("lazy-github"));
    assert_eq!(config.appearance.color_depth, ColorDepthSetting::Auto);
    let json: Value = serde_json::from_str(&config.to_json_pretty()).unwrap();
    assert_eq!(json["appearance"]["color_depth"], "auto");
}

#[test]
fn the_default_theme_is_dark() {
    let config = Config::default_in(&ConfigPaths::in_dir("lazy-github"));
    assert_eq!(config.appearance.theme, "dark");
}

#[test]
fn full_python_config_loads_every_value_and_saves_identically() {
    let text = fixture("python-full.json");
    let (_tmp, paths, c) = load_ok(&text);

    assert_eq!(c.appearance.theme, "nord");
    assert!(c.appearance.show_command_log);
    assert!(!c.appearance.show_workflows);
    assert!(!c.appearance.show_issues);
    assert!(!c.appearance.show_pull_requests);
    assert!(c.appearance.extra.is_empty());

    assert!(c.notifications.enabled);
    assert!(!c.notifications.mark_notification_as_read_when_selected);

    let overrides: Vec<_> = c
        .bindings
        .overrides
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        overrides,
        [("app.quit", "ctrl+q"), ("common.table.select", "enter,space")]
    );

    assert_eq!(
        c.repositories.additional_repos_to_track,
        ["Textualize/textual", "org/repo.with-chars_1"]
    );
    assert_eq!(c.repositories.favorites, ["gizmo385/gh-lazy", "gizmo385/dotfiles"]);

    assert_eq!(c.pull_requests.state_filter, IssueStateFilter::Open);
    assert_eq!(c.pull_requests.owner_filter, IssueOwnerFilter::Mine);
    assert_eq!(c.pull_requests.preferred_merge_method, MergeMethod::Rebase);
    assert_eq!(c.pull_requests.additional_suggested_pr_reviewers, ["octocat", "hubot"]);
    assert!(c.pull_requests.preload_pull_request_for_current_commit);
    assert_eq!(
        c.pull_requests.pull_request_template.as_deref(),
        Some(Path::new("/home/lazy/templates/pr.md"))
    );

    assert_eq!(c.issues.state_filter, IssueStateFilter::Open);
    assert_eq!(c.issues.owner_filter, IssueOwnerFilter::Mine);

    assert_eq!(c.cache.auth_cache_duration, 7200);
    assert_eq!(c.cache.auth_last_checked, Some(dt("2026-10-24T18:30:05.123456")));
    assert_eq!(c.cache.cache_directory, Path::new("/var/cache/lazy-github"));
    assert_eq!(c.cache.default_ttl, 300);
    assert_eq!(c.cache.list_repos_ttl, 43200);
    assert_eq!(c.cache.list_issues_ttl, 1800);
    assert_eq!(c.cache.default_ttl(), std::time::Duration::from_mins(5));

    assert!(!c.core.first_start);
    assert_eq!(c.core.logfile_path, Path::new("/var/log/lazy-github/app.log"));
    assert_eq!(c.core.logfile_max_bytes, 1_000_000);
    assert_eq!(c.core.logfile_count, 3);

    assert_eq!(c.api.base_url, "https://github.example.com/api/v3");
    assert_eq!(c.api.extra.get("client_type"), Some(&Value::from("GITHUB_CLI")));
    assert!(c.extra.is_empty());

    c.save(&paths).unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths.file).unwrap(),
        with_rust_only_keys(&text)
    );
}

#[test]
fn partial_config_keeps_present_fields_and_defaults_the_rest() {
    let (_tmp, paths, c) = load_ok(&fixture("python-partial.json"));

    let mut expected = Config::default_in(&paths);
    expected.appearance.theme = "dracula".to_owned();
    expected.appearance.show_command_log = true;
    expected.repositories.favorites = vec!["gizmo385/gh-lazy".to_owned()];
    expected.pull_requests.preferred_merge_method = MergeMethod::Merge;
    expected.cache.auth_last_checked = Some(dt("2025-03-04T05:06:07"));
    assert_eq!(c, expected);
    assert_eq!(c.cache.cache_directory, paths.dir.join(".cache"));
    assert_eq!(c.core.logfile_path, paths.dir.join("lazy_github.log"));
}

/// Every key of `subset` is in `superset` with an equal value, recursing into objects.
fn assert_contains(superset: &Value, subset: &Value, at: &str) {
    match (superset, subset) {
        (Value::Object(sup), Value::Object(sub)) => {
            for (key, value) in sub {
                let found = sup
                    .get(key)
                    .unwrap_or_else(|| panic!("{at}.{key} missing after round trip"));
                assert_contains(found, value, &format!("{at}.{key}"));
            }
        }
        _ => assert_eq!(superset, subset, "{at} changed in round trip"),
    }
}

fn keys(value: &Value) -> Vec<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

#[test]
fn unknown_fields_survive_a_round_trip_after_known_keys() {
    let text = fixture("unknown-fields.json");
    let (_tmp, paths, c) = load_ok(&text);
    // An unknown theme is kept as written (app-owned-themes D5), where Python would rewrite it as its default.
    assert_eq!(c.appearance.theme, "not-a-real-theme");
    // Override strings are kept raw, Textual key names included.
    assert_eq!(c.bindings.overrides["app.help"], "question_mark");
    assert_eq!(c.bindings.overrides["common.search"], "slash");
    assert_eq!(c.bindings.overrides["app.dollar"], "dollar_sign");
    assert_eq!(c.bindings.overrides["common.table.select"], "q, escape");
    c.save(&paths).unwrap();

    let input: Value = serde_json::from_str(&text).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(output["appearance"]["theme"], "not-a-real-theme");
    assert_contains(&output, &input, "");
    assert_eq!(output["api"]["client_type"], "GITHUB_CLI");

    assert_eq!(
        keys(&output),
        [
            "appearance",
            "notifications",
            "bindings",
            "repositories",
            "pull_requests",
            "issues",
            "cache",
            "core",
            "api",
            "search",
            "future_section",
            "another_unknown"
        ]
    );
    assert_eq!(
        keys(&output["appearance"]),
        [
            "theme",
            "show_command_log",
            "show_workflows",
            "show_issues",
            "show_pull_requests",
            "color_depth",
            "zz_unknown",
            "aa_unknown"
        ]
    );
    assert_eq!(keys(&output["bindings"]), ["overrides", "bindings_extra"]);
    assert_eq!(
        keys(&output["bindings"]["overrides"]),
        keys(&input["bindings"]["overrides"])
    );
    assert_eq!(keys(&output["api"]), ["base_url", "client_type", "timeout"]);
}

#[test]
fn lax_values_load_and_save_canonically() {
    let (_tmp, paths, c) = load_ok(&fixture("lax-values.json"));
    assert!(c.appearance.show_command_log);
    assert!(!c.appearance.show_workflows);
    assert!(!c.appearance.show_issues);
    assert!(c.appearance.show_pull_requests);
    assert!(c.notifications.enabled);
    assert!(!c.notifications.mark_notification_as_read_when_selected);
    assert!(c.pull_requests.preload_pull_request_for_current_commit);
    assert_eq!(c.cache.auth_cache_duration, 3600);
    assert_eq!(c.cache.default_ttl, 120);
    assert_eq!(c.cache.list_repos_ttl, 7200);
    assert_eq!(c.cache.list_issues_ttl, 60);
    assert!(!c.core.first_start);
    assert_eq!(c.core.logfile_max_bytes, 1000);
    assert_eq!(c.core.logfile_count, 2);

    c.save(&paths).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(output["appearance"]["show_command_log"], Value::Bool(true));
    assert_eq!(output["notifications"]["enabled"], Value::Bool(true));
    assert_eq!(output["core"]["first_start"], Value::Bool(false));
    assert_eq!(output["cache"]["auth_cache_duration"], Value::from(3600));
    assert_eq!(output["cache"]["default_ttl"], Value::from(120));
    assert_eq!(output["core"]["logfile_count"], Value::from(2));
}

/// What loading one value does, on either side.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// It loads, and a save writes this value.
    Saved(Value),
    /// The file is rejected at this field.
    Error(String),
    /// Python raises this exception from inside validation, which the app doesn't catch (no Rust equivalent).
    Exception(String),
}

/// One case of `python-parsing.json`.
#[derive(Debug, serde::Deserialize)]
struct ParseCase {
    field: String,
    input: Value,
    #[serde(flatten)]
    python: PythonResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
enum PythonResult {
    Ok { saved: Value },
    Error { loc: String },
    Exception { exception: String },
}

impl From<PythonResult> for Outcome {
    fn from(result: PythonResult) -> Self {
        match result {
            PythonResult::Ok { saved } => Outcome::Saved(saved),
            PythonResult::Error { loc } => Outcome::Error(loc),
            PythonResult::Exception { exception } => Outcome::Exception(exception),
        }
    }
}

/// What the Rust app does instead, for a case listed in [`PARSE_DIFFS`].
#[derive(Debug, Clone, Copy)]
enum Expected {
    /// Loads, and a save writes this JSON.
    Saved(&'static str),
    /// Loads as this instant in local civil time, which a save writes naive (whole seconds only).
    LocalTime(&'static str),
    /// The file is rejected at this field.
    Rejected(&'static str),
}

impl Expected {
    fn outcome(self) -> Outcome {
        match self {
            Expected::Saved(json) => Outcome::Saved(serde_json::from_str(json).unwrap()),
            Expected::LocalTime(instant) => {
                let local = instant
                    .parse::<jiff::Timestamp>()
                    .unwrap()
                    .to_zoned(jiff::tz::TimeZone::system());
                Outcome::Saved(local.strftime("%Y-%m-%dT%H:%M:%S").to_string().into())
            }
            Expected::Rejected(field) => Outcome::Error(field.to_owned()),
        }
    }
}

/// The cases of `python-parsing.json` where the Rust app deliberately differs: field, input (JSON), what Rust does, and
/// why, naming the py-bug.
const PARSE_DIFFS: &[(&str, &str, Expected, &str)] = &[
    (
        "cache.auth_last_checked",
        r#""None""#,
        Expected::Saved("null"),
        "B2: Python's Settings modal writes an unset datetime as \"None\", which Python then can't load",
    ),
    (
        "cache.auth_last_checked",
        r#""2026-01-02T03:04:05+02:00""#,
        Expected::LocalTime("2026-01-02T03:04:05+02:00"),
        "B37: Python keeps the offset, and the aware datetime crashes the auth check at startup",
    ),
    (
        "cache.auth_last_checked",
        "0",
        Expected::LocalTime("1970-01-01T00:00:00Z"),
        "B37: Python loads a Unix timestamp as an aware UTC datetime, which crashes the auth check at startup",
    ),
    (
        "appearance.theme",
        r#"{"name": "nord"}"#,
        Expected::Rejected("appearance.theme"),
        "B39: Python's theme validator raises TypeError for an unhashable value, which crashes startup; Rust reports \
         the field instead",
    ),
    (
        "appearance.theme",
        r#""Nord""#,
        Expected::Saved(r#""Nord""#),
        "app-owned-themes D5: a name the app doesn't know is kept as written, not rewritten as the default",
    ),
    (
        "appearance.theme",
        r#""not-a-real-theme""#,
        Expected::Saved(r#""not-a-real-theme""#),
        "app-owned-themes D5: a name the app doesn't know is kept as written, not rewritten as the default",
    ),
    (
        "appearance.theme",
        r#""""#,
        Expected::Saved(r#""""#),
        "app-owned-themes D5: a name the app doesn't know is kept as written, not rewritten as the default",
    ),
    (
        "appearance.theme",
        "5",
        Expected::Saved(r#""dark""#),
        "app-owned-themes D2: the default theme is called dark, not textual-dark",
    ),
    (
        "appearance.theme",
        "null",
        Expected::Saved(r#""dark""#),
        "app-owned-themes D2: the default theme is called dark, not textual-dark",
    ),
];

/// Loads `{section: {key: input}}` and reports what a save writes at that field, or the field it's rejected at.
fn rust_outcome(field: &str, input: &Value) -> Outcome {
    let (section, key) = field.split_once('.').unwrap();
    let text = serde_json::json!({ section: { key: input } }).to_string();
    match load_text(&text).2 {
        Ok(config) => {
            let saved: Value = serde_json::from_str(&config.to_json_pretty()).unwrap();
            Outcome::Saved(saved[section][key].clone())
        }
        Err(ConfigError::Invalid { field, .. }) => Outcome::Error(field),
        // Not a field error, so it matches nothing Python reports; the message shows in the failure.
        Err(other) => Outcome::Error(format!("not a field error: {other}")),
    }
}

/// Every case the Python app's validators were run on (`python-parsing.json`, from `generate.py`): Rust loads the
/// value to what Python saves, or rejects the file at the field Python names, except for the cases in
/// [`PARSE_DIFFS`]. A listed difference that no longer differs fails too.
#[test]
fn python_parsing_cases_match() {
    let cases: Vec<ParseCase> = serde_json::from_str(&fixture("python-parsing.json")).unwrap();
    let diffs: Vec<(&str, Value, Expected, &str)> = PARSE_DIFFS
        .iter()
        .map(|&(field, input, expected, why)| (field, serde_json::from_str(input).unwrap(), expected, why))
        .collect();
    let mut used = vec![false; diffs.len()];
    let mut failures = Vec::new();

    for case in cases {
        let python = Outcome::from(case.python);
        let rust = rust_outcome(&case.field, &case.input);
        let diff = diffs
            .iter()
            .position(|(field, input, ..)| *field == case.field && *input == case.input);
        let at = format!("{} = {}", case.field, case.input);
        match diff {
            Some(i) => {
                used[i] = true;
                let (.., expected, why) = diffs[i];
                if rust == python {
                    failures.push(format!(
                        "{at}: listed in PARSE_DIFFS ({why}) but now matches Python: {rust:?}"
                    ));
                } else if rust != expected.outcome() {
                    failures.push(format!("{at}: expected {:?} ({why}), got {rust:?}", expected.outcome()));
                }
            }
            None if rust != python => failures.push(format!("{at}: Python {python:?}, Rust {rust:?}")),
            None => {}
        }
    }
    for (i, (field, input, ..)) in diffs.iter().enumerate() {
        if !used[i] {
            failures.push(format!(
                "PARSE_DIFFS lists {field} = {input}, which python-parsing.json has no case for"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Python's settings modal (bug B2 in #71) saves numbers as strings and an unset `auth_last_checked` as "None".
#[test]
fn python_settings_modal_output_loads() {
    let text = r#"{
        "cache": {
            "auth_cache_duration": "86400",
            "auth_last_checked": "None",
            "default_ttl": "600",
            "list_repos_ttl": "86400",
            "list_issues_ttl": "3600"
        },
        "core": {"logfile_max_bytes": "5000000", "logfile_count": "5"}
    }"#;
    let (_tmp, paths, c) = load_ok(text);
    // The old TTL trio, as strings, loads as the new defaults (cache-ttl-defaults-and-refresh D1 a).
    assert_eq!(c, Config::default_in(&paths));
    c.save(&paths).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(output["cache"]["auth_last_checked"], Value::Null);
    assert_eq!(output["cache"]["default_ttl"], Value::from(60));
}

#[test]
fn missing_file_gives_defaults_and_creates_nothing() {
    let tmp = TempDir::new();
    let paths = tmp.paths();
    let config = Config::load(&paths).unwrap();
    assert_eq!(config, Config::default_in(&paths));
    assert!(!paths.dir.exists());
}

#[test]
fn empty_object_gives_defaults() {
    let (_tmp, paths, c) = load_ok("{}");
    assert_eq!(c, Config::default_in(&paths));
}

#[test]
fn bad_json_reports_line_and_column() {
    let (_tmp, paths, result) = load_text("{\n    \"appearance\": {\n        \"theme\": \"nord\",\n    }\n}");
    match result {
        Err(ConfigError::Syntax { path, line, column, .. }) => {
            assert_eq!(path, paths.file);
            assert_eq!((line, column), (4, 5));
        }
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

#[test]
fn trailing_content_is_a_syntax_error() {
    let (_tmp, _paths, result) = load_text("{} {}");
    assert!(
        matches!(result, Err(ConfigError::Syntax { line: 1, column: 4, .. })),
        "{result:?}"
    );
}

#[test]
fn bad_enum_value_reports_the_field_path() {
    let (_tmp, paths, result) = load_text(r#"{"issues":{"state_filter":"bogus"}}"#);
    let err = result.unwrap_err();
    match &err {
        ConfigError::Invalid { field, message, .. } => {
            assert_eq!(field, "issues.state_filter");
            assert!(message.contains("bogus"), "{message}");
            assert!(message.contains("all") && message.contains("open") && message.contains("closed"));
        }
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
    let shown = err.to_string();
    assert!(shown.contains(&paths.file.display().to_string()), "{shown}");
    assert!(shown.contains("lazy-github check-config --fix"), "{shown}");
}

/// The field cases are in `python-parsing.json`; this one is a whole section.
#[test]
fn a_null_section_reports_the_section() {
    let (_tmp, _paths, result) = load_text(r#"{"issues":null}"#);
    match result {
        Err(ConfigError::Invalid { field, .. }) => assert_eq!(field, "issues"),
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
}

/// Py bug B39: Python crashes on these, so Rust reports the field (the broken-config error) instead.
#[rstest]
#[case::list(r#"["nord"]"#)]
#[case::object(r#"{"name": "nord"}"#)]
fn a_list_or_object_theme_is_rejected(#[case] theme: &str) {
    let (_tmp, _paths, result) = load_text(&format!(r#"{{"appearance":{{"theme":{theme}}}}}"#));
    match result {
        Err(ConfigError::Invalid { field, .. }) => assert_eq!(field, "appearance.theme"),
        other => panic!("expected an invalid-value error, got {other:?}"),
    }
}

#[rstest]
#[case::seconds("2026-01-02T03:04:05")]
#[case::micros("2026-01-02T03:04:05.000007")]
#[case::millis("2026-01-02T03:04:05.120000")]
fn auth_last_checked_round_trips(#[case] raw: &str) {
    let (_tmp, paths, c) = load_ok(&format!(r#"{{"cache":{{"auth_last_checked":"{raw}"}}}}"#));
    assert_eq!(c.cache.auth_last_checked, Some(dt(raw)));
    c.save(&paths).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(output["cache"]["auth_last_checked"], raw);
}

#[cfg(unix)]
#[test]
fn save_writes_through_a_symlinked_config_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = TempDir::new();
    let paths = tmp.paths();
    std::fs::create_dir_all(&paths.dir).unwrap();
    let dotfiles = tmp.0.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let target = dotfiles.join("lazy-github.json");
    std::fs::write(&target, "{}").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, &paths.file).unwrap();

    let mut config = Config::load(&paths).unwrap();
    config.repositories.favorites.push("gizmo385/gh-lazy".to_owned());
    config.save(&paths).unwrap();

    assert!(std::fs::symlink_metadata(&paths.file).unwrap().is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), config.to_json_pretty());
    let mode = std::fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<_> = std::fs::read_dir(&dotfiles)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["lazy-github.json"]);
}

#[test]
fn save_creates_the_dir_atomically_without_trailing_newline() {
    let tmp = TempDir::new();
    let paths = ConfigPaths::in_dir(tmp.0.join("a").join("b"));
    let mut config = Config::default_in(&paths);
    config.repositories.favorites.push("gizmo385/gh-lazy".to_owned());
    config.save(&paths).unwrap();

    let written = std::fs::read_to_string(&paths.file).unwrap();
    assert_eq!(written, config.to_json_pretty());
    assert!(!written.ends_with('\n'));
    let names: Vec<_> = std::fs::read_dir(&paths.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["config.json"]);

    // Saving over an existing file replaces it.
    config.repositories.favorites.clear();
    config.save(&paths).unwrap();
    assert_eq!(Config::load(&paths).unwrap(), config);
}

#[test]
fn non_ascii_is_written_unescaped() {
    let tmp = TempDir::new();
    let paths = tmp.paths();
    let mut config = Config::default_in(&paths);
    config.appearance.theme = "thème-★".to_owned();
    assert!(config.to_json_pretty().contains("\"theme\": \"thème-★\""));
}

// Ports of tests/test_config.py.

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Saves `config` and loads it back.
fn round_trip(config: &Config) -> Config {
    let tmp = TempDir::new();
    let paths = tmp.paths();
    config.save(&paths).unwrap();
    Config::load(&paths).unwrap()
}

#[rstest]
#[case::two(&["gizmo385/dotfiles", "gizmo385/discord.clj"])]
#[case::three(&["gizmo385/dotfiles", "gizmo385/discord.clj", "Textualize/textual"])]
#[case::single_item(&["gizmo385/dotfiles"])]
#[case::empty(&[])]
#[case::order_preserved(&["item3", "item1", "item2", "item4"])]
#[case::duplicates_preserved(&["user1", "user2", "user1"])]
#[case::special_characters(&["user/repo-with-dashes", "user/repo_with_underscores", "user/repo.with.dots", "user/repo123"])]
fn list_fields_round_trip(#[case] items: &[&str]) {
    let mut config = Config::default_in(&ConfigPaths::in_dir("unused"));
    config.repositories.favorites = strings(items);
    config.repositories.additional_repos_to_track = strings(items);
    config.pull_requests.additional_suggested_pr_reviewers = strings(items);

    let loaded = round_trip(&config);
    assert_eq!(loaded.repositories.favorites, items);
    assert_eq!(loaded.repositories.additional_repos_to_track, items);
    assert_eq!(loaded.pull_requests.additional_suggested_pr_reviewers, items);

    let json: Value = serde_json::from_str(&config.to_json_pretty()).unwrap();
    let expected = Value::from(strings(items));
    assert_eq!(json["repositories"]["favorites"], expected);
    assert_eq!(json["repositories"]["additional_repos_to_track"], expected);
    assert_eq!(json["pull_requests"]["additional_suggested_pr_reviewers"], expected);
}

#[test]
fn favorites_are_written_as_a_json_array() {
    let mut config = Config::default_in(&ConfigPaths::in_dir("unused"));
    config.repositories.favorites = strings(&["user/repo1", "user/repo2"]);
    let text = config.to_json_pretty();
    assert!(text.contains("\"favorites\": ["), "{text}");
    assert!(text.contains("\"user/repo1\""));
    assert!(text.contains("\"user/repo2\""));
    assert!(!text.contains("\"favorites\": \"user/repo1, user/repo2\""));
}

#[test]
fn list_items_have_no_nested_quotes() {
    let mut config = Config::default_in(&ConfigPaths::in_dir("unused"));
    config.pull_requests.additional_suggested_pr_reviewers = strings(&["person1", "person2", "person3"]);
    let json: Value = serde_json::from_str(&config.to_json_pretty()).unwrap();
    for item in json["pull_requests"]["additional_suggested_pr_reviewers"]
        .as_array()
        .unwrap()
    {
        let item = item.as_str().unwrap();
        assert!(!item.contains(['"', '[', ']', '\'']), "{item}");
    }
}

// Some behavior depends on environment variables (`HOME`, `TZ`) that can't be changed safely inside this process.
// A parent test reruns this test binary for one child test with the variables set, and the child checks the result.
// A child test does nothing unless `CHILD_ENV` names it.
const CHILD_ENV: &str = "LAZY_GITHUB_TEST_CHILD";

#[cfg(unix)]
fn run_child(name: &str, envs: &[(&str, &str)]) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture"])
        .env(CHILD_ENV, name)
        .envs(envs.iter().copied())
        .status()
        .unwrap();
    assert!(status.success(), "child test {name} failed");
}

fn is_child(name: &str) -> bool {
    std::env::var(CHILD_ENV).is_ok_and(|v| v == name)
}

#[cfg(unix)]
#[test]
fn discover_uses_home_and_ignores_xdg_config_home() {
    run_child(
        "discover_child",
        &[("HOME", "/home/lazy"), ("XDG_CONFIG_HOME", "/somewhere/else")],
    );
}

#[test]
fn discover_child() {
    if !is_child("discover_child") {
        return;
    }
    let paths = ConfigPaths::discover().unwrap();
    assert_eq!(paths.dir, Path::new("/home/lazy/.config/lazy-github"));
    assert_eq!(paths.file, Path::new("/home/lazy/.config/lazy-github/config.json"));
}

#[cfg(unix)]
#[test]
fn auth_last_checked_converts_offset_timestamps_to_local_time() {
    run_child("offset_timestamp_child", &[("TZ", "America/New_York")]);
}

#[test]
fn offset_timestamp_child() {
    if !is_child("offset_timestamp_child") {
        return;
    }
    for (raw, local) in [
        ("2026-01-02T03:04:05.5+02:00", "2026-01-01T20:04:05.5"),
        ("2026-01-02T03:04:05Z", "2026-01-01T22:04:05"),
        ("2026-07-02T03:04:05-07:00", "2026-07-02T06:04:05"),
    ] {
        let (_tmp, _paths, c) = load_ok(&format!(r#"{{"cache":{{"auth_last_checked":"{raw}"}}}}"#));
        assert_eq!(c.cache.auth_last_checked, Some(dt(local)), "{raw}");
    }
}
