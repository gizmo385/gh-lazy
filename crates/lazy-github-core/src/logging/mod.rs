//! Logging: a size-rotated log file in the Python app's format, and the stream behind the command-log pane.
//!
//! Nothing here writes to the terminal, apart from one line on stderr when the log file can't be opened.
//!
//! Conventions for the rest of the app:
//!
//! - `info!` is for something the user did or that changed state, the places Python calls `lg.info`. It shows up in
//!   the command log.
//! - `warn!` and `error!` are for failures the user should see. They show up in the command log.
//! - `debug!` and `trace!` are diagnostics, for the file only.
//! - Keep messages identical to the Python text wherever a parity scenario can see the command log.

mod command_log;
mod format;
mod rotating;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;

pub use command_log::{COMMAND_LOG_CAPACITY, CommandLogLayer, command_log_layer};
pub use format::{LogClock, LogLine, system_clock};
pub use rotating::RotatingFileWriter;

use crate::config::CoreConfig;

/// The environment variable that overrides which events reach the log file, in `Targets` syntax such as `trace` or
/// `lazy_github_core=trace,warn`. A debugging aid; users never see it.
pub const LOG_FILTER_ENV: &str = "LAZY_GITHUB_LOG";

/// Where and how the log file is written.
pub struct LoggingOptions {
    /// The log file.
    pub path: PathBuf,
    /// The size at which the file rotates, in bytes. `0` never rotates.
    pub max_bytes: u64,
    /// How many rotated files are kept. `0` keeps one ever-growing file.
    pub backup_count: u32,
    /// Where timestamps come from.
    pub clock: LogClock,
}

impl From<&CoreConfig> for LoggingOptions {
    fn from(core: &CoreConfig) -> Self {
        Self {
            path: core.logfile_path.clone(),
            max_bytes: core.logfile_max_bytes(),
            backup_count: core.logfile_count(),
            clock: system_clock(),
        }
    }
}

/// Keeps logging running. Hold it in `main`: dropping it flushes the log file and stops writing to it.
pub struct LoggingHandle {
    _file: Option<WorkerGuard>,
    /// Command-log lines, including those logged before anything reads them (up to [`COMMAND_LOG_CAPACITY`]).
    pub command_log: mpsc::Receiver<LogLine>,
    dropped_command_log_lines: Arc<AtomicU64>,
}

impl LoggingHandle {
    /// How many command-log lines were dropped because [`Self::command_log`] was full.
    pub fn dropped_command_log_lines(&self) -> u64 {
        self.dropped_command_log_lines.load(Ordering::Relaxed)
    }
}

/// Installs the global subscriber: the log file and the command-log stream.
///
/// This never fails. If the log file can't be opened, one line goes to stderr and the app carries on with only the
/// command log, as the Python app does. Call it before the TUI starts. Calling it again leaves the first subscriber
/// in place.
pub fn init(opts: LoggingOptions) -> LoggingHandle {
    let (command_log, rx, dropped) = command_log_layer(opts.clock.clone());
    let (filter, filter_error) = file_filter(std::env::var(LOG_FILTER_ENV).ok().as_deref());

    let (file_layer, guard) = match RotatingFileWriter::open(&opts.path, opts.max_bytes, opts.backup_count) {
        Ok(writer) => {
            let (writer, guard) = tracing_appender::non_blocking(writer);
            (Some(file_layer(writer, opts.clock, filter)), Some(guard))
        }
        Err(e) => {
            eprintln!(
                "LazyGithub: could not open log file {}: {e}; continuing without a log file",
                opts.path.display()
            );
            (None, None)
        }
    };

    // Another subscriber is only installed already in tests, where keeping it is the right outcome.
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(command_log)
        .try_init();
    if let Some(e) = filter_error {
        // `debug!` keeps this out of the command log: users never see the variable.
        tracing::debug!("Ignoring {LOG_FILTER_ENV}: {e}");
    }

    LoggingHandle {
        _file: guard,
        command_log: rx,
        dropped_command_log_lines: dropped,
    }
}

/// The file layer: [`LogLine`]s written to `writer`, filtered by `filter`.
fn file_layer<S, W>(writer: W, clock: LogClock, filter: Targets) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .event_format(format::FileFormat { clock })
        .with_writer(writer)
        .with_filter(filter)
}

