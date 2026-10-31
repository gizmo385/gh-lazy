//! A workflow run's Jobs tab: a heading, the tree of jobs and their steps, a rule, and below it the logs of the job
//! picked with `l`, scrolling as one body. Python's `WorkflowRunJobsTabPane`.
//!
//! The tree takes the rows the rest leaves in the tab, and at least [`TREE_MIN_ROWS`] (Python's `height: 1fr;
//! min-height: 20`). It's drawn whole off screen and its visible rows copied over the blank rows the body leaves for
//! it, so it scrolls with the body; its own lines scroll inside it.

use std::ops::Range;
use std::sync::Arc;

use crossterm::event::MouseEvent;
use lazy_github_core::models::{JobConclusion, JobStatus, WorkflowJob};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::components::tree::{Tree, TreeAction};
use crate::effect::RequestId;
use crate::keymap::{BindingId, Keymap};
use crate::theme::{Accent, RichAnsi, Theme};

use super::super::overview_text::{HeaderStyles, append, push_label, push_rule};
use super::status::{Tint, job_icon};

/// The fewest rows the tree takes.
const TREE_MIN_ROWS: usize = 20;

/// Logs longer than this, in characters, are cut here.
const MAX_LOG_CHARS: usize = 50_000;

/// Textual's vertical scrollbar width, which the body loses while it overflows.
const SCROLLBAR_WIDTH: u16 = 2;

/// What the tree's nodes carry: the index of a job in the run's jobs, for the job and for each of its steps. `None`
/// for the `No jobs found` leaf.
type JobRef = Option<usize>;

/// What shows below `Job Logs`.
#[derive(Debug)]
enum JobLogsView {
    /// No job picked yet: the hint.
    Hint,
    /// Job `job`'s logs, requested with `request`.
    Loading { job: usize, request: RequestId },
    /// Job `job`'s logs, as a fenced block.
    Shown { job: usize, logs: Markdown },
    /// Job `job` has no logs, or they failed to load.
    Missing { job: usize },
}

/// The focus stops inside the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The tab's scroll area.
    Body,
    Tree,
}

/// The Jobs tab of one run.
#[derive(Debug)]
pub(in crate::screens::main) struct JobsTab {
    scroll: ScrollView,
    body: Body,
    tree: Tree<JobRef>,
    focus: Stop,
    /// Where the tree's visible rows were drawn last, and how many of its rows were scrolled off above them.
    tree_view: Option<TreeView>,
    /// The body rows the tree took in the last frame.
    tree_rows: Option<Range<usize>>,
}

/// Where the tree landed in the last frame.
#[derive(Debug, Clone, Copy)]
struct TreeView {
    /// Its visible rows, on screen.
    visible: Rect,
    /// Its whole size.
    size: Rect,
    /// Its rows above the visible ones.
    hidden_above: u16,
}

/// Everything around the tree, with its layouts.
#[derive(Debug)]
struct Body {
    run_number: u64,
    /// `None` until the jobs arrive.
    jobs: Option<Arc<[WorkflowJob]>>,
    /// The hint, with the key from the keymap.
    hint: Markdown,
    logs: JobLogsView,
    /// The tab's height at the last draw: the tree takes what's left of it.
    height: u16,
    /// Everything below the tree, by width.
    below: LayoutCache,
    /// The whole body, with blank rows for the tree, by width.
    layouts: LayoutCache,
}

impl JobsTab {
    /// The Jobs tab of run `run_number`, its tree empty until the jobs arrive, the hint naming `l`'s key in `keymap`.
    pub(super) fn new(run_number: u64, keymap: &Keymap) -> Self {
        let key = keymap.first_key_display(BindingId::WorkflowsViewJobLogs);
        Self {
            scroll: ScrollView::default(),
            body: Body {
                run_number,
                jobs: None,
                hint: Markdown::parse(&format!("Select a job and press '{key}' to view logs.")),
                logs: JobLogsView::Hint,
                height: 0,
                below: LayoutCache::default(),
                layouts: LayoutCache::default(),
            },
            tree: Tree::new(),
            focus: Stop::Body,
            tree_view: None,
            tree_rows: None,
        }
    }

