//! A compose sheet's body: the editor, or its markdown rendering in the same place, under a `Write  Preview` tabs row.

use std::cell::RefCell;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use tokio::time::Instant;

use super::quote::trim_blank_tail;
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::markdown::{LinkSpan, Markdown, Rendered};
use crate::components::scroll_view::LayoutCache;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState, VERTICAL_WIDTH};
use crate::components::text_editor::{
    EditorBorder, EditorOutcome, EditorPadding, EditorVariant, TEXT_EDITOR_BINDINGS, TextEditor,
};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::{Paint, Theme};

/// The tabs, as drawn: a cell of padding each side, so the active one's highlight frames its name.
const WRITE_TAB: &str = " Write ";
const PREVIEW_TAB: &str = " Preview ";
/// The columns each side of the body's text.
const PADDING: u16 = 1;

/// What the body shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    /// The editor.
    Write,
    /// The text's markdown rendering, in the editor's place.
    Preview,
}

/// What a mouse event did to the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMouse {
    /// A press on a tab or in the body, which focuses the body (and, writing, moves the cursor there).
    Focused,
    /// A drag or a release the editor's selection took, or a wheel notch that scrolled the body.
    Handled,
    /// Nothing of the body's: a wheel notch it couldn't scroll further, or a press elsewhere.
    Missed,
}

/// The editor and its preview. See the module docs.
#[derive(Debug)]
pub struct ComposeBody {
    editor: TextEditor,
    mode: BodyMode,
    /// Dim in the editor while it's empty.
    placeholder: &'static str,
    /// The fewest rows the body takes.
    min_rows: u16,
    /// The text as markdown, parsed when the preview is shown: the text can't change while it is.
    preview: Markdown,
    /// The preview's layouts by width and theme: measured inside the padding, drawn beside a scrollbar once it
    /// overflows. Its background is the body's, which follows from the theme.
    preview_layout: RefCell<LayoutCache>,
    /// The preview's first row shown, and its rows and window at the last draw, which scrolling is clamped to.
    preview_offset: usize,
    preview_size: (usize, usize),
    /// Where the two tabs and the body were last drawn, for the mouse.
    tabs: [Rect; 2],
    area: Rect,
}

impl ComposeBody {
    /// An empty body: a `variant` editor with no border and a column of padding each side, at least `min_rows` tall,
    /// showing `placeholder` while empty.
    pub fn new(variant: EditorVariant, placeholder: &'static str, min_rows: u16) -> Self {
        Self {
            editor: TextEditor::new(variant, "")
                .with_border(EditorBorder::None)
                .with_padding(EditorPadding {
                    top: 0,
                    horizontal: PADDING,
                }),
            mode: BodyMode::Write,
            placeholder,
            min_rows,
            preview: Markdown::default(),
            preview_layout: RefCell::new(LayoutCache::default()),
            preview_offset: 0,
            preview_size: (0, 0),
            tabs: [Rect::default(); 2],
            area: Rect::default(),
        }
    }

    /// The same body holding `text` (a prefilled template, or the description being edited), the cursor at its
    /// start.
    #[must_use]
    pub fn with_text(mut self, text: &str) -> Self {
        self.editor.set_text(text);
        self
    }

