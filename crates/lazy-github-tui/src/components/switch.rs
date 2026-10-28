//! Textual's `Switch`: an on/off toggle drawn as a 4-cell slider in a `tall` border, with `padding: 0 2`.
//!
//! Like a button, a switch doesn't know whether it has focus: the dialog's [`crate::modal::FocusRing`] does, and
//! passes it to [`Switch::render`]. The 0.3 s slide and the hover colors are Textual-only cosmetics (PLAN §10.8): the
//! thumb jumps as the value flips.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};

use crate::components::disabled;
use crate::keymap::BindingId;
use crate::theme::{AnsiColor, Paint, Theme, ThemeColor};

/// A focused switch's binding: `enter` or `space` toggles it (`show=False`, so never in the footer).
pub const SWITCH_BINDINGS: &[BindingId] = &[BindingId::TextualSwitchToggle];

/// Cells between each side border and the slider (`padding: 0 2`).
const PADDING: u16 = 2;

/// The slider's cells; the thumb is half of them.
const TRACK: u16 = 4;

/// An on/off switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Switch {
    value: bool,
    /// Textual's `disabled`: drawn faded and never focused, and clicks do nothing.
    disabled: bool,
}

impl Switch {
    /// Columns it takes: the side borders, the padding and the slider.
    pub const WIDTH: u16 = 2 + 2 * PADDING + TRACK;

    /// Rows it takes: the slider between the top and bottom border.
    pub const HEIGHT: u16 = 3;

    /// A switch that's on when `value` is.
    pub fn new(value: bool) -> Self {
        Self { value, disabled: false }
    }

    /// Disables the switch or enables it again. A disabled switch is drawn faded (Textual's `opacity: 0.7`) and
    /// ignores clicks; its owner keeps it out of the focus order.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// Whether it's disabled.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Whether it's on.
    pub fn value(&self) -> bool {
        self.value
    }

    /// Flips it: `enter` or `space` while it's focused, or a click.
    pub fn toggle(&mut self) {
        self.value = !self.value;
    }

    /// The cells a switch drawn in `area` covers: [`Self::WIDTH`] x [`Self::HEIGHT`] at its top left.
    fn bounds(area: Rect) -> Rect {
        Rect {
            width: area.width.min(Self::WIDTH),
            height: area.height.min(Self::HEIGHT),
            ..area
        }
    }

    /// A mouse event against a switch drawn in `area`: a left press on it toggles it and returns true, and the dialog
    /// focuses it. Textual toggles on the click, which completes on the release; toggling on the press is the same to
    /// the eye and needs no state.
    pub fn handle_mouse(&mut self, event: &MouseEvent, area: Rect) -> bool {
        let hit = !self.disabled
            && event.kind == MouseEventKind::Down(MouseButton::Left)
            && Self::bounds(area).contains(Position::new(event.column, event.row));
        if hit {
            self.toggle();
        }
        hit
    }

    /// Draws the switch into the top-left [`Self::WIDTH`] x [`Self::HEIGHT`] cells of `area`, clipped to it. `under`
    /// is the parent's background, which shows in the half-cell `▊`/`▎` edges.
    pub fn render(&self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let area = Self::bounds(area).intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let styles = Styles::new(theme, under, focused && !self.disabled, self.value);
        let last_row = Self::HEIGHT - 1;
        // The thumb is the slider's left half while off and its right half while on.
        let thumb = if self.value { TRACK / 2..TRACK } else { 0..TRACK / 2 };
        for dy in 0..area.height {
            for dx in 0..area.width {
                let (symbol, style) = match (dx, dy) {
                    (0, _) => ("▊", styles.left),
                    (dx, _) if dx == Self::WIDTH - 1 => ("▎", styles.right),
                    (_, 0) => ("▔", styles.inner),
                    (_, dy) if dy == last_row => ("▁", styles.inner),
                    (dx, _) => match (dx - 1).checked_sub(PADDING) {
                        Some(cell) if thumb.contains(&cell) => (" ", styles.thumb),
                        Some(cell) if cell < TRACK => (" ", styles.track),
                        _ => (" ", styles.fill),
                    },
                };
                buf[(area.x + dx, area.y + dy)].set_symbol(symbol).set_style(style);
            }
        }
        if self.disabled {
            disabled::fade(buf, area, theme, under);
        }
    }
}

/// A switch's styles for one frame, from `Switch.DEFAULT_CSS`.
struct Styles {
    /// The padding: `$foreground` on `$surface`, tinted `$foreground 5%` while focused.
    fill: Style,
    /// The slider's other half: its `background`, `$panel-darken-2`, under the switch's text color.
    track: Style,
    /// The thumb: `ScrollBarRender`'s reversed blank in the slider's `color`, over the switch's background.
    thumb: Style,
    /// `▊` on the left: the parent's background over the border color, reversed (Textual's border location 2).
    left: Style,
    /// `▔` and `▁`: the border color over the switch's background (location 0).
    inner: Style,
    /// `▎` on the right: the border color over the parent's background (location 1).
    right: Style,
}

