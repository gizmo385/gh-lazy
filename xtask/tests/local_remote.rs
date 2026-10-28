//! `git_remote_branches`: the bare repo the harness makes, git in the work dir reaching it over fake-ssh with the
//! app's environment, and `wait.remote_branch`. Uses real git; needs no Python.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use portable_pty::CommandBuilder;
use xtask::parity::env::{self, LocalRemote, Paths, RunEnv};
use xtask::parity::runner::{self, RunState};
use xtask::parity::scenario::{App, Roots, Scenario, Size};

/// `cargo test` builds the examples next to the test binaries, in `target/<profile>/examples/`.
fn example(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(Path::parent).unwrap();
    let path = profile
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing; run `cargo build -p xtask --examples`",
        path.display()
    );
    path
}

fn roots() -> Roots {
    let parity = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity");
    Roots {
        fixtures: parity.join("fixtures"),
        configs: parity.join("configs"),
        seeds: parity.join("seeds"),
    }
}

fn scenario(branches: &str, steps: &str) -> Scenario {
    let source = format!(
        "version = 1\nsizes = [[80, 24]]\ntimeout_ms = 15000\n[env]\ngit_remote = \"o/n\"\n\
         git_remote_protocol = \"ssh\"\ngit_branch = \"topic\"\ngit_remote_branches = {branches}\n{steps}"
    );
    Scenario::parse(Path::new("local-remote.toml"), &source, &roots()).unwrap_or_else(|e| panic!("{e}"))
}

/// Prepares a run of `scenario` with `echo_tui` standing in for fake-gh, so fake-ssh (the example next to it) is
/// installed.
fn prepare(scenario: &Scenario, out: &Path) -> RunEnv {
    let echo = example("echo_tui");
    // Built as a dependency of this test; the assert says which command builds it when it's missing.
    example("fake-ssh");
    let paths = Paths {
        fake_gh: echo.clone(),
        rust_app: echo.clone(),
        python: echo,
        roots: roots(),
    };
    std::fs::create_dir_all(out).unwrap();
    env::prepare(scenario, App::Rust, Size { cols: 80, rows: 24 }, &paths, out, false).unwrap()
}

/// `git <args>` in the run's work dir with exactly the environment the app gets.
fn app_git(run: &RunEnv, args: &[&str]) -> std::process::Output {
    let mut git = Command::new("git");
    git.args(args).env_clear();
    for (key, value) in run.command.iter_extra_env_as_str() {
        git.env(key, value);
    }
    git.current_dir(run.command.get_cwd().unwrap());
    git.output().unwrap()
}

