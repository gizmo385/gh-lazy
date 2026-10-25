//! A single-line text input: Textual's `Input` with its default look (a tall border, `padding: 0 2`) and its editing
//! keys, minus selection and the clipboard.
//!
//! `tui-textarea` (PLAN §4) is for the multi-line editors. A one-line box is small enough to own, which lets it match
//! Textual's styling and word rules exactly; the M3 lookup modals can reuse it.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::components::table::is_printable;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{AnsiColor, Paint, Theme};

/// The input's bindings, in Textual's `Input.BINDINGS` order. Selection (`shift+…`, `ctrl+shift+a`) and the
/// clipboard (`ctrl+x`, `ctrl+c`, `ctrl+v`) come with the M3 shared input.
pub const INPUT_BINDINGS: &[BindingId] = &[
    BindingId::TextualInputCursorLeft,
    BindingId::TextualInputCursorLeftWord,
    BindingId::TextualInputCursorRight,
    BindingId::TextualInputCursorRightWord,
    BindingId::TextualInputDeleteLeft,
    BindingId::TextualInputHome,
    BindingId::TextualInputEnd,
    BindingId::TextualInputDeleteRight,
    BindingId::TextualInputSubmit,
    BindingId::TextualInputDeleteLeftWord,
    BindingId::TextualInputDeleteLeftAll,
    BindingId::TextualInputDeleteRightWord,
    BindingId::TextualInputDeleteRightAll,
];

/// Rows the input takes: the text between the top and bottom border.
pub const HEIGHT: u16 = 3;

/// Cells between each side border and the text (`padding: 0 2`).
const PADDING: u16 = 2;

/// What a key did to the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum InputOutcome {
    /// Taken: the text or the cursor may have changed.
    Changed,
    /// `enter`: the owner acts on [`TextInput::value`].
    Submitted,
    /// Not the input's: the owner passes it on.
    Ignored,
}

/// The input's text and cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    /// A char index into `value`, from 0 to its length.
    cursor: usize,
    placeholder: &'static str,
    /// The first text cell shown, which follows the cursor.
    scroll: usize,
}

