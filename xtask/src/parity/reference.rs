//! Recorded Python references (`parity/references/<scenario>/<cols>x<rows>/`): the manifest, the four input hashes
//! that decide whether a reference is still what Python would draw, and the freshness check behind
//! `cargo xtask parity --check-references`. The layout and the invalidation rules are documented in
//! `parity/README.md` ("Recorded references").
//!
//! Every hash is a git object id from `git hash-object`, so file contents are hashed after `.gitattributes`
//! filters and a CRLF checkout of a `text eol=lf` file hashes like an LF one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use fake_gh::argv;
use fake_gh::fixture::{Accept, LoggedRequestMatcher, QueryValue, RequestKey};
use fake_gh::log::{LogEntry, RequestLog};
use fake_gh::output::Rendered;
use fake_gh::store::{FixtureStore, LoadedFixture};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::check::{self, Mismatch};
use super::env;
use super::grid::{Grid, Line, Style};
use super::scenario::{App, Check, Ignore, Roots, Scenario, Size, Step, WaitFor};
use super::screen;
use super::structure::{self, ThemeColors};
use super::{Context, HarnessError, Result};

/// Bump when capture semantics change outside [`HARNESS_INPUTS`], e.g. a new env var set in a file that isn't
/// listed. Every reference goes stale.
pub const CAPTURE_VERSION: u32 = 1;
/// The `format` of `manifest.toml` this harness reads and writes.
pub const MANIFEST_FORMAT: u32 = 1;
/// The Python app and its dependencies (`python_app`). Repo-relative, `/`-separated.
pub const PYTHON_APP_INPUTS: &[&str] = &["lazy_github", "pyproject.toml", "uv.lock"];
/// The capture harness (`harness`): `gh` output, the environment, the PTY, key encoding and the grid dump.
pub const HARNESS_INPUTS: &[&str] = &[
    "crates/fake-gh/src",
    "crates/fake-gh/Cargo.toml",
    "xtask/src/parity/env.rs",
    "xtask/src/parity/runner.rs",
    "xtask/src/parity/keys.rs",
    "xtask/src/parity/grid.rs",
];
/// Crates whose versions are part of `harness`, as `Cargo.lock` pins them.
pub const PINNED_CRATES: &[&str] = &["vt100", "portable-pty"];

/// Where references live, relative to the repo root.
pub const REFERENCES_DIR: &str = "parity/references";
pub const MANIFEST_FILE: &str = "manifest.toml";
pub const REQUESTS_FILE: &str = "requests.jsonl";
/// The extension of a capture's screen file (format in #161).
pub const SCREEN_EXTENSION: &str = "screen";

/// The names of the inputs, in the order a stale reference lists them.
pub const INPUT_NAMES: [&str; 4] = ["scenario", "fixtures", "python_app", "harness"];

const MANIFEST_HEADER: &str = "# Written by `cargo xtask parity --record`. Don't edit.\n";

/// `manifest.toml`: what a reference was recorded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// [`MANIFEST_FORMAT`].
    pub format: u32,
    /// `<cols>x<rows>`, the same as the reference's directory name.
    pub size: String,
    /// The scenario's captures, in step order. Each has a `<capture>.screen`.
    pub captures: Vec<String>,
    pub inputs: Inputs,
    pub recorded_with: RecordedWith,
}

/// The four input hashes. A reference is fresh when all of them equal the current ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    /// The capture key: the scenario fields that decide what Python draws, at this size.
    pub scenario: String,
    /// The fixture files that could answer a request in `requests.jsonl`.
    pub fixtures: String,
    /// [`PYTHON_APP_INPUTS`].
    pub python_app: String,
    /// [`HARNESS_INPUTS`], [`CAPTURE_VERSION`] and the [`PINNED_CRATES`] versions.
    pub harness: String,
}

impl Inputs {
    /// The names of the inputs that differ from `current`, in [`INPUT_NAMES`] order.
    pub fn changed(&self, current: &Inputs) -> Vec<&'static str> {
        let pairs = [
            (&self.scenario, &current.scenario),
            (&self.fixtures, &current.fixtures),
            (&self.python_app, &current.python_app),
            (&self.harness, &current.harness),
        ];
        INPUT_NAMES
            .into_iter()
            .zip(pairs)
            .filter_map(|(name, (a, b))| (a != b).then_some(name))
            .collect()
    }
}

/// Where a reference was recorded. Informational: never compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedWith {
    /// The Python version, e.g. `3.11.14`.
    pub python: String,
    /// `std::env::consts::OS`.
    pub os: String,
}

/// A manifest that doesn't parse, located by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    /// 1-based; 0 when the problem has no line.
    pub line: usize,
    pub message: String,
}

impl Manifest {
    /// The file's text, with the do-not-edit header.
    pub fn render(&self) -> String {
        // Plain strings and integers in fixed tables; serializing can't fail.
        let body = toml::to_string(self).expect("manifest serializes");
        format!("{MANIFEST_HEADER}{body}")
    }

    /// Parses `manifest.toml`. Checks only the TOML shape; [`MANIFEST_FORMAT`] is checked by the caller, so a
    /// newer format gets its own message.
    pub fn parse(text: &str) -> std::result::Result<Self, ManifestError> {
        toml::from_str(text).map_err(|e| ManifestError {
            line: e
                .span()
                .map_or(0, |s| text.get(..s.start).map_or(0, |t| t.matches('\n').count()) + 1),
            message: e.message().trim_end().to_owned(),
        })
    }
}

/// The state of one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    /// The inputs whose hashes changed, in [`INPUT_NAMES`] order.
    Stale(Vec<&'static str>),
    Missing,
    /// What's wrong, starting with its location: `startup/160x48/main.screen:3: ...`.
    Invalid(String),
}

/// One problem `--check-references` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Stale {
        scenario: String,
        size: Size,
        inputs: Vec<&'static str>,
    },
    Missing {
        scenario: String,
        size: Size,
    },
    Invalid {
        scenario: String,
        /// Starts with the location: `startup/160x48/main.screen:3: ...`.
        detail: String,
    },
    /// A file or directory under [`REFERENCES_DIR`] that no selected scenario, size or capture accounts for.
    Orphaned {
        /// Repo-relative, `/`-separated; directories end in `/`.
        path: String,
        /// Why nothing accounts for it, e.g. `no such scenario`.
        reason: String,
        /// The scenario whose directory holds it, when there is one.
        scenario: Option<String>,
    },
}

impl Problem {
    /// The scenario the problem belongs to, for its `--ci` annotation. `None` for a directory of a scenario that
    /// doesn't exist.
    pub fn scenario(&self) -> Option<&str> {
        match self {
            Problem::Stale { scenario, .. } | Problem::Missing { scenario, .. } | Problem::Invalid { scenario, .. } => {
                Some(scenario)
            }
            Problem::Orphaned { scenario, .. } => scenario.as_deref(),
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Problem::Missing { .. })
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Stale { scenario, size, inputs } => write!(
                f,
                "stale reference: {scenario}/{size}: {} changed; run cargo xtask parity --record {scenario}",
                inputs.join(", ")
            ),
            Problem::Missing { scenario, size } => write!(
                f,
                "missing reference: {scenario}/{size}; run cargo xtask parity --record {scenario}"
            ),
            Problem::Invalid { detail, .. } => write!(f, "invalid reference: {detail}"),
            Problem::Orphaned { path, reason, .. } => write!(
                f,
                "orphaned reference: {path} ({reason}); cargo xtask parity --record deletes it"
            ),
        }
    }
}

/// What [`check`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The references checked: one per scenario that runs Python, per size.
    pub checked: usize,
    pub problems: Vec<Problem>,
}

impl Report {
    /// `N stale, N missing, N orphaned, N invalid`.
    pub fn counts(&self) -> String {
        let count = |pred: fn(&Problem) -> bool| self.problems.iter().filter(|p| pred(p)).count();
        format!(
            "{} stale, {} missing, {} orphaned, {} invalid",
            count(|p| matches!(p, Problem::Stale { .. })),
            count(|p| matches!(p, Problem::Missing { .. })),
            count(|p| matches!(p, Problem::Orphaned { .. })),
            count(|p| matches!(p, Problem::Invalid { .. })),
        )
    }
}

/// Checks every reference of `scenarios` that run Python, at each of their sizes, and reports orphans inside their
/// directories. With `all` (every scenario is in `scenarios`), directories under [`REFERENCES_DIR`] that belong to
/// no scenario are reported too.
pub fn check(repo: &Path, scenarios: &[Scenario], roots: &Roots, all: bool) -> Result<Report> {
    let mut hasher = Hasher::new(repo);
    let references = repo.join(REFERENCES_DIR);
    let mut report = Report::default();
    for scenario in scenarios {
        let dir = references.join(&scenario.name);
        if !scenario.apps.contains(&App::Python) {
            if dir.exists() {
                report.problems.push(Problem::Orphaned {
                    path: display_dir(repo, &dir),
                    reason: format!("{} doesn't run python", scenario.name),
                    scenario: Some(scenario.name.clone()),
                });
            }
            continue;
        }
        for &size in &scenario.sizes {
            report.checked += 1;
            let problem = match freshness_with(&mut hasher, &dir, scenario, size, roots)? {
                Freshness::Fresh => continue,
                Freshness::Stale(inputs) => Problem::Stale {
                    scenario: scenario.name.clone(),
                    size,
                    inputs,
                },
                Freshness::Missing => Problem::Missing {
                    scenario: scenario.name.clone(),
                    size,
                },
                Freshness::Invalid(detail) => Problem::Invalid {
                    scenario: scenario.name.clone(),
                    detail,
                },
            };
            report.problems.push(problem);
        }
        report.problems.extend(orphans_in_scenario(repo, &dir, scenario)?);
    }
    if all {
        let names: BTreeSet<&str> = scenarios.iter().map(|s| s.name.as_str()).collect();
        for path in list_dir(&references)? {
            let known = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| names.contains(n));
            if !known {
                report.problems.push(Problem::Orphaned {
                    path: display_entry(repo, &path),
                    reason: "no such scenario".to_owned(),
                    scenario: None,
                });
            }
        }
    }
    Ok(report)
}