impl Styles {
    fn new(theme: &Theme, under: Paint, focused: bool, on: bool) -> Self {
        let p = theme.palette;
        let surface = p.surface.over(under);
        let bg = if focused {
            theme.tint(surface, p.foreground, 5.0)
        } else {
            surface
        };
        let fill = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
        let track_bg = p.panel_darken_2.over(bg);
        let border = theme.color(if focused { p.border } else { p.border_blurred }, bg);
        Self {
            fill,
            track: fill.bg(theme.paint(track_bg)),
            thumb: Style::new()
                .fg(theme.color(slider_color(theme, on), track_bg))
                .bg(theme.paint(bg))
                .add_modifier(Modifier::REVERSED),
            left: Style::new()
                .fg(border)
                .bg(theme.paint(under))
                .add_modifier(Modifier::REVERSED),
            inner: Style::new().fg(border).bg(theme.paint(bg)),
            right: Style::new().fg(border).bg(theme.paint(under)),
        }
    }
}

/// The slider's `color`, by the CSS rules that win: `$success` while on; off, `$primary 15%` on a light theme
/// (`:light`, the ANSI light theme included, whose rule comes after `:ansi`'s), `ansi_red` on the ANSI dark theme and
/// `$panel` otherwise.
fn slider_color(theme: &Theme, on: bool) -> ThemeColor {
    let p = theme.palette;
    if on {
        p.success
    } else if !theme.dark {
        p.primary.multiply_alpha(0.15)
    } else if theme.ansi {
        ThemeColor::Ansi(AnsiColor::Red)
    } else {
        p.panel
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;
    use ratatui::style::Color;

    use super::*;
    use crate::test_util::buffer_to_styled_string;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn left_press(column: u16, row: u16) -> MouseEvent {
        mouse(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    #[test]
    fn toggling_flips_the_value() {
        let mut switch = Switch::new(false);
        assert!(!switch.value());
        switch.toggle();
        assert!(switch.value());
        switch.toggle();
        assert!(!switch.value());
        assert!(Switch::new(true).value());
        assert_eq!(Switch::default(), Switch::new(false));
    }

    #[test]
    fn a_disabled_switch_ignores_clicks_and_draws_faded() {
        let area = Rect::new(0, 0, Switch::WIDTH, Switch::HEIGHT);
        let mut switch = Switch::new(true);
        switch.set_disabled(true);
        assert!(switch.is_disabled());
        assert!(!switch.handle_mouse(&left_press(4, 1), area));
        assert!(switch.value());
        let theme = Theme::default();
        let under = theme.palette.surface.over(theme.background());
        let draw = |switch: &Switch| {
            let mut buf = Buffer::empty(area);
            switch.render(&mut buf, area, &theme, under, true);
            buf
        };
        let enabled = draw(&Switch::new(true));
        let disabled = draw(&switch);
        // Same cells, other colors; drawn unfocused whatever the owner says.
        assert_eq!(
            enabled
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>(),
            disabled
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        );
        assert_ne!(enabled[(4, 1)].bg, disabled[(4, 1)].bg);
    }

    #[test]
    fn a_click_inside_toggles_and_one_outside_doesnt() {
        let area = Rect::new(10, 5, 40, 3);
        let mut switch = Switch::new(false);
        // Every corner of the 10 x 3 box, borders included.
        for (column, row) in [(10, 5), (19, 5), (10, 7), (19, 7), (14, 6)] {
            let before = switch.value();
            assert!(switch.handle_mouse(&left_press(column, row), area), "({column}, {row})");
            assert_ne!(switch.value(), before);
        }
        let before = switch.value();
        // Right of the box (still in `area`), below it, left of it, and not a left press.
        for event in [
            left_press(20, 6),
            left_press(14, 8),
            left_press(9, 6),
            mouse(MouseEventKind::Down(MouseButton::Right), 14, 6),
            mouse(MouseEventKind::Up(MouseButton::Left), 14, 6),
            mouse(MouseEventKind::Moved, 14, 6),
        ] {
            assert!(!switch.handle_mouse(&event, area), "{event:?}");
        }
        assert_eq!(switch.value(), before);
    }

    #[test]
    fn toggle_is_hidden_on_enter_and_space() {
        let def = BindingId::TextualSwitchToggle.def();
        assert!(!def.show);
        assert_eq!(
            (def.name, def.default_keys, def.action, def.description),
            ("textual.switch.toggle_switch", "enter,space", "toggle_switch", "Toggle")
        );
    }

    /// The Lookup dialog's background, `$surface-lighten-3` over the screen.
    fn dialog_background(theme: &Theme) -> Paint {
        theme.palette.surface_lighten_3.over(theme.background())
    }

    /// The four switches, off and on, unfocused and focused, side by side on the dialog's background.
    fn states(theme: &Theme) -> String {
        let under = dialog_background(theme);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4 * (Switch::WIDTH + 1), Switch::HEIGHT));
        for cell in &mut buf.content {
            cell.set_bg(theme.paint(under));
        }
        let cases = [(false, false), (false, true), (true, false), (true, true)];
        for (index, (value, focused)) in cases.into_iter().enumerate() {
            let x = u16::try_from(index).unwrap() * (Switch::WIDTH + 1);
            let area = Rect::new(x, 0, Switch::WIDTH, Switch::HEIGHT);
            Switch::new(value).render(&mut buf, area, theme, under, focused);
        }
        buffer_to_styled_string(&buf)
    }

    #[test]
    fn switch_states() {
        insta::assert_snapshot!(states(&Theme::default()));
    }

    #[test]
    fn switch_states_light() {
        insta::assert_snapshot!(states(&Theme::by_name("textual-light").unwrap()));
    }

    #[test]
    fn switch_states_ansi_dark() {
        insta::assert_snapshot!(states(&Theme::by_name("ansi-dark").unwrap()));
    }

    fn drawn(value: bool, focused: bool) -> Buffer {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, Switch::WIDTH, Switch::HEIGHT));
        let area = buf.area;
        Switch::new(value).render(&mut buf, area, &theme, dialog_background(&theme), focused);
        buf
    }

    fn rgb(hex: u32) -> Color {
        let [_, r, g, b] = hex.to_be_bytes();
        Color::Rgb(r, g, b)
    }

    /// `(fg, bg, reversed)` of a cell.
    fn cell(buf: &Buffer, x: u16, y: u16) -> (Color, Color, bool) {
        let cell = &buf[(x, y)];
        (cell.fg, cell.bg, cell.modifier.contains(Modifier::REVERSED))
    }

    #[test]
    fn the_cells_match_pythons_captures() {
        // `modal-frame` `open`: off and unfocused.
        let off = drawn(false, false);
        let text: String = (0..Switch::WIDTH).map(|x| off[(x, 0)].symbol().to_owned()).collect();
        assert_eq!(text, "▊▔▔▔▔▔▔▔▔▎");
        let text: String = (0..Switch::WIDTH).map(|x| off[(x, 2)].symbol().to_owned()).collect();
        assert_eq!(text, "▊▁▁▁▁▁▁▁▁▎");
        assert_eq!(cell(&off, 0, 1), (rgb(0x19_19_19), rgb(0x4f_4f_4f), true));
        assert_eq!(cell(&off, 1, 0), (rgb(0x19_19_19), rgb(0x1e_1e_1e), false));
        assert_eq!(cell(&off, 9, 1), (rgb(0x19_19_19), rgb(0x4f_4f_4f), false));
        assert_eq!(off[(2, 1)].bg, rgb(0x1e_1e_1e));
        assert_eq!(cell(&off, 3, 1), (rgb(0x24_2f_38), rgb(0x1e_1e_1e), true));
        assert_eq!(cell(&off, 4, 1), (rgb(0x24_2f_38), rgb(0x1e_1e_1e), true));
        assert_eq!(cell(&off, 5, 1), (rgb(0xe0_e0_e0), rgb(0x00_0f_18), false));
        assert_eq!(cell(&off, 6, 1), (rgb(0xe0_e0_e0), rgb(0x00_0f_18), false));
        assert_eq!(off[(7, 1)].bg, rgb(0x1e_1e_1e));

        // `modal-frame` `switch-focused`: off and focused, tinted with a `$border` border.
        let focused = drawn(false, true);
        assert_eq!(cell(&focused, 0, 1), (rgb(0x01_78_d4), rgb(0x4f_4f_4f), true));
        assert_eq!(cell(&focused, 1, 0), (rgb(0x01_78_d4), rgb(0x27_27_27), false));
        assert_eq!(focused[(2, 1)].bg, rgb(0x27_27_27));
        assert_eq!(cell(&focused, 3, 1), (rgb(0x24_2f_38), rgb(0x27_27_27), true));
        assert_eq!(cell(&focused, 5, 1), (rgb(0xe0_e0_e0), rgb(0x00_0f_18), false));

        // `repo-lookup` `switch-on`: on and focused, the thumb on the right in `$success`.
        let on = drawn(true, true);
        assert_eq!(cell(&on, 3, 1), (rgb(0xe0_e0_e0), rgb(0x00_0f_18), false));
        assert_eq!(cell(&on, 4, 1), (rgb(0xe0_e0_e0), rgb(0x00_0f_18), false));
        assert_eq!(cell(&on, 5, 1), (rgb(0x4e_bf_71), rgb(0x27_27_27), true));
        assert_eq!(cell(&on, 6, 1), (rgb(0x4e_bf_71), rgb(0x27_27_27), true));
    }

    #[test]
    fn a_small_area_clips_the_switch() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 2));
        let area = buf.area;
        Switch::new(true).render(&mut buf, area, &theme, theme.background(), true);
        assert_eq!(buf[(0, 0)].symbol(), "▊");
        assert_eq!(buf[(5, 0)].symbol(), "▔");
        // Off the buffer entirely: nothing to draw.
        Switch::new(true).render(&mut buf, Rect::new(10, 10, 10, 3), &theme, theme.background(), true);
    }
}