impl TextInput {
    /// An empty input showing `placeholder`.
    pub fn new(placeholder: &'static str) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            placeholder,
            scroll: 0,
        }
    }

    /// The text.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The cursor, as a char index.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replaces the text and puts the cursor at its end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.len();
    }

    /// Whether the input takes `key` before any binding: printable characters, which Textual's `Input` consumes
    /// (`check_consume_key`), and its own bindings, which are innermost in the chain.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        printable(key).is_some() || keymap.match_in(INPUT_BINDINGS, key).is_some()
    }

    /// A key: a printable character is typed, an editing key edits, `enter` submits, and anything else is
    /// [`InputOutcome::Ignored`].
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap) -> InputOutcome {
        if let Some(c) = printable(key) {
            self.insert(c.encode_utf8(&mut [0; 4]));
            return InputOutcome::Changed;
        }
        match keymap.match_in(INPUT_BINDINGS, key) {
            Some(id) => self.on_binding(id),
            None => InputOutcome::Ignored,
        }
    }

    fn on_binding(&mut self, id: BindingId) -> InputOutcome {
        use BindingId as B;
        let len = self.len();
        match id {
            B::TextualInputSubmit => return InputOutcome::Submitted,
            B::TextualInputCursorLeft => self.cursor = self.cursor.saturating_sub(1),
            B::TextualInputCursorRight => self.cursor = (self.cursor + 1).min(len),
            B::TextualInputCursorLeftWord => self.cursor = self.word_start_before(),
            B::TextualInputCursorRightWord => self.cursor = self.word_start_after().unwrap_or(len),
            B::TextualInputHome => self.cursor = 0,
            B::TextualInputEnd => self.cursor = len,
            B::TextualInputDeleteLeft => {
                if self.cursor > 0 {
                    self.delete(self.cursor - 1, self.cursor);
                }
            }
            B::TextualInputDeleteRight => self.delete(self.cursor, (self.cursor + 1).min(len)),
            B::TextualInputDeleteLeftWord => self.delete(self.word_start_before(), self.cursor),
            // Up to the start of the next word, or everything when there's none.
            B::TextualInputDeleteRightWord => self.delete(self.cursor, self.word_start_after().unwrap_or(len)),
            B::TextualInputDeleteLeftAll => self.delete(0, self.cursor),
            B::TextualInputDeleteRightAll => self.delete(self.cursor, len),
            _ => return InputOutcome::Ignored,
        }
        InputOutcome::Changed
    }

    /// A bracketed paste: only its first line goes in, as Textual's `Input._on_paste` does with `splitlines()[0]`.
    pub fn handle_paste(&mut self, text: &str) {
        let line = text.split(is_line_break).next().unwrap_or_default();
        self.insert(line);
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    fn byte_at(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map_or(self.value.len(), |(i, _)| i)
    }

    fn insert(&mut self, text: &str) {
        let at = self.byte_at(self.cursor);
        self.value.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    /// Deletes the chars in `start..end` and leaves the cursor at `start`.
    fn delete(&mut self, start: usize, end: usize) {
        let (from, to) = (self.byte_at(start), self.byte_at(end));
        self.value.replace_range(from..to, "");
        self.cursor = start;
    }

    /// Textual's `_WORD_START`, `(?<=\W)\w`, over the chars before the cursor: the last word start, or 0.
    fn word_start_before(&self) -> usize {
        let chars: Vec<char> = self.value.chars().take(self.cursor).collect();
        (1..chars.len())
            .rev()
            .find(|&i| !is_word(chars[i - 1]) && is_word(chars[i]))
            .unwrap_or(0)
    }

    /// The first word start after the cursor. Textual searches the text from the cursor on as a new string, so a
    /// word starting right at the cursor never counts.
    fn word_start_after(&self) -> Option<usize> {
        let chars: Vec<char> = self.value.chars().collect();
        (self.cursor + 1..chars.len()).find(|&i| !is_word(chars[i - 1]) && is_word(chars[i]))
    }

    /// Draws the input into the top [`HEIGHT`] rows of `area`, over `under` (what the parent painted).
    ///
    /// The cursor is always drawn while focused: Textual blinks it, but the blink is a timer the snapshots and the
    /// parity harness can't pin.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: bool) {
        let area = Rect {
            height: area.height.min(HEIGHT),
            ..area
        };
        if area.is_empty() {
            return;
        }
        let styles = Styles::new(theme, focused);
        draw_tall_border(buf, area, &styles);

        // The text row, between the border and the padding.
        if area.height < 2 || area.width <= 2 * (PADDING + 1) {
            return;
        }
        let text = Rect {
            x: area.x + 1 + PADDING,
            y: area.y + 1,
            width: area.width - 2 * (PADDING + 1),
            height: 1,
        };
        let width = usize::from(text.width);
        let mut cells: Vec<(char, Style)> = Vec::new();
        if self.value.is_empty() {
            self.scroll = 0;
            cells.extend(self.placeholder.chars().map(|c| (c, styles.placeholder)));
            if focused {
                match cells.first_mut() {
                    Some(first) => first.1 = styles.cursor_over(first.1),
                    None => cells.push((' ', styles.cursor_over(styles.text))),
                }
            }
        } else {
            cells.extend(self.value.chars().map(|c| (c, styles.text)));
            if focused {
                if self.cursor == cells.len() {
                    cells.push((' ', styles.text));
                }
                if let Some(cell) = cells.get_mut(self.cursor) {
                    cell.1 = styles.cursor_over(cell.1);
                }
            }
            // Scroll so the cursor's cell is in view, as Textual's `scroll_to_region` does on every move.
            let before: usize = cells.iter().take(self.cursor).map(|&(c, _)| char_width(c)).sum();
            let at = cells.get(self.cursor).map_or(1, |&(c, _)| char_width(c));
            if before < self.scroll {
                self.scroll = before;
            } else if before + at > self.scroll + width {
                self.scroll = before + at - width;
            }
        }
        let line: Line<'_> = cells
            .into_iter()
            .map(|(c, style)| Span::styled(c.to_string(), style))
            .collect();
        Paragraph::new(line)
            .style(styles.text)
            .scroll((0, u16::try_from(self.scroll).unwrap_or(u16::MAX)))
            .render(text, buf);
    }
}

/// The character a key types, when it's printable: no CTRL or ALT, and printable by Python's `str.isprintable`.
fn printable(key: &KeyChord) -> Option<char> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Char(c) if !key.mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && is_printable(c) => {
            Some(c)
        }
        _ => None,
    }
}

/// Python's `\w` in a `str` pattern: alphanumerics and the underscore.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The line boundaries Python's `str.splitlines` splits on (`\r\n` splits at its `\r` first, which is enough for
/// taking the first line).
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn char_width(c: char) -> usize {
    Span::raw(c.encode_utf8(&mut [0; 4]) as &str).width()
}

