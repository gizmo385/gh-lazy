//! Loading fixture sets and matching requests against them.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fixture::{FORMAT_VERSION, Fixture, RequestKey, normalize_path};
use crate::log::RequestLog;

/// A fixture file read from a set.
#[derive(Debug, Clone)]
pub struct LoadedFixture {
    /// Position of the set in `FAKE_GH_FIXTURES`; lower wins ties.
    pub set_index: usize,
    /// The set directory this fixture came from.
    pub set_dir: PathBuf,
    /// The fixture file on disk.
    pub file: PathBuf,
    /// The file relative to the parent of its set directory, with `/` separators, e.g. `base/user/GET.json`.
    pub display: String,
    pub fixture: Fixture,
}

impl LoadedFixture {
    /// The `body_file` sibling, resolved against the fixture's directory.
    pub fn body_file_path(&self) -> Option<PathBuf> {
        let name = self.fixture.response.body_file.as_ref()?;
        Some(self.file.parent().unwrap_or(Path::new(".")).join(name))
    }

    /// `when` is worth 2 and `body` 1, so the more specific fixture wins.
    fn score(&self) -> u8 {
        2 * u8::from(self.fixture.request.when.is_some()) + u8::from(self.fixture.request.body.is_some())
    }

    fn same_matcher(&self, other: &LoadedFixture) -> bool {
        let (a, b) = (&self.fixture.request, &other.fixture.request);
        a.key() == b.key() && a.body == b.body && a.when == b.when
    }
}

/// Why a fixture set couldn't be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("fixture set {} is not a directory", .0.display())]
    MissingSet(PathBuf),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Parse { path: PathBuf, source: serde_json::Error },
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    #[error("{a} and {b} have identical request matchers")]
    Duplicate { a: String, b: String },
}

/// An incoming request, normalized.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub key: RequestKey,
    /// The body: parsed JSON when it parses, otherwise a JSON string holding the text.
    pub body: Option<Value>,
}

impl Request {
    /// Turns raw body text into the value fixtures are compared against.
    pub fn body_value(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
    }
}

/// The result of [`FixtureStore::lookup`].
#[derive(Debug)]
pub enum Lookup<'a> {
    Hit(&'a LoadedFixture),
    Miss,
    /// Two fixtures in one set tie for the best score.
    Ambiguous(Vec<&'a LoadedFixture>),
}

/// Every fixture from the configured sets.
#[derive(Debug, Default)]
pub struct FixtureStore {
    fixtures: Vec<LoadedFixture>,
}

impl FixtureStore {
    /// Loads every set. The first problem found is returned.
    pub fn load(sets: &[PathBuf]) -> Result<Self, LoadError> {
        let mut fixtures = Vec::new();
        for (index, dir) in sets.iter().enumerate() {
            let (mut loaded, mut errors) = scan_set(dir, index);
            errors.extend(find_duplicates(&loaded));
            if let Some(err) = errors.into_iter().next() {
                return Err(err);
            }
            fixtures.append(&mut loaded);
        }
        Ok(FixtureStore { fixtures })
    }

    pub fn fixtures(&self) -> &[LoadedFixture] {
        &self.fixtures
    }

    /// Finds the fixture that answers `req`, given what the log has served so far.
    pub fn lookup(&self, req: &Request, log: &RequestLog) -> Lookup<'_> {
        let mut candidates: Vec<&LoadedFixture> = self
            .fixtures
            .iter()
            .filter(|f| {
                let r = &f.fixture.request;
                r.key() == req.key
                    && r.body.as_ref().is_none_or(|b| req.body.as_ref() == Some(b))
                    && r.when
                        .as_ref()
                        .is_none_or(|w| w.after.iter().all(|m| log.has_served(m)))
            })
            .collect();
        candidates.sort_by_key(|f| (std::cmp::Reverse(f.score()), f.set_index));
        match candidates.as_slice() {
            [] => Lookup::Miss,
            [best, next, ..] if best.score() == next.score() && best.set_index == next.set_index => Lookup::Ambiguous(
                candidates
                    .iter()
                    .copied()
                    .take_while(|f| f.score() == best.score() && f.set_index == best.set_index)
                    .collect(),
            ),
            [best, ..] => Lookup::Hit(best),
        }
    }
}

