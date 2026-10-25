//! The user's settings, read from and written to the same `config.json` the Python app uses.
//!
//! Every section and field mirrors `lazy_github/lib/config.py`, in the same order and with the same defaults, so the
//! two apps can share one file. Keys this version doesn't know are kept and written back after the known ones.
//!
//! There is no global instance. The app owns one [`Config`], changes it in place and calls [`Config::save`].

mod lax;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use indexmap::IndexMap;
use jiff::civil::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Unknown keys of a JSON object, in file order.
pub type ExtraFields = Map<String, Value>;

const CONFIG_FILE_NAME: &str = "config.json";

/// Where the config lives: the folder and the `config.json` inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    /// The config folder, which also holds the default cache directory and log file.
    pub dir: PathBuf,
    /// The `config.json` file.
    pub file: PathBuf,
}

impl ConfigPaths {
    /// The real location. On Linux and macOS this is `$HOME/.config/lazy-github`, exactly what the Python app uses:
    /// `XDG_CONFIG_HOME` is ignored and macOS doesn't use `~/Library/Application Support`. On Windows it is the
    /// native config directory plus `lazy-github`.
    pub fn discover() -> Result<Self, ConfigError> {
        #[cfg(windows)]
        {
            use etcetera::BaseStrategy as _;
            let strategy = etcetera::base_strategy::Windows::new().map_err(|_| ConfigError::NoHomeDir)?;
            Ok(Self::in_dir(strategy.config_dir().join("lazy-github")))
        }
        #[cfg(not(windows))]
        {
            let home = etcetera::home_dir().map_err(|_| ConfigError::NoHomeDir)?;
            Ok(Self::for_home(&home))
        }
    }

    /// The location for a given home directory on Linux and macOS: `<home>/.config/lazy-github`.
    pub fn for_home(home: &Path) -> Self {
        Self::in_dir(home.join(".config").join("lazy-github"))
    }

    /// A config folder chosen by the caller, used by tests and the parity harness.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let file = dir.join(CONFIG_FILE_NAME);
        Self { dir, file }
    }
}

/// Why the config couldn't be loaded or saved.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The home directory, which holds the config folder, is unknown.
    #[error("could not determine your home directory")]
    NoHomeDir,
    /// The config file exists but couldn't be read.
    #[error("failed to read {}: {source}", path.display())]
    Read {
        /// The config file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The config file isn't valid JSON.
    #[error(
        "{} is not valid JSON (line {line}, column {column}): {message}. Fix the file or run `lazy-github clear-config`.",
        path.display()
    )]
    Syntax {
        /// The config file.
        path: PathBuf,
        /// 1-based line of the error.
        line: usize,
        /// 1-based column of the error.
        column: usize,
        /// What is wrong.
        message: String,
    },
    /// The config file is valid JSON but a value has the wrong type or isn't allowed.
    #[error("{}: `{field}`: {message}. Fix the value or run `lazy-github clear-config`.", path.display())]
    Invalid {
        /// The config file.
        path: PathBuf,
        /// The dotted path of the bad value, such as `issues.state_filter`.
        field: String,
        /// What is wrong.
        message: String,
    },
    /// The config file or folder couldn't be written.
    #[error("failed to write {}: {source}", path.display())]
    Write {
        /// The file or folder being written.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Which issues or pull requests to list, by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueStateFilter {
    /// Open and closed.
    #[default]
    All,
    /// Only open ones.
    Open,
    /// Only closed ones.
    Closed,
}

/// Which issues or pull requests to list, by author.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueOwnerFilter {
    /// Only ones the authenticated user opened.
    Mine,
    /// Everyone's.
    #[default]
    All,
}

/// How GitHub merges a pull request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    /// A merge commit.
    Merge,
    /// One squashed commit.
    #[default]
    Squash,
    /// The commits rebased onto the base branch.
    Rebase,
}

