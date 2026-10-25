//! Startup order and its side effects: arguments first, then the config, then logging, then the terminal.
//!
//! These point `HOME` at a temp dir to keep the real config folder out of it. Windows finds its config folder through
//! the shell API rather than `HOME`, so they only run elsewhere. stdout is a pipe here, so a run that gets past the
//! config stops before touching the terminal.
#![cfg(not(windows))]

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

struct TempHome(PathBuf);

impl TempHome {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lazy-github-bin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn config_dir(&self) -> PathBuf {
        self.0.join(".config").join("lazy-github")
    }

    fn write_config(&self, text: &str) {
        std::fs::create_dir_all(self.config_dir()).unwrap();
        std::fs::write(self.config_dir().join("config.json"), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::cargo_bin("lazy-github")
            .unwrap()
            .args(args)
            .env("HOME", &self.0)
            .env_remove("LAZY_GITHUB_LOG")
            .output()
            .unwrap()
    }

    /// Every path under the home, relative to it.
    fn contents(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(root).unwrap().to_owned());
                if path.is_dir() {
                    walk(&path, root, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &self.0, &mut out);
        out.sort();
        out
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_version_and_usage_errors_touch_nothing() {
    let home = TempHome::new("no-side-effects");
    for (args, code) in [(&["--help"][..], 0), (&["--version"][..], 0), (&["--bogus"][..], 2)] {
        let output = home.run(args);
        assert_eq!(output.status.code(), Some(code), "{args:?}: {}", text(&output.stderr));
        assert_eq!(home.contents(), Vec::<PathBuf>::new(), "{args:?}");
    }
    let output = home.run(&["--version"]);
    assert!(text(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_bad_config_value_exits_1_naming_the_field() {
    let home = TempHome::new("bad-value");
    home.write_config(r#"{"issues":{"state_filter":"bogus"}}"#);
    for args in [&[][..], &["run"][..]] {
        let output = home.run(args);
        assert_eq!(output.status.code(), Some(1));
        let stderr = text(&output.stderr);
        assert!(stderr.starts_with("Error: "), "{stderr:?}");
        assert!(stderr.contains("`issues.state_filter`"), "{stderr:?}");
        assert!(stderr.contains("lazy-github clear-config"), "{stderr:?}");
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert_eq!(text(&output.stdout), "");
    }
    assert!(!home.config_dir().join("lazy_github.log").exists());

    // Commands that don't need the config still work.
    let output = home.run(&["--version"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(!home.config_dir().join("lazy_github.log").exists());
}

#[test]
fn invalid_json_exits_1_with_the_position() {
    let home = TempHome::new("bad-json");
    home.write_config(r#"{"appearance": "#);
    let output = home.run(&[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.contains("is not valid JSON (line 1, column 15)"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    for traceback in ["Traceback", "panicked", "Location:", "Backtrace"] {
        assert!(!stderr.contains(traceback), "{stderr:?}");
    }
    assert_eq!(text(&output.stdout), "");
    assert!(!home.config_dir().join("lazy_github.log").exists());
}

#[test]
fn a_run_without_a_terminal_logs_and_stops_before_the_tui() {
    let home = TempHome::new("no-tty");
    let output = home.run(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("stdout is not one"),
        "{}",
        text(&output.stderr)
    );
    assert_eq!(text(&output.stdout), "");
    let log = std::fs::read_to_string(home.config_dir().join("lazy_github.log")).unwrap();
    assert!(
        log.lines()
            .next()
            .unwrap()
            .ends_with(concat!(" - Starting LazyGithub ", env!("CARGO_PKG_VERSION"))),
        "{log:?}"
    );
}

#[test]
fn an_unopenable_log_file_prints_one_line_and_carries_on() {
    let home = TempHome::new("unwritable");
    let blocker = home.0.join("not-a-dir");
    std::fs::write(&blocker, "").unwrap();
    let logfile = blocker.join("lazy_github.log");
    // Temp paths on Linux and macOS need no JSON escaping.
    home.write_config(&format!(r#"{{"core": {{"logfile_path": "{}"}}}}"#, logfile.display()));

    let output = home.run(&[]);
    let stderr = text(&output.stderr);
    let prefix = format!("LazyGithub: could not open log file {}: ", logfile.display());
    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.starts_with(&prefix), "{stderr:?}");
    assert!(first.ends_with("; continuing without a log file"), "{stderr:?}");
    // It carried on to the terminal check.
    assert!(stderr.contains("stdout is not one"), "{stderr:?}");
}