/// Reads every fixture in one set, collecting all problems rather than stopping at the first. `lint` uses this
/// directly; [`FixtureStore::load`] stops at the first error.
pub fn scan_set(dir: &Path, set_index: usize) -> (Vec<LoadedFixture>, Vec<LoadError>) {
    let mut fixtures = Vec::new();
    let mut errors = Vec::new();
    if !dir.is_dir() {
        errors.push(LoadError::MissingSet(dir.to_path_buf()));
        return (fixtures, errors);
    }
    let mut files = Vec::new();
    if let Err(err) = walk(dir, &mut files) {
        errors.push(err);
    }
    files.sort();
    let base = dir.parent().unwrap_or(Path::new(""));
    for file in files {
        match load_file(&file) {
            Ok(fixture) => fixtures.push(LoadedFixture {
                set_index,
                set_dir: dir.to_path_buf(),
                display: display_path(file.strip_prefix(base).unwrap_or(&file)),
                file,
                fixture,
            }),
            Err(err) => errors.push(err),
        }
    }
    (fixtures, errors)
}

/// Pairs of fixtures in one set with identical matchers.
pub fn find_duplicates(fixtures: &[LoadedFixture]) -> Vec<LoadError> {
    let mut errors = Vec::new();
    for (i, a) in fixtures.iter().enumerate() {
        for b in &fixtures[i + 1..] {
            if a.set_index == b.set_index && a.same_matcher(b) {
                errors.push(LoadError::Duplicate {
                    a: a.display.clone(),
                    b: b.display.clone(),
                });
            }
        }
    }
    errors
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let io_err = |source| LoadError::Io {
        path: dir.to_path_buf(),
        source,
    };
    for entry in std::fs::read_dir(dir).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let path = entry.path();
        // `file_type` doesn't follow symlinks, so a symlinked directory (possibly a loop) is never recursed into.
        // A symlink to a file is still read.
        let file_type = entry.file_type().map_err(io_err)?;
        if file_type.is_dir() {
            walk(&path, out)?;
        } else if (file_type.is_file() || (file_type.is_symlink() && path.is_file())) && is_fixture_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// `*.json`, except `*.body.*` siblings that hold a fixture's `body_file`.
pub fn is_fixture_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let is_json = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
    is_json && !name.contains(".body.")
}

