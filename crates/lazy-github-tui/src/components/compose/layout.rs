//! Where a compose sheet goes and how tall each of its parts is.
//!
//! [`sheet_geometry`] gives the limits a screen sets (the frame's width, the rows it may take, the quote's cap),
//! [`ComposeLayout`] stacks the parts within them with the body as tall as its text, and [`SheetPlacement`] puts the
//! frame on the screen, keeping its top still from one draw to the next.

use std::ops::Range;

use ratatui::layout::Rect;

/// Rows above and below the sheet that it leaves to the screen behind it: the margin row under the screen's top, and
/// the one above the footer.
const MARGIN: u16 = 1;

/// The fewest rows a quote is capped at, however short the screen.
const MIN_QUOTE_CAP: u16 = 2;

/// The limits a screen puts on a compose sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetGeometry {
    /// The frame's outer width, border included.
    pub width: u16,
    /// The tallest the frame may be, border included: the screen less the footer row and a margin row above and
    /// below, or, expanded, the whole screen above the footer.
    pub max_height: u16,
    /// The columns the parts are drawn in: inside the border, less a column of padding each side.
    pub content_width: u16,
    /// The most rows a quote shows, its `⋯` row included: a sixth of [`Self::max_height`], or a third expanded.
    pub quote_cap: u16,
}

impl SheetGeometry {
    /// The rows inside the border the parts may take.
    pub fn room(&self) -> u16 {
        self.max_height.saturating_sub(2)
    }
}

/// The limits for a sheet at most `max_width` wide (100 for most dialogs) on `screen`, the whole terminal footer row
/// included. Expanded, the sheet is the whole screen above the footer.
pub fn sheet_geometry(screen: Rect, max_width: u16, expanded: bool) -> SheetGeometry {
    let above_footer = screen.height.saturating_sub(1);
    let (width, max_height, quote_share) = if expanded {
        (screen.width, above_footer, 3)
    } else {
        let width = max_width.min(screen.width.saturating_sub(4)).max(screen.width.min(4));
        (width, above_footer.saturating_sub(2 * MARGIN), 6)
    };
    SheetGeometry {
        width,
        max_height,
        content_width: width.saturating_sub(4),
        quote_cap: (max_height / quote_share).max(MIN_QUOTE_CAP),
    }
}

/// The heights of a sheet's parts, which [`ComposeLayout::new`] stacks. The heading and the tabs row are a row each
/// and always there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ComposeParts {
    /// The quote's rows, its `⋯` row included; 0 without one.
    pub quote: u16,
    /// The one-row fields' rows; 0 without any.
    pub fields: u16,
    /// The rows the body's text needs ([`super::ComposeBody::rows_needed`]).
    pub body_needed: u16,
    /// The fewest rows the body gets: 3 for a comment, 5 for a description.
    pub body_min: u16,
    /// The rows of the sections after the body (Create PR's Reviewers); 0 without any.
    pub sections: u16,
}

/// Where each part of a sheet goes, in rows from the top of the frame's interior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeLayout {
    /// The heading's row.
    pub heading: u16,
    /// The quote's rows, empty without one. A blank row follows it.
    pub quote: Range<u16>,
    /// The fields' rows, empty without any. A blank row precedes and follows them.
    pub fields: Range<u16>,
    /// The `Write  Preview` row.
    pub tabs: u16,
    /// The body's rows.
    pub body: Range<u16>,
    /// The sections' rows after a blank row, empty without any.
    pub sections: Range<u16>,
    /// The rows the parts take, which is more than the room when even the body's minimum doesn't fit.
    pub height: u16,
}

impl ComposeLayout {
    /// Stacks `parts` in `room` rows. The body is as tall as its text needs, at least its minimum, at most what's
    /// left once the other parts have theirs; with `fill` (expanded) it takes all that's left. When the minimum
    /// doesn't fit, the body keeps it and the parts take more than `room`: the dialog scrolls its interior.
    pub fn new(parts: &ComposeParts, room: u16, fill: bool) -> Self {
        let heading = 0;
        let mut y = 1;
        let quote = if parts.quote > 0 {
            let rows = y..y + parts.quote;
            y = rows.end + 1;
            rows
        } else {
            y..y
        };
        let fields = if parts.fields > 0 {
            if parts.quote == 0 {
                y += 1;
            }
            let rows = y..y + parts.fields;
            y = rows.end + 1;
            rows
        } else {
            y..y
        };
        let tabs = y;
        y += 1;
        let after = if parts.sections > 0 { parts.sections + 1 } else { 0 };
        let left = room.saturating_sub(y + after);
        let rows = if fill { left } else { parts.body_needed.min(left) };
        let body = y..y + rows.max(parts.body_min);
        y = body.end;
        let sections = if parts.sections > 0 {
            let rows = y + 1..y + 1 + parts.sections;
            y = rows.end;
            rows
        } else {
            y..y
        };
        Self {
            heading,
            quote,
            fields,
            tabs,
            body,
            sections,
            height: y,
        }
    }

