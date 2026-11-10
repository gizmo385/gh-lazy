//! Where the Create/Edit PR sheet's parts go (decision compose-dialogs): its rows, from the shared
//! [`ComposeLayout`], and the columns of the one-row branch row, `Base <base> ← Compare <head>  Draft [ ]`.

use std::ops::Range;

use ratatui::layout::Rect;
use ratatui::text::Span;

use super::reviewers;
use crate::components::compose::{ComposeLayout, label_column as widest_label};
use crate::components::switch::Switch;

/// The labels in the label column.
pub(super) const BASE_LABEL: &str = "Base";
pub(super) const TITLE_LABEL: &str = "Title";
/// The labels between the branch row's boxes, with the spaces around them: ` ← Compare ` and `  Draft `.
pub(super) const COMPARE_ARROW: &str = " ← ";
pub(super) const COMPARE_LABEL: &str = "Compare ";
pub(super) const DRAFT_GAP: u16 = 2;
pub(super) const DRAFT_LABEL: &str = "Draft ";
/// The tabs row's label, before `Write  Preview`, and where the tabs start: after it and a space, which with the
/// tab's own padding leaves two cells before `Write`, as on the issue dialogs.
pub(super) const DESCRIPTION_LABEL: &str = "Description";
pub(super) const TABS_COLUMN: u16 = 12;

/// The label column's width: the widest of Base, Title and Reviewers, then a gap.
pub(super) fn label_column() -> u16 {
    widest_label([BASE_LABEL, TITLE_LABEL, reviewers::LABEL])
}

/// The columns `text` takes.
pub(super) fn text_width(text: &str) -> u16 {
    u16::try_from(Span::raw(text).width()).unwrap_or(u16::MAX)
}

/// The sheet's rows, from the top of its interior: the shared layout, with the upstream warning in the quote's place
/// (a row under the heading, then a blank row) and the Reviewers section after the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SheetRows {
    pub(super) layout: ComposeLayout,
}

impl SheetRows {
    /// The warning's row, while it shows.
    pub(super) fn warning(&self) -> Option<u16> {
        (!self.layout.quote.is_empty()).then_some(self.layout.quote.start)
    }

    /// The Base, Compare and Draft row.
    pub(super) fn branches(&self) -> u16 {
        self.layout.fields.start
    }

    /// The Title row.
    pub(super) fn title(&self) -> u16 {
        self.layout.fields.start + 1
    }

    /// The Reviewers section's rows.
    pub(super) fn reviewers(&self) -> Range<u16> {
        self.layout.sections.clone()
    }
}

/// The branch row's columns, left to right, in a row `area` wide: the Base label in the label column, the two boxes
/// sharing what the labels and the switch leave, the Compare label between them, then Draft and its switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BranchColumns {
    pub(super) base_label: Rect,
    pub(super) base: Rect,
    pub(super) compare_label: Rect,
    pub(super) compare: Rect,
    pub(super) draft_label: Rect,
    pub(super) draft: Rect,
}

impl BranchColumns {
    pub(super) fn new(area: Rect, label_column: u16) -> Self {
        let compare_label = text_width(COMPARE_ARROW) + text_width(COMPARE_LABEL);
        let draft_label = DRAFT_GAP + text_width(DRAFT_LABEL);
        let fixed = label_column + compare_label + draft_label + Switch::COMPACT_WIDTH;
        let boxes = area.width.saturating_sub(fixed);
        let base = boxes / 2;
        let mut x = area.x;
        let mut next = |width: u16| {
            let rect = Rect {
                x,
                width,
                height: 1,
                ..area
            }
            .intersection(area);
            x = x.saturating_add(width);
            rect
        };
        Self {
            base_label: next(label_column),
            base: next(base),
            compare_label: next(compare_label),
            compare: next(boxes - base),
            draft_label: next(draft_label),
            draft: next(Switch::COMPACT_WIDTH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boxes_share_what_the_labels_and_the_switch_leave() {
        assert_eq!(label_column(), 11);
        // At 160x48 the parts are 96 columns wide, from column 32.
        let columns = BranchColumns::new(Rect::new(32, 4, 96, 1), 11);
        assert_eq!(columns.base_label, Rect::new(32, 4, 11, 1));
        assert_eq!(columns.base, Rect::new(43, 4, 31, 1));
        assert_eq!(columns.compare_label, Rect::new(74, 4, 11, 1));
        assert_eq!(columns.compare, Rect::new(85, 4, 32, 1));
        assert_eq!(columns.draft_label, Rect::new(117, 4, 8, 1));
        assert_eq!(columns.draft, Rect::new(125, 4, 3, 1));
        assert_eq!(columns.draft.right(), 128);
        // Too narrow for the boxes: they're empty and nothing passes the row's end.
        let narrow = BranchColumns::new(Rect::new(0, 0, 30, 1), 11);
        assert!(narrow.base.is_empty() && narrow.compare.is_empty());
        assert!(narrow.draft.right() <= 30);
    }
}
