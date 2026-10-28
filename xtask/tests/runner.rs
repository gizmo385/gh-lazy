//! Drives the `echo_tui` example through the parity runner: waits, keys, typing, resizing, mouse steps, captures and
//! failures. Needs no Python.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use xtask::parity::env::RunEnv;
use xtask::parity::grid::Grid;
use xtask::parity::runner::{self, AppRunStatus, RunState};
use xtask::parity::scenario::{Roots, Scenario, Size};

/// `cargo test` builds examples next to the test binaries: `target/<profile>/deps/runner-*` and
/// `target/<profile>/examples/echo_tui`.
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

fn scenario(steps: &str) -> Scenario {
    let parity = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity");
    let roots = Roots {
        fixtures: parity.join("fixtures"),
        configs: parity.join("configs"),
        seeds: parity.join("seeds"),
    };
    let source = format!("version = 1\nsizes = [[80, 24]]\ntimeout_ms = 15000\n{steps}");
    Scenario::parse(Path::new("echo.toml"), &source, &roots).unwrap_or_else(|e| panic!("{e}"))
}

fn run(scenario: &Scenario, out: &Path) -> AppRunStatus {
    let mut command = CommandBuilder::new(echo_tui());
    command.env("TERM", "xterm-256color");
    let env = RunEnv::for_command(command, Size { cols: 80, rows: 24 }, scenario.timing);
    runner::run(env, &scenario.steps, out).unwrap()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn waits_keys_typing_resize_and_captures() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario(
        r#"
[[step]]
wait = { text = "kbd false" }

[[step]]
wait = { regex = "(?m)^settled$" }

[[step]]
capture = "start"

[[step]]
keys = ["a", "up", "enter", "ctrl+a", "escape", "f5", "space"]

[[step]]
type = "héllo"

[[step]]
wait = { text = "héllo" }

[[step]]
resize = [60, 15]

[[step]]
wait = { text = "size 60x15" }

[[step]]
wait = { stable_ms = 250 }

[[step]]
capture = "end"
"#,
    );
    let started = Instant::now();
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Ok, "{status:?}");
    assert!(status.passed());
    // The kitty keyboard check returns on the DA1 reply instead of waiting out crossterm's 2 s timeout.
    assert!(started.elapsed() < Duration::from_secs(10));

    let start = read(&out.path().join("start.txt"));
    assert_eq!(start.lines().count(), 24);
    // The cursor position report saw the cursor where the app had moved it.
    assert!(
        start.starts_with("cpr 7,0 kbd false\nsettled\nsize 80x24\nkeys:\n"),
        "{start}"
    );

    let end = read(&out.path().join("end.txt"));
    assert_eq!(end.lines().count(), 15);
    assert!(end.contains("size 60x15"), "{end}");
    assert!(end.contains("keys: a<Up><Enter><C-a><Esc><F(5)> héllo"), "{end}");

    let grid: Grid = serde_json::from_str(&read(&out.path().join("end.grid.json"))).unwrap();
    assert_eq!((grid.cols, grid.rows), (60, 15));
    assert_eq!(grid.to_text(), end);
    assert!(out.path().join("pty.raw").is_file());
    let saved: AppRunStatus = serde_json::from_str(&read(&out.path().join("status.json"))).unwrap();
    assert_eq!(saved.status, RunState::Ok);
    assert!(!out.path().join("failure.txt").exists());
}

#[test]
fn stable_wait_lasts_until_the_screen_stops_changing() {
    let out = tempfile::tempdir().unwrap();
    // The ticks change row 1 every 40 ms for about 320 ms. A 1.5 s stable wait can only finish after them unless the
    // machine stalls the app for longer than that between two ticks, so a slow CI box doesn't make this flaky.
    let s = scenario(
        "[[step]]\nwait = { text = \"cpr\" }\n\n[[step]]\nwait = { stable_ms = 1500 }\n\n[[step]]\ncapture = \"after\"\n",
    );
    let started = Instant::now();
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Ok, "{status:?}");
    // Timing-independent: a stable wait never returns before its duration has passed.
    assert!(started.elapsed() >= Duration::from_millis(1500));
    let after = read(&out.path().join("after.txt"));
    assert!(after.contains("settled"), "{after}");
}

#[test]
fn a_step_timeout_dumps_the_screen() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario(
        "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nwait = { text = \"never shown\", timeout_ms = 300 }\n",
    );
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Timeout, "{status:?}");
    assert_eq!(status.step, Some(1));
    assert!(read(&out.path().join("failure.txt")).contains("settled"));
    let grid: Grid = serde_json::from_str(&read(&out.path().join("failure.grid.json"))).unwrap();
    assert_eq!(grid.rows, 24);
    let saved: serde_json::Value = serde_json::from_str(&read(&out.path().join("status.json"))).unwrap();
    assert_eq!(saved["status"], "timeout");
    assert_eq!(saved["step"], 1);
}

