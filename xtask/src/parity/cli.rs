//! The `cargo xtask parity` command: resolve and validate scenarios, build and sync, run every scenario × size ×
//! app, and write `summary.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::check;
use super::ci::{self, Finding};
use super::env::{self, Paths};
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
    /// Run only the Python app.
    #[arg(long, conflicts_with = "rust_only")]
    pub python_only: bool,
    /// Run only the Rust app.
    #[arg(long)]
    pub rust_only: bool,
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

    let requested: Vec<App> = if args.python_only {
        vec![App::Python]
    } else if args.rust_only {
        vec![App::Rust]
    } else {
        App::ALL.to_vec()
    };
    let needs_python = scenarios.iter().any(|s| s.apps.contains(&App::Python)) && requested.contains(&App::Python);

    build_and_sync(&repo, args, needs_python)?;

    // `cargo xtask` runs target/<profile>/xtask, next to the binaries the build above produced.
    let exe = std::env::current_exe().context("finding the xtask binary")?;
    let profile_dir = exe
        .parent()
        .ok_or_else(|| HarnessError("xtask binary has no parent dir".into()))?;
    let paths = Paths {
        fake_gh: profile_dir.join(format!("fake-gh{}", std::env::consts::EXE_SUFFIX)),
        rust_app: profile_dir.join(format!("lazy-github{}", std::env::consts::EXE_SUFFIX)),
        python: args.python.clone().unwrap_or_else(|| venv_python(&repo)),
        roots,
    };
    if !paths.fake_gh.is_file() {
        return Err(HarnessError(format!(
            "{} doesn't exist; build it with `cargo build -p fake-gh` or drop --no-build",
            paths.fake_gh.display()
        )));
    }

    let out = if let Some(out) = &args.out {
        absolute(out)?
    } else {
        let latest = repo.join("parity/reports/latest");
        if latest.exists() {
            std::fs::remove_dir_all(&latest).with_context(|| format!("wiping {}", latest.display()))?;
        }
        latest
    };
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let themes = Themes::load(&repo.join("parity/themes.toml"))?;

    let options = RunOptions {
        apps: &requested,
        sizes: &args.sizes,
        retries: args.retries.unwrap_or(u32::from(args.ci)),
        keep_home: args.keep_home,
        cap_ungated_waits: args.ci,
        themes: &themes,
    };
    let runs = run_all(&scenarios, &paths, &out, &options, jobs(args.jobs));
    let (summaries, annotations) = split_runs(runs, &scenarios, &repo, args.ci);

    let summary = Summary {
        version: SUMMARY_VERSION,
        scenarios: summaries,
    };
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(&summary).expect("summary serializes");
    std::fs::write(out.join("summary.json"), json + "\n").context("writing summary.json")?;
    let meta = report::RunMeta::current(&repo, args.python_only, args.rust_only);
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

/// Builds the binaries and syncs the venv, unless the flags say not to. `--ci` asks both tools for plain output.
fn build_and_sync(repo: &Path, args: &ParityArgs, needs_python: bool) -> Result<()> {
    let color: &[&str] = if args.ci { &["--color", "never"] } else { &[] };
    if !args.no_build {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let build = [&["build", "-p", "lazy-github", "-p", "fake-gh"], color].concat();
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

struct RunOptions<'a> {
    apps: &'a [App],
    /// Overrides the scenarios' sizes when not empty.
    sizes: &'a [Size],
    retries: u32,
    keep_home: bool,
    /// Cap the waits of runs that can't fail the gate at [`UNGATED_WAIT_CAP`].
    cap_ungated_waits: bool,
    themes: &'a Themes,
}

/// A scenario's summary and its structural mismatches.
type ScenarioRun = (ScenarioSummary, Vec<Finding>);

/// Runs the scenarios on up to `jobs` threads and returns their results in input order.
fn run_all(
    scenarios: &[Scenario],
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
    jobs: usize,
) -> Vec<ScenarioRun> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<ScenarioRun>>> = scenarios.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(scenarios.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(scenario) = scenarios.get(i) else { break };
                    let run = run_scenario(scenario, paths, out, options);
                    *results[i].lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(run);
                }
            });
        }
    });
    results
        .into_iter()
        .zip(scenarios)
        .map(|(slot, scenario)| {
            slot.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .unwrap_or_else(|| {
                    let summary = ScenarioSummary {
                        error: Some("the scenario's worker thread panicked".into()),
                        ..empty_summary(scenario)
                    };
                    (summary, Vec::new())
                })
        })
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

fn run_scenario(scenario: &Scenario, paths: &Paths, out: &Path, options: &RunOptions<'_>) -> ScenarioRun {
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
            for &app in &apps {
                let app_out = dir.join(size.to_string()).join(app.name());
                let status = run_app(scenario, app, size, paths, &app_out, options)?;
                println!("{} {size} {app}: {}", scenario.name, status.describe());
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
            let (captures, found) = check::check_size(scenario, &dir.join(size.to_string()), &statuses, &theme)?;
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
fn run_app(
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