/// The file's filter: DEBUG and above from the app's crates and WARN and above from dependencies, the way Python
/// mutes noisy libraries, unless `env` (the value of [`LOG_FILTER_ENV`]) overrides it. An unparseable override is
/// ignored and its error returned.
fn file_filter(env: Option<&str>) -> (Targets, Option<tracing_subscriber::filter::ParseError>) {
    let default = || {
        Targets::new()
            .with_target("lazy_github", Level::DEBUG)
            .with_default(Level::WARN)
    };
    match env.map(str::trim).filter(|s| !s.is_empty()).map(str::parse::<Targets>) {
        None => (default(), None),
        Some(Ok(targets)) => (targets, None),
        Some(Err(e)) => (default(), Some(e)),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use jiff::civil::date;
    use tracing_subscriber::fmt::MakeWriter;

    use super::LogClock;

    /// Always 2026-10-25 01:02:03.
    pub(crate) fn fixed_clock() -> LogClock {
        Arc::new(|| date(2026, 10, 25).at(1, 2, 3, 0))
    }

    /// A temporary directory removed on drop. Tests use their own so they can run in parallel.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lazy-github-logging-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An in-memory writer for formatter tests.
    #[derive(Clone, Default)]
    pub(crate) struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl CaptureWriter {
        pub(crate) fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = Self;

        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::test_support::{TempDir, fixed_clock};
    use super::*;

    #[test]
    fn options_come_from_the_core_config() {
        let core = CoreConfig {
            logfile_path: PathBuf::from("/x/lazy_github.log"),
            logfile_max_bytes: 123,
            logfile_count: 7,
            ..CoreConfig::default()
        };
        let opts = LoggingOptions::from(&core);
        assert_eq!(opts.path, core.logfile_path);
        assert_eq!(opts.max_bytes, 123);
        assert_eq!(opts.backup_count, 7);
    }

    /// Python's `RotatingFileHandler` never rolls over at a size of 0 or less, and keeps no backups for a count of 0 or
    /// less.
    #[test]
    fn negative_log_sizes_never_rotate_and_keep_no_backups() {
        let core = CoreConfig {
            logfile_max_bytes: -1,
            logfile_count: -3,
            ..CoreConfig::default()
        };
        let opts = LoggingOptions::from(&core);
        assert_eq!(opts.max_bytes, 0);
        assert_eq!(opts.backup_count, 0);
    }

    /// Runs `f` with the real file layer writing through `tracing_appender::non_blocking`, and returns the file.
    fn log_to_file(filter: Targets, f: impl FnOnce()) -> String {
        let tmp = TempDir::new();
        let path = tmp.path().join("lazy_github.log");
        let writer = RotatingFileWriter::open(&path, 0, 0).unwrap();
        let (writer, guard) = tracing_appender::non_blocking(writer);
        crate::test_log::keep_callsites_interested();
        let subscriber = tracing_subscriber::registry().with(file_layer(writer, fixed_clock(), filter));
        tracing::subscriber::with_default(subscriber, f);
        drop(guard); // Flushes the worker.
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn default_filter_keeps_app_debug_and_foreign_warnings() {
        let (filter, err) = file_filter(None);
        assert!(err.is_none());
        let out = log_to_file(filter, || {
            tracing::debug!(target: "lazy_github_core::gh", "Running Github CLI command: gh api /user");
            tracing::trace!(target: "lazy_github_core", "too detailed");
            tracing::info!(target: "hyper::client", "foreign info");
            tracing::warn!(target: "hyper::client", "foreign warning");
        });
        assert_eq!(
            out,
            "DEBUG 2026-10-25 01:02:03 - Running Github CLI command: gh api /user\n\
             WARNING 2026-10-25 01:02:03 - foreign warning\n"
        );
    }

    #[test]
    fn env_override_replaces_the_filter() {
        let (filter, err) = file_filter(Some("lazy_github_core=trace,error"));
        assert!(err.is_none());
        let out = log_to_file(filter, || {
            tracing::trace!(target: "lazy_github_core", "detail");
            tracing::warn!(target: "hyper", "foreign warning");
        });
        assert_eq!(out, "TRACE 2026-10-25 01:02:03 - detail\n");
    }

    #[test]
    fn an_invalid_override_falls_back_to_the_default() {
        let (filter, err) = file_filter(Some("lazy_github=bogus"));
        assert!(err.is_some());
        assert!(filter.would_enable("lazy_github_core", &Level::DEBUG));
        assert!(!filter.would_enable("hyper", &Level::INFO));
    }

    #[test]
    fn file_layer_appends_across_runs() {
        let tmp = TempDir::new();
        let path = tmp.path().join("lazy_github.log");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"INFO 2026-10-24 00:00:00 - from Python\n")
            .unwrap();
        let writer = RotatingFileWriter::open(&path, 5_000_000, 5).unwrap();
        let (writer, guard) = tracing_appender::non_blocking(writer);
        crate::test_log::keep_callsites_interested();
        let subscriber = tracing_subscriber::registry().with(file_layer(writer, fixed_clock(), file_filter(None).0));
        tracing::subscriber::with_default(subscriber, || tracing::info!(target: "lazy_github", "from Rust"));
        drop(guard);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "INFO 2026-10-24 00:00:00 - from Python\nINFO 2026-10-25 01:02:03 - from Rust\n"
        );
    }
}
