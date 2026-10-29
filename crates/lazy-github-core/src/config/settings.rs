//! What the Settings modal lists, and how each value is shown and written back.
//!
//! [`SECTIONS`] is the single source for the modal's tabs: one [`Section`] per `config.json` section, in file order,
//! each with its [`Field`]s in file order. A field reads its value from a [`Config`] as a [`FieldValue`] and writes an
//! edited one back, typed the way the loader reads `config.json`. Each field touches only its own section.
//!
//! `bindings.overrides` has no field here: the Key Bindings tab lists the keymap's bindings instead.

use std::path::PathBuf;

use super::lax::{naive_datetime, parse_int_str};
use super::{Config, DEFAULT_THEME, IssueOwnerFilter, IssueStateFilter, MergeMethod, TEXTUAL_THEMES};

/// One tab of the Settings modal: a `config.json` section.
#[derive(Debug)]
pub struct Section {
    /// The section's key in `config.json`, such as `pull_requests`.
    pub key: &'static str,
    /// The tab title, such as "Pull Requests".
    pub title: &'static str,
    /// The description shown dimmed at the top of the tab: the Python section class's docstring, if it has one.
    pub doc: Option<&'static str>,
    /// The fields the tab lists, in file order. Empty for `bindings`, whose rows come from the keymap.
    pub fields: &'static [Field],
}

/// One setting: a field of a section.
#[derive(Debug)]
pub struct Field {
    /// The key of the section it belongs to, such as `cache`.
    pub section: &'static str,
    /// The field's key in its section, such as `default_ttl`.
    pub name: &'static str,
    /// How the value is edited.
    pub kind: FieldKind,
    get: fn(&Config) -> FieldValue,
    set: fn(&mut Config, FieldValue) -> Result<(), FieldError>,
}

/// How a field is edited, which decides its widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// On or off. The value is a [`FieldValue::Bool`].
    Bool,
    /// One of a fixed set of values, in this order. The value is a [`FieldValue::Choice`] of a [`Choice::value`].
    Choice(&'static [Choice]),
    /// A theme name from [`TEXTUAL_THEMES`], in that order. The value is a [`FieldValue::Choice`].
    Theme,
    /// A list of strings. The value is a [`FieldValue::List`].
    List,
    /// A file path that may be unset, shown blank when unset. The value is a [`FieldValue::Text`].
    OptionalPath,
    /// Free text, checked on write. The value is a [`FieldValue::Text`].
    Text(TextKind),
    /// A choice shown but never written: [`Field::get`] always gives the one value this app supports, and
    /// [`Field::set`] leaves the config alone.
    ReadOnlyChoice(&'static [Choice]),
}

/// What a [`FieldKind::Text`] field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    /// A whole number, written as a JSON number.
    Integer,
    /// A date and time, or `None`.
    Timestamp,
    /// A path that must not be blank.
    Path,
    /// Any text, stored as typed.
    Plain,
}

/// One option of a [`FieldKind::Choice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// What the option shows: Python's `str.title()` of the value, such as `Github_Cli`.
    pub label: &'static str,
    /// The value as `config.json` stores it, such as `GITHUB_CLI`.
    pub value: &'static str,
}

/// A field's value as the modal edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// A [`FieldKind::Bool`] value.
    Bool(bool),
    /// A [`FieldKind::Choice`], [`FieldKind::Theme`] or [`FieldKind::ReadOnlyChoice`] value.
    Choice(&'static str),
    /// A [`FieldKind::List`] value.
    List(Vec<String>),
    /// A [`FieldKind::Text`] or [`FieldKind::OptionalPath`] value, as shown or typed.
    Text(String),
}

