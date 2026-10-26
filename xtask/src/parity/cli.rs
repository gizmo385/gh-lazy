//! The `cargo xtask parity` command: resolve and validate scenarios, decide where each Python side comes from (its
//! recorded reference, or a live run checked against it), build and sync, run every scenario × size × app, and write
//! `summary.json`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::check;
use super::ci::{self, Finding};
use super::env::{self, Paths};
use super::record;
use super::reference::{self, Freshness};
use super::report;
use super::runner::{self, AppRunStatus};
use super::scenario::{App, Roots, Scenario, Size};
use super::structure::Themes;
use super::summary::{SUMMARY_VERSION, ScenarioResult, ScenarioSummary, SizeSummary, Summary};
use super::{Context, HarnessError, Result};

/// Exit code: every gated scenario passed and the Python side is clean.
pub const EXIT_OK: u8 = 0;
/// Exit code: a gated scenario failed, or the Python side failed anywhere.
pub const EXIT_FAILED: u8 = 1;
/// Exit code: a scenario file is invalid, or the harness itself failed.
pub const EXIT_HARNESS: u8 = 2;

/// Run parity scenarios against the Python and Rust apps and capture their screens.
#[derive(Debug, Clone, clap::Args)]
#[allow(clippy::struct_excessive_bools, reason = "each bool is a command-line switch")]
pub struct ParityArgs {
    /// Scenario names (file stems in parity/scenarios/) or paths to scenario files. None means --all.
    #[arg(value_name = "SCENARIO")]
    pub scenarios: Vec<String>,
    /// Run every scenario in parity/scenarios/.
    #[arg(long)]
    pub all: bool,
    /// Run only the Python app, live, and check it against its recorded reference (drift).
    #[arg(long, conflicts_with = "rust_only")]
    pub python_only: bool,
    /// Run only the Rust app. Recorded references are neither read nor checked.
    #[arg(long)]
    pub rust_only: bool,
    /// Run Python live instead of reading its recorded reference: for every selected scenario, or with `=a,b` for
    /// those (the others read theirs). A live run is checked against its reference when that's fresh (drift).
    #[arg(
        long,
        value_name = "SCENARIO,...",
        num_args = 0..=1,
        require_equals = true,
        value_delimiter = ',',
        conflicts_with = "rust_only"
    )]
    pub live_python: Option<Vec<String>>,
    /// Run at this size instead of the scenarios' sizes. Repeatable.
    #[arg(long = "size", value_name = "COLSxROWS")]
    pub sizes: Vec<Size>,
    /// The report directory [default: parity/reports/latest, wiped first].
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Copy each run's home/ and work/ into its report dir.
    #[arg(long)]
    pub keep_home: bool,
    /// Don't run `cargo build -p lazy-github -p fake-gh` first.
    #[arg(long)]
    pub no_build: bool,
    /// Don't run `uv sync --frozen` first.
    #[arg(long)]
    pub no_sync: bool,
    /// The Python interpreter to run `-m lazy_github` with [default: the repo's .venv].
    #[arg(long, value_name = "PATH")]
    pub python: Option<PathBuf>,
    /// Re-run a failed app run up to N times; a run that passes on retry is marked flaky [default: 0, or 1 with
    /// --ci]. A Rust run in a scenario with `gate = false` is never retried.
    #[arg(long, value_name = "N")]
    pub retries: Option<u32>,
    /// Run up to N scenarios in parallel. Sizes and apps within a scenario always run one after another [default:
    /// the CPU count, at most 4].
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub jobs: Option<u16>,
    /// Running in CI: retry failed runs once, cap each wait at 3 s for runs that can't fail the gate (Rust in a
    /// `gate = false` scenario), print GitHub Actions `::error`/`::warning` annotations, and ask cargo and uv for
    /// plain (uncolored) output.
    #[arg(long)]
    pub ci: bool,
    /// Check every scenario file (or the ones named) without running anything.
    #[arg(long)]
    pub validate: bool,
    /// Check the recorded Python references in parity/references/ (all, or the named scenarios') against the
    /// current inputs, without building or running anything. Exits 1 if any is stale, missing, orphaned or
    /// invalid.
    #[arg(
        long,
        conflicts_with_all = [
            "python_only", "rust_only", "live_python", "sizes", "validate", "out", "keep_home", "no_build", "no_sync",
            "python", "retries", "jobs",
        ]
    )]
    pub check_references: bool,
    /// Record the Python references in parity/references/ that are stale or missing (all, or the named
    /// scenarios'): run Python with no retries, twice, and write a reference only when both runs pass and agree.
    /// Builds only fake-gh. Run dumps go to --out [default: parity/reports/record, wiped first].
    #[arg(
        long,
        conflicts_with_all = [
            "python_only", "rust_only", "live_python", "sizes", "retries", "ci", "validate", "check_references",
        ]
    )]
    pub record: bool,
    /// With --record: re-record fresh references too.
    #[arg(long, requires = "record")]
    pub force: bool,
    /// Read fixture sets from this directory instead of parity/fixtures/. For tests that need a modified set.
    #[arg(long, value_name = "DIR", hide = true)]
    pub fixtures_dir: Option<PathBuf>,
}

/// The repository root: xtask lives one level below it.
pub fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_owned()
}

/// The venv interpreter `uv sync` creates.
pub fn venv_python(repo: &Path) -> PathBuf {
    if cfg!(windows) {
        repo.join(".venv").join("Scripts").join("python.exe")
    } else {
        repo.join(".venv/bin/python")
    }
}

