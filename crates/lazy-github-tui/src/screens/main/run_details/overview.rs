//! A workflow run's Overview tab: the status and title, the run's fields, the commit, a summary line per job and the
//! re-run buttons, scrolling as one body. Python's `WorkflowRunOverviewTabPane.compose`.
//!
//! Python puts the job lines and the buttons in two nested scroll areas that split the height left over, each its
//! own focus stop that shows nothing; here they're part of the one body, and the buttons are its only inner stops
//! (#241's owner decision, a Textual-only layout detail).

use std::sync::Arc;

use lazy_github_core::models::{WorkflowJob, WorkflowRun};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::components::button::{self, Button, ButtonVariant};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::link_map::LinkSink;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::theme::Theme;

use super::super::overview_text::{HeaderStyles, append, push_label, push_rule};
use super::status::{duration, job_icon, run_status};

/// Textual's `margin: 1 2` on the buttons: the columns left of each one.
const BUTTON_MARGIN_LEFT: u16 = 2;

/// Rows each button takes in the body: the button and the blank row of margin below it, which collapses with the
/// next one's top margin.
const BUTTON_PITCH: usize = button::HEIGHT as usize + 1;

/// The Overview of one run, its scroll position and which of its stops has focus.
#[derive(Debug)]
pub(in crate::screens::main) struct RunOverview {
    scroll: ScrollView,
    body: Body,
    /// "Re-run Workflow", then "Re-run Failed Jobs".
    buttons: [Button; 2],
    /// The focused stop inside the content: `None` for the body itself, else the index of a shown button.
    focus: Option<usize>,
    /// Where each shown button was drawn last, clipped to the view, by index.
    button_areas: Vec<(Rect, usize)>,
    /// The button a left press landed on, until the release: the two make a click, which presses it.
    armed: Option<usize>,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    run: Arc<WorkflowRun>,
    /// The fields, parsed once.
    fields: Markdown,
    /// The commit's lines, parsed once.
    commit: Markdown,
    /// `None` until the jobs arrive.
    jobs: Option<Arc<[WorkflowJob]>>,
    layouts: LayoutCache,
}

impl Body {
    /// How many buttons show: "Re-run Failed Jobs" only once the jobs are in and one completed with a failure.
    fn buttons_shown(&self) -> usize {
        let failed = self
            .jobs
            .as_deref()
            .is_some_and(|jobs| jobs.iter().any(WorkflowJob::failed));
        1 + usize::from(failed)
    }
}

impl RunOverview {
    /// The Overview of `run`, scrolled to the top, its jobs loading.
    pub(super) fn new(run: Arc<WorkflowRun>) -> Self {
        Self {
            scroll: ScrollView::default(),
            body: Body {
                fields: Markdown::parse(&fields_source(&run)),
                commit: Markdown::parse(&commit_source(&run)),
                run,
                jobs: None,
                layouts: LayoutCache::default(),
            },
            buttons: [
                Button::new("Re-run Workflow", ButtonVariant::Primary),
                Button::new("Re-run Failed Jobs", ButtonVariant::Default),
            ],
            focus: None,
            button_areas: Vec::new(),
            armed: None,
        }
    }

    /// The jobs arrived: their lines replace "Loading jobs...", and "Re-run Failed Jobs" may show.
    pub(super) fn set_jobs(&mut self, jobs: Arc<[WorkflowJob]>) {
        self.body.jobs = Some(jobs);
        self.body.layouts.clear();
    }

    /// The labels of the buttons that show, in order.
    #[cfg(test)]
    pub(in crate::screens::main) fn shown_buttons(&self) -> Vec<&str> {
        self.buttons[..self.body.buttons_shown()]
            .iter()
            .map(Button::label)
            .collect()
    }

    /// The label of the focused button, if one has focus.
    #[cfg(test)]
    pub(in crate::screens::main) fn focused_button(&self) -> Option<&str> {
        self.focus.map(|index| self.buttons[index].label())
    }

    /// The first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Whether a button has focus, rather than the body itself.
    pub(super) fn button_focused(&self) -> bool {
        self.focus.is_some()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    /// Moves focus to the next stop `forward` or back, and says whether there was one.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        let shown = self.body.buttons_shown();
        self.focus = match (self.focus, forward) {
            (None, true) => Some(0),
            (Some(index), true) if index + 1 < shown => Some(index + 1),
            (Some(0), false) => None,
            (Some(index), false) => Some(index - 1),
            _ => return false,
        };
        self.reveal_focused();
        true
    }

