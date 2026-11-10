//! The review threads the body shows under their lines (#390, decision `diff-inline-comments`): one
//! [`ThreadView`] per thread of a file in the diff, collapsed to a summary row or expanded to its comments.
//!
//! A thread's rows start at its summary row (`▶`, or `▼` once expanded). Expanded, an outdated thread's next row is
//! the line it was written on, then every thread shows each comment: its author line, then its body as markdown,
//! replies indented two more cells, all behind a bar. Those comment rows depend on the width, so they're laid out on
//! demand and cached by width and theme.

use lazy_github_core::references;
use lazy_github_core::review_anchor::{DiffThread, OriginalLine};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::components::comment::author_line;
use crate::components::markdown::{self, LinkSpan, Markdown, Rendered, TextRun};
use crate::display;
use crate::theme::{Paint, Theme};

/// The bar in front of an expanded thread's comment rows, and the cell after it.
pub(super) const BAR: &str = "▎ ";
/// The cells [`BAR`] takes.
pub(super) const BAR_WIDTH: u16 = 2;
/// How many more cells a reply is indented than the thread's first comment.
const REPLY_INDENT: u16 = 2;
/// The narrowest a comment's body is laid out, so a thread measured before the body's first draw has a height.
pub(super) const MIN_BODY_WIDTH: u16 = 10;

/// Where a thread shows in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Right after the file's header: a comment on the whole file.
    File,
    /// Under this line: by hunk and line.
    Line { hunk: usize, line: usize },
    /// Under a deleted file's `file was removed` row, whose lines don't show.
    Removed,
    /// In the outdated group after the file's last hunk.
    Outdated,
}

/// One thread as the body shows it.
#[derive(Debug, Clone)]
pub struct ThreadView {
    pub thread: DiffThread,
    /// The file's index in the diff.
    pub file: usize,
    pub place: Place,
    /// The line an outdated thread was written on, when its hunk says.
    pub original: Option<OriginalLine>,
    pub expanded: bool,
    /// Each comment's author line and parsed body.
    authors: Vec<String>,
    bodies: Vec<Markdown>,
    /// The comment rows laid out at a width in a theme.
    layout: Option<(u16, Theme, Rendered)>,
}

impl ThreadView {
    /// `thread`, in file `file` at `place`, collapsed. Bodies' `#N` references point into `repo_full_name`.
    pub fn new(thread: DiffThread, file: usize, place: Place, original: Option<OriginalLine>, repo: &str) -> Self {
        let authors = thread
            .comments
            .iter()
            .map(|c| author_line(c.comment.user.as_ref(), c.comment.created_at))
            .collect();
        let bodies = thread
            .comments
            .iter()
            .map(|c| Markdown::parse(&references::expand(&c.comment.body, repo)))
            .collect();
        Self {
            thread,
            file,
            place,
            original: if place == Place::Outdated { original } else { None },
            expanded: false,
            authors,
            bodies,
            layout: None,
        }
    }

    /// The first comment's id, which the thread's expanded state is kept by.
    pub fn root_id(&self) -> u64 {
        self.thread.root().comment.id
    }

    /// The rows the thread takes with its comments laid out `width` cells wide (the code column's width).
    pub fn rows(&mut self, width: u16) -> usize {
        if !self.expanded {
            return 1;
        }
        let comments = match &self.layout {
            Some((w, _, rendered)) if *w == width => rendered.lines.len(),
            // Wrapping doesn't depend on the colors: the default theme measures as well as the one drawn with.
            _ => self.comment_rows(width, &Theme::default()).lines.len(),
        };
        1 + usize::from(self.original.is_some()) + comments
    }

    /// The comment rows laid out `width` cells wide in `theme`: per comment, its author line and its body, behind
    /// the bar. Link columns count from the code column.
    pub fn comment_rows(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let fresh = matches!(&self.layout, Some((w, t, _)) if *w == width && t == theme);
        if !fresh {
            let rendered = self.lay_out(width, theme);
            self.layout = Some((width, *theme, rendered));
        }
        match &self.layout {
            Some((_, _, rendered)) => rendered,
            // Set just above.
            None => unreachable!("the layout was just made"),
        }
    }

