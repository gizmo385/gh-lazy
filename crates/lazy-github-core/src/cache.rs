//! The local table caches: the rows of a table saved as JSON so the next start can show them before the live fetch
//! answers. Python's `lib/cache.py`.
//!
//! The files keep the Python app's names and shape, so either app reads the other's: one JSON array of the models,
//! every field written (`null` for none) and timestamps as `2026-04-06T12:00:00Z`. Loading is all or nothing: a file
//! with one bad entry counts as no cache, with a warning in the log.
//!
//! This module only names, parses and builds the files, and reads and writes them blocking. The UI asks its runtime
//! to do the reading and writing off the async threads.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Which cache a file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope<'a> {
    /// App-wide, such as the repos table's `repos.json`.
    App,
    /// One repo's, by full name (`owner/name`), such as its pull requests.
    Repo(&'a str),
}

/// The file for the cache `name` in `scope`, under `dir` (the config's `cache.cache_directory`): `<name>.json` for
/// [`CacheScope::App`] and `<owner>_<repo>_<name>.json` for [`CacheScope::Repo`].
pub fn cache_path(dir: &Path, scope: CacheScope<'_>, name: &str) -> PathBuf {
    let file = match scope {
        CacheScope::App => format!("{name}.json"),
        CacheScope::Repo(full_name) => format!("{}_{name}.json", full_name.replace('/', "_")),
    };
    dir.join(file)
}

/// Why a cache file was ignored. Each message is the Python app's warning for the same case; the detail after it is
/// the JSON library's own wording, which differs between the apps.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The text isn't JSON.
    #[error(
        "Failed to parse cache file '{}' as valid JSON: {source}. Ignoring cache and will fetch fresh data.",
        path.display()
    )]
    Json {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        source: serde_json::Error,
    },
    /// An item doesn't decode as the expected model.
    #[error(
        "Cache file '{}' contains data that doesn't match the expected schema for {type_name}. This can happen when \
         model fields change. Ignoring cache and will fetch fresh data. Error: {message}",
        path.display()
    )]
    Schema {
        /// The file.
        path: PathBuf,
        /// The model's Python class name, such as `Repository`.
        type_name: String,
        /// Which item and field failed, and why.
        message: String,
    },
    /// Anything else: the JSON isn't an array, an item isn't an object, or the file couldn't be read.
    #[error(
        "Unexpected error loading cache from '{}': {message}. Ignoring cache and will fetch fresh data.",
        path.display()
    )]
    Unexpected {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

/// Parses a cache file's text, all or nothing: every item decoded, or the reason the file is ignored.
///
/// `type_name` is the model's name in the Python app (`Repository`, `PartialPullRequest`, `Issue`, `WorkflowRun`),
/// for the schema warning.
///
/// # Errors
///
/// When the text isn't JSON, isn't an array, or any item doesn't decode as `T`.
pub fn try_parse<T: DeserializeOwned>(path: &Path, text: &str, type_name: &str) -> Result<Vec<T>, CacheError> {
    let value: Value = serde_json::from_str(text).map_err(|source| CacheError::Json {
        path: path.to_owned(),
        source,
    })?;
    let Value::Array(items) = value else {
        return Err(CacheError::Unexpected {
            path: path.to_owned(),
            message: format!("expected a JSON array, found {}", kind(&value)),
        });
    };
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            // Python unpacks each item into the model's keyword arguments, which fails outside the schema check for
            // anything but an object.
            if !item.is_object() {
                return Err(CacheError::Unexpected {
                    path: path.to_owned(),
                    message: format!("item {index} is {}, not an object", kind(&item)),
                });
            }
            serde_path_to_error::deserialize(item).map_err(|error| CacheError::Schema {
                path: path.to_owned(),
                type_name: type_name.to_owned(),
                message: format!("item {index}: {error}"),
            })
        })
        .collect()
}

/// Parses a cache file's text, all or nothing, logging the warning for a bad file and returning no items for it.
///
/// See [`try_parse`].
pub fn parse<T: DeserializeOwned>(path: &Path, text: &str, type_name: &str) -> Vec<T> {
    try_parse(path, text, type_name).unwrap_or_else(|error| {
        tracing::warn!("{error}");
        Vec::new()
    })
}

/// The items of a cache file, from the result of reading it as the runtime reports it ([`read`], with the error as
/// text): none for a missing file, silently, and none with the warning for one that couldn't be read or parsed.
pub fn load<T: DeserializeOwned>(path: &Path, read: Result<Option<String>, String>, type_name: &str) -> Vec<T> {
    match read {
        Ok(Some(text)) => parse(path, &text, type_name),
        Ok(None) => Vec::new(),
        Err(message) => {
            let error = CacheError::Unexpected {
                path: path.to_owned(),
                message,
            };
            tracing::warn!("{error}");
            Vec::new()
        }
    }
}

