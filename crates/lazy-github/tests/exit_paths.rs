//! Every way the app can end leaves the terminal as it found it, and no `gh` child running (#329).
//!
//! Each test runs the real binary in a pseudo-terminal, with fake-gh as `gh`. A fixture that answers the repos list
//! after ten minutes keeps a `gh` child in flight when the app is told to stop. The `gh` on `PATH` is a script that
//! records its pid and ignores `SIGHUP`, `SIGINT` and `SIGTERM` before it execs fake-gh, so only the app's
//! `SIGKILL` (`kill_on_drop`) can end it early: the kernel's `SIGHUP` to the terminal's process group when the app
//! exits doesn't count.
//!
//! After the app exits, `stty -a` runs on the same terminal: raw mode must be off. Unix-only: signals are the subject,
//! and the `gh` stand-in is a shell script.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, SlavePty, native_pty_system};

/// How long any one wait may take before the test fails.
const TIMEOUT: Duration = Duration::from_secs(30);

const ROWS: u16 = 40;
const COLS: u16 = 160;

/// What [`TerminalGuard`] writes on restore without the kitty flags, which these runs never push (the DA1 reply
/// says the terminal has no kitty protocol).
///
/// [`TerminalGuard`]: lazy_github_tui::tui::TerminalGuard
const RESTORE: &str =
    "\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h";
const ENTER_ALTERNATE_SCREEN: &str = "\x1b[?1049h";

/// Printed by the shell after the app is gone, so everything before it is the app's.
const AFTER_APP: &str = "__AFTER_APP__";
const STTY_DONE: &str = "__STTY_DONE__";

/// `target/<profile>/examples/<name>`, built by `cargo test`.
fn example(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(Path::parent).unwrap();
    let path = profile.join("examples").join(name);
    assert!(
        path.is_file(),
        "{} is missing; run `cargo build -p lazy-github --examples`",
        path.display()
    );
    path
}

fn fixtures() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sets = [
        manifest.join("tests/fixtures/in-flight"),
        manifest.join("../../parity/fixtures/base"),
    ];
    std::env::join_paths(sets).unwrap().into_string().unwrap()
}

