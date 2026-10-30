//! `check-config`: lists the values in `config.json` that stop it loading and, with `--fix`, resets them.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lazy_github_core::config::doctor::{self, Fix, Problem, Report};
use lazy_github_core::config::{ConfigError, ConfigPaths};

use super::clear::{Answer, ask};
use super::{aborted, error, finish};

const START_OVER: &str = "Run lazy-github clear-config to start over.";

/// `lazy-github check-config [--fix [--no-confirm]]`. Without `fix` it only reports. With it, each problem is
/// offered for a reset on `stdin`, or all are reset when `stdin` is `None` (`--no-confirm`). Exits 0 when no
/// problems remain.
pub fn check_config(
    paths: Result<ConfigPaths, ConfigError>,
    fix: bool,
    stdin: Option<&mut impl BufRead>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => return error(stderr, e),
    };
    let file = paths.file.display();
    let text = match std::fs::read_to_string(&paths.file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let result = writeln!(stdout, "{file} doesn't exist; LazyGithub uses the defaults");
            return finish(result.map(|()| ExitCode::SUCCESS), stderr);
        }
        Err(source) => {
            return error(
                stderr,
                ConfigError::Read {
                    path: paths.file.clone(),
                    source,
                },
            );
        }
    };
    let report = match doctor::check(&text, &paths) {
        Ok(report) => report,
        Err(e @ ConfigError::Syntax { .. }) => {
            let code = error(stderr, e);
            // Nothing is left to report a failed write to.
            let _ = writeln!(stderr, "{START_OVER}");
            return code;
        }
        Err(e) => return error(stderr, e),
    };
    let result = (|| {
        print_report(&report, &paths.file, stdout)?;
        if report.problems.is_empty() {
            return Ok(ExitCode::SUCCESS);
        }
        if !fix {
            return Ok(ExitCode::FAILURE);
        }
        let mut stdin = stdin;
        let mut chosen = Vec::new();
        for problem in &report.problems {
            match ask(&prompt(problem), stdin.as_deref_mut(), stdout)? {
                Answer::Aborted => return Ok(aborted(stderr)),
                Answer::No => {}
                Answer::Yes => chosen.push(problem.field.clone()),
            }
        }
        if chosen.is_empty() {
            return Ok(ExitCode::FAILURE);
        }

        let backup = backup_path(&paths.file);
        if let Err(e) = std::fs::write(&backup, &text) {
            return Ok(error(stderr, format_args!("failed to write {}: {e}", backup.display())));
        }
        writeln!(stdout, "Saved a backup in {}", backup.display())?;
        if let Err(e) = doctor::repair(&text, &paths, &chosen).and_then(|repaired| repaired.save(&paths)) {
            return Ok(error(stderr, e));
        }
        let plural = if chosen.len() == 1 { "" } else { "s" };
        writeln!(stdout, "Reset {} field{plural}", chosen.len())?;
        Ok(if chosen.len() == report.problems.len() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        })
    })();
    finish(result, stderr)
}

/// The problems under a count, then the unknown keys.
fn print_report(report: &Report, file: &Path, stdout: &mut impl Write) -> io::Result<()> {
    let file = file.display();
    match report.problems.len() {
        0 => writeln!(stdout, "{file}: no problems")?,
        1 => writeln!(stdout, "{file}: 1 problem")?,
        n => writeln!(stdout, "{file}: {n} problems")?,
    }
    for problem in &report.problems {
        writeln!(stdout, "  {problem}")?;
    }
    for key in &report.unknown_keys {
        writeln!(stdout, "{file}: unknown key `{key}` (kept)")?;
    }
    Ok(())
}

fn prompt(problem: &Problem) -> String {
    match &problem.fix {
        Fix::Reset(default) => format!("Reset {} to its default ({default})?", problem.field),
        // A keybinding override: without it the binding takes its default key.
        Fix::Remove => format!("Remove {}?", problem.field),
    }
}

/// `config.json.bak` next to `config.json`.
fn backup_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_owned();
    name.push(".bak");
    file.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::check_config;
    use lazy_github_core::config::ConfigPaths;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lazy-github-check-unit-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn paths(&self) -> ConfigPaths {
            ConfigPaths::in_dir(self.0.join("lazy-github"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The exit code, stdout and stderr of `check-config` on `config` with `input` as stdin (`None`: `--no-confirm`).
    fn run(paths: &ConfigPaths, fix: bool, input: Option<&str>) -> (ExitCode, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut stdin = input.map(Cursor::new);
        let code = check_config(Ok(paths.clone()), fix, stdin.as_mut(), &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn a_kept_field_leaves_the_file_failing_to_load() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(&paths.dir).unwrap();
        let text = r#"{"issues":{"state_filter":"bogus","owner_filter":"everyone"}}"#;
        std::fs::write(&paths.file, text).unwrap();
        let (code, out, err) = run(&paths, true, Some("y\nn\n"));
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(err, "");
        assert!(out.ends_with("Reset 1 field\n"), "{out}");
        let (code, out, _) = run(&paths, false, None);
        assert_eq!(code, ExitCode::FAILURE);
        assert!(out.contains("1 problem\n  issues.owner_filter"), "{out}");
    }

    #[test]
    fn an_abort_writes_nothing() {
        let tmp = TempDir::new();
        let paths = tmp.paths();
        std::fs::create_dir_all(&paths.dir).unwrap();
        std::fs::write(&paths.file, r#"{"issues":5}"#).unwrap();
        let (code, _, err) = run(&paths, true, Some(""));
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(err, "Aborted!\n");
        assert_eq!(std::fs::read_to_string(&paths.file).unwrap(), r#"{"issues":5}"#);
        assert!(!paths.dir.join("config.json.bak").exists());
    }
}
