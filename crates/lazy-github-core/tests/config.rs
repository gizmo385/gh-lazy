//! Config load/save compatibility with the Python app. Fixtures in `tests/fixtures/config/` come from
//! `generate.py` there.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use jiff::civil::DateTime;
use lazy_github_core::config::{Config, ConfigError, ConfigPaths, IssueOwnerFilter, IssueStateFilter, MergeMethod};
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
    assert_eq!(config.to_json_pretty(), fixture("python-default.json"));
}

/// Regenerates `python-default.json` and `python-full.json` with the Python app and checks the committed copies match
/// (the golden manifest is `parity/goldens/config.toml`). The generator also checks the hand-written fixtures load in
/// Python, and fails if one doesn't.
// Unix only, like `default_matches_python_byte_for_byte`: the goldens hold `/home/lazy/...` paths.
#[cfg(unix)]
#[test]
#[ignore = "needs uv and the Python app"]
fn config_goldens_are_fresh() {
    assert_goldens_fresh(
        "crates/lazy-github-core/tests/fixtures/config/generate.py",
        &[
            "crates/lazy-github-core/tests/fixtures/config/python-default.json",
            "crates/lazy-github-core/tests/fixtures/config/python-full.json",
        ],
        Path::new("/home/lazy"),
        "HOME=/home/lazy uv run python crates/lazy-github-core/tests/fixtures/config/generate.py",
    );
}