/// A temp dir for one run: home, work dir, `gh` and its logs. Removed on drop, after killing any `gh` left over so a
/// failing test doesn't leave ten-minute sleepers behind.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("lazy-github-exit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["home/.config/lazy-github", "work", "bin"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(
            root.join("home/.config/lazy-github/config.json"),
            r#"{"core": {"first_start": false}, "api": {"client_type": "GITHUB_CLI"}}"#,
        )
        .unwrap();
        Self { root }
    }

    /// Puts the `gh` script on this sandbox's `bin`.
    fn install_gh(&self) {
        let script = format!(
            "#!/bin/sh\necho $$ >> '{pids}'\ntrap '' HUP INT TERM\nexec '{fake_gh}' \"$@\"\n",
            pids = self.pid_file().display(),
            fake_gh = example("fake-gh").display(),
        );
        let gh = self.root.join("bin/gh");
        std::fs::write(&gh, script).unwrap();
        let status = Command::new("chmod").arg("+x").arg(&gh).status().unwrap();
        assert!(status.success());
    }

    fn pid_file(&self) -> PathBuf {
        self.root.join("gh.pids")
    }

    fn log_file(&self) -> PathBuf {
        self.root.join("fake-gh.jsonl")
    }

    fn app_log(&self) -> PathBuf {
        self.root.join("home/.config/lazy-github/lazy_github.log")
    }

    /// The pids of every `gh` started so far.
    fn gh_pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.pid_file())
            .unwrap_or_default()
            .lines()
            .map(|line| line.trim().parse().unwrap())
            .collect()
    }

    /// The app's command, with the sandbox's `bin` and then the system `PATH`, for git: `gh` is only ever ours.
    fn command(&self, extra_env: &[(&str, &str)]) -> CommandBuilder {
        let path = std::env::join_paths(
            std::iter::once(self.root.join("bin"))
                .chain(std::env::var_os("PATH").iter().flat_map(std::env::split_paths)),
        )
        .unwrap();
        self.command_with_path(path.into(), extra_env)
    }

    fn command_with_path(&self, path: PathBuf, extra_env: &[(&str, &str)]) -> CommandBuilder {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_lazy-github"));
        command.env_clear();
        let home = self.root.join("home");
        for (key, value) in [
            ("HOME", home.clone()),
            ("XDG_CONFIG_HOME", home.join(".config")),
            ("XDG_CACHE_HOME", home.join(".cache")),
            ("XDG_STATE_HOME", home.join(".local/state")),
            ("PATH", path),
            ("LAZY_GITHUB_ORIGINAL_PWD", self.root.join("work")),
            ("FAKE_GH_FIXTURES", PathBuf::from(fixtures())),
            ("FAKE_GH_LOG", self.log_file()),
        ] {
            command.env(key, value);
        }
        for (key, value) in [
            ("TERM", "xterm-256color"),
            ("TEXTUAL_ANIMATIONS", "none"),
            ("LANG", "C.UTF-8"),
        ]
        .into_iter()
        .chain(extra_env.iter().copied())
        {
            command.env(key, value);
        }
        command.cwd(self.root.join("work"));
        command
    }

    /// Blocks until fake-gh has logged the repos request, which then sleeps: a `gh` child is in flight.
    fn wait_for_repos_request(&self) {
        let deadline = Instant::now() + TIMEOUT;
        // The log line is written before fake-gh sleeps, so once it's there the child is running.
        while !std::fs::read_to_string(self.log_file()).is_ok_and(|log| log.contains("\"/user/repos\"")) {
            assert!(Instant::now() < deadline, "fake-gh never logged the repos request");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            self.gh_pids().into_iter().any(is_running),
            "the in-flight gh should be running: {:?}",
            self.gh_pids()
        );
    }

    /// Fails unless every `gh` the app started is gone, allowing the kernel a moment to deliver the `SIGKILL`.
    fn assert_no_gh_left(&self) {
        let pids = self.gh_pids();
        assert!(!pids.is_empty(), "the app never ran gh");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let running: Vec<u32> = pids.iter().copied().filter(|&pid| is_running(pid)).collect();
            if running.is_empty() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "gh children still running after the app exited: {running:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for pid in self.gh_pids() {
            // Most are gone already, which `kill` reports on stderr.
            let _ = Command::new("kill")
                .args(["-s", "KILL", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `ps` state of `pid`, or `None` once it's gone. A zombie counts as gone: it has exited and only waits for reaping.
fn process_state(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !state.is_empty() && !state.starts_with('Z')).then_some(state)
}

fn is_running(pid: u32) -> bool {
    process_state(pid).is_some()
}

fn signal(pid: u32, name: &str) {
    let status = Command::new("kill")
        .args(["-s", name, &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "kill -s {name} {pid}");
}

/// Everything the terminal has received, with a condvar to wait on.
#[derive(Default)]
struct Output {
    bytes: Mutex<Vec<u8>>,
    grew: Condvar,
}

/// The app running in a pseudo-terminal.
struct Pty {
    pid: u32,
    output: Arc<Output>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    exit: mpsc::Receiver<u32>,
    // Kept open so the terminal outlives the app, for `stty`.
    slave: Box<dyn SlavePty + Send>,
    _master: Box<dyn MasterPty + Send>,
}

impl Pty {
    fn spawn(command: CommandBuilder) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut child = pair.slave.spawn_command(command).unwrap();
        let pid = child.process_id().unwrap();
        let (exit_tx, exit) = mpsc::channel();
        std::thread::spawn(move || {
            let status = child.wait().unwrap();
            let _ = exit_tx.send(status.exit_code());
        });
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let output = Arc::new(Output::default());
        let mut reader = pair.master.try_clone_reader().unwrap();
        {
            let (output, writer) = (Arc::clone(&output), Arc::clone(&writer));
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                // Queries answered so far, by kind.
                let mut answered = [0; 2];
                while let Ok(n @ 1..) = reader.read(&mut buf) {
                    let mut bytes = output.bytes.lock().unwrap();
                    bytes.extend_from_slice(&buf[..n]);
                    // crossterm asks for the kitty protocol and then DA1; answering DA1 alone says "no kitty"
                    // without waiting out its timeout. The cursor position is asked at startup; any answer does.
                    let replies: [(&str, &[u8]); 2] = [("\x1b[c", b"\x1b[?62;22c"), ("\x1b[6n", b"\x1b[1;1R")];
                    for ((query, reply), answered) in replies.into_iter().zip(&mut answered) {
                        let asked = count(&bytes, query);
                        for _ in *answered..asked {
                            let _ = writer.lock().unwrap().write_all(reply);
                        }
                        *answered = asked;
                    }
                    output.grew.notify_all();
                }
            });
        }
        Self {
            pid,
            output,
            writer,
            exit,
            slave: pair.slave,
            _master: pair.master,
        }
    }

    fn send(&self, keys: &str) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(keys.as_bytes()).unwrap();
        writer.flush().unwrap();
    }

    /// Blocks until `done` holds for the output so far, and returns that output.
    fn wait_output(&self, what: &str, done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + TIMEOUT;
        let mut bytes = self.output.bytes.lock().unwrap();
        loop {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if done(&text) {
                return text;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}; output: {text:?}");
            bytes = self.output.grew.wait_timeout(bytes, left).unwrap().0;
        }
    }

    /// Blocks until the screen, as a terminal would show it, contains `text`.
    fn wait_screen(&self, text: &str) {
        self.wait_output(&format!("{text:?} on screen"), |out| {
            let size = |n| std::num::NonZeroU16::new(n).unwrap();
            let mut parser = vt100::Parser::new(size(ROWS), size(COLS), 0);
            parser.process(out.as_bytes());
            parser.screen().contents().contains(text)
        });
    }

    /// Blocks until the app has exited, and returns its exit code, everything it wrote and the terminal's `stty -a`
    /// afterwards.
    fn finish(self) -> Finished {
        let code = self
            .exit
            .recv_timeout(TIMEOUT)
            .unwrap_or_else(|_| panic!("the app didn't exit; output: {:?}", self.output_text()));
        let mut stty = CommandBuilder::new("sh");
        stty.args(["-c", &format!("echo {AFTER_APP}; stty -a; echo {STTY_DONE}")]);
        let mut child = self.slave.spawn_command(stty).unwrap();
        child.wait().unwrap();
        let out = self.wait_output("stty", |out| out.contains(&format!("{STTY_DONE}\r\n")));
        let (app, after) = out.split_once(AFTER_APP).unwrap();
        Finished {
            code,
            output: app.to_owned(),
            stty: after.to_owned(),
        }
    }

    fn output_text(&self) -> String {
        String::from_utf8_lossy(&self.output.bytes.lock().unwrap()).into_owned()
    }
}