    /// Replaces the text of an open body, as a restored draft does: the editor starts over at its top with no undo
    /// history, and a preview shown is parsed again.
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
        if self.mode == BodyMode::Preview {
            self.preview = Markdown::parse(text);
            self.preview_layout.borrow_mut().clear();
            self.preview_offset = 0;
        }
    }

    /// The text as typed.
    pub fn text(&self) -> String {
        self.editor.text()
    }

    /// The editor, whose cursor and selection the preview keeps.
    pub fn editor(&self) -> &TextEditor {
        &self.editor
    }

    /// What the body shows.
    pub fn mode(&self) -> BodyMode {
        self.mode
    }

    /// The fewest rows the body takes.
    pub fn min_rows(&self) -> u16 {
        self.min_rows
    }

    /// Shows `mode`. The preview is parsed from the text as it is and starts at its top; the editor comes back with
    /// its text, cursor and selection as they were.
    pub fn set_mode(&mut self, mode: BodyMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        if mode == BodyMode::Preview {
            self.preview = Markdown::parse(&self.editor.text());
            self.preview_layout.borrow_mut().clear();
            self.preview_offset = 0;
        }
    }

    /// Swaps Write and Preview.
    pub fn toggle_mode(&mut self) {
        self.set_mode(match self.mode {
            BodyMode::Write => BodyMode::Preview,
            BodyMode::Preview => BodyMode::Write,
        });
    }

    /// The editor gained focus: its next edit starts a new undo batch.
    pub fn focus_gained(&mut self) {
        self.editor.focus_gained();
    }

    /// The rows the body needs to show everything `width` cells wide without scrolling, at least its minimum: the
    /// editor's wrapped rows, or the preview's.
    pub fn rows_needed(&self, width: u16, theme: &Theme, under: Paint) -> u16 {
        let rows = match self.mode {
            BodyMode::Write => self.editor.content_rows(width),
            BodyMode::Preview => {
                let background = theme.roles().surface.pane.over(under);
                self.with_preview(text_width(width), theme, background, |rendered| rendered.lines.len())
            }
        };
        u16::try_from(rows).unwrap_or(u16::MAX).max(self.min_rows)
    }

    /// Whether the body takes `key` before any binding. The preview takes what the editor would, so a printable key
    /// does nothing there rather than reach the dialog's bindings, and the editor's movement keys scroll it.
    pub fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.editor.consumes_key(key, keymap)
    }

    /// A key the body consumed, at `now`. The editor edits; the preview scrolls with the editor's up, down, page and
    /// line start and end keys, and ignores the rest.
    pub fn handle_key(&mut self, key: &KeyChord, keymap: &Keymap, clipboard: &str, now: Instant) -> EditorOutcome {
        match self.mode {
            BodyMode::Write => self.editor.handle_key(key, keymap, clipboard, now),
            BodyMode::Preview => {
                let window = self.preview_size.1;
                let target = match keymap.match_in(TEXT_EDITOR_BINDINGS, key) {
                    Some(BindingId::TextualTextAreaCursorUp) => self.preview_offset.saturating_sub(1),
                    Some(BindingId::TextualTextAreaCursorDown) => self.preview_offset + 1,
                    Some(BindingId::TextualTextAreaCursorPageUp) => self.preview_offset.saturating_sub(window),
                    Some(BindingId::TextualTextAreaCursorPageDown) => self.preview_offset + window,
                    Some(BindingId::TextualTextAreaCursorLineStart) => 0,
                    Some(BindingId::TextualTextAreaCursorLineEnd) => usize::MAX,
                    _ => return EditorOutcome::Ignored,
                };
                if self.scroll_preview_to(target) {
                    EditorOutcome::Moved
                } else {
                    EditorOutcome::Ignored
                }
            }
        }
    }

    /// A bracketed paste: into the editor, or nowhere while previewing.
    pub fn handle_paste(&mut self, text: &str, now: Instant) -> EditorOutcome {
        match self.mode {
            BodyMode::Write => self.editor.handle_paste(text, now),
            BodyMode::Preview => EditorOutcome::Ignored,
        }
    }

    /// Whether a press in the editor started a drag that hasn't been released.
    pub fn is_selecting(&self) -> bool {
        self.editor.is_selecting()
    }

    /// A mouse event, in the coordinates the body was last drawn in: a press on a tab shows it, a press in the body
    /// moves the editor's cursor there, a drag selects, and the wheel scrolls.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> BodyMouse {
        if self.editor.captured_mouse(event) {
            return BodyMouse::Handled;
        }
        let at = (event.column, event.row).into();
        if let Some(wheel) = Wheel::from_mouse(&event) {
            if !self.area.contains(at) {
                return BodyMouse::Missed;
            }
            let rows = match wheel {
                Wheel::Up => -WHEEL_ROWS.cast_signed(),
                Wheel::Down => WHEEL_ROWS.cast_signed(),
                Wheel::Left | Wheel::Right => return BodyMouse::Missed,
            };
            let scrolled = match self.mode {
                BodyMode::Write => self.editor.scroll(rows),
                BodyMode::Preview => self.scroll_preview_to(self.preview_offset.saturating_add_signed(rows)),
            };
            return if scrolled {
                BodyMouse::Handled
            } else {
                BodyMouse::Missed
            };
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return BodyMouse::Missed;
        }
        if let Some(mode) = [BodyMode::Write, BodyMode::Preview]
            .into_iter()
            .zip(self.tabs)
            .find_map(|(mode, tab)| tab.contains(at).then_some(mode))
        {
            self.set_mode(mode);
            return BodyMouse::Focused;
        }
        if !self.area.contains(at) {
            return BodyMouse::Missed;
        }
        if self.mode == BodyMode::Write {
            self.editor.press(event.column, event.row);
        }
        BodyMouse::Focused
    }

    /// Draws the tabs row into `tabs` and the body into `body`, over `under` (the sheet's background); the editor's
    /// cursor shows while `focused`. Returns the preview's links in view, in rows and columns from `body`'s top left.
    pub fn render(
        &mut self,
        buf: &mut Buffer,
        tabs: Rect,
        body: Rect,
        theme: &Theme,
        under: Paint,
        focused: bool,
    ) -> Vec<LinkSpan> {
        self.render_tabs(buf, tabs, theme, under);
        self.area = body;
        match self.mode {
            BodyMode::Write => {
                self.editor.render(buf, body, theme, under, focused);
                if self.editor.is_empty()
                    && let Some((x, y)) = self.editor.text_origin()
                {
                    // Only the color, over the plain editor's background: the cursor's cell keeps its highlight.
                    let background = theme.roles().surface.pane.over(under);
                    let dim = Style::new().fg(theme.color(theme.roles().text.disabled, background));
                    let room = body.right().saturating_sub(x + PADDING);
                    buf.set_stringn(x, y, self.placeholder, usize::from(room), dim);
                }
                Vec::new()
            }
            BodyMode::Preview => self.render_preview(buf, body, theme, under),
        }
    }

    /// `Write  Preview`, the active one reversed, then a rule to the right edge.
    fn render_tabs(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint) {
        let roles = theme.roles();
        let plain = Style::new().fg(theme.color(roles.text.plain, under));
        let style = |mode: BodyMode| {
            if mode == self.mode {
                plain.add_modifier(Modifier::REVERSED)
            } else {
                Style::new().fg(theme.color(roles.text.muted, under))
            }
        };
        let mut x = area.x;
        let mut targets = [Rect::default(); 2];
        for (target, (mode, label)) in targets
            .iter_mut()
            .zip([(BodyMode::Write, WRITE_TAB), (BodyMode::Preview, PREVIEW_TAB)])
        {
            let (next, _) = buf.set_span(
                x,
                area.y,
                &Span::styled(label, style(mode)),
                area.right().saturating_sub(x),
            );
            *target = Rect::new(x, area.y, next - x, area.height.min(1));
            x = (next + 1).min(area.right());
        }
        self.tabs = targets;
        let rule = Style::new().fg(theme.color(roles.border.rule, under));
        let width = area.right().saturating_sub(x);
        buf.set_stringn(x, area.y, "─".repeat(usize::from(width)), usize::from(width), rule);
    }

    /// The preview in the body's place, on the editor's background, with a scrollbar only once it overflows.
    fn render_preview(&mut self, buf: &mut Buffer, body: Rect, theme: &Theme, under: Paint) -> Vec<LinkSpan> {
        let roles = theme.roles();
        let background = roles.surface.pane.over(under);
        buf.set_style(
            body,
            Style::new()
                .fg(theme.color(roles.text.plain, background))
                .bg(theme.paint(background)),
        );
        let window = usize::from(body.height);
        let mut width = text_width(body.width);
        let overflows = self.with_preview(width, theme, background, |rendered| rendered.lines.len()) > window;
        if overflows {
            width = width.saturating_sub(VERTICAL_WIDTH);
        }
        let text = Rect {
            x: body.x + PADDING,
            width,
            ..body
        };
        let (offset, links, rows) = self.with_preview(width, theme, background, |rendered| {
            let offset = self.preview_offset.min(rendered.lines.len().saturating_sub(window));
            for (y, line) in (text.top()..text.bottom()).zip(rendered.lines.iter().skip(offset)) {
                buf.set_line(text.x, y, line, text.width);
            }
            let links: Vec<LinkSpan> = rendered
                .links
                .iter()
                .filter(|span| (offset..offset + window).contains(&span.row))
                .map(|span| LinkSpan {
                    row: span.row - offset,
                    columns: span.columns.start + PADDING..span.columns.end + PADDING,
                    target: span.target.clone(),
                })
                .collect();
            (offset, links, rendered.lines.len())
        });
        self.preview_offset = offset;
        self.preview_size = (rows, window);
        if overflows {
            let back = roles.scrollbar.track.over(background);
            let fg = theme.color(roles.text.plain, background);
            scrollbar::render(
                buf,
                Rect {
                    x: text.right(),
                    width: VERTICAL_WIDTH,
                    ..text
                },
                Axis::Vertical,
                ScrollbarState {
                    content: self.preview_size.0,
                    window,
                    position: offset,
                },
                ScrollbarColors {
                    bar: theme.paint(roles.scrollbar.thumb.over(back)),
                    back: theme.paint(back),
                    fg,
                    bg: theme.paint(background),
                },
            );
        }
        links
    }

    /// Scrolls the preview to `offset`, clamped to its last draw. Whether it moved.
    fn scroll_preview_to(&mut self, offset: usize) -> bool {
        let (rows, window) = self.preview_size;
        let offset = offset.min(rows.saturating_sub(window));
        let moved = offset != self.preview_offset;
        self.preview_offset = offset;
        moved
    }

    /// Runs `f` over the preview laid out `width` cells wide over `background` (the body's, from the theme), laying
    /// it out first if that isn't a layout kept. Blank rows at the end are dropped.
    fn with_preview<T>(&self, width: u16, theme: &Theme, background: Paint, f: impl FnOnce(&Rendered) -> T) -> T {
        let mut layouts = self.preview_layout.borrow_mut();
        f(layouts.get(width, theme, || {
            let mut rendered = self.preview.layout_unpadded_over(width, theme, background);
            trim_blank_tail(&mut rendered);
            rendered
        }))
    }
}

