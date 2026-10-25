//! Textual's `LoadingIndicator`, the overlay a widget shows while `loading = True`: five dots pulsing through a
//! gradient, or the static text "Loading..." when animations are off.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme::{Paint, Rgb, Theme, ThemeColor};

/// How often the dots are redrawn (Textual's `auto_refresh = 1 / 16`).
pub const FRAME_INTERVAL: Duration = Duration::from_micros(62_500);

/// What Textual shows with `TEXTUAL_ANIMATIONS=none`.
const STATIC_TEXT: &str = "Loading...";

/// The dot, `●`.
const DOT: &str = "\u{25cf}";

/// Textual's `Gradient` quality: the number of precomputed steps.
const QUALITY: usize = 50;

/// Paints the overlay over `area`: `$boost` over `$background`, with the indicator centered. `elapsed` is the time
/// since the overlay first appeared, which sets the animation's phase.
pub fn render(buf: &mut Buffer, area: Rect, theme: &Theme, animations: bool, elapsed: Duration) {
    let palette = theme.palette;
    let background = palette.boost.over(theme.background());
    let primary = palette.primary.over(background);
    let base = Style::new().fg(theme.paint(primary)).bg(theme.paint(background));
    for pos in area.intersection(buf.area).positions() {
        buf[pos].reset();
        buf[pos].set_style(base);
    }
    let line = if animations {
        let colors = dot_colors(background, primary, elapsed);
        let mut spans = Vec::with_capacity(9);
        for (i, color) in colors.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(DOT, Style::new().fg(theme.paint(color))));
        }
        Line::from(spans)
    } else {
        Line::raw(STATIC_TEXT)
    };
    // `content-align: center middle`.
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX).min(area.width);
    let centered = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(1) / 2,
        width,
        height: area.height.min(1),
    };
    Paragraph::new(line).render(centered, buf);
}

/// The five dots' colors at `elapsed`: Textual's gradient from `$background` blended 10% toward `$primary`, through
/// `$primary` at 0.7, to `$primary` lightened 10%, sampled at `(1 - blend)²` with `blend = (t * 0.8 - i / 8) mod 1`.
fn dot_colors(background: Paint, primary: Paint, elapsed: Duration) -> [Paint; 5] {
    let Paint::Rgb(color) = primary else {
        // Textual can't blend the terminal's own colors: every stop is the ANSI primary.
        return [primary; 5];
    };
    let start = match (ThemeColor::Rgba {
        r: color.r,
        g: color.g,
        b: color.b,
        a: 0.1,
    })
    .over(background)
    {
        Paint::Rgb(c) => c,
        Paint::Ansi(_) => color,
    };
    let gradient = Gradient::new(&[(0.0, start), (0.7, color), (1.0, color.lighten(0.1))]);
    let t = elapsed.as_secs_f64();
    std::array::from_fn(|i| {
        // `i` is below 5.
        #[allow(clippy::cast_precision_loss)]
        let blend = (t * 0.8 - i as f64 / 8.0).rem_euclid(1.0);
        Paint::Rgb(gradient.get((1.0 - blend).powi(2)))
    })
}

/// Textual's `Gradient`: colors precomputed at [`QUALITY`] evenly spaced steps, then blended between neighbors.
struct Gradient {
    colors: Vec<Rgb>,
}

impl Gradient {
    /// A gradient through `stops` (position, color), which start at 0 and end at 1.
    fn new(stops: &[(f64, Rgb)]) -> Self {
        let mut colors = Vec::with_capacity(QUALITY);
        let mut segment = 0;
        for step in 0..QUALITY {
            // Small counts: exact as floats.
            #[allow(clippy::cast_precision_loss)]
            let step = step as f64 / (QUALITY - 1) as f64;
            while segment + 2 < stops.len() && step > stops[segment + 1].0 {
                segment += 1;
            }
            let ((from_at, from), (to_at, to)) = (stops[segment], stops[segment + 1]);
            colors.push(from.blend(to, (step - from_at) / (to_at - from_at)));
        }
        Self { colors }
    }

    /// The color at `position` (0 to 1).
    fn get(&self, position: f64) -> Rgb {
        let last = self.colors.len() - 1;
        if position <= 0.0 {
            return self.colors[0];
        }
        if position >= 1.0 {
            return self.colors[last];
        }
        // `position` is in (0, 1), so the index is below `last`.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let (index, fraction) = {
            let scaled = position * last as f64;
            (scaled as usize, scaled.fract())
        };
        self.colors[index].blend(self.colors[index + 1], fraction)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::test_util::buffer_to_string;

    fn hex(paint: Paint) -> String {
        match paint {
            Paint::Rgb(c) => format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b),
            Paint::Ansi(a) => format!("{a:?}"),
        }
    }

    #[test]
    fn the_dots_match_textuals_gradient() {
        // Textual 8.2.5 under textual-dark, from `LoadingIndicator.render`'s arithmetic.
        let theme = Theme::default();
        let background = theme.palette.boost.over(theme.background());
        let primary = theme.palette.primary.over(background);
        for (t, expected) in [
            (0.0, ["#4091F0", "#172530", "#152B3B", "#13344E", "#0F4168"]),
            (0.3, ["#0468B6", "#117EDB", "#17242D", "#172530", "#152B3C"]),
            (0.5, ["#0B4E82", "#0662A9", "#0579D5", "#358CEB", "#17242E"]),
            (1.0, ["#162735", "#133045", "#113B5C", "#0C4B7B", "#075EA1"]),
            (1.7, ["#09548E", "#046AB8", "#147EDB", "#17242D", "#172530"]),
        ] {
            let colors = dot_colors(background, primary, Duration::from_secs_f64(t));
            assert_eq!(colors.map(hex), expected, "t = {t}");
        }
    }

    #[test]
    fn lighten_matches_textual() {
        assert_eq!(Rgb::new(0x01, 0x78, 0xD4).lighten(0.1), Rgb::new(64, 145, 240));
        assert_eq!(Rgb::new(0xE0, 0xE0, 0xE0).lighten(0.5), Rgb::WHITE);
        assert_eq!(Rgb::BLACK.lighten(-0.1), Rgb::BLACK);
    }

    #[test]
    fn static_text_is_centered_in_primary_on_boost() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &theme, false, Duration::ZERO);
        assert_eq!(buffer_to_string(&buf), "\n\n     Loading...\n\n\n");
        let cell = &buf[(5, 2)];
        assert_eq!(cell.fg, Color::Rgb(0x01, 0x78, 0xD4));
        assert_eq!(cell.bg, Color::Rgb(27, 27, 27));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(27, 27, 27));
    }

    #[test]
    fn dots_are_centered() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 11, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &theme, true, Duration::ZERO);
        assert_eq!(buffer_to_string(&buf), " ● ● ● ● ●\n");
        assert_eq!(buf[(1, 0)].fg, Color::Rgb(0x40, 0x91, 0xF0));
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 0), (0, 3)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            render(&mut buf, area, &Theme::default(), true, Duration::from_secs(3));
            render(&mut buf, area, &Theme::default(), false, Duration::ZERO);
        }
    }
}
