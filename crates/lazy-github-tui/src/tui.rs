//! Terminal setup and teardown, the panic hook, and the event loop.

use std::io::{self, IsTerminal as _, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use color_eyre::eyre::bail;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange,
    EnableMouseCapture, EventStream, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use futures_util::StreamExt as _;
use lazy_github_core::config::ConfigPaths;
use lazy_github_core::git::GitCli;
use lazy_github_core::github::Github;
use lazy_github_core::logging::LogLine;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::App;
use crate::backend::WideEmojiBackend;
use crate::event::{Event, Signal};
use crate::runtime::Runtime;

/// How many already-queued events are applied between two draws, at most, so a flood can't starve drawing.
const MAX_EVENTS_PER_FRAME: usize = 256;

/// Puts the terminal into the app's modes and takes them out again exactly once, on [`Self::restore`], on drop, or
/// from the panic hook through [`Self::restore_fn`], whichever comes first.
///
/// Raw mode isn't written as bytes, so it's toggled separately with `crossterm::terminal`.
pub struct TerminalGuard<W: Write> {
    out: W,
    kitty_pushed: bool,
    restored: Arc<AtomicBool>,
}

impl<W: Write> TerminalGuard<W> {
    /// Writes the enter sequence: alternate screen, hidden cursor, mouse capture, bracketed paste, focus-change
    /// reporting and, with `kitty`, the kitty keyboard flag `DISAMBIGUATE_ESCAPE_CODES`.
    ///
    /// The guard exists before the first write, so if a write fails partway, dropping it on the `?` restores
    /// whatever was entered.
    pub fn enter(out: W, kitty: bool) -> io::Result<Self> {
        let mut guard = Self {
            out,
            kitty_pushed: false,
            restored: Arc::new(AtomicBool::new(false)),
        };
        execute!(
            guard.out,
            EnterAlternateScreen,
            Hide,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        if kitty {
            execute!(
                guard.out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
            guard.kitty_pushed = true;
        }
        Ok(guard)
    }

    /// Writes the exit sequence, the enter sequence undone in reverse. Later calls, and the restore closure, write
    /// nothing.
    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        write_restore(&mut self.out, self.kitty_pushed)
    }

    /// A closure for the panic hook that restores the terminal through a clone of the writer, sharing "already
    /// restored" with this guard.
    pub fn restore_fn(&self) -> impl Fn() + Send + Sync + 'static
    where
        W: Clone + Send + 'static,
    {
        let out = Mutex::new(self.out.clone());
        let restored = Arc::clone(&self.restored);
        let kitty = self.kitty_pushed;
        move || {
            if !restored.swap(true, Ordering::SeqCst) {
                let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                // Nothing can be reported from a panic hook; the report printed next is what matters.
                let _ = write_restore(&mut *out, kitty);
            }
        }
    }
}

impl<W: Write> Drop for TerminalGuard<W> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn write_restore(out: &mut impl Write, kitty_pushed: bool) -> io::Result<()> {
    if kitty_pushed {
        execute!(out, PopKeyboardEnhancementFlags)?;
    }
    execute!(
        out,
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    )
}

/// Installs a panic hook that calls `restore` first, then logs the panic, then runs the previous hook (color-eyre's,
/// when `color_eyre::install()` ran first), which prints the report.
pub fn install_panic_hook(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        tracing::error!(panic = %info, "LazyGithub panicked");
        previous(info);
    }));
}

/// A handle to stdout that can be cloned into the panic hook. Every clone writes to the same, locked, stdout.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdoutWriter;

impl Write for StdoutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stdout().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

/// Turns raw mode off on drop.
struct RawMode;

impl RawMode {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Termination signals as a stream. Never yields off unix.
struct Signals {
    #[cfg(unix)]
    term: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hup: tokio::signal::unix::Signal,
}

impl Signals {
    fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                term: signal(SignalKind::terminate())?,
                hup: signal(SignalKind::hangup())?,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    async fn recv(&mut self) -> Signal {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.term.recv() => Signal::Term,
                _ = self.hup.recv() => Signal::Hup,
            }
        }
        #[cfg(not(unix))]
        {
            std::future::pending().await
        }
    }
}

