//! Git unified diffs, as GitHub serves them for a pull request, parsed into files, hunks and lines.
//!
//! [`parse`] reads the text of `GET /repos/{owner}/{repo}/pulls/{n}` with `Accept: application/vnd.github.diff`. It
//! keeps the real line numbers on both sides and each line's diff `position`, the number GitHub's review comments
//! address lines by. The labels the Diff tab shows ([`Hunk::label`], [`ChangedFile::label`]) are derived here too, so
//! they're testable without a terminal.

use std::fmt;
use std::iter::Peekable;
use std::str::Lines;

/// The path a diff uses for the missing side of an added or deleted file.
pub const DEV_NULL: &str = "/dev/null";

/// The first line of each file's section.
const SECTION_START: &str = "diff --git ";

/// A parsed diff: the changed files, in diff order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// The changed files, in the order the diff lists them.
    pub files: Vec<ChangedFile>,
}

/// One file's section of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// The file's path: the new side's, or the old side's for a deleted file. Without the `a/` or `b/` prefix.
    pub path: String,
    /// The old side's path, without the `a/` prefix. [`DEV_NULL`] for an added file.
    pub old_path: String,
    /// Whether the change deletes the file.
    pub deleted: bool,
    /// The file's hunks, in order. Empty for a binary file, a pure rename or a mode change.
    pub hunks: Vec<Hunk>,
}

/// One `@@` hunk: a run of changed lines with their context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ ... @@` header line, verbatim.
    pub header: String,
    /// The line ranges the header gives, or `None` when it doesn't parse. The lines then have no line numbers.
    pub range: Option<HunkRange>,
    /// The text after the header's closing `@@`, trimmed (often the enclosing function). Empty when there's none.
    pub context: String,
    /// The hunk's lines, in order.
    pub lines: Vec<DiffLine>,
}

/// The ranges of a hunk header `@@ -<old_start>[,<old_count>] +<new_start>[,<new_count>] @@`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    /// The first old-side line number.
    pub old_start: u32,
    /// How many old-side lines the hunk covers; 1 when the header leaves it out.
    pub old_count: u32,
    /// The first new-side line number.
    pub new_start: u32,
    /// How many new-side lines the hunk covers; 1 when the header leaves it out.
    pub new_count: u32,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// What the line is.
    pub kind: LineKind,
    /// The line without its ` `, `+`, `-` or `\` marker. For [`LineKind::NoNewline`], the marker's message (`No
    /// newline at end of file`).
    pub text: String,
    /// The old-side line number: set for context and removed lines.
    pub old: Option<u32>,
    /// The new-side line number: set for context and added lines.
    pub new: Option<u32>,
    /// The diff position GitHub's review comments use: 1 is the line after the file's first hunk header, and the
    /// count runs on through later hunk headers.
    pub position: u32,
}

/// The kind of a [`DiffLine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// Unchanged (` `). An empty line, whose marker space was stripped somewhere along the way, counts as one.
    Context,
    /// Added (`+`).
    Added,
    /// Removed (`-`).
    Removed,
    /// The `\ No newline at end of file` marker, which belongs to the line before it.
    NoNewline,
}

/// How many lines a hunk or file adds and removes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Counts {
    /// Added lines.
    pub additions: u32,
    /// Removed lines.
    pub deletions: u32,
}

impl Counts {
    /// Whether nothing is added or removed.
    pub fn is_empty(self) -> bool {
        self.additions == 0 && self.deletions == 0
    }
}

impl std::ops::Add for Counts {
    type Output = Counts;

    fn add(self, other: Counts) -> Counts {
        Counts {
            additions: self.additions + other.additions,
            deletions: self.deletions + other.deletions,
        }
    }
}

impl fmt::Display for Counts {
    /// ` (+A -D)`, leaving out a zero count, or nothing when both are zero. The leading space separates it from the
    /// text it follows in a [`Label`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.additions, self.deletions) {
            (0, 0) => Ok(()),
            (added, 0) => write!(f, " (+{added})"),
            (0, removed) => write!(f, " (-{removed})"),
            (added, removed) => write!(f, " (+{added} -{removed})"),
        }
    }
}

/// A file or hunk title in parts, so the counts can be styled apart from the text: `text`, then `counts` as
/// [`Counts`]' `Display` writes them, then `suffix`. `Display` writes the whole title plainly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// What comes before the counts: `Lines 3-7`, `📄 viewer.rs`.
    pub text: String,
    /// The counts to show; zero counts are left out.
    pub counts: Counts,
    /// What comes after the counts: ` • <context>` for a hunk with context, otherwise empty.
    pub suffix: String,
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", self.text, self.counts, self.suffix)
    }
}

