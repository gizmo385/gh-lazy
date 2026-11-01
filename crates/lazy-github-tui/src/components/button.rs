//! Textual's `Button` in its default style, and `ModalDialogButtons`, the Submit/Cancel row most dialogs end with.
//!
//! A button doesn't know whether it has focus: the dialog's [`crate::modal::FocusRing`] does, and passes it to
//! [`Button::render`].

use std::borrow::Cow;
use std::time::Duration;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use tokio::time::Instant;

use crate::keymap::BindingId;
use crate::theme::{AnsiColor, Button as ButtonRoles, ButtonColors, Paint, Theme, ThemeColor};

/// A focused button's binding: `enter` presses it (`show=False`, so never in the footer).
pub const BUTTON_BINDINGS: &[BindingId] = &[BindingId::TextualButtonPress];

/// Rows a button takes: its label between a top and a bottom border.
pub const HEIGHT: u16 = 3;

/// Textual's `min-width: 16`.
const MIN_WIDTH: u16 = 16;

/// A press within this long of the last counted one is ignored (Textual's `active_effect_duration`).
const DEBOUNCE: Duration = Duration::from_millis(200);

/// A button's color scheme (Textual's `ButtonVariant`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// `$surface`, for a neutral action.
    Default,
    /// Blue: Confirm's Yes.
    Primary,
    /// Green: every Submit.
    Success,
    /// Yellow.
    Warning,
    /// Red: every Cancel, and Confirm's No.
    Error,
}

impl ButtonVariant {
    /// Every variant, in Textual's order.
    pub const ALL: [ButtonVariant; 5] = [
        ButtonVariant::Default,
        ButtonVariant::Primary,
        ButtonVariant::Success,
        ButtonVariant::Warning,
        ButtonVariant::Error,
    ];

    /// This variant's colors in the theme's button roles.
    fn colors(self, button: &ButtonRoles) -> &ButtonColors {
        match self {
            ButtonVariant::Default => &button.default,
            ButtonVariant::Primary => &button.primary,
            ButtonVariant::Success => &button.success,
            ButtonVariant::Warning => &button.warning,
            ButtonVariant::Error => &button.error,
        }
    }
}

/// What a mouse event did to a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonMouse {
    /// Nothing: not a left button event on the enabled button, or a click inside the debounce.
    Missed,
    /// A left press on it. Textual focuses a widget on mouse down, so the dialog focuses it.
    Focus,
    /// The release that completes a click on it, counted as a press.
    Pressed,
}

/// A clickable, focusable button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    label: Cow<'static, str>,
    variant: ButtonVariant,
    disabled: bool,
    /// The last press that counted, for the debounce.
    last_press: Option<Instant>,
    /// Where it was drawn last, for the mouse.
    area: Rect,
    /// A left press landed on it and its release hasn't come yet: the two make a click.
    armed: bool,
}

impl Button {
    /// An enabled button.
    pub fn new(label: impl Into<Cow<'static, str>>, variant: ButtonVariant) -> Self {
        Self {
            label: label.into(),
            variant,
            disabled: false,
            last_press: None,
            area: Rect::default(),
            armed: false,
        }
    }

    /// The label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Replaces the label (Edit issue's "Saving...").
    pub fn set_label(&mut self, label: impl Into<Cow<'static, str>>) {
        self.label = label.into();
    }

    /// The variant.
    pub fn variant(&self) -> ButtonVariant {
        self.variant
    }

    /// Whether it's disabled.
    pub fn disabled(&self) -> bool {
        self.disabled
    }

