//! `cargo xtask perf`: the large-data timing tests in `lazy-github-tui` (`src/perf_tests.rs`, #325), in a release
//! build, one process per case so each case's peak RSS is its own. They're `#[ignore]`d, so the required CI job never
//! runs them; this does, locally or in the non-required "Perf" workflow.
//!
//! Each case prints a `PERF case=... slowest_ms=... peak_rss_mib=...` line; the summary table is made from those.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode};

/// The module the timing tests live in, and the prefix of their names.
const PREFIX: &str = "perf_tests::perf_";

#[derive(Debug, clap::Args)]
pub struct PerfArgs {
    /// Only the cases whose test name contains this, such as `diff` or `markdown`.
    #[arg(long)]
    pub case: Option<String>,
}

/// One case's `PERF` line, as the test printed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseSummary {
    pub case: String,
    pub frames: String,
    pub median_ms: String,
    pub slowest_ms: String,
    pub over_budget: String,
    pub peak_rss_mib: String,
}

/// The `PERF` line in a case's output, if it got as far as printing one.
pub fn parse_summary(output: &str) -> Option<CaseSummary> {
    let line = output.lines().find_map(|line| line.strip_prefix("PERF "))?;
    // `case=` comes first and may hold spaces; every later field is one `key=value` word.
    let rest = line.strip_prefix("case=")?;
    let frames_at = rest.find(" frames=")?;
    let case = rest[..frames_at].to_owned();
    let field = |key: &str| {
        rest[frames_at..]
            .split_whitespace()
            .find_map(|word| word.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_owned)
    };
    Some(CaseSummary {
        case,
        frames: field("frames")?,
        median_ms: field("median_ms")?,
        slowest_ms: field("slowest_ms")?,
        over_budget: field("over_budget")?,
        peak_rss_mib: field("peak_rss_mib")?,
    })
}

/// The timing tests' names, from the test binary's `--list` output.
pub fn parse_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .filter(|name| name.starts_with(PREFIX))
        .map(str::to_owned)
        .collect()
}

pub fn run(repo: &Path, args: &PerfArgs) -> ExitCode {
    match run_cases(repo, args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask perf: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cargo_test(repo: &Path, test_args: &[&str]) -> Command {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(repo)
        .args(["test", "--release", "--quiet", "-p", "lazy-github-tui", "--lib", "--"])
        .args(test_args);
    command
}

/// Runs every case, printing each one's output and then a table. `Ok(false)` when a case failed.
fn run_cases(repo: &Path, args: &PerfArgs) -> Result<bool, String> {
    let list = cargo_test(repo, &["--ignored", "--list", PREFIX])
        .output()
        .map_err(|e| format!("running cargo test: {e}"))?;
    if !list.status.success() {
        return Err(format!(
            "building the timing tests failed:\n{}",
            String::from_utf8_lossy(&list.stderr)
        ));
    }
    let names: Vec<String> = parse_list(&String::from_utf8_lossy(&list.stdout))
        .into_iter()
        .filter(|name| args.case.as_deref().is_none_or(|case| name.contains(case)))
        .collect();
    if names.is_empty() {
        return Err("no timing tests matched".to_owned());
    }
    let mut rows = Vec::new();
    let mut all_passed = true;
    for name in &names {
        println!("== {name}");
        let output = cargo_test(repo, &["--ignored", "--exact", name, "--nocapture"])
            .output()
            .map_err(|e| format!("running {name}: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        print!("{stdout}");
        if !output.status.success() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            all_passed = false;
        }
        rows.push((name.clone(), output.status.success(), parse_summary(&stdout)));
    }
    println!("{}", table(&rows));
    Ok(all_passed)
}

/// The summary, as a Markdown table, so it can go straight into a PR or a job summary.
pub fn table(rows: &[(String, bool, Option<CaseSummary>)]) -> String {
    let mut out = String::from(
        "| Case | Frames | Median ms | Slowest ms | Over 50 ms | Peak RSS MiB | Result |\n| --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for (name, passed, summary) in rows {
        let result = if *passed { "pass" } else { "FAIL" };
        let _ = match summary {
            Some(s) => writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {result} |",
                s.case, s.frames, s.median_ms, s.slowest_ms, s.over_budget, s.peak_rss_mib
            ),
            None => writeln!(out, "| {name} | | | | | | {result} |"),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_line_parses() {
        let output = "pr diff: 3 frames, slowest:\n  1.0 ms  press J\nPERF case=pr diff frames=3 median_ms=0.5 \
                      slowest_ms=1.000 over_budget=0 peak_rss_mib=12.5\n";
        assert_eq!(
            parse_summary(output),
            Some(CaseSummary {
                case: "pr diff".into(),
                frames: "3".into(),
                median_ms: "0.5".into(),
                slowest_ms: "1.000".into(),
                over_budget: "0".into(),
                peak_rss_mib: "12.5".into(),
            })
        );
        assert_eq!(parse_summary("no summary here"), None);
    }

    #[test]
    fn the_list_keeps_only_the_timing_tests() {
        let output = "perf_tests::perf_repos_1000: test\nperf_tests::perf_job_logs: test\nother::test_x: test\n\n2 \
                      tests, 0 benchmarks\n";
        assert_eq!(
            parse_list(output),
            ["perf_tests::perf_repos_1000", "perf_tests::perf_job_logs"]
        );
    }

    #[test]
    fn a_case_without_a_summary_still_gets_a_row() {
        let rows = vec![("perf_tests::perf_x".to_owned(), false, None)];
        assert!(table(&rows).contains("| perf_tests::perf_x | | | | | | FAIL |"));
    }
}
