//! Runs one app in a pseudo-terminal, replays a scenario's steps against it and dumps the screen at each capture.
//!
//! The runner is synchronous: a reader thread feeds the PTY's output into a shared `vt100::Parser` (answering the
//! terminal queries `vt100` doesn't), and the calling thread runs the steps, polling the parser every
//! [`POLL`].

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use fake_gh::log::{LogEntry, RequestLog};
use portable_pty::{Child, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use super::env::RunEnv;
use super::grid::{Grid, screen_text};
use super::keys;
use super::scenario::{DEFAULT_WAIT_TIMEOUT, Size, Step, WaitFor};
use super::{Context, Result};

/// How often waits look at the screen. Nothing in the runner sleeps longer than this between checks.
pub const POLL: Duration = Duration::from_millis(20);
/// The minimum pause after `escape`, so Textual's escape-sequence timeout doesn't fuse it with the next key.
pub const ESCAPE_DELAY: Duration = Duration::from_millis(100);
/// How long a killed app gets to exit, and the reader thread to drain, before the runner moves on.
const KILL_WAIT: Duration = Duration::from_secs(2);
/// How long a failed run waits for the app's last output before dumping the screen.
const DRAIN_WAIT: Duration = Duration::from_millis(200);

/// How an app run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// Every step ran.
    Ok,
    /// A step, or the whole run, took too long.
    Timeout,
    /// The app exited before the last step.
    Exited,
    /// The PTY couldn't be opened or the app couldn't be started.
    SpawnError,
}

/// `status.json`: the outcome of one app run at one size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRunStatus {
    pub status: RunState,
    /// The index of the step that failed.
    pub step: Option<usize>,
    pub exit_code: Option<u32>,
    pub duration_ms: u64,
    /// A retry passed after this run failed (`--retries`).
    pub flaky: bool,
    /// fake-gh logged misses or errors, so the run fails even though every step ran.
    pub fixture_problems: bool,
    /// fake-gh log lines with `"miss": true`: the requests that need fixtures.
    pub misses: Vec<LogEntry>,
    /// fake-gh log lines with an `error`.
    pub errors: Vec<LogEntry>,
    /// Why the run failed, for humans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Read from a recorded reference instead of run: nothing was started, and `duration_ms` is 0. Left out of the
    /// JSON when false, so a live run's status looks as it always did.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recorded: bool,
}

impl AppRunStatus {
    fn new(status: RunState, step: Option<usize>, exit_code: Option<u32>) -> Self {
        Self {
            status,
            step,
            exit_code,
            duration_ms: 0,
            flaky: false,
            fixture_problems: false,
            misses: Vec::new(),
            errors: Vec::new(),
            message: None,
            recorded: false,
        }
    }

    /// The status of a Python side read from a recorded reference: `ok`, which recording guarantees.
    pub fn recorded() -> Self {
        Self {
            recorded: true,
            ..Self::new(RunState::Ok, None, None)
        }
    }

    /// Whether this run counts as passing: every step ran and fake-gh had no problems.
    pub fn passed(&self) -> bool {
        self.status == RunState::Ok && !self.fixture_problems
    }

    /// A one-line description for the console.
    pub fn describe(&self) -> String {
        if self.recorded {
            return "recorded".to_owned();
        }
        let secs = Duration::from_millis(self.duration_ms).as_secs_f64();
        let mut s = match self.status {
            RunState::Ok if self.fixture_problems => {
                format!(
                    "fixture problems ({} misses, {} errors)",
                    self.misses.len(),
                    self.errors.len()
                )
            }
            RunState::Ok => "ok".to_owned(),
            RunState::Timeout => format!("timeout at step {}", fmt_step(self.step)),
            RunState::Exited => match self.exit_code {
                Some(code) => format!("exited with code {code} at step {}", fmt_step(self.step)),
                None => format!("exited at step {}", fmt_step(self.step)),
            },
            RunState::SpawnError => "spawn error".to_owned(),
        };
        let _ = write!(s, " ({secs:.1}s)");
        if self.flaky {
            s.push_str(", flaky");
        }
        if let Some(message) = &self.message {
            let _ = write!(s, ": {message}");
        }
        s
    }
}

