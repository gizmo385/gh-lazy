//! The Search modal's recent queries: `search_history.json` in the cache directory ([`CacheScope::App`], as
//! `repos.json`), one JSON array of strings, newest first, at most [`MAX_RECENT`]. LazyGithub-only: the Python app has
//! no search history (decision `search-issues-and-prs`).
//!
//! As with the table caches, this module only names, parses and builds the file; the UI's runtime reads and writes
//! it off the async threads ([`crate::cache::read`], [`crate::cache::write`]).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cache::{self, CacheError, CacheScope};

/// The file's cache name: `search_history.json`.
pub const SEARCH_HISTORY: &str = "search_history";

/// How many recent queries are kept.
pub const MAX_RECENT: usize = 20;

/// The history file under `dir`, the config's `cache.cache_directory`.
pub fn path(dir: &Path) -> PathBuf {
    cache::cache_path(dir, CacheScope::App, SEARCH_HISTORY)
}

/// Records `query`, trimmed, as the newest entry: an equal entry moves to the front, and only the newest
/// [`MAX_RECENT`] are kept. A blank query changes nothing. True when the list changed.
pub fn record(history: &mut Vec<String>, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() || history.first().is_some_and(|newest| newest == query) {
        return false;
    }
    history.retain(|entry| entry != query);
    history.insert(0, query.to_owned());
    history.truncate(MAX_RECENT);
    true
}

/// Parses the file's text, all or nothing: the queries, newest first and cut to [`MAX_RECENT`], or why the file is
/// ignored, with the table caches' wording.
///
/// # Errors
///
/// When the text isn't JSON, isn't an array, or an item isn't a string.
pub fn try_parse(path: &Path, text: &str) -> Result<Vec<String>, CacheError> {
    let value: Value = serde_json::from_str(text).map_err(|source| CacheError::Json {
        path: path.to_owned(),
        source,
    })?;
    let unexpected = |message: String| CacheError::Unexpected {
        path: path.to_owned(),
        message,
    };
    let Value::Array(items) = value else {
        return Err(unexpected(format!(
            "expected a JSON array, found {}",
            cache::kind(&value)
        )));
    };
    let mut queries = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| match item {
            Value::String(query) => Ok(query),
            other => Err(unexpected(format!(
                "item {index} is {}, not a string",
                cache::kind(&other)
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    queries.truncate(MAX_RECENT);
    Ok(queries)
}

/// The queries from the result of reading the file as the runtime reports it ([`cache::read`], with the error as
/// text): none for a missing file, silently, and none with the cache module's warning for one that couldn't be read
/// or parsed.
pub fn load(path: &Path, read: Result<Option<String>, String>) -> Vec<String> {
    let parsed = match read {
        Ok(Some(text)) => try_parse(path, &text),
        Ok(None) => return Vec::new(),
        Err(message) => Err(CacheError::Unexpected {
            path: path.to_owned(),
            message,
        }),
    };
    parsed.unwrap_or_else(|error| {
        tracing::warn!("{error}");
        Vec::new()
    })
}

/// The file's text for `history`: one compact JSON array of strings.
pub fn to_json(history: &[String]) -> String {
    cache::to_json(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    #[test]
    fn the_file_is_search_history_json_in_the_cache_directory() {
        let dir = Path::new("cache-dir");
        assert_eq!(path(dir), dir.join("search_history.json"));
    }

    #[test]
    fn record_trims_and_puts_the_query_first() {
        let mut history = strings(&["is:open"]);
        assert!(record(&mut history, "  is:pr author:@me \n"));
        assert_eq!(history, strings(&["is:pr author:@me", "is:open"]));
    }

    #[test]
    fn record_moves_an_equal_entry_to_the_front() {
        let mut history = strings(&["a", "b", "c"]);
        assert!(record(&mut history, "c "));
        assert_eq!(history, strings(&["c", "a", "b"]));
        // Already the newest: nothing changes.
        assert!(!record(&mut history, "c"));
        assert_eq!(history, strings(&["c", "a", "b"]));
    }

    #[test]
    fn record_ignores_a_blank_query() {
        let mut history = strings(&["a"]);
        assert!(!record(&mut history, ""));
        assert!(!record(&mut history, "   "));
        assert_eq!(history, strings(&["a"]));
    }

    #[test]
    fn record_keeps_the_newest_twenty() {
        let mut history = Vec::new();
        for n in 0..25 {
            record(&mut history, &format!("q{n}"));
        }
        assert_eq!(history.len(), MAX_RECENT);
        assert_eq!(history.first().map(String::as_str), Some("q24"));
        assert_eq!(history.last().map(String::as_str), Some("q5"));
    }

    #[test]
    fn a_missing_file_is_an_empty_history() {
        assert_eq!(load(&path(Path::new("c")), Ok(None)), Vec::<String>::new());
    }

    #[test]
    fn a_valid_file_loads_newest_first_and_cut_to_twenty() {
        let file = path(Path::new("c"));
        assert_eq!(load(&file, Ok(Some(r#"["b","a"]"#.into()))), strings(&["b", "a"]));
        let many = to_json(&(0..30).map(|n| format!("q{n}")).collect::<Vec<_>>());
        let loaded = load(&file, Ok(Some(many)));
        assert_eq!(loaded.len(), MAX_RECENT);
        assert_eq!(loaded.first().map(String::as_str), Some("q0"));
    }

    #[test]
    fn a_bad_file_is_an_empty_history() {
        let file = path(Path::new("c"));
        for text in ["", "nope", "{}", r#"["a", 3]"#, r#"[{"q":"a"}]"#] {
            assert_eq!(load(&file, Ok(Some(text.into()))), Vec::<String>::new(), "{text}");
        }
        assert_eq!(load(&file, Err("permission denied".into())), Vec::<String>::new());
    }

    #[test]
    fn bad_files_say_why_with_the_cache_wording() {
        let file = path(Path::new("c"));
        let error = try_parse(&file, "{}").unwrap_err();
        assert!(
            error.to_string().contains("expected a JSON array, found an object"),
            "{error}"
        );
        let error = try_parse(&file, r#"["a", 3]"#).unwrap_err();
        assert!(
            error.to_string().contains("item 1 is a number, not a string"),
            "{error}"
        );
        assert!(matches!(try_parse(&file, "nope"), Err(CacheError::Json { .. })));
    }

    #[test]
    fn to_json_round_trips() {
        let history = strings(&["is:pr is:open", "label:\"good first issue\""]);
        let text = to_json(&history);
        assert_eq!(text, r#"["is:pr is:open","label:\"good first issue\""]"#);
        assert_eq!(try_parse(Path::new("f"), &text).unwrap(), history);
    }
}