/// Whether the reference for `scenario` at `size` under `repo`'s [`REFERENCES_DIR`] is fresh.
pub fn freshness(repo: &Path, scenario: &Scenario, size: Size, roots: &Roots) -> Result<Freshness> {
    let dir = repo.join(REFERENCES_DIR).join(&scenario.name);
    freshness_with(&mut Hasher::new(repo), &dir, scenario, size, roots)
}

/// The four hashes for a Python run of `scenario` at `size` that made the requests in `requests`.
pub fn current_inputs(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    requests: &RequestLog,
    roots: &Roots,
) -> Result<Inputs> {
    Hasher::new(repo).inputs(scenario, size, requests, roots)
}

fn freshness_with(
    hasher: &mut Hasher<'_>,
    scenario_dir: &Path,
    scenario: &Scenario,
    size: Size,
    roots: &Roots,
) -> Result<Freshness> {
    let dir = scenario_dir.join(size.to_string());
    let at = |file: &str| format!("{}/{size}/{file}", scenario.name);
    let invalid = |detail: String| Ok(Freshness::Invalid(detail));
    if !dir.is_dir() || list_dir(&dir)?.is_empty() {
        // An empty directory can't be committed, so it's a local leftover: the reference is simply not there.
        return Ok(Freshness::Missing);
    }

    let Some(text) = read_optional(&dir.join(MANIFEST_FILE))? else {
        return invalid(format!("{}/{size}: no {MANIFEST_FILE}", scenario.name));
    };
    let manifest = match Manifest::parse(&text) {
        Ok(m) => m,
        Err(e) if e.line == 0 => return invalid(format!("{}: {}", at(MANIFEST_FILE), e.message)),
        Err(e) => return invalid(format!("{}:{}: {}", at(MANIFEST_FILE), e.line, e.message)),
    };
    if manifest.format != MANIFEST_FORMAT {
        return invalid(format!(
            "{}: format {}; this harness reads format {MANIFEST_FORMAT}",
            at(MANIFEST_FILE),
            manifest.format
        ));
    }
    if manifest.size != size.to_string() {
        return invalid(format!(
            "{}: size {:?} doesn't match its directory",
            at(MANIFEST_FILE),
            manifest.size
        ));
    }
    let captures: Vec<&str> = scenario.capture_names().collect();
    if manifest.captures != captures {
        return invalid(format!(
            "{}: captures {:?} don't match the scenario's {captures:?}",
            at(MANIFEST_FILE),
            manifest.captures
        ));
    }
    for capture in &captures {
        let file = format!("{capture}.{SCREEN_EXTENSION}");
        let Some(text) = read_optional(&dir.join(&file))? else {
            return invalid(format!("{}: missing", at(&file)));
        };
        if let Err(e) = screen::parse(&text) {
            return invalid(format!("{}:{}: {}", at(&file), e.line, e.message));
        }
    }
    let Some(text) = read_optional(&dir.join(REQUESTS_FILE))? else {
        return invalid(format!("{}: missing", at(REQUESTS_FILE)));
    };
    let requests = match parse_requests(&text) {
        Ok(r) => r,
        Err((line, message)) => return invalid(format!("{}:{line}: {message}", at(REQUESTS_FILE))),
    };

    let current = hasher.inputs(scenario, size, &requests, roots)?;
    let changed = manifest.inputs.changed(&current);
    Ok(if changed.is_empty() {
        Freshness::Fresh
    } else {
        Freshness::Stale(changed)
    })
}

/// Parses `requests.jsonl` strictly: unlike [`RequestLog::parse`], a line that doesn't parse is an error, since the
/// fixtures hash would silently skip it.
pub fn parse_requests(text: &str) -> std::result::Result<RequestLog, (usize, String)> {
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: LogEntry = serde_json::from_str(line).map_err(|e| (i + 1, e.to_string()))?;
        entries.push(entry);
    }
    Ok(RequestLog::from_entries(entries))
}

/// Entries in a scenario's reference directory that none of its sizes and captures account for.
fn orphans_in_scenario(repo: &Path, dir: &Path, scenario: &Scenario) -> Result<Vec<Problem>> {
    let mut problems = Vec::new();
    let orphan = |path: String, reason: String| Problem::Orphaned {
        path,
        reason,
        scenario: Some(scenario.name.clone()),
    };
    let sizes: BTreeSet<String> = scenario.sizes.iter().map(Size::to_string).collect();
    let captures: BTreeSet<&str> = scenario.capture_names().collect();
    for path in list_dir(dir)? {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !(path.is_dir() && sizes.contains(&name)) {
            let reason = if path.is_dir() {
                format!("{} has no size {name}", scenario.name)
            } else {
                "not part of a reference".to_owned()
            };
            problems.push(orphan(display_entry(repo, &path), reason));
            continue;
        }
        for file in list_dir(&path)? {
            let file_name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let capture = file_name
                .strip_suffix(SCREEN_EXTENSION)
                .and_then(|s| s.strip_suffix('.'));
            let reason = match capture {
                _ if file.is_dir() => "not part of a reference".to_owned(),
                Some(c) if captures.contains(c) => continue,
                Some(c) => format!("{} has no capture {c}", scenario.name),
                None if file_name == MANIFEST_FILE || file_name == REQUESTS_FILE => continue,
                None => "not part of a reference".to_owned(),
            };
            problems.push(orphan(display_entry(repo, &file), reason));
        }
    }
    Ok(problems)
}

/// The entries of `dir`, sorted, without dotfiles. Empty when `dir` doesn't exist.
fn list_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(HarnessError(format!("listing {}: {e}", dir.display()))),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.with_context(|| format!("listing {}", dir.display()))?.path();
        if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(HarnessError(format!("reading {}: {e}", path.display()))),
    }
}