/// Takes over the terminal and runs `app` until it quits, then gives the terminal back exactly as it was.
///
/// `log_rx` is the command-log stream from `LoggingHandle`. Requests run against `gh`, git requests against `git`
/// (in the user's working directory) and config saves go to `paths`. Must be called inside a multi-threaded tokio
/// runtime.
///
/// Returns an error when the terminal can't be set up or read, and when any thread panics. The panic hook restores
/// the terminal at once, and the loop then stops instead of drawing into the restored screen.
pub async fn run<G: Github>(
    mut app: App,
    gh: G,
    git: GitCli,
    log_rx: mpsc::Receiver<LogLine>,
    paths: ConfigPaths,
) -> color_eyre::Result<()> {
    if !io::stdout().is_terminal() {
        bail!("LazyGithub needs a terminal, but stdout is not one");
    }
    let signals = Signals::new()?;
    // Pushing the kitty flags is what makes `shift+enter` and `ctrl+m` distinguishable (#73).
    let kitty = terminal::supports_keyboard_enhancement().unwrap_or(false);

    // Declared first so it's dropped last, after the modes are written off.
    let raw_mode = RawMode::enable()?;
    let mut guard = TerminalGuard::enter(StdoutWriter, kitty)?;
    let restore = guard.restore_fn();
    let (panic_tx, panics) = mpsc::unbounded_channel();
    install_panic_hook(move || {
        restore();
        let _ = terminal::disable_raw_mode();
        // Tokio catches panics in spawned tasks, so without this the loop would carry on.
        let _ = panic_tx.send(());
    });
    let mut terminal = Terminal::new(WideEmojiBackend::new(CrosstermBackend::new(io::stdout())))?;
    terminal.clear()?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(gh, git, tx, paths);
    let mut inputs = Inputs {
        term: TerminalEvents::spawn(),
        log: log_rx,
        log_open: true,
        panics,
        signals: Some(signals),
    };
    runtime.execute_all(app.init());
    let result = event_loop(&mut app, &mut runtime, &mut terminal, &mut rx, &mut inputs).await;

    drop(inputs);
    drop(terminal);
    let restored = guard.restore();
    drop(raw_mode);
    // The terminal is back, so waiting for an in-flight config save doesn't hold the screen.
    runtime.shutdown().await;
    result?;
    restored?;
    Ok(())
}

/// Everything the loop waits on besides the runtime's events.
struct Inputs {
    term: TerminalEvents,
    log: mpsc::Receiver<LogLine>,
    log_open: bool,
    /// One message per panic on any thread, from the panic hook.
    panics: mpsc::UnboundedReceiver<()>,
    signals: Option<Signals>,
}

async fn event_loop<G: Github, B: Backend>(
    app: &mut App,
    runtime: &mut Runtime<G>,
    terminal: &mut Terminal<B>,
    rx: &mut mpsc::UnboundedReceiver<Event>,
    inputs: &mut Inputs,
) -> color_eyre::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    while !app.should_quit() {
        terminal.draw(|frame| app.render(frame))?;
        let deadline = app.next_deadline();
        let event = tokio::select! {
            biased;
            Some(()) = inputs.panics.recv() => bail!("a background task panicked"),
            event = inputs.term.rx.recv() => match event {
                Some(event) => Event::from(event?),
                None => bail!("the terminal event reader stopped"),
            },
            Some(event) = rx.recv() => event,
            line = inputs.log.recv(), if inputs.log_open => {
                let Some(line) = line else {
                    inputs.log_open = false;
                    continue;
                };
                Event::CommandLog(line)
            }
            () = sleep_until(deadline), if deadline.is_some() => Event::Deadline,
            signal = recv_signal(inputs.signals.as_mut()) => Event::Signal(signal),
        };
        runtime.execute_all(app.handle_event(event));

        // Apply whatever else is already waiting before drawing again.
        for _ in 0..MAX_EVENTS_PER_FRAME {
            if app.should_quit() {
                break;
            }
            let event = if let Ok(event) = rx.try_recv() {
                event
            } else if let Ok(event) = inputs.term.rx.try_recv() {
                Event::from(event?)
            } else {
                break;
            };
            runtime.execute_all(app.handle_event(event));
        }
    }
    Ok(())
}

async fn recv_signal(signals: Option<&mut Signals>) -> Signal {
    match signals {
        Some(signals) => signals.recv().await,
        None => std::future::pending().await,
    }
}

