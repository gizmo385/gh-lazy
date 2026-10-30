//! Create PR drafts: the form saved per repo so reopening Create PR picks up where it was left. Python's
//! `lib/pr_drafts.py`.
//!
//! A draft is `<cache_directory>/pr_drafts/<owner>_<repo>.json`, one JSON object with the seven keys of Python's
//! `PullRequestDraft`, pretty-printed with a 2-space indent. Either app reads the other's file: the JSON values are
//! the same, though Python escapes non-ASCII text as `\uXXXX` and Rust writes it as UTF-8. A file that doesn't load
//! is ignored, with a warning in the log.
//!
//! This module names, parses and builds the files, and saves and clears them blocking. The UI asks its runtime to do
//! the reading and writing off the async threads.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The folder under the cache directory that holds the drafts.
const DRAFTS_DIR: &str = "pr_drafts";

/// One repo's saved Create PR form, with Python's field names and order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrDraft {
    /// The repo it's for, `owner/name`.
    pub repo_full_name: String,
    /// The title, stripped when saved.
    pub title: String,
    /// The description, stripped when saved.
    pub description: String,
    /// Base, as typed.
    pub base_ref: String,
    /// Compare, as typed.
    pub head_ref: String,
    /// The Draft switch.
    pub is_draft: bool,
    /// The listed reviewers, in list order.
    pub reviewers: Vec<String>,
}

impl PrDraft {
    /// Whether there's nothing worth keeping: no title, description or reviewers. Saving such a draft deletes the
    /// file instead.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.description.is_empty() && self.reviewers.is_empty()
    }

    /// Whether restoring it is worth a toast: a title or description with non-blank text, or any reviewers.
    pub fn has_content(&self) -> bool {
        !self.title.trim().is_empty() || !self.description.trim().is_empty() || !self.reviewers.is_empty()
    }
}

/// The draft file for the repo `repo_full_name` (`owner/name`) under `cache_dir` (the config's
/// `cache.cache_directory`): `<cache_dir>/pr_drafts/<owner>_<name>.json`.
pub fn draft_path(cache_dir: &Path, repo_full_name: &str) -> PathBuf {
    cache_dir
        .join(DRAFTS_DIR)
        .join(format!("{}.json", repo_full_name.replace('/', "_")))
}

/// Whether `path` is a draft file: one in a `pr_drafts` folder, as [`draft_path`] names them. The runtime's cache
/// writer uses it to log a draft's save and clear as Python does.
pub fn is_draft_path(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .is_some_and(|dir| dir == DRAFTS_DIR)
}

