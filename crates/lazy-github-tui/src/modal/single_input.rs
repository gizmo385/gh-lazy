//! What the single-input dialogs share (the lookups and Paste link): a markdown H1, a bold label over a checked input,
//! rules and a button row, stacked and centered in the interior, and the input's Textual validation.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::frame::on_screen;
use crate::components::button::ButtonRow;
use crate::components::switch::Switch;
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::theme::{Paint, Theme};

/// The heading's block: `MarkdownH1`'s two rows of margin above and one below.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
const HEADING_PADDING: u16 = 2;

/// A horizontal `Rule`: its line with `margin: 1 0`.
const RULE_ROWS: u16 = 3;

/// Where each part of the dialog goes in its interior. The parts are stacked top to bottom and the stack is
/// centered vertically (the container's `align: center middle`), so a dialog without the switch sits lower.
pub(super) struct Areas {
    pub(super) heading: Rect,
    pub(super) label: Rect,
    pub(super) input: Rect,
    pub(super) rules: [Option<Rect>; 2],
    pub(super) track_label: Option<Rect>,
    pub(super) track: Option<Rect>,
    pub(super) buttons: Rect,
}

impl Areas {
    /// The parts in `area`: heading, label, input, a rule, then with `with_track` the switch's label, the switch and
    /// a second rule, and the buttons.
    pub(super) fn new(area: Rect, with_track: bool) -> Self {
        let track_rows = if with_track { 1 + Switch::HEIGHT + RULE_ROWS } else { 0 };
        let total = HEADING_ROWS + 1 + text_input::HEIGHT + RULE_ROWS + track_rows + ButtonRow::HEIGHT;
        let mut y = area.y + area.height.saturating_sub(total) / 2;
        let mut next = |height: u16| {
            let rect = Rect { y, height, ..area }.intersection(area);
            y = y.saturating_add(height);
            rect
        };
        let heading_block = next(HEADING_ROWS);
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: heading_block.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        let label = next(1);
        let input = next(text_input::HEIGHT);
        let rule = |block: Rect| {
            Rect {
                y: block.y + 1,
                height: 1,
                ..block
            }
            .intersection(area)
        };
        let first_rule = rule(next(RULE_ROWS));
        let (track_label, track, second_rule) = if with_track {
            let track_label = next(1);
            let track = next(Switch::HEIGHT);
            (Some(track_label), Some(track), Some(rule(next(RULE_ROWS))))
        } else {
            (None, None, None)
        };
        let buttons = next(ButtonRow::HEIGHT);
        Self {
            heading,
            label,
            input,
            rules: [Some(first_rule), second_rule],
            track_label,
            track,
            buttons,
        }
    }

    /// The rows drawn as text cut to `screen` ([`on_screen`]). The widgets clip themselves, and keep their shape
    /// where they're cut, as the mouse sees them.
    pub(super) fn clipped(self, screen: Rect) -> Self {
        let clip = |rect: Rect| on_screen(rect, screen);
        Self {
            heading: clip(self.heading),
            label: clip(self.label),
            rules: self.rules.map(|rule| rule.map(clip)),
            track_label: self.track_label.map(clip),
            ..self
        }
    }

    /// Draws the heading, the label and the rules over the interior's `under` color.
    pub(super) fn render_text(&self, buf: &mut Buffer, heading: &str, label: &str, theme: &Theme, under: Paint) {
        let p = theme.palette;
        let text = Style::new().fg(theme.color(p.foreground, under));

        // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style, the whole row styled.
        let heading_style = p
            .markdown_h1_text_style
            .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
        let width = usize::from(self.heading.width);
        let heading_text = format!("{heading:^width$}");
        buf.set_stringn(self.heading.x, self.heading.y, heading_text, width, heading_style);

        let label = Span::styled(label, text.add_modifier(Modifier::BOLD));
        buf.set_span(self.label.x, self.label.y, &label, self.label.width);

        // `Rule { color: $secondary }`.
        let rule = Style::new().fg(theme.color(p.secondary, under));
        for rect in self.rules.into_iter().flatten() {
            buf.set_stringn(
                rect.x,
                rect.y,
                "─".repeat(usize::from(rect.width)),
                usize::from(rect.width),
                rule,
            );
        }
    }
}

/// A single-line input with Textual's validation: the red border follows `check`, run when the text changes, on
/// `enter` and when the input loses focus. Its text is selected when it gains focus (`select_on_focus`), so typing
/// replaces it.
#[derive(Debug)]
pub(super) struct CheckedInput {
    input: TextInput,
    check: fn(&str) -> bool,
}

impl CheckedInput {
    pub(super) fn new(input: TextInput, check: fn(&str) -> bool) -> Self {
        Self { input, check }
    }

    pub(super) fn input(&self) -> &TextInput {
        &self.input
    }

    pub(super) fn input_mut(&mut self) -> &mut TextInput {
        &mut self.input
    }

    /// Textual's `Input.validate`: the red border follows the latest check.
    pub(super) fn validate(&mut self) {
        self.input.set_invalid(!(self.check)(self.input.value()));
    }

    /// Focus moved, from the input when `had` and to it when `has`.
    pub(super) fn focus_moved(&mut self, had: bool, has: bool) {
        match (had, has) {
            (true, false) => self.validate(),
            (false, true) => self.input.select_all(),
            _ => {}
        }
    }

    /// Edits the input with `edit`, checking it if the text changed (Textual's `Input.Changed`) or on a submit.
    pub(super) fn edit(&mut self, edit: impl FnOnce(&mut TextInput) -> InputOutcome) -> InputOutcome {
        let before = self.input.value().to_owned();
        let outcome = edit(&mut self.input);
        match outcome {
            InputOutcome::Changed | InputOutcome::Copied(_) if self.input.value() != before => self.validate(),
            InputOutcome::Submitted => self.validate(),
            InputOutcome::Changed | InputOutcome::Copied(_) | InputOutcome::Ignored => {}
        }
        outcome
    }

    /// A bracketed paste: the first line goes in at the cursor, and is checked.
    pub(super) fn paste(&mut self, text: &str) {
        let _ = self.edit(|input| {
            input.handle_paste(text);
            InputOutcome::Changed
        });
    }
}
