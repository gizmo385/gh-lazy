//! The tracing layer behind the command-log pane: INFO and above from the app's own crates, as [`LogLine`]s.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::{Context, Layer};

use super::format::{LogClock, LogLine};

/// How many lines the command-log channel holds before new lines are dropped.
pub const COMMAND_LOG_CAPACITY: usize = 1024;

static DROPPED_LINES: AtomicU64 = AtomicU64::new(0);

/// How many command-log lines have been dropped because the channel was full, across the whole process.
pub fn dropped_command_log_lines() -> u64 {
    DROPPED_LINES.load(Ordering::Relaxed)
}

/// Turns each INFO-or-above event from a `lazy_github*` target into a [`LogLine`] and hands it to `sink`.
///
/// The line holds the message and fields but not error source chains; those go only to the log file.
pub struct CommandLogLayer<F: Fn(LogLine) + Send + Sync + 'static> {
    sink: F,
    clock: LogClock,
    filter: Targets,
}

impl<F: Fn(LogLine) + Send + Sync + 'static> CommandLogLayer<F> {
    /// A layer that calls `sink` with each command-log line, timestamped by `clock`.
    pub fn new(sink: F, clock: LogClock) -> Self {
        Self {
            sink,
            clock,
            // Target matching is by prefix, so this covers `lazy_github_core` and `lazy_github_tui` too.
            filter: Targets::new().with_target("lazy_github", Level::INFO),
        }
    }
}

impl<S: Subscriber, F: Fn(LogLine) + Send + Sync + 'static> Layer<S> for CommandLogLayer<F> {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        // Filtered here rather than with a per-layer filter so the layer behaves the same on its own in tests.
        if self.filter.would_enable(metadata.target(), metadata.level()) {
            (self.sink)(LogLine::from_event(event, &self.clock, false));
        }
    }
}

/// A [`CommandLogLayer`] that sends its lines into a bounded channel, and the receiving end. When the channel is
/// full the line is dropped and counted (see [`dropped_command_log_lines`]), so logging never blocks.
pub fn command_log_layer(
    clock: LogClock,
) -> (
    CommandLogLayer<impl Fn(LogLine) + Send + Sync + 'static>,
    mpsc::Receiver<LogLine>,
) {
    let (tx, rx) = mpsc::channel(COMMAND_LOG_CAPACITY);
    let sink = move |line| {
        // A closed channel means nobody shows the command log any more; dropping the line is all there is to do.
        if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(line) {
            DROPPED_LINES.fetch_add(1, Ordering::Relaxed);
        }
    };
    (CommandLogLayer::new(sink, clock), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::test_support::fixed_clock;
    use jiff::civil::date;
    use tracing_subscriber::layer::SubscriberExt as _;

    fn drain(rx: &mut mpsc::Receiver<LogLine>) -> Vec<LogLine> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn app_events_at_info_and_above_arrive_in_order() {
        let (layer, mut rx) = command_log_layer(fixed_clock());
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            tracing::info!(target: "lazy_github_tui", "Selected repo foo/bar");
            tracing::debug!(target: "lazy_github_tui", "not shown");
            tracing::info!(target: "hyper", "foreign");
            tracing::error!(target: "lazy_github_tui::pane", "Error fetching repositories from Github API");
            tracing::warn!(target: "lazy_github_core", "careful");
        });
        let lines = drain(&mut rx);
        let timestamp = date(2026, 10, 25).at(1, 2, 3, 0);
        assert_eq!(
            lines,
            [
                LogLine {
                    level: Level::INFO,
                    timestamp,
                    message: "Selected repo foo/bar".to_owned()
                },
                LogLine {
                    level: Level::ERROR,
                    timestamp,
                    message: "Error fetching repositories from Github API".to_owned()
                },
                LogLine {
                    level: Level::WARN,
                    timestamp,
                    message: "careful".to_owned()
                },
            ]
        );
        assert_eq!(lines[0].to_string(), "INFO 2026-10-25 01:02:03 - Selected repo foo/bar");
    }

    #[test]
    fn error_sources_are_left_out() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer")]
        struct Outer(#[source] std::io::Error);

        let (layer, mut rx) = command_log_layer(fixed_clock());
        let err = Outer(std::io::Error::other("inner"));
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            tracing::error!(
                target: "lazy_github_core",
                error = &err as &(dyn std::error::Error + 'static),
                "Merge failed"
            );
        });
        assert_eq!(drain(&mut rx)[0].message, "Merge failed error=outer");
    }

    #[test]
    fn a_full_channel_drops_lines_without_blocking() {
        let (layer, mut rx) = command_log_layer(fixed_clock());
        let before = dropped_command_log_lines();
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            for n in 0..COMMAND_LOG_CAPACITY + 10 {
                tracing::info!(target: "lazy_github_core", "line {n}");
            }
        });
        let lines = drain(&mut rx);
        assert_eq!(lines.len(), COMMAND_LOG_CAPACITY);
        assert_eq!(
            lines.last().unwrap().message,
            format!("line {}", COMMAND_LOG_CAPACITY - 1)
        );
        // Other tests may drop lines concurrently, so this only checks a lower bound.
        assert!(dropped_command_log_lines() - before >= 10);
    }
}
