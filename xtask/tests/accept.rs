//! Drives `--accept` with the `echo_tui` example standing in for the Rust app, in a temp repo: two agreeing runs write
//! a snapshot, a second accept leaves it unchanged, two disagreeing runs write nothing and print the diff, orphans are
//! deleted, and a normal run then fails with a `snapshot` mismatch once the screen no longer matches. Needs no Python.

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use xtask::parity::cli::{self, EXIT_FAILED, EXIT_OK, ParityArgs};
use xtask::parity::env::Paths;
use xtask::parity::scenario::{Roots, Scenario};
use xtask::parity::snapshot::{self, AcceptOptions, AcceptOutcome, AcceptReport};
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

#[derive(Parser)]
struct Wrapper {
    #[command(flatten)]
    args: ParityArgs,
}

fn args(argv: &[&str]) -> ParityArgs {
    Wrapper::try_parse_from(std::iter::once("parity").chain(argv.iter().copied()))
        .unwrap()
        .args
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// A git repo with what scenarios need, and the Rust-only scenarios `steady` and `differs`.
struct Repo {
    dir: tempfile::TempDir,
}

const STEADY: &str = "parity/snapshots/steady/80x24";

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
        repo.write("parity/configs/default.json", "{}\n");
        std::fs::create_dir_all(repo.path().join("parity/fixtures/base")).unwrap();
        let scenario =
            |steps: &str| format!("version = 1\ngate = true\napps = [\"rust\"]\nsizes = [[80, 24]]\n{steps}");
        repo.write(
            "parity/scenarios/steady.toml",
            &scenario(
                "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\ncapture = \"main\"\n\n\
                 [expect.main]\ncontains = [\"size 80x24\"]\n",
            ),
        );
        // Ctrl+B draws a box titled with the process id, so two runs never agree.
        repo.write(
            "parity/scenarios/differs.toml",
            &scenario(
                "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nkeys = [\"ctrl+b\"]\n\n\
                 [[step]]\nwait = { text = \"boxed\" }\n\n[[step]]\ncapture = \"main\"\n",
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
            seeds: self.path().join("parity/seeds"),
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

    fn paths(&self) -> Paths {
        let echo = echo_tui();
        Paths {
            fake_gh: echo.clone(),
            rust_app: echo.clone(),
            python: echo,
            roots: self.roots(),
        }
    }

    fn accept(&self, names: &[&str], all: bool) -> AcceptReport {
        let themes = Themes::default();
        let options = AcceptOptions {
            all,
            jobs: 2,
            keep_home: false,
            themes: &themes,
        };
        let out = self.path().join("out");
        snapshot::accept(self.path(), &self.scenarios(names), &self.paths(), &out, &options).unwrap()
    }

    /// A normal run of `steady` (Rust only), returning the exit code and the report directory.
    fn run(&self) -> (u8, PathBuf) {
        let out = self.path().join("report");
        let argv = [
            "steady",
            "--no-build",
            "--no-sync",
            "--jobs",
            "1",
            "--out",
            out.to_str().unwrap(),
        ];
        let code = cli::run_scenarios(
            self.path(),
            &self.scenarios(&["steady"]),
            &self.roots(),
            &Themes::default(),
            &args(&argv),
            Some(self.paths()),
        )
        .unwrap();
        (code, out)
    }
}

fn outcome<'a>(report: &'a AcceptReport, name: &str) -> &'a AcceptOutcome {
    &report
        .outcomes
        .iter()
        .find(|u| u.scenario == name)
        .unwrap_or_else(|| panic!("no outcome for {name}: {report:?}"))
        .outcome
}

#[test]
fn accepts_from_two_agreeing_runs_and_leaves_an_unchanged_snapshot_alone() {
    let repo = Repo::new();
    // A leftover from an interrupted write never ends up in the snapshot, and is deleted as an orphan.
    repo.write(&format!("{STEADY}.tmp/junk.screen"), "junk");

    let report = repo.accept(&["steady"], true);
    assert_eq!(outcome(&report, "steady"), &AcceptOutcome::Accepted, "{report:?}");
    assert!(report.all_accepted());
    let screen = repo.read(&format!("{STEADY}/main.screen"));
    assert!(screen.starts_with("lazy-github-screen 1\nsize 80x24\n"), "{screen}");
    assert!(screen.contains("\nsettled\n"), "{screen}");
    let svg = repo.read(&format!("{STEADY}/main.svg"));
    assert!(svg.contains("<title>steady · main · 80x24</title>"), "{svg}");
    assert_eq!(repo.read(&format!("{STEADY}/effects.txt")), "");
    assert!(!repo.exists(&format!("{STEADY}.tmp")));
    assert!(!repo.exists(&format!("{STEADY}.old")));
    // Both runs' dumps are kept for debugging.
    for run in ["run-1", "run-2"] {
        assert!(repo.exists(&format!("out/steady/80x24/{run}/main.txt")), "{run}");
    }
    assert_eq!(
        snapshot::check(repo.path(), &repo.scenarios(&["steady"]), false).unwrap(),
        []
    );

    let report = repo.accept(&["steady"], true);
    assert_eq!(outcome(&report, "steady"), &AcceptOutcome::Unchanged, "{report:?}");
    assert_eq!(repo.read(&format!("{STEADY}/main.screen")), screen);

    // A normal run agrees with it.
    let (code, out) = repo.run();
    assert_eq!(code, EXIT_OK);
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    assert!(check.get("snapshot").is_none(), "{check:#}");
    assert_eq!(
        read_json(&out.join("summary.json"))["scenarios"][0]["result"],
        "rust-only"
    );
}