/// Runs the command and returns its exit code.
pub fn run(args: &ParityArgs) -> std::process::ExitCode {
    let code = run_inner(args).unwrap_or_else(|e| {
        eprintln!("parity: {e}");
        if args.ci {
            println!("{}", ci::harness_error(format!("parity: {e}")));
        }
        EXIT_HARNESS
    });
    std::process::ExitCode::from(code)
}

fn run_inner(args: &ParityArgs) -> Result<u8> {
    let repo = repo_root();
    let roots = Roots {
        fixtures: args
            .fixtures_dir
            .clone()
            .unwrap_or_else(|| repo.join("parity/fixtures")),
        configs: repo.join("parity/configs"),
    };

    let files = resolve_scenarios(&repo, args)?;
    let Some(scenarios) = load_scenarios(&repo, &files, &roots, args.ci) else {
        return Ok(EXIT_HARNESS);
    };
    if args.validate {
        println!("parity: {} scenario file(s) valid", scenarios.len());
        return Ok(EXIT_OK);
    }
    if args.check_references {
        return check_references(&repo, &scenarios, &roots, args);
    }
    if args.record {
        return record_references(&repo, &scenarios, roots, args);
    }
    let themes = Themes::load(&repo.join("parity/themes.toml"))?;
    run_scenarios(&repo, &scenarios, &roots, &themes, args, None)
}

/// Where a scenario's Python side comes from in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    /// Read from its recorded reference in `parity/references/`; nothing is started.
    Recorded,
    /// Run live, then checked against its reference when that's fresh (drift).
    Live,
}

/// What a run does, resolved from the flags once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The apps requested: both, or the one `--python-only` or `--rust-only` names.
    pub apps: Vec<App>,
    /// Per selected scenario, in order: where its Python side comes from, or `None` when it doesn't run Python
    /// (`--rust-only`, or not in its `apps`).
    pub python: Vec<Option<PythonSource>>,
    /// What the report says about the Python side.
    pub python_meta: report::AppSource,
}

impl Plan {
    /// Resolves the flags for `scenarios`. Naming a scenario in `--live-python=` that isn't selected is an error.
    pub fn new(args: &ParityArgs, scenarios: &[Scenario]) -> Result<Self> {
        let named: Option<&[String]> = args.live_python.as_deref().filter(|names| !names.is_empty());
        for name in named.unwrap_or_default() {
            if !scenarios.iter().any(|s| s.name == *name) {
                return Err(HarnessError(format!(
                    "--live-python names {name:?}, which isn't a selected scenario"
                )));
            }
        }
        let apps = if args.python_only {
            vec![App::Python]
        } else if args.rust_only {
            vec![App::Rust]
        } else {
            App::ALL.to_vec()
        };
        let all_live = args.python_only || (args.live_python.is_some() && named.is_none());
        let python: Vec<_> = scenarios
            .iter()
            .map(|s| {
                let runs = apps.contains(&App::Python) && s.apps.contains(&App::Python);
                let live = all_live || named.is_some_and(|names| names.contains(&s.name));
                runs.then_some(if live {
                    PythonSource::Live
                } else {
                    PythonSource::Recorded
                })
            })
            .collect();
        let live: Vec<String> = scenarios
            .iter()
            .zip(&python)
            .filter(|(_, p)| **p == Some(PythonSource::Live))
            .map(|(s, _)| s.name.clone())
            .collect();
        let any_recorded = python.contains(&Some(PythonSource::Recorded));
        let python_meta = if !apps.contains(&App::Python) {
            report::AppSource::NotRun
        } else if !any_recorded && (all_live || !live.is_empty()) {
            report::AppSource::Live
        } else if !live.is_empty() {
            report::AppSource::LiveFor(live)
        } else {
            report::AppSource::Recorded
        };
        Ok(Self {
            apps,
            python,
            python_meta,
        })
    }

    fn rust_meta(&self) -> report::AppSource {
        if self.apps.contains(&App::Rust) {
            report::AppSource::Live
        } else {
            report::AppSource::NotRun
        }
    }
}

/// The freshness of each reference a run reads or checks for drift, by scenario name and size.
type Freshnesses = HashMap<(String, Size), Freshness>;

/// The freshness of the reference of every scenario × size that runs Python in `plan`.
fn freshnesses(repo: &Path, scenarios: &[Scenario], plan: &Plan, sizes: &[Size], roots: &Roots) -> Result<Freshnesses> {
    let units: Vec<(&Scenario, Size)> = scenarios
        .iter()
        .zip(&plan.python)
        .filter(|(_, python)| python.is_some())
        .flat_map(|(s, _)| {
            let sizes = if sizes.is_empty() { &s.sizes[..] } else { sizes };
            sizes.iter().map(move |&size| (s, size))
        })
        .collect();
    let states = reference::freshness_of(repo, &units, roots)?;
    Ok(units
        .into_iter()
        .zip(states)
        .map(|((s, size), state)| ((s.name.clone(), size), state))
        .collect())
}

