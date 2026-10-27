//! The split diff viewer: Python's `SplitDiffViewer` in its `DiffViewerContainer`. From the top, scrolling together:
//! the file filter, then one collapsible section per file, each holding one collapsible section per hunk, each
//! holding the hunk's [`HunkPane`].
//!
//! The viewer doesn't know about pull requests: it takes a parsed [`Diff`], so the review form (#187) can be added
//! after the files. Inside the details content it has an inner focus ring ([`DiffStop`]) computed from what's shown
//! and open, and a text input (the filter) that takes its keys before the binding chain, as the table's search box
//! does.

mod highlight;
mod pane;

use std::ops::Range;

use lazy_github_core::diff::{ChangedFile, Counts, Diff, Label};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub use highlight::code_spans;
pub use pane::{HunkPane, PaneLayout};

use super::collapsible::{CONTENT_INDENT, Collapsible, Placement, SectionFocus};
use super::command_log::Wheel;
use super::info::Scroll;
use super::markdown::{self, Rendered, TextRun};
use super::scroll_view::{Layout, LayoutCache, ScrollView};
use super::text_input::{self, InputOutcome, TextInput};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Owner};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{RichAnsi, Theme};

/// The filter's placeholder. It names the default key even with `diff.filter_files` rebound, as Python's does.
const FILTER_PLACEHOLDER: &str = "Filter files (press '/' to focus, ESC to clear)";

/// What an open deleted file shows.
const FILE_REMOVED: &str = "file was removed";

/// `SplitDiffViewer.BINDINGS`: next and previous hunk, and the filter key, while focus is anywhere in the viewer.
pub const DIFF_VIEWER_BINDINGS: &[BindingId] = &[
    BindingId::DiffNextHunk,
    BindingId::DiffPreviousHunk,
    BindingId::DiffFilterFiles,
];

/// `UnifiedDiffPane.BINDINGS`: the line cursor (hard-coded `j` / `k` in Python, Py bug B9) and add comment (#187).
pub const HUNK_PANE_BINDINGS: &[BindingId] = &[
    BindingId::DiffCursorDown,
    BindingId::DiffCursorUp,
    BindingId::DiffAddComment,
];

/// A focused section title's bindings: Textual's `CollapsibleTitle`.
const TITLE_BINDINGS: &[BindingId] = &[BindingId::TextualCollapsibleToggle];

/// The focused filter's bindings: `FileFilterInput`'s `escape`, which it takes before anything else, then
/// [`text_input::INPUT_BINDINGS`].
const FILTER_BINDINGS: &[BindingId] = &[
    BindingId::TextualFileFilterClear,
    BindingId::TextualInputCursorLeft,
    BindingId::TextualInputCursorRight,
    BindingId::TextualInputHome,
    BindingId::TextualInputEnd,
    BindingId::TextualInputCursorLeftSelect,
    BindingId::TextualInputCursorLeftWord,
    BindingId::TextualInputCursorLeftWordSelect,
    BindingId::TextualInputCursorRightSelect,
    BindingId::TextualInputCursorRightWord,
    BindingId::TextualInputCursorRightWordSelect,
    BindingId::TextualInputDeleteLeft,
    BindingId::TextualInputSelectAll,
    BindingId::TextualInputHomeSelect,
    BindingId::TextualInputEndSelect,
    BindingId::TextualInputDeleteRight,
    BindingId::TextualInputSubmit,
    BindingId::TextualInputDeleteLeftWord,
    BindingId::TextualInputDeleteLeftAll,
    BindingId::TextualInputDeleteRightWord,
    BindingId::TextualInputDeleteRightAll,
    BindingId::TextualInputCut,
    BindingId::TextualInputCopy,
    BindingId::TextualInputPaste,
];

/// A focus stop inside the viewer, in ring order: the scroll area, the filter, then each shown file's title, and
/// inside an open file each hunk's title and, when that hunk is open, its pane. Files and hunks are by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStop {
    /// The scrolling area itself (`DiffViewerContainer`).
    Scroll,
    /// The file filter.
    Filter,
    /// A file section's title.
    FileTitle(usize),
    /// A hunk section's title: the file, then the hunk.
    HunkTitle(usize, usize),
    /// A hunk's pane: the file, then the hunk.
    Pane(usize, usize),
}