/// The input's styles for one frame, from `Input.DEFAULT_CSS`.
struct Styles {
    /// The text and the padding: `$foreground` on `$surface`, tinted `$foreground 5%` while focused.
    text: Style,
    placeholder: Style,
    cursor: Style,
    /// `▊` on the left: the parent's background over the border color, reversed (Textual's border location 2).
    left: Style,
    /// `▔`, `▁` and the blank between: the border color over the input's background (location 0).
    inner: Style,
    /// `▎` on the right: the border color over the parent's background (location 1).
    right: Style,
}

impl Styles {
    fn new(theme: &Theme, focused: bool) -> Self {
        let p = theme.palette;
        let under = theme.background();
        if theme.ansi {
            // `&:ansi`: the terminal's own colors.
            let default = theme.paint(Paint::Ansi(AnsiColor::Default));
            let text = Style::new().fg(default).bg(default);
            let border = theme.color(if focused { p.border } else { p.border_blurred }, under);
            return Self {
                text,
                placeholder: text.add_modifier(Modifier::DIM),
                cursor: Style::new()
                    .fg(theme.paint(Paint::Ansi(AnsiColor::Black)))
                    .bg(theme.paint(Paint::Ansi(AnsiColor::White))),
                left: Style::new().fg(border).bg(default).add_modifier(Modifier::REVERSED),
                inner: Style::new().fg(border).bg(default),
                right: Style::new().fg(border).bg(default),
            };
        }
        let surface = p.surface.over(under);
        let bg = if focused {
            theme.tint(surface, p.foreground, 5.0)
        } else {
            surface
        };
        let text = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
        let border = theme.color(if focused { p.border } else { p.border_blurred }, bg);
        let cursor_bg = p.input_cursor_background.over(bg);
        let cursor = p.input_cursor_text_style.apply(
            Style::new()
                .fg(theme.color(p.input_cursor_foreground, cursor_bg))
                .bg(theme.paint(cursor_bg)),
        );
        Self {
            text,
            placeholder: text.fg(theme.color(p.text_disabled, bg)),
            cursor,
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
        }
    }

    /// The cursor drawn over a cell: Textual stylizes the cursor's cell with `input--cursor`, whose colors win.
    fn cursor_over(&self, cell: Style) -> Style {
        cell.patch(self.cursor)
    }
}

