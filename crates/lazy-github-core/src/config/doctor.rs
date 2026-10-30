//! The config doctor behind `lazy-github check-config`: every value in `config.json` that stops it loading, and a
//! repaired config with the chosen ones reset to their defaults.
//!
//! It reuses the loader rather than a second set of rules. The file is loaded; when a value fails, that value is
//! recorded and replaced by its default (taken from [`Config::default_in`], serialized, so defaults live in one
//! place), and the load runs again, until it succeeds. Values the loader accepts, lax coercions and negative
//! durations included, are never problems.

use std::fmt;

use serde_json::Value;

use super::{Config, ConfigError, ConfigPaths, PathSegment, field_path, load_error, to_json_pretty, write_atomically};

/// Unknown keys the Python app writes, which Rust keeps without modeling: not worth a note.
const KNOWN_EXTRA_KEYS: &[&str] = &["api.client_type"];

/// What [`check`] found in a config file.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The values that stop the file loading, in the order the loader meets them (file order).
    pub problems: Vec<Problem>,
    /// Dotted paths of keys LazyGithub doesn't know. They load, and are kept when the config is saved.
    pub unknown_keys: Vec<String>,
}

/// A value that stops the config loading.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The dotted path of the value, as the broken-config error writes it (`issues.state_filter`). A bad entry of a
    /// list is the whole list, and a bad value of the whole file is `.`.
    pub field: String,
    /// The value in the file, or `null` if it has none there.
    pub found: Value,
    /// The deserializer's message, such as ``unknown variant `bogus`, expected one of `all`, `open`, `closed` ``.
    pub message: String,
    /// What repairing it does.
    pub fix: Fix,
}

/// How a [`Problem`] is repaired.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// The value is replaced by its default.
    Reset(Value),
    /// The entry is removed: a keybinding override, which has no default of its own.
    Remove,
}

impl Problem {
    /// What the loader allows, from the message: `one of "all", "open", "closed"` or `a theme name`. `None` when the
    /// message doesn't say.
    pub fn expected(&self) -> Option<String> {
        let (_, expected) = self.message.rsplit_once(", expected ")?;
        if expected.starts_with("struct ") {
            return Some("an object".to_owned());
        }
        Some(expected.replace('`', "\""))
    }
}

/// `<field>: found <value>, expected <what's allowed>`, the value as compact JSON.
impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: found {}", self.field, self.found)?;
        match self.expected() {
            Some(expected) => write!(f, ", expected {expected}"),
            None => write!(f, ": {}", self.message),
        }
    }
}

/// Finds every value in `text`, a `config.json`, that stops it loading, and the keys LazyGithub doesn't know.
///
/// # Errors
///
/// [`ConfigError::Syntax`] if `text` isn't valid JSON, which can't be checked field by field. [`ConfigError::Invalid`]
/// in the unexpected case that resetting a value to its default doesn't get the loader past it.
pub fn check(text: &str, paths: &ConfigPaths) -> Result<Report, ConfigError> {
    let walk = walk(text, paths, |_| false)?;
    Ok(Report {
        problems: walk.problems,
        unknown_keys: unknown_keys(&walk.config),
    })
}

/// The config in `text` with the problems named in `fields` (by [`Problem::field`], as [`check`] reports them)
/// repaired, and the rest of the file as it was.
///
/// # Errors
///
/// As [`check`].
pub fn repair(text: &str, paths: &ConfigPaths, fields: &[String]) -> Result<Repaired, ConfigError> {
    let walk = walk(text, paths, |field| fields.iter().any(|f| f == field))?;
    if walk.problems.iter().all(|problem| fields.contains(&problem.field)) {
        Ok(Repaired::Config(Box::new(walk.config)))
    } else {
        Ok(Repaired::Partial(walk.output))
    }
}

/// What [`repair`] gives.
#[derive(Debug, Clone, PartialEq)]
pub enum Repaired {
    /// Every problem was repaired: the config loads, and saves as [`Config::save`] writes it.
    Config(Box<Config>),
    /// Some problems were kept, so the file still doesn't load: its JSON with the others repaired.
    Partial(Value),
}

impl Repaired {
    /// Writes the repaired config to the config file, atomically. A [`Repaired::Partial`] is written in the same
    /// format as [`Config::save`] uses, its values untouched.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Write`] if the file or folder couldn't be written.
    pub fn save(&self, paths: &ConfigPaths) -> Result<(), ConfigError> {
        match self {
            Self::Config(config) => config.save(paths),
            Self::Partial(value) => write_atomically(paths, &to_json_pretty(value)),
        }
    }
}

/// The result of loading a file with every problem reset.
struct Walk {
    problems: Vec<Problem>,
    /// The loaded config, every problem reset.
    config: Config,
    /// The file with only the selected problems repaired.
    output: Value,
}