    /// The body's height.
    pub fn body_rows(&self) -> u16 {
        self.body.end - self.body.start
    }
}

/// Where a sheet's frame was last drawn, so the next frame keeps its top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    screen: Rect,
    top: u16,
    expanded: bool,
}

/// The sheet frame's place on the screen. It opens centered for its height; after that its top stays put as it grows
/// or shrinks, and moves up only when its bottom would pass the margin row above the footer, never into the margin
/// row at the top. A resize, or expanding and collapsing, centers it again.
///
/// [`crate::modal::Dialog::frame`] takes `&self`, so the dialog [records](Self::record) each frame it draws, and the
/// next [`Self::place`] starts from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetPlacement {
    last: Option<Placed>,
}

impl SheetPlacement {
    /// The frame of a sheet of outer size `(width, height)` on `screen` (the footer row included), clipped to the rows
    /// it may take. Expanded, the whole screen above the footer.
    pub fn place(&self, screen: Rect, (width, height): (u16, u16), expanded: bool) -> Rect {
        let above_footer = screen.height.saturating_sub(1);
        if expanded {
            return Rect {
                height: above_footer,
                ..screen
            };
        }
        // A screen too short for the margins has none.
        let margin = if above_footer > 2 * MARGIN { MARGIN } else { 0 };
        let top_min = screen.y + margin;
        let bottom_max = screen.y + above_footer - margin;
        let height = height.min(bottom_max - top_min);
        let width = width.min(screen.width);
        let top = match self.last {
            Some(last) if last.screen == screen && !last.expanded => last.top.clamp(top_min, bottom_max - height),
            _ => top_min + (bottom_max - top_min - height) / 2,
        };
        Rect {
            x: screen.x + (screen.width - width) / 2,
            y: top,
            width,
            height,
        }
    }

