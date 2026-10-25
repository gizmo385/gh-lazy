//! The binary starts logging without writing to the terminal.
//!
//! These point `HOME` at a temp dir to keep the real config folder out of it. Windows finds its config folder through
//! the shell API rather than `HOME`, so they only run elsewhere.
#![cfg(not(windows))]

use std::path::{Path, PathBuf};

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

    fn command(&self) -> Command {
        let mut cmd = Command::cargo_bin("lazy-github").unwrap();
        cmd.env("HOME", &self.0).env_remove("LAZY_GITHUB_LOG");
        cmd
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn version_logs_to_the_file_and_nothing_to_stderr() {
    let home = TempHome::new("version");
    let output = home.command().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));

    let log = read(&home.config_dir().join("lazy_github.log"));
    assert!(
        log.starts_with("DEBUG ")
            && log
                .trim_end()
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
    std::fs::create_dir_all(home.config_dir()).unwrap();
    let config = serde_json_string(&logfile);
    std::fs::write(home.config_dir().join("config.json"), config).unwrap();

    let output = home.command().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let prefix = format!("LazyGithub: could not open log file {}: ", logfile.display());
    assert!(stderr.starts_with(&prefix), "{stderr:?}");
    assert!(stderr.ends_with("; continuing without a log file\n"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

fn serde_json_string(logfile: &Path) -> String {
    // Temp paths on Linux and macOS need no JSON escaping.
    format!(r#"{{"core": {{"logfile_path": "{}"}}}}"#, logfile.display())
}