#[track_caller]
fn app_git_ok(run: &RunEnv, args: &[&str]) -> String {
    let output = app_git(run, args);
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// The branches of the bare repo, sorted.
fn bare_branches(remote: &LocalRemote) -> Vec<String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(&remote.git_dir)
        .args(["for-each-ref", "--format=%(refname)", "refs/heads"])
        .env("HOME", &remote.home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Runs the scenario's steps against `echo_tui` with `remote` as the run's local remote.
fn run_steps(scenario: &Scenario, remote: &LocalRemote, out: &Path) -> RunState {
    let mut command = CommandBuilder::new(example("echo_tui"));
    command.env("TERM", "xterm-256color");
    let mut env = RunEnv::for_command(command, Size { cols: 80, rows: 24 }, scenario.timing);
    env.remote = Some(remote.clone());
    runner::run(env, &scenario.steps, out).unwrap().status
}

#[test]
fn the_work_repo_reaches_the_bare_repo_over_fake_ssh() {
    let tmp = tempfile::tempdir().unwrap();
    let s = scenario("[\"main\", \"dev\"]", "");
    let run = prepare(&s, &tmp.path().join("out"));
    let remote = run.remote.clone().expect("the run has a local remote");
    let root = run.run_dir.as_ref().unwrap().path().to_owned();
    assert_eq!(remote.git_dir, root.join("remotes").join("o").join("n.git"));
    assert_eq!(bare_branches(&remote), ["refs/heads/dev", "refs/heads/main"]);

    let env_var = |key: &str| run.command.get_env(key).map(std::ffi::OsStr::to_owned);
    assert_eq!(
        env_var("GIT_SSH"),
        Some(root.join("bin").join(fake_gh::fake_ssh_file_name()).into())
    );
    assert_eq!(env_var("GIT_SSH_VARIANT"), Some("simple".into()));
    assert_eq!(env_var("FAKE_SSH_ROOT"), Some(root.join("remotes").into()));

    assert_eq!(
        app_git_ok(&run, &["remote", "get-url", "origin"]),
        "git@github.com:o/n.git\n"
    );
    // Pushed by path: the work branch has no upstream.
    assert!(
        !app_git(&run, &["config", "--get", "branch.topic.merge"])
            .status
            .success()
    );
    let main = app_git_ok(&run, &["ls-remote", "origin", "main"]);
    assert!(main.ends_with("\trefs/heads/main\n"), "{main:?}");
    assert_eq!(app_git_ok(&run, &["ls-remote", "origin", "other"]), "");

    assert!(!remote.has_branch("feature"));
    app_git_ok(&run, &["push", "-q", "--set-upstream", "origin", "HEAD:feature"]);
    assert!(remote.has_branch("feature"));
    assert_eq!(
        bare_branches(&remote),
        ["refs/heads/dev", "refs/heads/feature", "refs/heads/main"]
    );
    assert_eq!(
        app_git_ok(&run, &["config", "--get", "branch.topic.merge"]),
        "refs/heads/feature\n"
    );

    // fake-ssh logged the two ls-remotes and the push in the run's request log.
    let log = std::fs::read_to_string(run.log.as_ref().unwrap()).unwrap();
    let services: Vec<String> = log
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["service"].to_string())
        .collect();
    assert_eq!(
        services,
        ["\"git-upload-pack\"", "\"git-upload-pack\"", "\"git-receive-pack\""]
    );
    run.finish().unwrap();
}

#[test]
fn an_empty_branch_list_makes_a_bare_repo_with_no_branches() {
    let tmp = tempfile::tempdir().unwrap();
    let run = prepare(&scenario("[]", ""), &tmp.path().join("out"));
    let remote = run.remote.clone().unwrap();
    assert!(bare_branches(&remote).is_empty());
    assert_eq!(app_git_ok(&run, &["ls-remote", "origin"]), "");
    run.finish().unwrap();
}

#[test]
fn without_the_key_there_is_no_remote_and_no_ssh_env() {
    let tmp = tempfile::tempdir().unwrap();
    let source = "version = 1\n[env]\ngit_remote = \"o/n\"\ngit_remote_protocol = \"ssh\"\n";
    let s = Scenario::parse(Path::new("plain.toml"), source, &roots()).unwrap();
    let run = prepare(&s, &tmp.path().join("out"));
    assert!(run.remote.is_none());
    assert!(run.command.get_env("GIT_SSH").is_none());
    assert!(!run.run_dir.as_ref().unwrap().path().join("remotes").exists());
    run.finish().unwrap();
}

#[test]
fn wait_remote_branch_passes_after_the_push_and_times_out_before_it() {
    let tmp = tempfile::tempdir().unwrap();
    let steps = "[[step]]\nwait = { remote_branch = \"feature\", timeout_ms = 500 }\n";
    let s = scenario("[\"main\"]", steps);
    let run = prepare(&s, &tmp.path().join("out"));
    let remote = run.remote.clone().unwrap();

    let before = tmp.path().join("before");
    std::fs::create_dir_all(&before).unwrap();
    assert_eq!(run_steps(&s, &remote, &before), RunState::Timeout);

    app_git_ok(&run, &["push", "-q", "--set-upstream", "origin", "HEAD:feature"]);
    let after = tmp.path().join("after");
    std::fs::create_dir_all(&after).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(run_steps(&s, &remote, &after), RunState::Ok);
    assert!(started.elapsed() < Duration::from_secs(10));
    run.finish().unwrap();
}

#[test]
fn keep_home_copies_the_remotes() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let s = scenario("[\"main\"]", "");
    let echo = example("echo_tui");
    let paths = Paths {
        fake_gh: echo.clone(),
        rust_app: echo.clone(),
        python: echo,
        roots: roots(),
    };
    std::fs::create_dir_all(&out).unwrap();
    let run = env::prepare(&s, App::Rust, Size { cols: 80, rows: 24 }, &paths, &out, true).unwrap();
    run.finish().unwrap();
    assert!(out.join("remotes").join("o").join("n.git").join("HEAD").is_file());
    assert!(out.join("work").join(".git").is_dir());
}
