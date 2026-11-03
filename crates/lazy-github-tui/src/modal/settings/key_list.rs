//! The Key Bindings tab's list (#381): every rebindable binding on one line, under its group's header, with one
//! highlighted line. `enter` waits for the next key and replaces the binding's keys with it, `a` adds the next key,
//! the cancel key stops waiting, and `r` resets the binding. A detail line under the list describes the highlighted
//! binding. Decision settings-key-bindings-list.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use indexmap::IndexMap;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};

use super::key_row::KeyRow;
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::option_nav;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::display;
use crate::keymap::{BINDING_GROUPS, BindingId, KeyChord, KeySpec, Keymap, textual_name};
use crate::theme::{Paint, Theme, ThemeColor};

/// The list's bindings while it has focus and isn't waiting for a key: `OptionList`'s moves (hidden), then change,
/// add and reset.
pub(crate) const KEY_LIST_BINDINGS: &[BindingId] = &[
    BindingId::TextualOptionListCursorDown,
    BindingId::TextualOptionListLast,
    BindingId::TextualOptionListFirst,
    BindingId::TextualOptionListPageDown,
    BindingId::TextualOptionListPageUp,
    BindingId::TextualOptionListCursorUp,
    BindingId::SettingsKeysChange,
    BindingId::SettingsKeysAdd,
    BindingId::SettingsKeysReset,
];

/// The list's binding while it waits for a key: cancel. Every other key is recorded.
pub(crate) const KEY_CAPTURE_BINDINGS: &[BindingId] = &[BindingId::SettingsKeysCancel];

/// The action column's widest.
const MAX_ACTION_WIDTH: usize = 28;

/// The key column's width.
const KEYS_WIDTH: usize = 16;

/// The id column shows only when at least this many cells are left for it.
const MIN_ID_WIDTH: usize = 12;

/// The marker cells in front of the action: `*` for a changed binding, and a cell left for a conflict's `!` (#382).
const MARKER_WIDTH: usize = 2;

/// The blank cells between columns.
const GAP: usize = 2;

/// What the capture cell shows while the list waits for a key.
const PRESS_A_KEY: &str = "press a key…";

/// The fewest list lines the tab keeps before it scrolls as a whole instead.
pub(super) const MIN_LINES: u16 = 3;

/// One line of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Group `BINDING_GROUPS[index]`'s header.
    Header(usize),
    /// Row `index`.
    Binding(usize),
}

/// What a capture does with the recorded key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureMode {
    /// `enter`: the key replaces the binding's keys.
    Replace,
    /// `a`: the key is added to them.
    Add,
}

/// Whether the list took a key, a binding or a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(super) enum KeyListOutcome {
    /// The list's: something may have changed.
    Consumed,
    /// Not the list's.
    Ignored,
}

/// The list's state.
#[derive(Debug, Clone)]
pub(super) struct KeyList {
    /// Every binding, in [`BINDING_GROUPS`] order.
    rows: Vec<KeyRow>,
    /// Each row's group.
    groups: Vec<usize>,
    /// The lines the filter shows.
    lines: Vec<Line>,
    /// The highlighted row. `None` only when the filter shows nothing.
    highlighted: Option<usize>,
    /// The first line in view.
    scroll: usize,
    /// The lines in view at the last draw, which a page key moves by. 0 until the first draw.
    view: usize,
    /// The highlight moved before the first draw, so the draw scrolls to it.
    reveal_pending: bool,
    /// Waiting for a key, and what to do with it.
    capture: Option<CaptureMode>,
    /// The action column's width: the longest action, capped.
    action_width: usize,
    /// Where the lines were drawn last, in the coordinates of the buffer they went to.
    lines_area: Rect,
}