    pub(super) fn focus_first(&mut self) {
        self.focus = None;
    }

    pub(super) fn focus_last(&mut self) {
        self.focus = Some(self.body.buttons_shown() - 1);
        self.reveal_focused();
    }

    /// A left press at `position`, inside the content: it focuses the button there, else the body.
    pub(super) fn click(&mut self, position: Position) {
        self.focus = self.button_at(position);
        self.armed = self.focus;
    }

    /// A left release at `position`: whether it completes a click on the button the press landed on.
    pub(super) fn release(&mut self, position: Position) -> bool {
        self.armed
            .take()
            .is_some_and(|index| self.button_at(position) == Some(index))
    }

    fn button_at(&self, position: Position) -> Option<usize> {
        self.button_areas
            .iter()
            .find(|(area, _)| area.contains(position))
            .map(|&(_, index)| index)
    }

    /// `enter` on the focused button, or a click on it, at `now`: which re-run it asks for, `Some(true)` for
    /// "Re-run Failed Jobs". `None` without a focused button, or for a press inside its debounce.
    pub(super) fn press(&mut self, now: tokio::time::Instant) -> Option<bool> {
        let index = self.focus?;
        self.buttons[index].press(now).then_some(index == 1)
    }

    /// Scrolls the focused button into view, as Textual does for a widget that takes focus.
    fn reveal_focused(&mut self) {
        if let Some(index) = self.focus {
            let top = self.button_top(index, self.scroll.last_size().0);
            self.scroll.reveal(top..top + usize::from(button::HEIGHT));
        }
    }

    /// The content row button `index` starts on, in a body `content` rows long: the buttons end it.
    fn button_top(&self, index: usize, content: usize) -> usize {
        content.saturating_sub(BUTTON_PITCH * self.body.buttons_shown()) + BUTTON_PITCH * index
    }

    /// Draws the body into `area`, and the shown buttons over the blank rows it leaves for them. `focused` says whether
    /// focus is inside the content. The body's only link, `(#N)`, is a terminal hyperlink, recorded into `hyperlinks`
    /// when the frame keeps them; it's no in-app link (#196).
    pub(super) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: &Theme,
        focused: bool,
        hyperlinks: Option<&LinkSink>,
    ) {
        self.scroll.render(frame, area, theme, &mut self.body);
        if let Some(hyperlinks) = hyperlinks {
            self.scroll.record_links(area, theme, &mut self.body, hyperlinks);
        }
        self.button_areas.clear();
        let (content, window) = self.scroll.last_size();
        let offset = self.scroll.offset();
        // Beside a scrollbar, the buttons stay clear of it.
        let width = if content > window {
            area.width.saturating_sub(2)
        } else {
            area.width
        };
        for index in 0..self.body.buttons_shown() {
            let top = self.button_top(index, content);
            let button = &mut self.buttons[index];
            let size = Rect::new(0, 0, button.width(theme), button::HEIGHT);
            let mut scratch = Buffer::empty(size);
            button.render(&mut scratch, size, theme, focused && self.focus == Some(index));
            let mut drawn: Option<Rect> = None;
            for dy in 0..button::HEIGHT {
                let Some(row) = (top + usize::from(dy)).checked_sub(offset).filter(|&row| row < window) else {
                    continue;
                };
                // `row` is below the window's height, a u16.
                let y = area.y + u16::try_from(row).unwrap_or(u16::MAX);
                let visible =
                    Rect::new(area.x + BUTTON_MARGIN_LEFT, y, size.width, 1).intersection(Rect { width, ..area });
                for x in visible.left()..visible.right() {
                    let cell = scratch[(x - area.x - BUTTON_MARGIN_LEFT, dy)].clone();
                    frame.buffer_mut()[(x, y)] = cell;
                }
                drawn = Some(drawn.map_or(visible, |drawn| drawn.union(visible)));
            }
            if let Some(drawn) = drawn.filter(|drawn| !drawn.is_empty()) {
                self.button_areas.push((drawn, index));
            }
        }
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let shown = self.buttons_shown();
        let Body {
            run,
            fields,
            commit,
            jobs,
            layouts,
        } = self;
        layouts.get(width, theme, || {
            overview(run, fields, commit, jobs.as_deref(), shown, width, theme)
        })
    }
}

