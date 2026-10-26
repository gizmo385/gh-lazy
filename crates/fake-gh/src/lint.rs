//! `fake-gh lint`: checks fixture sets for load errors, duplicate matchers and non-canonical names.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::fixture::{Fixture, dir_for, file_name_for, normalize_path, normalize_query};
use crate::store::{LoadedFixture, find_duplicates, scan_set};

/// One problem found by [`lint_sets`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem(pub String);

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The outcome of linting some sets.
#[derive(Debug, Default)]
pub struct Report {
    pub fixtures: usize,
    pub problems: Vec<Problem>,
}

/// Lints each set on its own (duplicates are only an error within a set).
pub fn lint_sets(sets: &[PathBuf]) -> Report {
    let mut report = Report::default();
    for (index, dir) in sets.iter().enumerate() {
        let (loaded, errors) = scan_set(dir, index);
        report.fixtures += loaded.len();
        report.problems.extend(errors.iter().map(|e| Problem(e.to_string())));
        report
            .problems
            .extend(find_duplicates(&loaded).iter().map(|e| Problem(e.to_string())));
        for f in &loaded {
            report.problems.extend(check_canonical(f));
        }
    }
    report
}

fn check_canonical(f: &LoadedFixture) -> Vec<Problem> {
    let mut problems = Vec::new();
    let problem = |msg: String| Problem(format!("{}: {msg}", f.display));
    // The store normalizes method, path and query on load; read the file as written to check them.
    if let Some(raw) = std::fs::read_to_string(&f.file)
        .ok()
        .and_then(|t| serde_json::from_str::<Fixture>(&t).ok())
    {
        if raw.request.method != raw.request.method.to_ascii_uppercase() {
            problems.push(problem(format!("method {:?} should be uppercase", raw.request.method)));
        }
        let path = normalize_path(&raw.request.path);
        if raw.request.path != path {
            problems.push(problem(format!(
                "path {:?} should be written as {path:?}",
                raw.request.path
            )));
        }
        let mut query = raw.request.query.clone();
        normalize_query(&raw.request.method, &mut query);
        if query != raw.request.query {
            problems.push(problem(
                r#"query has per_page but no page; write "page": "1""#.to_owned(),
            ));
        }
    }

    if let Some(name) = &f.fixture.response.body_file {
        let mut components = Path::new(name).components();
        let is_sibling =
            matches!(components.next(), Some(std::path::Component::Normal(_))) && components.next().is_none();
        if !is_sibling {
            problems.push(problem(format!(
                "body_file {name:?} must be a file name in the fixture's directory"
            )));
        }
    }

    let key = f.fixture.request.key();
    let want_dir = f.set_dir.join(dir_for(&key));
    let actual_dir = f.file.parent().unwrap_or(Path::new(""));
    let name = f
        .file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let want_name = file_name_for(&key);
    if actual_dir != want_dir || !name_matches(&name, &want_name) {
        let rel = dir_for(&key).join(&want_name);
        let rel: Vec<_> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        problems.push(problem(format!("non-canonical file name; expected {}", rel.join("/"))));
    }
    problems
}

/// `GET.json` matches `GET.json` and the numbered forms `GET.<n>.json`.
fn name_matches(name: &str, want: &str) -> bool {
    if name == want {
        return true;
    }
    let stem = want.strip_suffix(".json").unwrap_or(want);
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn write(dir: &Path, rel: &str, json: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    }

    const USER: &str = r#"{"version":1,"request":{"method":"GET","path":"/user"},"response":{"status":200}}"#;

    #[test]
    fn canonical_set_passes() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(&set, "user/GET.json", USER);
        write(
            &set,
            "user/GET.2.json",
            r#"{"version":1,"request":{"method":"GET","path":"/user","body":1},"response":{"status":200}}"#,
        );
        let report = lint_sets(&[set]);
        assert_eq!(report.problems, []);
        assert_eq!(report.fixtures, 2);
    }

    #[test]
    fn reports_names_methods_paths_and_duplicates() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(&set, "user/GET.json", USER);
        write(&set, "me.json", USER);
        write(
            &set,
            "x/get.json",
            r#"{"version":1,"request":{"method":"get","path":"x/"},"response":{"status":200}}"#,
        );
        let report = lint_sets(&[set]);
        let text: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
        assert!(
            text.iter().any(|p| p.contains("identical request matchers")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|p| p == "base/me.json: non-canonical file name; expected user/GET.json"),
            "{text:?}"
        );
        assert!(text.iter().any(|p| p.contains("should be uppercase")), "{text:?}");
        assert!(
            text.iter().any(|p| p.contains(r#"should be written as "/x""#)),
            "{text:?}"
        );
    }

    #[test]
    fn per_page_without_page_is_reported() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(
            &set,
            "x/GET.page-1_per_page-2.json",
            r#"{"version":1,"request":{"method":"GET","path":"/x","query":{"per_page":"2"}},"response":{"status":200}}"#,
        );
        let report = lint_sets(&[set]);
        let text: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            text,
            [r#"base/x/GET.page-1_per_page-2.json: query has per_page but no page; write "page": "1""#]
        );
    }

    #[test]
    fn body_file_must_be_a_sibling() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(&set, "outside.txt", "x");
        write(
            &set,
            "user/GET.json",
            r#"{"version":1,"request":{"method":"GET","path":"/user"},"response":{"status":200,"body_file":"../outside.txt"}}"#,
        );
        let report = lint_sets(&[set]);
        let text: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            text,
            [r#"base/user/GET.json: body_file "../outside.txt" must be a file name in the fixture's directory"#]
        );
    }

    #[test]
    fn numbered_names() {
        assert!(name_matches("GET.json", "GET.json"));
        assert!(name_matches("GET.3.json", "GET.json"));
        assert!(!name_matches("GET.x.json", "GET.json"));
        assert!(!name_matches("GET..json", "GET.json"));
    }
}