    /// The jobs arrived: the tree is built from them, collapsed.
    pub(super) fn set_jobs(&mut self, jobs: Arc<[WorkflowJob]>, theme: &Theme) {
        self.tree = job_tree(&jobs, theme);
        self.body.jobs = Some(jobs);
        self.body.clear();
    }

    /// The theme changed: the labels' icons take its colors, the tree's cursor and expansion staying as they are.
    pub(super) fn retheme(&mut self, theme: &Theme) {
        let Some(jobs) = &self.body.jobs else { return };
        // In `job_tree`'s order: each job, then its steps. "No jobs found" has no colors.
        let labels = jobs.iter().flat_map(|job| {
            std::iter::once(label(&job.name, &job.status, job.conclusion.as_ref(), theme)).chain(
                job.steps
                    .iter()
                    .map(|step| label(&step.name, &step.status, step.conclusion.as_ref(), theme)),
            )
        });
        self.tree.restyle_labels(labels);
    }

    /// The job under the tree's cursor, or on its step: `None` without a cursor or on `No jobs found`.
    pub(super) fn cursor_job(&self) -> Option<usize> {
        self.tree.cursor_node().and_then(|(_, &job)| job)
    }

    /// The job at index `job` of the run's jobs, once they're in.
    pub(super) fn job(&self, job: usize) -> Option<&WorkflowJob> {
        self.body.jobs.as_deref().and_then(|jobs| jobs.get(job))
    }

    /// The request for the logs on their way, if any.
    pub(super) fn loading(&self) -> Option<RequestId> {
        match self.body.logs {
            JobLogsView::Loading { request, .. } => Some(request),
            _ => None,
        }
    }

    /// The job whose logs are loading, shown or missing.
    pub(super) fn logs_job(&self) -> Option<usize> {
        match self.body.logs {
            JobLogsView::Hint => None,
            JobLogsView::Loading { job, .. } | JobLogsView::Shown { job, .. } | JobLogsView::Missing { job } => {
                Some(job)
            }
        }
    }

    /// Job `job`'s logs were requested with `request`: `Loading logs...` until they come.
    pub(super) fn set_loading(&mut self, job: usize, request: RequestId) {
        self.set_logs(JobLogsView::Loading { job, request });
    }

    /// Job `job`'s logs, `None` when it has none or they failed to load.
    pub(super) fn set_logs_text(&mut self, job: usize, text: Option<&str>) {
        self.set_logs(match text {
            Some(text) => JobLogsView::Shown {
                job,
                logs: Markdown::parse(&logs_markdown(text)),
            },
            None => JobLogsView::Missing { job },
        });
    }

    fn set_logs(&mut self, logs: JobLogsView) {
        self.body.logs = logs;
        self.body.clear();
    }

    /// The tree, for tests.
    #[cfg(test)]
    pub(in crate::screens::main) fn tree(&self) -> &Tree<JobRef> {
        &self.tree
    }

    /// The body's first visible line.
    #[cfg(test)]
    pub(in crate::screens::main) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Whether the tree has focus, rather than the body.
    pub(super) fn tree_focused(&self) -> bool {
        self.focus == Stop::Tree
    }

