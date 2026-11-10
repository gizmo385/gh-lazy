//! The review part of the diff viewer: the pending comments ([`PendingComment`]), which Add Comment fills and the body
//! shows under their lines (#391), and the Submit review form below the files (`SubmitReview`), which sends them with
//! the review, with a row above it counting them.

use std::ops::Range;

use lazy_github_core::diff::{DiffLine, Hunk, LineKind};
use lazy_github_core::github::{DraftReviewComment, NewReview, ReviewEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::buffer_line;
use crate::components::button::{self, Button, ButtonVariant};
use crate::components::markdown::Rendered;
use crate::components::select::{self, DEFAULT_PROMPT, Select};
use crate::components::text_input::{self, TextInput};
use crate::theme::Theme;

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

/// What the row above the form says while `pending` comments will be sent with the review: nothing without any.
fn pending_note(pending: usize) -> Option<String> {
    match pending {
        0 => None,
        1 => Some("1 pending comment will be sent with the review".to_owned()),
        n => Some(format!("{n} pending comments will be sent with the review")),
    }
}

/// Adds `form` to `out`, `width` cells wide, with `focused` drawn focused, and returns where its fields landed:
/// its top margin, with `pending` comments to send a muted row counting them inside it, then
/// "Review Status:" and the select, the summary, the button with a row of margin above and below, then its bottom
/// margin.
pub(super) fn layout_form(
    out: &mut Rendered,
    form: &mut ReviewForm,
    focused: Option<FormFocus>,
    pending: usize,
    width: u16,
    theme: &Theme,
) -> FormPlaced {
    let under = theme.background();
    let base = Style::new()
        .fg(theme.color(theme.roles().text.plain, under))
        .bg(theme.paint(under));
    let note = pending_note(pending);
    // The note takes the middle of three blank rows: the form's top margin with a row more.
    let note_rows = usize::from(note.is_some());
    let status_rows = if form.status.is_some() {
        1 + usize::from(select::HEIGHT)
    } else {
        0
    };
    let input = usize::from(text_input::HEIGHT);
    let button_rows = usize::from(button::HEIGHT);
    let height = note_rows + FORM_MARGIN + status_rows + input + 1 + button_rows + 1 + FORM_MARGIN;
    let area = Rect::new(0, 0, width, u16::try_from(height).unwrap_or(u16::MAX));
    let mut buf = Buffer::empty(area);
    buf.set_style(area, base);
    let row = |r: usize| u16::try_from(r).unwrap_or(u16::MAX);
    let start = out.lines.len();
    if let Some(note) = &note {
        let muted = Style::new().fg(theme.color(theme.roles().text.muted, under));
        buf.set_stringn(0, 1, note, usize::from(width), base.patch(muted));
    }
    let mut y = note_rows + FORM_MARGIN;
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
    }

    #[test]
    fn the_note_counts_the_pending_comments() {
        assert_eq!(pending_note(0), None);
        assert_eq!(
            pending_note(1).as_deref(),
            Some("1 pending comment will be sent with the review")
        );
        assert_eq!(
            pending_note(3).as_deref(),
            Some("3 pending comments will be sent with the review")
        );
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