/// `path` relative to `repo`, with `/` separators.
fn relative(repo: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(repo).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn display_dir(repo: &Path, path: &Path) -> String {
    format!("{}/", relative(repo, path))
}

fn display_entry(repo: &Path, path: &Path) -> String {
    if path.is_dir() {
        display_dir(repo, path)
    } else {
        relative(repo, path)
    }
}

/// One side of an agreement check: a Python run, or a stored reference.
#[derive(Debug, Clone)]
pub struct Side {
    /// How messages name it: `run 1`, `the reference`.
    pub label: String,
    /// Why the run didn't pass ([`AppRunStatus::describe`](super::runner::AppRunStatus::describe)), or `None`.
    pub failure: Option<String>,
    /// The captures it produced, in step order.
    pub captures: Vec<(String, Grid)>,
    /// Its request log as `requests.jsonl` holds it ([`screen::normalize_log`]).
    pub requests: String,
}

impl Side {
    /// The grid of capture `name`, if the side has it.
    pub fn capture(&self, name: &str) -> Option<&Grid> {
        self.captures.iter().find(|(n, _)| n == name).map(|(_, g)| g)
    }

    fn capture_names(&self) -> Vec<String> {
        self.captures.iter().map(|(n, _)| n.clone()).collect()
    }
}

/// Why two sides don't agree, or why one of them can't be a reference at all.
#[derive(Debug, Clone, PartialEq)]
pub enum Disagreement {
    /// The run didn't pass: a timeout, an early exit, fixture problems.
    Failed { run: String, detail: String },
    /// The run fails the scenario's `[expect.<capture>]`.
    Expect {
        run: String,
        capture: String,
        mismatch: Mismatch,
    },
    /// The two sides produced different captures.
    Captures {
        a: String,
        b: String,
        a_names: Vec<String>,
        b_names: Vec<String>,
    },
    /// A check the capture asks for differs. In the mismatch, `python` is side `a` and `rust` is side `b`.
    Check {
        a: String,
        b: String,
        capture: String,
        mismatch: Mismatch,
    },
    /// The sets of served request keys differ.
    Requests {
        a: String,
        b: String,
        only_a: Vec<String>,
        only_b: Vec<String>,
    },
}

impl Disagreement {
    /// The capture it's about, when there is one.
    pub fn capture(&self) -> Option<&str> {
        match self {
            Disagreement::Expect { capture, .. } | Disagreement::Check { capture, .. } => Some(capture),
            _ => None,
        }
    }
}

impl fmt::Display for Disagreement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Disagreement::Failed { run, detail } => write!(f, "{run} {detail}"),
            Disagreement::Expect { run, capture, mismatch } => {
                write!(f, "{run} [expect] {capture}: {}", mismatch.describe())
            }
            Disagreement::Captures { a, b, a_names, b_names } => {
                write!(f, "{b} differs from {a}: captures {b_names:?}, not {a_names:?}")
            }
            Disagreement::Check {
                a,
                b,
                capture,
                mismatch,
            } => {
                write!(f, "{b} differs from {a}: {capture}: {}", mismatch.describe())
            }
            Disagreement::Requests { a, b, only_a, only_b } => {
                write!(f, "{b} differs from {a}: served requests differ")?;
                if !only_a.is_empty() {
                    write!(f, "; only in {a}: {}", only_a.join(", "))?;
                }
                if !only_b.is_empty() {
                    write!(f, "; only in {b}: {}", only_b.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

/// Why `side` can't be a reference on its own: it didn't pass, or it fails the scenario's `[expect]`. Empty when it
/// can.
pub fn verify(scenario: &Scenario, theme: &ThemeColors, side: &Side) -> Vec<Disagreement> {
    if let Some(detail) = &side.failure {
        return vec![Disagreement::Failed {
            run: side.label.clone(),
            detail: detail.clone(),
        }];
    }
    let mut out = Vec::new();
    for (name, grid) in &side.captures {
        let Some(expect) = scenario.expect.get(name) else {
            continue;
        };
        let found = check::check_expect(App::Python, &structure::extract(grid, theme), expect);
        out.extend(found.into_iter().map(|mismatch| Disagreement::Expect {
            run: side.label.clone(),
            capture: name.clone(),
            mismatch,
        }));
    }
    out
}

/// Whether `a` and `b` agree: both pass [`verify`], they produced the same captures, every capture's `checks` find
/// nothing between them (outside its `ignore`d boxes, as in the cross-app check), and they served the same request
/// keys. Recording uses it between two Python runs, or a run and the old reference; the drift check (#164) between
/// a live run and a reference. Empty when they agree; the first entry is the one to show.
pub fn agree(scenario: &Scenario, theme: &ThemeColors, a: &Side, b: &Side) -> Vec<Disagreement> {
    let mut out = verify(scenario, theme, a);
    out.extend(verify(scenario, theme, b));
    if a.failure.is_some() || b.failure.is_some() {
        return out;
    }
    let (a_names, b_names) = (a.capture_names(), b.capture_names());
    if a_names != b_names {
        out.push(Disagreement::Captures {
            a: a.label.clone(),
            b: b.label.clone(),
            a_names,
            b_names,
        });
    }
    for step in &scenario.steps {
        let Step::Capture(capture) = step else { continue };
        let (Some(ga), Some(gb)) = (a.capture(&capture.name), b.capture(&capture.name)) else {
            continue;
        };
        let (sa, sb) = (structure::extract(ga, theme), structure::extract(gb, theme));
        out.extend(
            check::compare(&sa, &sb, &capture.checks, &capture.ignore)
                .into_iter()
                .map(|mismatch| Disagreement::Check {
                    a: a.label.clone(),
                    b: b.label.clone(),
                    capture: capture.name.clone(),
                    mismatch,
                }),
        );
    }
    let (keys_a, keys_b) = (served_keys(&a.requests), served_keys(&b.requests));
    if keys_a != keys_b {
        out.push(Disagreement::Requests {
            a: a.label.clone(),
            b: b.label.clone(),
            only_a: keys_a.difference(&keys_b).cloned().collect(),
            only_b: keys_b.difference(&keys_a).cloned().collect(),
        });
    }
    out
}

/// The served requests in a request log, each as `GET /path?k=v (json)`, or as its command (`auth status`) when it
/// isn't an API request.
pub fn served_keys(requests: &str) -> BTreeSet<String> {
    RequestLog::parse(requests)
        .entries()
        .iter()
        .filter(|e| e.is_served())
        .map(|e| match logged_key(e) {
            Some(key) => {
                let query: Vec<String> = key
                    .query
                    .iter()
                    .flat_map(|(k, v)| match v {
                        QueryValue::One(v) => vec![format!("{k}={v}")],
                        QueryValue::Many(vs) => vs.iter().map(|v| format!("{k}={v}")).collect(),
                    })
                    .collect();
                let query = if query.is_empty() {
                    String::new()
                } else {
                    format!("?{}", query.join("&"))
                };
                format!("{} {}{query} ({})", key.method, key.path, key.accept)
            }
            None => e.command.clone().unwrap_or_else(|| e.argv.join(" ")),
        })
        .collect()
}

/// Whether two grids are equal cell for cell (text and style) outside the interiors of boxes whose title contains
/// one of `ignore.boxes` in both grids. Recording keeps an old `.screen` when this holds, so a re-record that only
/// moves a Command Log timestamp leaves the file alone. The cursor isn't compared: no check reads it.
pub fn identical_outside_ignored(a: &Grid, b: &Grid, ignore: &Ignore, theme: &ThemeColors) -> bool {
    if (a.cols, a.rows) != (b.cols, b.rows) || a.lines.len() != b.lines.len() {
        return false;
    }
    let interiors = |grid: &Grid| -> Vec<[usize; 4]> {
        if ignore.boxes.is_empty() {
            return Vec::new();
        }
        structure::extract(grid, theme)
            .boxes
            .iter()
            .filter(|p| ignore.boxes.iter().any(|i| p.title.contains(i.as_str())))
            .map(|p| p.rect.map(usize::from))
            .collect()
    };
    let (ia, ib) = (interiors(a), interiors(b));
    // `[col, row, width, height]` of the border; the interior is everything strictly inside it.
    let inside = |rects: &[[usize; 4]], row: usize, col: usize| {
        rects
            .iter()
            .any(|&[c, r, w, h]| row > r && row + 1 < r + h && col > c && col + 1 < c + w)
    };
    a.lines.iter().zip(&b.lines).enumerate().all(|(row, (la, lb))| {
        if la == lb {
            return true;
        }
        let (sa, sb) = (cell_styles(la), cell_styles(lb));
        if la.cells.len() != lb.cells.len() || sa.len() != sb.len() {
            return false;
        }
        (0..la.cells.len()).all(|col| {
            (la.cells[col] == lb.cells[col] && sa[col] == sb[col]) || (inside(&ia, row, col) && inside(&ib, row, col))
        })
    })
}

/// Each cell's style in a row.
fn cell_styles(line: &Line) -> Vec<&Style> {
    line.runs
        .iter()
        .flat_map(|run| std::iter::repeat_n(&run.style, usize::from(run.len)))
        .collect()
}

/// Computes hashes in one repo, remembering the two that don't depend on the scenario.
struct Hasher<'a> {
    repo: &'a Path,
    python_app: Option<String>,
    harness: Option<String>,
}

impl<'a> Hasher<'a> {
    fn new(repo: &'a Path) -> Self {
        Self {
            repo,
            python_app: None,
            harness: None,
        }
    }

    fn inputs(&mut self, scenario: &Scenario, size: Size, requests: &RequestLog, roots: &Roots) -> Result<Inputs> {
        let python_app = match &self.python_app {
            Some(h) => h.clone(),
            None => self.python_app.insert(python_app_hash(self.repo)?).clone(),
        };
        let harness = match &self.harness {
            Some(h) => h.clone(),
            None => self.harness.insert(harness_hash(self.repo)?).clone(),
        };
        Ok(Inputs {
            scenario: scenario_hash(self.repo, scenario, size, roots)?,
            fixtures: fixtures_hash(self.repo, scenario, requests, roots)?,
            python_app,
            harness,
        })
    }
}

/// The `python_app` hash: [`PYTHON_APP_INPUTS`].
pub fn python_app_hash(repo: &Path) -> Result<String> {
    file_set_hash(repo, PYTHON_APP_INPUTS, "")
}

/// The `harness` hash: [`HARNESS_INPUTS`], [`CAPTURE_VERSION`] and the [`PINNED_CRATES`] versions in `Cargo.lock`.
pub fn harness_hash(repo: &Path) -> Result<String> {
    let mut extra = format!("capture-version {CAPTURE_VERSION}\n");
    for (name, version) in pinned_versions(repo)? {
        // Writing to a `String` can't fail.
        let _ = writeln!(extra, "crate {name} {version}");
    }
    file_set_hash(repo, HARNESS_INPUTS, &extra)
}

/// `name version` for each of [`PINNED_CRATES`] in `repo`'s `Cargo.lock`, sorted. A crate locked at two versions
/// appears twice.
fn pinned_versions(repo: &Path) -> Result<Vec<(String, String)>> {
    #[derive(Deserialize)]
    struct Lock {
        #[serde(default)]
        package: Vec<Package>,
    }
    #[derive(Deserialize)]
    struct Package {
        name: String,
        version: String,
    }
    let path = repo.join("Cargo.lock");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let lock: Lock = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut versions = Vec::new();
    for name in PINNED_CRATES {
        let before = versions.len();
        versions.extend(
            lock.package
                .iter()
                .filter(|p| p.name == *name)
                .map(|p| (p.name.clone(), p.version.clone())),
        );
        if versions.len() == before {
            return Err(HarnessError(format!("{} has no package {name}", path.display())));
        }
    }
    versions.sort();
    Ok(versions)
}

/// Hashes the files under `paths` (repo-relative, `/`-separated): tracked plus untracked-but-not-ignored, so local
/// edits count and `__pycache__` doesn't. Listed paths that don't exist are skipped. `extra` is appended to the
/// sorted `path blob` lines before the final hash.
pub fn file_set_hash(repo: &Path, paths: &[&str], extra: &str) -> Result<String> {
    let existing: Vec<&str> = paths.iter().copied().filter(|p| repo.join(p).exists()).collect();
    let mut files = Vec::new();
    // With no pathspec at all, ls-files would list the whole repo.
    if !existing.is_empty() {
        let mut args = vec!["ls-files", "-co", "--exclude-standard", "-z", "--"];
        args.extend(&existing);
        let out = git(repo, &args, None)?;
        for raw in out.split(|&b| b == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8(raw.to_vec())
                .map_err(|_| HarnessError(format!("a path under {existing:?} isn't valid UTF-8")))?;
            // `-c` still lists a tracked file deleted from the working tree, which can't be hashed.
            if repo.join(&path).is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    let blobs = blob_ids(repo, &files)?;
    let mut text = blob_lines(&files, &blobs);
    text.push_str(extra);
    hash_bytes(repo, text.as_bytes())
}

/// The serialized capture key. Field order is fixed, so equal keys serialize to equal text.
#[derive(Serialize)]
struct CaptureKey<'a> {
    config: Value,
    fixtures: &'a [String],
    gh_auth: &'static str,
    git_remote: Option<&'a str>,
    git_branch: &'a str,
    key_delay_ms: u64,
    capture_stable_ms: u64,
    size: String,
    steps: Vec<KeyStep<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum KeyStep<'a> {
    /// Without `timeout_ms`: how long a wait may take doesn't change what's captured after it.
    Wait(KeyWait<'a>),
    Keys(&'a [String]),
    Type(&'a str),
    Paste(&'a str),
    Capture {
        name: &'a str,
        checks: &'a [Check],
        ignore: &'a Ignore,
    },
    SleepMs(u64),
    Resize(String),
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum KeyWait<'a> {
    Text(&'a str),
    Gone(&'a str),
    Regex(&'a str),
    StableMs(u64),
    Log(&'a LoggedRequestMatcher),
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The capture key as JSON: the scenario's resolved config, env, timing, `size` and steps. It leaves out
/// `description`, `gate`, `parity_rows`, `[expect]`, timeouts and `apps`, which don't change what Python draws.
pub fn capture_key(scenario: &Scenario, size: Size, roots: &Roots) -> Result<String> {
    let steps = scenario
        .steps
        .iter()
        .map(|step| match step {
            Step::Wait(w) => KeyStep::Wait(match &w.until {
                WaitFor::Text(t) => KeyWait::Text(t),
                WaitFor::Gone(t) => KeyWait::Gone(t),
                WaitFor::Regex(re) => KeyWait::Regex(re.as_str()),
                WaitFor::Stable(d) => KeyWait::StableMs(millis(*d)),
                WaitFor::Log(m) => KeyWait::Log(m),
            }),
            Step::Keys(keys) => KeyStep::Keys(keys),
            Step::Type(t) => KeyStep::Type(t),
            Step::Paste(t) => KeyStep::Paste(t),
            Step::Capture(c) => KeyStep::Capture {
                name: &c.name,
                checks: &c.checks,
                ignore: &c.ignore,
            },
            Step::Sleep(d) => KeyStep::SleepMs(millis(*d)),
            Step::Resize(s) => KeyStep::Resize(s.to_string()),
        })
        .collect();
    let key = CaptureKey {
        config: env::scenario_config(scenario, roots)?,
        fixtures: &scenario.env.fixtures,
        gh_auth: scenario.env.gh_auth.env_value(),
        git_remote: scenario.env.git_remote.as_deref(),
        git_branch: &scenario.env.git_branch,
        key_delay_ms: millis(scenario.timing.key_delay),
        capture_stable_ms: millis(scenario.timing.capture_stable),
        size: size.to_string(),
        steps,
    };
    serde_json::to_string(&key).context("serializing the capture key")
}

/// The `scenario` hash: the [`capture_key`].
pub fn scenario_hash(repo: &Path, scenario: &Scenario, size: Size, roots: &Roots) -> Result<String> {
    hash_bytes(repo, capture_key(scenario, size, roots)?.as_bytes())
}

/// The `fixtures` hash: every fixture in the scenario's sets whose request key equals a served request in
/// `requests`, plus its `body_file`. A run with no misses got every response from these files, so while they're
/// unchanged it makes the same requests and gets the same answers. A `--paginate` request also takes the fixtures
/// its `Link: rel="next"` pages lead to, since the log names only the first page.
pub fn fixtures_hash(repo: &Path, scenario: &Scenario, requests: &RequestLog, roots: &Roots) -> Result<String> {
    let sets: Vec<PathBuf> = scenario.env.fixtures.iter().map(|s| roots.fixtures.join(s)).collect();
    let store = FixtureStore::load(&sets).context("loading the scenario's fixture sets")?;

    let mut queue: Vec<(RequestKey, bool)> = requests
        .entries()
        .iter()
        .filter(|e| e.is_served())
        .filter_map(|e| Some((logged_key(e)?, paginates(e))))
        .collect();
    let mut seen = BTreeSet::new();
    // Display path (`set/relpath`) to the file on disk.
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    while let Some((key, paginate)) = queue.pop() {
        // Plain strings and maps; serializing can't fail.
        let id = serde_json::to_string(&key).expect("request key serializes");
        if !seen.insert((id, paginate)) {
            continue;
        }
        for fixture in store.fixtures().iter().filter(|f| f.fixture.request.key() == key) {
            files.insert(fixture.display.clone(), fixture.file.clone());
            if let Some(body) = fixture.body_file_path() {
                files.insert(body_display(fixture, &body), body);
            }
            if paginate && let Some(next) = next_page(fixture, &key) {
                queue.push((next, true));
            }
        }
    }

    let paths: Vec<String> = files.values().map(|p| relative(repo, p)).collect();
    let blobs = blob_ids(repo, &paths)?;
    let names: Vec<&String> = files.keys().collect();
    hash_bytes(repo, blob_lines(&names, &blobs).as_bytes())
}

/// `<name> <blob>\n` for each pair, in order.
fn blob_lines(names: &[impl fmt::Display], blobs: &[String]) -> String {
    let mut text = String::new();
    for (name, blob) in names.iter().zip(blobs) {
        // Writing to a `String` can't fail.
        let _ = writeln!(text, "{name} {blob}");
    }
    text
}

/// The fixture key a log entry was served for. Logged paths and queries are already normalized.
fn logged_key(entry: &LogEntry) -> Option<RequestKey> {
    Some(RequestKey {
        method: entry.method.clone()?,
        path: entry.path.clone()?,
        query: entry.query.clone().unwrap_or_default(),
        accept: entry.accept.as_deref().map(Accept::normalize).unwrap_or_default(),
    })
}

/// Whether the logged invocation followed `Link` headers. An argv that doesn't parse counts as paginating, which
/// can only widen the hash.
fn paginates(entry: &LogEntry) -> bool {
    match argv::parse(&entry.argv) {
        Ok(argv::Command::Api(req)) => req.paginate,
        Ok(_) => false,
        Err(_) => true,
    }
}

/// The key fake-gh would look up after serving `fixture` with `--paginate`.
fn next_page(fixture: &LoadedFixture, key: &RequestKey) -> Option<RequestKey> {
    // Only the headers matter here, so the body file isn't read.
    let rendered = Rendered::from_response(&fixture.fixture.response, None).ok()?;
    if rendered.status >= 400 {
        return None;
    }
    let url = rendered.next_link()?;
    RequestKey::from_url(&key.method, &url, &[], key.accept.clone()).ok()
}

/// `body` as `set/relpath`, like the fixture's own display path.
fn body_display(fixture: &LoadedFixture, body: &Path) -> String {
    let base = fixture.set_dir.parent().unwrap_or(Path::new(""));
    relative(base, body)
}

/// The blob id of each file, in order, as `git hash-object` computes it from the working tree.
fn blob_ids(repo: &Path, paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(bad) = paths.iter().find(|p| p.contains('\n')) {
        return Err(HarnessError(format!("can't hash {bad:?}: the path holds a newline")));
    }
    let input = paths.join("\n") + "\n";
    let out = git(repo, &["hash-object", "--stdin-paths"], Some(input.as_bytes()))?;
    let out = String::from_utf8_lossy(&out);
    let ids: Vec<String> = out.lines().map(str::to_owned).collect();
    if ids.len() != paths.len() {
        return Err(HarnessError(format!(
            "git hash-object returned {} ids for {} paths",
            ids.len(),
            paths.len()
        )));
    }
    Ok(ids)
}

/// `git hash-object --stdin`, which hashes the bytes as they are (no filters).
fn hash_bytes(repo: &Path, bytes: &[u8]) -> Result<String> {
    let out = git(repo, &["hash-object", "--stdin"], Some(bytes))?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// Runs git in `repo` with a cleared environment, so neither the developer's config nor a `GIT_DIR` from a hook
/// changes what it lists or how it hashes.
fn git(repo: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let shown = || format!("git {}", args.join(" "));
    let mut command = Command::new("git");
    command.args(args).current_dir(repo).env_clear();
    for var in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(var) {
            command.env(var, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().with_context(|| format!("running {}", shown()))?;
    let output = std::thread::scope(|scope| {
        // Written from another thread: git may fill its stdout pipe before it has read all of stdin.
        let writer = child.stdin.take().map(|mut pipe| {
            let bytes = stdin.unwrap_or_default();
            scope.spawn(move || pipe.write_all(bytes))
        });
        let output = child.wait_with_output();
        let written = writer.map_or(Ok(Ok(())), std::thread::ScopedJoinHandle::join);
        (output, written)
    });
    let (output, written) = output;
    let output = output.with_context(|| format!("running {}", shown()))?;
    if !output.status.success() {
        return Err(HarnessError(format!(
            "{} in {} failed: {}",
            shown(),
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    match written {
        Ok(Ok(())) => Ok(output.stdout),
        Ok(Err(e)) => Err(HarnessError(format!("writing to {}: {e}", shown()))),
        Err(_) => Err(HarnessError(format!("the thread writing to {} panicked", shown()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp git repo with no commits.
    struct Repo(tempfile::TempDir);

    impl Repo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            git(dir.path(), &["init", "-q"], None).unwrap();
            Self(dir)
        }

        /// A repo with every hashed input, one fixture set and a scenario `demo` at 80x24.
        fn harness() -> Self {
            let repo = Self::new();
            repo.write("lazy_github/__init__.py", "print('hi')\n");
            repo.write("pyproject.toml", "[project]\nversion = \"0.6.6\"\n");
            repo.write("uv.lock", "version = 1\n");
            repo.write("crates/fake-gh/src/lib.rs", "// fake-gh\n");
            repo.write("crates/fake-gh/Cargo.toml", "[package]\n");
            repo.write("xtask/src/parity/runner.rs", "// runner\n");
            repo.write(
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"vt100\"\nversion = \"0.18.1\"\n\n\
                 [[package]]\nname = \"portable-pty\"\nversion = \"0.9.0\"\n",
            );
            repo.write("parity/configs/default.json", "{}\n");
            repo.write(
                "parity/fixtures/base/user/GET.json",
                &fixture("GET", "/user", "", "\"login\":\"a\""),
            );
            repo.write("parity/scenarios/demo.toml", DEMO);
            repo
        }

        fn path(&self) -> &Path {
            self.0.path()
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn remove(&self, rel: &str) {
            let path = self.path().join(rel);
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                std::fs::remove_file(path).unwrap();
            }
        }

        fn commit(&self) {
            git(self.path(), &["add", "-A"], None).unwrap();
            let identity = [
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ];
            git(self.path(), &[&identity[..], &["commit", "-qm", "x"]].concat(), None).unwrap();
        }

        fn roots(&self) -> Roots {
            Roots {
                fixtures: self.path().join("parity/fixtures"),
                configs: self.path().join("parity/configs"),
            }
        }

        fn scenario(&self, name: &str) -> Scenario {
            Scenario::load(
                &self.path().join(format!("parity/scenarios/{name}.toml")),
                &self.roots(),
            )
            .unwrap()
        }

        fn files_hash(&self, paths: &[&str]) -> String {
            file_set_hash(self.path(), paths, "").unwrap()
        }
    }

    const DEMO: &str = "version = 1\nsizes = [[80, 24]]\n\n[[step]]\nwait = { text = \"Hi\" }\n\n[[step]]\ncapture = \
                        \"main\"\n";
    const SIZE: Size = Size { cols: 80, rows: 24 };

    /// A fixture file. `extra_request` is appended to the request object; `body` is the response body's members.
    fn fixture(method: &str, path: &str, extra_request: &str, body: &str) -> String {
        format!(
            r#"{{"version":1,"request":{{"method":"{method}","path":"{path}"{extra_request}}},"response":{{"status":200,"body":{{{body}}}}}}}"#
        )
    }

    /// A served `gh api <url>` log line.
    fn served(path: &str, fixture: &str) -> LogEntry {
        LogEntry {
            argv: vec!["api".into(), path.into()],
            command: Some("api".into()),
            method: Some("GET".into()),
            path: Some(path.into()),
            query: Some(fake_gh::fixture::Query::new()),
            accept: Some("json".into()),
            fixture: Some(fixture.into()),
            status: Some(200),
            ..LogEntry::default()
        }
    }

    fn jsonl(entries: &[LogEntry]) -> String {
        entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap() + "\n")
            .collect()
    }

    /// Writes a fresh reference for `scenario` at `size` that served `entries`.
    fn record(repo: &Repo, scenario: &Scenario, size: Size, entries: &[LogEntry]) {
        let dir = format!("{REFERENCES_DIR}/{}/{size}", scenario.name);
        let log = RequestLog::from_entries(entries.to_vec());
        let manifest = Manifest {
            format: MANIFEST_FORMAT,
            size: size.to_string(),
            captures: scenario.capture_names().map(str::to_owned).collect(),
            inputs: current_inputs(repo.path(), scenario, size, &log, &repo.roots()).unwrap(),
            recorded_with: RecordedWith {
                python: "3.11.14".into(),
                os: "linux".into(),
            },
        };
        repo.write(&format!("{dir}/{MANIFEST_FILE}"), &manifest.render());
        repo.write(&format!("{dir}/{REQUESTS_FILE}"), &jsonl(entries));
        for capture in scenario.capture_names() {
            repo.write(&format!("{dir}/{capture}.screen"), &blank_screen(size));
        }
    }

    /// A valid `.screen` of an empty screen.
    fn blank_screen(size: Size) -> String {
        let line = Line {
            cells: vec![" ".to_owned(); usize::from(size.cols)],
            runs: vec![super::super::grid::Run {
                col: 0,
                len: size.cols,
                style: Style::default(),
            }],
        };
        screen::render(&Grid {
            version: super::super::grid::GRID_VERSION,
            cols: size.cols,
            rows: size.rows,
            cursor: super::super::grid::Cursor {
                row: 0,
                col: 0,
                visible: false,
            },
            lines: vec![line; usize::from(size.rows)],
        })
        .unwrap()
    }

    fn user_log() -> Vec<LogEntry> {
        vec![served("/user", "base/user/GET.json")]
    }

    fn fresh_demo() -> (Repo, Scenario) {
        let repo = Repo::harness();
        let demo = repo.scenario("demo");
        record(&repo, &demo, SIZE, &user_log());
        (repo, demo)
    }

    fn state(repo: &Repo, scenario: &Scenario) -> Freshness {
        freshness(repo.path(), scenario, SIZE, &repo.roots()).unwrap()
    }

    fn problems(repo: &Repo, scenarios: &[Scenario], all: bool) -> Vec<String> {
        check(repo.path(), scenarios, &repo.roots(), all)
            .unwrap()
            .problems
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    // ---- the capture key ----

    const KEYED: &str = r#"version = 1
description = "d"
gate = false
apps = ["python", "rust"]
parity_rows = ["a.b"]
timeout_ms = 20000

[env]
fixtures = ["base", "other"]

[[step]]
wait = { text = "Hi", timeout_ms = 1000 }

[[step]]
keys = ["j"]

[[step]]
capture = "main"
checks = ["titles"]
ignore = { boxes = ["Log"] }

[expect.main]
contains = ["Hi"]
"#;

    fn key_roots() -> (tempfile::TempDir, Roots) {
        let dir = tempfile::tempdir().unwrap();
        for set in ["base", "other"] {
            std::fs::create_dir_all(dir.path().join("fixtures").join(set)).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("configs")).unwrap();
        std::fs::write(dir.path().join("configs/default.json"), "{\"a\": 1}").unwrap();
        let roots = Roots {
            fixtures: dir.path().join("fixtures"),
            configs: dir.path().join("configs"),
        };
        (dir, roots)
    }

    fn key_of(source: &str, size: Size, roots: &Roots) -> String {
        let scenario = Scenario::parse(Path::new("parity/scenarios/keyed.toml"), source, roots)
            .unwrap_or_else(|e| panic!("{e}\n{source}"));
        capture_key(&scenario, size, roots).unwrap()
    }

    /// `KEYED` with `from` replaced by `to`; `from` must occur in it.
    fn variant(from: &str, to: &str) -> String {
        assert!(KEYED.contains(from), "{from:?}");
        KEYED.replacen(from, to, 1)
    }

    #[test]
    fn the_capture_key_leaves_out_what_does_not_change_the_capture() {
        let (_dir, roots) = key_roots();
        let base = key_of(KEYED, SIZE, &roots);
        for (from, to) in [
            ("gate = false", "gate = true"),
            ("parity_rows = [\"a.b\"]", "parity_rows = [\"c.d\", \"e.f\"]"),
            ("description = \"d\"", "description = \"something else\""),
            ("contains = [\"Hi\"]", "contains = [\"Bye\"]\nfocused = [\"x\"]"),
            ("[expect.main]\ncontains = [\"Hi\"]\n", ""),
            ("timeout_ms = 1000", "timeout_ms = 9000"),
            ("timeout_ms = 20000", "timeout_ms = 5"),
            ("apps = [\"python\", \"rust\"]", "apps = [\"python\"]"),
        ] {
            assert_eq!(key_of(&variant(from, to), SIZE, &roots), base, "{from} -> {to}");
        }
    }

    #[test]
    fn the_capture_key_changes_with_everything_that_changes_the_capture() {
        let (dir, roots) = key_roots();
        let base = key_of(KEYED, SIZE, &roots);
        let no_expect = |s: String| s.replace("[expect.main]\ncontains = [\"Hi\"]\n", "");
        for (from, to) in [
            ("keys = [\"j\"]", "keys = [\"k\"]"),
            ("text = \"Hi\"", "gone = \"Hi\""),
            ("text = \"Hi\"", "text = \"Ho\""),
            ("capture = \"main\"", "capture = \"other\""),
            ("checks = [\"titles\"]", "checks = [\"titles\", \"tabs\"]"),
            ("ignore = { boxes = [\"Log\"] }", "ignore = { boxes = [\"Details\"] }"),
            (
                "[env]\n",
                "[env]\nconfig_overrides = { appearance = { theme = \"x\" } }\n",
            ),
            ("fixtures = [\"base\", \"other\"]", "fixtures = [\"other\", \"base\"]"),
            ("[env]\n", "[env]\ngh_auth = \"logged-out\"\n"),
            ("[env]\n", "[env]\ngit_remote = \"o/r\"\n"),
            ("[env]\n", "[env]\ngit_branch = \"dev\"\n"),
            ("timeout_ms = 20000", "timeout_ms = 20000\nkey_delay_ms = 99"),
            ("timeout_ms = 20000", "timeout_ms = 20000\ncapture_stable_ms = 99"),
        ] {
            // `[expect]` names the capture, so it goes too (which alone leaves the key unchanged).
            let source = no_expect(variant(from, to));
            assert_ne!(key_of(&source, SIZE, &roots), base, "{from} -> {to}");
        }
        assert_ne!(key_of(KEYED, Size { cols: 100, rows: 30 }, &roots), base, "size");

        std::fs::write(dir.path().join("configs/default.json"), "{\"a\": 2}").unwrap();
        assert_ne!(key_of(KEYED, SIZE, &roots), base, "config file contents");
    }

    #[test]
    fn the_capture_key_holds_the_resolved_config_and_steps() {
        let (_dir, roots) = key_roots();
        let key: Value = serde_json::from_str(&key_of(KEYED, SIZE, &roots)).unwrap();
        assert_eq!(
            key,
            serde_json::json!({
                "config": {"a": 1, "core": {"first_start": false}, "api": {"client_type": "GITHUB_CLI"}},
                "fixtures": ["base", "other"],
                "gh_auth": "logged-in",
                "git_remote": null,
                "git_branch": "main",
                "key_delay_ms": 30,
                "capture_stable_ms": 300,
                "size": "80x24",
                "steps": [
                    {"wait": {"text": "Hi"}},
                    {"keys": ["j"]},
                    {"capture": {"name": "main", "checks": ["titles"], "ignore": {"boxes": ["Log"]}}},
                ],
            })
        );
    }

    // ---- file-set hashes ----

    #[test]
    fn an_uncommitted_edit_changes_a_file_set_hash() {
        let repo = Repo::new();
        repo.write("lazy_github/app.py", "a = 1\n");
        repo.commit();
        let before = repo.files_hash(&["lazy_github"]);
        assert_eq!(repo.files_hash(&["lazy_github"]), before, "deterministic");
        repo.write("lazy_github/app.py", "a = 2\n");
        assert_ne!(repo.files_hash(&["lazy_github"]), before);
    }

    #[test]
    fn ignored_files_dont_count_but_new_untracked_ones_do() {
        let repo = Repo::new();
        repo.write(".gitignore", "__pycache__/\n");
        repo.write("lazy_github/app.py", "a = 1\n");
        repo.commit();
        let before = repo.files_hash(&["lazy_github"]);
        repo.write("lazy_github/__pycache__/x.pyc", "bytecode");
        assert_eq!(repo.files_hash(&["lazy_github"]), before, "ignored");
        repo.write("lazy_github/new.py", "b = 1\n");
        assert_ne!(repo.files_hash(&["lazy_github"]), before, "untracked");
    }

    #[test]
    fn a_crlf_checkout_of_an_lf_file_hashes_the_same() {
        let repo = Repo::new();
        repo.write(".gitattributes", "*.py text eol=lf\n");
        repo.write("lazy_github/app.py", "a = 1\nb = 2\n");
        repo.commit();
        let lf = repo.files_hash(&["lazy_github"]);
        repo.write("lazy_github/app.py", "a = 1\r\nb = 2\r\n");
        assert_eq!(repo.files_hash(&["lazy_github"]), lf);
        // The control: a file the attribute doesn't cover is hashed byte for byte.
        repo.write("lazy_github/notes.txt", "a\n");
        let before = repo.files_hash(&["lazy_github"]);
        repo.write("lazy_github/notes.txt", "a\r\n");
        assert_ne!(repo.files_hash(&["lazy_github"]), before);
    }

    #[test]
    fn listed_paths_that_dont_exist_are_skipped() {
        let repo = Repo::new();
        repo.write("lazy_github/app.py", "a = 1\n");
        repo.write("elsewhere.txt", "x\n");
        repo.commit();
        let only = repo.files_hash(&["lazy_github"]);
        assert_eq!(repo.files_hash(&["lazy_github", "uv.lock"]), only);
        // Not the whole repo: with nothing listed, nothing is hashed.
        assert_eq!(repo.files_hash(&["nope"]), repo.files_hash(&[]));
        assert_ne!(repo.files_hash(&["nope"]), only);
        // A tracked file deleted from the working tree is gone, not an error.
        repo.remove("lazy_github/app.py");
        assert_eq!(repo.files_hash(&["lazy_github"]), repo.files_hash(&[]));
    }

    #[test]
    fn the_harness_hash_includes_the_pinned_crate_versions() {
        let repo = Repo::harness();
        let before = harness_hash(repo.path()).unwrap();
        repo.write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"vt100\"\nversion = \"0.18.2\"\n\n\
             [[package]]\nname = \"portable-pty\"\nversion = \"0.9.0\"\n",
        );
        assert_ne!(harness_hash(repo.path()).unwrap(), before);
        repo.write("Cargo.lock", "version = 4\n");
        let err = harness_hash(repo.path()).unwrap_err();
        assert!(err.to_string().contains("no package vt100"), "{err}");
    }

    // ---- the fixture closure ----

    fn closure(repo: &Repo, sets: &[&str], entries: &[LogEntry]) -> String {
        let sets = sets.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
        repo.write(
            "parity/scenarios/closure.toml",
            &format!("version = 1\n[env]\nfixtures = [{sets}]\n"),
        );
        let scenario = repo.scenario("closure");
        let log = RequestLog::from_entries(entries.to_vec());
        fixtures_hash(repo.path(), &scenario, &log, &repo.roots()).unwrap()
    }

    #[test]
    fn the_fixture_hash_covers_only_fixtures_that_could_answer_a_logged_request() {
        let repo = Repo::harness();
        repo.write(
            "parity/fixtures/base/diff/GET.json",
            r#"{"version":1,"request":{"method":"GET","path":"/diff"},"response":{"status":200,"body_file":"GET.body.diff"}}"#,
        );
        repo.write("parity/fixtures/base/diff/GET.body.diff", "diff a b\n");
        std::fs::create_dir_all(repo.path().join("parity/fixtures/overlay")).unwrap();
        let log = vec![
            served("/user", "base/user/GET.json"),
            served("/diff", "base/diff/GET.json"),
        ];
        let sets = ["overlay", "base"];
        let before = closure(&repo, &sets, &log);

        repo.write("parity/fixtures/base/other/GET.json", &fixture("GET", "/other", "", ""));
        assert_eq!(
            closure(&repo, &sets, &log),
            before,
            "a fixture for a key not in the log"
        );
        let mut unserved = served("/other", "base/other/GET.json");
        unserved.miss = true;
        unserved.fixture = None;
        assert_eq!(
            closure(&repo, &sets, &[log.clone(), vec![unserved]].concat()),
            before,
            "a miss in the log"
        );

        repo.write(
            "parity/fixtures/base/user/GET.json",
            &fixture("GET", "/user", "", "\"login\":\"b\""),
        );
        let edited = closure(&repo, &sets, &log);
        assert_ne!(edited, before, "editing a served fixture");

        repo.write("parity/fixtures/base/diff/GET.body.diff", "diff c d\n");
        let body_edited = closure(&repo, &sets, &log);
        assert_ne!(body_edited, edited, "editing a served fixture's body_file");

        repo.write(
            "parity/fixtures/overlay/user/GET.json",
            &fixture(
                "GET",
                "/user",
                r#","when":{"after":[{"method":"GET","path":"/diff"}]}"#,
                "",
            ),
        );
        let with_when = closure(&repo, &sets, &log);
        assert_ne!(
            with_when, body_edited,
            "a same-key fixture with `when` in an earlier set"
        );

        repo.write(
            "parity/fixtures/overlay/user/GET.2.json",
            &fixture("GET", "/user", r#","body":{"x":1}"#, ""),
        );
        assert_ne!(closure(&repo, &sets, &log), with_when, "a same-key fixture with `body`");
    }

    #[test]
    fn the_fixture_hash_follows_pages_of_a_paginated_request() {
        let repo = Repo::harness();
        let page = |n: u32, link: &str| {
            format!(
                r#"{{"version":1,"request":{{"method":"GET","path":"/items","query":{{"page":"{n}","per_page":"2"}}}},"response":{{"status":200,"headers":{{"Link":"{link}"}},"body":[{n}]}}}}"#
            )
        };
        repo.write(
            "parity/fixtures/base/items/GET.page-1_per_page-2.json",
            &page(1, r#"<https://api.github.com/items?page=2&per_page=2>; rel=\"next\""#),
        );
        repo.write("parity/fixtures/base/items/GET.page-2_per_page-2.json", &page(2, ""));
        let mut entry = served("/items", "base/items/GET.page-1_per_page-2.json");
        entry.query = Some(fake_gh::fixture::query_from_pairs([("page", "1"), ("per_page", "2")]));
        let plain = vec![entry.clone()];
        entry.argv = vec!["api".into(), "--paginate".into(), "/items?per_page=2".into()];
        let paginated = vec![entry];

        let plain_before = closure(&repo, &["base"], &plain);
        let paginated_before = closure(&repo, &["base"], &paginated);
        repo.write("parity/fixtures/base/items/GET.page-2_per_page-2.json", &page(2, " "));
        assert_eq!(
            closure(&repo, &["base"], &plain),
            plain_before,
            "page 2 without --paginate"
        );
        assert_ne!(
            closure(&repo, &["base"], &paginated),
            paginated_before,
            "page 2 with --paginate"
        );
    }

    // ---- the manifest ----

    #[test]
    fn the_manifest_renders_with_a_header_and_parses_back() {
        let manifest = Manifest {
            format: 1,
            size: "160x48".into(),
            captures: vec!["main".into()],
            inputs: Inputs {
                scenario: "3f1c".into(),
                fixtures: "a09e".into(),
                python_app: "77d2".into(),
                harness: "c41b".into(),
            },
            recorded_with: RecordedWith {
                python: "3.11.14".into(),
                os: "linux".into(),
            },
        };
        let text = manifest.render();
        assert_eq!(
            text,
            "# Written by `cargo xtask parity --record`. Don't edit.\nformat = 1\nsize = \"160x48\"\ncaptures = \
             [\"main\"]\n\n[inputs]\nscenario = \"3f1c\"\nfixtures = \"a09e\"\npython_app = \"77d2\"\nharness = \
             \"c41b\"\n\n[recorded_with]\npython = \"3.11.14\"\nos = \"linux\"\n"
        );
        assert_eq!(Manifest::parse(&text), Ok(manifest));
        let err = Manifest::parse("format = 1\nsize = \"1x1\"\nbogus = 2\n").unwrap_err();
        assert_eq!(err.line, 3, "{err:?}");
    }

    #[test]
    fn recorded_with_is_never_compared() {
        let (repo, demo) = fresh_demo();
        let path = repo.path().join(format!("{REFERENCES_DIR}/demo/80x24/{MANIFEST_FILE}"));
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("3.11.14", "3.14.7")
            .replace("linux", "macos");
        std::fs::write(&path, text).unwrap();
        assert_eq!(state(&repo, &demo), Freshness::Fresh);
    }

    // ---- freshness ----

    #[test]
    fn a_just_recorded_reference_is_fresh() {
        let (repo, demo) = fresh_demo();
        assert_eq!(state(&repo, &demo), Freshness::Fresh);
        let report = check(repo.path(), &[demo], &repo.roots(), true).unwrap();
        assert_eq!(
            report,
            Report {
                checked: 1,
                problems: vec![]
            }
        );
    }

    #[test]
    fn a_stale_reference_names_each_changed_input_in_order() {
        let (repo, demo) = fresh_demo();
        repo.write("lazy_github/__init__.py", "print('changed')\n");
        assert_eq!(state(&repo, &demo), Freshness::Stale(vec!["python_app"]));
        repo.write("crates/fake-gh/src/lib.rs", "// changed\n");
        assert_eq!(state(&repo, &demo), Freshness::Stale(vec!["python_app", "harness"]));
        repo.write(
            "parity/fixtures/base/user/GET.json",
            &fixture("GET", "/user", "", "\"login\":\"z\""),
        );
        assert_eq!(
            state(&repo, &demo),
            Freshness::Stale(vec!["fixtures", "python_app", "harness"])
        );
        repo.write("parity/scenarios/demo.toml", &DEMO.replace("\"Hi\"", "\"Ho\""));
        let demo = repo.scenario("demo");
        assert_eq!(
            state(&repo, &demo),
            Freshness::Stale(vec!["scenario", "fixtures", "python_app", "harness"])
        );
        assert_eq!(
            problems(&repo, &[demo], true),
            [
                "stale reference: demo/80x24: scenario, fixtures, python_app, harness changed; run cargo xtask parity \
              --record demo"
            ]
        );
    }

    #[test]
    fn each_input_alone_makes_a_reference_stale() {
        let edits = [
            ("scenario", "parity/configs/default.json", "{\"x\": 1}\n".to_owned()),
            (
                "fixtures",
                "parity/fixtures/base/user/GET.json",
                fixture("GET", "/user", "", "\"login\":\"other\""),
            ),
            ("python_app", "uv.lock", "version = 2\n".to_owned()),
            ("harness", "xtask/src/parity/runner.rs", "// changed\n".to_owned()),
        ];
        for (input, file, contents) in edits {
            let (repo, demo) = fresh_demo();
            repo.write(file, &contents);
            assert_eq!(state(&repo, &demo), Freshness::Stale(vec![input]), "{file}");
        }
    }

    #[test]
    fn what_isnt_an_input_never_makes_a_reference_stale() {
        let (repo, _) = fresh_demo();
        repo.write("parity/themes.toml", "x = 1\n");
        repo.write("xtask/src/parity/check.rs", "// not an input\n");
        repo.write("lazy_github/__pycache__/x.pyc", "bytecode");
        repo.write(".gitignore", "__pycache__/\n");
        let edited = DEMO.replace(
            "version = 1",
            "version = 1\ngate = true\ndescription = \"x\"\nparity_rows = [\"a\"]",
        ) + "\n[expect.main]\ncontains = [\"Hi\"]\n";
        repo.write("parity/scenarios/demo.toml", &edited);
        assert_eq!(state(&repo, &repo.scenario("demo")), Freshness::Fresh);
    }

    #[test]
    fn a_reference_that_isnt_there_is_missing() {
        let repo = Repo::harness();
        let demo = repo.scenario("demo");
        assert_eq!(state(&repo, &demo), Freshness::Missing);
        // An empty directory can't be committed; it's a leftover, not a broken reference.
        std::fs::create_dir_all(repo.path().join(REFERENCES_DIR).join("demo/80x24")).unwrap();
        assert_eq!(state(&repo, &demo), Freshness::Missing);
        assert_eq!(
            problems(&repo, &[demo], true),
            ["missing reference: demo/80x24; run cargo xtask parity --record demo"]
        );
    }

    #[test]
    fn orphaned_directories_and_files_are_reported() {
        let (repo, demo) = fresh_demo();
        let refs = format!("{REFERENCES_DIR}/");
        repo.write(&format!("{refs}old-name/80x24/{MANIFEST_FILE}"), "");
        repo.write(&format!("{refs}demo/100x30/{MANIFEST_FILE}"), "");
        repo.write(&format!("{refs}demo/80x24/gone.screen"), "lazy-github-screen 1\n");
        repo.write(&format!("{refs}demo/80x24/main.txt"), "");
        assert_eq!(
            problems(&repo, std::slice::from_ref(&demo), true),
            [
                "orphaned reference: parity/references/demo/100x30/ (demo has no size 100x30); cargo xtask parity \
                 --record deletes it",
                "orphaned reference: parity/references/demo/80x24/gone.screen (demo has no capture gone); cargo \
                 xtask parity --record deletes it",
                "orphaned reference: parity/references/demo/80x24/main.txt (not part of a reference); cargo xtask \
                 parity --record deletes it",
                "orphaned reference: parity/references/old-name/ (no such scenario); cargo xtask parity --record \
                 deletes it",
            ]
        );
        // Checking only some scenarios can't tell whether another scenario's directory is an orphan.
        assert_eq!(problems(&repo, std::slice::from_ref(&demo), false).len(), 3);
        // The reference itself is still fresh.
        assert_eq!(state(&repo, &demo), Freshness::Fresh);
    }

    #[test]
    fn a_scenario_that_doesnt_run_python_has_no_reference() {
        let (repo, _) = fresh_demo();
        repo.write(
            "parity/scenarios/demo.toml",
            &DEMO.replace("version = 1", "version = 1\napps = [\"rust\"]"),
        );
        let report = check(repo.path(), &[repo.scenario("demo")], &repo.roots(), true).unwrap();
        assert_eq!(report.checked, 0);
        let lines: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "orphaned reference: parity/references/demo/ (demo doesn't run python); cargo xtask parity --record \
              deletes it"
            ]
        );
    }

    fn invalid(repo: &Repo, demo: &Scenario) -> String {
        match state(repo, demo) {
            Freshness::Invalid(detail) => detail,
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn manifest_captures_must_match_the_scenarios() {
        let (repo, _) = fresh_demo();
        repo.write(
            "parity/scenarios/demo.toml",
            &format!("{DEMO}\n[[step]]\ncapture = \"second\"\n"),
        );
        let demo = repo.scenario("demo");
        assert_eq!(
            invalid(&repo, &demo),
            "demo/80x24/manifest.toml: captures [\"main\"] don't match the scenario's [\"main\", \"second\"]"
        );
        assert_eq!(
            problems(&repo, &[demo], true),
            [
                "invalid reference: demo/80x24/manifest.toml: captures [\"main\"] don't match the scenario's \
              [\"main\", \"second\"]"
            ]
        );
    }

    #[test]
    fn a_wrong_format_or_broken_files_are_invalid() {
        let (repo, demo) = fresh_demo();
        let dir = format!("{REFERENCES_DIR}/demo/80x24");
        let good = std::fs::read_to_string(repo.path().join(&dir).join(MANIFEST_FILE)).unwrap();
        let manifest = format!("{dir}/{MANIFEST_FILE}");

        repo.write(&manifest, &good.replace("format = 1", "format = 2"));
        assert_eq!(
            invalid(&repo, &demo),
            "demo/80x24/manifest.toml: format 2; this harness reads format 1"
        );
        repo.write(&manifest, &good.replace("size = \"80x24\"", "size = \"1x1\""));
        assert!(invalid(&repo, &demo).contains("size \"1x1\" doesn't match"));
        repo.write(&manifest, &good.replace("[inputs]", "[inputs\n"));
        let detail = invalid(&repo, &demo);
        assert!(detail.starts_with("demo/80x24/manifest.toml:6: "), "{detail}");
        repo.write(&manifest, &good);
        assert_eq!(state(&repo, &demo), Freshness::Fresh);

        repo.write(&format!("{dir}/main.screen"), "not a screen\n");
        assert!(invalid(&repo, &demo).starts_with("demo/80x24/main.screen:1: "));
        repo.remove(&format!("{dir}/main.screen"));
        assert_eq!(invalid(&repo, &demo), "demo/80x24/main.screen: missing");
        repo.write(&format!("{dir}/main.screen"), &blank_screen(SIZE));

        repo.write(&format!("{dir}/{REQUESTS_FILE}"), &(jsonl(&user_log()) + "{oops\n"));
        assert!(invalid(&repo, &demo).starts_with("demo/80x24/requests.jsonl:2: "));
        repo.remove(&format!("{dir}/{REQUESTS_FILE}"));
        assert_eq!(invalid(&repo, &demo), "demo/80x24/requests.jsonl: missing");

        repo.remove(&manifest);
        assert_eq!(invalid(&repo, &demo), "demo/80x24: no manifest.toml");
    }

    #[test]
    fn a_screen_that_doesnt_parse_is_invalid_at_its_line() {
        let (repo, demo) = fresh_demo();
        let path = format!("{REFERENCES_DIR}/demo/80x24/main.screen");
        let good = std::fs::read_to_string(repo.path().join(&path)).unwrap();
        // Line 5 is the first text row; a 24-row screen can't end there.
        let short: String = good.lines().take(5).flat_map(|l| [l, "\n"]).collect();
        repo.write(&path, &short);
        assert_eq!(
            invalid(&repo, &demo),
            "demo/80x24/main.screen:6: expected 24 text rows, found 1"
        );
    }

    // ---- agreement ----

    /// A table pane and a Command Log under it.
    const LOGGED: &str = "\
┌─ [1] Repositories ──────┐
│ Owner       Name        │
│ parity-bot  alpha       │
│ gizmo       beta        │
└─────────────────────────┘
┌─ Command Log ───────────┐
│ 04:08:08 loaded         │
└─────────────────────────┘
--- styles

.hhhhhhhhhhhhhhhhhhhhhhhhh.
.rrrrrrrrrrrrrrrrrrrrrrrrr.
.ccccccccccccccccccccccccc.
--- legend
h fg=#e0e0e0 bg=#2d3740 bold
r fg=#e0e0e0 bg=#272727
c fg=#ddedf9 bg=#0178d4 bold
";

    fn logged(from: &str, to: &str) -> Grid {
        assert!(LOGGED.contains(from), "{from:?}");
        crate::parity::test_grid::parse(&LOGGED.replacen(from, to, 1))
    }

    fn agreeing_scenario(capture: &str, expect: &str) -> Scenario {
        let (_dir, roots) = key_roots();
        let source =
            format!("version = 1\n[env]\nfixtures = [\"base\"]\n\n[[step]]\ncapture = \"main\"\n{capture}\n{expect}");
        Scenario::parse(Path::new("parity/scenarios/agree.toml"), &source, &roots).unwrap_or_else(|e| panic!("{e}"))
    }

    fn side(label: &str, grid: Grid, requests: &[LogEntry]) -> Side {
        Side {
            label: label.into(),
            failure: None,
            captures: vec![("main".into(), grid)],
            requests: jsonl(requests),
        }
    }

    fn disagreements(scenario: &Scenario, a: &Side, b: &Side) -> Vec<String> {
        agree(scenario, &ThemeColors::default(), a, b)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    const IGNORE_LOG: &str = "ignore = { boxes = [\"Command Log\"] }";

    #[test]
    fn the_synthetic_grid_has_a_table_with_rows() {
        let s = structure::extract(&logged("", ""), &ThemeColors::default());
        let titles: Vec<&str> = s.boxes.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["[1] Repositories", "Command Log"]);
        assert_eq!(s.boxes[0].tables[0].rows.len(), 2, "{s:#?}");
    }

    #[test]
    fn grids_identical_apart_from_an_ignored_box_agree() {
        let scenario = agreeing_scenario(IGNORE_LOG, "");
        let a = side("run 1", logged("", ""), &user_log());
        let b = side("run 2", logged("04:08:08", "04:09:10"), &user_log());
        assert_eq!(disagreements(&scenario, &a, &b), Vec::<String>::new());
    }

    #[test]
    fn a_differing_row_in_a_checked_table_disagrees() {
        let scenario = agreeing_scenario(IGNORE_LOG, "");
        let a = side("run 1", logged("", ""), &user_log());
        let b = side("run 2", logged("alpha", "alphA"), &user_log());
        let found = disagreements(&scenario, &a, &b);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("run 2 differs from run 1: main: rows [[1] Repositories]: "),
            "{found:?}"
        );
    }

    #[test]
    fn the_same_difference_agrees_when_only_titles_are_checked() {
        let scenario = agreeing_scenario("checks = [\"titles\"]", "");
        let a = side("run 1", logged("", ""), &user_log());
        let b = side("run 2", logged("alpha", "alphA"), &user_log());
        assert_eq!(disagreements(&scenario, &a, &b), Vec::<String>::new());
    }

    #[test]
    fn a_differing_hidden_cursor_agrees() {
        let scenario = agreeing_scenario("", "");
        let a = side("run 1", logged("", ""), &user_log());
        let mut moved = logged("", "");
        moved.cursor.row = 5;
        moved.cursor.col = 7;
        let b = side("run 2", moved, &user_log());
        assert_eq!(disagreements(&scenario, &a, &b), Vec::<String>::new());
    }

    #[test]
    fn differing_served_requests_disagree() {
        let scenario = agreeing_scenario("", "");
        let a = side("run 1", logged("", ""), &user_log());
        let mut miss = served("/nope", "base/nope/GET.json");
        miss.miss = true;
        miss.fixture = None;
        // A miss isn't a served request, so it doesn't count.
        let b = side("run 2", logged("", ""), &[user_log(), vec![miss]].concat());
        assert_eq!(disagreements(&scenario, &a, &b), Vec::<String>::new());

        let b = side(
            "run 2",
            logged("", ""),
            &[user_log(), vec![served("/user/repos", "base/user/repos/GET.json")]].concat(),
        );
        assert_eq!(
            disagreements(&scenario, &a, &b),
            ["run 2 differs from run 1: served requests differ; only in run 2: GET /user/repos (json)"]
        );
    }

    #[test]
    fn a_failed_expect_or_run_disagrees() {
        let scenario = agreeing_scenario("", "[expect.main]\ncontains = [\"gamma\"]\n");
        let a = side("run 1", logged("", ""), &user_log());
        let b = side("run 2", logged("beta", "gamma"), &user_log());
        let found = disagreements(&scenario, &a, &b);
        assert_eq!(
            found.first().map(String::as_str),
            Some("run 1 [expect] main: contains: no row contains \"gamma\"")
        );
        assert_eq!(
            verify(&scenario, &ThemeColors::default(), &a)[0].capture(),
            Some("main")
        );

        let failed = Side {
            failure: Some("timeout at step 3 (15.1s)".into()),
            captures: Vec::new(),
            ..b.clone()
        };
        assert_eq!(
            disagreements(&scenario, &b, &failed),
            ["run 2 timeout at step 3 (15.1s)"]
        );
    }

    #[test]
    fn different_capture_lists_disagree() {
        let scenario = agreeing_scenario("", "");
        let a = side("run 1", logged("", ""), &user_log());
        let mut b = a.clone();
        b.label = "run 2".into();
        b.captures.push(("extra".into(), logged("", "")));
        assert_eq!(
            disagreements(&scenario, &a, &b),
            ["run 2 differs from run 1: captures [\"main\", \"extra\"], not [\"main\"]"]
        );
    }

    #[test]
    fn a_timestamp_inside_an_ignored_box_counts_as_identical() {
        let ignore = Ignore {
            boxes: vec!["Command Log".into()],
        };
        let theme = ThemeColors::default();
        let (a, b) = (logged("", ""), logged("04:08:08", "04:09:10"));
        assert!(identical_outside_ignored(&a, &b, &ignore, &theme));
        assert!(!identical_outside_ignored(&a, &b, &Ignore::default(), &theme));
        // The box's title is outside its interior.
        let renamed = logged("Command Log ─", "Command Logs ");
        assert!(!identical_outside_ignored(&a, &renamed, &ignore, &theme));
    }

    #[test]
    fn a_style_change_outside_an_ignored_box_counts_as_different() {
        let ignore = Ignore {
            boxes: vec!["Command Log".into()],
        };
        let theme = ThemeColors::default();
        let a = logged("", "");
        assert!(identical_outside_ignored(&a, &a.clone(), &ignore, &theme));
        // The selected row's style on one more cell, with the same text.
        let restyled = logged(".ccccccccccccccccccccccccc.", ".cccccccccccccccccccccccccc");
        assert!(!identical_outside_ignored(&a, &restyled, &ignore, &theme));
        let mut moved = a.clone();
        moved.cursor.row = 3;
        assert!(
            identical_outside_ignored(&a, &moved, &ignore, &theme),
            "the cursor isn't compared"
        );
    }

    #[test]
    fn served_keys_name_api_requests_and_commands() {
        let auth = LogEntry {
            argv: vec!["auth".into(), "status".into()],
            command: Some("auth status".into()),
            fixture: Some("base/auth.json".into()),
            ..LogEntry::default()
        };
        let mut paged = served("/items", "base/items/GET.json");
        paged.query = Some(fake_gh::fixture::query_from_pairs([("page", "2"), ("per_page", "50")]));
        let keys: Vec<String> = served_keys(&jsonl(&[auth, paged])).into_iter().collect();
        assert_eq!(keys, ["GET /items?page=2&per_page=50 (json)", "auth status"]);
    }
}