/// Settings that change the appearance of LazyGithub, including which sections are shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // One bool per setting, mirroring the file.
pub struct AppearanceSettings {
    /// The Textual theme name. Kept as written, even when unknown; the theme code falls back where it's used. A
    /// non-string value loads as `textual-dark`, as in Python.
    #[serde(deserialize_with = "lax::theme")]
    pub theme: String,
    /// Whether the command log is shown on startup.
    #[serde(deserialize_with = "lax::bool")]
    pub show_command_log: bool,
    /// Whether workflows are shown on startup.
    #[serde(deserialize_with = "lax::bool")]
    pub show_workflows: bool,
    /// Whether issues are shown on startup.
    #[serde(deserialize_with = "lax::bool")]
    pub show_issues: bool,
    /// Whether pull requests are shown on startup.
    #[serde(deserialize_with = "lax::bool")]
    pub show_pull_requests: bool,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "textual-dark".to_owned(),
            show_command_log: false,
            show_workflows: true,
            show_issues: true,
            show_pull_requests: true,
            extra: ExtraFields::new(),
        }
    }
}

/// Settings for the optional notifications feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    /// Whether notifications are loaded and shown.
    #[serde(deserialize_with = "lax::bool")]
    pub enabled: bool,
    /// Whether selecting an unread notification marks it as read.
    #[serde(deserialize_with = "lax::bool")]
    pub mark_notification_as_read_when_selected: bool,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mark_notification_as_read_when_selected: true,
            extra: ExtraFields::new(),
        }
    }
}

/// Keybinding overrides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BindingsSettings {
    /// Binding id (such as `app.quit`) to a key string in Textual's syntax (such as `ctrl+q` or `enter,space`),
    /// in file order. The strings are stored untouched.
    pub overrides: IndexMap<String, String>,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Repository settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepositorySettings {
    /// `owner/name` of repositories the user doesn't own but wants listed.
    pub additional_repos_to_track: Vec<String>,
    /// `owner/name` of repositories pinned at the top of the repositories table.
    pub favorites: Vec<String>,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// How pull requests are listed and merged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PullRequestSettings {
    /// Which pull requests to list, by state.
    pub state_filter: IssueStateFilter,
    /// Which pull requests to list, by author.
    pub owner_filter: IssueOwnerFilter,
    /// How pull requests are merged.
    pub preferred_merge_method: MergeMethod,
    /// Extra logins to suggest as reviewers.
    pub additional_suggested_pr_reviewers: Vec<String>,
    /// Whether to find and show the pull request for the current commit on startup.
    #[serde(deserialize_with = "lax::bool")]
    pub preload_pull_request_for_current_commit: bool,
    /// A file used as the default body of new pull requests.
    #[serde(
        serialize_with = "lax::serialize_optional_path",
        deserialize_with = "lax::optional_path"
    )]
    pub pull_request_template: Option<PathBuf>,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// How issues are listed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IssueSettings {
    /// Which issues to list, by state.
    pub state_filter: IssueStateFilter,
    /// Which issues to list, by author.
    pub owner_filter: IssueOwnerFilter,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// How long data from the GitHub API is cached. Durations are whole seconds, as in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    /// How long a successful authentication check is trusted, in seconds.
    #[serde(deserialize_with = "lax::u64")]
    pub auth_cache_duration: u64,
    /// When authentication was last checked, in local civil time.
    #[serde(with = "lax::naive_datetime")]
    pub auth_last_checked: Option<DateTime>,
    /// Where request results and table caches are stored. Always set once loaded; it is empty only in
    /// `CacheSettings::default()`, which can't know the config folder.
    #[serde(serialize_with = "lax::serialize_path", deserialize_with = "lax::path")]
    pub cache_directory: PathBuf,
    /// The default cache TTL for requests, in seconds.
    #[serde(deserialize_with = "lax::u64")]
    pub default_ttl: u64,
    /// The cache TTL for listing repositories, in seconds.
    #[serde(deserialize_with = "lax::u64")]
    pub list_repos_ttl: u64,
    /// The cache TTL for listing issues, in seconds.
    #[serde(deserialize_with = "lax::u64")]
    pub list_issues_ttl: u64,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            auth_cache_duration: 24 * 60 * 60,
            auth_last_checked: None,
            cache_directory: PathBuf::new(),
            default_ttl: 10 * 60,
            list_repos_ttl: 24 * 60 * 60,
            list_issues_ttl: 60 * 60,
            extra: ExtraFields::new(),
        }
    }
}

impl CacheSettings {
    /// [`Self::auth_cache_duration`] as a [`Duration`].
    pub fn auth_cache_duration(&self) -> Duration {
        Duration::from_secs(self.auth_cache_duration)
    }