    fn lay_out(&self, width: u16, theme: &Theme) -> Rendered {
        let styles = ThreadStyles::new(theme);
        let mut out = Rendered::default();
        for (index, (author, body)) in self.authors.iter().zip(&self.bodies).enumerate() {
            let indent = if index == 0 { 0 } else { REPLY_INDENT };
            let left = BAR_WIDTH + indent;
            let room = width.saturating_sub(left).max(MIN_BODY_WIDTH);
            let mut inside = markdown::wrap_text(&[TextRun::new(author.as_str(), styles.muted)], room);
            let mut body = body.layout_unpadded_over(room, theme, styles.under);
            // A paragraph's bottom margin: the next author line, or the thread's end, separates enough.
            while body
                .lines
                .last()
                .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
            {
                body.lines.pop();
            }
            let shift = inside.lines.len();
            inside.lines.extend(body.lines);
            inside.links.extend(body.links.into_iter().map(|span| LinkSpan {
                row: span.row + shift,
                ..span
            }));
            let first = out.lines.len();
            for line in inside.lines {
                let mut spans = vec![Span::styled(BAR, styles.bar)];
                if indent > 0 {
                    spans.push(Span::styled(" ".repeat(usize::from(indent)), styles.fill));
                }
                spans.extend(line.spans.into_iter().map(|span| {
                    let style = styles.fill.patch(span.style);
                    Span::styled(span.content, style)
                }));
                out.lines.push(Line::from(spans));
            }
            out.links.extend(inside.links.into_iter().map(|span| LinkSpan {
                row: span.row + first,
                columns: span.columns.start + left..span.columns.end + left,
                target: span.target,
            }));
        }
        out
    }

    /// The summary row's text after the code column: `▶ <label><author>: <first line>  · N replies`, cut to `width`
    /// cells with the reply count kept.
    pub fn summary_spans(&self, width: usize, styles: &ThreadStyles, highlight: Option<Style>) -> Vec<Span<'static>> {
        let base = highlight.unwrap_or(styles.fill);
        let glyph = if self.expanded { "▼ " } else { "▶ " };
        let root = self.thread.root();
        let login = root.comment.user.as_ref().map_or("Unknown", |user| user.login.as_str());
        let label = match (self.place, &self.original) {
            (Place::File, _) => "File comment · ".to_owned(),
            (Place::Outdated, Some(original)) => format!("Line {} (outdated) · ", original.number),
            (Place::Outdated, None) => "Outdated · ".to_owned(),
            (Place::Line { .. } | Place::Removed, _) => String::new(),
        };
        let replies = match self.thread.replies() {
            0 => String::new(),
            1 => "  · 1 reply".to_owned(),
            n => format!("  · {n} replies"),
        };
        let first_line = root.comment.body.lines().map(str::trim).find(|line| !line.is_empty());
        let first_line = display::sanitize(first_line.unwrap_or("")).into_owned();
        let login = display::sanitize(login).into_owned();
        let head = format!("{glyph}{label}");
        let used = display::width(&head) + display::width(&login) + 2 + display::width(&replies);
        let first_room = width.saturating_sub(used);
        let mut spans = vec![
            Span::styled(head, base),
            Span::styled(login, base.add_modifier(Modifier::BOLD)),
            Span::styled(": ", base),
            Span::styled(display::truncate(&first_line, first_room).into_owned(), base),
        ];
        if !replies.is_empty() {
            spans.push(Span::styled(replies, base.patch(styles.muted_fg)));
        }
        display::cut_spans(spans, width).0
    }
}

/// The colors thread rows are drawn in.
#[derive(Debug, Clone, Copy)]
pub struct ThreadStyles {
    /// What thread rows are drawn on (`$panel`).
    pub under: Paint,
    /// Text on it.
    pub fill: Style,
    /// Author lines and the outdated group's separator (`$text-muted`).
    pub muted: Style,
    /// The muted color alone, to patch over another style.
    pub muted_fg: Style,
    /// The bar in front of comment rows (`$secondary`).
    pub bar: Style,
    /// The flag cell of a line with threads (`$accent`).
    pub flag: Style,
    /// The flag cell of an outdated thread's original line, and of a line with pending comments (`$warning`).
    pub flag_outdated: Style,
    /// The bar in front of a pending comment's rows (`$warning`).
    pub pending_bar: Style,
    /// A pending comment's `Pending comment` title: bold `$warning`.
    pub pending_title: Style,
}

impl ThreadStyles {
    pub fn new(theme: &Theme) -> Self {
        let roles = theme.roles();
        let base = roles.surface.pane.over(theme.background());
        let under = roles.diff.review_body.over(base);
        let fill = Style::new()
            .fg(theme.color(roles.text.normal, under))
            .bg(theme.paint(under));
        let muted_fg = Style::new().fg(theme.color(roles.text.muted, under));
        Self {
            under,
            fill,
            muted: fill.patch(muted_fg),
            muted_fg,
            bar: Style::new()
                .fg(theme.color(roles.diff.review_comment_border, under))
                .bg(theme.paint(under)),
            flag: Style::new()
                .fg(theme.color(roles.brand.accent, base))
                .bg(theme.paint(base)),
            flag_outdated: Style::new().fg(theme.color(roles.status.warning, base)),
            pending_bar: Style::new()
                .fg(theme.color(roles.status.warning, under))
                .bg(theme.paint(under)),
            pending_title: Style::new()
                .fg(theme.color(roles.status.warning, under))
                .bg(theme.paint(under))
                .add_modifier(Modifier::BOLD),
        }
    }
}