impl DiffStop {
    /// The file the stop is in, if any.
    fn file(self) -> Option<usize> {
        match self {
            DiffStop::FileTitle(f) | DiffStop::HunkTitle(f, _) | DiffStop::Pane(f, _) => Some(f),
            DiffStop::Scroll | DiffStop::Filter => None,
        }
    }

    /// The hunk the stop is in, if any.
    fn hunk(self) -> Option<(usize, usize)> {
        match self {
            DiffStop::HunkTitle(f, h) | DiffStop::Pane(f, h) => Some((f, h)),
            DiffStop::Scroll | DiffStop::Filter | DiffStop::FileTitle(_) => None,
        }
    }
}

/// One file's section.
#[derive(Debug)]
struct FileSection {
    file: ChangedFile,
    section: Collapsible,
    hunks: Vec<HunkSection>,
    /// Whether the filter keeps it.
    shown: bool,
}

/// One hunk's section and pane.
#[derive(Debug)]
struct HunkSection {
    section: Collapsible,
    pane: HunkPane,
}

/// What the next draw scrolls into view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// A focused stop: its row, or the pane's rows.
    Stop(DiffStop),
    /// A section that opened or closed: all of its rows. A hunk is `(file, Some(hunk))`.
    Section(usize, Option<usize>),
}

/// The diff viewer: everything that scrolls, its scroll position and its inner focus.
#[derive(Debug)]
pub struct DiffView {
    scroll: ScrollView,
    body: Body,
    reveal: Option<Reveal>,
    /// Where the view was drawn last, for clicks.
    area: Rect,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    filter: TextInput,
    files: Vec<FileSection>,
    /// A line under the filter in place of the files: why the diff couldn't be parsed.
    notice: Option<String>,
    stop: DiffStop,
    /// Whether focus is inside the content, so the stop is drawn focused.
    focused: bool,
    layouts: LayoutCache,
    /// Where things landed in each cached layout, by width.
    placements: Vec<(u16, Placements)>,
    /// The width of the last layout asked for: the one the last draw used.
    last_width: u16,
}

/// Where the shown files landed in a layout, by file index (`None` for a file the filter hides).
#[derive(Debug, Clone, Default)]
struct Placements {
    files: Vec<Option<FilePlaced>>,
}

#[derive(Debug, Clone, Default)]
struct FilePlaced {
    section: Placement,
    /// By hunk index; empty while the file is closed.
    hunks: Vec<HunkPlaced>,
}

#[derive(Debug, Clone, Default)]
struct HunkPlaced {
    /// With its title's columns from the view's left edge.
    section: Placement,
    /// The pane's rows, its margins included; empty while the hunk is closed.
    pane: Range<usize>,
}

impl DiffView {
    /// The viewer of `diff`: every file open but deleted ones, every hunk closed, the scroll area focused.
    pub fn new(diff: Diff) -> Self {
        let files = diff
            .files
            .into_iter()
            .map(|file| {
                let hunks = file
                    .hunks
                    .iter()
                    .map(|hunk| HunkSection {
                        section: Collapsible::new(false),
                        pane: HunkPane::new(hunk),
                    })
                    .collect();
                FileSection {
                    section: Collapsible::new(!file.deleted),
                    file,
                    hunks,
                    shown: true,
                }
            })
            .collect();
        Self::with_body(files, None)
    }

    /// A diff that couldn't be parsed: the filter, then `error parsing diff - please view on github: <reason>`.
    pub fn invalid(reason: &str) -> Self {
        Self::with_body(
            Vec::new(),
            Some(format!("error parsing diff - please view on github: {reason}")),
        )
    }

    fn with_body(files: Vec<FileSection>, notice: Option<String>) -> Self {
        Self {
            scroll: ScrollView::default(),
            body: Body {
                filter: TextInput::new(FILTER_PLACEHOLDER),
                files,
                notice,
                stop: DiffStop::Scroll,
                focused: false,
                layouts: LayoutCache::default(),
                placements: Vec::new(),
                last_width: 0,
            },
            reveal: None,
            area: Rect::default(),
        }
    }

    /// The focused stop, if focus is inside the viewer.
    pub fn stop(&self) -> DiffStop {
        self.body.stop
    }

    /// The first visible row.
    pub fn offset(&self) -> usize {
        self.scroll.offset()
    }

    /// The filter's text.
    pub fn filter_text(&self) -> &str {
        self.body.filter.value()
    }

    /// Whether file `file`'s section is open.
    pub fn is_file_open(&self, file: usize) -> bool {
        self.body.files.get(file).is_some_and(|f| f.section.is_open())
    }