fn fmt_step(step: Option<usize>) -> String {
    step.map_or_else(|| "?".to_owned(), |s| s.to_string())
}

/// Runs `steps` against the app `env` describes and writes `status.json`, `pty.raw`, the captures and, on failure,
/// `failure.txt`/`failure.grid.json` into `out`. Errors are harness failures (the report dir can't be written);
/// everything the app does wrong is in the returned status.
pub fn run(env: RunEnv, steps: &[Step], out: &Path) -> Result<AppRunStatus> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let started = Instant::now();
    let mut status = match Session::start(&env, out) {
        Err(e) => AppRunStatus {
            message: Some(e),
            ..AppRunStatus::new(RunState::SpawnError, None, None)
        },
        Ok(mut session) => {
            let result = session.run_steps(steps, out);
            let status = match result {
                Ok(()) => AppRunStatus::new(RunState::Ok, None, None),
                Err((index, StepError::Timeout)) => {
                    session.dump_failure(out)?;
                    AppRunStatus::new(RunState::Timeout, Some(index), None)
                }
                Err((index, StepError::Exited(code))) => {
                    session.dump_failure(out)?;
                    AppRunStatus::new(RunState::Exited, Some(index), code)
                }
                // Dropping the session on the way out stops the app.
                Err((_, StepError::Io(e))) => return Err(e),
            };
            session.stop();
            status
        }
    };
    status.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    if let Some(log) = &env.log {
        let text = match std::fs::read_to_string(log) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", log.display())),
        };
        let log = RequestLog::parse(&text);
        status.misses = log.entries().iter().filter(|e| e.miss).cloned().collect();
        status.errors = log.entries().iter().filter(|e| e.error.is_some()).cloned().collect();
        status.fixture_problems = !status.misses.is_empty() || !status.errors.is_empty();
    }

    // status.json goes first, so a failed --keep-home copy still leaves a complete report.
    write_status(&status, out)?;
    if let Err(e) = env.finish() {
        status.message = Some(format!("--keep-home copy failed: {e}"));
        write_status(&status, out)?;
        return Err(e);
    }
    Ok(status)
}

/// Writes `status.json` into `out`.
pub fn write_status(status: &AppRunStatus, out: &Path) -> Result<()> {
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(status).expect("status serializes");
    std::fs::write(out.join("status.json"), json + "\n")
        .with_context(|| format!("writing status.json in {}", out.display()))
}

/// Why a step stopped the run.
enum StepError {
    Timeout,
    Exited(Option<u32>),
    Io(super::HarnessError),
}

/// Bytes for the writer thread, which owns the PTY's input side. Both keystrokes and query replies go through it,
/// so neither the step loop nor the reader thread ever blocks on a write the app isn't reading.
type Writer = mpsc::Sender<Vec<u8>>;

