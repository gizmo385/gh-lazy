use super::{Fix, Problem, Repaired, check, repair};
use crate::config::{Config, ConfigError, ConfigPaths};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn paths() -> ConfigPaths {
    ConfigPaths::in_dir(PathBuf::from("home").join("lazy-github"))
}

/// The report's problems, each as its line.
fn lines(text: &str) -> Vec<String> {
    check(text, &paths())
        .unwrap()
        .problems
        .iter()
        .map(Problem::to_string)
        .collect()
}

#[test]
fn a_config_that_loads_has_no_problems() {
    for text in [
        "{}",
        r#"{"issues":{"state_filter":"open"},"appearance":{"show_issues":"yes"}}"#,
    ] {
        let report = check(text, &paths()).unwrap();
        assert_eq!(report.problems, [], "{text}");
        assert_eq!(report.unknown_keys, Vec::<String>::new(), "{text}");
    }
}

#[test]
fn values_the_loader_accepts_are_not_problems() {
    // An unknown theme loads as the default, and a negative TTL loads as written.
    let report = check(
        r#"{"appearance":{"theme":"nope"},"cache":{"default_ttl":-5}}"#,
        &paths(),
    )
    .unwrap();
    assert_eq!(report.problems, []);
}

#[test]
fn a_bad_enum_value_is_one_problem() {
    let report = check(r#"{"issues":{"state_filter":"bogus"}}"#, &paths()).unwrap();
    assert_eq!(
        report.problems,
        [Problem {
            field: "issues.state_filter".to_owned(),
            found: json!("bogus"),
            message: "unknown variant `bogus`, expected one of `all`, `open`, `closed`".to_owned(),
            fix: Fix::Reset(json!("all")),
        }]
    );
    assert_eq!(
        report.problems[0].to_string(),
        r#"issues.state_filter: found "bogus", expected one of "all", "open", "closed""#
    );
}

#[test]
fn bad_values_in_two_sections_are_listed_in_file_order() {
    let text = r#"{
        "issues": {"state_filter": "bogus", "owner_filter": "mine"},
        "appearance": {"theme": ["nord"], "show_issues": "maybe"},
        "pull_requests": {"preferred_merge_method": "rebase"}
    }"#;
    assert_eq!(
        lines(text),
        [
            r#"issues.state_filter: found "bogus", expected one of "all", "open", "closed""#,
            r#"appearance.theme: found ["nord"], expected a theme name"#,
            r#"appearance.show_issues: found "maybe", expected a boolean"#,
        ]
    );
}

