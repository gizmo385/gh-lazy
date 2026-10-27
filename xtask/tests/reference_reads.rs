//! A normal parity run takes Python from the recorded references: it starts no Python, exits 2 before anything runs
//! when a reference it reads is stale, and with `--live-python` checks the live run against the reference (drift).
//! The `echo_tui` example stands in for both apps, in a temp repo. Needs no Python.

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use xtask::parity::cli::{self, EXIT_FAILED, EXIT_HARNESS, EXIT_OK, ParityArgs};
use xtask::parity::env::Paths;
use xtask::parity::record::{self, Options, Outcome};
use xtask::parity::reference;
use xtask::parity::scenario::{Roots, Scenario};
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

/// A git repo with what the hashes need and a scenario `steady` that both apps run, with its reference recorded
/// from `echo_tui`.
struct Repo {
    dir: tempfile::TempDir,
}

const STEADY: &str = "parity/references/steady/80x24";

impl Repo {
    fn recorded() -> Self {
        Self::recorded_with("")
    }

    /// [`Repo::recorded`] with `env` (TOML lines) in the scenario's `[env]` table, and a seed `s` holding
    /// `home/.cache/repos.json`.
    fn recorded_with(env: &str) -> Self {
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
        repo.write("parity/seeds/s/home/.cache/repos.json", "[]\n");
        repo.write(
            "parity/scenarios/steady.toml",
            &format!(
                "version = 1\nsizes = [[80, 24]]\n\n[env]\n{env}\n[[step]]\nwait = {{ text = \"settled\" }}\n\n\
                 [[step]]\ncapture = \"main\"\n\n[expect.main]\ncontains = [\"size 80x24\"]\n"
            ),
        );

        let options = Options {
            force: false,
            all: true,
            jobs: 1,
            keep_home: false,
            themes: &Themes::default(),
            python_version: "echo".into(),
        };
        let report = record::record(
            repo.path(),
            &repo.scenarios(),
            &repo.paths(echo_tui()),
            &repo.path().join("record"),
            &options,
        )
        .unwrap();
        assert!(
            matches!(report.outcomes[0].outcome, Outcome::Recorded { .. }),
            "{report:?}"
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

    fn roots(&self) -> Roots {
        Roots {
            fixtures: self.path().join("parity/fixtures"),
            configs: self.path().join("parity/configs"),
            seeds: self.path().join("parity/seeds"),
        }
    }

    fn scenarios(&self) -> Vec<Scenario> {
        let file = self.path().join("parity/scenarios/steady.toml");
        vec![Scenario::load(&file, &self.roots()).unwrap_or_else(|e| panic!("{e}"))]
    }

    fn paths(&self, python: PathBuf) -> Paths {
        let echo = echo_tui();
        Paths {
            fake_gh: echo.clone(),
            rust_app: echo,
            python,
            roots: self.roots(),
        }
    }

    /// Runs `steady` with `flags`, Python at `python`, and returns the exit code and the report directory.
    fn run(&self, flags: &[&str], python: PathBuf) -> (u8, PathBuf) {
        let out = self.path().join("out");
        let out_arg = out.to_str().unwrap();
        let argv = [
            &["steady", "--no-build", "--no-sync", "--jobs", "1", "--out", out_arg][..],
            flags,
        ]
        .concat();
        let code = cli::run_scenarios(
            self.path(),
            &self.scenarios(),
            &self.roots(),
            &Themes::default(),
            &args(&argv),
            Some(self.paths(python)),
        )
        .unwrap();
        (code, out)
    }
}

#[test]
fn the_default_mode_starts_no_python() {
    let repo = Repo::recorded();
    let (code, out) = repo.run(&[], PathBuf::from("/nonexistent/python"));
    assert_eq!(code, EXIT_OK);

    let python = out.join("steady/80x24/python");
    let status = read_json(&python.join("status.json"));
    assert_eq!(status["recorded"], true, "{status:#}");
    assert_eq!(status["status"], "ok");
    assert_eq!(status["duration_ms"], 0);
    // Nothing ran: there's no PTY output, only what the reference holds.
    assert!(!python.join("pty.raw").exists());
    let main = std::fs::read_to_string(python.join("main.txt")).unwrap();
    assert!(main.contains("size 80x24"), "{main}");
    assert!(python.join("main.grid.json").is_file());
    assert!(python.join("fake-gh.log").is_file());

    // Rust ran live, and the checks ran against the reference.
    let rust = read_json(&out.join("steady/80x24/rust/status.json"));
    assert!(rust.get("recorded").is_none(), "{rust:#}");
    assert!(out.join("steady/80x24/rust/pty.raw").is_file());
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    assert_eq!(check["expect"], serde_json::json!([]), "{check:#}");
    assert!(check.get("drift").is_none(), "{check:#}");
    let summary = read_json(&out.join("summary.json"));
    assert_eq!(summary["scenarios"][0]["result"], "pass", "{summary:#}");
    assert_eq!(summary["scenarios"][0]["sizes"][0]["captures"][0]["check"], "pass");

    let index = std::fs::read_to_string(out.join("index.html")).unwrap();
    assert!(index.contains("<p>Python: recorded</p>"), "{index}");
    let md = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(md.contains("\n\nPython: recorded\n\n"), "{md}");
    let page = std::fs::read_to_string(out.join("steady/report.html")).unwrap();
    assert!(page.contains("<figcaption>Python (recorded)</figcaption>"), "{page}");
}

#[test]
fn a_live_run_that_agrees_with_its_reference_has_no_drift() {
    let repo = Repo::recorded();
    let (code, out) = repo.run(&["--live-python"], echo_tui());
    assert_eq!(code, EXIT_OK);
    let status = read_json(&out.join("steady/80x24/python/status.json"));
    assert!(status.get("recorded").is_none(), "{status:#}");
    assert!(out.join("steady/80x24/python/pty.raw").is_file());
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    assert!(check.get("drift").is_none(), "{check:#}");
    let md = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(md.contains("\n\nPython: live\n\n"), "{md}");
}

#[test]
fn a_live_run_that_disagrees_with_its_reference_drifts() {
    let repo = Repo::recorded();
    // The screen isn't hashed, so the reference stays fresh, but it no longer shows what Python draws.
    let screen = repo.path().join(STEADY).join("main.screen");
    let text = std::fs::read_to_string(&screen).unwrap();
    // Only in the screen's text, not in the header's `size 80x24`.
    let (header, rows) = text.split_once("--- text\n").unwrap();
    assert!(rows.contains("size 80x24"), "{text}");
    let tampered = format!("{header}--- text\n{}", rows.replace("size 80x24", "size 80x99"));
    std::fs::write(&screen, tampered).unwrap();

    let (code, out) = repo.run(&["--python-only"], echo_tui());
    assert_eq!(code, EXIT_FAILED);
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    let drift = check["drift"].as_array().unwrap_or_else(|| panic!("{check:#}"));
    assert_eq!(drift.len(), 1, "{check:#}");
    assert_eq!(drift[0]["check"], "contains");
    assert_eq!(drift[0]["expected"], "size 80x24");
    assert!(
        drift[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("the reference [expect]: no row contains"),
        "{check:#}"
    );
    let summary = read_json(&out.join("summary.json"));
    let scenario = &summary["scenarios"][0];
    assert_eq!(scenario["result"], "fail");
    assert_eq!(
        scenario["sizes"][0]["captures"][0]["against"],
        serde_json::json!(["python"])
    );

    // The default mode reads the bad reference, and its [expect] fails against Python.
    let (code, _) = repo.run(&[], PathBuf::from("/nonexistent/python"));
    assert_eq!(code, EXIT_FAILED);
}

#[test]
fn a_stale_reference_exits_2_before_anything_runs() {
    let repo = Repo::recorded();
    repo.write(
        "parity/scenarios/steady.toml",
        "version = 1\nsizes = [[80, 24]]\nkey_delay_ms = 7\n\n[[step]]\nwait = { text = \"settled\" }\n\n\
         [[step]]\ncapture = \"main\"\n",
    );
    let (code, out) = repo.run(&[], PathBuf::from("/nonexistent/python"));
    assert_eq!(code, EXIT_HARNESS);
    assert!(!out.exists(), "nothing ran, so there's no report");

    // Live, the stale reference is only skipped.
    let (code, out) = repo.run(&["--live-python=steady"], echo_tui());
    assert_eq!(code, EXIT_OK);
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    assert!(check.get("drift").is_none(), "{check:#}");
}

/// A seed is a capture input: editing one of its files makes the scenario's reference stale.
#[test]
fn a_changed_seed_file_makes_the_reference_stale() {
    let repo = Repo::recorded_with("seed = \"s\"\n");
    let fresh = reference::check(repo.path(), &repo.scenarios(), &repo.roots(), true).unwrap();
    assert_eq!(fresh.problems, [], "{fresh:?}");

    repo.write("parity/seeds/s/home/.cache/repos.json", "[{}]\n");
    let report = reference::check(repo.path(), &repo.scenarios(), &repo.roots(), true).unwrap();
    let problems: Vec<String> = report.problems.iter().map(ToString::to_string).collect();
    assert_eq!(
        problems,
        ["stale reference: steady/80x24: scenario changed; run cargo xtask parity --record steady"]
    );
    let (code, _) = repo.run(&[], PathBuf::from("/nonexistent/python"));
    assert_eq!(code, EXIT_HARNESS);
}

/// The binary against the real repo: a copy of `startup` with one more step has stale references, and the run
/// stops with their `--check-references` lines before it builds or starts anything.
#[test]
fn the_stale_line_names_the_way_around_it() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let scenario = tmp.path().join("startup.toml");
    let mut source = std::fs::read_to_string(repo.join("parity/scenarios/startup.toml")).unwrap();
    source.push_str("\n[[step]]\nsleep_ms = 1\n");
    std::fs::write(&scenario, source).unwrap();
    let out = tmp.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("parity")
        .arg(&scenario)
        .args(["--python", "/nonexistent/python", "--no-sync", "--ci", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    // Other inputs may be stale too (a harness edit not re-recorded yet); `scenario` always is.
    let stale = |line: &str, size: &str| {
        let rest = line.strip_prefix(&format!("stale reference: startup/{size}: scenario"));
        rest.is_some_and(|r| r.ends_with(" changed; run cargo xtask parity --record startup (or use --live-python)"))
    };
    assert_eq!(lines.len(), 4, "{stdout}");
    assert!(stale(lines[0], "160x48") && stale(lines[1], "100x30"), "{stdout}");
    for (line, size) in lines[2..].iter().zip(["160x48", "100x30"]) {
        // The file is the scenario's path, escaped for a workflow command.
        let (file, message) = line.split_once("startup.toml::").unwrap();
        assert!(file.starts_with("::error file="), "{stdout}");
        assert!(stale(message, size), "{stdout}");
    }
    assert!(!out.exists());
}

impl Repo {
    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(self.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

/// `--live-python-since` reads every reference when nothing Python-related changed, and runs Python live (checked
/// for drift) for a scenario whose reference changed.
#[test]
fn live_python_since_runs_python_only_where_its_reference_changed() {
    let repo = Repo::recorded();
    // Out of the diff, like the report directory in a real checkout.
    repo.write(".gitignore", "/out/\n/record/\n");
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-qm", "recorded"]);
    let (code, out) = repo.run(&["--live-python-since", "HEAD"], PathBuf::from("/nonexistent/python"));
    assert_eq!(code, EXIT_OK);
    let status = read_json(&out.join("steady/80x24/python/status.json"));
    assert_eq!(status["recorded"], true, "{status:#}");

    // Commit a different screen, then put the recorded one back: the reference differs from HEAD.
    let screen = repo.path().join(STEADY).join("main.screen");
    let recorded = std::fs::read_to_string(&screen).unwrap();
    std::fs::write(&screen, recorded.replace("size 80x24", "size 80x99")).unwrap();
    repo.git(&["commit", "-qam", "tampered"]);
    std::fs::write(&screen, &recorded).unwrap();
    let (code, out) = repo.run(&["--live-python-since", "HEAD"], echo_tui());
    assert_eq!(code, EXIT_OK);
    let status = read_json(&out.join("steady/80x24/python/status.json"));
    assert!(status.get("recorded").is_none(), "{status:#}");
    assert!(out.join("steady/80x24/python/pty.raw").is_file());
    let check = read_json(&out.join("steady/80x24/main.check.json"));
    assert!(check.get("drift").is_none(), "{check:#}");
    let md = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(md.contains("\n\nPython: live\n\n"), "{md}");
}

/// The binary against the real repo: `--plan` prints one `live-python=` line and nothing else, and exits 0 before
/// it builds anything (a nested `cargo build` would wait on this test run's lock).
#[test]
fn plan_prints_one_output_line_without_building() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["parity", "--all", "--live-python-since", "HEAD", "--plan"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let value = stdout
        .strip_prefix("live-python=")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("{stdout:?}"));
    assert!(!value.is_empty() && !value.contains(['\n', ' ']), "{stdout:?}");
}