    /// A tree binding, with the tree focused.
    pub(super) fn tree_binding(&mut self, id: BindingId) -> bool {
        let Some(action) = TreeAction::from_binding(id) else {
            return false;
        };
        // Python acts on no selection: `enter` only toggles.
        let _ = self.tree.handle(action);
        true
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    /// A release, move or wheel over the content: the tree's, when it's over the tree.
    pub(super) fn mouse(&mut self, mouse: MouseEvent) -> bool {
        let Some(view) = self.tree_view else {
            return false;
        };
        if !view.visible.contains(Position::new(mouse.column, mouse.row)) {
            return false;
        }
        // In the tree's own coordinates, as if it were drawn whole at the origin.
        let local = MouseEvent {
            column: mouse.column - view.visible.x,
            row: mouse.row - view.visible.y + view.hidden_above,
            ..mouse
        };
        // A click only toggles a job: Python acts on no selection.
        let _ = self.tree.handle_mouse(&local, view.size);
        true
    }

    /// Moves focus to the next stop `forward` or back, and says whether there was one.
    pub(super) fn focus_step(&mut self, forward: bool) -> bool {
        self.focus = match (self.focus, forward) {
            (Stop::Body, true) => Stop::Tree,
            (Stop::Tree, false) => Stop::Body,
            _ => return false,
        };
        self.reveal_focused();
        true
    }

    pub(super) fn focus_first(&mut self) {
        self.focus = Stop::Body;
    }

    pub(super) fn focus_last(&mut self) {
        self.focus = Stop::Tree;
        self.reveal_focused();
    }

    /// A left press at `position`, inside the content: it focuses the tree when it's on it, else the body.
    pub(super) fn click(&mut self, position: Position) {
        let on_tree = self.tree_view.is_some_and(|view| view.visible.contains(position));
        self.focus = if on_tree { Stop::Tree } else { Stop::Body };
    }

    /// Scrolls the tree into view when it takes focus, as Textual does for a widget that takes focus.
    fn reveal_focused(&mut self) {
        if self.focus == Stop::Tree
            && let Some(rows) = self.tree_rows.clone()
        {
            self.scroll.reveal(rows);
        }
    }

    /// Draws the body into `area`, and the tree over the rows it leaves for it. `focused` says whether focus is
    /// inside the content.
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        if self.body.height != area.height {
            self.body.height = area.height;
            self.body.layouts.clear();
        }
        // The width the body gets: as `ScrollView` decides it, less the scrollbar when it overflows.
        let window = usize::from(area.height);
        let width = if self.body.layout(area.width, theme).lines.len() > window {
            area.width.saturating_sub(SCROLLBAR_WIDTH)
        } else {
            area.width
        };
        self.scroll.render(frame, area, theme, &mut self.body);
        let rows = self.body.tree_rows(width, theme);
        self.tree_rows = Some(rows.clone());
        self.tree_view = None;
        let height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
        if width == 0 || height == 0 {
            return;
        }

        let size = Rect::new(0, 0, width, height);
        let mut scratch = Buffer::empty(size);
        self.tree
            .render_to(&mut scratch, size, theme, focused && self.focus == Stop::Tree);
        let offset = self.scroll.offset();
        let mut visible: Option<(Rect, u16)> = None;
        for (dy, content_row) in (0..height).zip(rows) {
            let Some(row) = content_row.checked_sub(offset).filter(|&row| row < window) else {
                continue;
            };
            // `row` is below the window's height, a u16.
            let y = area.y + u16::try_from(row).unwrap_or(u16::MAX);
            for dx in 0..width {
                let cell = scratch[(dx, dy)].clone();
                frame.buffer_mut()[(area.x + dx, y)] = cell;
            }
            let line = Rect::new(area.x, y, width, 1);
            visible = Some(match visible {
                Some((drawn, hidden)) => (drawn.union(line), hidden),
                None => (line, dy),
            });
        }
        self.tree_view = visible.map(|(visible, hidden_above)| TreeView {
            visible,
            size,
            hidden_above,
        });
    }
}

impl Body {
    /// Forgets every layout, for when the jobs or the logs change.
    fn clear(&mut self) {
        self.below.clear();
        self.layouts.clear();
    }

    fn heading(&self, width: u16, theme: &Theme) -> Rendered {
        let HeaderStyles { text, .. } = HeaderStyles::new(theme);
        let bold = text.add_modifier(Modifier::BOLD);
        let mut out = Rendered::default();
        push_label(
            &mut out,
            &[TextRun::new(format!("Jobs for Run #{}", self.run_number), bold)],
            width,
        );
        out
    }

    /// The content rows the tree takes at `width`: the rows left once the heading and everything below the tree are
    /// laid out, and at least [`TREE_MIN_ROWS`].
    fn tree_rows(&mut self, width: u16, theme: &Theme) -> Range<usize> {
        if width == 0 {
            return 0..0;
        }
        let heading = self.heading(width, theme).lines.len();
        let below = self.below(width, theme).lines.len();
        let left = usize::from(self.height).saturating_sub(heading + below);
        heading..heading + left.max(TREE_MIN_ROWS)
    }

