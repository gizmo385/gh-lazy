//! Drives `--record` with the `echo_tui` example standing in for Python, in a temp repo: a reference is recorded
//! from two agreeing runs, re-recorded from one run with its files kept, not recorded when the runs differ or time
//! out, written by one rename, and orphans are deleted. Needs no Python.

use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::parity::env::Paths;
use xtask::parity::record::{self, Options, Outcome, Report};
use xtask::parity::reference::{self, Freshness};
use xtask::parity::scenario::{Roots, Scenario, Size};
use xtask::parity::structure::Themes;

/// `cargo test` builds examples next to the test binaries.
fn echo_tui() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(Path::parent).unwrap();
    let path = profile
        .join("examples")
        .join(format!("echo_tui{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing; run `cargo build -p xtask --example echo_tui`",
        path.display()
    );
    path
}

const SIZE: Size = Size { cols: 80, rows: 24 };

/// A git repo with what the hashes and scenarios need, and the scenarios `steady`, `differs` and `slow`.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Self {
        let repo = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        assert!(status.success());
        repo.write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"vt100\"\nversion = \"0.18.1\"\n\n\
             [[package]]\nname = \"portable-pty\"\nversion = \"0.9.0\"\n",
        );
        repo.write("parity/configs/default.json", "{}\n");
        std::fs::create_dir_all(repo.path().join("parity/fixtures/base")).unwrap();
        let scenario = |steps: &str| format!("version = 1\napps = [\"python\"]\nsizes = [[80, 24]]\n{steps}");
        repo.write(
            "parity/scenarios/steady.toml",
            &scenario(
                "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\ncapture = \"main\"\n\n\
                 [expect.main]\ncontains = [\"size 80x24\"]\n",
            ),
        );
        repo.write(
            "parity/scenarios/differs.toml",
            &scenario(
                "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nkeys = [\"ctrl+b\"]\n\n\
                 [[step]]\nwait = { text = \"boxed\" }\n\n[[step]]\ncapture = \"main\"\n",
            ),
        );
        repo.write(
            "parity/scenarios/slow.toml",
            &scenario(
                "[[step]]\nwait = { text = \"never shown\", timeout_ms = 300 }\n\n[[step]]\ncapture = \"main\"\n",
            ),
        );
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        let path = self.path().join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn exists(&self, rel: &str) -> bool {
        self.path().join(rel).exists()
    }

    fn roots(&self) -> Roots {
        Roots {
            fixtures: self.path().join("parity/fixtures"),
            configs: self.path().join("parity/configs"),
        }
    }

    fn scenarios(&self, names: &[&str]) -> Vec<Scenario> {
        names
            .iter()
            .map(|n| {
                Scenario::load(&self.path().join(format!("parity/scenarios/{n}.toml")), &self.roots())
                    .unwrap_or_else(|e| panic!("{e}"))
            })
            .collect()
    }

    fn record(&self, names: &[&str], force: bool, all: bool) -> Report {
        let echo = echo_tui();
        let paths = Paths {
            fake_gh: echo.clone(),
            rust_app: echo.clone(),
            python: echo,
            roots: self.roots(),
        };
        let themes = Themes::default();
        let options = Options {
            force,
            all,
            jobs: 2,
            keep_home: false,
            themes: &themes,
            python_version: "echo".into(),
        };
        let out = self.path().join("out");
        record::record(self.path(), &self.scenarios(names), &paths, &out, &options).unwrap()
    }

    fn freshness(&self, name: &str) -> Freshness {
        reference::freshness(self.path(), &self.scenarios(&[name])[0], SIZE, &self.roots()).unwrap()
    }
}

fn outcome<'a>(report: &'a Report, name: &str) -> &'a Outcome {
    &report
        .outcomes
        .iter()
        .find(|u| u.scenario == name)
        .unwrap_or_else(|| panic!("no outcome for {name}: {report:?}"))
        .outcome
}

const STEADY: &str = "parity/references/steady/80x24";