    /// [`Self::default_ttl`] as a [`Duration`].
    pub fn default_ttl(&self) -> Duration {
        Duration::from_secs(self.default_ttl)
    }

    /// [`Self::list_repos_ttl`] as a [`Duration`].
    pub fn list_repos_ttl(&self) -> Duration {
        Duration::from_secs(self.list_repos_ttl)
    }

    /// [`Self::list_issues_ttl`] as a [`Duration`].
    pub fn list_issues_ttl(&self) -> Duration {
        Duration::from_secs(self.list_issues_ttl)
    }
}

/// Shared core behavior, such as logging.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreConfig {
    /// Whether this is the first time LazyGithub has started.
    #[serde(deserialize_with = "lax::bool")]
    pub first_start: bool,
    /// Where the application log is written. Always set once loaded; it is empty only in `CoreConfig::default()`,
    /// which can't know the config folder.
    #[serde(serialize_with = "lax::serialize_path", deserialize_with = "lax::path")]
    pub logfile_path: PathBuf,
    /// How large the log grows before it is rotated, in bytes.
    #[serde(deserialize_with = "lax::u64")]
    pub logfile_max_bytes: u64,
    /// How many rotated logs are kept.
    #[serde(deserialize_with = "lax::u64")]
    pub logfile_count: u64,
    /// Unknown keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            first_start: true,
            logfile_path: PathBuf::new(),
            logfile_max_bytes: 5_000_000,
            logfile_count: 5,
            extra: ExtraFields::new(),
        }
    }
}

/// How the GitHub API is reached.
///
/// Python's `client_type` isn't modeled, since this app always uses `gh`; it is kept in [`Self::extra`] and written
/// back unchanged.
///
/// `ApiConfig::default()` carries Python's default `"client_type": "RAW_HTTP"`, so a new file matches what Python
/// writes. That default applies only when the whole `api` section is missing: an `api` section without the key
/// stays without it, because a flattened `extra` is filled from the input, not from the container default. Both
/// mean `RAW_HTTP` to Python.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    /// The GitHub API base URL.
    pub base_url: String,
    /// Unknown keys, written back on save. This includes Python's `client_type`.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Default for ApiConfig {
    fn default() -> Self {
        // A fresh config carries Python's default `client_type` so a new file matches what the Python app writes.
        let mut extra = ExtraFields::new();
        extra.insert("client_type".to_owned(), Value::String("RAW_HTTP".to_owned()));
        Self {
            base_url: "https://api.github.com".to_owned(),
            extra,
        }
    }
}

/// All of LazyGithub's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Appearance of UI elements.
    pub appearance: AppearanceSettings,
    /// Interaction with GitHub notifications.
    pub notifications: NotificationSettings,
    /// Keybinding overrides.
    pub bindings: BindingsSettings,
    /// Repository settings.
    pub repositories: RepositorySettings,
    /// Pull request settings.
    pub pull_requests: PullRequestSettings,
    /// Issue settings.
    pub issues: IssueSettings,
    /// Caching settings.
    pub cache: CacheSettings,
    /// Shared core behavior, such as logging.
    pub core: CoreConfig,
    /// How the GitHub API is reached.
    pub api: ApiConfig,
    /// Unknown top-level keys, written back on save.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

impl Config {
    /// The defaults, with the paths that live in the config folder filled in.
    pub fn default_in(paths: &ConfigPaths) -> Self {
        let mut config = Self::default();
        config.resolve(paths);
        config
    }