    /// Whether the filter keeps file `file`.
    pub fn is_file_shown(&self, file: usize) -> bool {
        self.body.files.get(file).is_some_and(|f| f.shown)
    }

    /// Whether hunk `hunk` of file `file` is open.
    pub fn is_hunk_open(&self, file: usize, hunk: usize) -> bool {
        self.hunk(file, hunk).is_some_and(|h| h.section.is_open())
    }

    /// The pane of hunk `hunk` of file `file`.
    pub fn pane(&self, file: usize, hunk: usize) -> Option<&HunkPane> {
        self.hunk(file, hunk).map(|h| &h.pane)
    }

    fn hunk(&self, file: usize, hunk: usize) -> Option<&HunkSection> {
        self.body.files.get(file)?.hunks.get(hunk)
    }

    /// A `textual.scroll.*` binding with the viewer focused.
    pub fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    /// A wheel notch over the viewer.
    pub fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    // Focus.

    /// The inner focus ring, from what's shown and open.
    fn stops(&self) -> Vec<DiffStop> {
        let mut stops = vec![DiffStop::Scroll, DiffStop::Filter];
        for (f, file) in self.body.files.iter().enumerate().filter(|(_, file)| file.shown) {
            stops.push(DiffStop::FileTitle(f));
            if !file.section.is_open() || file.file.deleted {
                continue;
            }
            for (h, hunk) in file.hunks.iter().enumerate() {
                stops.push(DiffStop::HunkTitle(f, h));
                if hunk.section.is_open() {
                    stops.push(DiffStop::Pane(f, h));
                }
            }
        }
        stops
    }

    /// Moves to `stop` and scrolls it into view at the next draw. Focus moving onto the filter selects its text, as
    /// Textual's `Input` does on focus.
    fn focus(&mut self, stop: DiffStop) {
        if stop == DiffStop::Filter && self.body.stop != DiffStop::Filter {
            self.body.filter.select_all();
        }
        self.body.set_stop(stop);
        if stop != DiffStop::Scroll {
            self.reveal = Some(Reveal::Stop(stop));
        }
    }

    /// One stop `forward` or back; `false` past either end, where focus leaves the viewer.
    pub fn focus_step(&mut self, forward: bool) -> bool {
        let stops = self.stops();
        let at = stops.iter().position(|&stop| stop == self.body.stop).unwrap_or(0);
        let next = if forward {
            at.checked_add(1).filter(|&next| next < stops.len())
        } else {
            at.checked_sub(1)
        };
        match next {
            Some(next) => {
                self.focus(stops[next]);
                true
            }
            None => false,
        }
    }

    /// Focus enters from before: the scroll area.
    pub fn focus_first(&mut self) {
        self.focus(DiffStop::Scroll);
    }

    /// Focus enters from after: the last stop.
    pub fn focus_last(&mut self) {
        let last = self.stops().last().copied().unwrap_or(DiffStop::Scroll);
        self.focus(last);
    }

    /// The chain links in front of the content's scroll link, for the focused stop.
    pub fn chain(&self) -> Vec<ChainLink> {
        let viewer = ChainLink::new(Owner::DiffViewer, DIFF_VIEWER_BINDINGS);
        match self.body.stop {
            DiffStop::Scroll => vec![viewer],
            DiffStop::Filter => vec![ChainLink::new(Owner::DiffFilter, FILTER_BINDINGS), viewer],
            DiffStop::FileTitle(_) | DiffStop::HunkTitle(..) => {
                vec![ChainLink::new(Owner::CollapsibleTitle, TITLE_BINDINGS), viewer]
            }
            DiffStop::Pane(..) => vec![ChainLink::new(Owner::HunkPane, HUNK_PANE_BINDINGS), viewer],
        }
    }

    /// How the footer and help panel show a binding of [`Self::chain`]. With the scroll area focused the viewer's
    /// keys work but aren't shown: in Python the scroll area is outside the `SplitDiffViewer` that binds them, so
    /// its footer doesn't list them.
    pub fn binding_state(&self, owner: Owner, _id: BindingId) -> BindingState {
        if owner == Owner::DiffViewer && self.body.stop == DiffStop::Scroll {
            BindingState::Hidden
        } else {
            BindingState::Enabled
        }
    }