/// Why a draft file was ignored. Each message is the Python app's warning for the same case; the detail after it is
/// the library's own wording, which differs between the apps.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// The text isn't JSON.
    #[error("Failed to parse PR draft file '{}' as valid JSON: {source}. Ignoring draft.", path.display())]
    Json {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        source: serde_json::Error,
    },
    /// The object has a key missing or one it shouldn't, or a value of the wrong type.
    #[error("PR draft file '{}' has invalid structure: {message}. Ignoring draft.", path.display())]
    Structure {
        /// The file.
        path: PathBuf,
        /// Which key, and what's wrong with it.
        message: String,
    },
    /// Anything else: the JSON isn't an object, or the file couldn't be read.
    #[error("Unexpected error loading PR draft from '{}': {message}. Ignoring draft.", path.display())]
    Unexpected {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

/// Parses a draft file's text.
///
/// # Errors
///
/// When the text isn't JSON, isn't an object, or isn't exactly the seven keys with values of their types.
pub fn try_parse(path: &Path, text: &str) -> Result<PrDraft, DraftError> {
    let value: Value = serde_json::from_str(text).map_err(|source| DraftError::Json {
        path: path.to_owned(),
        source,
    })?;
    // Python unpacks the value into the dataclass's keyword arguments, which fails outside the structure check for
    // anything but an object.
    if !value.is_object() {
        return Err(DraftError::Unexpected {
            path: path.to_owned(),
            message: "the draft is not a JSON object".to_owned(),
        });
    }
    serde_path_to_error::deserialize(value).map_err(|error| DraftError::Structure {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

/// The draft in a file, from the result of reading it as the runtime reports it (`cache::read`, with the error as
/// text): none for a missing file, silently, and none with the warning for one that couldn't be read or parsed.
pub fn load(path: &Path, read: Result<Option<String>, String>) -> Option<PrDraft> {
    let error = match read {
        Ok(Some(text)) => match try_parse(path, &text) {
            Ok(draft) => return Some(draft),
            Err(error) => error,
        },
        Ok(None) => return None,
        Err(message) => DraftError::Unexpected {
            path: path.to_owned(),
            message,
        },
    };
    tracing::warn!("{error}");
    None
}

/// The file's text for `draft`: the seven keys in Python's order, with a 2-space indent and no trailing newline.
pub fn to_json(draft: &PrDraft) -> String {
    // Infallible: a derived `Serialize` of strings, a boolean and a list of strings, written to a `String`.
    serde_json::to_string_pretty(draft).unwrap_or_else(|error| unreachable!("a PR draft failed to serialize: {error}"))
}

/// Writes a draft file, creating its folder first when it's missing. The file is replaced whole, so a failure (a
/// full disk, say) leaves the previous draft.
///
/// # Errors
///
/// When the folder can't be created or the file can't be written.
pub fn save(path: &Path, text: &str) -> io::Result<()> {
    save_with(path, text, |file, bytes| file.write_all(bytes))
}

/// [`save`], with `write_all` putting the bytes into the temp file, so tests can fill the disk.
fn save_with(
    path: &Path,
    text: &str,
    write_all: impl FnOnce(&mut dyn io::Write, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic_write::write_with(path, text.as_bytes(), write_all)?;
    tracing::debug!("Saved PR draft to {}", path.display());
    Ok(())
}

/// Deletes a draft file. A missing one is fine.
///
/// # Errors
///
/// When the file exists but can't be deleted.
pub fn clear(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            tracing::debug!("Cleared PR draft at {}", path.display());
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> PrDraft {
        PrDraft {
            repo_full_name: "o/r".into(),
            title: "A title".into(),
            description: "One.\n\nTwo.".into(),
            base_ref: "main".into(),
            head_ref: "feature/x".into(),
            is_draft: true,
            reviewers: vec!["friend".into()],
        }
    }

    fn path() -> PathBuf {
        draft_path(Path::new("cache"), "o/r")
    }

    /// What `f` logs at `DEBUG` and above, one trimmed line per event as `LEVEL message`, and its result.
    fn logs<R>(f: impl FnOnce() -> R) -> (R, String) {
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
            .with_max_level(tracing::Level::DEBUG)
            .without_time()
            .with_target(false)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = out.lines().map(str::trim).collect();
        (result, lines.join("\n"))
    }

    #[test]
    fn drafts_are_named_per_repo_in_their_own_folder() {
        let path = draft_path(Path::new("cache-dir"), "parity-bot/pr-details");
        assert_eq!(
            path,
            Path::new("cache-dir")
                .join("pr_drafts")
                .join("parity-bot_pr-details.json")
        );
        assert!(is_draft_path(&path));
        assert!(!is_draft_path(
            &Path::new("cache-dir").join("parity-bot_pr-details_issues.json")
        ));
        assert!(!is_draft_path(Path::new("pr_drafts")));
    }

    #[test]
    fn whitespace_counts_for_emptiness_but_not_for_content() {
        assert!(PrDraft::default().is_empty());
        assert!(!PrDraft::default().has_content());
        let blank = PrDraft {
            title: "  ".into(),
            description: "\n\t".into(),
            ..PrDraft::default()
        };
        assert!(!blank.is_empty());
        assert!(!blank.has_content());
        let reviewers_only = PrDraft {
            reviewers: vec!["friend".into()],
            ..PrDraft::default()
        };
        assert!(!reviewers_only.is_empty());
        assert!(reviewers_only.has_content());
        let described = PrDraft {
            description: " body ".into(),
            ..PrDraft::default()
        };
        assert!(described.has_content());
        assert!(draft().has_content());
    }

    #[test]
    fn to_json_round_trips_with_a_two_space_indent_in_pythons_order() {
        let text = to_json(&draft());
        assert!(
            text.starts_with("{\n  \"repo_full_name\": \"o/r\",\n  \"title\": \"A title\","),
            "{text}"
        );
        assert!(text.ends_with("\n  ]\n}"), "{text}");
        assert_eq!(try_parse(&path(), &text).unwrap(), draft());
        let non_ascii = PrDraft {
            title: "Ünïcödé — ✓".into(),
            ..draft()
        };
        assert_eq!(try_parse(&path(), &to_json(&non_ascii)).unwrap(), non_ascii);
    }

    #[test]
    fn text_that_isnt_json_is_a_parse_warning() {
        let error = try_parse(&path(), "{\"title\": ").unwrap_err();
        assert!(matches!(error, DraftError::Json { .. }));
        let message = error.to_string();
        let prefix = format!("Failed to parse PR draft file '{}' as valid JSON: ", path().display());
        assert!(message.starts_with(&prefix), "{message}");
        assert!(message.ends_with(". Ignoring draft."), "{message}");
    }

    #[test]
    fn a_missing_or_unknown_key_or_a_wrong_type_is_a_structure_warning() {
        let mut value = serde_json::to_value(draft()).unwrap();
        value.as_object_mut().unwrap().remove("head_ref");
        let missing = try_parse(&path(), &value.to_string()).unwrap_err();
        assert!(matches!(missing, DraftError::Structure { .. }), "{missing}");
        assert!(missing.to_string().contains("head_ref"), "{missing}");
        let prefix = format!("PR draft file '{}' has invalid structure: ", path().display());
        assert!(missing.to_string().starts_with(&prefix), "{missing}");

        let mut value = serde_json::to_value(draft()).unwrap();
        value["labels"] = Value::Array(Vec::new());
        let unknown = try_parse(&path(), &value.to_string()).unwrap_err();
        assert!(matches!(unknown, DraftError::Structure { .. }), "{unknown}");
        assert!(unknown.to_string().contains("labels"), "{unknown}");

        let mut value = serde_json::to_value(draft()).unwrap();
        value["is_draft"] = Value::String("yes".into());
        let wrong = try_parse(&path(), &value.to_string()).unwrap_err();
        assert!(matches!(wrong, DraftError::Structure { .. }), "{wrong}");
        assert!(wrong.to_string().contains("is_draft"), "{wrong}");
    }

    #[test]
    fn json_that_isnt_an_object_is_an_unexpected_warning() {
        let error = try_parse(&path(), "[1, 2]").unwrap_err();
        assert!(matches!(error, DraftError::Unexpected { .. }));
        let prefix = format!("Unexpected error loading PR draft from '{}': ", path().display());
        assert!(error.to_string().starts_with(&prefix), "{error}");
    }

    #[test]
    fn load_is_silent_for_no_file_and_warns_for_a_bad_one() {
        let (draft_read, logged) = logs(|| load(&path(), Ok(Some(to_json(&draft())))));
        assert_eq!(draft_read, Some(draft()));
        assert!(logged.is_empty(), "{logged}");
        let (none, logged) = logs(|| load(&path(), Ok(None)));
        assert_eq!(none, None);
        assert!(logged.is_empty(), "{logged}");
        let (bad, logged) = logs(|| load(&path(), Ok(Some("[]".into()))));
        assert_eq!(bad, None);
        assert!(
            logged.contains("WARN Unexpected error loading PR draft from"),
            "{logged}"
        );
        let (unreadable, logged) = logs(|| load(&path(), Err("permission denied".into())));
        assert_eq!(unreadable, None);
        assert!(logged.contains("permission denied. Ignoring draft."), "{logged}");
    }

    #[test]
    fn a_full_disk_fails_the_save_and_keeps_the_previous_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = draft_path(dir.path(), "o/r");
        save(&path, &to_json(&draft())).unwrap();
        let before = std::fs::read(&path).unwrap();
        let (error, logged) = logs(|| save_with(&path, "{}", crate::test_fs::disk_full_after(1)).unwrap_err());
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        // The runtime logs the failure; nothing here claims the draft was saved.
        assert_eq!(logged, "");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_fails_the_save_and_the_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = draft_path(dir.path(), "o/r");
        save(&path, &to_json(&draft())).unwrap();
        let Some(_guard) = crate::test_fs::read_only(path.parent().unwrap()) else {
            return; // Root writes anyway.
        };
        assert_eq!(save(&path, "{}").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(clear(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), to_json(&draft()));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_draft_loads_as_none_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = draft_path(dir.path(), "o/r");
        save(&path, &to_json(&draft())).unwrap();
        let Some(_guard) = crate::test_fs::unreadable(&path) else {
            return; // Root reads anyway.
        };
        let read = crate::cache::read(&path).map_err(|e| e.to_string());
        assert!(read.is_err(), "{read:?}");
        let (loaded, logged) = logs(|| load(&path, read));
        assert_eq!(loaded, None);
        let prefix = format!("WARN Unexpected error loading PR draft from '{}': ", path.display());
        assert!(logged.starts_with(&prefix), "{logged}");
        assert!(logged.ends_with(". Ignoring draft."), "{logged}");
    }

    #[test]
    fn save_creates_the_folder_and_clear_tolerates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = draft_path(dir.path(), "o/r");
        let ((), saved) = logs(|| save(&path, "{}").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        assert_eq!(saved, format!("DEBUG Saved PR draft to {}", path.display()));
        let ((), cleared) = logs(|| clear(&path).unwrap());
        assert!(!path.exists());
        assert_eq!(cleared, format!("DEBUG Cleared PR draft at {}", path.display()));
        let ((), missing) = logs(|| clear(&path).unwrap());
        assert_eq!(missing, "");
    }
}