/// `**Label:** value` lines, one field per line (Py bug B62: Python's single newlines run them together).
fn markdown_lines(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(label, value)| format!("**{label}:** {value}"))
        .collect::<Vec<_>>()
        // A hard break: two trailing spaces.
        .join("  \n")
}

/// The run's fields: `Conclusion` only when it has one, `Attempt` only when it's set and not 0, the times in UTC (Py
/// bug B26).
fn fields_source(run: &WorkflowRun) -> String {
    let time = |at: jiff::Timestamp| at.strftime("%Y-%m-%d %H:%M:%S").to_string();
    let mut fields = vec![
        ("Run Number", run.run_number.to_string()),
        ("Event", run.event.clone()),
        ("Branch", run.head_branch.clone()),
        ("Status", run.status.clone()),
    ];
    if let Some(conclusion) = run.conclusion.as_deref().filter(|conclusion| !conclusion.is_empty()) {
        fields.push(("Conclusion", conclusion.to_owned()));
    }
    if let Some(attempt) = run.run_attempt.filter(|&attempt| attempt != 0) {
        fields.push(("Attempt", attempt.to_string()));
    }
    fields.push(("Created", time(run.created_at)));
    fields.push(("Updated", time(run.updated_at)));
    markdown_lines(&fields)
}

/// The commit's SHA, short as inline code and in full, and who triggered and started the run.
fn commit_source(run: &WorkflowRun) -> String {
    let short: String = run.head_sha.chars().take(7).collect();
    markdown_lines(&[
        ("SHA", format!("`{short}` ({})", run.head_sha)),
        ("Triggered by", run.triggering_actor.login.clone()),
        ("Actor", run.actor.login.clone()),
    ])
}