impl Hunk {
    /// The lines added and removed.
    pub fn counts(&self) -> Counts {
        self.lines
            .iter()
            .fold(Counts::default(), |counts, line| match line.kind {
                LineKind::Added => Counts {
                    additions: counts.additions + 1,
                    ..counts
                },
                LineKind::Removed => Counts {
                    deletions: counts.deletions + 1,
                    ..counts
                },
                LineKind::Context | LineKind::NoNewline => counts,
            })
    }

    /// The hunk's title: `Line <c>` when the new side is one line long, else `Lines <c>-<c+d-1>`, then the counts,
    /// then ` • <context>`. A header that doesn't parse is the title as is, without counts.
    pub fn label(&self) -> Label {
        let Some(range) = self.range else {
            return Label {
                text: self.header.clone(),
                counts: Counts::default(),
                suffix: String::new(),
            };
        };
        let text = if range.new_count == 1 {
            format!("Line {}", range.new_start)
        } else {
            // Signed, as Python computes it: a deleted file's `+0,0` range reads `Lines 0--1`.
            let end = i64::from(range.new_start) + i64::from(range.new_count) - 1;
            format!("Lines {}-{end}", range.new_start)
        };
        let suffix = if self.context.is_empty() {
            String::new()
        } else {
            format!(" • {}", self.context)
        };
        Label {
            text,
            counts: self.counts(),
            suffix,
        }
    }
}

impl ChangedFile {
    /// The lines added and removed across every hunk.
    pub fn counts(&self) -> Counts {
        self.hunks
            .iter()
            .map(Hunk::counts)
            .fold(Counts::default(), |a, b| a + b)
    }

    /// The last component of [`Self::path`].
    pub fn basename(&self) -> &str {
        // Diff paths always use `/`, whatever the platform.
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The file's title: `📄 <basename>` and the counts, or `🗑️  <basename> (deleted)` without counts.
    pub fn label(&self) -> Label {
        if self.deleted {
            Label {
                text: format!("🗑️  {} (deleted)", self.basename()),
                counts: Counts::default(),
                suffix: String::new(),
            }
        } else {
            Label {
                text: format!("📄 {}", self.basename()),
                counts: self.counts(),
                suffix: String::new(),
            }
        }
    }
}

/// Why a diff couldn't be parsed. `Display` is the reason, shown after `error parsing diff - please view on github: `.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiffError {
    /// A file's header stopped before its first hunk: the diff ended, or the next file's section began.
    #[error("unexpected end of the file header of `{section}`")]
    UnexpectedEnd {
        /// The section's `diff --git` line.
        section: String,
    },
    /// A file's header has a line that doesn't belong there, such as a `---` line not followed by `+++`.
    #[error("unexpected line {line} in the file header of `{section}`: `{text}`")]
    UnexpectedLine {
        /// The section's `diff --git` line.
        section: String,
        /// The line's number in the diff, from 1.
        line: usize,
        /// The line.
        text: String,
    },
}

type DiffLines<'a> = Peekable<std::iter::Enumerate<Lines<'a>>>;

/// Parses a git unified diff, in one pass. An empty diff has no files.
///
/// Each `diff --git` section becomes a [`ChangedFile`]. A section without `---` / `+++` lines (a binary file, a pure
/// rename, a mode change) is a file without hunks. Anything before the first section is ignored. LF and CRLF line
/// endings both work.
pub fn parse(diff: &str) -> Result<Diff, DiffError> {
    let mut lines: DiffLines<'_> = diff.lines().enumerate().peekable();
    while lines.next_if(|(_, line)| !line.starts_with(SECTION_START)).is_some() {}
    let mut files = Vec::new();
    while let Some((_, section)) = lines.next() {
        files.push(parse_section(section, &mut lines)?);
    }
    Ok(Diff { files })
}

