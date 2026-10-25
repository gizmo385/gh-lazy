//! Parity fixture tools: `cargo xtask record-fixtures` and `cargo xtask check-fixtures`. See `parity/README.md`,
//! "Recording fixtures".

pub mod check;
pub mod recipe;
pub mod record;
pub mod scrub;
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use record::{ChangeKind, GhCli, Mode, Options, describe_key};

/// `cargo xtask record-fixtures`: record a fixture set from the real `gh`. Never run in CI.
#[derive(Debug, clap::Args)]
pub struct RecordArgs {
    /// The fixture set, a directory name under parity/fixtures/.
    #[arg(long)]
    set: String,
    /// The recipe to record from. Defaults to parity/fixtures/<set>/recipe.toml.
    #[arg(long)]
    recipe: Option<PathBuf>,
    /// A fake-gh request log; its GET misses are appended to the recipe. Repeatable.
    #[arg(long = "from-log", value_name = "FAKE_GH_LOG")]
    from_log: Vec<PathBuf>,
    /// The real gh to record with. Defaults to the first gh on PATH.
    #[arg(long, value_name = "PATH")]
    gh: Option<PathBuf>,
    /// Record, then report changes against the committed fixtures without writing. Exits 1 on any change.
    #[arg(long, conflicts_with = "dry_run")]
    check: bool,
    /// Print the planned requests and touch nothing.
    #[arg(long)]
    dry_run: bool,
}

/// `cargo xtask check-fixtures`: run the scrub rules over committed fixtures, without network.
#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    /// The fixture root. Defaults to parity/fixtures.
    dir: Option<PathBuf>,
}

/// The repository root (the parent of `xtask/`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new(".."))
        .to_path_buf()
}

pub fn record_fixtures(args: &RecordArgs) -> ExitCode {
    if args.set.is_empty() || args.set.contains(['/', '\\']) || args.set.starts_with('.') {
        eprintln!("error: --set takes a set name like `base`, not a path");
        return ExitCode::FAILURE;
    }
    let set_dir = repo_root().join("parity").join("fixtures").join(&args.set);
    let mode = match (args.check, args.dry_run) {
        (_, true) => Mode::DryRun,
        (true, false) => Mode::Check,
        (false, false) => Mode::Write,
    };
    let gh = GhCli {
        program: args.gh.clone().unwrap_or_else(|| PathBuf::from("gh")),
    };
    let opts = Options {
        recipe: args.recipe.clone().unwrap_or_else(|| set_dir.join("recipe.toml")),
        set_dir,
        from_logs: args.from_log.clone(),
        mode,
        today: today_utc(),
        gh: &gh,
    };
    let outcome = match record::run(&opts) {
        Ok(outcome) => outcome,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if mode == Mode::DryRun {
        println!("Would record {} request(s):", outcome.requests.len());
        for entry in &outcome.requests {
            println!("  {}", describe_key(&entry.key()));
        }
    }
    if !outcome.added_to_recipe.is_empty() {
        let verb = if mode == Mode::Write { "Added" } else { "Would add" };
        println!(
            "{verb} {} request(s) from --from-log to the recipe:",
            outcome.added_to_recipe.len()
        );
        for entry in &outcome.added_to_recipe {
            println!("  {}", describe_key(&entry.key()));
        }
    }
    let mut changed = 0;
    for file in &outcome.files {
        let path = file.path.display();
        match &file.kind {
            ChangeKind::Unchanged => println!("  unchanged {path}"),
            ChangeKind::New => println!("  new       {path}"),
            ChangeKind::Changed => println!("  changed   {path}"),
            ChangeKind::Renamed { from } => {
                let from: Vec<String> = from.iter().map(|p| p.display().to_string()).collect();
                println!("  renamed   {path} (from {})", from.join(", "));
            }
        }
        changed += usize::from(file.kind != ChangeKind::Unchanged);
    }
    if !outcome.refused.is_empty() {
        println!("Refused (outside the recording scope):");
        for (key, reason) in &outcome.refused {
            println!("  {}: {reason}", describe_key(key));
        }
    }
    if !outcome.write_by_hand.is_empty() {
        println!("Write these by hand (the recorder never sends writes):");
        for (key, body) in &outcome.write_by_hand {
            match body {
                Some(body) => println!("  {} body {body}", describe_key(key)),
                None => println!("  {}", describe_key(key)),
            }
        }
    }
    for problem in &outcome.problems {
        eprintln!("problem: {problem}");
    }
    let recorded = if mode == Mode::DryRun { 0 } else { outcome.files.len() };
    println!(
        "Summary: recorded {recorded}, refused {}, write-by-hand {}",
        outcome.refused.len(),
        outcome.write_by_hand.len()
    );
    if mode == Mode::Check {
        println!("{changed} file(s) differ from the committed fixtures.");
    }

    if !outcome.problems.is_empty() || (mode == Mode::Check && changed > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

pub fn check_fixtures(args: &CheckArgs) -> ExitCode {
    let root = args
        .dir
        .clone()
        .unwrap_or_else(|| repo_root().join("parity").join("fixtures"));
    let problems = check::check_root(&root);
    for problem in &problems {
        eprintln!("{problem}");
    }
    if problems.is_empty() {
        println!("check-fixtures: no findings under {}", root.display());
        ExitCode::SUCCESS
    } else {
        eprintln!("check-fixtures: {} finding(s)", problems.len());
        ExitCode::FAILURE
    }
}

/// Today's date in UTC as `YYYY-MM-DD`.
fn today_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // `d` is in 1..=31 and `m` in 1..=12, so the casts can't truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_751), (2026, 10, 25));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
