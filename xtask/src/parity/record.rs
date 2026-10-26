//! `cargo xtask parity --record`: runs Python for each stale or missing reference and writes it when the runs
//! agree. The rules are in `parity/README.md` ("Recording").
//!
//! A (scenario, size) is one unit of work, spread over `--jobs` threads. Within a unit the runs are sequential:
//!
//! 1. One run. If an old reference exists and the run agrees with it, the manifest is rewritten and each old
//!    `.screen` is kept when it's identical outside ignored boxes, so a re-record that changes nothing leaves only
//!    the manifest in the diff.
//! 2. Otherwise a second run, and run 1 is written if the two agree.
//! 3. Otherwise nothing is written. A failed run is never written either, and there are no retries, so a flaky
//!    Python run shows up here rather than in a reference.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::check;
use super::cli::{self, RunOptions};
use super::diff;
use super::env::Paths;
use super::grid::Grid;
use super::reference::{
    self, Disagreement, Freshness, MANIFEST_FILE, MANIFEST_FORMAT, Manifest, Problem, REFERENCES_DIR, REQUESTS_FILE,
    RecordedWith, SCREEN_EXTENSION, Side,
};
use super::scenario::{App, Scenario, Size, Step};
use super::screen;
use super::structure::{ThemeColors, Themes};
use super::{Context, HarnessError, Result};

/// How to record.
pub struct Options<'a> {
    /// Re-record fresh references too.
    pub force: bool,
    /// Every scenario is selected, so reference directories of scenarios that don't exist are orphans.
    pub all: bool,
    /// How many units run at once.
    pub jobs: usize,
    /// Copy each run's `home/` and `work/` into its dump directory.
    pub keep_home: bool,
    pub themes: &'a Themes,
    /// `recorded_with.python` in the manifests written.
    pub python_version: String,
}

/// What happened to one reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Already fresh; nothing ran.
    Fresh,
    /// One run agreed with the old reference.
    Unchanged { duration: Duration },
    /// Two runs agreed; run 1 was written.
    Recorded { duration: Duration },
    /// Nothing was written.
    NotRecorded {
        /// The first disagreement, e.g. `run 2 differs from run 1: main: ...`.
        reason: String,
        /// A unified diff of what the two runs disagree on, when there is one.
        diff: String,
    },
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Fresh => f.write_str("fresh"),
            Outcome::Unchanged { duration } => write!(f, "unchanged (1 run, {:.1}s)", duration.as_secs_f64()),
            Outcome::Recorded { duration } => write!(f, "recorded (2 runs, {:.1}s)", duration.as_secs_f64()),
            Outcome::NotRecorded { reason, .. } => write!(f, "not recorded: {reason}"),
        }
    }
}

/// One reference's outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitOutcome {
    pub scenario: String,
    pub size: Size,
    pub outcome: Outcome,
}

/// What [`record`] did.
#[derive(Debug, Clone)]
pub struct Report {
    /// One per selected scenario that runs Python, per size, in order.
    pub outcomes: Vec<UnitOutcome>,
    /// The orphans deleted, as `--check-references` names them.
    pub deleted: Vec<String>,
    /// What's still wrong afterwards: the references that couldn't be recorded.
    pub remaining: reference::Report,
}

impl Report {
    /// `N recorded, N unchanged, N fresh, N not recorded, N orphan(s) deleted`.
    pub fn counts(&self) -> String {
        let count = |pred: fn(&Outcome) -> bool| self.outcomes.iter().filter(|u| pred(&u.outcome)).count();
        format!(
            "{} recorded, {} unchanged, {} fresh, {} not recorded, {} orphan(s) deleted",
            count(|o| matches!(o, Outcome::Recorded { .. })),
            count(|o| matches!(o, Outcome::Unchanged { .. })),
            count(|o| matches!(o, Outcome::Fresh)),
            count(|o| matches!(o, Outcome::NotRecorded { .. })),
            self.deleted.len(),
        )
    }
}