#[test]
fn records_from_two_agreeing_runs_and_keeps_files_on_an_unchanged_rerecord() {
    let repo = Repo::new();
    // A leftover from an interrupted write never ends up in the reference.
    repo.write(&format!("{STEADY}.tmp/junk.screen"), "junk");

    let report = repo.record(&["steady"], false, true);
    assert!(
        matches!(outcome(&report, "steady"), Outcome::Recorded { .. }),
        "{report:?}"
    );
    assert!(report.remaining.problems.is_empty(), "{report:?}");
    assert_eq!(repo.freshness("steady"), Freshness::Fresh);
    let screen = repo.read(&format!("{STEADY}/main.screen"));
    assert!(screen.starts_with("lazy-github-screen 1\nsize 80x24\n"), "{screen}");
    assert!(screen.contains("\nsettled\n"), "{screen}");
    let manifest = repo.read(&format!("{STEADY}/manifest.toml"));
    assert!(manifest.contains("python = \"echo\""), "{manifest}");
    assert!(repo.exists(&format!("{STEADY}/requests.jsonl")));
    assert!(!repo.exists(&format!("{STEADY}.tmp")));
    assert!(!repo.exists(&format!("{STEADY}.old")));
    // Both runs' dumps are kept for debugging.
    for run in ["run-1", "run-2"] {
        assert!(repo.exists(&format!("out/steady/80x24/{run}/main.txt")), "{run}");
    }

    // Fresh references are skipped.
    let report = repo.record(&["steady"], false, true);
    assert_eq!(outcome(&report, "steady"), &Outcome::Fresh);

    // A forced re-record that changes nothing runs once and keeps the screen byte for byte.
    let before = screen;
    let report = repo.record(&["steady"], true, true);
    assert!(
        matches!(outcome(&report, "steady"), Outcome::Unchanged { .. }),
        "{report:?}"
    );
    assert_eq!(repo.read(&format!("{STEADY}/main.screen")), before);
    assert!(repo.exists("out/steady/80x24/run-1/main.txt"));
    assert!(!repo.exists("out/steady/80x24/run-2"));
    assert_eq!(repo.freshness("steady"), Freshness::Fresh);
}

#[test]
fn runs_that_differ_or_fail_are_not_recorded() {
    let repo = Repo::new();
    let report = repo.record(&["differs", "slow"], false, true);

    let Outcome::NotRecorded { reason, diff } = outcome(&report, "differs") else {
        panic!("{report:?}");
    };
    assert!(
        reason.starts_with("run 2 differs from run 1: main: titles: "),
        "{reason}"
    );
    assert!(diff.contains("--- run 1") && diff.contains("+++ run 2"), "{diff}");
    assert!(diff.contains("pid "), "{diff}");
    assert!(!repo.exists("parity/references/differs"));

    let Outcome::NotRecorded { reason, diff } = outcome(&report, "slow") else {
        panic!("{report:?}");
    };
    assert!(reason.starts_with("run 1 timeout at step 0 ("), "{reason}");
    assert!(diff.is_empty(), "{diff}");
    // A failed first run isn't run again.
    assert!(!repo.exists("out/slow/80x24/run-2"));
    assert!(!repo.exists("parity/references/slow"));

    let remaining: Vec<String> = report.remaining.problems.iter().map(ToString::to_string).collect();
    assert_eq!(
        remaining,
        [
            "missing reference: differs/80x24; run cargo xtask parity --record differs",
            "missing reference: slow/80x24; run cargo xtask parity --record slow",
        ]
    );
}

#[test]
fn orphans_are_deleted() {
    let repo = Repo::new();
    repo.record(&["steady"], false, true);
    repo.write("parity/references/gone/80x24/manifest.toml", "");
    repo.write("parity/references/steady/100x30/manifest.toml", "");
    repo.write(&format!("{STEADY}/old.screen"), "");
    let before = repo.read(&format!("{STEADY}/main.screen"));

    // Naming a scenario can't tell whether another scenario's directory is an orphan.
    let report = repo.record(&["steady"], false, false);
    assert_eq!(outcome(&report, "steady"), &Outcome::Fresh);
    assert_eq!(
        report.deleted,
        [
            "parity/references/steady/100x30/",
            "parity/references/steady/80x24/old.screen"
        ]
    );
    assert!(repo.exists("parity/references/gone"));

    let report = repo.record(&["steady"], false, true);
    assert_eq!(report.deleted, ["parity/references/gone/"]);
    assert!(report.remaining.problems.is_empty(), "{report:?}");
    assert_eq!(repo.read(&format!("{STEADY}/main.screen")), before);
}