/// Why [`Field::set`] refused a value. The message follows the field's label without its colon: "Default Ttl must be
/// a whole number".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    /// An integer field's text isn't a whole number.
    #[error("must be a whole number")]
    NotAWholeNumber,
    /// A timestamp field's text isn't a date and time or `None`.
    #[error("must be a date and time like 2026-10-24 18:30:05, or None")]
    NotATimestamp,
    /// A path field's text is blank.
    #[error("can't be empty")]
    Empty,
    /// The value doesn't fit the field: a different [`FieldValue`] variant than its [`FieldKind`] takes, or a choice
    /// that isn't one of its options. The modal builds values from the field's kind, so this is a caller bug.
    #[error("isn't a valid value")]
    Mismatch,
}

impl Field {
    /// The label shown before the value: the name with `_` as spaces, title-cased as Python's `str.title()` does,
    /// then `:`, such as "Default Ttl:".
    pub fn label(&self) -> String {
        let mut label = py_title(&self.name.replace('_', " "));
        label.push(':');
        label
    }

    /// The value as the modal shows it.
    pub fn get(&self, config: &Config) -> FieldValue {
        (self.get)(config)
    }

    /// Writes an edited value back to this field's section, typed as `config.json` stores it. On error the config is
    /// unchanged.
    pub fn set(&self, config: &mut Config, value: FieldValue) -> Result<(), FieldError> {
        (self.set)(config, value)
    }
}

// One field of each common kind, its section and name taken from the `Config` field path it reads.
macro_rules! bool_field {
    ($section:ident . $name:ident) => {
        Field {
            section: stringify!($section),
            name: stringify!($name),
            kind: FieldKind::Bool,
            get: |c| FieldValue::Bool(c.$section.$name),
            set: |c, v| {
                c.$section.$name = match v {
                    FieldValue::Bool(b) => b,
                    _ => return Err(FieldError::Mismatch),
                };
                Ok(())
            },
        }
    };
}

macro_rules! list_field {
    ($section:ident . $name:ident) => {
        Field {
            section: stringify!($section),
            name: stringify!($name),
            kind: FieldKind::List,
            get: |c| FieldValue::List(c.$section.$name.clone()),
            set: |c, v| {
                c.$section.$name = match v {
                    FieldValue::List(items) => items,
                    _ => return Err(FieldError::Mismatch),
                };
                Ok(())
            },
        }
    };
}

macro_rules! choice_field {
    ($section:ident . $name:ident, $enum:ty, $choices:ident) => {
        Field {
            section: stringify!($section),
            name: stringify!($name),
            kind: FieldKind::Choice($choices),
            get: |c| FieldValue::Choice(c.$section.$name.as_str()),
            set: |c, v| {
                c.$section.$name = <$enum as ChoiceEnum>::from_value(choice(&v)?).ok_or(FieldError::Mismatch)?;
                Ok(())
            },
        }
    };
}

macro_rules! integer_field {
    ($section:ident . $name:ident) => {
        Field {
            section: stringify!($section),
            name: stringify!($name),
            kind: FieldKind::Text(TextKind::Integer),
            get: |c| FieldValue::Text(c.$section.$name.to_string()),
            set: |c, v| {
                c.$section.$name = parse_int_str(&text(v)?).ok_or(FieldError::NotAWholeNumber)?;
                Ok(())
            },
        }
    };
}

macro_rules! path_field {
    ($section:ident . $name:ident) => {
        Field {
            section: stringify!($section),
            name: stringify!($name),
            kind: FieldKind::Text(TextKind::Path),
            get: |c| FieldValue::Text(c.$section.$name.to_string_lossy().into_owned()),
            set: |c, v| {
                let text = text(v)?;
                let text = text.trim();
                if text.is_empty() {
                    return Err(FieldError::Empty);
                }
                c.$section.$name = PathBuf::from(text);
                Ok(())
            },
        }
    };
}