impl KeyList {
    /// The list of every rebindable binding, each on its override from `overrides` or its default keys, with the
    /// first one highlighted.
    pub(super) fn new(overrides: &IndexMap<String, String>) -> Self {
        let (groups, rows): (Vec<usize>, Vec<KeyRow>) = KeyRow::all(overrides).into_iter().unzip();
        let action_width = rows
            .iter()
            .map(|row| display::width(&row.action()))
            .max()
            .unwrap_or(0)
            .min(MAX_ACTION_WIDTH);
        let mut list = Self {
            rows,
            groups,
            lines: Vec::new(),
            highlighted: None,
            scroll: 0,
            view: 0,
            reveal_pending: false,
            capture: None,
            action_width,
            lines_area: Rect::default(),
        };
        list.filter(None);
        list
    }

    /// The bindings, in list order.
    #[cfg(test)]
    pub(super) fn rows(&self) -> &[KeyRow] {
        &self.rows
    }

    /// The highlighted binding.
    pub(super) fn highlighted(&self) -> Option<&KeyRow> {
        self.rows.get(self.highlighted?)
    }

    /// Shows only the bindings whose description or id `query` fuzzy-matches, or every one for `None`. A header
    /// shows while one of its bindings does. A hidden highlight moves to the first binding shown.
    pub(super) fn filter(&mut self, query: Option<&str>) {
        self.lines.clear();
        let mut group = None;
        for (index, row) in self.rows.iter().enumerate() {
            if !query.is_none_or(|query| row.matches(query)) {
                continue;
            }
            if group != Some(self.groups[index]) {
                group = Some(self.groups[index]);
                self.lines.push(Line::Header(self.groups[index]));
            }
            self.lines.push(Line::Binding(index));
        }
        if self.highlighted.and_then(|row| self.line_of(row)).is_none() {
            let first = self.binding_lines().next().map(|(_, row)| row);
            self.highlighted = first;
        }
        self.scroll = 0;
        self.reveal();
    }

    /// Whether the filter shows any binding.
    pub(super) fn has_lines(&self) -> bool {
        self.highlighted.is_some()
    }

    /// Whether it's waiting for a key.
    pub(super) fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Stops waiting for a key, leaving the binding as it was.
    pub(super) fn cancel_capture(&mut self) {
        self.capture = None;
    }

