//! A workflow run's Logs tab: `Full Logs for Run #<n>` and every job's logs below it, one after another, in one
//! fenced block. Python's `WorkflowRunLogsTabPane`.
//!
//! Python fetches the logs as soon as the run is selected; here they're fetched the first time the tab shows, one job
//! at a time in job order, taking any logs the Jobs tab already fetched for the selection.

use std::collections::HashMap;
use std::fmt::Write as _;

use lazy_github_core::models::WorkflowJob;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::effect::RequestId;
use crate::theme::Theme;

use super::super::overview_text::{HeaderStyles, append, push_label};

/// The aggregated logs longer than this, in characters, are cut here.
const MAX_LOG_CHARS: usize = 100_000;

/// What shows while the logs load.
const LOADING: &str = "Loading full workflow logs...";

/// What shows for a run without jobs, or whose jobs failed to load.
const NO_JOBS: &str = "No jobs found for this workflow run.";

/// How far the tab has got with the logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogsState {
    /// The tab hasn't shown yet.
    NotStarted,
    /// The tab showed before the jobs arrived: it starts once they do.
    WaitingForJobs,
    /// Job `next`'s logs were requested with `request`; the jobs before it have theirs.
    Fetching { next: usize, request: RequestId },
    /// The text is final.
    Done,
}

/// The Logs tab of one run.
#[derive(Debug)]
pub(in crate::screens::main) struct LogsTab {
    state: LogsState,
    scroll: ScrollView,
    body: Body,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    run_number: u64,
    /// The loading line, then the logs or `No jobs found`.
    text: Markdown,
    layouts: LayoutCache,
}

/// What the tab wants next from the run's details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Next {
    /// Nothing: it's waiting, or done.
    Nothing,
    /// Job `index`'s logs, which aren't in the selection's cache.
    Request(usize),
}

impl LogsTab {
    /// The Logs tab of run `run_number`, not started.
    pub(super) fn new(run_number: u64) -> Self {
        Self {
            state: LogsState::NotStarted,
            scroll: ScrollView::default(),
            body: Body {
                run_number,
                text: Markdown::parse(LOADING),
                layouts: LayoutCache::default(),
            },
        }
    }

    /// Where the tab has got.
    #[cfg(test)]
    pub(super) fn state(&self) -> LogsState {
        self.state
    }

    /// The logs request on its way, if any.
    pub(super) fn pending(&self) -> Option<RequestId> {
        match self.state {
            LogsState::Fetching { request, .. } => Some(request),
            _ => None,
        }
    }

    /// The job whose logs are on their way, if any.
    pub(super) fn fetching_job(&self) -> Option<usize> {
        match self.state {
            LogsState::Fetching { next, .. } => Some(next),
            _ => None,
        }
    }

    /// The tab showed: the first time, it starts once the jobs (`None` while they load) are in. Says what it wants.
    pub(super) fn on_shown(&mut self, jobs: Option<&[WorkflowJob]>, logs: &HashMap<u64, Option<String>>) -> Next {
        if self.state != LogsState::NotStarted {
            return Next::Nothing;
        }
        self.state = LogsState::WaitingForJobs;
        match jobs {
            Some(jobs) => self.walk(0, jobs, logs),
            None => Next::Nothing,
        }
    }

    /// The jobs arrived: a tab that already showed starts.
    pub(super) fn on_jobs(&mut self, jobs: &[WorkflowJob], logs: &HashMap<u64, Option<String>>) -> Next {
        if self.state != LogsState::WaitingForJobs {
            return Next::Nothing;
        }
        self.walk(0, jobs, logs)
    }

    /// The pending job's logs arrived and are in `logs`: on to the next job.
    pub(super) fn on_job_logs(&mut self, jobs: &[WorkflowJob], logs: &HashMap<u64, Option<String>>) -> Next {
        let LogsState::Fetching { next, .. } = self.state else {
            return Next::Nothing;
        };
        self.walk(next + 1, jobs, logs)
    }

    /// Job `index`'s logs were requested with `request`.
    pub(super) fn set_fetching(&mut self, index: usize, request: RequestId) {
        self.state = LogsState::Fetching { next: index, request };
    }

