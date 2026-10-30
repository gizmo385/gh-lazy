//! The links drawn on screen in one frame, with where they landed: what a click on a link (#339) and the link hints
//! (#340) look up. Views that draw linked text record into the frame's map as they draw, so the map holds exactly what
//! the user sees, with no walk over the widgets afterwards.

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

use ratatui::layout::Rect;

use super::markdown::LinkSpan;

/// A link's text on one screen row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenLink {
    /// The column of its first cell.
    pub x: u16,
    /// Its row.
    pub y: u16,
    /// How many cells it covers on that row.
    pub width: u16,
    /// Where it goes.
    pub target: Arc<str>,
}

impl ScreenLink {
    /// Whether the cell at `(x, y)` is one of the link's.
    pub fn contains(&self, x: u16, y: u16) -> bool {
        y == self.y && (self.x..self.x.saturating_add(self.width)).contains(&x)
    }
}

/// The links of one frame, in the order they were drawn: top to bottom within each view, each row left to right. A
/// link that wraps is one entry per row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkMap {
    links: Vec<ScreenLink>,
}

/// The map a frame records into while it draws (`RenderCx::links`): views share the render context by reference.
pub type LinkSink = RefCell<LinkMap>;

impl LinkMap {
    /// The link drawn at `(x, y)`, if any. The last one drawn there wins, as it's the one on top.
    pub fn at(&self, x: u16, y: u16) -> Option<&ScreenLink> {
        self.links.iter().rev().find(|link| link.contains(x, y))
    }

    /// Every link, in draw order.
    pub fn iter(&self) -> impl Iterator<Item = &ScreenLink> {
        self.links.iter()
    }

    /// Whether nothing on screen is a link.
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Records `spans` of content scrolled to `offset` (its first visible row) and drawn in `area`: each span's row
    /// becomes a screen row, and only the cells inside `area` are kept, so rows scrolled out above or below and
    /// columns past the right edge are left out.
    pub fn record(&mut self, area: Rect, offset: usize, spans: &[LinkSpan]) {
        let rows = offset..offset.saturating_add(usize::from(area.height));
        for span in spans.iter().filter(|span| rows.contains(&span.row)) {
            let Some(cells) = clip(&span.columns, area.width) else {
                continue;
            };
            // The row is within the area's height, a u16.
            let dy = u16::try_from(span.row - offset).unwrap_or(u16::MAX);
            self.links.push(ScreenLink {
                x: area.x.saturating_add(cells.start),
                y: area.y.saturating_add(dy),
                width: cells.end - cells.start,
                target: Arc::clone(&span.target),
            });
        }
    }

    /// Records one link drawn at `link`'s cells as they are.
    pub fn push(&mut self, link: ScreenLink) {
        self.links.push(link);
    }
}

/// `columns` cut to the first `width` columns, or `None` when nothing is left.
fn clip(columns: &Range<u16>, width: u16) -> Option<Range<u16>> {
    let end = columns.end.min(width);
    (columns.start < end).then_some(columns.start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(row: usize, columns: Range<u16>, target: &str) -> LinkSpan {
        LinkSpan {
            row,
            columns,
            target: target.into(),
        }
    }

    fn link(x: u16, y: u16, width: u16, target: &str) -> ScreenLink {
        ScreenLink {
            x,
            y,
            width,
            target: target.into(),
        }
    }

    fn recorded(area: Rect, offset: usize, spans: &[LinkSpan]) -> Vec<ScreenLink> {
        let mut map = LinkMap::default();
        map.record(area, offset, spans);
        map.iter().cloned().collect()
    }

    #[test]
    fn spans_move_by_the_area_and_the_scroll_offset() {
        let area = Rect::new(10, 5, 20, 4);
        let spans = [span(3, 2..6, "https://a"), span(4, 0..3, "https://b")];
        assert_eq!(
            recorded(area, 2, &spans),
            [link(12, 6, 4, "https://a"), link(10, 7, 3, "https://b")]
        );
    }

    #[test]
    fn rows_scrolled_out_above_or_below_are_left_out() {
        let area = Rect::new(0, 0, 20, 3);
        let spans = [
            span(0, 0..4, "https://above"),
            span(1, 0..4, "https://first"),
            span(3, 0..4, "https://last"),
            span(4, 0..4, "https://below"),
        ];
        let targets: Vec<String> = recorded(area, 1, &spans).iter().map(|l| l.target.to_string()).collect();
        assert_eq!(targets, ["https://first", "https://last"]);
    }

    #[test]
    fn links_are_cut_at_the_right_edge() {
        let area = Rect::new(0, 0, 10, 2);
        let spans = [span(0, 6..14, "https://cut"), span(1, 10..14, "https://past")];
        assert_eq!(recorded(area, 0, &spans), [link(6, 0, 4, "https://cut")]);
    }

    #[test]
    fn a_wrapped_link_is_one_entry_per_row() {
        let area = Rect::new(0, 0, 10, 5);
        let spans = [span(0, 5..10, "https://long"), span(1, 0..4, "https://long")];
        assert_eq!(
            recorded(area, 0, &spans),
            [link(5, 0, 5, "https://long"), link(0, 1, 4, "https://long")]
        );
    }

    #[test]
    fn at_hits_only_the_links_cells() {
        let mut map = LinkMap::default();
        map.push(link(4, 2, 3, "https://a"));
        assert_eq!(map.at(4, 2).map(|l| &*l.target), Some("https://a"));
        assert_eq!(map.at(6, 2).map(|l| &*l.target), Some("https://a"));
        assert_eq!(map.at(7, 2), None);
        assert_eq!(map.at(3, 2), None);
        assert_eq!(map.at(4, 1), None);
        assert!(!map.is_empty());
        assert!(LinkMap::default().is_empty());
    }
}