/// The whole tab laid out `width` cells wide, with blank rows where the `shown` buttons go.
fn overview(
    run: &WorkflowRun,
    fields: &Markdown,
    commit: &Markdown,
    jobs: Option<&[WorkflowJob]>,
    shown: usize,
    width: u16,
    theme: &Theme,
) -> Rendered {
    let mut out = Rendered::default();
    if width == 0 {
        return out;
    }
    let HeaderStyles { text, .. } = HeaderStyles::new(theme);
    let bold = text.add_modifier(Modifier::BOLD);
    let heading = |out: &mut Rendered, label: &str| push_label(out, &[TextRun::new(label, bold)], width);

    let (status, tint) = run_status(run);
    // The title as typed: Python passes it as a markup variable, so brackets in it are text. `(#N)` links to the run
    // in the browser, through the terminal.
    let header = [
        TextRun::new(status, text.patch(tint.style(theme))),
        TextRun::new(" ", text),
        TextRun::new(run.display_title.as_str(), bold),
        TextRun::new(" ", text),
        TextRun::link(format!("(#{})", run.run_number), text, run.html_url.as_str()),
    ];
    push_label(&mut out, &header, width);
    append(&mut out, fields.layout(width, theme));

    push_rule(&mut out, width, theme);
    heading(&mut out, "Commit Information");
    append(&mut out, commit.layout(width, theme));

    push_rule(&mut out, width, theme);
    heading(&mut out, "Jobs Summary");
    match jobs {
        None => push_label(&mut out, &[TextRun::new("Loading jobs...", text)], width),
        Some([]) => push_label(&mut out, &[TextRun::new("No jobs found", text)], width),
        Some(jobs) => {
            for job in jobs {
                let (icon, tint) = job_icon(&job.status, job.conclusion.as_ref());
                let line = [
                    TextRun::new(icon, text.patch(tint.style(theme))),
                    TextRun::new(
                        format!(" {} ({})", job.name, duration(job.started_at, job.completed_at)),
                        text,
                    ),
                ];
                push_label(&mut out, &line, width);
            }
        }
    }

    push_rule(&mut out, width, theme);
    heading(&mut out, "Actions");
    // The first button's top margin, then each button and the margin below it.
    out.lines.push(Line::default());
    out.lines
        .extend(std::iter::repeat_with(Line::default).take(BUTTON_PITCH * shown));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::run_details::tests::{fixture_jobs, fixture_run};

    fn text(rendered: &Rendered) -> Vec<String> {
        rendered
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }

    fn lines(run: &WorkflowRun, jobs: Option<&[WorkflowJob]>, width: u16) -> Vec<String> {
        let tab = RunOverview::new(Arc::new(run.clone()));
        let shown = 1 + usize::from(jobs.is_some_and(|jobs| jobs.iter().any(WorkflowJob::failed)));
        text(&overview(
            run,
            &tab.body.fields,
            &tab.body.commit,
            jobs,
            shown,
            width,
            &Theme::default(),
        ))
    }

    #[test]
    fn run_42_with_its_jobs() {
        let jobs = fixture_jobs(5_000_042);
        let rule = "─".repeat(94);
        assert_eq!(
            lines(&fixture_run(42), Some(&jobs), 94),
            [
                "✗ Failure Fix the flaky test (#42)",
                "  Run Number: 42",
                "  Event: push",
                "  Branch: main",
                "  Status: completed",
                "  Conclusion: failure",
                "  Attempt: 2",
                "  Created: 2026-03-04 05:06:07",
                "  Updated: 2026-03-04 07:20:00",
                "",
                "",
                &rule,
                "",
                "Commit Information",
                "  SHA: 0123456 (0123456789abcdef0123456789abcdef01234567)",
                "  Triggered by: parity-friend",
                "  Actor: parity-bot",
                "",
                "",
                &rule,
                "",
                "Jobs Summary",
                "✓ build (1m 5s)",
                "✗ test (2h 3m)",
                "− lint (0s)",
                "○ deploy (9s)",
                "",
                &rule,
                "",
                "Actions",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]
        );
    }

    #[test]
    fn the_header_is_colored_and_the_title_bold() {
        let theme = Theme::default();
        let run = fixture_run(42);
        let tab = RunOverview::new(Arc::new(run.clone()));
        let rendered = overview(&run, &tab.body.fields, &tab.body.commit, None, 1, 94, &theme);
        let header = &rendered.lines[0].spans;
        assert_eq!(header[0].content, "✗ Failure");
        assert_eq!(header[0].style.fg, Some(theme.accent(crate::theme::Accent::Red)));
        assert!(header[2].style.add_modifier.contains(Modifier::BOLD));
        assert!(!header[4].style.add_modifier.contains(Modifier::BOLD));
        // The one link is `(#42)`, the run's page, which the Overview records as a terminal hyperlink only (#196, #356).
        let links: Vec<_> = rendered
            .links
            .iter()
            .map(|l| (l.row, l.columns.clone(), &*l.target))
            .collect();
        assert_eq!(links, [(0, 29..34, run.html_url.as_str())]);
    }

    #[test]
    fn loading_then_no_jobs() {
        let run = fixture_run(42);
        let loading = lines(&run, None, 94);
        assert!(loading.contains(&"Loading jobs...".to_owned()), "{loading:#?}");
        let none = lines(&run, Some(&[]), 94);
        assert!(none.contains(&"No jobs found".to_owned()), "{none:#?}");
        assert!(!none.contains(&"Loading jobs...".to_owned()));
        // One button's rows after "Actions": its margin, the button and the margin below.
        assert_eq!(none.len() - none.iter().position(|line| line == "Actions").unwrap(), 6);
    }

    #[test]
    fn conclusion_and_attempt_are_left_out_when_unset() {
        let mut run = fixture_run(41);
        run.conclusion = None;
        run.run_attempt = Some(0);
        let lines = lines(&run, None, 94);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("Conclusion:") && !line.contains("Attempt:")),
            "{lines:#?}"
        );
        run.conclusion = Some(String::new());
        run.run_attempt = None;
        let lines = self::lines(&run, None, 94);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("Conclusion:") && !line.contains("Attempt:")),
            "{lines:#?}"
        );
        assert!(lines.contains(&"  Status: in_progress".to_owned()), "{lines:#?}");
    }

    #[test]
    fn a_title_with_markup_is_literal() {
        let mut run = fixture_run(42);
        run.display_title = "[b]not bold[/] and [/]".into();
        assert_eq!(lines(&run, None, 94)[0], "✗ Failure [b]not bold[/] and [/] (#42)");
    }

    #[test]
    fn the_header_wraps_and_a_zero_width_is_empty() {
        let run = fixture_run(42);
        assert_eq!(lines(&run, None, 20)[..2], ["✗ Failure Fix the", "flaky test (#42)"]);
        assert!(lines(&run, None, 0).is_empty());
    }
}
