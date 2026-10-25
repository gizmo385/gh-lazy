//! The Python log line format, `<LEVEL> <YYYY-MM-DD HH:MM:SS> - <message>`, shared by the log file and the command
//! log.

use std::fmt::{self, Write as _};
use std::sync::Arc;

use jiff::civil::DateTime;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Where log timestamps come from. Tests and the parity harness pass a fixed clock.
pub type LogClock = Arc<dyn Fn() -> DateTime + Send + Sync>;

/// The local wall clock, which is what Python's `logging` uses.
pub fn system_clock() -> LogClock {
    Arc::new(|| jiff::Zoned::now().datetime())
}

/// One rendered log event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// The event's level.
    pub level: Level,
    /// When the event happened, in local time.
    pub timestamp: DateTime,
    /// The message followed by the event's fields as ` key=value`.
    pub message: String,
}

impl LogLine {
    /// Renders `event` at the clock's current time. With `error_sources`, an error field is followed by its source
    /// chain, as the file records it; without, only the error's own message is kept, as the command log shows it.
    pub fn from_event(event: &Event<'_>, clock: &LogClock, error_sources: bool) -> Self {
        let mut visitor = FieldVisitor {
            message: String::new(),
            fields: String::new(),
            error_sources,
        };
        event.record(&mut visitor);
        let mut message = visitor.message;
        if message.is_empty() {
            message.push_str(visitor.fields.trim_start());
        } else {
            message.push_str(&visitor.fields);
        }
        Self {
            level: *event.metadata().level(),
            timestamp: clock(),
            message,
        }
    }
}

impl fmt::Display for LogLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} - {}",
            level_name(self.level),
            self.timestamp.strftime("%Y-%m-%d %H:%M:%S"),
            self.message
        )
    }
}

/// Python's level names. tracing's `WARN` is spelled `WARNING`, as Python spells it.
fn level_name(level: Level) -> &'static str {
    match level {
        Level::TRACE => "TRACE",
        Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARNING",
        Level::ERROR => "ERROR",
    }
}

/// Collects the message and renders the other fields the way `tracing-subscriber`'s default formatter does: strings
/// quoted, other values with `Debug`, errors with `Display` and an optional `<name>.sources=[…]` list.
struct FieldVisitor {
    message: String,
    fields: String,
    error_sources: bool,
}

impl FieldVisitor {
    fn field_name(field: &Field) -> &str {
        let name = field.name();
        name.strip_prefix("r#").unwrap_or(name)
    }
}

// `write!` into a `String` can't fail, so its results are ignored throughout.
impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        let name = Self::field_name(field);
        let _ = write!(self.fields, " {name}={value}");
        if self.error_sources && value.source().is_some() {
            let _ = write!(self.fields, " {name}.sources=[");
            let mut current = Some(value);
            let mut first = true;
            while let Some(err) = current {
                let sep = if first { "" } else { ", " };
                let _ = write!(self.fields, "{sep}{err}");
                first = false;
                current = err.source();
            }
            self.fields.push(']');
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", Self::field_name(field));
        }
    }
}

/// The log file's event formatter: one [`LogLine`] per line, with error source chains and no ANSI codes.
pub(crate) struct FileFormat {
    pub(crate) clock: LogClock,
}

impl<S, N> FormatEvent<S, N> for FileFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        writeln!(writer, "{}", LogLine::from_event(event, &self.clock, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::test_support::{CaptureWriter, fixed_clock};
    use tracing_subscriber::layer::SubscriberExt as _;

    #[derive(Debug, thiserror::Error)]
    #[error("outer failure")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner cause")]
    struct Inner;

    fn render(f: impl FnOnce()) -> String {
        let out = CaptureWriter::default();
        let layer = tracing_subscriber::fmt::layer()
            .event_format(FileFormat { clock: fixed_clock() })
            .with_writer(out.clone());
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), f);
        out.contents()
    }

    #[test]
    fn info_renders_in_the_python_format() {
        let out = render(|| tracing::info!("Selected repo foo/bar"));
        assert_eq!(out, "INFO 2026-10-25 01:02:03 - Selected repo foo/bar\n");
    }

    #[test]
    fn warn_is_spelled_warning() {
        let out = render(|| tracing::warn!("Careful"));
        assert_eq!(out, "WARNING 2026-10-25 01:02:03 - Careful\n");
    }

    #[test]
    fn every_level_has_its_python_name() {
        let out = render(|| {
            tracing::trace!("t");
            tracing::debug!("d");
            tracing::error!("e");
        });
        assert_eq!(
            out,
            "TRACE 2026-10-25 01:02:03 - t\nDEBUG 2026-10-25 01:02:03 - d\nERROR 2026-10-25 01:02:03 - e\n"
        );
    }

    #[test]
    fn fields_follow_the_message() {
        let out = render(|| tracing::debug!(count = 3, name = "x", r#type = ?Some(1), "Running {}", "gh"));
        assert_eq!(
            out,
            "DEBUG 2026-10-25 01:02:03 - Running gh count=3 name=\"x\" type=Some(1)\n"
        );
    }

    #[test]
    fn fields_without_a_message() {
        let out = render(|| tracing::info!(count = 3));
        assert_eq!(out, "INFO 2026-10-25 01:02:03 - count=3\n");
    }

    #[test]
    fn display_errors_render_their_message() {
        let err = Outer(Inner);
        let out = render(|| tracing::error!(error = %err, "Merge failed"));
        assert_eq!(out, "ERROR 2026-10-25 01:02:03 - Merge failed error=outer failure\n");
    }

    #[test]
    fn error_values_render_their_source_chain() {
        let err = Outer(Inner);
        let out = render(|| tracing::error!(error = &err as &(dyn std::error::Error + 'static), "Merge failed"));
        assert_eq!(
            out,
            "ERROR 2026-10-25 01:02:03 - Merge failed error=outer failure error.sources=[outer failure, inner cause]\n"
        );
    }
}