/// The references a run reads that aren't fresh, in scenario and size order, as `--check-references` reports them.
fn problems_reading(
    scenarios: &[Scenario],
    plan: &Plan,
    sizes: &[Size],
    freshness: &Freshnesses,
) -> Vec<reference::Problem> {
    let mut problems = Vec::new();
    for (scenario, python) in scenarios.iter().zip(&plan.python) {
        if *python != Some(PythonSource::Recorded) {
            continue;
        }
        let sizes = if sizes.is_empty() { &scenario.sizes[..] } else { sizes };
        for &size in sizes {
            let state = freshness
                .get(&(scenario.name.clone(), size))
                .cloned()
                .unwrap_or(Freshness::Missing);
            problems.extend(reference::problem(scenario, size, state));
        }
    }
    problems
}

/// What a stale reference in a run that reads it says: the `--check-references` line and the way around it.
const LIVE_HINT: &str = " (or use --live-python)";

/// Runs `scenarios` (already loaded and valid) under `repo` as `args` say: checks the references the run reads
/// (exit 2 when one isn't fresh, before anything is built or started), builds and syncs, runs every scenario × size
/// × app, and writes `summary.json` and the report. `paths` overrides the binaries and the interpreter the flags
/// would pick.
pub fn run_scenarios(
    repo: &Path,
    scenarios: &[Scenario],
    roots: &Roots,
    themes: &Themes,
    args: &ParityArgs,
    paths: Option<Paths>,
) -> Result<u8> {
    let plan = Plan::new(args, scenarios)?;
    let freshness = freshnesses(repo, scenarios, &plan, &args.sizes, roots)?;
    let problems = problems_reading(scenarios, &plan, &args.sizes, &freshness);
    if !problems.is_empty() {
        for problem in &problems {
            println!("{problem}{LIVE_HINT}");
        }
        if args.ci {
            let report = reference::Report { checked: 0, problems };
            for mut annotation in reference_annotations(repo, scenarios, &report) {
                annotation.message.push_str(LIVE_HINT);
                println!("{annotation}");
            }
        }
        return Ok(EXIT_HARNESS);
    }

    let needs_python = plan.python.contains(&Some(PythonSource::Live));
    build_and_sync(repo, args, &["lazy-github", "fake-gh"], needs_python)?;
    let paths = match paths {
        Some(paths) => paths,
        None => self::paths(repo, args, roots.clone())?,
    };
    let out = report_dir(repo, args, "latest")?;

    let options = RunOptions {
        apps: &plan.apps,
        sizes: &args.sizes,
        retries: args.retries.unwrap_or(u32::from(args.ci)),
        keep_home: args.keep_home,
        cap_ungated_waits: args.ci,
        themes,
    };
    let context = RunContext {
        repo,
        paths: &paths,
        out: &out,
        options: &options,
        freshness: &freshness,
    };
    let units: Vec<(&Scenario, Option<PythonSource>)> = scenarios.iter().zip(plan.python.iter().copied()).collect();
    let runs = run_all(&units, &context, jobs(args.jobs));
    let (summaries, annotations) = split_runs(runs, scenarios, repo, args.ci);

    let summary = Summary {
        version: SUMMARY_VERSION,
        scenarios: summaries,
    };
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(&summary).expect("summary serializes");
    std::fs::write(out.join("summary.json"), json + "\n").context("writing summary.json")?;
    let meta = report::RunMeta::current(repo, plan.python_meta.clone(), plan.rust_meta());
    // A reporting failure is printed but keeps the run's exit code, so it can never mask a gate result.
    if let Err(e) = report::write_reports(&out, &summary, &meta) {
        eprintln!("parity: writing the report: {e}");
    }

    println!();
    for s in &summary.scenarios {
        println!("{}: {}", s.name, s.result);
    }
    println!("report: {}", out.display());
    for annotation in &annotations {
        println!("{annotation}");
    }

    Ok(exit_code(&summary))
}

/// The binaries and resources runs need. `cargo xtask` runs target/<profile>/xtask, next to the binaries
/// `cargo build` produces.
fn paths(repo: &Path, args: &ParityArgs, roots: Roots) -> Result<Paths> {
    let exe = std::env::current_exe().context("finding the xtask binary")?;
    let profile_dir = exe
        .parent()
        .ok_or_else(|| HarnessError("xtask binary has no parent dir".into()))?;
    let paths = Paths {
        fake_gh: profile_dir.join(format!("fake-gh{}", std::env::consts::EXE_SUFFIX)),
        rust_app: profile_dir.join(format!("lazy-github{}", std::env::consts::EXE_SUFFIX)),
        python: args.python.clone().unwrap_or_else(|| venv_python(repo)),
        roots,
    };
    if !paths.fake_gh.is_file() {
        return Err(HarnessError(format!(
            "{} doesn't exist; build it with `cargo build -p fake-gh` or drop --no-build",
            paths.fake_gh.display()
        )));
    }
    Ok(paths)
}

/// `--out`, or `parity/reports/<default>/` wiped first. Created either way.
fn report_dir(repo: &Path, args: &ParityArgs, default: &str) -> Result<PathBuf> {
    let out = if let Some(out) = &args.out {
        absolute(out)?
    } else {
        let dir = repo.join("parity/reports").join(default);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("wiping {}", dir.display()))?;
        }
        dir
    };
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    Ok(out)
}