    /// The rule, the logs' header and the logs (or what stands for them), laid out `width` cells wide.
    fn below(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let Body {
            jobs,
            hint,
            logs,
            below,
            ..
        } = self;
        below.get(width, theme, || below_tree(jobs.as_deref(), hint, logs, width, theme))
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let rows = self.tree_rows(width, theme);
        let heading = self.heading(width, theme);
        let Body {
            jobs,
            hint,
            logs,
            below,
            layouts,
            ..
        } = self;
        let below = below.get(width, theme, || below_tree(jobs.as_deref(), hint, logs, width, theme));
        layouts.get(width, theme, || {
            let mut out = heading;
            out.lines.extend(std::iter::repeat_with(Line::default).take(rows.len()));
            append(&mut out, below.clone());
            out
        })
    }
}

/// The rule, the logs' header and the logs (or what stands for them), laid out `width` cells wide.
fn below_tree(
    jobs: Option<&[WorkflowJob]>,
    hint: &Markdown,
    logs: &JobLogsView,
    width: u16,
    theme: &Theme,
) -> Rendered {
    let mut out = Rendered::default();
    if width == 0 {
        return out;
    }
    let HeaderStyles { text, .. } = HeaderStyles::new(theme);
    let bold = text.add_modifier(Modifier::BOLD);
    push_rule(&mut out, width, theme);
    let header = match logs {
        JobLogsView::Hint => "Job Logs".to_owned(),
        JobLogsView::Loading { job, .. } | JobLogsView::Shown { job, .. } | JobLogsView::Missing { job } => {
            let name = jobs.and_then(|jobs| jobs.get(*job)).map_or("", |job| job.name.as_str());
            format!("Logs for Job: {name}")
        }
    };
    push_label(&mut out, &[TextRun::new(header, bold)], width);
    let rendered = match logs {
        JobLogsView::Hint => hint.layout(width, theme),
        JobLogsView::Loading { .. } => Markdown::parse("Loading logs...").layout(width, theme),
        JobLogsView::Shown { logs, .. } => logs.layout(width, theme),
        JobLogsView::Missing { .. } => {
            Markdown::parse("No logs available or failed to fetch logs.").layout(width, theme)
        }
    };
    append(&mut out, rendered);
    out
}

/// The tree of `jobs`: a collapsed node per job and a leaf per step, `<icon> <name> (<status>)` with the raw status,
/// or the one leaf `No jobs found`.
fn job_tree(jobs: &[WorkflowJob], theme: &Theme) -> Tree<JobRef> {
    let mut tree = Tree::new();
    if jobs.is_empty() {
        tree.add(None, Line::from("No jobs found"), None, false);
        return tree;
    }
    for (index, job) in jobs.iter().enumerate() {
        let node = tree.add(
            None,
            label(&job.name, &job.status, job.conclusion.as_ref(), theme),
            Some(index),
            true,
        );
        for step in &job.steps {
            tree.add(
                Some(node),
                label(&step.name, &step.status, step.conclusion.as_ref(), theme),
                Some(index),
                false,
            );
        }
    }
    tree
}

/// `<icon> <name> (<status>)`, the icon tinted. A queued job or step shows the Overview's `⋯` (Py bug B63).
fn label(name: &str, status: &JobStatus, conclusion: Option<&JobConclusion>, theme: &Theme) -> Line<'static> {
    let (icon, tint) = job_icon(status, conclusion);
    Line::from(vec![
        Span::styled(icon, tree_tint(tint).style(theme)),
        Span::raw(format!(" {name} ({})", status.as_str())),
    ])
}

/// The Overview's tint as the tree shows it. Python's tree labels are Rich markup (Textual's `Tree.process_label`),
/// not Textual's, so `red`, `yellow` and `cyan` are the ANSI colors through the terminal theme. Rich has no
/// `greenyellow`, which leaves Python's `✓` uncolored; Rust keeps it greenyellow (Py bug B120).
fn tree_tint(tint: Tint) -> Tint {
    match tint {
        Tint::Accent(Accent::Red) => Tint::RichAnsi(RichAnsi::Red),
        Tint::Accent(Accent::Yellow) => Tint::RichAnsi(RichAnsi::Yellow),
        Tint::Accent(Accent::Cyan) => Tint::RichAnsi(RichAnsi::Cyan),
        other => other,
    }
}