    /// The lines that are bindings, with their line index and row.
    fn binding_lines(&self) -> impl DoubleEndedIterator<Item = (usize, usize)> + '_ {
        self.lines.iter().enumerate().filter_map(|(line, &kind)| match kind {
            Line::Binding(row) => Some((line, row)),
            Line::Header(_) => None,
        })
    }

    fn line_of(&self, row: usize) -> Option<usize> {
        self.lines.iter().position(|&line| line == Line::Binding(row))
    }

    /// The binding on line `line`, or else the nearest one before it, or else after it; with `forward`, after
    /// first. A page key stops short of a header rather than passing it.
    fn binding_near(&self, line: usize, forward: bool) -> Option<usize> {
        let after = self.binding_lines().find(|&(at, _)| at >= line).map(|(_, row)| row);
        let before = self
            .binding_lines()
            .rev()
            .find(|&(at, _)| at <= line)
            .map(|(_, row)| row);
        if forward { after.or(before) } else { before.or(after) }
    }

    /// Highlights `row` and scrolls it into view.
    fn highlight(&mut self, row: Option<usize>) {
        if row.is_some() {
            self.highlighted = row;
            self.reveal();
        }
    }

    /// Scrolls so the highlighted line is in view, with its group's header when it's the group's first binding.
    fn reveal(&mut self) {
        let Some(line) = self.highlighted.and_then(|row| self.line_of(row)) else {
            return;
        };
        if self.view == 0 {
            self.reveal_pending = true;
            return;
        }
        let (first, height) = match line.checked_sub(1).map(|above| self.lines[above]) {
            Some(Line::Header(_)) if self.view > 1 => (line - 1, 2),
            _ => (line, 1),
        };
        self.scroll = option_nav::scroll_into_view(self.scroll, first, height, self.view);
    }

    /// A binding of [`KEY_LIST_BINDINGS`]: the moves skip headers and stop at the ends, the page keys move by the
    /// lines in view; change and add start waiting for a key, reset puts the binding back to its defaults.
    pub(super) fn on_binding(&mut self, id: BindingId) -> KeyListOutcome {
        use BindingId as B;
        if !KEY_LIST_BINDINGS.contains(&id) {
            return KeyListOutcome::Ignored;
        }
        let Some(row) = self.highlighted else {
            return KeyListOutcome::Consumed;
        };
        let line = self.line_of(row).unwrap_or(0);
        let page = self.view.max(1);
        match id {
            B::TextualOptionListCursorDown => {
                let next = self.binding_lines().find(|&(at, _)| at > line).map(|(_, row)| row);
                self.highlight(next);
            }
            B::TextualOptionListCursorUp => {
                let previous = self
                    .binding_lines()
                    .rev()
                    .find(|&(at, _)| at < line)
                    .map(|(_, row)| row);
                self.highlight(previous);
            }
            B::TextualOptionListFirst => {
                let first = self.binding_lines().next().map(|(_, row)| row);
                self.highlight(first);
            }
            B::TextualOptionListLast => {
                let last = self.binding_lines().next_back().map(|(_, row)| row);
                self.highlight(last);
            }
            B::TextualOptionListPageDown => {
                let target = (line + page).min(self.lines.len().saturating_sub(1));
                self.highlight(self.binding_near(target, false));
            }
            B::TextualOptionListPageUp => {
                let target = line.saturating_sub(page);
                self.highlight(self.binding_near(target, true));
            }
            B::SettingsKeysChange => self.capture = Some(CaptureMode::Replace),
            B::SettingsKeysAdd => self.capture = Some(CaptureMode::Add),
            B::SettingsKeysReset => {
                let _ = self.rows[row].reset();
            }
            _ => return KeyListOutcome::Ignored,
        }
        KeyListOutcome::Consumed
    }

    /// A key while waiting for one: the cancel key stops waiting; any other key Textual names is recorded into the
    /// highlighted binding, by the capture's mode, and the wait ends; a key with no name is ignored and the wait goes
    /// on.
    pub(super) fn capture(&mut self, chord: &KeyChord, keymap: &Keymap) -> KeyListOutcome {
        let Some(mode) = self.capture else {
            return KeyListOutcome::Ignored;
        };
        if keymap.match_in(KEY_CAPTURE_BINDINGS, chord).is_some() {
            self.capture = None;
            return KeyListOutcome::Consumed;
        }
        let Some(name) = textual_name(chord) else {
            return KeyListOutcome::Consumed;
        };
        self.capture = None;
        if let Some(row) = self.highlighted.and_then(|row| self.rows.get_mut(row)) {
            match mode {
                CaptureMode::Replace => row.replace(name),
                CaptureMode::Add => {
                    let _ = row.add(&name);
                }
            }
        }
        KeyListOutcome::Consumed
    }

    /// Writes every binding into `overrides` as Save does ([`KeyRow::save`]). Overrides for ids the app doesn't know
    /// are left alone.
    pub(super) fn save(&self, overrides: &mut IndexMap<String, String>) {
        for row in &self.rows {
            row.save(overrides);
        }
    }

    /// A left press at `event`, in the coordinates the list was drawn in: on a binding it highlights it, or, when
    /// the list already had focus (`focused`) and the binding was highlighted, starts waiting for a key to replace
    /// its keys. A press on a header does nothing more than the focus the dialog gives the list.
    pub(super) fn press(&mut self, event: MouseEvent, focused: bool) -> KeyListOutcome {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || !self.lines_area.contains(Position::new(event.column, event.row))
        {
            return KeyListOutcome::Ignored;
        }
        let line = self.scroll + usize::from(event.row - self.lines_area.y);
        let Some(&Line::Binding(row)) = self.lines.get(line) else {
            return KeyListOutcome::Consumed;
        };
        if focused && self.highlighted == Some(row) {
            self.capture = Some(CaptureMode::Replace);
        } else {
            self.highlight(Some(row));
        }
        KeyListOutcome::Consumed
    }

    /// Whether `column`, `row` (in the coordinates the list was drawn in) is on its lines.
    pub(super) fn contains(&self, column: u16, row: u16) -> bool {
        self.lines_area.contains(Position::new(column, row))
    }

    /// A wheel notch over the list: it scrolls, and the highlight stays where it is.
    pub(super) fn wheel(&mut self, wheel: Wheel) {
        let max = self.lines.len().saturating_sub(self.view);
        self.scroll = match wheel {
            Wheel::Up => self.scroll.saturating_sub(WHEEL_ROWS),
            Wheel::Down => self.scroll + WHEEL_ROWS,
            Wheel::Left | Wheel::Right => self.scroll,
        }
        .min(max);
    }

    /// The detail line: what the highlighted binding is, or what the wait is for.
    pub(super) fn detail(&self) -> String {
        let Some(row) = self.highlighted() else {
            return String::new();
        };
        let cancel = KeySpec::parse_list(BindingId::SettingsKeysCancel.def().default_keys)
            .first()
            .map(|(token, _)| KeySpec::display(token))
            .unwrap_or_default();
        match self.capture {
            Some(CaptureMode::Replace) => format!("Press the new key for {}. {cancel} cancels.", row.action()),
            Some(CaptureMode::Add) => format!("Press a key to add to {}. {cancel} cancels.", row.action()),
            None => {
                let mut detail = format!("{} · default {}", row.config_id, row.default_display());
                if !row.is_default() {
                    detail.push_str(" · changed");
                }
                detail
            }
        }
    }

    /// Draws the list into `area` over `under`: its lines in every row but the last, with a scrollbar in their last
    /// two columns when they overflow, and the detail line in the last row. `focused` draws the highlight as a
    /// focused list's.
    pub(super) fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let area = area.intersection(buf.area);
        if area.height < 2 || area.width == 0 {
            self.lines_area = Rect::default();
            return;
        }
        let r = theme.roles();
        let view = area.height - 1;
        self.view = usize::from(view);
        let max = self.lines.len().saturating_sub(self.view);
        self.scroll = self.scroll.min(max);
        if std::mem::take(&mut self.reveal_pending) {
            self.reveal();
        }
        let overflows = self.lines.len() > self.view;
        let lines_area = Rect {
            height: view,
            width: if overflows {
                area.width.saturating_sub(scrollbar::VERTICAL_WIDTH)
            } else {
                area.width
            },
            ..area
        };
        self.lines_area = lines_area;

        let plain = Style::new().fg(theme.color(r.text.plain, under));
        let highlight = highlight_ink(theme, under, focused);
        for (dy, &line) in (0..view).zip(self.lines.iter().skip(self.scroll)) {
            let y = lines_area.y + dy;
            match line {
                Line::Header(group) => {
                    let title = BINDING_GROUPS.get(group).map_or("", |group| group.title);
                    buf.set_stringn(
                        lines_area.x,
                        y,
                        title,
                        usize::from(lines_area.width),
                        plain.add_modifier(Modifier::BOLD),
                    );
                }
                Line::Binding(row) => {
                    let (style, fg, bg) = if self.highlighted == Some(row) {
                        highlight
                    } else {
                        (plain, r.text.plain, under)
                    };
                    let row_area = Rect {
                        y,
                        height: 1,
                        ..lines_area
                    };
                    self.draw_binding(buf, row_area, theme, row, style, (fg, bg));
                }
            }
        }
        if overflows {
            let bar = Rect {
                x: lines_area.right(),
                width: area.right() - lines_area.right(),
                ..lines_area
            };
            let state = ScrollbarState {
                content: self.lines.len(),
                window: self.view,
                position: self.scroll,
            };
            draw_scrollbar(buf, bar, theme, under, state);
        }
        let detail_style = theme.rich_dim(plain, r.text.plain, under);
        buf.set_stringn(
            area.x,
            area.bottom() - 1,
            display::truncate(&self.detail(), usize::from(area.width)),
            usize::from(area.width),
            detail_style,
        );
    }

    /// Draws binding `row` on the one-row `area` in `style` (its fg is `ink.0` over `ink.1`): the marker, the
    /// action, the keys (or the capture prompt) and the id.
    fn draw_binding(
        &self,
        buf: &mut Buffer,
        area: Rect,
        theme: &Theme,
        row: usize,
        style: Style,
        ink: (ThemeColor, Paint),
    ) {
        let r = theme.roles();
        let Some(binding) = self.rows.get(row) else { return };
        let width = usize::from(area.width);
        for x in area.left()..area.right() {
            buf[(x, area.y)].set_symbol(" ").set_style(style);
        }
        let changed = !binding.is_default();
        if changed {
            let accent = style.fg(theme.color(r.brand.accent, ink.1));
            buf.set_stringn(area.x, area.y, "*", width, accent);
        }
        let mut x = MARKER_WIDTH + 1;
        let put = |buf: &mut Buffer, at: usize, text: &str, cells: usize, style: Style| {
            let Some(room) = width.checked_sub(at).filter(|&room| room > 0) else {
                return;
            };
            let cells = cells.min(room);
            // `at` is within the area's width, a u16.
            let column = area.x + u16::try_from(at).unwrap_or(u16::MAX);
            buf.set_stringn(column, area.y, display::truncate(text, cells), cells, style);
        };
        put(buf, x, &binding.action(), self.action_width, style);
        x += self.action_width + GAP;
        if self.capture.is_some() && self.highlighted == Some(row) {
            let prompt = Style::new()
                .fg(theme.color(r.brand.accent, ink.1))
                .bg(theme.paint(ink.1))
                .add_modifier(Modifier::REVERSED);
            put(buf, x, PRESS_A_KEY, KEYS_WIDTH, prompt);
        } else {
            let keys = if changed {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            };
            put(buf, x, &binding.keys_display(), KEYS_WIDTH, keys);
        }
        x += KEYS_WIDTH + GAP;
        if width.saturating_sub(x) >= MIN_ID_WIDTH {
            let dim = theme.rich_dim(style, ink.0, ink.1);
            put(buf, x, binding.config_id, width - x, dim);
        }
    }
}

