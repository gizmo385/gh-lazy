//! Terminal setup and teardown, the panic hook, and the event loop.

use std::io::{self, IsTerminal as _, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

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
use lazy_github_core::github::Github;
use lazy_github_core::logging::LogLine;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::App;
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
    pub fn enter(mut out: W, kitty: bool) -> io::Result<Self> {
        execute!(
            out,
            EnterAlternateScreen,
            Hide,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        if kitty {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        Ok(Self {
            out,
            kitty_pushed: kitty,
            restored: Arc::new(AtomicBool::new(false)),
        })
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
/// `log_rx` is the command-log stream from `LoggingHandle`. Requests run against `gh` and config saves go to
/// `paths`. Must be called inside a multi-threaded tokio runtime.
pub async fn run<G: Github>(
    mut app: App,
    gh: G,
    mut log_rx: mpsc::Receiver<LogLine>,
    paths: ConfigPaths,
) -> color_eyre::Result<()> {
    if !io::stdout().is_terminal() {
        color_eyre::eyre::bail!("LazyGithub needs a terminal, but stdout is not one");
    }
    let mut signals = Signals::new()?;
    // Pushing the kitty flags is what makes `shift+enter` and `ctrl+m` distinguishable (#73).
    let kitty = terminal::supports_keyboard_enhancement().unwrap_or(false);

    // Declared first so it's dropped last, after the modes are written off.
    let raw_mode = RawMode::enable()?;
    let mut guard = TerminalGuard::enter(StdoutWriter, kitty)?;
    let restore = guard.restore_fn();
    install_panic_hook(move || {
        restore();
        let _ = terminal::disable_raw_mode();
    });
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(gh, tx, paths);
    let result = event_loop(
        &mut app,
        &mut runtime,
        &mut terminal,
        &mut rx,
        &mut log_rx,
        &mut signals,
    )
    .await;

    drop(terminal);
    let restored = guard.restore();
    drop(raw_mode);
    // The terminal is back, so waiting for an in-flight config save doesn't hold the screen.
    runtime.shutdown().await;
    result?;
    restored?;
    Ok(())
}

async fn event_loop<G: Github>(
    app: &mut App,
    runtime: &mut Runtime<G>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    rx: &mut mpsc::UnboundedReceiver<Event>,
    log_rx: &mut mpsc::Receiver<LogLine>,
    signals: &mut Signals,
) -> color_eyre::Result<()> {
    let mut term = terminal_events();
    let mut log_open = true;
    runtime.execute_all(app.init());
    while !app.should_quit() {
        terminal.draw(|frame| app.render(frame))?;
        let deadline = app.next_deadline();
        let event = tokio::select! {
            biased;
            event = term.rx.recv() => match event {
                Some(event) => Event::from(event?),
                None => break,
            },
            Some(event) = rx.recv() => event,
            line = log_rx.recv(), if log_open => {
                let Some(line) = line else {
                    log_open = false;
                    continue;
                };
                Event::CommandLog(line)
            }
            () = sleep_until(deadline), if deadline.is_some() => Event::Deadline,
            signal = signals.recv() => Event::Signal(signal),
        };
        runtime.execute_all(app.handle_event(event));

        // Apply whatever else is already waiting before drawing again.
        for _ in 0..MAX_EVENTS_PER_FRAME {
            if app.should_quit() {
                break;
            }
            let event = if let Ok(event) = rx.try_recv() {
                event
            } else if let Ok(event) = term.rx.try_recv() {
                Event::from(event?)
            } else {
                break;
            };
            runtime.execute_all(app.handle_event(event));
        }
    }
    Ok(())
}

/// Terminal events, read by a task of their own.
///
/// `EventStream` keeps the waker from the first poll that found nothing, so polling it from a `select!` that drops
/// the future, or with `now_or_never`, can leave it waking a stale waker. One task polling it forever avoids that and
/// makes queued events available to `try_recv` for coalescing.
struct TerminalEvents {
    rx: mpsc::UnboundedReceiver<io::Result<crossterm::event::Event>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TerminalEvents {
    fn drop(&mut self) {
        // Dropping the stream stops crossterm's reader thread.
        self.task.abort();
    }
}

fn terminal_events() -> TerminalEvents {
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(event) = stream.next().await {
            if tx.send(event).is_err() {
                break;
            }
        }
    });
    TerminalEvents { rx, task }
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
}
