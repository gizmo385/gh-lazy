//! The review part of the diff viewer, below the files: the pending comments (Python's "Pending Comments:" header
//! and its `CommentPreview`s), which Add Comment fills, then the Submit review form (`SubmitReview`), which sends them
//! with the review.

use std::ops::Range;

use lazy_github_core::diff::{DiffLine, Hunk, LineKind};
use lazy_github_core::github::{DraftReviewComment, NewReview, ReviewEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget};

use super::buffer_line;
use crate::components::button::{self, Button, ButtonVariant};
use crate::components::markdown::{self, Rendered, TextRun};
use crate::components::select::{self, DEFAULT_PROMPT, Select};
use crate::components::text_input::{self, TextInput};
use crate::theme::Theme;

/// The header's text.
const HEADER: &str = "Pending Comments:";

/// The header's `height: 3`.
const HEADER_HEIGHT: usize = 3;

/// Columns a box takes each side of what it holds: its `solid` border and its `padding: 1`.
const BOX_SIDE: u16 = 2;

/// The form's `margin: 2 0`.
const FORM_MARGIN: usize = 2;

/// The status select's options, in Python's order (`ReviewState` without the disallowed states).
const STATUS_OPTIONS: [(&str, ReviewEvent); 3] = [
    ("Approved", ReviewEvent::Approve),
    ("Changes Requested", ReviewEvent::RequestChanges),
    ("Commented", ReviewEvent::Comment),
];

/// A line of the diff, by index: the file, the hunk in it, and the line in the hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffLineRef {
    /// The file's index in the diff.
    pub file: usize,
    /// The hunk's index in the file.
    pub hunk: usize,
    /// The line's index in the hunk.
    pub line: usize,
}

/// A comment added with Add Comment, kept until the review is submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingComment {
    /// The file's path, as the diff names it.
    pub path: String,
    /// The line commented on.
    pub line: DiffLineRef,
    /// The line's diff position, which the review sends.
    pub position: u32,
    /// The line's number in the file: the new side's, or the old side's for a removed line (Py bug B22).
    pub line_number: u32,
    /// The line as the diff has it, its marker first, trimmed.
    pub line_text: String,
    /// The comment, as typed.
    pub body: String,
}

impl PendingComment {
    /// The comment on line `line` of `hunk`, a hunk of the file at `path`, with an empty body. `None` when the hunk has
    /// no such line.
    pub fn on(path: &str, hunk: &Hunk, line: DiffLineRef) -> Option<Self> {
        let diff_line = hunk.lines.get(line.line)?;
        Some(Self {
            path: path.to_owned(),
            line,
            position: diff_line.position,
            line_number: line_number(&hunk.lines[..=line.line]),
            line_text: line_text(diff_line),
            body: String::new(),
        })
    }

    /// The preview's first line: `<path>:<line> - <line text>`.
    fn heading(&self) -> String {
        format!("{}:{} - {}", self.path, self.line_number, self.line_text)
    }
}

/// A line as Python's pane holds it, its marker first, with whitespace trimmed from both ends: `+Run ...`, `-old`,
/// `\ No newline at end of file`, or a context line without its leading space.
pub fn line_text(line: &DiffLine) -> String {
    // The parser keeps the "No newline" marker's message without the `\ ` before it.
    let marker = match line.kind {
        LineKind::Context => " ",
        LineKind::Added => "+",
        LineKind::Removed => "-",
        LineKind::NoNewline => "\\ ",
    };
    format!("{marker}{}", line.text).trim().to_owned()
}

/// The file line number of the last of `lines`: the new side's, the old side's for a removed line, and for the "No
/// newline" marker, which has neither, the number of the line it follows. 0 when there's none.
fn line_number(lines: &[DiffLine]) -> u32 {
    lines
        .iter()
        .rev()
        .find_map(|line| match line.kind {
            LineKind::Removed => line.old,
            LineKind::Context | LineKind::Added => line.new,
            LineKind::NoNewline => None,
        })
        .unwrap_or(0)
}