/// The preview's text width in a body `width` cells wide: inside its padding.
fn text_width(width: u16) -> u16 {
    width.saturating_sub(2 * PADDING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::KeySpec;

    fn chord(key: &str) -> KeyChord {
        match KeySpec::parse(key) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
        }
    }

    fn press(body: &mut ComposeBody, key: &str) -> EditorOutcome {
        body.handle_key(&chord(key), &Keymap::defaults(), "", Instant::now())
    }

    fn draw(body: &mut ComposeBody, width: u16, rows: u16) -> Buffer {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, rows + 1));
        let _ = body.render(
            &mut buf,
            Rect::new(0, 0, width, 1),
            Rect::new(0, 1, width, rows),
            &theme,
            theme.background(),
            true,
        );
        buf
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn the_body_needs_its_wrapped_rows_and_at_least_its_minimum() {
        let theme = Theme::default();
        let under = theme.background();
        let mut body = ComposeBody::new(EditorVariant::Plain, "Write a comment (markdown)", 3);
        assert_eq!(body.rows_needed(40, &theme, under), 3);
        let _ = body.handle_paste(&"word ".repeat(40), Instant::now());
        assert_eq!(body.rows_needed(40, &theme, under), 6);
        // The preview needs its rendered rows: a paragraph of the same words, 38 wide.
        body.set_mode(BodyMode::Preview);
        assert_eq!(body.rows_needed(40, &theme, under), 6);
    }

    #[test]
    fn the_placeholder_shows_while_empty() {
        let mut body = ComposeBody::new(EditorVariant::Plain, "Write a reply (markdown)", 3);
        let buf = draw(&mut body, 40, 3);
        assert_eq!(row(&buf, 0), " Write   Preview  ──────────────────────");
        assert_eq!(row(&buf, 1), " Write a reply (markdown)");
        let _ = press(&mut body, "x");
        assert_eq!(row(&draw(&mut body, 40, 3), 1), " x");
    }

    #[test]
    fn the_preview_ignores_printable_keys_and_keeps_the_text_and_cursor() {
        let mut body = ComposeBody::new(EditorVariant::Plain, "", 3);
        let _ = body.handle_paste("Some **bold**\nand more", Instant::now());
        let _ = press(&mut body, "up");
        let cursor = body.editor().cursor();
        body.toggle_mode();
        assert_eq!(body.mode(), BodyMode::Preview);
        let buf = draw(&mut body, 40, 3);
        assert_eq!(row(&buf, 1), " Some bold and more");
        assert!(body.consumes_key(&chord("q"), &Keymap::defaults()));
        assert_eq!(press(&mut body, "q"), EditorOutcome::Ignored);
        assert_eq!(body.handle_paste("pasted", Instant::now()), EditorOutcome::Ignored);
        body.toggle_mode();
        assert_eq!(body.text(), "Some **bold**\nand more");
        assert_eq!(body.editor().cursor(), cursor);
    }

    #[test]
    fn the_preview_scrolls_with_the_movement_keys_and_shows_a_bar_only_when_it_overflows() {
        let mut body = ComposeBody::new(EditorVariant::Plain, "", 3);
        let _ = body.handle_paste("one\n\ntwo\n\nthree", Instant::now());
        body.set_mode(BodyMode::Preview);
        // The bar's columns, left of the right padding, look like the padding unless a bar is drawn there.
        let bar_drawn = |buf: &Buffer, rows: u16| {
            (1..=rows).any(|y| {
                let (bar, pad) = (buf[(27, y)].style(), buf[(29, y)].style());
                (bar.bg, bar.add_modifier) != (pad.bg, pad.add_modifier)
            })
        };
        // Five rows in five: no bar, nothing to scroll.
        let buf = draw(&mut body, 30, 5);
        assert_eq!(row(&buf, 5), " three");
        assert!(!bar_drawn(&buf, 5));
        assert_eq!(press(&mut body, "down"), EditorOutcome::Ignored);
        // In three, a bar, and the keys scroll.
        let buf = draw(&mut body, 30, 3);
        assert!(bar_drawn(&buf, 3));
        assert_eq!(press(&mut body, "down"), EditorOutcome::Moved);
        assert_eq!(press(&mut body, "end"), EditorOutcome::Moved);
        assert_eq!(
            row(&draw(&mut body, 30, 3), 3).trim_end_matches(['▁', ' ', '▇', '█']),
            " three"
        );
        assert_eq!(press(&mut body, "home"), EditorOutcome::Moved);
        assert_eq!(row(&draw(&mut body, 30, 3), 1).trim_end(), " one");
    }

    #[test]
    fn a_click_on_a_tab_switches() {
        let mut body = ComposeBody::new(EditorVariant::Plain, "", 3);
        let _ = draw(&mut body, 40, 3);
        let click = |column| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert_eq!(body.handle_mouse(click(10)), BodyMouse::Focused);
        assert_eq!(body.mode(), BodyMode::Preview);
        assert_eq!(body.handle_mouse(click(2)), BodyMouse::Focused);
        assert_eq!(body.mode(), BodyMode::Write);
        // The rule isn't a tab.
        assert_eq!(body.handle_mouse(click(30)), BodyMouse::Missed);
    }
}