/// `--record`: builds fake-gh, syncs the venv, records, deletes orphans and prints a count. Exit 0 when every
/// selected reference is fresh at the end, 1 when some couldn't be recorded.
fn record_references(repo: &Path, scenarios: &[Scenario], roots: Roots, args: &ParityArgs) -> Result<u8> {
    let needs_python = scenarios.iter().any(|s| s.apps.contains(&App::Python));
    build_and_sync(repo, args, &["fake-gh"], needs_python)?;
    let paths = paths(repo, args, roots)?;
    let python_version = if needs_python {
        python_version(&paths.python)?
    } else {
        String::new()
    };
    let out = report_dir(repo, args, "record")?;
    let themes = Themes::load(&repo.join("parity/themes.toml"))?;
    let options = record::Options {
        force: args.force,
        all: args.all || args.scenarios.is_empty(),
        jobs: jobs(args.jobs),
        keep_home: args.keep_home,
        themes: &themes,
        python_version,
    };
    let report = record::record(repo, scenarios, &paths, &out, &options)?;
    println!();
    for problem in &report.remaining.problems {
        println!("{problem}");
    }
    println!("parity: {}", report.counts());
    println!("run dumps: {}", out.display());
    Ok(if report.remaining.problems.is_empty() {
        EXIT_OK
    } else {
        EXIT_FAILED
    })
}

/// `<python> --version`'s version number, for `recorded_with.python`.
fn python_version(python: &Path) -> Result<String> {
    let output = Command::new(python)
        .arg("--version")
        .output()
        .with_context(|| format!("running {} --version", python.display()))?;
    if !output.status.success() {
        return Err(HarnessError(format!(
            "{} --version failed ({}): {}",
            python.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    // Python 2 wrote its version to stderr.
    let text = [output.stdout, output.stderr].concat();
    Ok(parse_python_version(&String::from_utf8_lossy(&text)))
}

/// `Python 3.11.14` -> `3.11.14`; anything else is kept as printed.
fn parse_python_version(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    line.strip_prefix("Python ").unwrap_or(line).to_owned()
}

/// `--check-references`: one line per problem (and, with `--ci`, an `::error` on the scenario file), then a count.
fn check_references(repo: &Path, scenarios: &[Scenario], roots: &Roots, args: &ParityArgs) -> Result<u8> {
    let all = args.all || args.scenarios.is_empty();
    let report = reference::check(repo, scenarios, roots, all)?;
    for problem in &report.problems {
        println!("{problem}");
    }
    if report.problems.is_empty() {
        println!("parity: {} recorded reference(s), all fresh", report.checked);
        return Ok(EXIT_OK);
    }
    println!(
        "parity: {} reference problem(s) ({}) across {} reference(s)",
        report.problems.len(),
        report.counts(),
        report.checked
    );
    if args.ci {
        for annotation in reference_annotations(repo, scenarios, &report) {
            println!("{annotation}");
        }
    }
    Ok(EXIT_FAILED)
}

/// An `::error` per problem, on the scenario file it belongs to (none for a directory of a removed scenario).
fn reference_annotations(repo: &Path, scenarios: &[Scenario], report: &reference::Report) -> Vec<ci::Annotation> {
    report
        .problems
        .iter()
        .map(|problem| ci::Annotation {
            level: ci::Level::Error,
            file: problem
                .scenario()
                .and_then(|name| scenarios.iter().find(|s| s.name == name))
                .map(|s| ci::scenario_file(repo, &s.path)),
            line: None,
            message: problem.to_string(),
        })
        .collect()
}

/// Loads and validates every scenario file, printing each problem (and, with `--ci`, an `::error` for it). `None`
/// when any file is invalid.
fn load_scenarios(repo: &Path, files: &[PathBuf], roots: &Roots, ci: bool) -> Option<Vec<Scenario>> {
    let mut scenarios = Vec::new();
    let mut invalid = Vec::new();
    for file in files {
        match Scenario::load(file, roots) {
            Ok(s) => scenarios.push(s),
            Err(e) => {
                eprintln!("{e}");
                invalid.push(ci::invalid_scenario(repo, &e));
            }
        }
    }
    for (path, problem) in duplicate_names(&scenarios) {
        eprintln!("{}: {problem}", path.display());
        invalid.push(ci::Annotation {
            level: ci::Level::Error,
            file: Some(ci::scenario_file(repo, path)),
            line: None,
            message: problem,
        });
    }
    if invalid.is_empty() {
        return Some(scenarios);
    }
    if ci {
        for annotation in &invalid {
            println!("{annotation}");
        }
    }
    None
}

/// Splits the runs into their summaries and, with `--ci`, the annotations for them.
fn split_runs(
    runs: Vec<ScenarioRun>,
    scenarios: &[Scenario],
    repo: &Path,
    ci: bool,
) -> (Vec<ScenarioSummary>, Vec<ci::Annotation>) {
    let mut annotations = Vec::new();
    let summaries = runs
        .into_iter()
        .zip(scenarios)
        .map(|((summary, findings), scenario)| {
            if ci {
                let file = ci::scenario_file(repo, &scenario.path);
                annotations.extend(ci::annotations(&file, &summary, &findings));
            }
            summary
        })
        .collect();
    (summaries, annotations)
}

/// Each scenario owns `<out>/<name>/`, so two files with one stem in one run would wipe each other's results (and
/// race under `--jobs`). Returns the later file and a message for each clash.
fn duplicate_names(scenarios: &[Scenario]) -> Vec<(&Path, String)> {
    let mut seen = BTreeMap::new();
    let mut problems = Vec::new();
    for s in scenarios {
        if let Some(first) = seen.insert(s.name.as_str(), &s.path) {
            problems.push((
                s.path.as_path(),
                format!(
                    "scenario name {:?} is also used by {}; scenario names must be unique in a run",
                    s.name,
                    first.display()
                ),
            ));
        }
    }
    problems
}

fn exit_code(summary: &Summary) -> u8 {
    if summary.scenarios.iter().any(|s| s.error.is_some()) {
        EXIT_HARNESS
    } else if summary.scenarios.iter().any(|s| s.result == ScenarioResult::Fail) {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// Builds `packages` and syncs the venv, unless the flags say not to. `--ci` asks both tools for plain output.
fn build_and_sync(repo: &Path, args: &ParityArgs, packages: &[&str], needs_python: bool) -> Result<()> {
    let color: &[&str] = if args.ci { &["--color", "never"] } else { &[] };
    if !args.no_build {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut build = vec!["build"];
        for package in packages {
            build.extend(["-p", package]);
        }
        build.extend(color);
        run_tool(repo, &cargo, &build)?;
    }
    if needs_python && !args.no_sync {
        run_tool(repo, "uv".as_ref(), &[&["sync", "--frozen", "--quiet"], color].concat())?;
    }
    Ok(())
}

fn run_tool(repo: &Path, program: &std::ffi::OsStr, args: &[&str]) -> Result<()> {
    let shown = format!("{} {}", program.to_string_lossy(), args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(repo)
        .status()
        .with_context(|| format!("running {shown}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(HarnessError(format!("{shown} failed ({status})")))
    }
}

/// Turns the positional arguments into scenario files: a name is a stem in `parity/scenarios/`, anything with a path
/// separator or a `.toml` extension is a path. No names (or `--all`) means every scenario.
fn resolve_scenarios(repo: &Path, args: &ParityArgs) -> Result<Vec<PathBuf>> {
    let dir = repo.join("parity/scenarios");
    let mut files = Vec::new();
    if args.all || args.scenarios.is_empty() {
        let entries = std::fs::read_dir(&dir).with_context(|| format!("listing {}", dir.display()))?;
        for entry in entries {
            let path = entry.with_context(|| format!("listing {}", dir.display()))?.path();
            if path.extension().is_some_and(|e| e == "toml") {
                files.push(path);
            }
        }
        files.sort();
    }
    for name in &args.scenarios {
        let as_path = Path::new(name);
        let file = if as_path.extension().is_some_and(|e| e == "toml") || as_path.components().count() > 1 {
            as_path.to_owned()
        } else {
            dir.join(format!("{name}.toml"))
        };
        if !file.is_file() {
            return Err(HarnessError(format!(
                "no scenario {name:?} (looked for {})",
                file.display()
            )));
        }
        if !files.contains(&file) {
            files.push(file);
        }
    }
    Ok(files)
}

pub(super) struct RunOptions<'a> {
    pub(super) apps: &'a [App],
    /// Overrides the scenarios' sizes when not empty.
    pub(super) sizes: &'a [Size],
    pub(super) retries: u32,
    pub(super) keep_home: bool,
    /// Cap the waits of runs that can't fail the gate at [`UNGATED_WAIT_CAP`].
    pub(super) cap_ungated_waits: bool,
    pub(super) themes: &'a Themes,
}

/// What every scenario of a run shares.
struct RunContext<'a> {
    repo: &'a Path,
    paths: &'a Paths,
    out: &'a Path,
    options: &'a RunOptions<'a>,
    freshness: &'a Freshnesses,
}

/// A scenario's summary and its structural mismatches.
type ScenarioRun = (ScenarioSummary, Vec<Finding>);

/// Runs the scenarios on up to `jobs` threads and returns their results in input order.
fn run_all(units: &[(&Scenario, Option<PythonSource>)], context: &RunContext<'_>, jobs: usize) -> Vec<ScenarioRun> {
    parallel_map(units, jobs, |&(scenario, python)| {
        run_scenario(scenario, python, context)
    })
    .into_iter()
    .zip(units)
    .map(|(result, &(scenario, _))| {
        result.unwrap_or_else(|| {
            let summary = ScenarioSummary {
                error: Some("the scenario's worker thread panicked".into()),
                ..empty_summary(scenario)
            };
            (summary, Vec::new())
        })
    })
    .collect()
}

/// Calls `f` on every item from up to `jobs` threads, taking items in order, and returns the results in input
/// order. `None` for an item whose call panicked.
pub(super) fn parallel_map<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<Option<R>> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(items.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = f(item);
                    *results[i].lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
                }
            });
        }
    });
    results
        .into_iter()
        .map(|slot| slot.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner))
        .collect()
}