/// Loads `text`, resetting each value that fails until it loads. Every problem is repaired in the copy that's
/// loaded, so the walk reaches the next one; only the `selected` ones are repaired in [`Walk::output`]. The walk
/// doesn't depend on `selected`, so [`check`] and [`repair`] meet the same problems under the same names.
fn walk(text: &str, paths: &ConfigPaths, selected: impl Fn(&str) -> bool) -> Result<Walk, ConfigError> {
    let file = &paths.file;
    // The original text first, so the first error is exactly the one the loader gives.
    let mut loaded = Config::deserialize_json(text);
    let mut scratch: Value = match &loaded {
        Ok(_) => Value::Null,
        Err((field, err)) if !err.is_data() => return Err(load_error(file, field, err)),
        Err(_) => serde_json::from_str(text).map_err(|err| load_error(file, &[], &err))?,
    };
    let mut output = scratch.clone();
    // Can't fail, as `Config::to_json_pretty` explains; mapped rather than unwrapped all the same.
    let defaults = serde_json::to_value(Config::default_in(paths)).map_err(|err| load_error(file, &[], &err))?;
    let mut problems = Vec::new();
    let mut config = loop {
        let (field, err) = match loaded {
            Ok(config) => break config,
            Err(failure) => failure,
        };
        if !err.is_data() {
            return Err(load_error(file, &field, &err));
        }
        let (target, fix) = fix_for(&defaults, &field);
        let found = get(&scratch, &target).cloned().unwrap_or(Value::Null);
        // Each round must change the file, or the loader would fail on the same value forever.
        if !apply(&mut scratch, &target, &fix) {
            return Err(load_error(file, &field, &err));
        }
        let name = field_path(&target);
        if selected(&name) {
            apply(&mut output, &target, &fix);
        }
        problems.push(Problem {
            field: name,
            found,
            message: super::strip_position(&err),
            fix,
        });
        loaded = Config::deserialize_json(&scratch.to_string());
    };
    config.resolve(paths);
    Ok(Walk {
        problems,
        config,
        output,
    })
}

/// Where to repair a value that failed at `field`, and how: its default if it has one, otherwise the map entry it
/// belongs to is removed (a keybinding override) or the list it's in is reset.
fn fix_for(defaults: &Value, field: &[PathSegment]) -> (Vec<PathSegment>, Fix) {
    if let Some(default) = get(defaults, field) {
        return (field.to_vec(), Fix::Reset(default.clone()));
    }
    // The whole file always has a default, so some prefix does.
    for len in (0..field.len()).rev() {
        let Some(default) = get(defaults, &field[..len]) else {
            continue;
        };
        if default.is_object() && matches!(field[len], PathSegment::Key(_)) {
            return (field[..=len].to_vec(), Fix::Remove);
        }
        return (field[..len].to_vec(), Fix::Reset(default.clone()));
    }
    (Vec::new(), Fix::Reset(defaults.clone()))
}

/// The value at `path` in `value`.
fn get<'a>(value: &'a Value, path: &[PathSegment]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, segment| match segment {
        PathSegment::Key(key) => value.get(key),
        PathSegment::Index(index) => value.get(index),
        PathSegment::Unknown => None,
    })
}

/// The value at `path` in `value`, to change.
fn get_mut<'a>(value: &'a mut Value, path: &[PathSegment]) -> Option<&'a mut Value> {
    path.iter().try_fold(value, |value, segment| match segment {
        PathSegment::Key(key) => value.get_mut(key),
        PathSegment::Index(index) => value.get_mut(index),
        PathSegment::Unknown => None,
    })
}

/// Applies `fix` at `path`, and says whether that changed anything.
fn apply(value: &mut Value, path: &[PathSegment], fix: &Fix) -> bool {
    match fix {
        Fix::Reset(default) => match get_mut(value, path) {
            Some(slot) if slot != default => {
                *slot = default.clone();
                true
            }
            _ => false,
        },
        Fix::Remove => {
            let Some((PathSegment::Key(key), parent)) = path.split_last() else {
                return false;
            };
            get_mut(value, parent)
                .and_then(Value::as_object_mut)
                .and_then(|object| object.shift_remove(key))
                .is_some()
        }
    }
}

/// The dotted paths of the unknown keys of `config` and its sections, top level first.
fn unknown_keys(config: &Config) -> Vec<String> {
    let sections = [
        ("appearance", &config.appearance.extra),
        ("notifications", &config.notifications.extra),
        ("bindings", &config.bindings.extra),
        ("repositories", &config.repositories.extra),
        ("pull_requests", &config.pull_requests.extra),
        ("issues", &config.issues.extra),
        ("cache", &config.cache.extra),
        ("core", &config.core.extra),
        ("api", &config.api.extra),
    ];
    let top = config.extra.keys().cloned();
    let nested = sections
        .into_iter()
        .flat_map(|(section, extra)| extra.keys().map(move |key| format!("{section}.{key}")));
    top.chain(nested)
        .filter(|key| !KNOWN_EXTRA_KEYS.contains(&key.as_str()))
        .collect()
}

#[cfg(test)]
mod tests;