    /// Loads the config. A missing file gives the defaults and is not created; missing sections and fields take
    /// their defaults.
    pub fn load(paths: &ConfigPaths) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(&paths.file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default_in(paths)),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: paths.file.clone(),
                    source,
                });
            }
        };
        let mut config = Self::from_json(&text, &paths.file)?;
        config.resolve(paths);
        Ok(config)
    }

    fn from_json(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let to_error = |field: String, inner: &serde_json::Error| {
            let message = strip_position(inner);
            if inner.is_data() {
                ConfigError::Invalid {
                    path: path.to_owned(),
                    field,
                    message,
                }
            } else {
                ConfigError::Syntax {
                    path: path.to_owned(),
                    line: inner.line(),
                    column: inner.column(),
                    message,
                }
            }
        };
        let config = serde_path_to_error::deserialize(&mut deserializer).map_err(|err| {
            let field = err.path().to_string();
            to_error(field, err.inner())
        })?;
        // Rejects trailing content after the object, as `json.loads` does.
        deserializer.end().map_err(|err| to_error(String::new(), &err))?;
        Ok(config)
    }

    /// Writes the config atomically, creating the config folder if it's missing.
    pub fn save(&self, paths: &ConfigPaths) -> Result<(), ConfigError> {
        std::fs::create_dir_all(&paths.dir).map_err(|source| ConfigError::Write {
            path: paths.dir.clone(),
            source,
        })?;

        // Write through a symlinked config.json (dotfiles managers), as Python's in-place write does: the temp file
        // sits next to the real target and the rename replaces the target, not the link. `canonicalize` fails when
        // the file doesn't exist yet, which is the first save.
        let target = std::fs::canonicalize(&paths.file).unwrap_or_else(|_| paths.file.clone());
        let mut tmp_name = target.file_name().unwrap_or_default().to_owned();
        tmp_name.push(".tmp");
        let tmp = target.with_file_name(tmp_name);

        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(self.to_json_pretty().as_bytes())?;
            // Keep the mode of an existing file (say, `chmod 600`), as an in-place write would.
            if let Ok(metadata) = std::fs::metadata(&target) {
                file.set_permissions(metadata.permissions())?;
            }
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, &target)
        };
        write().map_err(|source| {
            // Best effort: the original error is the one worth reporting.
            let _ = std::fs::remove_file(&tmp);
            ConfigError::Write {
                path: paths.file.clone(),
                source,
            }
        })
    }

    /// The exact text [`Self::save`] writes: pydantic's `model_dump_json(indent=4)` format, with 4-space
    /// indentation, non-ASCII unescaped and no trailing newline.
    pub fn to_json_pretty(&self) -> String {
        let mut out = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        // Infallible: every key is a string and every value (paths included, see `lax::serialize_path`) serializes
        // without error into a `Vec`.
        self.serialize(&mut serializer).expect("config serializes to JSON");
        // Infallible: serde_json only writes UTF-8.
        String::from_utf8(out).expect("serde_json writes UTF-8")
    }

    /// Fills the defaults that depend on the config folder.
    fn resolve(&mut self, paths: &ConfigPaths) {
        if self.cache.cache_directory.as_os_str().is_empty() {
            self.cache.cache_directory = paths.dir.join(".cache");
        }
        if self.core.logfile_path.as_os_str().is_empty() {
            self.core.logfile_path = paths.dir.join("lazy_github.log");
        }
    }
}

/// `serde_json` appends " at line L column C" to its messages; the position is reported separately.
fn strip_position(err: &serde_json::Error) -> String {
    let message = err.to_string();
    let suffix = format!(" at line {} column {}", err.line(), err.column());
    message.strip_suffix(&suffix).map(str::to_owned).unwrap_or(message)
}

#[cfg(test)]
mod tests {
    use super::lax::naive_datetime;
    use super::*;

    #[test]
    fn discover_location_for_home_ignores_xdg() {
        let paths = ConfigPaths::for_home(Path::new("/home/lazy"));
        assert_eq!(paths.dir, Path::new("/home/lazy/.config/lazy-github"));
        assert_eq!(paths.file, Path::new("/home/lazy/.config/lazy-github/config.json"));
    }

    #[test]
    fn datetime_format_matches_pydantic() {
        let dt: DateTime = "2026-10-24T18:30:05".parse().unwrap();
        assert_eq!(naive_datetime::format(dt), "2026-10-24T18:30:05");
        let dt: DateTime = "2026-10-24T18:30:05.12".parse().unwrap();
        assert_eq!(naive_datetime::format(dt), "2026-10-24T18:30:05.120000");
    }

    #[test]
    fn datetime_truncates_to_microseconds() {
        let dt = naive_datetime::parse("2024-01-02T03:04:05.1234567").unwrap();
        assert_eq!(naive_datetime::format(dt), "2024-01-02T03:04:05.123456");
    }

    #[test]
    fn datetime_accepts_space_separator() {
        let dt = naive_datetime::parse("2024-01-02 03:04:05").unwrap();
        assert_eq!(naive_datetime::format(dt), "2024-01-02T03:04:05");
    }
}