    /// A binding matched at one of [`Self::chain`]'s links. `None` lets the key fall through.
    pub fn on_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        match (owner, id) {
            (Owner::CollapsibleTitle, BindingId::TextualCollapsibleToggle) => match self.body.stop {
                DiffStop::FileTitle(f) => self.toggle(f, None),
                DiffStop::HunkTitle(f, h) => self.toggle(f, Some(h)),
                _ => return None,
            },
            (Owner::HunkPane, BindingId::DiffCursorDown | BindingId::DiffCursorUp) => {
                let DiffStop::Pane(f, h) = self.body.stop else {
                    return None;
                };
                let pane = &mut self.body.files[f].hunks[h].pane;
                let moved = if id == BindingId::DiffCursorDown {
                    pane.cursor_down()
                } else {
                    pane.cursor_up()
                };
                if moved {
                    self.body.layouts.clear();
                }
            }
            // Add comment and its modal are #187.
            (Owner::HunkPane, BindingId::DiffAddComment) => return Some(Action::Unimplemented(id)),
            (Owner::DiffViewer, BindingId::DiffNextHunk) => self.jump_to_hunk(true),
            (Owner::DiffViewer, BindingId::DiffPreviousHunk) => self.jump_to_hunk(false),
            (Owner::DiffViewer, BindingId::DiffFilterFiles) => self.focus(DiffStop::Filter),
            (Owner::DiffFilter, BindingId::TextualFileFilterClear) => {
                self.body.filter.set_value("");
                self.apply_filter();
                self.focus(DiffStop::Scroll);
            }
            // The filter took its editing keys before the chain.
            (Owner::DiffFilter, _) => {}
            _ => return None,
        }
        Some(Action::Consumed)
    }

    /// A key before the binding chain: the focused filter takes printable keys and its editing keys. `enter` leaves
    /// it for the first shown file's first hunk (Py bug B59). Paste types `clipboard`, and copy and cut hand the
    /// text to the app's clipboard.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str) -> Option<Action> {
        let filter = &mut self.body.filter;
        if self.body.stop != DiffStop::Filter || !filter.consumes_key(key, keymap) {
            return None;
        }
        let mut action = Action::Consumed;
        match filter.handle_key(key, keymap, clipboard) {
            InputOutcome::Changed => self.apply_filter(),
            InputOutcome::Submitted => self.leave_filter(),
            // A cut changes the text, so the filter applies again.
            InputOutcome::Copied(text) => {
                self.apply_filter();
                action = Action::Copy(text);
            }
            InputOutcome::Ignored => {}
        }
        self.body.layouts.clear();
        Some(action)
    }

    /// A bracketed paste: its first line goes into the filter while the filter has focus. Says whether it did.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        if self.body.stop != DiffStop::Filter {
            return false;
        }
        self.body.filter.handle_paste(text);
        self.apply_filter();
        true
    }

    /// Shows the files whose full path contains the filter's text, ignoring case; all of them when it's empty.
    fn apply_filter(&mut self) {
        let needle = self.body.filter.value().to_lowercase();
        for file in &mut self.body.files {
            file.shown = needle.is_empty() || file.file.path.to_lowercase().contains(&needle);
        }
        self.body.layouts.clear();
    }

    /// `enter` in the filter. Python focuses the first pane whatever its hunk, so a closed one has focus where it
    /// can't be seen (Py bug B59); this focuses the first shown file's first hunk: its pane when it's open, else its
    /// title (or the file's title, when the file is closed).
    fn leave_filter(&mut self) {
        let first = self
            .body
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.shown)
            .find(|(_, file)| !file.file.deleted && !file.hunks.is_empty());
        let stop = match first {
            Some((f, file)) if !file.section.is_open() => DiffStop::FileTitle(f),
            Some((f, file)) if file.hunks[0].section.is_open() => DiffStop::Pane(f, 0),
            Some((f, _)) => DiffStop::HunkTitle(f, 0),
            None => DiffStop::Scroll,
        };
        self.focus(stop);
    }

    /// `J` / `K` (Py bug B8): the next or previous hunk's pane among the shown files, opening its file and hunk.
    /// From outside any hunk, the first or the last; at either end, nothing.
    fn jump_to_hunk(&mut self, forward: bool) {
        let hunks: Vec<(usize, usize)> = self
            .body
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.shown && !file.file.deleted)
            .flat_map(|(f, file)| (0..file.hunks.len()).map(move |h| (f, h)))
            .collect();
        let target = match self.body.stop.hunk().and_then(|at| hunks.iter().position(|&h| h == at)) {
            Some(i) if forward => hunks.get(i + 1),
            Some(i) => i.checked_sub(1).and_then(|i| hunks.get(i)),
            None if forward => hunks.first(),
            None => hunks.last(),
        };
        let Some(&(f, h)) = target else {
            return;
        };
        let file = &mut self.body.files[f];
        file.section.set_open(true);
        file.hunks[h].section.set_open(true);
        self.body.layouts.clear();
        self.focus(DiffStop::Pane(f, h));
    }

    /// Opens or closes a file's section or, with `hunk`, one of its hunks' sections, and scrolls it into view, as
    /// Textual's `Collapsible` does after a toggle.
    fn toggle(&mut self, file: usize, hunk: Option<usize>) {
        let Some(section) = self.body.files.get_mut(file) else {
            return;
        };
        match hunk {
            Some(h) => {
                if let Some(hunk) = section.hunks.get_mut(h) {
                    hunk.section.toggle();
                }
            }
            None => section.section.toggle(),
        }
        self.body.layouts.clear();
        self.reveal = Some(Reveal::Section(file, hunk));
    }

    /// A left press at a screen cell, against the last frame: a title focuses and toggles its section, a pane or the
    /// filter takes focus (the filter's cursor moving to the press), and anything else focuses the scroll area.
    pub fn click(&mut self, position: Position) {
        let stop = self.stop_at(position);
        match stop {
            DiffStop::FileTitle(f) => self.toggle(f, None),
            DiffStop::HunkTitle(f, h) => self.toggle(f, Some(h)),
            DiffStop::Filter => {
                self.focus(stop);
                // Only the columns matter to the press.
                let input = Rect {
                    width: self.body.last_width,
                    height: text_input::HEIGHT,
                    ..self.area
                };
                self.body.filter.press(input, position.x);
                self.body.layouts.clear();
                return;
            }
            DiffStop::Pane(..) | DiffStop::Scroll => {}
        }
        self.focus(stop);
    }

    /// The stop drawn at a screen cell.
    fn stop_at(&self, position: Position) -> DiffStop {
        let area = self.area;
        let width = self.body.last_width;
        if !area.contains(position) || position.x >= area.x.saturating_add(width) {
            return DiffStop::Scroll;
        }
        let row = self.scroll.offset() + usize::from(position.y - area.y);
        let x = position.x - area.x;
        if row < usize::from(text_input::HEIGHT) {
            return DiffStop::Filter;
        }
        let Some(placements) = self.body.placements(width) else {
            return DiffStop::Scroll;
        };
        for (f, placed) in placements.files.iter().enumerate() {
            let Some(placed) = placed else { continue };
            let at = &placed.section;
            if row == at.title_row && at.title_columns.contains(&x) {
                return DiffStop::FileTitle(f);
            }
            for (h, hunk) in placed.hunks.iter().enumerate() {
                if row == hunk.section.title_row && hunk.section.title_columns.contains(&x) {
                    return DiffStop::HunkTitle(f, h);
                }
                // The box, between its margins, from the hunk's indent.
                let boxed = hunk.pane.start + 1..hunk.pane.end.saturating_sub(1);
                if boxed.contains(&row) && x >= 2 * CONTENT_INDENT {
                    return DiffStop::Pane(f, h);
                }
            }
        }
        DiffStop::Scroll
    }

    // Drawing.

    /// Draws the viewer into `area`, `focused` when focus is inside the details content.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        self.body.set_focused(focused);
        if let Some(reveal) = self.reveal.take() {
            let width = self.scroll.measure(area, theme, &mut self.body);
            if let Some(rows) = self.body.rows_to_reveal(reveal, width) {
                self.scroll.reveal(rows);
            }
        }
        self.scroll.render(frame, area, theme, &mut self.body);
        self.area = area;
    }
}