/// The Settings tabs, in `config.json` order.
pub static SECTIONS: &[Section] = &[
    Section {
        key: "appearance",
        title: "Appearance",
        doc: Some(
            "Settings focused on altering the appearance of LazyGithub, including hiding or showing different sections.",
        ),
        fields: &[
            Field {
                section: "appearance",
                name: "theme",
                kind: FieldKind::Theme,
                get: |c| FieldValue::Choice(theme_name(&c.appearance.theme)),
                set: |c, v| {
                    let name = choice(&v)?;
                    if !TEXTUAL_THEMES.contains(&name) {
                        return Err(FieldError::Mismatch);
                    }
                    name.clone_into(&mut c.appearance.theme);
                    Ok(())
                },
            },
            bool_field!(appearance.show_command_log),
            bool_field!(appearance.show_workflows),
            bool_field!(appearance.show_issues),
            bool_field!(appearance.show_pull_requests),
        ],
    },
    Section {
        key: "notifications",
        title: "Notifications",
        doc: Some(
            "Controls the settings for the optional notification feature, which relies on the standard GitHub CLI.",
        ),
        fields: &[
            bool_field!(notifications.enabled),
            bool_field!(notifications.mark_notification_as_read_when_selected),
        ],
    },
    Section {
        key: "bindings",
        title: "Key Bindings",
        doc: Some(
            "Custom keybinding overrides for LazyGithub. When rebinding, pressing ESCAPE will reset to the default binding.",
        ),
        fields: &[],
    },
    Section {
        key: "repositories",
        title: "Repositories",
        doc: Some("Repository-specific settings"),
        fields: &[
            list_field!(repositories.additional_repos_to_track),
            list_field!(repositories.favorites),
        ],
    },
    Section {
        key: "pull_requests",
        title: "Pull Requests",
        doc: Some("Changes how pull requests are retrieved from the Github API"),
        fields: &[
            choice_field!(pull_requests.state_filter, IssueStateFilter, STATE_CHOICES),
            choice_field!(pull_requests.owner_filter, IssueOwnerFilter, OWNER_CHOICES),
            choice_field!(pull_requests.preferred_merge_method, MergeMethod, MERGE_CHOICES),
            list_field!(pull_requests.additional_suggested_pr_reviewers),
            bool_field!(pull_requests.preload_pull_request_for_current_commit),
            Field {
                section: "pull_requests",
                name: "pull_request_template",
                kind: FieldKind::OptionalPath,
                get: |c| {
                    let path = c.pull_requests.pull_request_template.as_deref();
                    FieldValue::Text(path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
                },
                set: |c, v| {
                    let text = text(v)?;
                    let text = text.trim();
                    c.pull_requests.pull_request_template = (!text.is_empty()).then(|| PathBuf::from(text));
                    Ok(())
                },
            },
        ],
    },
    Section {
        key: "issues",
        title: "Issues",
        doc: Some("Changes how issues are retrieved from the Github API"),
        fields: &[
            choice_field!(issues.state_filter, IssueStateFilter, STATE_CHOICES),
            choice_field!(issues.owner_filter, IssueOwnerFilter, OWNER_CHOICES),
        ],
    },
    Section {
        key: "cache",
        title: "Cache",
        doc: Some("Settings that control how long data will be cached from the GitHub API"),
        fields: &[
            integer_field!(cache.auth_cache_duration),
            Field {
                section: "cache",
                name: "auth_last_checked",
                kind: FieldKind::Text(TextKind::Timestamp),
                get: |c| {
                    FieldValue::Text(
                        c.cache
                            .auth_last_checked
                            .map_or_else(|| "None".to_owned(), naive_datetime::display),
                    )
                },
                set: |c, v| {
                    let text = text(v)?;
                    let text = text.trim();
                    c.cache.auth_last_checked = if text.is_empty() || text == "None" {
                        None
                    } else {
                        Some(naive_datetime::parse(text).ok_or(FieldError::NotATimestamp)?)
                    };
                    Ok(())
                },
            },
            path_field!(cache.cache_directory),
            integer_field!(cache.default_ttl),
            integer_field!(cache.list_repos_ttl),
            integer_field!(cache.list_issues_ttl),
        ],
    },
    Section {
        key: "core",
        title: "Core",
        doc: None,
        fields: &[
            bool_field!(core.first_start),
            path_field!(core.logfile_path),
            integer_field!(core.logfile_max_bytes),
            integer_field!(core.logfile_count),
        ],
    },
    Section {
        key: "api",
        title: "Api",
        doc: Some("Controlling how the GitHub API is accessed in LazyGithub"),
        fields: &[
            Field {
                section: "api",
                name: "base_url",
                kind: FieldKind::Text(TextKind::Plain),
                get: |c| FieldValue::Text(c.api.base_url.clone()),
                // Stored as typed, untrimmed, as Python stores it.
                set: |c, v| {
                    c.api.base_url = text(v)?;
                    Ok(())
                },
            },
            Field {
                section: "api",
                name: "client_type",
                kind: FieldKind::ReadOnlyChoice(CLIENT_TYPE_CHOICES),
                // This app only talks to GitHub through `gh` (decision gh-only-backend), whatever the file says, and
                // leaves the stored value in `api.extra` for the Python app.
                get: |_| FieldValue::Choice(GITHUB_CLI),
                set: |_, _| Ok(()),
            },
        ],
    },
];

/// `api.client_type`'s value for the `gh` backend.
const GITHUB_CLI: &str = "GITHUB_CLI";

const STATE_CHOICES: &[Choice] = &[
    Choice {
        label: "All",
        value: "all",
    },
    Choice {
        label: "Open",
        value: "open",
    },
    Choice {
        label: "Closed",
        value: "closed",
    },
];

const OWNER_CHOICES: &[Choice] = &[
    Choice {
        label: "Mine",
        value: "mine",
    },
    Choice {
        label: "All",
        value: "all",
    },
];

const MERGE_CHOICES: &[Choice] = &[
    Choice {
        label: "Merge",
        value: "merge",
    },
    Choice {
        label: "Squash",
        value: "squash",
    },
    Choice {
        label: "Rebase",
        value: "rebase",
    },
];

/// Python's `BackendType`, in its order.
const CLIENT_TYPE_CHOICES: &[Choice] = &[
    Choice {
        label: "Raw_Http",
        value: "RAW_HTTP",
    },
    Choice {
        label: "Github_Cli",
        value: GITHUB_CLI,
    },
];

/// A config enum a [`FieldKind::Choice`] edits.
trait ChoiceEnum: Copy + 'static {
    const ALL: &'static [Self];
    fn as_str(self) -> &'static str;

    fn from_value(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == value)
    }
}