fn empty_summary(scenario: &Scenario) -> ScenarioSummary {
    ScenarioSummary {
        name: scenario.name.clone(),
        description: scenario.description.clone(),
        gate: scenario.gate,
        parity_rows: scenario.parity_rows.clone(),
        result: ScenarioResult::Fail,
        sizes: Vec::new(),
        error: None,
    }
}

fn run_scenario(scenario: &Scenario, python: Option<PythonSource>, context: &RunContext<'_>) -> ScenarioRun {
    let RunContext {
        repo,
        paths,
        out,
        options,
        freshness,
    } = *context;
    let mut summary = empty_summary(scenario);
    let mut findings = Vec::new();
    let dir = out.join(&scenario.name);
    let sizes = if options.sizes.is_empty() {
        &scenario.sizes[..]
    } else {
        options.sizes
    };
    let apps: Vec<App> = scenario
        .apps
        .iter()
        .copied()
        .filter(|a| options.apps.contains(a))
        .collect();
    let result = (|| -> Result<()> {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("wiping {}", dir.display()))?;
        }
        let theme = options.themes.colors(&check::scenario_theme(scenario, &paths.roots)?);
        for &size in sizes {
            let mut statuses = BTreeMap::new();
            let mut drift = Vec::new();
            for &app in &apps {
                let app_out = dir.join(size.to_string()).join(app.name());
                let reference_dir = record::reference_dir(repo, scenario, size);
                let status = match (app, python) {
                    (App::Python, Some(PythonSource::Recorded)) => reference::materialize(&reference_dir, &app_out)?,
                    _ => run_app(scenario, app, size, paths, &app_out, options)?,
                };
                println!("{} {size} {app}: {}", scenario.name, status.describe());
                if app == App::Python && python == Some(PythonSource::Live) {
                    match freshness.get(&(scenario.name.clone(), size)) {
                        Some(Freshness::Fresh) => {
                            let live = reference::read_run(scenario, &status, &app_out, reference::LIVE_LABEL)?;
                            let stored = reference::load(&reference_dir, scenario)?;
                            drift = reference::drift(scenario, &theme, &live, &stored.side);
                        }
                        state => println!(
                            "{} {size}: reference {}, drift not checked",
                            scenario.name,
                            match state {
                                Some(Freshness::Stale(_)) => "stale",
                                Some(Freshness::Invalid(_)) => "invalid",
                                _ => "missing",
                            }
                        ),
                    }
                }
                for miss in &status.misses {
                    println!(
                        "  miss: {} {}{}",
                        miss.method.as_deref().unwrap_or("?"),
                        miss.path.as_deref().unwrap_or("?"),
                        miss.query.as_ref().map(|q| format!(" {q:?}")).unwrap_or_default()
                    );
                }
                for error in &status.errors {
                    println!("  fake-gh error: {}", error.error.as_deref().unwrap_or_default());
                }
                statuses.insert(app, status);
            }
            let (captures, found) =
                check::check_size(scenario, &dir.join(size.to_string()), &statuses, &theme, &drift)?;
            findings.extend(found);
            summary.sizes.push(SizeSummary {
                size: size.to_string(),
                apps: statuses,
                captures,
            });
        }
        Ok(())
    })();

    summary.result = ScenarioResult::from_statuses(scenario.gate, &summary.sizes);
    if let Err(e) = result {
        eprintln!("{}: {e}", scenario.name);
        summary.error = Some(e.to_string());
        summary.result = ScenarioResult::Fail;
    }
    (summary, findings)
}