    /// Notes that the frame was drawn at `frame` on `screen`.
    pub fn record(&mut self, screen: Rect, frame: Rect, expanded: bool) {
        self.last = Some(Placed {
            screen,
            top: frame.y,
            expanded,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Rect = Rect::new(0, 0, 160, 48);
    const SMALL: Rect = Rect::new(0, 0, 100, 30);

    #[test]
    fn the_sheet_fits_the_screen_less_the_footer_and_the_margins() {
        let wide = sheet_geometry(WIDE, 100, false);
        assert_eq!((wide.width, wide.max_height, wide.content_width), (100, 45, 96));
        assert_eq!((wide.room(), wide.quote_cap), (43, 7));
        let small = sheet_geometry(SMALL, 100, false);
        assert_eq!((small.width, small.max_height, small.content_width), (96, 27, 92));
        assert_eq!(small.quote_cap, 4);
        // Add comment's 80.
        assert_eq!(sheet_geometry(WIDE, 80, false).width, 80);
        // Expanded: the whole screen above the footer, the quote up to a third of it.
        let expanded = sheet_geometry(SMALL, 100, true);
        assert_eq!((expanded.width, expanded.max_height, expanded.quote_cap), (100, 29, 9));
        // A tiny screen still caps the quote at two rows.
        assert_eq!(sheet_geometry(Rect::new(0, 0, 48, 10), 100, false).quote_cap, 2);
    }

    fn comment(quote: u16, needed: u16) -> ComposeParts {
        ComposeParts {
            quote,
            body_needed: needed,
            body_min: 3,
            ..ComposeParts::default()
        }
    }

    #[test]
    fn an_empty_comment_gets_the_body_s_minimum() {
        let layout = ComposeLayout::new(&comment(0, 1), 43, false);
        assert_eq!((layout.heading, layout.tabs, layout.body.clone()), (0, 1, 2..5));
        assert_eq!(layout.height, 5);
        // A reply: the quote, then a blank row.
        let layout = ComposeLayout::new(&comment(1, 1), 43, false);
        assert_eq!(
            (layout.quote.clone(), layout.tabs, layout.body.clone()),
            (1..2, 3, 4..7)
        );
        assert_eq!(layout.height, 7);
    }

    #[test]
    fn the_body_grows_with_its_text_up_to_the_room() {
        for needed in [4, 20, 38] {
            let layout = ComposeLayout::new(&comment(0, needed), 43, false);
            assert_eq!(layout.body_rows(), needed);
            assert_eq!(layout.height, needed + 2);
        }
        // 41 rows are left under the heading and the tabs: the body stops there.
        let layout = ComposeLayout::new(&comment(0, 60), 43, false);
        assert_eq!((layout.body.clone(), layout.height), (2..43, 43));
        // With a capped quote of 7 rows (160x48), the body gets 33.
        let layout = ComposeLayout::new(&comment(7, 40), 43, false);
        assert_eq!((layout.body_rows(), layout.height), (33, 43));
        // At 100x30: a quote of 4 and 18 rows of body.
        let layout = ComposeLayout::new(&comment(4, 40), 25, false);
        assert_eq!((layout.body_rows(), layout.height), (18, 25));
    }

    #[test]
    fn filling_gives_the_body_every_row_left() {
        let layout = ComposeLayout::new(&comment(1, 1), 27, true);
        assert_eq!((layout.body.clone(), layout.height), (4..27, 27));
    }

    #[test]
    fn fields_and_sections_have_blank_rows_around_them() {
        let parts = ComposeParts {
            fields: 2,
            body_needed: 1,
            body_min: 5,
            sections: 1,
            ..ComposeParts::default()
        };
        let layout = ComposeLayout::new(&parts, 43, false);
        assert_eq!(layout.fields, 2..4);
        assert_eq!(layout.tabs, 5);
        assert_eq!(layout.body, 6..11);
        assert_eq!(layout.sections, 12..13);
        assert_eq!(layout.height, 13);
    }

    #[test]
    fn a_tiny_terminal_keeps_the_minimum_and_overflows() {
        // A 48x10 screen: 7 rows inside the frame, a reply needs 1 + 2 + 1 + 1 + 3.
        let room = sheet_geometry(Rect::new(0, 0, 48, 10), 100, false).room();
        assert_eq!(room, 5);
        let layout = ComposeLayout::new(&comment(2, 10), room, false);
        assert_eq!((layout.body_rows(), layout.height), (3, 8));
        assert!(layout.height > room);
    }

    #[test]
    fn the_sheet_opens_centered() {
        let placement = SheetPlacement::default();
        assert_eq!(placement.place(WIDE, (100, 7), false), Rect::new(30, 20, 100, 7));
        assert_eq!(placement.place(SMALL, (96, 9), false), Rect::new(2, 10, 96, 9));
        // As tall as it may be: from the row under the top margin to the one above the bottom margin.
        assert_eq!(placement.place(WIDE, (100, 45), false), Rect::new(30, 1, 100, 45));
        // Taller is clipped.
        assert_eq!(placement.place(WIDE, (100, 60), false), Rect::new(30, 1, 100, 45));
    }

    #[test]
    fn the_sheet_grows_down_keeping_its_top_then_up() {
        let mut placement = SheetPlacement::default();
        let first = placement.place(WIDE, (100, 7), false);
        placement.record(WIDE, first, false);
        // Down while there's room: the bottom may reach row 45, above the margin row 46.
        assert_eq!(placement.place(WIDE, (100, 20), false).y, 20);
        assert_eq!(placement.place(WIDE, (100, 26), false).y, 20);
        // Then up, never above row 1.
        assert_eq!(placement.place(WIDE, (100, 30), false).y, 16);
        assert_eq!(placement.place(WIDE, (100, 45), false).y, 1);
        let grown = placement.place(WIDE, (100, 30), false);
        placement.record(WIDE, grown, false);
        // Shrinking keeps the top.
        assert_eq!(placement.place(WIDE, (100, 7), false).y, 16);
    }

    #[test]
    fn a_resize_or_expanding_centers_it_again() {
        let mut placement = SheetPlacement::default();
        let grown = placement.place(WIDE, (100, 30), false);
        placement.record(WIDE, Rect { y: 2, ..grown }, false);
        assert_eq!(placement.place(WIDE, (100, 7), false).y, 2);
        // Another screen size: centered there.
        assert_eq!(placement.place(SMALL, (96, 7), false).y, 11);
        // Expanded: the whole screen above the footer.
        assert_eq!(placement.place(WIDE, (100, 7), true), Rect::new(0, 0, 160, 47));
        placement.record(WIDE, Rect::new(0, 0, 160, 47), true);
        // Collapsed again: centered.
        assert_eq!(placement.place(WIDE, (100, 7), false).y, 20);
    }

    #[test]
    fn a_screen_too_short_for_margins_has_none() {
        let placement = SheetPlacement::default();
        assert_eq!(
            placement.place(Rect::new(0, 0, 40, 3), (36, 9), false),
            Rect::new(2, 0, 36, 2)
        );
    }
}