impl ChoiceEnum for IssueStateFilter {
    const ALL: &'static [Self] = &Self::ALL;
    fn as_str(self) -> &'static str {
        self.as_str()
    }
}

impl ChoiceEnum for IssueOwnerFilter {
    const ALL: &'static [Self] = &Self::ALL;
    fn as_str(self) -> &'static str {
        self.as_str()
    }
}

impl ChoiceEnum for MergeMethod {
    const ALL: &'static [Self] = &Self::ALL;
    fn as_str(self) -> &'static str {
        self.as_str()
    }
}

fn choice(value: &FieldValue) -> Result<&'static str, FieldError> {
    match *value {
        FieldValue::Choice(value) => Ok(value),
        _ => Err(FieldError::Mismatch),
    }
}

fn text(value: FieldValue) -> Result<String, FieldError> {
    match value {
        FieldValue::Text(text) => Ok(text),
        _ => Err(FieldError::Mismatch),
    }
}

/// The `'static` name of a loaded theme. Loading leaves only names in [`TEXTUAL_THEMES`]; anything else (set in code)
/// shows as the default, which is what it would load as.
fn theme_name(theme: &str) -> &'static str {
    TEXTUAL_THEMES
        .iter()
        .copied()
        .find(|name| *name == theme)
        .unwrap_or(DEFAULT_THEME)
}

/// Python's `str.title()` for ASCII text: a letter after a non-letter is upper-cased, any other letter lower-cased.
fn py_title(text: &str) -> String {
    let mut after_letter = false;
    text.chars()
        .map(|c| {
            let titled = if after_letter {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            };
            after_letter = c.is_ascii_alphabetic();
            titled
        })
        .collect()
}

#[cfg(test)]
mod tests;