/// The most a condition wait may take in a run that can't fail the gate, under `--ci`. Such a run is usually a Rust
/// side that can't draw what it waits for yet, so waiting out the scenario's `timeout_ms` (15 s or more) only costs
/// CI time. The app is up within a second, so a ready step finishes well inside this.
const UNGATED_WAIT_CAP: Duration = Duration::from_secs(3);

/// `--jobs`, or the CPU count up to [`DEFAULT_MAX_JOBS`].
fn jobs(arg: Option<u16>) -> usize {
    arg.map_or_else(
        || std::thread::available_parallelism().map_or(1, |n| n.get().min(DEFAULT_MAX_JOBS)),
        usize::from,
    )
}

/// More parallel runs than this start to slow each app enough to risk timeouts; 4 is what CI runners have.
const DEFAULT_MAX_JOBS: usize = 4;

/// Runs one app at one size, retrying a failure up to `options.retries` times. A run whose failure can't fail the
/// gate (Rust in an ungated scenario) isn't retried: the retry would only cost time.
pub(super) fn run_app(
    scenario: &Scenario,
    app: App,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
) -> Result<AppRunStatus> {
    let gates = ci::gates(scenario.gate, app);
    let retries = if gates { options.retries } else { 0 };
    let mut attempt = 0;
    loop {
        if out.exists() {
            std::fs::remove_dir_all(out).with_context(|| format!("wiping {}", out.display()))?;
        }
        std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
        let mut env = env::prepare(scenario, app, size, paths, out, options.keep_home)?;
        if options.cap_ungated_waits && !gates {
            env.timing.wait_cap = Some(UNGATED_WAIT_CAP);
        }
        let mut status = runner::run(env, &scenario.steps, out)?;
        if status.passed() || attempt >= retries {
            if status.passed() && attempt > 0 {
                status.flaky = true;
                runner::write_status(&status, out)?;
            }
            return Ok(status);
        }
        attempt += 1;
        println!(
            "{} {size} {app}: {}; retrying ({attempt}/{})",
            scenario.name,
            status.describe(),
            retries
        );
    }
}