    /// From job `from` on, the first job without logs in `logs` is the one to ask for. Once every job has been
    /// through, the text is built.
    fn walk(&mut self, from: usize, jobs: &[WorkflowJob], logs: &HashMap<u64, Option<String>>) -> Next {
        // Logs fetched for the selection are reused; a job whose logs failed earlier is asked again, as Python asks
        // for every job.
        let missing = jobs
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, job)| !matches!(logs.get(&job.id), Some(Some(_))));
        if let Some((index, _)) = missing {
            return Next::Request(index);
        }
        self.state = LogsState::Done;
        self.body.text = Markdown::parse(&full_logs_markdown(jobs, logs));
        self.body.layouts.clear();
        Next::Nothing
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    /// The body's first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.scroll.render(frame, area, theme, &mut self.body);
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let Body {
            run_number,
            text,
            layouts,
        } = self;
        layouts.get(width, theme, || {
            let mut out = Rendered::default();
            if width == 0 {
                return out;
            }
            let HeaderStyles { text: style, .. } = HeaderStyles::new(theme);
            let heading = format!("Full Logs for Run #{run_number}");
            push_label(
                &mut out,
                &[TextRun::new(heading, style.add_modifier(Modifier::BOLD))],
                width,
            );
            append(&mut out, text.layout(width, theme));
            out
        })
    }
}

/// The markdown the tab shows for `jobs` and their `logs` (by job id, `None` or absent for a job without): for each
/// job in order, a header between two lines of `=` and its logs verbatim or `(No logs available for this job)`, all
/// in one fenced block, cut after [`MAX_LOG_CHARS`] characters with a note after a blank line. Without jobs, `No jobs
/// found for this workflow run.`
///
/// A log's last line ending is its terminator, not a blank row: one blank row follows a job's logs, as in Python,
/// whose gh backend drops it (Py bug B47). Blank and header-shaped lines inside the logs stay, per B47's answer.
pub(super) fn full_logs_markdown(jobs: &[WorkflowJob], logs: &HashMap<u64, Option<String>>) -> String {
    if jobs.is_empty() {
        return NO_JOBS.to_owned();
    }
    let rule = "=".repeat(80);
    let mut all = String::new();
    for job in jobs {
        // Python's `str(None)` for a job without a conclusion.
        let conclusion = job.conclusion.as_ref().map_or("None", |conclusion| conclusion.as_str());
        let text = match logs.get(&job.id).and_then(Option::as_deref).map(without_terminator) {
            Some(text) if !text.is_empty() => text,
            _ => "(No logs available for this job)\n",
        };
        // Writing to a `String` can't fail.
        let _ = write!(
            all,
            "\n{rule}\nJob: {} (Status: {}, Conclusion: {conclusion})\n{rule}\n\n{text}\n",
            job.name,
            job.status.as_str()
        );
    }
    if let Some((cut, _)) = all.char_indices().nth(MAX_LOG_CHARS) {
        all.truncate(cut);
        all.push_str("\n\n... (logs truncated, too long to display)");
    }
    format!("```\n{all}\n```")
}