fn display_path(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn load_file(path: &Path) -> Result<Fixture, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut fixture: Fixture = serde_json::from_str(&text).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let invalid = |message: String| LoadError::Invalid {
        path: path.to_path_buf(),
        message,
    };
    if fixture.version != FORMAT_VERSION {
        return Err(invalid(format!(
            "unsupported fixture version {} (expected {FORMAT_VERSION})",
            fixture.version
        )));
    }
    let r = &fixture.response;
    let bodies = [r.body.is_some(), r.body_text.is_some(), r.body_file.is_some()];
    if bodies.iter().filter(|b| **b).count() > 1 {
        return Err(invalid(
            "response has more than one of body, body_text and body_file".into(),
        ));
    }
    if let Some(name) = &r.body_file {
        let sibling = path.parent().unwrap_or(Path::new(".")).join(name);
        if !sibling.is_file() {
            return Err(invalid(format!("body_file {name} does not exist")));
        }
    }
    // Lenient on load; `lint` reports keys that weren't written normalized.
    fixture.request.method = fixture.request.method.to_ascii_uppercase();
    fixture.request.path = normalize_path(&fixture.request.path);
    Ok(fixture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Accept, query_from_pairs};
    use crate::log::LogEntry;
    use crate::test_support::TempDir;

    fn write(dir: &Path, rel: &str, json: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    }

    fn fixture(method: &str, path: &str, extra_request: &str, status: u16) -> String {
        format!(
            r#"{{"version":1,"request":{{"method":"{method}","path":"{path}"{extra_request}}},"response":{{"status":{status}}}}}"#
        )
    }

    fn put_merge(body: Option<Value>) -> Request {
        Request {
            key: RequestKey::from_url("PUT", "/repos/o/r/pulls/1/merge", &[], Accept::Json).unwrap(),
            body,
        }
    }

    fn status_of(lookup: &Lookup<'_>) -> u16 {
        match lookup {
            Lookup::Hit(f) => f.fixture.response.status,
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    #[test]
    fn when_beats_body_beats_plain() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(&set, "a/PUT.json", &fixture("PUT", "/repos/o/r/pulls/1/merge", "", 201));
        write(
            &set,
            "a/PUT.2.json",
            &fixture(
                "PUT",
                "/repos/o/r/pulls/1/merge",
                r#","body":{"merge_method":"squash"}"#,
                202,
            ),
        );
        write(
            &set,
            "a/PUT.3.json",
            &fixture(
                "PUT",
                "/repos/o/r/pulls/1/merge",
                r#","when":{"after":[{"method":"GET","path":"/user"}]}"#,
                203,
            ),
        );
        let store = FixtureStore::load(std::slice::from_ref(&set)).unwrap();
        let squash = Some(serde_json::json!({"merge_method": "squash"}));

        let empty = RequestLog::from_entries(vec![]);
        assert_eq!(status_of(&store.lookup(&put_merge(None), &empty)), 201);
        assert_eq!(status_of(&store.lookup(&put_merge(squash.clone()), &empty)), 202);

        let after_user = RequestLog::from_entries(vec![LogEntry {
            method: Some("GET".into()),
            path: Some("/user".into()),
            fixture: Some("base/user/GET.json".into()),
            ..LogEntry::default()
        }]);
        assert_eq!(status_of(&store.lookup(&put_merge(squash.clone()), &after_user)), 203);
        // With no log configured, `when` is always false.
        assert_eq!(
            status_of(&store.lookup(&put_merge(squash), &RequestLog::disabled())),
            202
        );
    }

    #[test]
    fn earlier_set_overrides_later() {
        let tmp = TempDir::new();
        let overlay = tmp.path().join("overlay");
        let base = tmp.path().join("base");
        write(&base, "user/GET.json", &fixture("GET", "/user", "", 200));
        write(&overlay, "user/GET.json", &fixture("GET", "/user", "", 500));
        let store = FixtureStore::load(&[overlay, base]).unwrap();
        let req = Request {
            key: RequestKey::from_url("GET", "/user", &[], Accept::Json).unwrap(),
            body: None,
        };
        let Lookup::Hit(hit) = store.lookup(&req, &RequestLog::disabled()) else {
            panic!("expected a hit")
        };
        assert_eq!(hit.fixture.response.status, 500);
        assert_eq!(hit.display, "overlay/user/GET.json");
    }

    #[test]
    fn duplicate_in_one_set_is_a_load_error() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        // Same key written with a different query order and file name: still a duplicate.
        write(
            &set,
            "x/GET.json",
            &fixture("GET", "/x", r#","query":{"a":"1","b":"2"}"#, 200),
        );
        write(
            &set,
            "elsewhere.json",
            &fixture("GET", "/x/", r#","query":{"b":"2","a":"1"}"#, 200),
        );
        let err = FixtureStore::load(&[set]).unwrap_err();
        let LoadError::Duplicate { a, b } = &err else {
            panic!("expected a duplicate, got {err}")
        };
        assert_eq!((a.as_str(), b.as_str()), ("base/elsewhere.json", "base/x/GET.json"));
    }

    #[test]
    fn same_key_in_two_sets_is_not_a_duplicate() {
        let tmp = TempDir::new();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        write(&a, "x/GET.json", &fixture("GET", "/x", "", 200));
        write(&b, "x/GET.json", &fixture("GET", "/x", "", 200));
        assert!(FixtureStore::load(&[a, b]).is_ok());
    }

    #[test]
    fn ambiguous_tie_at_request_time() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        // Different `when`s, both satisfied: same score, same set.
        write(
            &set,
            "x/GET.json",
            &fixture("GET", "/x", r#","when":{"after":[{"method":"GET","path":"/a"}]}"#, 200),
        );
        write(
            &set,
            "x/GET.2.json",
            &fixture("GET", "/x", r#","when":{"after":[{"method":"GET","path":"/b"}]}"#, 200),
        );
        let store = FixtureStore::load(&[set]).unwrap();
        let served = |p: &str| LogEntry {
            method: Some("GET".into()),
            path: Some(p.into()),
            fixture: Some("f".into()),
            ..LogEntry::default()
        };
        let log = RequestLog::from_entries(vec![served("/a"), served("/b")]);
        let req = Request {
            key: RequestKey::from_url("GET", "/x", &[], Accept::Json).unwrap(),
            body: None,
        };
        assert!(matches!(store.lookup(&req, &log), Lookup::Ambiguous(v) if v.len() == 2));
    }

    #[test]
    fn query_and_accept_must_match() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(
            &set,
            "x/GET.json",
            &fixture("GET", "/x", r#","query":{"page":"1"},"accept":"diff""#, 200),
        );
        let store = FixtureStore::load(&[set]).unwrap();
        let mut key = RequestKey::from_url("GET", "/x?page=1", &[], Accept::Diff).unwrap();
        let log = RequestLog::disabled();
        assert!(matches!(
            store.lookup(
                &Request {
                    key: key.clone(),
                    body: None
                },
                &log
            ),
            Lookup::Hit(_)
        ));
        key.query = query_from_pairs([("page", "2")]);
        assert!(matches!(
            store.lookup(
                &Request {
                    key: key.clone(),
                    body: None
                },
                &log
            ),
            Lookup::Miss
        ));
        key.query = query_from_pairs([("page", "1")]);
        key.accept = Accept::Json;
        assert!(matches!(store.lookup(&Request { key, body: None }, &log), Lookup::Miss));
    }

    #[test]
    fn invalid_files_are_load_errors() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(
            &set,
            "x/GET.json",
            r#"{"version":1,"request":{"method":"GET","path":"/x"},"response":{"status":200,"body":1,"body_text":"x"}}"#,
        );
        assert!(matches!(
            FixtureStore::load(std::slice::from_ref(&set)),
            Err(LoadError::Invalid { .. })
        ));
        write(
            &set,
            "x/GET.json",
            r#"{"version":1,"request":{"method":"GET","path":"/x"},"response":{"status":200,"body_file":"nope.body.txt"}}"#,
        );
        assert!(matches!(
            FixtureStore::load(std::slice::from_ref(&set)),
            Err(LoadError::Invalid { .. })
        ));
        write(&set, "x/GET.json", "{");
        assert!(matches!(
            FixtureStore::load(std::slice::from_ref(&set)),
            Err(LoadError::Parse { .. })
        ));
        assert!(matches!(
            FixtureStore::load(&[tmp.path().join("missing")]),
            Err(LoadError::MissingSet(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_not_followed() {
        let tmp = TempDir::new();
        let set = tmp.path().join("base");
        write(&set, "user/GET.json", &fixture("GET", "/user", "", 200));
        // A loop back to the set root would recurse forever if followed.
        std::os::unix::fs::symlink(&set, set.join("user").join("loop")).unwrap();
        std::os::unix::fs::symlink(set.join("user/GET.json"), set.join("linked.json")).unwrap();
        let (loaded, errors) = scan_set(&set, 0);
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<&str> = loaded.iter().map(|f| f.display.as_str()).collect();
        assert_eq!(names, ["base/linked.json", "base/user/GET.json"]);
    }

    #[test]
    fn body_siblings_are_not_fixtures() {
        assert!(is_fixture_file(Path::new("a/GET.json")));
        assert!(!is_fixture_file(Path::new("a/GET.diff.body.json")));
        assert!(!is_fixture_file(Path::new("a/GET.diff.body.diff")));
        assert!(!is_fixture_file(Path::new("a/recipe.toml")));
    }
}