impl Body {
    fn set_stop(&mut self, stop: DiffStop) {
        if self.stop != stop {
            self.stop = stop;
            self.layouts.clear();
        }
    }

    /// Focus moved in or out of the content. Only a stop other than the scroll area looks different.
    fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if self.stop != DiffStop::Scroll {
                self.layouts.clear();
            }
        }
    }

    fn placements(&self, width: u16) -> Option<&Placements> {
        self.placements.iter().find(|(w, _)| *w == width).map(|(_, p)| p)
    }

    /// The rows to scroll into view for `reveal`, in the layout `width` wide.
    fn rows_to_reveal(&self, reveal: Reveal, width: u16) -> Option<Range<usize>> {
        let placements = self.placements(width)?;
        let file = |f: usize| placements.files.get(f)?.as_ref();
        let hunk = |f: usize, h: usize| file(f)?.hunks.get(h);
        let title = |placement: &Placement| placement.title_row..placement.title_row + 1;
        match reveal {
            Reveal::Stop(DiffStop::Scroll) => None,
            Reveal::Stop(DiffStop::Filter) => Some(0..usize::from(text_input::HEIGHT)),
            Reveal::Stop(DiffStop::FileTitle(f)) => file(f).map(|p| title(&p.section)),
            Reveal::Stop(DiffStop::HunkTitle(f, h)) => hunk(f, h).map(|p| title(&p.section)),
            Reveal::Stop(DiffStop::Pane(f, h)) => hunk(f, h).map(|p| p.pane.clone()),
            Reveal::Section(f, None) => file(f).map(|p| p.section.rows.clone()),
            Reveal::Section(f, Some(h)) => hunk(f, h).map(|p| p.section.rows.clone()),
        }
    }

    /// How the section of file `file` or, with `hunk`, of one of its hunks looks with focus where it is.
    fn section_focus(&self, file: usize, hunk: Option<usize>) -> SectionFocus {
        if !self.focused {
            return SectionFocus::default();
        }
        let stop = self.stop;
        match hunk {
            None => SectionFocus {
                title: stop == DiffStop::FileTitle(file),
                within: stop.file() == Some(file),
            },
            Some(h) => SectionFocus {
                title: stop == DiffStop::HunkTitle(file, h),
                within: stop.hunk() == Some((file, h)),
            },
        }
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        self.last_width = width;
        let focus: Vec<FileFocus> = (0..self.files.len())
            .map(|f| FileFocus {
                section: self.section_focus(f, None),
                hunks: (0..self.files[f].hunks.len())
                    .map(|h| self.section_focus(f, Some(h)))
                    .collect(),
                pane: match self.stop {
                    DiffStop::Pane(file, h) if self.focused && file == f => Some(h),
                    _ => None,
                },
            })
            .collect();
        let filter_focused = self.focused && self.stop == DiffStop::Filter;
        let Body {
            filter,
            files,
            notice,
            layouts,
            placements,
            ..
        } = self;
        layouts.get(width, theme, || {
            let mut out = Rendered::default();
            let mut placed = Placements::default();
            if width == 0 {
                return out;
            }
            out.lines.extend(filter_lines(filter, width, theme, filter_focused));
            if let Some(notice) = notice {
                let text = Style::new().fg(theme.color(theme.palette.foreground, theme.background()));
                let notice = markdown::wrap_text(&[TextRun::new(notice.as_str(), text)], width);
                out.lines.extend(notice.lines);
            }
            for (file, focus) in files.iter_mut().zip(&focus) {
                placed
                    .files
                    .push(file.shown.then(|| layout_file(&mut out, file, focus, width, theme)));
            }
            placements.retain(|(w, _)| *w != width);
            if placements.len() == 2 {
                placements.remove(0);
            }
            placements.push((width, placed));
            out
        })
    }
}

