//! The `cargo xtask parity` command: resolve and validate scenarios, build, run every scenario × size, check each run
//! against its `[expect]` and its snapshot, and write `summary.json` and the report.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::check;
use super::ci::{self, Finding};
use super::env::{self, Paths};
use super::focus::FocusColors;
use super::read_only::ReadOnlyConfigDir;
use super::report;
use super::runner::{self, AppRunStatus};
use super::scenario::{Roots, Scenario, Size};
use super::snapshot;
use super::structure::ThemeColors;
use super::summary::{SUMMARY_VERSION, ScenarioResult, ScenarioSummary, SizeSummary, Summary};
use super::theme;
use super::{Context, HarnessError, Result};

/// Exit code: every scenario passed.
pub const EXIT_OK: u8 = 0;
/// Exit code: a scenario failed.
pub const EXIT_FAILED: u8 = 1;
/// Exit code: a scenario file is invalid, or the harness itself failed.
pub const EXIT_HARNESS: u8 = 2;

/// Run parity scenarios against the app and check its screens against their expectations and snapshots.
#[derive(Debug, Clone, clap::Args)]
#[allow(clippy::struct_excessive_bools, reason = "each bool is a command-line switch")]
pub struct ParityArgs {
    /// Scenario names (file stems in parity/scenarios/) or paths to scenario files. None means --all.
    #[arg(value_name = "SCENARIO")]
    pub scenarios: Vec<String>,
    /// Run every scenario in parity/scenarios/.
    #[arg(long)]
    pub all: bool,
    /// Run at this size instead of the scenarios' sizes. Repeatable. A size the scenario doesn't list has no snapshot
    /// to compare.
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
    /// Re-run a failed run up to N times; a run that passes on retry is marked flaky [default: 0, or 1 with --ci].
    #[arg(long, value_name = "N")]
    pub retries: Option<u32>,
    /// Run up to N scenarios in parallel. Sizes within a scenario always run one after another [default: the CPU
    /// count, at most 4].
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub jobs: Option<u16>,
    /// Running in CI: retry failed runs once, print GitHub Actions `::error`/`::warning` annotations, and ask cargo
    /// for plain (uncolored) output.
    #[arg(long)]
    pub ci: bool,
    /// Check every scenario file (or the ones named) without running anything.
    #[arg(long)]
    pub validate: bool,
    /// Accept the snapshots in parity/snapshots/ (all, or the named scenarios'): run the app twice per size, with no
    /// retries, and write a snapshot only when both runs pass and agree. Builds lazy-github and fake-gh. Run dumps go
    /// to --out [default: parity/reports/accept, wiped first].
    #[arg(long, conflicts_with_all = ["sizes", "retries", "ci", "validate"])]
    pub accept: bool,
    /// Read fixture sets from this directory instead of parity/fixtures/. For tests that need a modified set.
    #[arg(long, value_name = "DIR", hide = true)]
    pub fixtures_dir: Option<PathBuf>,
}

/// The repository root: xtask lives one level below it.
pub fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_owned()
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
        seeds: repo.join("parity/seeds"),
    };

    let files = resolve_scenarios(&repo, args)?;
    let Some(scenarios) = load_scenarios(&repo, &files, &roots, args.ci) else {
        return Ok(EXIT_HARNESS);
    };
    if args.validate {
        println!("parity: {} scenario file(s) valid", scenarios.len());
        return Ok(EXIT_OK);
    }
    if args.accept {
        return accept_snapshots(&repo, &scenarios, roots, args);
    }
    run_scenarios(&repo, &scenarios, &roots, args, None)
}