#[test]
fn the_whole_run_is_bounded_by_timeout_ms() {
    let out = tempfile::tempdir().unwrap();
    let mut s = scenario("[[step]]\nsleep_ms = 20000\n");
    s.timing.timeout = Duration::from_millis(500);
    let started = Instant::now();
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Timeout, "{status:?}");
    assert_eq!(status.step, Some(0));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn wait_cap_shortens_condition_waits_but_not_stability_waits() {
    let out = tempfile::tempdir().unwrap();
    let mut s = scenario(
        "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nwait = { stable_ms = 2500 }\n\n[[step]]\nwait = { text = \"never shown\", timeout_ms = 10000 }\n",
    );
    s.timing.wait_cap = Some(Duration::from_secs(2));
    let started = Instant::now();
    let status = run(&s, out.path());
    // The stability wait outlasts the cap and still passes; the text wait gives up at the cap, not after 10 s.
    assert_eq!(status.status, RunState::Timeout, "{status:?}");
    assert_eq!(status.step, Some(2));
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
}

#[test]
fn an_early_exit_is_reported_with_its_code() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario(
        "[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nkeys = [\"ctrl+d\"]\n\n[[step]]\nwait = { text = \"never\" }\n",
    );
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Exited, "{status:?}");
    assert_eq!(status.exit_code, Some(3));
    assert!(matches!(status.step, Some(1 | 2)), "{status:?}");
    assert!(out.path().join("failure.txt").is_file());
}

#[test]
fn a_missing_program_is_a_spawn_error() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario("[[step]]\nsleep_ms = 1\n");
    let command = CommandBuilder::new(out.path().join("no-such-program"));
    let env = RunEnv::for_command(command, Size { cols: 80, rows: 24 }, s.timing);
    let status = runner::run(env, &s.steps, out.path()).unwrap();
    assert_eq!(status.status, RunState::SpawnError, "{status:?}");
    assert!(status.message.is_some());
}

/// `run` with `ECHO_TUI_MOUSE` set: `off` leaves mouse reporting off, `press` turns it on without motion.
fn run_with_mouse(scenario: &Scenario, out: &Path, mouse: &str) -> AppRunStatus {
    let mut command = CommandBuilder::new(echo_tui());
    command.env("TERM", "xterm-256color");
    command.env("ECHO_TUI_MOUSE", mouse);
    let env = RunEnv::for_command(command, Size { cols: 80, rows: 24 }, scenario.timing);
    runner::run(env, &scenario.steps, out).unwrap()
}

#[test]
fn click_wheel_and_move_send_sgr_reports() {
    let out = tempfile::tempdir().unwrap();
    // The text target needs no wait first: it polls until `settled` shows. echo_tui echoes 0-based cells too.
    let s = scenario(
        r#"
[[step]]
click = { text = "settled", offset = [2, 0] }

[[step]]
wait = { text = "mouse: move@2,1 down@2,1 up@2,1" }

[[step]]
wheel = { at = [10, 12], direction = "down", count = 2 }

[[step]]
wait = { text = "up@2,1 move@10,12 wheel-down@10,12 wheel-down@10,12" }

[[step]]
wheel = { at = [10, 12], direction = "up" }

[[step]]
move = { at = [79, 23] }

[[step]]
capture = "mouse"
"#,
    );
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Ok, "{status:?}");
    let screen = read(&out.path().join("mouse.txt"));
    assert!(
        screen.contains("mouse: wheel-down@10,12 move@10,12 wheel-up@10,12 move@79,23\n"),
        "{screen}"
    );
}

#[test]
fn a_mouse_step_fails_when_the_app_has_not_enabled_mouse_reporting() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario("[[step]]\nwait = { text = \"settled\" }\n\n[[step]]\nclick = { at = [0, 0] }\n");
    let status = run_with_mouse(&s, out.path(), "off");
    assert_eq!(status.status, RunState::StepFailed, "{status:?}");
    assert_eq!(status.step, Some(1));
    assert_eq!(
        status.message.as_deref(),
        Some("click: the app hasn't enabled SGR mouse reporting (mode None, encoding Default)")
    );
    assert!(read(&out.path().join("failure.txt")).contains("settled"));
    let saved: serde_json::Value = serde_json::from_str(&read(&out.path().join("status.json"))).unwrap();
    assert_eq!(saved["status"], "step-failed");
}

#[test]
fn motion_reports_are_only_sent_in_any_motion_mode() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario(
        r#"
[[step]]
wait = { text = "settled" }

[[step]]
click = { at = [3, 4] }

[[step]]
wheel = { at = [5, 6], direction = "left" }

[[step]]
wait = { text = "mouse: down@3,4 up@3,4 wheel-left@5,6" }

[[step]]
move = { at = [1, 1] }
"#,
    );
    let status = run_with_mouse(&s, out.path(), "press");
    assert_eq!(status.status, RunState::StepFailed, "{status:?}");
    assert_eq!(status.step, Some(4));
    assert_eq!(
        status.message.as_deref(),
        Some("move: the app reports no mouse motion (mode PressRelease), so a terminal wouldn't send it")
    );
}

#[test]
fn a_mouse_target_off_screen_fails_the_step() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario("[[step]]\nclick = { text = \"settled\", offset = [0, -2] }\n");
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::StepFailed, "{status:?}");
    assert_eq!(
        status.message.as_deref(),
        Some("click: target [0, -1] is off the 80x24 screen")
    );
}

#[test]
fn a_text_target_that_never_shows_times_out() {
    let out = tempfile::tempdir().unwrap();
    let s = scenario("[[step]]\nclick = { text = \"never shown\", timeout_ms = 300 }\n");
    let status = run(&s, out.path());
    assert_eq!(status.status, RunState::Timeout, "{status:?}");
    assert_eq!(status.step, Some(0));
    assert!(out.path().join("failure.txt").is_file());
}