/// The file's text for `items`, as one compact JSON array in the Python app's shape.
pub fn to_json<T: Serialize>(items: impl IntoIterator<Item = T>) -> String {
    let items: Vec<T> = items.into_iter().collect();
    // Infallible for the cached models: derived `Serialize` impls of structs with string keys, strings, numbers,
    // booleans and timestamps, written to a `String`.
    serde_json::to_string(&items).unwrap_or_else(|error| unreachable!("a cache model failed to serialize: {error}"))
}

/// Reads a cache file. `None` when there is no file (or something other than a file) at `path`.
///
/// # Errors
///
/// When the file exists but can't be read, or isn't UTF-8.
pub fn read(path: &Path) -> io::Result<Option<String>> {
    tracing::debug!("Loading cached data from: {}", path.display());
    // Python only reads `is_file()` paths, so a folder in the way is no cache rather than an error.
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read_to_string(path).map(Some)
}

/// Writes a cache file, creating its directory first when it's missing. The file is replaced whole, so a failure
/// (a full disk, say) leaves the previous one.
///
/// # Errors
///
/// When the directory can't be created or the file can't be written.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    write_with(path, text, |file, bytes| file.write_all(bytes))
}

/// [`write`], with `write_all` putting the bytes into the temp file, so tests can fill the disk.
fn write_with(
    path: &Path,
    text: &str,
    write_all: impl FnOnce(&mut dyn io::Write, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    tracing::debug!("Saving cached data to: {}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic_write::write_with(path, text.as_bytes(), write_all)
}

/// Deletes a cache file. A missing one is fine.
///
/// # Errors
///
/// When the file exists but can't be deleted.
pub fn remove(path: &Path) -> io::Result<()> {
    tracing::debug!("Removing cached data at: {}", path.display());
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Row {
        name: String,
        count: u32,
    }

    fn row(name: &str, count: u32) -> Row {
        Row {
            name: name.into(),
            count,
        }
    }

    fn path() -> PathBuf {
        Path::new("cache").join("rows.json")
    }

    #[test]
    fn file_names_for_both_scopes() {
        let dir = Path::new("cache-dir");
        assert_eq!(cache_path(dir, CacheScope::App, "repos"), dir.join("repos.json"));
        assert_eq!(
            cache_path(dir, CacheScope::Repo("gizmo385/gh-lazy"), "issues"),
            dir.join("gizmo385_gh-lazy_issues.json")
        );
        assert_eq!(
            cache_path(dir, CacheScope::Repo("o/my.repo"), "workflow_runs"),
            dir.join("o_my.repo_workflow_runs.json")
        );
    }

    #[test]
    fn every_item_decodes_in_order() {
        let rows: Vec<Row> = parse(&path(), r#"[{"name":"a","count":1},{"name":"b","count":2}]"#, "Row");
        assert_eq!(rows, [row("a", 1), row("b", 2)]);
        let empty: Vec<Row> = parse(&path(), "[]", "Row");
        assert_eq!(empty, []);
    }

    #[test]
    fn text_that_isnt_json_is_ignored_with_the_json_warning() {
        for text in ["", "[{", "not json"] {
            let error = try_parse::<Row>(&path(), text, "Row").unwrap_err();
            assert!(matches!(error, CacheError::Json { .. }), "{error:?}");
            let message = error.to_string();
            let prefix = format!("Failed to parse cache file '{}' as valid JSON: ", path().display());
            assert!(message.starts_with(&prefix), "{message}");
            assert!(
                message.ends_with(". Ignoring cache and will fetch fresh data."),
                "{message}"
            );
        }
    }

    #[test]
    fn json_that_isnt_an_array_is_ignored_with_the_unexpected_warning() {
        for (text, found) in [("{}", "an object"), ("3", "a number"), ("null", "null")] {
            let error = try_parse::<Row>(&path(), text, "Row").unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Unexpected error loading cache from '{}': expected a JSON array, found {found}. Ignoring cache \
                     and will fetch fresh data.",
                    path().display()
                )
            );
        }
        let error = try_parse::<Row>(&path(), r#"[{"name":"a","count":1},5]"#, "Row").unwrap_err();
        assert!(matches!(error, CacheError::Unexpected { .. }), "{error:?}");
        assert!(
            error.to_string().contains("item 1 is a number, not an object"),
            "{error}"
        );
    }

    #[test]
    fn one_bad_item_discards_the_whole_file_with_the_schema_warning() {
        let text = r#"[{"name":"a","count":1},{"name":"b","count":"two"}]"#;
        let error = try_parse::<Row>(&path(), text, "Repository").unwrap_err();
        let message = error.to_string();
        let prefix = format!(
            "Cache file '{}' contains data that doesn't match the expected schema for Repository. This can happen \
             when model fields change. Ignoring cache and will fetch fresh data. Error: item 1: count: ",
            path().display()
        );
        assert!(message.starts_with(&prefix), "{message}");
        let rows: Vec<Row> = parse(&path(), text, "Repository");
        assert_eq!(rows, []);
    }

    /// The levels of what `f` logs at `WARN` and above, one per line, and its result.
    fn warnings<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
        #[derive(Clone, Default)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        crate::test_log::keep_callsites_interested();
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .without_time()
            .with_target(false)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        (result, out.lines().map(|line| line.trim().to_owned()).collect())
    }

    #[test]
    fn parse_logs_one_warning_for_a_bad_file() {
        for text in ["nope", "{}", r#"[{"name":"a"}]"#] {
            let (rows, logged) = warnings(|| parse::<Row>(&path(), text, "Row"));
            assert_eq!(rows, []);
            let [line] = logged.as_slice() else {
                panic!("{logged:?}");
            };
            assert!(line.starts_with("WARN "), "{line}");
            assert!(line.ends_with(&try_parse::<Row>(&path(), text, "Row").unwrap_err().to_string()));
        }
        let (rows, logged) = warnings(|| parse::<Row>(&path(), "[]", "Row"));
        assert_eq!((rows, logged), (vec![], vec![]));
    }

    #[test]
    fn load_is_silent_for_a_missing_file_and_warns_for_an_unreadable_one() {
        let (rows, logged) = warnings(|| load::<Row>(&path(), Ok(None), "Row"));
        assert_eq!((rows, logged), (vec![], vec![]));
        let rows: Vec<Row> = load(&path(), Ok(Some(r#"[{"name":"a","count":1}]"#.into())), "Row");
        assert_eq!(rows, [row("a", 1)]);
        let (rows, logged) = warnings(|| load::<Row>(&path(), Err("stream did not contain valid UTF-8".into()), "Row"));
        assert_eq!(rows, []);
        assert_eq!(
            logged,
            [format!(
                "WARN Unexpected error loading cache from '{}': stream did not contain valid UTF-8. Ignoring cache \
                 and will fetch fresh data.",
                path().display()
            )]
        );
    }

    #[test]
    fn to_json_writes_one_compact_array() {
        assert_eq!(
            to_json([row("a", 1), row("b", 2)]),
            r#"[{"name":"a","count":1},{"name":"b","count":2}]"#
        );
        assert_eq!(to_json(Vec::<Row>::new()), "[]");
        let back: Vec<Row> = parse(&path(), &to_json([&row("c", 3)]), "Row");
        assert_eq!(back, [row("c", 3)]);
    }

    #[test]
    fn a_missing_file_reads_as_none_and_write_creates_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(".cache").join("repos.json");
        assert_eq!(read(&path).unwrap(), None);
        write(&path, "[]").unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some("[]"));
        write(&path, "[1]").unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some("[1]"));
        // A folder where the file should be is no cache.
        assert_eq!(read(dir.path()).unwrap(), None);
    }

    #[test]
    fn a_full_disk_fails_the_write_and_keeps_the_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.json");
        write(&path, "[1]").unwrap();
        let error = write_with(&path, "[1,2,3]", crate::test_fs::disk_full_after(2)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(read(&path).unwrap().as_deref(), Some("[1]"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_fails_the_write_and_keeps_the_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.json");
        write(&path, "[1]").unwrap();
        let Some(_guard) = crate::test_fs::read_only(dir.path()) else {
            return; // Root writes anyway.
        };
        let error = write(&path, "[2]").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(read(&path).unwrap().as_deref(), Some("[1]"));
        // A new cache file fails the same way, and so does one in a folder that would have to be created.
        assert!(write(&dir.path().join("issues.json"), "[]").is_err());
        assert!(write(&dir.path().join("pr_drafts").join("o_r.json"), "{}").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_a_read_error_and_loads_as_no_cache_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.json");
        write(&path, r#"[{"name":"a","count":1}]"#).unwrap();
        let Some(_guard) = crate::test_fs::unreadable(&path) else {
            return; // Root reads anyway.
        };
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let (rows, logged) = warnings(|| load::<Row>(&path, read(&path).map_err(|e| e.to_string()), "Row"));
        assert_eq!(rows, []);
        let [line] = logged.as_slice() else {
            panic!("{logged:?}");
        };
        let prefix = format!("WARN Unexpected error loading cache from '{}': ", path.display());
        assert!(line.starts_with(&prefix), "{line}");
    }

    #[test]
    fn a_write_under_a_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("cache");
        std::fs::write(&blocker, "").unwrap();
        assert!(write(&blocker.join("repos.json"), "[]").is_err());
    }
}