/// Runs `generator` (relative to the repository root) with `--out <tempdir>` and `HOME` set to `home`, and fails with
/// a diff unless each of `goldens` is byte for byte the file it wrote. `lazy-github-tui/tests/registry.rs` has the same
/// helper: two short copies are simpler than a shared test crate.
#[cfg(unix)]
fn assert_goldens_fresh(generator: &str, goldens: &[&str], home: &Path, regenerate: &str) {
    use std::process::Command;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = TempDir::new();
    // `uv run` finds (and syncs) the project's interpreter under the real HOME, so uv's cache and Python installs are
    // the usual ones; only the generator sees `home`.
    let found = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(
        found.status.success(),
        "uv run failed:\n{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let python = String::from_utf8(found.stdout).unwrap();
    let run = Command::new(python.trim())
        .arg(generator)
        .arg("--out")
        .arg(&out.0)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{generator} failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for golden in goldens {
        let committed = std::fs::read(root.join(golden)).unwrap();
        let fresh = std::fs::read(out.0.join(Path::new(golden).file_name().unwrap())).unwrap();
        if committed != fresh {
            let (committed, fresh) = (String::from_utf8_lossy(&committed), String::from_utf8_lossy(&fresh));
            let diff = similar::TextDiff::from_lines(&*committed, &*fresh)
                .unified_diff()
                .header(golden, "regenerated")
                .to_string();
            let diff: Vec<&str> = diff.lines().take(40).collect();
            panic!(
                "{golden} is stale; the Python app now writes:\n{}\n\n\
                 Regenerate it from the repository root with:\n    {regenerate}",
                diff.join("\n")
            );
        }
    }
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
    assert_eq!(std::fs::read_to_string(&paths.file).unwrap(), text);
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
    // The theme is kept as written, even though Textual doesn't know it.
    assert_eq!(c.appearance.theme, "not-a-real-theme");
    // Override strings are kept raw, Textual key names included.
    assert_eq!(c.bindings.overrides["app.help"], "question_mark");
    assert_eq!(c.bindings.overrides["common.search"], "slash");
    assert_eq!(c.bindings.overrides["app.dollar"], "dollar_sign");
    assert_eq!(c.bindings.overrides["common.table.select"], "q, escape");
    c.save(&paths).unwrap();

    let input: Value = serde_json::from_str(&text).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
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

#[rstest]
#[case::true_lower("\"true\"", true)]
#[case::false_mixed("\"FaLsE\"", false)]
#[case::yes("\"YES\"", true)]
#[case::no("\"no\"", false)]
#[case::on("\"On\"", true)]
#[case::off("\"off\"", false)]
#[case::t("\"T\"", true)]
#[case::f("\"f\"", false)]
#[case::y("\"y\"", true)]
#[case::n("\"N\"", false)]
#[case::one_str("\"1\"", true)]
#[case::zero_str("\"0\"", false)]
#[case::one("1", true)]
#[case::zero("0", false)]
#[case::json_true("true", true)]
fn lax_bool_forms(#[case] raw: &str, #[case] expected: bool) {
    let (_tmp, _paths, c) = load_ok(&format!(r#"{{"notifications": {{"enabled": {raw}}}}}"#));
    assert_eq!(c.notifications.enabled, expected);
}

#[rstest]
#[case::bool_two("2")]
#[case::bool_word("\"maybe\"")]
#[case::bool_null("null")]
fn bad_bools_are_errors(#[case] raw: &str) {
    let (_tmp, _paths, result) = load_text(&format!(r#"{{"notifications": {{"enabled": {raw}}}}}"#));
    assert!(matches!(result, Err(ConfigError::Invalid { ref field, .. }) if field == "notifications.enabled"));
}

#[rstest]
#[case::int("600", 600)]
#[case::whole_float("600.0", 600)]
#[case::string("\"600\"", 600)]
#[case::string_float("\"600.0\"", 600)]
#[case::underscores("\"1_000\"", 1000)]
fn lax_int_forms(#[case] raw: &str, #[case] expected: u64) {
    let (_tmp, _paths, c) = load_ok(&format!(r#"{{"cache": {{"default_ttl": {raw}}}}}"#));
    assert_eq!(c.cache.default_ttl, expected);
}

#[rstest]
#[case::fraction("600.5")]
#[case::word("\"ten\"")]
#[case::negative("-1")]
#[case::null("null")]
#[case::exponent_string("\"1e3\"")]
#[case::fraction_string("\"600.5\"")]
fn bad_ints_are_errors(#[case] raw: &str) {
    let (_tmp, _paths, result) = load_text(&format!(r#"{{"cache": {{"default_ttl": {raw}}}}}"#));
    assert!(matches!(result, Err(ConfigError::Invalid { ref field, .. }) if field == "cache.default_ttl"));
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
    assert_eq!(c, Config::default_in(&paths));
    c.save(&paths).unwrap();
    let output: Value = serde_json::from_str(&std::fs::read_to_string(&paths.file).unwrap()).unwrap();
    assert_eq!(output["cache"]["auth_last_checked"], Value::Null);
    assert_eq!(output["cache"]["default_ttl"], Value::from(600));
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
    assert!(shown.contains("lazy-github clear-config"), "{shown}");
}

#[rstest]
#[case::null_list(r#"{"repositories":{"favorites":null}}"#, "repositories.favorites")]
#[case::null_section(r#"{"issues":null}"#, "issues")]
#[case::null_path(r#"{"cache":{"cache_directory":null}}"#, "cache.cache_directory")]
#[case::non_string_override(r#"{"bindings":{"overrides":{"app.quit":1}}}"#, "bindings.overrides.app.quit")]
#[case::string_list(r#"{"repositories":{"favorites":"a/b, c/d"}}"#, "repositories.favorites")]
#[case::bad_datetime(r#"{"cache":{"auth_last_checked":"yesterday"}}"#, "cache.auth_last_checked")]
fn invalid_values_report_their_field(#[case] text: &str, #[case] expected: &str) {
    let (_tmp, _paths, result) = load_text(text);
    match result {
        Err(ConfigError::Invalid { field, .. }) => assert_eq!(field, expected),
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

#[rstest]
#[case::null("null")]
#[case::number("5")]
#[case::object(r#"{"name": "nord"}"#)]
fn non_string_theme_loads_as_textual_dark(#[case] raw: &str) {
    let (_tmp, _paths, c) = load_ok(&format!(r#"{{"appearance": {{"theme": {raw}}}}}"#));
    assert_eq!(c.appearance.theme, "textual-dark");
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