/// Textual's `tall` border: `▊▔▎` / `▊ ▎` / `▊▁▎`, with the inside filled with the text style.
fn draw_tall_border(buf: &mut Buffer, area: Rect, styles: &Styles) {
    let last_row = area.height - 1;
    for dy in 0..area.height {
        let y = area.y + dy;
        let fill = if dy == 0 {
            "▔"
        } else if dy == last_row && area.height == HEIGHT {
            "▁"
        } else {
            " "
        };
        for dx in 0..area.width {
            let x = area.x + dx;
            let (symbol, style) = if dx == 0 {
                ("▊", styles.left)
            } else if dx == area.width - 1 {
                ("▎", styles.right)
            } else if fill == " " {
                (" ", styles.text)
            } else {
                (fill, styles.inner)
            };
            buf[(x, y)].set_symbol(symbol).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::keymap::KeySpec;

    fn chord(key: &str) -> KeyChord {
        match KeySpec::parse(key) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
        }
    }

    /// An input holding `value` with the cursor at char `cursor`.
    fn input(value: &str, cursor: usize) -> TextInput {
        let mut input = TextInput::new("Search...");
        input.set_value(value);
        input.cursor = cursor;
        input
    }

    fn press(input: &mut TextInput, key: &str) -> InputOutcome {
        input.handle_key(&chord(key), &Keymap::defaults())
    }

    #[test]
    fn printable_keys_type_at_the_cursor() {
        let mut input = TextInput::new("Search...");
        for key in ["q", "j", "slash", "space", "J", "question_mark"] {
            assert_eq!(press(&mut input, key), InputOutcome::Changed);
        }
        assert_eq!(input.value(), "qj/ J?");
        assert_eq!(input.cursor(), 6);
        let mut middle = self::input("ac", 1);
        let _ = press(&mut middle, "b");
        assert_eq!((middle.value(), middle.cursor()), ("abc", 2));
    }

    #[test]
    fn non_text_keys_are_ignored() {
        let mut input = input("abc", 1);
        for key in [
            "tab",
            "shift+tab",
            "escape",
            "ctrl+p",
            "f1",
            "up",
            "down",
            "ctrl+v",
            "alt+x",
        ] {
            assert_eq!(press(&mut input, key), InputOutcome::Ignored, "{key}");
        }
        assert_eq!((input.value(), input.cursor()), ("abc", 1));
        let keymap = Keymap::defaults();
        assert!(!input.consumes_key(&chord("tab"), &keymap));
        assert!(input.consumes_key(&chord("q"), &keymap));
        assert!(input.consumes_key(&chord("ctrl+f"), &keymap));
        assert!(input.consumes_key(&chord("enter"), &keymap));
    }

    #[test]
    fn enter_submits_without_changing_the_text() {
        let mut input = input("abc", 1);
        assert_eq!(press(&mut input, "enter"), InputOutcome::Submitted);
        assert_eq!((input.value(), input.cursor()), ("abc", 1));
    }

    #[rstest]
    #[case::left("left", 3, 2)]
    #[case::left_at_start("left", 0, 0)]
    #[case::right("right", 3, 4)]
    #[case::right_at_end("right", 11, 11)]
    #[case::home("home", 5, 0)]
    #[case::ctrl_a("ctrl+a", 5, 0)]
    #[case::end("end", 5, 11)]
    #[case::ctrl_e("ctrl+e", 5, 11)]
    // `foo-bar baz`: words start at 0, 4 and 8.
    #[case::word_left_from_end("ctrl+left", 11, 8)]
    #[case::word_left_from_word_start("ctrl+left", 8, 4)]
    #[case::word_left_inside_word("ctrl+left", 6, 4)]
    #[case::word_left_to_start("ctrl+left", 4, 0)]
    #[case::word_left_at_start("ctrl+left", 0, 0)]
    #[case::word_right_from_start("ctrl+right", 0, 4)]
    #[case::word_right_from_dash("ctrl+right", 3, 4)]
    #[case::word_right_from_word_start("ctrl+right", 4, 8)]
    #[case::word_right_to_end("ctrl+right", 8, 11)]
    fn cursor_keys_move(#[case] key: &str, #[case] from: usize, #[case] to: usize) {
        let mut input = input("foo-bar baz", from);
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!(input.cursor(), to);
        assert_eq!(input.value(), "foo-bar baz");
    }

    #[rstest]
    #[case::backspace("backspace", 4, "foobar baz", 3)]
    #[case::backspace_at_start("backspace", 0, "foo-bar baz", 0)]
    #[case::delete("delete", 3, "foobar baz", 3)]
    #[case::ctrl_d("ctrl+d", 3, "foobar baz", 3)]
    #[case::delete_at_end("delete", 11, "foo-bar baz", 11)]
    #[case::word_left("ctrl+w", 11, "foo-bar ", 8)]
    #[case::word_left_inside_word("ctrl+w", 6, "foo-r baz", 4)]
    #[case::word_left_from_word_start("ctrl+w", 8, "foo-baz", 4)]
    #[case::word_left_to_start("ctrl+w", 3, "-bar baz", 0)]
    #[case::word_right("ctrl+f", 0, "bar baz", 0)]
    #[case::word_right_inside_word("ctrl+f", 5, "foo-bbaz", 5)]
    #[case::word_right_from_word_start("ctrl+f", 4, "foo-baz", 4)]
    #[case::word_right_without_next_word("ctrl+f", 9, "foo-bar b", 9)]
    #[case::all_left("ctrl+u", 4, "bar baz", 0)]
    #[case::all_right("ctrl+k", 4, "foo-", 4)]
    fn delete_keys_delete(#[case] key: &str, #[case] from: usize, #[case] value: &str, #[case] cursor: usize) {
        let mut input = input("foo-bar baz", from);
        assert_eq!(press(&mut input, key), InputOutcome::Changed);
        assert_eq!((input.value(), input.cursor()), (value, cursor));
    }

    #[test]
    fn editing_counts_chars_not_bytes() {
        let mut input = input("héllo wörld", 11);
        let _ = press(&mut input, "ctrl+w");
        assert_eq!((input.value(), input.cursor()), ("héllo ", 6));
        let _ = press(&mut input, "left");
        let _ = press(&mut input, "backspace");
        assert_eq!((input.value(), input.cursor()), ("héll ", 4));
    }

    #[rstest]
    #[case::one_line("lazy", "xlazyy")]
    #[case::multi_line("lazy\ngithub\nmore", "xlazyy")]
    #[case::crlf("lazy\r\ngithub", "xlazyy")]
    #[case::leading_newline("\nlazy", "xy")]
    #[case::unicode_line_separator("lazy\u{2028}github", "xlazyy")]
    fn paste_inserts_only_the_first_line(#[case] pasted: &str, #[case] value: &str) {
        let mut input = input("xy", 1);
        input.handle_paste(pasted);
        assert_eq!(input.value(), value);
        assert_eq!(input.cursor(), value.chars().count() - 1);
    }
}
