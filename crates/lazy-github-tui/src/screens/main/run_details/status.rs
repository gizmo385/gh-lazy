//! How a run's and a job's state read on screen, and how long a job took: shared by the run's Overview and Jobs tabs.
//! Python's `_format_run_status`, `_format_job_status` and `_calculate_duration`.

use std::borrow::Cow;

use jiff::Timestamp;
use lazy_github_core::models::{JobConclusion, JobStatus, WorkflowRun};
use ratatui::style::{Modifier, Style};

use crate::theme::{Accent, Theme};

/// Rich's `dim`, as Textual draws it: the text color at this alpha over the background.
const DIM_ALPHA: f64 = 0.66;

/// The color a status is drawn in: one of the fixed accent colors, or Rich's `dim`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::screens::main) enum Tint {
    /// `[greenyellow]`, `[red]`, ...
    Accent(Accent),
    /// `[dim]`.
    Dim,
}

impl Tint {
    /// The style for text in this tint on the details pane's background.
    pub(in crate::screens::main) fn style(self, theme: &Theme) -> Style {
        match self {
            Tint::Accent(accent) => Style::new().fg(theme.accent(accent)),
            Tint::Dim => {
                let palette = theme.palette;
                if theme.ansi {
                    Style::new()
                        .fg(theme.color(palette.foreground, theme.background()))
                        .add_modifier(Modifier::DIM)
                } else {
                    Style::new().fg(theme.color(palette.foreground.multiply_alpha(DIM_ALPHA), theme.background()))
                }
            }
        }
    }
}

/// The run's state as the Overview's header starts: `✓ Success`, `⋯ In Progress`, or the raw `status` dimmed when
/// it's none of the known ones (a run that timed out reads `completed`).
pub(in crate::screens::main) fn run_status(run: &WorkflowRun) -> (Cow<'_, str>, Tint) {
    let (text, tint) = match (run.status.as_str(), run.conclusion.as_deref()) {
        ("completed", Some("success")) => ("✓ Success", Tint::Accent(Accent::GreenYellow)),
        ("completed", Some("failure")) => ("✗ Failure", Tint::Accent(Accent::Red)),
        ("completed", Some("cancelled")) => ("○ Cancelled", Tint::Accent(Accent::Yellow)),
        ("completed", Some("skipped")) => ("− Skipped", Tint::Dim),
        ("in_progress", _) => ("⋯ In Progress", Tint::Accent(Accent::Cyan)),
        ("queued", _) => ("⋯ Queued", Tint::Accent(Accent::Yellow)),
        (status, _) => return (Cow::Borrowed(status), Tint::Dim),
    };
    (Cow::Borrowed(text), tint)
}

/// The glyph a job (or a step) with `status` and `conclusion` is shown with: `?` dimmed for anything not listed (a
/// neutral, timed-out or action-required conclusion, `waiting`, `pending`).
pub(in crate::screens::main) fn job_icon(
    status: &JobStatus,
    conclusion: Option<&JobConclusion>,
) -> (&'static str, Tint) {
    match (status, conclusion) {
        (JobStatus::Completed, Some(JobConclusion::Success)) => ("✓", Tint::Accent(Accent::GreenYellow)),
        (JobStatus::Completed, Some(JobConclusion::Failure)) => ("✗", Tint::Accent(Accent::Red)),
        (JobStatus::Completed, Some(JobConclusion::Cancelled)) => ("○", Tint::Accent(Accent::Yellow)),
        (JobStatus::Completed, Some(JobConclusion::Skipped)) => ("−", Tint::Dim),
        (JobStatus::InProgress, _) => ("⋯", Tint::Accent(Accent::Cyan)),
        (JobStatus::Queued, _) => ("⋯", Tint::Accent(Accent::Yellow)),
        _ => ("?", Tint::Dim),
    }
}