/// How one file's sections look with focus where it is.
struct FileFocus {
    section: SectionFocus,
    /// By hunk index.
    hunks: Vec<SectionFocus>,
    /// The hunk whose pane has focus.
    pane: Option<usize>,
}

/// Adds `file`'s section to `out`, `width` cells wide, and returns where it and its hunks landed.
fn layout_file(out: &mut Rendered, file: &mut FileSection, focus: &FileFocus, width: u16, theme: &Theme) -> FilePlaced {
    let FileSection {
        file: changed,
        section,
        hunks: sections,
        ..
    } = file;
    let mut hunks = Vec::new();
    let placement = section.layout(
        out,
        &title(&changed.label(), theme),
        focus.section,
        width,
        theme,
        |cx| {
            if changed.deleted {
                return removed_notice(cx.width, theme);
            }
            let mut inner = Rendered::default();
            for (h, hunk) in sections.iter_mut().enumerate() {
                let placed = layout_hunk(&mut inner, hunk, changed, h, focus, cx.width, theme);
                hunks.push(placed);
            }
            inner
        },
    );
    // The hunks were laid out inside the file's contents: move them to the view's rows and columns.
    let offset = placement.content.start;
    for hunk in &mut hunks {
        let section = &mut hunk.section;
        section.rows = shift(&section.rows, offset);
        section.title_row += offset;
        section.content = shift(&section.content, offset);
        section.title_columns =
            section.title_columns.start + CONTENT_INDENT..section.title_columns.end + CONTENT_INDENT;
        hunk.pane = shift(&hunk.pane, offset);
    }
    FilePlaced {
        section: placement,
        hunks,
    }
}