/// Records the references of `scenarios` under `repo`, writing run dumps to `<out>/<scenario>/<size>/run-N/`, then
/// deletes orphans. Prints one line per reference as it finishes. A harness error in one unit doesn't stop the
/// others; the first is returned after all of them ran and the orphans were deleted.
pub fn record(repo: &Path, scenarios: &[Scenario], paths: &Paths, out: &Path, options: &Options<'_>) -> Result<Report> {
    let units: Vec<(&Scenario, Size)> = scenarios
        .iter()
        .filter(|s| s.apps.contains(&App::Python))
        .flat_map(|s| s.sizes.iter().map(move |&size| (s, size)))
        .collect();
    let results = cli::parallel_map(&units, options.jobs, |&(scenario, size)| {
        let result = record_unit(repo, scenario, size, paths, out, options);
        let mut text = format!("{} {size}: ", scenario.name);
        match &result {
            Ok(outcome) => {
                text.push_str(&outcome.to_string());
                text.push('\n');
                if let Outcome::NotRecorded { diff, .. } = outcome {
                    text.push_str(diff);
                }
            }
            Err(e) => {
                text.push_str("harness error: ");
                text.push_str(&e.to_string());
                text.push('\n');
            }
        }
        // One write per unit, so lines from parallel units don't interleave.
        print!("{text}");
        result
    });

    let mut outcomes = Vec::new();
    let mut first_error = None;
    for (&(scenario, size), result) in units.iter().zip(results) {
        match result {
            Some(Ok(outcome)) => outcomes.push(UnitOutcome {
                scenario: scenario.name.clone(),
                size,
                outcome,
            }),
            Some(Err(e)) => {
                first_error.get_or_insert(e);
            }
            None => {
                first_error.get_or_insert_with(|| {
                    HarnessError(format!("{} {size}: the worker thread panicked", scenario.name))
                });
            }
        }
    }

    let mut deleted = Vec::new();
    for problem in reference::check(repo, scenarios, &paths.roots, options.all)?.problems {
        let Problem::Orphaned { path, .. } = problem else {
            continue;
        };
        remove(&repo_path(repo, &path))?;
        println!("deleted orphaned reference: {path}");
        deleted.push(path);
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    let remaining = reference::check(repo, scenarios, &paths.roots, options.all)?;
    Ok(Report {
        outcomes,
        deleted,
        remaining,
    })
}

/// A reference as it's stored: its grids for comparing and its files' text for keeping.
struct Stored {
    side: Side,
    /// Each capture's `.screen` text.
    screens: Vec<(String, String)>,
}

/// One finished Python run.
struct Run {
    side: Side,
    duration: Duration,
}

fn record_unit(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &Options<'_>,
) -> Result<Outcome> {
    let freshness = reference::freshness(repo, scenario, size, &paths.roots)?;
    if freshness == Freshness::Fresh && !options.force {
        return Ok(Outcome::Fresh);
    }
    let theme = options.themes.colors(&check::scenario_theme(scenario, &paths.roots)?);
    let dir = reference_dir(repo, scenario, size);
    // A missing or invalid reference has nothing to compare with.
    let old = match freshness {
        Freshness::Fresh | Freshness::Stale(_) => Some(load(&dir, scenario)?),
        Freshness::Missing | Freshness::Invalid(_) => None,
    };

    let dumps = out.join(&scenario.name).join(size.to_string());
    if dumps.exists() {
        std::fs::remove_dir_all(&dumps).with_context(|| format!("wiping {}", dumps.display()))?;
    }
    let run_options = RunOptions {
        apps: &[App::Python],
        sizes: &[],
        retries: 0,
        keep_home: options.keep_home,
        cap_ungated_waits: false,
        themes: options.themes,
    };
    let run = |n: u32| run_python(scenario, size, paths, &dumps.join(format!("run-{n}")), &run_options, n);

    let first = run(1)?;
    if let Some(problem) = reference::verify(scenario, &theme, &first.side).first() {
        return Ok(Outcome::NotRecorded {
            reason: problem.to_string(),
            diff: String::new(),
        });
    }
    if let Some(old) = &old
        && reference::agree(scenario, &theme, &old.side, &first.side).is_empty()
    {
        write_reference(
            repo,
            scenario,
            size,
            &theme,
            &first.side,
            Some(old),
            options,
            &paths.roots,
        )?;
        return Ok(Outcome::Unchanged {
            duration: first.duration,
        });
    }

    let second = run(2)?;
    let duration = first.duration + second.duration;
    match reference::agree(scenario, &theme, &first.side, &second.side).first() {
        None => {
            write_reference(repo, scenario, size, &theme, &first.side, None, options, &paths.roots)?;
            Ok(Outcome::Recorded { duration })
        }
        Some(problem) => Ok(Outcome::NotRecorded {
            reason: problem.to_string(),
            diff: disagreement_diff(problem, &first.side, &second.side),
        }),
    }
}

/// Runs Python once, with no retries and no wait caps, and reads back what it captured.
fn run_python(
    scenario: &Scenario,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
    n: u32,
) -> Result<Run> {
    let status = cli::run_app(scenario, App::Python, size, paths, out, options)?;
    let failure = (!status.passed()).then(|| status.describe());
    let mut captures = Vec::new();
    if failure.is_none() {
        for name in scenario.capture_names() {
            let path = out.join(format!("{name}.grid.json"));
            let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let grid: Grid = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
            captures.push((name.to_owned(), grid));
        }
    }
    let log = out.join("fake-gh.log");
    let log = match std::fs::read_to_string(&log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(HarnessError(format!("reading {}: {e}", log.display()))),
    };
    Ok(Run {
        side: Side {
            label: format!("run {n}"),
            failure,
            captures,
            requests: screen::normalize_log(&log),
        },
        duration: Duration::from_millis(status.duration_ms),
    })
}

/// Reads a reference that `freshness` found valid.
fn load(dir: &Path, scenario: &Scenario) -> Result<Stored> {
    let mut captures = Vec::new();
    let mut screens = Vec::new();
    for name in scenario.capture_names() {
        let path = dir.join(format!("{name}.{SCREEN_EXTENSION}"));
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let grid = screen::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        captures.push((name.to_owned(), grid));
        screens.push((name.to_owned(), text));
    }
    let path = dir.join(REQUESTS_FILE);
    let requests = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Stored {
        side: Side {
            label: "the reference".to_owned(),
            failure: None,
            captures,
            requests,
        },
        screens,
    })
}