/// The Submit review form: the status select (unless the viewer wrote the pull request, who can only comment), the
/// summary input and the submit button.
#[derive(Debug)]
pub(super) struct ReviewForm {
    /// `None` on the author's form.
    pub(super) status: Option<Select<ReviewEvent>>,
    pub(super) summary: TextInput,
    pub(super) submit: Button,
}

/// The review status is the blank option: Python's "Please select a status for the new review!".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingStatus;

impl ReviewForm {
    /// The reviewer's form ("Submit Review", the status at "Commented"), or with `author` the author's ("Add
    /// Comments", no status).
    pub(super) fn new(author: bool) -> Self {
        let (status, label) = if author {
            (None, "Add Comments")
        } else {
            let select = Select::new(DEFAULT_PROMPT, STATUS_OPTIONS, Some(&ReviewEvent::Comment));
            (Some(select), "Submit Review")
        };
        Self {
            status,
            summary: TextInput::new("Review summary"),
            submit: Button::new(label, ButtonVariant::Success),
        }
    }

    /// The review to send with `comments`, for the commit `commit_id`. The author's form always comments.
    pub(super) fn review(&self, commit_id: &str, comments: &[PendingComment]) -> Result<NewReview, MissingStatus> {
        let event = match &self.status {
            Some(select) => *select.value().ok_or(MissingStatus)?,
            None => ReviewEvent::Comment,
        };
        Ok(NewReview {
            event,
            commit_id: commit_id.to_owned(),
            body: self.summary.value().to_owned(),
            comments: comments
                .iter()
                .map(|comment| DraftReviewComment {
                    path: comment.path.clone(),
                    body: comment.body.clone(),
                    position: comment.position,
                })
                .collect(),
        })
    }
}

/// Which part of the form has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormFocus {
    Status,
    Summary,
    Submit,
}

/// Where the form's fields landed in the body.
#[derive(Debug, Clone, Default)]
pub(super) struct FormPlaced {
    /// The status box's rows; `None` on the author's form.
    pub(super) status: Option<Range<usize>>,
    pub(super) summary: Range<usize>,
    pub(super) submit: ButtonPlaced,
}

/// Adds `form` to `out`, `width` cells wide, with `focused` drawn focused, and returns where its fields landed:
/// its top margin, "Review Status:" and the select, the summary, the button with a row of margin above and below,
/// then its bottom margin.
pub(super) fn layout_form(
    out: &mut Rendered,
    form: &mut ReviewForm,
    focused: Option<FormFocus>,
    width: u16,
    theme: &Theme,
) -> FormPlaced {
    let under = theme.background();
    let base = Style::new()
        .fg(theme.color(theme.palette.foreground, under))
        .bg(theme.paint(under));
    let status_rows = if form.status.is_some() {
        1 + usize::from(select::HEIGHT)
    } else {
        0
    };
    let input = usize::from(text_input::HEIGHT);
    let button_rows = usize::from(button::HEIGHT);
    let height = FORM_MARGIN + status_rows + input + 1 + button_rows + 1 + FORM_MARGIN;
    let area = Rect::new(0, 0, width, u16::try_from(height).unwrap_or(u16::MAX));
    let mut buf = Buffer::empty(area);
    buf.set_style(area, base);
    let row = |r: usize| u16::try_from(r).unwrap_or(u16::MAX);
    let start = out.lines.len();
    let mut y = FORM_MARGIN;
    let mut placed = FormPlaced::default();
    if let Some(status) = &form.status {
        buf.set_stringn(0, row(y), "Review Status:", usize::from(width), base);
        y += 1;
        let focused = focused == Some(FormFocus::Status) && !status.is_open();
        status.render_box(
            &mut buf,
            Rect::new(0, row(y), width, select::HEIGHT),
            theme,
            under,
            focused,
        );
        placed.status = Some(start + y..start + y + usize::from(select::HEIGHT));
        y += usize::from(select::HEIGHT);
    }
    form.summary.render(
        &mut buf,
        Rect::new(0, row(y), width, text_input::HEIGHT),
        theme,
        under,
        focused == Some(FormFocus::Summary),
    );
    placed.summary = start + y..start + y + input;
    y += input + 1;
    // `margin: 1` on the button: a cell in from the left.
    let button_width = form.submit.width(theme).min(width.saturating_sub(1));
    form.submit.render(
        &mut buf,
        Rect::new(1, row(y), button_width, button::HEIGHT),
        theme,
        focused == Some(FormFocus::Submit),
    );
    placed.submit = ButtonPlaced {
        rows: start + y..start + y + button_rows,
        columns: 1..1 + button_width,
    };
    out.lines.extend((0..area.height).map(|y| buffer_line(&buf, y)));
    placed
}