/// One file's section, after its `diff --git` line, up to the next section or the end.
fn parse_section(section: &str, lines: &mut DiffLines<'_>) -> Result<ChangedFile, DiffError> {
    let unexpected_line = |(index, text): (usize, &str)| DiffError::UnexpectedLine {
        section: section.to_owned(),
        line: index + 1,
        text: text.to_owned(),
    };
    let unexpected_end = || DiffError::UnexpectedEnd {
        section: section.to_owned(),
    };

    // The extended headers (`index`, `new file mode`, `rename from`, ...), up to `---` or the section's end.
    let (mut old_path, mut path) = git_line_paths(&section[SECTION_START.len()..]).unwrap_or_default();
    let mut deleted = false;
    loop {
        match lines.peek().copied() {
            None => return Ok(hunkless(old_path, path, deleted)),
            Some((_, line)) if line.starts_with(SECTION_START) => return Ok(hunkless(old_path, path, deleted)),
            Some((_, line)) if line.starts_with("--- ") => break,
            Some(item @ (_, line)) if line.starts_with("+++ ") || line.starts_with("@@") => {
                return Err(unexpected_line(item));
            }
            Some((_, line)) => {
                if let Some(from) = line.strip_prefix("rename from ") {
                    old_path = unquote(from);
                } else if let Some(to) = line.strip_prefix("rename to ") {
                    path = unquote(to);
                } else if line.starts_with("deleted file mode ") {
                    deleted = true;
                }
                lines.next();
            }
        }
    }

    // `---`, then `+++`, then the first hunk header.
    let Some((_, minus)) = lines.next() else {
        unreachable!("the loop above only breaks on a peeked `---` line");
    };
    let plus = match lines.next() {
        Some((_, line)) if line.starts_with("+++ ") => line,
        Some(item) => return Err(unexpected_line(item)),
        None => return Err(unexpected_end()),
    };
    match lines.peek().copied() {
        Some((_, line)) if line.starts_with("@@") => {}
        Some((_, line)) if line.starts_with(SECTION_START) => return Err(unexpected_end()),
        Some(item) => return Err(unexpected_line(item)),
        None => return Err(unexpected_end()),
    }
    let old_path = side_path(&minus[4..], "a/");
    let new_path = side_path(&plus[4..], "b/");
    let deleted = new_path == DEV_NULL;
    let path = if deleted { old_path.clone() } else { new_path };

    // The hunks. The first header has position 0; every later line, headers included, takes the next position.
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut position = 0;
    let (mut old_line, mut new_line) = (0, 0);
    while let Some((_, line)) = lines.next_if(|(_, line)| !line.starts_with(SECTION_START)) {
        if line.starts_with("@@") {
            if !hunks.is_empty() {
                position += 1;
            }
            let hunk = Hunk::from_header(line);
            if let Some(range) = hunk.range {
                (old_line, new_line) = (range.old_start, range.new_start);
            }
            hunks.push(hunk);
            continue;
        }
        position += 1;
        // The peek above saw a hunk header before this loop, so the first line here was one.
        let Some(hunk) = hunks.last_mut() else {
            unreachable!("a section's hunk lines start with a hunk header");
        };
        let numbered = hunk.range.is_some();
        let (kind, text) = classify(line);
        let (old, new) = match kind {
            LineKind::Context => (Some(old_line), Some(new_line)),
            LineKind::Removed => (Some(old_line), None),
            LineKind::Added => (None, Some(new_line)),
            LineKind::NoNewline => (None, None),
        };
        let (old, new) = if numbered { (old, new) } else { (None, None) };
        if old.is_some() {
            old_line += 1;
        }
        if new.is_some() {
            new_line += 1;
        }
        hunk.lines.push(DiffLine {
            kind,
            text: text.to_owned(),
            old,
            new,
            position,
        });
    }

    Ok(ChangedFile {
        path,
        old_path,
        deleted,
        hunks,
    })
}

fn hunkless(old_path: String, path: String, deleted: bool) -> ChangedFile {
    ChangedFile {
        path,
        old_path,
        deleted,
        hunks: Vec::new(),
    }
}

impl Hunk {
    /// A hunk with no lines yet, from its header.
    fn from_header(header: &str) -> Hunk {
        let (range, context) = parse_hunk_header(header).map_or((None, ""), |(range, context)| (Some(range), context));
        Hunk {
            header: header.to_owned(),
            range,
            context: context.to_owned(),
            lines: Vec::new(),
        }
    }
}

/// The ranges and trimmed context of `@@ -a[,b] +c[,d] @@ <context>`, or `None` when it isn't one.
fn parse_hunk_header(header: &str) -> Option<(HunkRange, &str)> {
    let rest = header.strip_prefix("@@ -")?;
    let (old_start, old_count, rest) = parse_side(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, new_count, rest) = parse_side(rest)?;
    let context = rest.strip_prefix(" @@")?.trim();
    let range = HunkRange {
        old_start,
        old_count,
        new_start,
        new_count,
    };
    Some((range, context))
}