    /// Disables or enables it. The dialog also disables the field in its focus ring, which blurs it.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        self.armed = false;
    }

    /// Its width: the label with a cell each side, at least 16, plus the side borders the ANSI themes draw.
    pub fn width(&self, theme: &Theme) -> u16 {
        let label = u16::try_from(Span::raw(self.label.as_ref()).width()).unwrap_or(u16::MAX);
        let borders = if theme.roles().button.outlined { 2 } else { 0 };
        label.saturating_add(2 + borders).max(MIN_WIDTH)
    }

    /// A press at `now`, by `enter` or a click: whether it counts. It doesn't while disabled, or within 0.2 s of the
    /// last press that counted.
    pub fn press(&mut self, now: Instant) -> bool {
        if self.disabled {
            return false;
        }
        if self
            .last_press
            .is_some_and(|last| now.saturating_duration_since(last) < DEBOUNCE)
        {
            return false;
        }
        self.last_press = Some(now);
        true
    }

    /// A mouse event at `now`, against where the button was drawn last. A left press on it asks for focus and its
    /// release on it presses it.
    pub fn handle_mouse(&mut self, event: &MouseEvent, now: Instant) -> ButtonMouse {
        let inside = self.area.contains(Position::new(event.column, event.row));
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.armed = inside && !self.disabled;
                if self.armed {
                    ButtonMouse::Focus
                } else {
                    ButtonMouse::Missed
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let click = std::mem::take(&mut self.armed) && inside;
                if click && self.press(now) {
                    ButtonMouse::Pressed
                } else {
                    ButtonMouse::Missed
                }
            }
            _ => ButtonMouse::Missed,
        }
    }

    /// Draws the button into the top [`HEIGHT`] rows of `area`, clipped to it.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: bool) {
        let area = Rect {
            height: area.height.min(HEIGHT),
            ..area
        }
        .intersection(buf.area);
        self.area = area;
        if area.is_empty() {
            return;
        }
        let styles = Styles::new(self.variant, theme, focused, self.disabled);
        let sides = u16::from(theme.roles().button.outlined);
        for dy in 0..area.height {
            let y = area.y + dy;
            for dx in 0..area.width {
                let x = area.x + dx;
                let edge = sides == 1 && (dx == 0 || dx == area.width - 1);
                let (symbol, style) = match dy {
                    _ if edge && dx == 0 => ("▊", styles.left),
                    _ if edge => ("▎", styles.right),
                    0 => ("▔", styles.top),
                    2 => ("▁", styles.bottom),
                    _ => (" ", styles.fill),
                };
                buf[(x, y)].set_symbol(symbol).set_style(style);
            }
        }
        if area.height < 2 {
            return;
        }
        // The label with Textual's `line-pad: 1`, centered between the side borders.
        let inner = area.width.saturating_sub(2 * sides);
        let text = format!(" {} ", self.label);
        let text_width = u16::try_from(Span::raw(text.as_str()).width()).unwrap_or(u16::MAX);
        let x = area.x + sides + inner.saturating_sub(text_width) / 2;
        let width = inner.saturating_sub(x - area.x - sides);
        buf.set_stringn(x, area.y + 1, &text, usize::from(width), styles.label);
    }
}

/// A button's styles for one frame, from `Button.DEFAULT_CSS` (`-style-default`).
struct Styles {
    top: Style,
    bottom: Style,
    /// The label row around the label.
    fill: Style,
    label: Style,
    /// The ANSI themes' side borders.
    left: Style,
    right: Style,
}

impl Styles {
    fn new(variant: ButtonVariant, theme: &Theme, focused: bool, disabled: bool) -> Self {
        let roles = theme.roles();
        let button = &roles.button;
        let focus_style = |style: Style| {
            if focused {
                button.focus_style.apply(style)
            } else {
                style
            }
        };
        let colors = variant.colors(button);
        if button.outlined {
            // `Button:ansi`: the terminal's background, the text and a `tall` border in the variant's color.
            let default = theme.paint(Paint::Ansi(AnsiColor::Default));
            let under = Paint::Ansi(AnsiColor::Default);
            let border = if disabled {
                button.disabled_border
            } else {
                colors.border
            };
            let border = Style::new().fg(theme.color(border, under)).bg(default);
            let mut label = Style::new()
                .fg(theme.color(colors.label, under))
                .bg(default)
                .add_modifier(Modifier::BOLD);
            if disabled {
                label = label.add_modifier(Modifier::DIM);
            }
            return Self {
                top: border,
                bottom: border,
                fill: Style::new().bg(default),
                label: focus_style(label),
                left: border.add_modifier(Modifier::REVERSED),
                right: border,
            };
        }
        let mut bg = colors.bg.over(theme.background());
        if focused {
            bg = theme.tint(bg, roles.surface.focus_tint, 100.0);
        }
        let color = if disabled { colors.label_disabled } else { colors.label };
        let paint = theme.paint(bg);
        let edge = |c: ThemeColor| Style::new().fg(theme.color(c, bg)).bg(paint);
        let label = Style::new()
            .fg(theme.color(color, bg))
            .bg(paint)
            .add_modifier(Modifier::BOLD);
        Self {
            top: edge(colors.light),
            bottom: edge(colors.dark),
            fill: Style::new().fg(theme.color(color, bg)).bg(paint),
            label: focus_style(label),
            left: edge(colors.light),
            right: edge(colors.light),
        }
    }
}

/// Which button of a [`ButtonRow`]: what it reports when pressed, and how a dialog names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    /// The submit button: does what `modal.submit` does.
    Submit,
    /// "Cancel": does what `modal.close` does.
    Cancel,
}