/// Adds hunk `h` of `changed` to `out`, a file's contents `width` cells wide, and returns where it landed in them.
fn layout_hunk(
    out: &mut Rendered,
    hunk: &mut HunkSection,
    changed: &ChangedFile,
    h: usize,
    focus: &FileFocus,
    width: u16,
    theme: &Theme,
) -> HunkPlaced {
    let focused = focus.pane == Some(h);
    let mut pane_rows = 0..0;
    let mut viewport = None;
    let HunkSection { section, pane } = hunk;
    let section = section.layout(
        out,
        &title(&changed.hunks[h].label(), theme),
        focus.hunks[h],
        width,
        theme,
        |content| {
            let layout = pane.layout(changed, &changed.hunks[h], content.width, theme, focused, content.under);
            viewport = Some(layout.viewport);
            pane_rows = 0..layout.lines.len();
            Rendered {
                lines: layout.lines,
                links: Vec::new(),
            }
        },
    );
    if let Some(viewport) = viewport {
        pane.set_viewport(viewport);
    }
    HunkPlaced {
        pane: shift(&pane_rows, section.content.start),
        section,
    }
}

fn shift(rows: &Range<usize>, by: usize) -> Range<usize> {
    rows.start + by..rows.end + by
}

/// The filter's three rows, drawn as the table's search box is and read back as lines.
fn filter_lines(filter: &mut TextInput, width: u16, theme: &Theme, focused: bool) -> Vec<Line<'static>> {
    let area = Rect::new(0, 0, width, text_input::HEIGHT);
    let mut buf = Buffer::empty(area);
    filter.render(&mut buf, area, theme, theme.background(), focused);
    (0..area.height).map(|y| buffer_line(&buf, y)).collect()
}

/// Row `y` of `buf` as a line of styled spans, a span per run of cells with the same style.
fn buffer_line(buf: &Buffer, y: u16) -> Line<'static> {
    let area = buf.area;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut skip = 0;
    for x in area.left()..area.right() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let cell = &buf[(x, y)];
        let symbol = cell.symbol();
        // A wide character covers the cells after it.
        skip = Span::raw(symbol).width().saturating_sub(1);
        let style = cell.style();
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(symbol),
            _ => spans.push(Span::styled(symbol.to_owned(), style)),
        }
    }
    Line::from(spans)
}

/// A deleted file's contents: `file was removed` in `$error`, between blank rows (its `margin: 1 0`).
fn removed_notice(width: u16, theme: &Theme) -> Rendered {
    let style = Style::new().fg(theme.color(theme.palette.error, theme.background()));
    let mut out = Rendered::default();
    out.lines.push(Line::default());
    out.lines
        .extend(markdown::wrap_text(&[TextRun::new(FILE_REMOVED, style)], width).lines);
    out.lines.push(Line::default());
    out
}

/// A file or hunk label as title runs: its counts green and red, as Python's `Text` styles them.
fn title(label: &Label, theme: &Theme) -> Vec<TextRun> {
    let plain = |text: String| TextRun::new(text, Style::new());
    let mut runs = vec![plain(label.text.clone())];
    let Counts { additions, deletions } = label.counts;
    if !label.counts.is_empty() {
        runs.push(plain(" (".into()));
        if additions > 0 {
            runs.push(TextRun::new(
                format!("+{additions}"),
                Style::new().fg(theme.rich_ansi(RichAnsi::Green)),
            ));
        }
        if additions > 0 && deletions > 0 {
            runs.push(plain(" ".into()));
        }
        if deletions > 0 {
            runs.push(TextRun::new(
                format!("-{deletions}"),
                Style::new().fg(theme.rich_ansi(RichAnsi::Red)),
            ));
        }
        runs.push(plain(")".into()));
    }
    if !label.suffix.is_empty() {
        runs.push(plain(label.suffix.clone()));
    }
    runs
}

#[cfg(test)]
mod tests;