#[test]
fn runs_that_disagree_are_not_accepted() {
    let repo = Repo::new();
    let report = repo.accept(&["differs"], true);
    let AcceptOutcome::NotAccepted { reason, diff } = outcome(&report, "differs") else {
        panic!("{report:?}");
    };
    assert_eq!(reason, "run 2 differs from run 1: main: rows 5-5 differ");
    assert!(diff.contains("--- run 1") && diff.contains("+++ run 2"), "{diff}");
    assert!(diff.contains("pid "), "{diff}");
    assert!(!report.all_accepted());
    assert!(!repo.exists("parity/snapshots/differs"));
}

#[test]
fn orphans_are_deleted() {
    let repo = Repo::new();
    repo.accept(&["steady"], true);
    repo.write("parity/snapshots/gone/80x24/main.screen", "");
    repo.write("parity/snapshots/steady/100x30/main.screen", "");
    repo.write(&format!("{STEADY}/old.svg"), "");
    repo.write(&format!("{STEADY}/notes.txt"), "");

    // Naming a scenario can't tell whether another scenario's directory is an orphan. The size directory with extra
    // files is rewritten whole, which drops them.
    let report = repo.accept(&["steady"], false);
    assert_eq!(outcome(&report, "steady"), &AcceptOutcome::Accepted, "{report:?}");
    assert_eq!(report.deleted, ["parity/snapshots/steady/100x30/"]);
    assert!(!repo.exists(&format!("{STEADY}/old.svg")));
    assert!(!repo.exists(&format!("{STEADY}/notes.txt")));
    assert!(repo.exists("parity/snapshots/gone"));

    let report = repo.accept(&["steady"], true);
    assert_eq!(report.deleted, ["parity/snapshots/gone/"]);
    assert_eq!(outcome(&report, "steady"), &AcceptOutcome::Unchanged, "{report:?}");
}

#[test]
fn a_run_that_no_longer_matches_its_snapshot_fails() {
    let repo = Repo::new();
    repo.accept(&["steady"], true);
    // What the app drew before a change: the run now draws `settled` where the snapshot has `settles`.
    let path = format!("{STEADY}/main.screen");
    let screen = repo.read(&path);
    repo.write(&path, &screen.replacen("\nsettled\n", "\nsettles\n", 1));

    let (code, out) = repo.run();
    assert_eq!(code, EXIT_FAILED);
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    let mismatches = check["snapshot"].as_array().unwrap_or_else(|| panic!("{check:#}"));
    assert_eq!(mismatches.len(), 1, "{check:#}");
    assert_eq!(mismatches[0]["check"], "snapshot");
    assert_eq!(
        mismatches[0]["message"],
        "rows 1-1 differ; run cargo xtask parity --accept steady and review the diff"
    );
    let summary = read_json(&out.join("summary.json"));
    let scenario = &summary["scenarios"][0];
    assert_eq!(scenario["result"], "fail");
    assert_eq!(
        scenario["sizes"][0]["captures"][0]["against"],
        serde_json::json!(["rust"])
    );
    // What the run saw, next to its dumps.
    let rust = out.join("steady/80x24/rust");
    assert_eq!(std::fs::read_to_string(rust.join("main.screen.new")).unwrap(), screen);
    assert!(rust.join("main.svg.new").is_file());
    let diff = std::fs::read_to_string(rust.join("main.snapshot.diff.txt")).unwrap();
    assert!(diff.contains("-settles\n+settled\n"), "{diff}");
    assert!(!rust.join("effects.txt.new").exists());
    let page = std::fs::read_to_string(out.join("steady/report.html")).unwrap();
    assert!(page.contains("80x24/rust/main.snapshot.diff.txt"), "{page}");
}