/// `ModalDialogButtons`: a green submit button and a red "Cancel", side by side and centered, with a blank row above
/// and below (`Button { margin: 1 }`). A [`Self::flush`] row has no margins, and a [`Self::spaced`] row only the side
/// ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonRow {
    submit: Button,
    cancel: Button,
    margins: RowMargins,
}

/// The buttons' margins in a [`ButtonRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowMargins {
    /// `margin: 1`.
    All,
    /// `margin: 0 1`.
    Sides,
    /// None.
    None,
}

impl ButtonRow {
    /// The rows a dialog reserves for the row: the buttons and their margins.
    pub const HEIGHT: u16 = HEIGHT + 2;

    /// The rows a [`Self::flush`] row takes: the buttons alone.
    pub const FLUSH_HEIGHT: u16 = HEIGHT;

    /// The row with a submit button labeled `submit_label` ("Submit", "Lookup", "Save", ...).
    pub fn new(submit_label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            submit: Button::new(submit_label, ButtonVariant::Success),
            cancel: Button::new("Cancel", ButtonVariant::Error),
            margins: RowMargins::All,
        }
    }

    /// The row without margins: the two buttons touching, centered, in a plain `Horizontal` (New issue, Edit issue).
    pub fn flush(submit_label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            margins: RowMargins::None,
            ..Self::new(submit_label)
        }
    }

    /// The row with a cell of margin left and right of each button but none above or below (`margin: 0 1`): Add
    /// Comment's. It's [`Self::FLUSH_HEIGHT`] tall.
    pub fn spaced(submit_label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            margins: RowMargins::Sides,
            ..Self::new(submit_label)
        }
    }

    /// The row with "Cancel" in `variant` rather than red (Add Comment's is the default variant).
    #[must_use]
    pub fn with_cancel_variant(mut self, variant: ButtonVariant) -> Self {
        self.cancel = Button::new("Cancel", variant);
        self
    }

    /// The rows the row takes: [`Self::HEIGHT`], or [`Self::FLUSH_HEIGHT`] for a row without margins above and below.
    pub fn height(&self) -> u16 {
        match self.margins {
            RowMargins::All => Self::HEIGHT,
            RowMargins::Sides | RowMargins::None => Self::FLUSH_HEIGHT,
        }
    }

    /// One of the buttons.
    pub fn button(&self, which: Pressed) -> &Button {
        match which {
            Pressed::Submit => &self.submit,
            Pressed::Cancel => &self.cancel,
        }
    }

    /// One of the buttons, to relabel or disable it.
    pub fn button_mut(&mut self, which: Pressed) -> &mut Button {
        match which {
            Pressed::Submit => &mut self.submit,
            Pressed::Cancel => &mut self.cancel,
        }
    }

    /// Presses `which` at `now` (`enter` on it): `Some(which)` when the press counts ([`Button::press`]).
    pub fn press(&mut self, which: Pressed, now: Instant) -> Option<Pressed> {
        self.button_mut(which).press(now).then_some(which)
    }

    /// A mouse event at `now`: the button it focuses or presses, if any.
    pub fn handle_mouse(&mut self, event: &MouseEvent, now: Instant) -> Option<(Pressed, ButtonMouse)> {
        [Pressed::Submit, Pressed::Cancel].into_iter().find_map(|which| {
            match self.button_mut(which).handle_mouse(event, now) {
                ButtonMouse::Missed => None,
                outcome => Some((which, outcome)),
            }
        })
    }

    /// Draws the row into the top [`Self::height`] rows of `area`, with `focused` drawn focused.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: Option<Pressed>) {
        let (submit, cancel) = (self.submit.width(theme), self.cancel.width(theme));
        // With side margins, one cell outside each button, and one between them where their margins collapse.
        let (side, top) = match self.margins {
            RowMargins::All => (1, 1),
            RowMargins::Sides => (1, 0),
            RowMargins::None => (0, 0),
        };
        let total = submit + cancel + 3 * side;
        let left = area.x + area.width.saturating_sub(total) / 2 + side;
        let gap = side;
        let y = area.y.saturating_add(top);
        let rows = area.height.saturating_sub(top).min(HEIGHT);
        let place = |x: u16, width: u16| {
            Rect {
                x,
                y,
                width,
                height: rows,
            }
            .intersection(area)
        };
        let submit_area = place(left, submit);
        let cancel_area = place(left + submit + gap, cancel);
        self.submit
            .render(buf, submit_area, theme, focused == Some(Pressed::Submit));
        self.cancel
            .render(buf, cancel_area, theme, focused == Some(Pressed::Cancel));
    }
}

#[cfg(test)]
mod tests;