/// `text` without the line ending of its last line.
fn without_terminator(text: &str) -> &str {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.strip_suffix('\r').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::run_details::tests::fixture_jobs;

    fn build_logs() -> HashMap<u64, Option<String>> {
        HashMap::from([
            (6_000_001, Some("build line 1\nbuild line 2\n".to_owned())),
            (6_000_002, Some("test line\n".to_owned())),
            (6_000_003, None),
            (6_000_004, Some("deploy line\n".to_owned())),
        ])
    }

    #[test]
    fn run_42s_logs_are_one_block_with_a_section_per_job() {
        let jobs = fixture_jobs(5_000_042);
        let rule = "=".repeat(80);
        let expected = format!(
            "```\n\
             \n{rule}\nJob: build (Status: completed, Conclusion: success)\n{rule}\n\nbuild line 1\nbuild line 2\n\
             \n{rule}\nJob: test (Status: completed, Conclusion: failure)\n{rule}\n\ntest line\n\
             \n{rule}\nJob: lint (Status: completed, Conclusion: skipped)\n{rule}\n\n(No logs available for this job)\n\n\
             \n{rule}\nJob: deploy (Status: completed, Conclusion: cancelled)\n{rule}\n\ndeploy line\n\
             \n```"
        );
        assert_eq!(full_logs_markdown(&jobs, &build_logs()), expected);
    }

    /// `run-details`' `logs`: one blank row between a job's last log line and the next job's rule, as Python shows.
    #[test]
    fn a_logs_last_line_ending_isnt_a_blank_row() {
        let jobs = &fixture_jobs(5_000_042)[..1];
        let rule = "=".repeat(80);
        let header = format!("```\n\n{rule}\nJob: build (Status: completed, Conclusion: success)\n{rule}\n\n");
        for (text, shown) in [
            ("a\nb\n", "a\nb\n"),
            ("a\r\nb\r\n", "a\r\nb\n"),
            ("a\nb", "a\nb\n"),
            // Blank lines inside the logs, the last one included, stay (B47's answer).
            ("a\n\nb\n\n", "a\n\nb\n\n"),
        ] {
            let logs = HashMap::from([(6_000_001, Some(text.to_owned()))]);
            assert_eq!(
                full_logs_markdown(jobs, &logs),
                format!("{header}{shown}\n```"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_missing_or_empty_log_is_no_logs() {
        let jobs = &fixture_jobs(5_000_042)[..1];
        let none = HashMap::new();
        let empty = HashMap::from([(6_000_001, Some(String::new()))]);
        let newline = HashMap::from([(6_000_001, Some("\n".to_owned()))]);
        for logs in [none, empty, newline] {
            let markdown = full_logs_markdown(jobs, &logs);
            assert!(
                markdown.ends_with("\n\n(No logs available for this job)\n\n\n```"),
                "{markdown}"
            );
        }
    }

    #[test]
    fn a_job_without_a_conclusion_shows_none() {
        let jobs = fixture_jobs(5_000_041);
        let markdown = full_logs_markdown(&jobs, &HashMap::new());
        assert!(
            markdown.contains("Job: build (Status: in_progress, Conclusion: None)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("Job: test (Status: queued, Conclusion: None)"),
            "{markdown}"
        );
    }

    #[test]
    fn no_jobs_is_a_plain_line() {
        assert_eq!(full_logs_markdown(&[], &HashMap::new()), NO_JOBS);
    }

    const TRUNCATED: &str = "\n\n... (logs truncated, too long to display)";

    /// One job whose section, header and all, is `total` characters long, its logs `é`s, and that section.
    fn section_of(total: usize) -> (Vec<WorkflowJob>, HashMap<u64, Option<String>>, String) {
        let jobs = fixture_jobs(5_000_042)[..1].to_vec();
        let empty = HashMap::from([(6_000_001, Some("x".to_owned()))]);
        // The section with a one-character log, less that character and the newline after it.
        let fixed = full_logs_markdown(&jobs, &empty);
        let fixed = &fixed["```\n".len()..fixed.len() - "\n```".len()];
        let header = &fixed[..fixed.len() - "x\n".len()];
        let fill = total - header.chars().count() - 1;
        let text = "é".repeat(fill);
        let section = format!("{header}{text}\n");
        assert_eq!(section.chars().count(), total);
        (jobs, HashMap::from([(6_000_001, Some(text))]), section)
    }

    #[test]
    fn the_logs_are_cut_at_100000_characters() {
        for total in [99_999, 100_000] {
            let (jobs, logs, section) = section_of(total);
            assert_eq!(full_logs_markdown(&jobs, &logs), format!("```\n{section}\n```"));
        }
        let (jobs, logs, section) = section_of(100_001);
        // The cut falls before the last character, the newline; the multi-byte ones before it are kept whole.
        let kept: String = section.chars().take(100_000).collect();
        assert_eq!(full_logs_markdown(&jobs, &logs), format!("```\n{kept}{TRUNCATED}\n```"));
    }
}
