//! The `cargo xtask parity` command: resolve and validate scenarios, build and sync, run every scenario × size ×
//! app, and write `summary.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::env::{self, Paths};
use super::report;
use super::runner::{self, AppRunStatus};
use super::scenario::{App, Roots, Scenario, Size};
use super::summary::{CaptureSummary, SUMMARY_VERSION, ScenarioResult, ScenarioSummary, SizeSummary, Summary};
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
    /// Re-run a failed app run up to N times; a run that passes on retry is marked flaky.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub retries: u32,
    /// Run up to N scenarios in parallel. Sizes and apps within a scenario always run one after another.
    #[arg(long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
    pub jobs: u16,
    /// Running in CI. Accepted now; the CI-specific output arrives with #70.
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
    let mut scenarios = Vec::new();
    let mut invalid = false;
    for file in &files {
        match Scenario::load(file, &roots) {
            Ok(s) => scenarios.push(s),
            Err(e) => {
                eprintln!("{e}");
                invalid = true;
            }
        }
    }
    for problem in duplicate_names(&scenarios) {
        eprintln!("{problem}");
        invalid = true;
    }
    if invalid {
        return Ok(EXIT_HARNESS);
    }
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

    if !args.no_build {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        run_tool(&repo, &cargo, &["build", "-p", "lazy-github", "-p", "fake-gh"])?;
    }
    if needs_python && !args.no_sync {
        run_tool(&repo, "uv".as_ref(), &["sync", "--frozen", "--quiet"])?;
    }

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
        out.clone()
    } else {
        let latest = repo.join("parity/reports/latest");
        if latest.exists() {
            std::fs::remove_dir_all(&latest).with_context(|| format!("wiping {}", latest.display()))?;
        }
        latest
    };
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;

    let options = RunOptions {
        apps: &requested,
        sizes: &args.sizes,
        retries: args.retries,
        keep_home: args.keep_home,
    };
    let summaries = run_all(&scenarios, &paths, &out, &options, usize::from(args.jobs));

    let summary = Summary {
        version: SUMMARY_VERSION,
        scenarios: summaries,
    };
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(&summary).expect("summary serializes");
    std::fs::write(out.join("summary.json"), json + "\n").context("writing summary.json")?;
    let meta = report::RunMeta::current(&repo, args.python_only, args.rust_only);
    report::write_reports(&out, &summary, &meta)?;

    println!();
    for s in &summary.scenarios {
        println!("{}: {}", s.name, s.result);
    }
    println!("report: {}", out.display());

    Ok(exit_code(&summary))
}

/// Each scenario owns `<out>/<name>/`, so two files with one stem in one run would wipe each other's results (and
/// race under `--jobs`). Returns one message per clash.
fn duplicate_names(scenarios: &[Scenario]) -> Vec<String> {
    let mut seen = BTreeMap::new();
    let mut problems = Vec::new();
    for s in scenarios {
        if let Some(first) = seen.insert(s.name.as_str(), &s.path) {
            problems.push(format!(
                "{}: scenario name {:?} is also used by {}; scenario names must be unique in a run",
                s.path.display(),
                s.name,
                first.display()
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
}

/// Runs the scenarios on up to `jobs` threads and returns their summaries in input order.
fn run_all(
    scenarios: &[Scenario],
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
    jobs: usize,
) -> Vec<ScenarioSummary> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<ScenarioSummary>>> = scenarios.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(scenarios.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(scenario) = scenarios.get(i) else { break };
                    let summary = run_scenario(scenario, paths, out, options);
                    *results[i].lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(summary);
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
                .unwrap_or_else(|| ScenarioSummary {
                    error: Some("the scenario's worker thread panicked".into()),
                    ..empty_summary(scenario)
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

fn run_scenario(scenario: &Scenario, paths: &Paths, out: &Path, options: &RunOptions<'_>) -> ScenarioSummary {
    let mut summary = empty_summary(scenario);
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
    let captures: Vec<CaptureSummary> = scenario
        .capture_names()
        .map(|name| CaptureSummary { name: name.to_owned() })
        .collect();

    let result = (|| -> Result<()> {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("wiping {}", dir.display()))?;
        }
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
            summary.sizes.push(SizeSummary {
                size: size.to_string(),
                apps: statuses,
                captures: captures.clone(),
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
    summary
}

/// Runs one app at one size, retrying a failure up to `options.retries` times.
fn run_app(
    scenario: &Scenario,
    app: App,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &RunOptions<'_>,
) -> Result<AppRunStatus> {
    let mut attempt = 0;
    loop {
        if out.exists() {
            std::fs::remove_dir_all(out).with_context(|| format!("wiping {}", out.display()))?;
        }
        std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
        let env = env::prepare(scenario, app, size, paths, out, options.keep_home)?;
        let mut status = runner::run(env, &scenario.steps, out)?;
        if status.passed() || attempt >= options.retries {
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
            options.retries
        );
    }
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
        assert_eq!((args.retries, args.jobs), (2, 4));
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
        assert!(
            problems[0].starts_with("other/startup.toml: scenario name \"startup\""),
            "{problems:?}"
        );
        assert!(duplicate_names(&scenarios[1..]).is_empty());
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