/// A unified diff of what `problem` is about: the capture's text, or the request logs.
fn disagreement_diff(problem: &Disagreement, a: &Side, b: &Side) -> String {
    if let Disagreement::Requests { .. } = problem {
        return diff::labeled_diff(&a.requests, &b.requests, &a.label, &b.label);
    }
    let Some(name) = problem.capture() else {
        return String::new();
    };
    match (a.capture(name), b.capture(name)) {
        (Some(ga), Some(gb)) => diff::labeled_diff(&ga.to_text(), &gb.to_text(), &a.label, &b.label),
        _ => String::new(),
    }
}

/// `parity/references/<scenario>/<size>/`.
pub fn reference_dir(repo: &Path, scenario: &Scenario, size: Size) -> PathBuf {
    repo.join(REFERENCES_DIR).join(&scenario.name).join(size.to_string())
}

/// Writes `run` as the reference, replacing the directory in one rename. With `old`, each old `.screen` that's
/// identical to the run's outside ignored boxes is kept as it was.
#[allow(clippy::too_many_arguments, reason = "one call site per path through record_unit")]
fn write_reference(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    theme: &ThemeColors,
    run: &Side,
    old: Option<&Stored>,
    options: &Options<'_>,
    roots: &super::scenario::Roots,
) -> Result<()> {
    let dir = reference_dir(repo, scenario, size);
    let tmp = dir.with_file_name(format!("{size}.tmp"));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).with_context(|| format!("wiping {}", tmp.display()))?;
    }
    std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let write = |name: &str, text: &str| {
        let path = tmp.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    };

    for step in &scenario.steps {
        let Step::Capture(capture) = step else { continue };
        let name = &capture.name;
        let grid = run
            .capture(name)
            .ok_or_else(|| HarnessError(format!("{} has no capture {name}", run.label)))?;
        let kept = old.and_then(|old| {
            let old_grid = old.side.capture(name)?;
            let (_, text) = old.screens.iter().find(|(n, _)| n == name)?;
            reference::identical_outside_ignored(old_grid, grid, &capture.ignore, theme).then_some(text)
        });
        let text = match kept {
            Some(text) => text.clone(),
            None => screen::render(grid).with_context(|| format!("{} {size} {name}", scenario.name))?,
        };
        write(&format!("{name}.{SCREEN_EXTENSION}"), &text)?;
    }
    write(REQUESTS_FILE, &run.requests)?;
    let requests = reference::parse_requests(&run.requests).map_err(|(line, message)| {
        HarnessError(format!("{} {size}: {REQUESTS_FILE}:{line}: {message}", scenario.name))
    })?;
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        size: size.to_string(),
        captures: scenario.capture_names().map(str::to_owned).collect(),
        inputs: reference::current_inputs(repo, scenario, size, &requests, roots)?,
        recorded_with: RecordedWith {
            python: options.python_version.clone(),
            os: std::env::consts::OS.to_owned(),
        },
    };
    write(MANIFEST_FILE, &manifest.render())?;
    replace_dir(&tmp, &dir)?;

    match reference::freshness(repo, scenario, size, roots)? {
        Freshness::Fresh => Ok(()),
        other => Err(HarnessError(format!(
            "{}/{size}: the reference just written isn't fresh ({other:?}); did an input change during the run?",
            scenario.name
        ))),
    }
}

/// Moves `from` to `to`, replacing `to`. A directory can't be renamed over a non-empty one, so the old one is moved
/// aside first; an interrupted replace leaves `<size>.old/` or `<size>.tmp/`, which the next `--record` deletes as
/// orphans.
fn replace_dir(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        let aside = to.with_extension("old");
        if aside.exists() {
            std::fs::remove_dir_all(&aside).with_context(|| format!("wiping {}", aside.display()))?;
        }
        std::fs::rename(to, &aside).with_context(|| format!("moving {} aside", to.display()))?;
        std::fs::rename(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
        std::fs::remove_dir_all(&aside).with_context(|| format!("removing {}", aside.display()))
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::rename(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))
    }
}

/// A repo-relative, `/`-separated path (directories end in `/`) on disk.
fn repo_path(repo: &Path, path: &str) -> PathBuf {
    path.split('/')
        .filter(|p| !p.is_empty())
        .fold(repo.to_owned(), |acc, p| acc.join(p))
}

fn remove(path: &Path) -> Result<()> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.with_context(|| format!("deleting {}", path.display()))
}