/// Runs `scenarios` (already loaded and valid) under `repo` as `args` say: builds, runs every scenario × size, checks
/// each run, and writes `summary.json` and the report. `paths` overrides the binaries the flags would pick.
pub fn run_scenarios(
    repo: &Path,
    scenarios: &[Scenario],
    roots: &Roots,
    args: &ParityArgs,
    paths: Option<Paths>,
) -> Result<u8> {
    build(repo, args, &["lazy-github", "fake-gh"])?;
    let paths = match paths {
        Some(paths) => paths,
        None => self::paths(roots.clone())?,
    };
    let out = report_dir(repo, args, "latest")?;

    let options = RunOptions {
        sizes: &args.sizes,
        retries: args.retries.unwrap_or(u32::from(args.ci)),
        keep_home: args.keep_home,
    };
    let context = RunContext {
        repo,
        paths: &paths,
        out: &out,
        options: &options,
    };
    let runs = run_all(scenarios, &context, jobs(args.jobs));
    let (summaries, annotations) = split_runs(runs, scenarios, repo, args.ci);

    let summary = Summary {
        version: SUMMARY_VERSION,
        scenarios: summaries,
    };
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(&summary).expect("summary serializes");
    std::fs::write(out.join("summary.json"), json + "\n").context("writing summary.json")?;
    let meta = report::RunMeta::current(repo);
    // A reporting failure is printed but keeps the run's exit code, so it can never mask a result.
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

/// The binaries runs need. `cargo xtask` runs target/<profile>/xtask, next to the binaries `cargo build` produces.
fn paths(roots: Roots) -> Result<Paths> {
    let exe = std::env::current_exe().context("finding the xtask binary")?;
    let profile_dir = exe
        .parent()
        .ok_or_else(|| HarnessError("xtask binary has no parent dir".into()))?;
    let paths = Paths {
        fake_gh: profile_dir.join(format!("fake-gh{}", std::env::consts::EXE_SUFFIX)),
        app: profile_dir.join(format!("lazy-github{}", std::env::consts::EXE_SUFFIX)),
        roots,
    };
    for (binary, package) in [(&paths.fake_gh, "fake-gh"), (&paths.app, "lazy-github")] {
        if !binary.is_file() {
            return Err(HarnessError(format!(
                "{} doesn't exist; build it with `cargo build -p {package}` or drop --no-build",
                binary.display()
            )));
        }
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

/// `--accept`: builds the app and fake-gh, accepts, deletes orphans and prints a count. Exit 0 when every selected
/// snapshot was written or unchanged, 1 when some weren't.
fn accept_snapshots(repo: &Path, scenarios: &[Scenario], roots: Roots, args: &ParityArgs) -> Result<u8> {
    build(repo, args, &["lazy-github", "fake-gh"])?;
    let paths = paths(roots)?;
    let out = report_dir(repo, args, "accept")?;
    let options = snapshot::AcceptOptions {
        all: args.all || args.scenarios.is_empty(),
        jobs: jobs(args.jobs),
        keep_home: args.keep_home,
    };
    let report = snapshot::accept(repo, scenarios, &paths, &out, &options)?;
    println!();
    println!("parity: {}", report.counts());
    println!("run dumps: {}", out.display());
    Ok(if report.all_accepted() { EXIT_OK } else { EXIT_FAILED })
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

/// Builds `packages`, unless `--no-build`. `--ci` asks cargo for plain output.
fn build(repo: &Path, args: &ParityArgs, packages: &[&str]) -> Result<()> {
    if args.no_build {
        return Ok(());
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = vec!["build"];
    for package in packages {
        build.extend(["-p", package]);
    }
    if args.ci {
        build.extend(["--color", "never"]);
    }
    let shown = format!("{} {}", cargo.to_string_lossy(), build.join(" "));
    let status = Command::new(&cargo)
        .args(&build)
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
    /// Overrides the scenarios' sizes when not empty.
    pub(super) sizes: &'a [Size],
    pub(super) retries: u32,
    pub(super) keep_home: bool,
}

/// What every scenario of a run shares.
struct RunContext<'a> {
    repo: &'a Path,
    paths: &'a Paths,
    out: &'a Path,
    options: &'a RunOptions<'a>,
}

/// A scenario's summary and its mismatches.
type ScenarioRun = (ScenarioSummary, Vec<Finding>);

/// Runs the scenarios on up to `jobs` threads and returns their results in input order.
fn run_all(scenarios: &[Scenario], context: &RunContext<'_>, jobs: usize) -> Vec<ScenarioRun> {
    parallel_map(scenarios, jobs, |scenario| run_scenario(scenario, context))
        .into_iter()
        .zip(scenarios)
        .map(|(result, scenario)| {
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
        parity_rows: scenario.parity_rows.clone(),
        result: ScenarioResult::Fail,
        sizes: Vec::new(),
        error: None,
    }
}

fn run_scenario(scenario: &Scenario, context: &RunContext<'_>) -> ScenarioRun {
    let RunContext {
        repo,
        paths,
        out,
        options,
    } = *context;
    let mut summary = empty_summary(scenario);
    let mut findings = Vec::new();
    let dir = out.join(&scenario.name);
    let sizes = if options.sizes.is_empty() {
        &scenario.sizes[..]
    } else {
        options.sizes
    };
    let result = (|| -> Result<()> {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("wiping {}", dir.display()))?;
        }
        let colors = theme::scenario_colors(scenario, &paths.roots)?;
        for &size in sizes {
            let run_out = dir.join(size.to_string());
            let run = run_app(scenario, size, paths, &run_out, options, &colors.focus)?;
            let status = run.status;
            println!("{} {size}: {}", scenario.name, status.describe());
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
            let snapshot = check_snapshot(repo, scenario, size, &run_out, &status, &colors.theme, &run.run_dirs)?;
            let (captures, found) = check::check_size(scenario, &run_out, &colors.theme, snapshot.as_deref())?;
            findings.extend(found);
            summary.sizes.push(SizeSummary {
                size: size.to_string(),
                status,
                captures,
            });
        }
        Ok(())
    })();

    summary.result = ScenarioResult::from_sizes(&summary.sizes);
    if let Err(e) = result {
        eprintln!("{}: {e}", scenario.name);
        summary.error = Some(e.to_string());
        summary.result = ScenarioResult::Fail;
    }
    (summary, findings)
}

/// Compares the run at `size` in `out` with its committed snapshot, when there's one to compare: the run passed on its
/// own (status and `[expect]`), and `size` is one of the scenario's. A run that failed reports that failure, not
/// snapshot mismatches for captures it never reached; a size the scenario doesn't list (`--size`) has no snapshot.
/// `None` when nothing was compared.
fn check_snapshot(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    out: &Path,
    status: &AppRunStatus,
    theme: &ThemeColors,
    run_dirs: &[String],
) -> Result<Option<Vec<snapshot::SnapshotMismatch>>> {
    if !status.passed() {
        return Ok(None);
    }
    if !scenario.sizes.contains(&size) {
        println!(
            "{} {size}: not a size of the scenario, snapshot not checked",
            scenario.name
        );
        return Ok(None);
    }
    if snapshot::expect_failure(scenario, out, theme)?.is_some() {
        return Ok(None);
    }
    let captured = snapshot::capture_run(scenario, out, theme, run_dirs)?;
    snapshot::check_run(repo, scenario, size, &captured, out).map(Some)
}

/// `--jobs`, or the CPU count up to [`DEFAULT_MAX_JOBS`].
fn jobs(arg: Option<u16>) -> usize {
    arg.map_or_else(
        || std::thread::available_parallelism().map_or(1, |n| n.get().min(DEFAULT_MAX_JOBS)),
        usize::from,
    )
}

/// More parallel runs than this start to slow each app enough to risk timeouts; 4 is what CI runners have.
const DEFAULT_MAX_JOBS: usize = 4;

/// One finished run.
pub(super) struct AppRun {
    pub(super) status: AppRunStatus,
    /// The last attempt's run directory, as made and in canonical form ([`snapshot::run_dir_forms`]), for masking.
    pub(super) run_dirs: Vec<String>,
}

/// Runs the app at one size, retrying a failure up to `options.retries` times. `focus` holds the scenario theme's
/// colors, for `wait.focused`.
pub(super) fn run_app(
    scenario: &Scenario,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
    focus: &FocusColors,
) -> Result<AppRun> {
    let mut attempt = 0;
    loop {
        if out.exists() {
            std::fs::remove_dir_all(out).with_context(|| format!("wiping {}", out.display()))?;
        }
        std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
        let mut env = env::prepare(scenario, size, paths, out, options.keep_home)?;
        let run_dirs = env
            .run_dir
            .as_ref()
            .map(|dir| snapshot::run_dir_forms(dir.path()))
            .unwrap_or_default();
        env.focus = focus.clone();
        // Held until the run (and its `--keep-home` copy) is done; dropping it cleans up the run dir.
        let read_only = match &env.run_dir {
            Some(run_dir) if scenario.env.config_dir_read_only => Some(ReadOnlyConfigDir::new(run_dir.path())?),
            _ => None,
        };
        let status = runner::run(env, &scenario.steps, out);
        drop(read_only);
        let mut status = status?;
        if status.passed() || attempt >= options.retries {
            if status.passed() && attempt > 0 {
                status.flaky = true;
                runner::write_status(&status, out)?;
            }
            return Ok(AppRun { status, run_dirs });
        }
        attempt += 1;
        println!(
            "{} {size}: {}; retrying ({attempt}/{})",
            scenario.name,
            status.describe(),
            options.retries
        );
    }
}

/// `out` against the current directory. The app runs from its own work dir, so a relative `--out` would point fake-gh
/// at a log path that doesn't exist from there.
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
            "--size",
            "100x30",
            "--size",
            "80x24",
            "--out",
            "x",
            "--keep-home",
            "--no-build",
            "--retries",
            "2",
            "--jobs",
            "4",
            "--ci",
        ])
        .unwrap();
        assert_eq!(args.scenarios, ["startup"]);
        assert!(args.keep_home && args.no_build && args.ci);
        assert_eq!(args.sizes, [Size { cols: 100, rows: 30 }, Size { cols: 80, rows: 24 }]);
        assert_eq!((args.retries, args.jobs), (Some(2), Some(4)));
        let ci = parse(&["--all", "--ci"]).unwrap();
        assert!(ci.all && ci.ci);
        assert_eq!(ci.retries, None);
    }

    #[test]
    fn rejects_invalid_flags() {
        assert!(parse(&["--size", "100"]).is_err());
        assert!(parse(&["--jobs", "0"]).is_err());
    }

    #[test]
    fn the_python_flags_are_gone() {
        for flag in [
            &["--python-only"][..],
            &["--rust-only"],
            &["--live-python"],
            &["--live-python=startup"],
            &["--live-python-since", "HEAD"],
            &["--plan"],
            &["--python", "python3"],
            &["--no-sync"],
            &["--record"],
            &["--force"],
            &["--check-references"],
        ] {
            let err = parse(flag).map(|_| ()).expect_err(&flag.join(" "));
            assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument, "{flag:?}: {err}");
        }
    }

    #[test]
    fn accept_takes_its_flags_but_no_run_flags() {
        let args = parse(&[
            "startup",
            "--accept",
            "--jobs",
            "2",
            "--no-build",
            "--out",
            "x",
            "--keep-home",
        ])
        .unwrap();
        assert!(args.accept && args.no_build && args.keep_home);
        assert_eq!(args.scenarios, ["startup"]);
        assert!(parse(&["--accept", "--all"]).unwrap().all);
        for flag in [
            &["--size", "100x30"][..],
            &["--retries", "1"],
            &["--ci"],
            &["--validate"],
        ] {
            let command = [&["--accept"][..], flag].concat();
            let err = parse(&command).map(|_| ()).expect_err(&command.join(" "));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{command:?}: {err}"
            );
        }
    }

    #[test]
    fn parallel_map_keeps_input_order() {
        let items: Vec<u32> = (0..20).collect();
        let doubled = parallel_map(&items, 4, |&n| n * 2);
        assert_eq!(doubled, items.iter().map(|n| Some(n * 2)).collect::<Vec<_>>());
        assert_eq!(parallel_map(&[] as &[u32], 4, |&n| n), Vec::<Option<u32>>::new());
    }

    #[test]
    fn rejects_two_scenarios_with_one_name() {
        let repo = repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
            seeds: repo.join("parity/seeds"),
        };
        let load = |path: &str| Scenario::parse(Path::new(path), "version = 2\n", &roots).unwrap();
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