/// A running app.
struct Session {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn Child + Send + Sync>,
    /// `None` once stopped, which ends the writer thread.
    writer: Option<Writer>,
    parser: Arc<Mutex<vt100::Parser>>,
    reader_done: Arc<AtomicBool>,
    stopped: bool,
    /// The end of the whole run's `timeout_ms`.
    deadline: Instant,
    /// [`Timing::wait_cap`](super::scenario::Timing::wait_cap).
    wait_cap: Option<Duration>,
    env_log: Option<std::path::PathBuf>,
    timing: super::scenario::Timing,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic in the reader thread leaves the parser usable; keep going so the run still gets a status.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn nonzero(n: u16) -> NonZeroU16 {
    // Sizes are validated above 0 when scenarios and flags are parsed.
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Session {
    fn start(env: &RunEnv, out: &Path) -> std::result::Result<Self, String> {
        let pair = native_pty_system()
            .openpty(pty_size(env.size))
            .map_err(|e| format!("opening a PTY: {e}"))?;
        let child = pair.slave.spawn_command(env.command.clone()).map_err(|e| {
            format!(
                "starting {}: {e}",
                env.command
                    .get_argv()
                    .first()
                    .map_or(String::new(), |a| a.to_string_lossy().into_owned())
            )
        })?;
        // The child has its own handle; ours would keep the master from seeing EOF when the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reading the PTY: {e}"))?;
        let pty_writer = pair.master.take_writer().map_err(|e| format!("writing the PTY: {e}"))?;
        let (writer, queue) = mpsc::channel();
        std::thread::spawn(move || write_input(pty_writer, &queue));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            nonzero(env.size.rows),
            nonzero(env.size.cols),
            0,
        )));
        let raw = File::create(out.join("pty.raw")).map_err(|e| format!("creating pty.raw: {e}"))?;
        let reader_done = Arc::new(AtomicBool::new(false));
        {
            let (parser, writer, done) = (Arc::clone(&parser), writer.clone(), Arc::clone(&reader_done));
            std::thread::spawn(move || read_output(reader, raw, &parser, &writer, &done));
        }
        Ok(Self {
            master: Some(pair.master),
            child,
            writer: Some(writer),
            parser,
            reader_done,
            stopped: false,
            deadline: Instant::now() + env.timing.timeout,
            wait_cap: env.timing.wait_cap,
            env_log: env.log.clone(),
            timing: env.timing,
        })
    }

    fn run_steps(&mut self, steps: &[Step], out: &Path) -> std::result::Result<(), (usize, StepError)> {
        for (index, step) in steps.iter().enumerate() {
            self.check_alive().map_err(|e| (index, e))?;
            self.step(step, out).map_err(|e| (index, e))?;
        }
        Ok(())
    }

    fn step(&mut self, step: &Step, out: &Path) -> std::result::Result<(), StepError> {
        match step {
            Step::Wait(wait) => self.wait(&wait.until, wait.timeout),
            Step::Keys(specs) => {
                for spec in specs {
                    let app_cursor = lock(&self.parser).screen().application_cursor();
                    // Specs were validated when the scenario was parsed; DECCKM doesn't change validity.
                    let bytes = keys::encode(spec, app_cursor).unwrap_or_default();
                    self.send(&bytes)?;
                    let delay = if keys::is_escape(spec) {
                        self.timing.key_delay.max(ESCAPE_DELAY)
                    } else {
                        self.timing.key_delay
                    };
                    self.pause(delay)?;
                }
                Ok(())
            }
            Step::Type(text) => {
                self.send(text.as_bytes())?;
                self.pause(self.timing.key_delay)
            }
            Step::Paste(text) => {
                if lock(&self.parser).screen().bracketed_paste() {
                    self.send(&[b"\x1b[200~", text.as_bytes(), b"\x1b[201~"].concat())?;
                } else {
                    self.send(text.as_bytes())?;
                }
                self.pause(self.timing.key_delay)
            }
            Step::Capture(capture) => {
                self.wait(&WaitFor::Stable(self.timing.capture_stable), DEFAULT_WAIT_TIMEOUT)?;
                let grid = Grid::from_screen(lock(&self.parser).screen());
                write_grid(&grid, out, &capture.name).map_err(StepError::Io)
            }
            Step::Sleep(duration) => self.pause(*duration),
            Step::Resize(size) => {
                if let Some(master) = &self.master {
                    master
                        .resize(pty_size(*size))
                        .map_err(|e| StepError::Io(super::HarnessError(format!("resizing the PTY to {size}: {e}"))))?;
                }
                lock(&self.parser).set_size(nonzero(size.rows), nonzero(size.cols));
                Ok(())
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> std::result::Result<(), StepError> {
        let sent = self.writer.as_ref().is_some_and(|w| w.send(bytes.to_vec()).is_ok());
        if !sent {
            // The writer thread stops when a write fails, which normally means the app is gone.
            self.check_alive()?;
            return Err(StepError::Io(super::HarnessError("writing to the PTY failed".into())));
        }
        Ok(())
    }

    /// Fails with [`StepError::Exited`] once the app has exited.
    fn check_alive(&mut self) -> std::result::Result<(), StepError> {
        match self.child.try_wait() {
            Ok(Some(status)) => Err(StepError::Exited(Some(status.exit_code()))),
            Ok(None) => Ok(()),
            Err(_) => Err(StepError::Exited(None)),
        }
    }

    /// Waits `duration`, checking every [`POLL`] that the app is alive and the run hasn't timed out.
    fn pause(&mut self, duration: Duration) -> std::result::Result<(), StepError> {
        let end = Instant::now() + duration;
        loop {
            self.check_alive()?;
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            if now >= self.deadline {
                return Err(StepError::Timeout);
            }
            std::thread::sleep(POLL.min(end - now));
        }
    }

    fn wait(&mut self, until: &WaitFor, timeout: Duration) -> std::result::Result<(), StepError> {
        let timeout = match (until, self.wait_cap) {
            (WaitFor::Stable(_), _) | (_, None) => timeout,
            (_, Some(cap)) => timeout.min(cap),
        };
        let end = (Instant::now() + timeout).min(self.deadline);
        let mut stable: Option<(String, Instant)> = None;
        loop {
            let done = match until {
                WaitFor::Text(needle) => self.text().lines().any(|row| row.contains(needle.as_str())),
                WaitFor::Gone(needle) => !self.text().lines().any(|row| row.contains(needle.as_str())),
                WaitFor::Regex(re) => re.is_match(&self.text()),
                WaitFor::Stable(duration) => {
                    let text = self.text();
                    let now = Instant::now();
                    match &stable {
                        Some((last, since)) if *last == text => now.duration_since(*since) >= *duration,
                        _ => {
                            stable = Some((text, now));
                            duration.is_zero()
                        }
                    }
                }
                WaitFor::Log(matcher) => self.env_log.as_ref().is_some_and(|path| {
                    std::fs::read_to_string(path).is_ok_and(|text| RequestLog::parse(&text).has_served(matcher))
                }),
            };
            if done {
                return Ok(());
            }
            self.check_alive()?;
            let now = Instant::now();
            if now >= end {
                return Err(StepError::Timeout);
            }
            std::thread::sleep(POLL.min(end - now));
        }
    }

    fn text(&self) -> String {
        screen_text(lock(&self.parser).screen())
    }

    fn dump_failure(&self, out: &Path) -> Result<()> {
        // Let the last output (a traceback, say) reach the parser before dumping.
        let end = Instant::now() + DRAIN_WAIT;
        while !self.reader_done.load(Ordering::Acquire) && Instant::now() < end {
            std::thread::sleep(POLL);
        }
        let grid = Grid::from_screen(lock(&self.parser).screen());
        write_grid(&grid, out, "failure")
    }

    /// Kills the app, waits up to [`KILL_WAIT`] for it, and closes the PTY. Safe to call more than once.
    fn stop(&mut self) {
        if std::mem::replace(&mut self.stopped, true) {
            return;
        }
        // Already exited is fine.
        let _ = self.child.kill();
        let end = Instant::now() + KILL_WAIT;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < end {
            std::thread::sleep(POLL);
        }
        self.master = None;
        self.writer = None;
        // The reader sees EOF once the PTY is closed, unless a grandchild still holds the terminal; in that case the
        // thread is left to finish on its own.
        let end = Instant::now() + KILL_WAIT;
        while !self.reader_done.load(Ordering::Acquire) && Instant::now() < end {
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Session {
    /// Every way out of a run, including `?` and panics, kills the app: dropping a portable-pty child doesn't.
    fn drop(&mut self) {
        self.stop();
    }
}

/// The writer thread: sends queued input to the app until the session hangs up or a write fails.
fn write_input(mut pty: Box<dyn Write + Send>, queue: &mpsc::Receiver<Vec<u8>>) {
    for bytes in queue {
        if pty.write_all(&bytes).and_then(|()| pty.flush()).is_err() {
            break;
        }
    }
}

fn write_grid(grid: &Grid, out: &Path, name: &str) -> Result<()> {
    std::fs::write(out.join(format!("{name}.txt")), grid.to_text())
        .with_context(|| format!("writing {name}.txt in {}", out.display()))?;
    std::fs::write(out.join(format!("{name}.grid.json")), grid.to_json() + "\n")
        .with_context(|| format!("writing {name}.grid.json in {}", out.display()))
}

/// The reader thread: PTY output → `pty.raw` and the parser, with query replies written back.
fn read_output(
    mut reader: Box<dyn Read + Send>,
    mut raw: File,
    parser: &Mutex<vt100::Parser>,
    writer: &Writer,
    done: &AtomicBool,
) {
    let mut responder = QueryResponder::default();
    let mut buf = [0u8; 8192];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO on Linux once the child side is closed.
            Err(_) => break,
        };
        let chunk = &buf[..n];
        // pty.raw is for debugging; losing it isn't worth failing the run over.
        let _ = raw.write_all(chunk);
        let mut parser = lock(parser);
        let mut pos = 0;
        for (end, query) in responder.feed(chunk) {
            // Process up to the query first, so a cursor-position reply sees the cursor where the app left it.
            parser.process(&chunk[pos..end]);
            pos = end;
            if let Some(reply) = query.reply(parser.screen().cursor_position()) {
                // Queued, never written here: a send only fails once the writer thread is gone with the app.
                let _ = writer.send(reply);
            }
        }
        parser.process(&chunk[pos..]);
    }
    done.store(true, Ordering::Release);
}

/// A terminal query found in the app's output.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Query {
    /// `ESC[6n`
    CursorPosition,
    /// `ESC[c` / `ESC[0c`
    PrimaryAttributes,
    /// `ESC[>c` / `ESC[>0c`
    SecondaryAttributes,
    /// `ESC[?<n>$p`
    PrivateMode(String),
    /// `ESC[?u`: the kitty keyboard protocol query, deliberately unanswered. crossterm follows it with a DA1
    /// query, and answering DA1 alone tells it the protocol isn't supported without waiting out its timeout.
    KittyKeyboard,
}

impl Query {
    /// The reply bytes, given the parser's 0-based cursor `(row, col)`.
    fn reply(&self, (row, col): (u16, u16)) -> Option<Vec<u8>> {
        match self {
            Query::CursorPosition => Some(format!("\x1b[{};{}R", u32::from(row) + 1, u32::from(col) + 1).into_bytes()),
            Query::PrimaryAttributes => Some(b"\x1b[?62;22c".to_vec()),
            Query::SecondaryAttributes => Some(b"\x1b[>0;10;1c".to_vec()),
            Query::PrivateMode(n) => Some(format!("\x1b[?{n};0$y").into_bytes()),
            Query::KittyKeyboard => None,
        }
    }
}

/// Finds terminal queries in a byte stream. It's a small CSI state machine, so a query split across reads is found
/// in the read that completes it.
#[derive(Debug, Default)]
struct QueryResponder {
    state: ScanState,
}

#[derive(Debug, Default)]
enum ScanState {
    #[default]
    Ground,
    Escape,
    /// Inside `ESC [`, holding the parameter and intermediate bytes so far.
    Csi(Vec<u8>),
}

/// Longer CSI sequences are never queries we answer, so the scanner gives up on them.
const MAX_CSI: usize = 32;

impl QueryResponder {
    /// Returns each query completed in `bytes`, with the offset just past its final byte.
    fn feed(&mut self, bytes: &[u8]) -> Vec<(usize, Query)> {
        let mut found = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            self.state = match std::mem::take(&mut self.state) {
                _ if b == 0x1b => ScanState::Escape,
                ScanState::Escape if b == b'[' => ScanState::Csi(Vec::new()),
                ScanState::Ground | ScanState::Escape => ScanState::Ground,
                ScanState::Csi(mut params) => match b {
                    0x20..=0x3f if params.len() < MAX_CSI => {
                        params.push(b);
                        ScanState::Csi(params)
                    }
                    0x40..=0x7e => {
                        if let Some(query) = classify(&params, b) {
                            found.push((i + 1, query));
                        }
                        ScanState::Ground
                    }
                    _ => ScanState::Ground,
                },
            };
        }
        found
    }
}

fn classify(params: &[u8], final_byte: u8) -> Option<Query> {
    match (params, final_byte) {
        (b"6", b'n') => Some(Query::CursorPosition),
        (b"" | b"0", b'c') => Some(Query::PrimaryAttributes),
        (b">" | b">0", b'c') => Some(Query::SecondaryAttributes),
        (b"?", b'u') => Some(Query::KittyKeyboard),
        ([b'?', mode @ .., b'$'], b'p') if !mode.is_empty() && mode.iter().all(u8::is_ascii_digit) => {
            // ASCII digits are valid UTF-8.
            Some(Query::PrivateMode(String::from_utf8_lossy(mode).into_owned()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_cursor_position_request_split_across_reads() {
        let mut r = QueryResponder::default();
        assert_eq!(r.feed(b"hello\x1b["), []);
        assert_eq!(r.feed(b"6nworld"), [(2, Query::CursorPosition)]);
        assert_eq!(Query::CursorPosition.reply((4, 9)), Some(b"\x1b[5;10R".to_vec()));

        // Split right after ESC too.
        let mut r = QueryResponder::default();
        assert_eq!(r.feed(b"\x1b"), []);
        assert_eq!(r.feed(b"[6n"), [(3, Query::CursorPosition)]);
    }

    #[test]
    fn recognizes_each_query() {
        let mut r = QueryResponder::default();
        let found = r.feed(b"\x1b[c\x1b[0c\x1b[>c\x1b[>0c\x1b[?2026$p\x1b[?u\x1b[6n");
        let queries: Vec<_> = found.into_iter().map(|(_, q)| q).collect();
        assert_eq!(
            queries,
            [
                Query::PrimaryAttributes,
                Query::PrimaryAttributes,
                Query::SecondaryAttributes,
                Query::SecondaryAttributes,
                Query::PrivateMode("2026".into()),
                Query::KittyKeyboard,
                Query::CursorPosition,
            ]
        );
        assert_eq!(Query::PrimaryAttributes.reply((0, 0)), Some(b"\x1b[?62;22c".to_vec()));
        assert_eq!(
            Query::SecondaryAttributes.reply((0, 0)),
            Some(b"\x1b[>0;10;1c".to_vec())
        );
        assert_eq!(
            Query::PrivateMode("2026".into()).reply((0, 0)),
            Some(b"\x1b[?2026;0$y".to_vec())
        );
        assert_eq!(Query::KittyKeyboard.reply((0, 0)), None);
    }

    #[test]
    fn ignores_other_sequences() {
        let mut r = QueryResponder::default();
        assert_eq!(
            r.feed(b"\x1b[1;31m\x1b[2J\x1b[?1049h\x1b[5n\x1b[6;1n\x1b[?$p\x1b]0;title\x07\x1bOP"),
            []
        );
        // An ESC inside a CSI starts a new sequence.
        assert_eq!(r.feed(b"\x1b[12\x1b[6n"), [(8, Query::CursorPosition)]);
    }

    #[test]
    fn status_json_shape() {
        let status = AppRunStatus {
            status: RunState::Timeout,
            step: Some(2),
            exit_code: None,
            duration_ms: 1500,
            flaky: false,
            fixture_problems: false,
            misses: Vec::new(),
            errors: Vec::new(),
            message: None,
            recorded: false,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"status": "timeout", "step": 2, "exit_code": null, "duration_ms": 1500, "flaky": false,
                "fixture_problems": false, "misses": [], "errors": []})
        );
        assert_eq!(serde_json::to_value(RunState::SpawnError).unwrap(), "spawn-error");
        assert!(!status.passed());
        assert_eq!(status.describe(), "timeout at step 2 (1.5s)");

        let recorded = AppRunStatus::recorded();
        let json = serde_json::to_value(&recorded).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"status": "ok", "step": null, "exit_code": null, "duration_ms": 0, "flaky": false,
                "fixture_problems": false, "misses": [], "errors": [], "recorded": true})
        );
        assert_eq!(serde_json::from_value::<AppRunStatus>(json).unwrap(), recorded);
        assert!(recorded.passed());
        assert_eq!(recorded.describe(), "recorded");
    }
}