/// `out` against the current directory. The apps under test run from their own work dir, so a relative `--out` would
/// point fake-gh at a log path that doesn't exist from there.
fn absolute(out: &Path) -> Result<PathBuf> {
    std::path::absolute(out).with_context(|| format!("resolving {}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: ParityArgs,
    }

    fn parse(argv: &[&str]) -> std::result::Result<ParityArgs, clap::Error> {
        Wrapper::try_parse_from(std::iter::once("parity").chain(argv.iter().copied())).map(|w| w.args)
    }

    #[test]
    fn parses_every_flag() {
        let args = parse(&[
            "startup",
            "--python-only",
            "--size",
            "100x30",
            "--size",
            "80x24",
            "--out",
            "x",
            "--keep-home",
            "--no-build",
            "--no-sync",
            "--python",
            "/usr/bin/python3",
            "--retries",
            "2",
            "--jobs",
            "4",
            "--ci",
        ])
        .unwrap();
        assert_eq!(args.scenarios, ["startup"]);
        assert!(args.python_only && args.keep_home && args.no_build && args.no_sync && args.ci);
        assert_eq!(args.sizes, [Size { cols: 100, rows: 30 }, Size { cols: 80, rows: 24 }]);
        assert_eq!((args.retries, args.jobs), (Some(2), Some(4)));
        let ci = parse(&["--all", "--ci", "--no-sync"]).unwrap();
        assert!(ci.all && ci.ci && ci.no_sync);
        assert_eq!(ci.retries, None);
    }

    #[test]
    fn rejects_conflicting_and_invalid_flags() {
        assert!(parse(&["--python-only", "--rust-only"]).is_err());
        assert!(parse(&["--size", "100"]).is_err());
        assert!(parse(&["--jobs", "0"]).is_err());
    }

    #[test]
    fn check_references_takes_scenarios_and_ci_but_no_run_flags() {
        let args = parse(&["startup", "--check-references", "--ci"]).unwrap();
        assert!(args.check_references && args.ci);
        assert_eq!(args.scenarios, ["startup"]);
        assert!(parse(&["--check-references", "--all"]).unwrap().all);
        for flag in [
            &["--python-only"][..],
            &["--rust-only"],
            &["--size", "100x30"],
            &["--validate"],
            &["--out", "x"],
            &["--keep-home"],
            &["--no-build"],
            &["--no-sync"],
            &["--python", "python3"],
            &["--retries", "1"],
            &["--jobs", "2"],
        ] {
            let command = [&["--check-references"][..], flag].concat();
            let err = parse(&command).map(|_| ()).expect_err(&command.join(" "));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{command:?}: {err}"
            );
        }
    }

    #[test]
    fn record_takes_its_flags_but_no_run_flags() {
        let args = parse(&[
            "startup",
            "--record",
            "--force",
            "--jobs",
            "2",
            "--python",
            "python3",
            "--no-sync",
            "--no-build",
            "--out",
            "x",
            "--keep-home",
        ])
        .unwrap();
        assert!(args.record && args.force && args.no_sync && args.no_build && args.keep_home);
        assert_eq!(args.scenarios, ["startup"]);
        assert!(parse(&["--record", "--all"]).unwrap().all);
        for flag in [
            &["--python-only"][..],
            &["--rust-only"],
            &["--size", "100x30"],
            &["--retries", "1"],
            &["--ci"],
            &["--validate"],
            &["--check-references"],
        ] {
            let command = [&["--record"][..], flag].concat();
            let err = parse(&command).map(|_| ()).expect_err(&command.join(" "));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{command:?}: {err}"
            );
        }
        let err = parse(&["--force"]).map(|_| ()).expect_err("--force alone");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument, "{err}");
    }

    /// Scenarios `a`, `b` and `c` that run both apps, and `r` that runs only Rust.
    fn plan_scenarios() -> Vec<Scenario> {
        let repo = repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
        };
        let load = |name: &str, source: &str| Scenario::parse(Path::new(&format!("{name}.toml")), source, &roots);
        let mut scenarios: Vec<Scenario> = ["a", "b", "c"]
            .into_iter()
            .map(|n| load(n, "version = 1\n").unwrap())
            .collect();
        scenarios.push(load("r", "version = 1\napps = [\"rust\"]\n").unwrap());
        scenarios
    }

    fn plan(argv: &[&str]) -> Result<Plan> {
        Plan::new(&parse(argv).unwrap(), &plan_scenarios())
    }

    #[test]
    fn plans_where_python_comes_from() {
        use PythonSource::{Live, Recorded};
        use report::AppSource;

        let default = plan(&[]).unwrap();
        assert_eq!(default.apps, App::ALL);
        assert_eq!(default.python, [Some(Recorded), Some(Recorded), Some(Recorded), None]);
        assert_eq!(default.python_meta, AppSource::Recorded);

        let live = plan(&["--live-python"]).unwrap();
        assert_eq!(live.apps, App::ALL);
        assert_eq!(live.python, [Some(Live), Some(Live), Some(Live), None]);
        assert_eq!(live.python_meta, AppSource::Live);

        let some = plan(&["--live-python=a"]).unwrap();
        assert_eq!(some.python, [Some(Live), Some(Recorded), Some(Recorded), None]);
        assert_eq!(some.python_meta, AppSource::LiveFor(vec!["a".into()]));
        let two = plan(&["--live-python=c,a"]).unwrap();
        assert_eq!(two.python_meta, AppSource::LiveFor(vec!["a".into(), "c".into()]));
        // Naming every scenario that runs Python is all of them.
        assert_eq!(plan(&["--live-python=a,b,c,r"]).unwrap().python_meta, AppSource::Live);

        let python_only = plan(&["--python-only"]).unwrap();
        assert_eq!(python_only.apps, [App::Python]);
        assert_eq!(python_only.python, [Some(Live), Some(Live), Some(Live), None]);
        assert_eq!(python_only.python_meta, AppSource::Live);
        assert_eq!(python_only.rust_meta(), AppSource::NotRun);

        let rust_only = plan(&["--rust-only"]).unwrap();
        assert_eq!(rust_only.apps, [App::Rust]);
        assert_eq!(rust_only.python, [None, None, None, None]);
        assert_eq!(rust_only.python_meta, AppSource::NotRun);
        assert_eq!(rust_only.rust_meta(), AppSource::Live);

        let err = plan(&["--live-python=a,x"]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--live-python names \"x\", which isn't a selected scenario"
        );
    }

    #[test]
    fn live_python_takes_names_only_after_an_equals_sign() {
        let args = parse(&["--live-python", "startup"]).unwrap();
        assert_eq!(args.live_python, Some(Vec::new()));
        assert_eq!(args.scenarios, ["startup"]);
        let args = parse(&["--live-python=a,b"]).unwrap();
        assert_eq!(args.live_python, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(parse(&[]).unwrap().live_python, None);
        for command in [
            &["--live-python", "--rust-only"][..],
            &["--live-python=a", "--rust-only"],
            &["--live-python", "--record"],
            &["--live-python", "--check-references"],
        ] {
            let err = parse(command).map(|_| ()).expect_err(&command.join(" "));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{command:?}: {err}"
            );
        }
    }

    #[test]
    fn a_run_reads_only_the_references_of_recorded_python_sides() {
        let scenarios = plan_scenarios();
        let size = Size { cols: 160, rows: 48 };
        let other = Size { cols: 100, rows: 30 };
        let mut freshness = Freshnesses::new();
        freshness.insert(("a".into(), size), Freshness::Stale(vec!["scenario"]));
        freshness.insert(("b".into(), size), Freshness::Fresh);
        freshness.insert(("c".into(), size), Freshness::Missing);
        let problems = |argv: &[&str], sizes: &[Size]| -> Vec<String> {
            let plan = plan(argv).unwrap();
            problems_reading(&scenarios, &plan, sizes, &freshness)
                .iter()
                .map(ToString::to_string)
                .collect()
        };
        assert_eq!(
            problems(&[], &[]),
            [
                "stale reference: a/160x48: scenario changed; run cargo xtask parity --record a",
                "missing reference: c/160x48; run cargo xtask parity --record c",
            ]
        );
        assert_eq!(
            problems(&["--live-python=a"], &[]),
            ["missing reference: c/160x48; run cargo xtask parity --record c"]
        );
        assert!(problems(&["--live-python"], &[]).is_empty());
        assert!(problems(&["--rust-only"], &[]).is_empty());
        // A size with no entry (one the scenario doesn't list) is missing.
        assert_eq!(problems(&[], &[other]).len(), 3);
    }

    #[test]
    fn reads_the_python_version() {
        assert_eq!(parse_python_version("Python 3.11.14\n"), "3.11.14");
        assert_eq!(parse_python_version("\nPython 3.14.7"), "3.14.7");
        assert_eq!(parse_python_version("echo_tui 1.0\n"), "echo_tui 1.0");
    }

    #[test]
    fn parallel_map_keeps_input_order() {
        let items: Vec<u32> = (0..20).collect();
        let doubled = parallel_map(&items, 4, |&n| n * 2);
        assert_eq!(doubled, items.iter().map(|n| Some(n * 2)).collect::<Vec<_>>());
        assert_eq!(parallel_map(&[] as &[u32], 4, |&n| n), Vec::<Option<u32>>::new());
    }

    #[test]
    fn reference_problems_annotate_their_scenario_file() {
        let repo = repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
        };
        let path = repo.join("parity/scenarios/startup.toml");
        let startup = Scenario::parse(&path, "version = 1\n", &roots).unwrap();
        let report = reference::Report {
            checked: 1,
            problems: vec![
                reference::Problem::Missing {
                    scenario: "startup".into(),
                    size: Size { cols: 160, rows: 48 },
                },
                reference::Problem::Orphaned {
                    path: "parity/references/old-name/".into(),
                    reason: "no such scenario".into(),
                    scenario: None,
                },
            ],
        };
        let lines: Vec<String> = reference_annotations(&repo, &[startup], &report)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "::error file=parity/scenarios/startup.toml::missing reference: startup/160x48; run cargo xtask \
                 parity --record startup",
                "::error::orphaned reference: parity/references/old-name/ (no such scenario); cargo xtask parity \
                 --record deletes it",
            ]
        );
    }

    #[test]
    fn rejects_two_scenarios_with_one_name() {
        let repo = repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
        };
        let load = |path: &str| Scenario::parse(Path::new(path), "version = 1\n", &roots).unwrap();
        let scenarios = [
            load("parity/scenarios/startup.toml"),
            load("other/startup.toml"),
            load("other/b.toml"),
        ];
        let problems = duplicate_names(&scenarios);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].0, Path::new("other/startup.toml"), "{problems:?}");
        assert!(problems[0].1.starts_with("scenario name \"startup\""), "{problems:?}");
        assert!(duplicate_names(&scenarios[1..]).is_empty());
    }

    #[test]
    fn makes_out_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute(Path::new("reports/x")).unwrap(), cwd.join("reports/x"));
        let abs = cwd.join("already");
        assert_eq!(absolute(&abs).unwrap(), abs);
    }

    #[test]
    fn resolves_names_and_paths() {
        let repo = repo_root();
        let args = parse(&["startup"]).unwrap();
        assert_eq!(
            resolve_scenarios(&repo, &args).unwrap(),
            [repo.join("parity/scenarios/startup.toml")]
        );
        let all = resolve_scenarios(&repo, &parse(&[]).unwrap()).unwrap();
        assert!(all.contains(&repo.join("parity/scenarios/startup.toml")));
        assert!(resolve_scenarios(&repo, &parse(&["no-such-scenario"]).unwrap()).is_err());
    }
}