/// Where a button landed in the body: its rows, and its columns from the view's left edge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ButtonPlaced {
    pub(super) rows: Range<usize>,
    pub(super) columns: Range<u16>,
}

impl ButtonPlaced {
    pub(super) fn contains(&self, row: usize, x: u16) -> bool {
        self.rows.contains(&row) && self.columns.contains(&x)
    }
}

/// Adds the Pending Comments section for `comments` to `out`, `width` cells wide, with the Remove button of comment
/// `focused` drawn focused, and returns where each Remove button landed. With no comments the section is hidden:
/// nothing is added.
pub(super) fn layout_pending(
    out: &mut Rendered,
    comments: &[PendingComment],
    focused: Option<usize>,
    width: u16,
    theme: &Theme,
) -> Vec<ButtonPlaced> {
    if comments.is_empty() || width == 0 {
        return Vec::new();
    }
    let p = theme.palette;
    let under = theme.background();
    let base = Style::new().fg(theme.color(p.foreground, under)).bg(theme.paint(under));
    let panel = p.panel.over(under);
    let muted = Style::new().fg(theme.color(p.text_muted, under));
    let body_text = Style::new().fg(theme.color(p.text, panel)).bg(theme.paint(panel));
    // Inside the container's border and padding, then each preview's.
    let inner = width.saturating_sub(2 * BOX_SIDE);
    let text_width = inner.saturating_sub(2 * BOX_SIDE);
    let previews: Vec<(Rendered, Rendered)> = comments
        .iter()
        .map(|comment| {
            let heading = markdown::wrap_text(&[TextRun::new(comment.heading(), muted)], text_width);
            let body = markdown::wrap_text(
                &[TextRun::new(comment.body.as_str(), body_text)],
                text_width.saturating_sub(2),
            );
            (heading, body)
        })
        .collect();
    // A preview: its border and padding rows, the heading, its margin, the body with its padding, its margin, the
    // button.
    let preview_height =
        |(heading, body): &(Rendered, Rendered)| 4 + heading.lines.len() + 1 + body.lines.len() + 2 + 1 + 3;
    // The container: its border and padding rows, then the previews, each with `margin: 1 0` collapsing between them.
    let container: usize = 4 + previews.iter().map(|p| preview_height(p) + 1).sum::<usize>() + 1;
    // The header's `margin: 2 0 1 0`.
    let top = 2 + HEADER_HEIGHT + 1;
    let height = u16::try_from(top + container).unwrap_or(u16::MAX);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, base);

    // The header: bold `$text` on `$accent`, centered on its first row.
    let accent = p.accent.over(under);
    let header = Rect::new(0, 2, width, 3);
    buf.set_style(header, Style::new().bg(theme.paint(accent)));
    let header_width = u16::try_from(HEADER.len()).unwrap_or(u16::MAX);
    buf.set_stringn(
        width.saturating_sub(header_width) / 2,
        header.y,
        HEADER,
        usize::from(width),
        Style::new()
            .fg(theme.color(p.text, accent))
            .bg(theme.paint(accent))
            .add_modifier(Modifier::BOLD),
    );

    let row = |r: usize| u16::try_from(r).unwrap_or(u16::MAX);
    let container_top = top;
    Block::bordered()
        .border_style(Style::new().fg(theme.color(p.accent, under)))
        .render(Rect::new(0, row(container_top), width, row(container)), &mut buf);
    let secondary = Style::new().fg(theme.color(p.secondary, under));
    let mut placed = Vec::with_capacity(comments.len());
    let mut y = container_top + 3;
    for (index, preview) in previews.iter().enumerate() {
        let (heading, body) = preview;
        let preview_rows = preview_height(preview);
        let boxed = Rect::new(BOX_SIDE, row(y), inner, row(preview_rows));
        Block::bordered().border_style(secondary).render(boxed, &mut buf);
        let x = 2 * BOX_SIDE;
        let mut line_y = y + 2;
        for line in &heading.lines {
            buf.set_line(x, row(line_y), line, text_width);
            line_y += 1;
        }
        line_y += 1;
        let body_rows = body.lines.len() + 2;
        buf.set_style(Rect::new(x, row(line_y), text_width, row(body_rows)), body_text);
        for (offset, line) in body.lines.iter().enumerate() {
            buf.set_line(x + 1, row(line_y + 1 + offset), line, text_width.saturating_sub(2));
        }
        line_y += body_rows + 1;
        let mut remove = Button::new("Remove", ButtonVariant::Warning);
        let button_width = remove.width(theme).min(text_width);
        remove.render(
            &mut buf,
            Rect::new(x, row(line_y), button_width, button::HEIGHT),
            theme,
            focused == Some(index),
        );
        placed.push(ButtonPlaced {
            rows: out.lines.len() + line_y..out.lines.len() + line_y + usize::from(button::HEIGHT),
            columns: x..x + button_width,
        });
        y += preview_rows + 1;
    }
    out.lines.extend((0..height).map(|y| buffer_line(&buf, y)));
    placed
}

