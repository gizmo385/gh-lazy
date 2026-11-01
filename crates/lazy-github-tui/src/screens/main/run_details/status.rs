//! How a run's and a job's state read on screen, and how long a job took: shared by the run's Overview and Jobs tabs.
//! Python's `_format_run_status`, `_format_job_status` and `_calculate_duration`.

use std::borrow::Cow;

use jiff::Timestamp;
use lazy_github_core::models::{JobConclusion, JobStatus, WorkflowRun};
use ratatui::style::{Modifier, Style};

use super::super::overview_text::mark;
use crate::theme::Theme;

/// What a status means, which picks its color: one of the marks, or Rich's `dim`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::screens::main) enum Tint {
    /// Succeeded (`[greenyellow]`).
    Positive,
    /// Failed (`[red]`).
    Negative,
    /// Cancelled or queued (`[yellow]`).
    Pending,
    /// In progress (`[cyan]`).
    Running,
    /// Anything else (`[dim]`).
    Dim,
}

impl Tint {
    /// The style for text in this tint, patched over the text style it's drawn in. Rich's `dim` is
    /// `Modifier::DIM` alone, so it dims whatever color the text has where it's drawn (a tree row's, or its cursor's),
    /// which [`Theme::resolve_dim`] blends at the end of the frame as Textual does.
    pub(in crate::screens::main) fn style(self, theme: &Theme) -> Style {
        let marks = &theme.roles().marks;
        match self {
            Tint::Positive => mark(theme, marks.positive),
            Tint::Negative => mark(theme, marks.negative),
            Tint::Pending => mark(theme, marks.pending),
            Tint::Running => mark(theme, marks.running),
            Tint::Dim => Style::new().add_modifier(Modifier::DIM),
        }
    }
}

/// The run's state as the Overview's header starts: `✓ Success`, `⋯ In Progress`, or the raw `status` dimmed when
/// it's none of the known ones (a run that timed out reads `completed`).
pub(in crate::screens::main) fn run_status(run: &WorkflowRun) -> (Cow<'_, str>, Tint) {
    let (text, tint) = match (run.status.as_str(), run.conclusion.as_deref()) {
        ("completed", Some("success")) => ("✓ Success", Tint::Positive),
        ("completed", Some("failure")) => ("✗ Failure", Tint::Negative),
        ("completed", Some("cancelled")) => ("○ Cancelled", Tint::Pending),
        ("completed", Some("skipped")) => ("− Skipped", Tint::Dim),
        ("in_progress", _) => ("⋯ In Progress", Tint::Running),
        ("queued", _) => ("⋯ Queued", Tint::Pending),
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
        (JobStatus::Completed, Some(JobConclusion::Success)) => ("✓", Tint::Positive),
        (JobStatus::Completed, Some(JobConclusion::Failure)) => ("✗", Tint::Negative),
        (JobStatus::Completed, Some(JobConclusion::Cancelled)) => ("○", Tint::Pending),
        (JobStatus::Completed, Some(JobConclusion::Skipped)) => ("−", Tint::Dim),
        (JobStatus::InProgress, _) => ("⋯", Tint::Running),
        (JobStatus::Queued, _) => ("⋯", Tint::Pending),
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
        let rows = [
            ("completed", Some("success"), "✓ Success", Tint::Positive),
            ("completed", Some("failure"), "✗ Failure", Tint::Negative),
            ("completed", Some("cancelled"), "○ Cancelled", Tint::Pending),
            ("completed", Some("skipped"), "− Skipped", Tint::Dim),
            ("in_progress", None, "⋯ In Progress", Tint::Running),
            ("in_progress", Some("success"), "⋯ In Progress", Tint::Running),
            ("queued", None, "⋯ Queued", Tint::Pending),
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
        let completed = |conclusion: C| job_icon(&S::Completed, Some(&conclusion));
        assert_eq!(completed(C::Success), ("✓", Tint::Positive));
        assert_eq!(completed(C::Failure), ("✗", Tint::Negative));
        assert_eq!(completed(C::Cancelled), ("○", Tint::Pending));
        assert_eq!(completed(C::Skipped), ("−", Tint::Dim));
        for other in [C::Neutral, C::TimedOut, C::ActionRequired, C::Other("new".into())] {
            assert_eq!(completed(other), ("?", Tint::Dim));
        }
        assert_eq!(job_icon(&S::Completed, None), ("?", Tint::Dim));
        assert_eq!(job_icon(&S::InProgress, None), ("⋯", Tint::Running));
        // A queued job shows `⋯` in yellow, never `?` (Py bug B63 is the Jobs tab's).
        assert_eq!(job_icon(&S::Queued, None), ("⋯", Tint::Pending));
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
    fn dim_is_the_dim_attribute_over_the_texts_own_color() {
        for theme in [Theme::default(), Theme::by_name("ansi-dark").unwrap()] {
            assert_eq!(Tint::Dim.style(&theme), Style::new().add_modifier(Modifier::DIM));
        }
        let theme = Theme::default();
        assert_eq!(
            Tint::Negative.style(&theme).fg,
            Some(theme.color(theme.roles().marks.negative, theme.background()))
        );
    }
}