/// How long a job ran: `Not started`, `In progress`, then the whole seconds as `59s`, `1m 5s` or `2h 3m`.
pub(in crate::screens::main) fn duration(started: Option<Timestamp>, completed: Option<Timestamp>) -> String {
    let Some(started) = started else {
        return "Not started".to_owned();
    };
    let Some(completed) = completed else {
        return "In progress".to_owned();
    };
    // Whole seconds, truncated as Python's `int(total_seconds())`.
    let seconds = completed.duration_since(started).as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, seconds % 3600 / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::run_details::tests::fixture_run;

    fn status_of(status: &str, conclusion: Option<&str>) -> (String, Tint) {
        let mut run = fixture_run(42);
        run.status = status.into();
        run.conclusion = conclusion.map(Into::into);
        let (text, tint) = run_status(&run);
        (text.into_owned(), tint)
    }

    #[test]
    fn run_statuses() {
        let accent = Tint::Accent;
        let rows = [
            ("completed", Some("success"), "✓ Success", accent(Accent::GreenYellow)),
            ("completed", Some("failure"), "✗ Failure", accent(Accent::Red)),
            ("completed", Some("cancelled"), "○ Cancelled", accent(Accent::Yellow)),
            ("completed", Some("skipped"), "− Skipped", Tint::Dim),
            ("in_progress", None, "⋯ In Progress", accent(Accent::Cyan)),
            ("in_progress", Some("success"), "⋯ In Progress", accent(Accent::Cyan)),
            ("queued", None, "⋯ Queued", accent(Accent::Yellow)),
            ("completed", Some("timed_out"), "completed", Tint::Dim),
            ("completed", None, "completed", Tint::Dim),
            ("waiting", None, "waiting", Tint::Dim),
        ];
        for (status, conclusion, text, tint) in rows {
            assert_eq!(
                status_of(status, conclusion),
                (text.to_owned(), tint),
                "{status} {conclusion:?}"
            );
        }
    }

    #[test]
    fn job_icons() {
        use JobConclusion as C;
        use JobStatus as S;
        let accent = Tint::Accent;
        let completed = |conclusion: C| job_icon(&S::Completed, Some(&conclusion));
        assert_eq!(completed(C::Success), ("✓", accent(Accent::GreenYellow)));
        assert_eq!(completed(C::Failure), ("✗", accent(Accent::Red)));
        assert_eq!(completed(C::Cancelled), ("○", accent(Accent::Yellow)));
        assert_eq!(completed(C::Skipped), ("−", Tint::Dim));
        for other in [C::Neutral, C::TimedOut, C::ActionRequired, C::Other("new".into())] {
            assert_eq!(completed(other), ("?", Tint::Dim));
        }
        assert_eq!(job_icon(&S::Completed, None), ("?", Tint::Dim));
        assert_eq!(job_icon(&S::InProgress, None), ("⋯", accent(Accent::Cyan)));
        // A queued job shows `⋯` in yellow, never `?` (Py bug B63 is the Jobs tab's).
        assert_eq!(job_icon(&S::Queued, None), ("⋯", accent(Accent::Yellow)));
        for other in [S::Waiting, S::Pending, S::Requested, S::Other("new".into())] {
            assert_eq!(job_icon(&other, None), ("?", Tint::Dim), "{other:?}");
        }
    }

    #[test]
    fn durations() {
        let at = |seconds: i64| Some(Timestamp::from_second(1_772_600_000 + seconds).unwrap());
        assert_eq!(duration(None, None), "Not started");
        assert_eq!(duration(None, at(5)), "Not started");
        assert_eq!(duration(at(0), None), "In progress");
        assert_eq!(duration(at(0), at(0)), "0s");
        assert_eq!(duration(at(0), at(59)), "59s");
        assert_eq!(duration(at(0), at(60)), "1m 0s");
        assert_eq!(duration(at(0), at(65)), "1m 5s");
        assert_eq!(duration(at(0), at(3599)), "59m 59s");
        assert_eq!(duration(at(0), at(3600)), "1h 0m");
        assert_eq!(duration(at(0), at(2 * 3600 + 3 * 60 + 25)), "2h 3m");
    }

    #[test]
    fn dim_is_the_foreground_faded_or_the_dim_attribute() {
        let theme = Theme::default();
        assert_ne!(
            Tint::Dim.style(&theme).fg,
            Some(theme.color(theme.palette.foreground, theme.background()))
        );
        assert_eq!(
            Tint::Accent(Accent::Red).style(&theme).fg,
            Some(theme.accent(Accent::Red))
        );
    }
}