/// Py bug B39: Python crashes with a `TypeError` on these.
#[test]
fn a_list_or_object_theme_is_a_problem() {
    let report = check(r#"{"appearance":{"theme":{"name":"nord"}}}"#, &paths()).unwrap();
    assert_eq!(report.problems.len(), 1);
    assert_eq!(report.problems[0].fix, Fix::Reset(json!("textual-dark")));
    assert_eq!(
        report.problems[0].to_string(),
        r#"appearance.theme: found {"name":"nord"}, expected a theme name"#
    );
}

/// Py bugs B2 and B37: the forms of `auth_last_checked` that don't load.
#[test]
fn an_unparseable_auth_last_checked_is_a_problem() {
    for found in [json!("yesterday"), json!(true), json!(["2026-01-01"]), json!("1e3")] {
        let text = json!({"cache": {"auth_last_checked": found}}).to_string();
        let report = check(&text, &paths()).unwrap();
        assert_eq!(report.problems.len(), 1, "{text}");
        let problem = &report.problems[0];
        assert_eq!(problem.field, "cache.auth_last_checked");
        assert_eq!(problem.found, found);
        assert_eq!(problem.fix, Fix::Reset(Value::Null));
        assert_eq!(
            problem.expected().as_deref(),
            Some("a datetime like 2024-01-31T12:34:56[.123456]")
        );
    }
}

#[test]
fn a_section_that_is_not_an_object_is_reset_whole() {
    let report = check(r#"{"issues":5,"notifications":null}"#, &paths()).unwrap();
    assert_eq!(
        report.problems.iter().map(Problem::to_string).collect::<Vec<_>>(),
        [
            "issues: found 5, expected an object",
            "notifications: found null, expected an object"
        ]
    );
    assert_eq!(
        report.problems[0].fix,
        Fix::Reset(json!({"state_filter": "all", "owner_filter": "all"}))
    );
}

#[test]
fn a_file_that_is_not_an_object_is_reset_whole() {
    let report = check("[1]", &paths()).unwrap();
    assert_eq!(report.problems.len(), 1);
    assert_eq!(report.problems[0].field, ".");
    assert_eq!(report.problems[0].found, json!([1]));
}

#[test]
fn defaults_that_depend_on_the_config_folder_are_filled_in() {
    let report = check(r#"{"cache":{"cache_directory":5}}"#, &paths()).unwrap();
    let default = paths().dir.join(".cache");
    assert_eq!(report.problems[0].fix, Fix::Reset(json!(default.to_string_lossy())));
}

#[test]
fn a_bad_list_entry_resets_the_list() {
    let report = check(r#"{"repositories":{"favorites":["a/b",5]}}"#, &paths()).unwrap();
    assert_eq!(
        report.problems.iter().map(Problem::to_string).collect::<Vec<_>>(),
        [r#"repositories.favorites: found ["a/b",5], expected a string"#]
    );
    assert_eq!(report.problems[0].fix, Fix::Reset(json!([])));
}

#[test]
fn a_bad_keybinding_override_is_removed() {
    let text = r#"{"bindings":{"overrides":{"app.quit":"ctrl+q","app.help":5,"app.x":null}}}"#;
    let report = check(text, &paths()).unwrap();
    assert_eq!(
        report.problems.iter().map(Problem::to_string).collect::<Vec<_>>(),
        [
            "bindings.overrides.app.help: found 5, expected a string",
            "bindings.overrides.app.x: found null, expected a string"
        ]
    );
    assert!(report.problems.iter().all(|problem| problem.fix == Fix::Remove));
}

#[test]
fn unknown_keys_are_notes_at_the_top_level_and_in_sections() {
    let text = r#"{"mystery":1,"issues":{"later":2,"state_filter":"bogus"},"api":{"client_type":"GITHUB_CLI"},"core":{"x":{}}}"#;
    let report = check(text, &paths()).unwrap();
    assert_eq!(report.problems.len(), 1);
    // `api.client_type` is Python's, kept without a note.
    assert_eq!(report.unknown_keys, ["mystery", "issues.later", "core.x"]);
}

#[test]
fn invalid_json_is_the_syntax_error() {
    for text in [r#"{"issues": "#, "{} {}", ""] {
        let result = check(text, &paths());
        assert!(
            matches!(result, Err(ConfigError::Syntax { .. })),
            "{text:?}: {result:?}"
        );
    }
}

#[test]
fn a_message_without_an_expectation_is_shown_whole() {
    let problem = Problem {
        field: "issues".to_owned(),
        found: json!(1),
        message: "something odd".to_owned(),
        fix: Fix::Remove,
    };
    assert_eq!(problem.expected(), None);
    assert_eq!(problem.to_string(), "issues: found 1: something odd");
}

/// A file with two problems, unknown keys and values only the lax loader accepts.
const BROKEN: &str = r#"{
  "zzz": {"kept": [1, 2]},
  "appearance": {"theme": "nord", "show_issues": "yes", "later": "x"},
  "issues": {"state_filter": "bogus", "owner_filter": "everyone"},
  "cache": {"auth_last_checked": "yesterday", "default_ttl": "1_000"},
  "bindings": {"overrides": {"app.quit": "ctrl+q", "app.help": 5}},
  "api": {}
}"#;

fn fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn repair_resets_only_the_given_fields() {
    let repaired = repair(
        BROKEN,
        &paths(),
        &fields(&["issues.state_filter", "bindings.overrides.app.help"]),
    )
    .unwrap();
    let Repaired::Partial(value) = repaired else {
        panic!("one problem is kept, so the file can't load: {repaired:?}");
    };
    let mut expected: Value = serde_json::from_str(BROKEN).unwrap();
    expected["issues"]["state_filter"] = json!("all");
    expected["bindings"]["overrides"]
        .as_object_mut()
        .unwrap()
        .shift_remove("app.help");
    assert_eq!(value, expected);
    // The file's order is kept.
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["zzz", "appearance", "issues", "cache", "bindings", "api"]);
}

#[test]
fn repair_of_nothing_changes_nothing() {
    let Repaired::Partial(value) = repair(BROKEN, &paths(), &[]).unwrap() else {
        panic!("nothing was repaired");
    };
    assert_eq!(value, serde_json::from_str::<Value>(BROKEN).unwrap());
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lazy-github-doctor-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn repair_of_every_field_saves_as_config_save_writes_the_fixed_file() {
    let tmp = TempDir::new();
    let paths = ConfigPaths::in_dir(tmp.0.join("lazy-github"));
    let report = check(BROKEN, &paths).unwrap();
    let all: Vec<String> = report.problems.iter().map(|problem| problem.field.clone()).collect();
    assert_eq!(
        all,
        [
            "issues.state_filter",
            "issues.owner_filter",
            "cache.auth_last_checked",
            "bindings.overrides.app.help"
        ]
    );
    let repaired = repair(BROKEN, &paths, &all).unwrap();
    assert!(matches!(repaired, Repaired::Config(_)), "{repaired:?}");
    repaired.save(&paths).unwrap();
    let saved = std::fs::read_to_string(&paths.file).unwrap();

    // The same file fixed by hand, loaded and saved.
    let fixed = BROKEN
        .replace(r#""bogus""#, r#""all""#)
        .replace(r#""everyone""#, r#""all""#)
        .replace(r#""yesterday""#, "null")
        .replace(r#", "app.help": 5"#, "");
    std::fs::write(&paths.file, &fixed).unwrap();
    let config = Config::load(&paths).unwrap();
    config.save(&paths).unwrap();
    assert_eq!(saved, std::fs::read_to_string(&paths.file).unwrap());
    // Unknown keys and lax values survive, as the loader saves them.
    assert!(saved.contains(r#""later": "x""#), "{saved}");
    assert!(saved.contains(r#""default_ttl": 1000"#), "{saved}");
    assert!(saved.contains(r#""kept": ["#), "{saved}");
}

#[test]
fn a_partial_repair_saves_in_the_config_format() {
    let tmp = TempDir::new();
    let paths = ConfigPaths::in_dir(tmp.0.join("lazy-github"));
    repair(
        r#"{"issues":{"state_filter":"bogus","owner_filter":"everyone"}}"#,
        &paths,
        &fields(&["issues.state_filter"]),
    )
    .unwrap()
    .save(&paths)
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths.file).unwrap(),
        "{\n    \"issues\": {\n        \"state_filter\": \"all\",\n        \"owner_filter\": \"everyone\"\n    }\n}"
    );
}