/// The highlighted line's style, with the theme color of its text and its background: a focused `OptionList`'s
/// `$block-cursor-*`, or the blurred cursor's background with `$foreground` text.
fn highlight_ink(theme: &Theme, under: Paint, focused: bool) -> (Style, ThemeColor, Paint) {
    let r = theme.roles();
    let (cursor, fg) = if focused {
        (r.cursor.focused, r.cursor.focused.fg)
    } else {
        (r.cursor.blurred, r.text.plain)
    };
    let bg = cursor.bg.over(under);
    let style = cursor
        .style
        .apply(Style::new().fg(theme.color(fg, bg)).bg(theme.paint(bg)));
    (style, fg, bg)
}

/// The list's own scrollbar in `bar`, in `$scrollbar` and `$scrollbar-background` over `under`.
fn draw_scrollbar(buf: &mut Buffer, bar: Rect, theme: &Theme, under: Paint, state: ScrollbarState) {
    let r = theme.roles();
    let back = r.scrollbar.track.over(under);
    let colors = ScrollbarColors {
        bar: theme.color(r.scrollbar.thumb, back),
        back: theme.paint(back),
        fg: theme.color(r.text.plain, under),
        bg: theme.paint(under),
    };
    scrollbar::render(buf, bar, Axis::Vertical, state, colors);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    fn list() -> KeyList {
        let mut list = KeyList::new(&IndexMap::new());
        list.view = 10;
        list
    }

    fn highlighted(list: &KeyList) -> BindingId {
        list.highlighted().expect("a highlight").id
    }

    fn chord(token: &str) -> KeyChord {
        match KeySpec::parse(token) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token} isn't a key"),
        }
    }

    fn value(list: &KeyList, id: BindingId) -> &str {
        &list.rows.iter().find(|row| row.id == id).expect("a row").value
    }

    #[test]
    fn the_lines_are_every_group_under_its_header_with_the_first_binding_highlighted() {
        let list = list();
        // Python's 59 ids and the six Rust-only diff ids.
        assert_eq!(list.lines.len(), 65 + BINDING_GROUPS.len());
        assert_eq!(list.lines[0], Line::Header(0));
        assert_eq!(list.lines[1], Line::Binding(0));
        assert_eq!(highlighted(&list), BindingId::AppQuit);
    }

    #[test]
    fn moves_skip_headers_and_stop_at_the_ends() {
        let mut list = list();
        let _ = list.on_binding(BindingId::TextualOptionListCursorUp);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
        for _ in 0..8 {
            let _ = list.on_binding(BindingId::TextualOptionListCursorDown);
        }
        assert_eq!(highlighted(&list), BindingId::UserViewProfile);
        // Past the Focus header.
        let _ = list.on_binding(BindingId::TextualOptionListCursorDown);
        assert_eq!(highlighted(&list), BindingId::MainReposFocus);
        let _ = list.on_binding(BindingId::TextualOptionListCursorUp);
        assert_eq!(highlighted(&list), BindingId::UserViewProfile);
        let _ = list.on_binding(BindingId::TextualOptionListLast);
        assert_eq!(highlighted(&list), BindingId::ConfirmNo);
        let _ = list.on_binding(BindingId::TextualOptionListCursorDown);
        assert_eq!(highlighted(&list), BindingId::ConfirmNo);
        let _ = list.on_binding(BindingId::TextualOptionListFirst);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
    }

    #[test]
    fn page_keys_move_by_the_lines_in_view() {
        let mut list = list();
        // Quit is line 1, so a page of 10 lands on line 11, Focus repos table, past the Focus header.
        let _ = list.on_binding(BindingId::TextualOptionListPageDown);
        assert_eq!(highlighted(&list), BindingId::MainReposFocus);
        assert!(list.scroll > 0);
        let _ = list.on_binding(BindingId::TextualOptionListPageUp);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
        assert_eq!(list.scroll, 0);
        let _ = list.on_binding(BindingId::TextualOptionListLast);
        let _ = list.on_binding(BindingId::TextualOptionListPageDown);
        assert_eq!(highlighted(&list), BindingId::ConfirmNo);
        let _ = list.on_binding(BindingId::TextualOptionListFirst);
        let _ = list.on_binding(BindingId::TextualOptionListPageUp);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
    }

    #[test]
    fn a_page_onto_a_header_stops_at_the_binding_before_it() {
        let mut list = list();
        // Quit is line 1; line 10 is the Focus header.
        list.view = 9;
        let _ = list.on_binding(BindingId::TextualOptionListPageDown);
        assert_eq!(highlighted(&list), BindingId::UserViewProfile);
    }

    fn on_help() -> KeyList {
        let mut list = list();
        for _ in 0..3 {
            let _ = list.on_binding(BindingId::TextualOptionListCursorDown);
        }
        assert_eq!(highlighted(&list), BindingId::AppOpenHelp);
        list
    }

    #[test]
    fn replace_sets_the_recorded_key_alone() {
        let keymap = Keymap::defaults();
        let mut list = on_help();
        let _ = list.on_binding(BindingId::SettingsKeysChange);
        assert!(list.is_capturing());
        let _ = list.capture(&chord("!"), &keymap);
        assert!(!list.is_capturing());
        assert_eq!(value(&list, BindingId::AppOpenHelp), "exclamation_mark");
    }

    #[test]
    fn add_appends_the_recorded_key_once() {
        let keymap = Keymap::defaults();
        let mut list = on_help();
        let _ = list.on_binding(BindingId::SettingsKeysAdd);
        let _ = list.capture(&chord("!"), &keymap);
        assert_eq!(value(&list, BindingId::AppOpenHelp), "?, exclamation_mark");
        let _ = list.on_binding(BindingId::SettingsKeysAdd);
        let _ = list.capture(&chord("!"), &keymap);
        assert!(!list.is_capturing());
        assert_eq!(value(&list, BindingId::AppOpenHelp), "?, exclamation_mark");
    }

    #[test]
    fn an_unnamed_key_is_ignored_and_the_wait_goes_on() {
        let keymap = Keymap::defaults();
        let mut list = on_help();
        let _ = list.on_binding(BindingId::SettingsKeysChange);
        let _ = list.capture(&KeyChord::new(KeyCode::CapsLock, KeyModifiers::NONE), &keymap);
        assert!(list.is_capturing());
        assert_eq!(value(&list, BindingId::AppOpenHelp), "?");
    }

    #[test]
    fn escape_cancels_and_tab_is_recorded() {
        let keymap = Keymap::defaults();
        let mut list = on_help();
        let _ = list.on_binding(BindingId::SettingsKeysChange);
        let _ = list.capture(&chord("escape"), &keymap);
        assert!(!list.is_capturing());
        assert_eq!(value(&list, BindingId::AppOpenHelp), "?");
        let _ = list.on_binding(BindingId::SettingsKeysChange);
        let _ = list.capture(&chord("tab"), &keymap);
        assert_eq!(value(&list, BindingId::AppOpenHelp), "tab");
        let _ = list.on_binding(BindingId::SettingsKeysAdd);
        let _ = list.capture(&chord("shift+tab"), &keymap);
        assert_eq!(value(&list, BindingId::AppOpenHelp), "tab, shift+tab");
    }

    #[test]
    fn reset_puts_the_defaults_back() {
        let mut overrides = IndexMap::new();
        overrides.insert("app.quit".to_owned(), "x".to_owned());
        let mut list = KeyList::new(&overrides);
        assert_eq!(list.detail(), "app.quit · default q · changed");
        let _ = list.on_binding(BindingId::SettingsKeysReset);
        assert_eq!(value(&list, BindingId::AppQuit), "q");
        assert_eq!(list.detail(), "app.quit · default q");
    }

    #[test]
    fn the_detail_line_says_what_the_wait_is_for() {
        let mut list = on_help();
        assert_eq!(list.detail(), "app.open_help · default ?");
        let _ = list.on_binding(BindingId::SettingsKeysChange);
        assert_eq!(list.detail(), "Press the new key for Keybindings. esc cancels.");
        list.cancel_capture();
        let _ = list.on_binding(BindingId::SettingsKeysAdd);
        assert_eq!(list.detail(), "Press a key to add to Keybindings. esc cancels.");
    }

    #[test]
    fn the_filter_hides_empty_groups_and_moves_a_hidden_highlight() {
        let mut list = on_help();
        list.filter(Some("lookup"));
        let headers: Vec<&str> = list
            .lines
            .iter()
            .filter_map(|line| match line {
                Line::Header(group) => Some(BINDING_GROUPS[*group].title),
                Line::Binding(_) => None,
            })
            .collect();
        assert_eq!(headers, ["Repositories", "Pull requests", "Issues"]);
        assert_eq!(highlighted(&list), BindingId::RepositoriesLookup);
        list.filter(Some("nothing matches this"));
        assert!(!list.has_lines());
        assert!(list.lines.is_empty());
        list.filter(None);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
    }

    #[test]
    fn a_kept_highlight_stays_through_the_filter() {
        let mut list = on_help();
        list.filter(Some("keybindings"));
        assert_eq!(highlighted(&list), BindingId::AppOpenHelp);
        list.filter(None);
        assert_eq!(highlighted(&list), BindingId::AppOpenHelp);
    }

    #[test]
    fn the_wheel_scrolls_without_moving_the_highlight() {
        let mut list = list();
        list.wheel(Wheel::Down);
        assert_eq!(list.scroll, WHEEL_ROWS);
        assert_eq!(highlighted(&list), BindingId::AppQuit);
        list.wheel(Wheel::Up);
        list.wheel(Wheel::Up);
        assert_eq!(list.scroll, 0);
    }
}