/// Terminal events, read by a task of their own.
///
/// `EventStream` keeps the waker from the first poll that found nothing, so polling it from a `select!` that drops
/// the future, or with `now_or_never`, can leave it waking a stale waker. One task polling it forever avoids that and
/// makes queued events available to `try_recv` for coalescing.
struct TerminalEvents {
    rx: mpsc::UnboundedReceiver<io::Result<crossterm::event::Event>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl TerminalEvents {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut stream = EventStream::new();
            while let Some(event) = stream.next().await {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
        Self { rx, task: Some(task) }
    }
}

impl Drop for TerminalEvents {
    fn drop(&mut self) {
        // Dropping the stream stops crossterm's reader thread.
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer whose clones share one buffer.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl SharedBuf {
        fn take(&self) -> String {
            String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
        }
    }

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const ENTER: &str =
        "\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h\x1b[?1004h";
    const KITTY_PUSH: &str = "\x1b[>1u";
    const KITTY_POP: &str = "\x1b[<1u";
    const RESTORE: &str =
        "\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h";

    #[test]
    fn enter_and_restore_with_kitty() {
        let buf = SharedBuf::default();
        let mut guard = TerminalGuard::enter(buf.clone(), true).unwrap();
        assert_eq!(buf.take(), format!("{ENTER}{KITTY_PUSH}"));
        guard.restore().unwrap();
        assert_eq!(buf.take(), format!("{KITTY_POP}{RESTORE}"));
    }

    #[test]
    fn enter_and_restore_without_kitty() {
        let buf = SharedBuf::default();
        let mut guard = TerminalGuard::enter(buf.clone(), false).unwrap();
        assert_eq!(buf.take(), ENTER);
        guard.restore().unwrap();
        assert_eq!(buf.take(), RESTORE);
    }

    #[test]
    fn restore_happens_once() {
        let buf = SharedBuf::default();
        let mut guard = TerminalGuard::enter(buf.clone(), true).unwrap();
        let restore = guard.restore_fn();
        buf.take();
        guard.restore().unwrap();
        assert_eq!(buf.take(), format!("{KITTY_POP}{RESTORE}"));
        guard.restore().unwrap();
        restore();
        drop(guard);
        assert_eq!(buf.take(), "");
    }

    #[test]
    fn drop_restores() {
        let buf = SharedBuf::default();
        drop(TerminalGuard::enter(buf.clone(), false).unwrap());
        assert_eq!(buf.take(), format!("{ENTER}{RESTORE}"));
    }

    #[test]
    fn the_restore_closure_shares_the_guard_flag() {
        let buf = SharedBuf::default();
        let guard = TerminalGuard::enter(buf.clone(), false).unwrap();
        buf.take();
        guard.restore_fn()();
        drop(guard);
        assert_eq!(buf.take(), RESTORE);
    }

    /// Fails exactly one write, the `n`th.
    struct FailNthWrite {
        buf: SharedBuf,
        n: usize,
    }

    impl Write for FailNthWrite {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.n = self.n.saturating_sub(1);
            if self.n == 0 {
                self.n = usize::MAX;
                return Err(io::Error::other("write failed"));
            }
            self.buf.write(data)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_partial_enter_is_restored() {
        let buf = SharedBuf::default();
        // The alternate screen goes out, then hiding the cursor fails.
        let out = FailNthWrite { buf: buf.clone(), n: 2 };
        assert!(TerminalGuard::enter(out, true).is_err());
        assert_eq!(buf.take(), format!("\x1b[?1049h{RESTORE}"));
    }

    /// A `Github` whose `check_auth` panics, as a bug in parsing `gh` output would.
    #[derive(Clone)]
    struct PanickingGithub;

    impl Github for PanickingGithub {
        async fn check_auth(&self) -> Result<(), lazy_github_core::gh::GhError> {
            panic!("check_auth exploded");
        }

        async fn current_user(&self) -> Result<lazy_github_core::models::User, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn list_repositories(
            &self,
            _query: lazy_github_core::github::RepoListQuery,
        ) -> Result<Vec<lazy_github_core::models::Repository>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn unread_notification_count(&self) -> Result<usize, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn list_issues(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _query: lazy_github_core::github::IssueListQuery,
        ) -> Result<Vec<lazy_github_core::models::IssueListItem>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn repo_readme(&self, _full_name: &str) -> Result<Option<String>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn repository(
            &self,
            _full_name: &str,
        ) -> Result<Option<lazy_github_core::models::Repository>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn list_workflows(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _page: lazy_github_core::github::PageQuery,
        ) -> Result<Vec<lazy_github_core::models::Workflow>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn list_workflow_runs(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _page: lazy_github_core::github::PageQuery,
        ) -> Result<Vec<lazy_github_core::models::WorkflowRun>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn pull_request_diff(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _number: u64,
        ) -> Result<String, lazy_github_core::github::DiffFetchError> {
            std::future::pending().await
        }

        async fn user(
            &self,
            _login: &str,
        ) -> Result<lazy_github_core::models::FullUser, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn user_repos(
            &self,
            _login: &str,
        ) -> Result<Vec<lazy_github_core::models::Repository>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn pull_request(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _number: u64,
        ) -> Result<lazy_github_core::models::FullPullRequest, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn workflow_run_jobs(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _run_id: u64,
        ) -> Result<Vec<lazy_github_core::models::WorkflowJob>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn commit_status(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _sha: &str,
        ) -> Result<lazy_github_core::models::CombinedStatus, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn check_runs(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _sha: &str,
        ) -> Result<Vec<lazy_github_core::models::CheckRun>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn issue_reactions(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _number: u64,
        ) -> Result<lazy_github_core::models::Reactions, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn issue(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _number: u64,
        ) -> Result<lazy_github_core::models::IssueListItem, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn issue_comments(
            &self,
            _issue: &lazy_github_core::models::Issue,
        ) -> Result<Vec<lazy_github_core::models::IssueComment>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn comment_reactions(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _comment: lazy_github_core::models::CommentRef,
        ) -> Result<lazy_github_core::models::Reactions, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn job_logs(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _job_id: u64,
        ) -> Result<Option<String>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn pull_request_reviews(
            &self,
            _pr: &lazy_github_core::models::FullPullRequest,
        ) -> Result<Vec<lazy_github_core::models::Review>, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }

        async fn create_issue(
            &self,
            _repo: &lazy_github_core::models::Repository,
            _issue: &lazy_github_core::github::NewIssue,
        ) -> Result<lazy_github_core::models::Issue, lazy_github_core::gh::GhError> {
            std::future::pending().await
        }
    }

    /// Runs the loop on a `TestBackend` with `effect` already started, and returns how it ended.
    async fn run_loop_until_it_stops(
        effect: Option<crate::effect::Effect>,
        keep_terminal_reader: bool,
        panics: mpsc::UnboundedReceiver<()>,
    ) -> color_eyre::Result<()> {
        let mut app = App::new(
            lazy_github_core::config::Config::default(),
            crate::theme::Theme::default(),
            crate::options::AppOptions::default(),
            crate::clock::FakeClock::new(),
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let paths = ConfigPaths::in_dir(std::env::temp_dir().join("lazy-github-tui-loop-test-unused"));
        let mut runtime = Runtime::new(PanickingGithub, GitCli::new(None), tx, paths);
        let (term_tx, term_rx) = mpsc::unbounded_channel();
        let (_log_tx, log_rx) = mpsc::channel(1);
        let mut inputs = Inputs {
            term: TerminalEvents {
                rx: term_rx,
                task: None,
            },
            log: log_rx,
            log_open: true,
            panics,
            signals: None,
        };
        let _term_tx = keep_terminal_reader.then_some(term_tx);
        runtime.execute_all(effect);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(20, 5)).unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            event_loop(&mut app, &mut runtime, &mut terminal, &mut rx, &mut inputs),
        )
        .await
        .expect("the loop should stop");
        runtime.shutdown().await;
        result
    }

    #[tokio::test]
    async fn a_panic_in_a_background_task_stops_the_loop_with_an_error() {
        let (panic_tx, panics) = mpsc::unbounded_channel();
        // The hook `run` installs, minus the terminal restore.
        install_panic_hook(move || {
            let _ = panic_tx.send(());
        });
        let effect = crate::effect::Effect::Github {
            id: crate::effect::RequestId::new(1),
            request: crate::effect::GithubRequest::CheckAuth,
        };
        let result = run_loop_until_it_stops(Some(effect), true, panics).await;
        // Back to the default hook for the rest of the test binary.
        drop(std::panic::take_hook());
        assert_eq!(result.unwrap_err().to_string(), "a background task panicked");
    }

    #[tokio::test]
    async fn a_stopped_terminal_reader_is_an_error() {
        let (_panic_tx, panics) = mpsc::unbounded_channel();
        let result = run_loop_until_it_stops(None, false, panics).await;
        assert_eq!(result.unwrap_err().to_string(), "the terminal event reader stopped");
    }
}