fn count(haystack: &[u8], needle: &str) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle.as_bytes())
        .count()
}

struct Finished {
    code: u32,
    /// Everything the app wrote to the terminal.
    output: String,
    /// `stty -a` once the app was gone.
    stty: String,
}

impl Finished {
    /// The terminal is out of raw mode: line editing, echo and signal keys are back.
    fn assert_cooked(&self) {
        let flags: Vec<&str> = self.stty.split([' ', ';', '\r', '\n', '\t']).collect();
        for flag in ["icanon", "echo", "isig"] {
            assert!(flags.contains(&flag), "{flag} is off after exit: {}", self.stty);
        }
    }

    /// The app's last bytes are the restore sequence, with nothing after it: the screen, cursor, mouse, paste and
    /// focus modes are all back.
    fn assert_ends_restored(&self) {
        assert!(
            self.output.trim_end_matches(['\r', '\n']).ends_with(RESTORE),
            "the output doesn't end with the restore sequence: {:?}",
            tail(&self.output)
        );
        self.assert_cooked();
    }
}

fn tail(text: &str) -> &str {
    let start = text.len().saturating_sub(400);
    let start = (start..text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    &text[start..]
}

/// Starts the app with a `gh` call in flight.
fn start_in_flight(sandbox: &Sandbox, extra_env: &[(&str, &str)]) -> Pty {
    sandbox.install_gh();
    let pty = Pty::spawn(sandbox.command(extra_env));
    sandbox.wait_for_repos_request();
    pty
}

#[test]
fn quitting_with_q_restores_and_kills_the_gh_child() {
    let sandbox = Sandbox::new("q");
    let pty = start_in_flight(&sandbox, &[]);
    pty.send("q");
    let finished = pty.finish();
    assert_eq!(finished.code, 0);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

#[test]
fn quitting_from_the_palette_restores_and_kills_the_gh_child() {
    let sandbox = Sandbox::new("palette");
    let pty = start_in_flight(&sandbox, &[]);
    pty.send("\x10");
    pty.wait_screen("Search for commands");
    pty.send("Quit");
    pty.wait_screen("Quit the application");
    pty.send("\r");
    let finished = pty.finish();
    assert_eq!(finished.code, 0);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

/// `ctrl+c` is a key in raw mode, so it shows Textual's hint, as Python does. `SIGINT` from another process quits as
/// `q` does.
#[test]
fn ctrl_c_hints_and_sigint_quits_with_the_gh_child_killed() {
    let sandbox = Sandbox::new("int");
    let pty = start_in_flight(&sandbox, &[]);
    pty.send("\x03");
    pty.wait_screen("to quit the app");
    signal(pty.pid, "INT");
    let finished = pty.finish();
    // Python's Textual app quits normally on `SIGINT` too.
    assert_eq!(finished.code, 0);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

#[test]
fn sigterm_restores_and_exits_143() {
    let sandbox = Sandbox::new("term");
    let pty = start_in_flight(&sandbox, &[]);
    signal(pty.pid, "TERM");
    let finished = pty.finish();
    assert_eq!(finished.code, 143);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

#[test]
fn sighup_restores_and_exits_129() {
    let sandbox = Sandbox::new("hup");
    let pty = start_in_flight(&sandbox, &[]);
    signal(pty.pid, "HUP");
    let finished = pty.finish();
    assert_eq!(finished.code, 129);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

/// `SIGTSTP` gives the terminal back and stops; `SIGCONT` takes it again and redraws, as Textual's driver does.
#[test]
fn sigtstp_restores_and_sigcont_resumes() {
    let sandbox = Sandbox::new("tstp");
    let pty = start_in_flight(&sandbox, &[]);
    pty.wait_screen("Repositories");
    signal(pty.pid, "TSTP");
    let suspended = pty.wait_output("the restore", |out| out.contains(RESTORE));
    assert_eq!(count(suspended.as_bytes(), ENTER_ALTERNATE_SCREEN), 1);
    // Continued only once it has stopped: a `SIGCONT` that came first would leave it stopped for good.
    let deadline = Instant::now() + TIMEOUT;
    while !process_state(pty.pid).is_some_and(|state| state.starts_with('T')) {
        assert!(Instant::now() < deadline, "the app never stopped");
        std::thread::sleep(Duration::from_millis(20));
    }
    signal(pty.pid, "CONT");
    pty.wait_output("the app to take the terminal again", |out| {
        count(out.as_bytes(), ENTER_ALTERNATE_SCREEN) == 2
    });
    // The full redraw after resuming puts the screen back.
    pty.wait_output("a redraw", |out| {
        let resumed = &out[out.rfind(ENTER_ALTERNATE_SCREEN).unwrap()..];
        resumed.contains("Repositories")
    });
    pty.send("q");
    let finished = pty.finish();
    assert_eq!(finished.code, 0);
    finished.assert_ends_restored();
    sandbox.assert_no_gh_left();
}

/// A panic inside a dialog, from the debug-only trigger: the terminal is restored before the report, which names the
/// log file.
fn assert_panic_in_a_dialog_restores(place: &str, debug_name: &str) {
    let sandbox = Sandbox::new(&format!("panic-{place}"));
    let pty = start_in_flight(&sandbox, &[("LAZY_GITHUB_TEST_PANIC", place)]);
    pty.wait_screen("Repositories");
    // The palette opens on `ctrl+p`; the next draw, or the next key, panics.
    pty.send("\x10x");
    let finished = pty.finish();
    assert_eq!(finished.code, 101);
    let output = &finished.output;
    let restored = output
        .find(RESTORE)
        .expect("the panic hook should restore the terminal");
    let report = output.find("panicked").expect("the panic should be reported");
    assert!(
        restored < report,
        "the report came before the restore: {:?}",
        tail(output)
    );
    let message = format!("LAZY_GITHUB_TEST_PANIC asked for a panic in {debug_name}");
    assert!(
        output[report..].contains(&message),
        "no {message:?} in {:?}",
        tail(output)
    );
    let log_line = format!("The log file is {}", sandbox.app_log().display());
    assert!(
        output[report..].contains(&log_line),
        "no {log_line:?} in {:?}",
        tail(output)
    );
    assert_eq!(count(output.as_bytes(), RESTORE), 1, "restored more than once");
    finished.assert_cooked();
    sandbox.assert_no_gh_left();
}

#[test]
fn a_panic_while_updating_a_dialog_restores_first() {
    assert_panic_in_a_dialog_restores("update", "Update");
}

#[test]
fn a_panic_while_drawing_a_dialog_restores_first() {
    assert_panic_in_a_dialog_restores("render", "Render");
}

/// A config that doesn't load stops the app before it touches the terminal, so the error lands on the normal screen.
#[test]
fn a_config_error_is_printed_on_an_untouched_terminal() {
    let sandbox = Sandbox::new("config");
    std::fs::write(sandbox.root.join("home/.config/lazy-github/config.json"), "{ not json").unwrap();
    let pty = Pty::spawn(sandbox.command(&[]));
    let finished = pty.finish();
    assert_eq!(finished.code, 1);
    assert!(finished.output.contains("Error:"), "{:?}", finished.output);
    assert!(
        !finished.output.contains(ENTER_ALTERNATE_SCREEN),
        "{:?}",
        finished.output
    );
    finished.assert_cooked();
}

/// Without `gh` the app shows its "not installed" screen; quitting from there restores as from anywhere else.
#[test]
fn quitting_when_gh_is_missing_restores() {
    let sandbox = Sandbox::new("no-gh");
    // Only the sandbox's empty `bin`, so a `gh` installed on the machine isn't found either.
    let pty = Pty::spawn(sandbox.command_with_path(sandbox.root.join("bin"), &[]));
    pty.wait_screen("installed");
    pty.send("q");
    let finished = pty.finish();
    assert_eq!(finished.code, 0);
    finished.assert_ends_restored();
}