/// `<start>[,<count>]` at the front of `text`, with the count defaulting to 1, and the rest of `text`.
fn parse_side(text: &str) -> Option<(u32, u32, &str)> {
    let (start, rest) = parse_number(text)?;
    match rest.strip_prefix(',') {
        Some(rest) => {
            let (count, rest) = parse_number(rest)?;
            Some((start, count, rest))
        }
        None => Some((start, 1, rest)),
    }
}

fn parse_number(text: &str) -> Option<(u32, &str)> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// A hunk line's kind and its text without the marker.
fn classify(line: &str) -> (LineKind, &str) {
    match line.as_bytes().first() {
        Some(b'+') => (LineKind::Added, &line[1..]),
        Some(b'-') => (LineKind::Removed, &line[1..]),
        Some(b' ') => (LineKind::Context, &line[1..]),
        Some(b'\\') => (LineKind::NoNewline, line[1..].trim_start()),
        // An empty line, or one no tool should produce: kept as context, whole, so nothing is lost.
        _ => (LineKind::Context, line),
    }
}

/// The path on a `---` or `+++` line (after the marker), without `prefix`. [`DEV_NULL`] stays as is.
fn side_path(side: &str, prefix: &str) -> String {
    // Git ends the line with a tab when the path has a space in it.
    let side = side.strip_suffix('\t').unwrap_or(side);
    if side == DEV_NULL {
        return DEV_NULL.to_owned();
    }
    without_prefix(unquote(side), prefix)
}

fn without_prefix(path: String, prefix: &str) -> String {
    match path.strip_prefix(prefix) {
        Some(rest) => rest.to_owned(),
        None => path,
    }
}

/// The old and new paths of a `diff --git a/<old> b/<new>` line (after `diff --git `), without their prefixes.
///
/// Only used for sections without `---` / `+++` lines. Unquoted paths with spaces are ambiguous; the two paths are
/// taken to be the same when they can be, which is always true except for renames, whose `rename from` and `rename
/// to` lines override these.
fn git_line_paths(paths: &str) -> Option<(String, String)> {
    let (old, new) = if paths.starts_with('"') || paths.ends_with('"') {
        split_quoted(paths)?
    } else {
        let half = paths.len() / 2;
        let same = paths.len() % 2 == 1
            && paths.is_char_boundary(half)
            && paths[half..].starts_with(" b/")
            && paths.starts_with("a/")
            && paths[2..half] == paths[half + 3..];
        if same {
            (paths[..half].to_owned(), paths[half + 1..].to_owned())
        } else {
            let split = paths.find(" b/")?;
            (paths[..split].to_owned(), paths[split + 1..].to_owned())
        }
    };
    Some((without_prefix(old, "a/"), without_prefix(new, "b/")))
}

/// Two space-separated paths, either or both quoted.
fn split_quoted(paths: &str) -> Option<(String, String)> {
    if let Some(rest) = paths.strip_prefix('"') {
        let end = closing_quote(rest)?;
        let old = unquote(&paths[..end + 2]);
        let new = paths[end + 2..].strip_prefix(' ')?;
        Some((old, unquote(new)))
    } else {
        let split = paths.find(" \"")?;
        Some((paths[..split].to_owned(), unquote(&paths[split + 1..])))
    }
}

/// The byte index in `text` of the first `"` that isn't escaped.
fn closing_quote(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some(index),
            _ => {}
        }
    }
    None
}

/// A path as git writes it, with C-style quoting undone when it's quoted (`"t\303\251st"` is `tést`).
fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'r') => bytes.push(b'\r'),
            Some(b'a') => bytes.push(0x07),
            Some(b'b') => bytes.push(0x08),
            Some(b'f') => bytes.push(0x0c),
            Some(b'v') => bytes.push(0x0b),
            Some(digit @ b'0'..=b'7') => {
                let mut value = u32::from(digit - b'0');
                for _ in 0..2 {
                    if let Some(next) = chars.next_if(|b| (b'0'..=b'7').contains(b)) {
                        value = value * 8 + u32::from(next - b'0');
                    }
                }
                // Three octal digits can exceed a byte; git never writes those, so keep the low byte.
                bytes.push(value.to_le_bytes()[0]);
            }
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests;