#[cfg(test)]
mod tests {
    use lazy_github_core::diff::HunkRange;

    use super::*;

    fn line(kind: LineKind, text: &str, old: Option<u32>, new: Option<u32>, position: u32) -> DiffLine {
        DiffLine {
            kind,
            text: text.to_owned(),
            old,
            new,
            position,
        }
    }

    /// `@@ -3,3 +3,3 @@`: a context line, a removed one, an added one, then a "No newline" marker.
    fn hunk() -> Hunk {
        Hunk {
            header: "@@ -3,3 +3,3 @@".to_owned(),
            range: Some(HunkRange {
                old_start: 3,
                old_count: 3,
                new_start: 3,
                new_count: 3,
            }),
            context: String::new(),
            lines: vec![
                line(LineKind::Context, "  ## Usage  ", Some(3), Some(3), 1),
                line(LineKind::Removed, "Run it.", Some(4), None, 2),
                line(LineKind::Added, "Run it, then press j.", None, Some(4), 3),
                line(LineKind::NoNewline, "No newline at end of file", None, None, 4),
            ],
        }
    }

    fn at(line: usize) -> PendingComment {
        PendingComment::on("README.md", &hunk(), DiffLineRef { file: 0, hunk: 0, line }).unwrap()
    }

    #[test]
    fn the_line_text_keeps_its_marker_and_is_trimmed() {
        let texts: Vec<String> = (0..4).map(|i| at(i).line_text).collect();
        assert_eq!(
            texts,
            [
                "## Usage",
                "-Run it.",
                "+Run it, then press j.",
                "\\ No newline at end of file"
            ]
        );
    }

    #[test]
    fn the_line_number_is_the_new_side_s_or_the_old_side_s_for_a_removed_line() {
        // A context line, a removed line, an added line (B22: not their index in the hunk), and the marker, which
        // takes the number of the line it follows.
        let numbers: Vec<u32> = (0..4).map(|i| at(i).line_number).collect();
        assert_eq!(numbers, [3, 4, 4, 4]);
        assert_eq!(at(1).position, 2);
        assert_eq!(at(1).heading(), "README.md:4 - -Run it.");
    }

    #[test]
    fn a_line_past_the_hunk_has_no_comment() {
        assert_eq!(
            PendingComment::on(
                "README.md",
                &hunk(),
                DiffLineRef {
                    file: 0,
                    hunk: 0,
                    line: 4
                }
            ),
            None
        );
    }
}