/// Logs as the markdown that shows them: one fenced block, verbatim, cut after [`MAX_LOG_CHARS`] characters with a
/// note after a blank line.
pub(super) fn logs_markdown(text: &str) -> String {
    let shown = match text.char_indices().nth(MAX_LOG_CHARS) {
        Some((cut, _)) => format!("{}\n\n... (logs truncated, too long to display)", &text[..cut]),
        None => text.to_owned(),
    };
    format!("```\n{shown}\n```")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::main::run_details::tests::fixture_jobs;
    use crate::theme::{ColorDepth, ColorMode};

    /// The tree's lines, drawn 60 cells wide.
    fn lines(tree: &mut Tree<JobRef>) -> Vec<String> {
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        tree.render_to(&mut buf, area, &Theme::default(), false);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn cursor_job(tree: &Tree<JobRef>) -> Option<JobRef> {
        tree.cursor_node().map(|(_, &job)| job)
    }

    #[test]
    fn run_42s_tree_has_a_collapsed_node_per_job_and_a_leaf_per_step() {
        let jobs = fixture_jobs(5_000_042);
        let mut tree = job_tree(&jobs, &Theme::default());
        assert_eq!(
            lines(&mut tree),
            [
                "▶ ✓ build (completed)",
                "▶ ✗ test (completed)",
                "▶ − lint (completed)",
                "▶ ○ deploy (completed)",
            ]
        );
        // Every node expanded: the steps are leaves under their job, and carry its index.
        let _ = tree.handle(TreeAction::CursorDown);
        let _ = tree.handle(TreeAction::ToggleExpandAll);
        let expanded = lines(&mut tree);
        assert_eq!(
            expanded[..5],
            [
                "▼ ✓ build (completed)",
                "├── ✓ Set up job (completed)",
                "├── ✓ Run cargo build (completed)",
                "└── ✓ Complete job (completed)",
                "▼ ✗ test (completed)",
            ]
        );
        assert!(
            expanded.contains(&"└── − Upload results (completed)".to_owned()),
            "{expanded:#?}"
        );
        let _ = tree.handle(TreeAction::CursorDown);
        assert_eq!(cursor_job(&tree), Some(Some(0)));
        for _ in 0..3 {
            let _ = tree.handle(TreeAction::CursorDown);
        }
        assert_eq!(cursor_job(&tree), Some(Some(1)));
    }

    #[test]
    fn no_jobs_is_one_leaf() {
        let mut tree = job_tree(&[], &Theme::default());
        assert_eq!(lines(&mut tree), ["No jobs found"]);
        let _ = tree.handle(TreeAction::CursorDown);
        assert_eq!(cursor_job(&tree), Some(None));
    }

    #[test]
    fn a_queued_job_shows_the_overviews_icon_and_its_raw_status() {
        let theme = Theme::default();
        let jobs = fixture_jobs(5_000_041);
        let mut tree = job_tree(&jobs, &theme);
        assert_eq!(lines(&mut tree), ["▶ ⋯ build (in_progress)", "▶ ⋯ test (queued)"]);
        // Py bug B63: a yellow `⋯`, not a dim `?`.
        let queued = label("test", &JobStatus::Queued, None, &theme);
        assert_eq!(queued.spans[0].content, "⋯");
        assert_eq!(queued.spans[0].style, Tint::RichAnsi(RichAnsi::Yellow).style(&theme));
        let running = label("build", &JobStatus::InProgress, None, &theme);
        assert_eq!(running.spans[0].style, Tint::RichAnsi(RichAnsi::Cyan).style(&theme));
        let pending = label("step", &JobStatus::Pending, None, &theme);
        assert_eq!(pending.to_string(), "? step (pending)");
        assert_eq!(pending.spans[0].style, Tint::Dim.style(&theme));
    }

    #[test]
    fn icons_are_rich_ansi_colors_except_the_greenyellow_check() {
        let icon_fg = |theme: &Theme, status: JobStatus, conclusion: Option<JobConclusion>| {
            label("job", &status, conclusion.as_ref(), theme).spans[0].style.fg
        };
        let dark = Theme::default();
        let light = Theme::by_name("textual-light").unwrap();
        let rgb = |r, g, b| Some(ratatui::style::Color::Rgb(r, g, b));
        let completed = |c| (JobStatus::Completed, Some(c));
        let rows = [
            // Py bug B120: Python's `✓` takes the row's color; Rust keeps the Overview's greenyellow.
            (
                completed(JobConclusion::Success),
                rgb(0xAD, 0xFF, 0x2F),
                rgb(0xAD, 0xFF, 0x2F),
            ),
            (
                completed(JobConclusion::Failure),
                rgb(0xF4, 0x00, 0x5F),
                rgb(0xAA, 0x37, 0x31),
            ),
            (
                completed(JobConclusion::Cancelled),
                rgb(0xFD, 0x97, 0x1F),
                rgb(0xCB, 0x90, 0x00),
            ),
            (
                (JobStatus::InProgress, None),
                rgb(0x58, 0xD1, 0xEB),
                rgb(0x00, 0x83, 0xB2),
            ),
            ((JobStatus::Queued, None), rgb(0xFD, 0x97, 0x1F), rgb(0xCB, 0x90, 0x00)),
        ];
        for ((status, conclusion), on_dark, on_light) in rows {
            assert_eq!(
                icon_fg(&dark, status.clone(), conclusion.clone()),
                on_dark,
                "{status:?} {conclusion:?}"
            );
            assert_eq!(
                icon_fg(&light, status.clone(), conclusion.clone()),
                on_light,
                "{status:?} {conclusion:?}"
            );
        }
        // The ANSI themes leave them to the terminal's palette.
        let ansi = Theme::by_name("ansi-dark").unwrap();
        let failed = icon_fg(&ansi, JobStatus::Completed, Some(JobConclusion::Failure));
        assert_eq!(failed, Some(ratatui::style::Color::Red));
    }

    const TRUNCATED: &str = "\n\n... (logs truncated, too long to display)";

    #[test]
    fn logs_are_fenced_and_cut_at_50000_characters() {
        let fenced = |body: &str| format!("```\n{body}\n```");
        let under = "a".repeat(49_999);
        assert_eq!(logs_markdown(&under), fenced(&under));
        let at = "a".repeat(50_000);
        assert_eq!(logs_markdown(&at), fenced(&at));
        let over = "a".repeat(50_001);
        assert_eq!(logs_markdown(&over), fenced(&format!("{at}{TRUNCATED}")));
        // Verbatim: blank lines and header-shaped lines kept.
        assert_eq!(logs_markdown("a\n\nWord: value\n"), "```\na\n\nWord: value\n\n```");
    }

    #[test]
    fn the_cut_counts_characters_not_bytes() {
        // 49,999 `a`s then `é` (two bytes): the 50,000th character is kept whole.
        let text = format!("{}é tail", "a".repeat(49_999));
        let expected = format!("{}é{TRUNCATED}", "a".repeat(49_999));
        assert_eq!(logs_markdown(&text), format!("```\n{expected}\n```"));
        // 50,000 two-byte characters fit, uncut.
        let wide = "é".repeat(50_000);
        assert_eq!(logs_markdown(&wide), format!("```\n{wide}\n```"));
    }

    #[test]
    fn a_theme_switch_recolors_the_labels_and_keeps_the_cursor_and_expansion() {
        let keymap = Keymap::new(&indexmap::IndexMap::new());
        let light = Theme::by_name("textual-light")
            .expect("a built-in theme")
            .with_color_mode(ColorMode {
                depth: ColorDepth::EightBit,
                no_color: false,
            });
        let draw = |tab: &mut JobsTab| {
            let area = Rect::new(0, 0, 60, 20);
            let mut buf = Buffer::empty(area);
            tab.tree.render_to(&mut buf, area, &light, false);
            buf
        };
        let jobs: Arc<[WorkflowJob]> = fixture_jobs(5_000_042).into();
        let mut built_light = JobsTab::new(42, &keymap);
        built_light.set_jobs(Arc::clone(&jobs), &light);
        let mut switched = JobsTab::new(42, &keymap);
        switched.set_jobs(jobs, &Theme::default());
        for tab in [&mut built_light, &mut switched] {
            let _ = tab.tree.handle(TreeAction::CursorDown);
            let _ = tab.tree.handle(TreeAction::ToggleExpandAll);
        }
        // The icons' accent colors are reduced to the terminal's colors, which the theme carries.
        assert_ne!(draw(&mut built_light), draw(&mut switched));
        switched.retheme(&light);
        assert_eq!(draw(&mut built_light), draw(&mut switched));
        assert_eq!(switched.cursor_job(), Some(0));
    }
}
