//! Where each part of the Create/Edit PR form goes: rows from the top of the scroll area's content, and the columns
//! of the branch row. Python's `CreateOrEditPullRequestContainer` children stacked in a `VerticalScroll`, with
//! Textual's margins (adjacent vertical margins collapse: the editor's `margin-bottom: 1` and the rule's top margin
//! share one row).

use std::ops::Range;

use ratatui::layout::Rect;

use crate::components::button::ButtonRow;
use crate::components::switch::Switch;
use crate::components::text_input;

/// The `Markdown` heading's block: `MarkdownH1`'s two rows of margin above and one below, then the `Markdown`
/// widget's `margin-bottom: 1`.
const HEADING_ROWS: u16 = 5;
/// The heading's row in its block.
pub(super) const HEADING_ROW: u16 = 2;
/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
pub(super) const HEADING_PADDING: u16 = 2;

/// The upstream warning: the label and its `margin-bottom: 1`.
const WARNING_ROWS: u16 = 2;

/// A `Rule`: its line with `margin: 1 0`.
const RULE_ROWS: u16 = 3;

/// `#pr_description { height: 30 }`.
pub(super) const EDITOR_ROWS: u16 = 30;

/// `BranchSelection Input { width: 30% }`.
const BRANCH_INPUT_PERCENT: u16 = 30;

/// The branch row's labels.
pub(super) const BASE_LABEL: &str = "Base";
pub(super) const COMPARE_ARROW: &str = "← ";
pub(super) const COMPARE_LABEL: &str = "Compare";
pub(super) const DRAFT_LABEL: &str = "Draft";

/// A part of the form, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Heading,
    /// The upstream warning, while it shows.
    Warning,
    /// The Base, Compare and Draft row.
    Branches,
    /// The rule under the branches.
    FirstRule,
    TitleLabel,
    Title,
    DescriptionLabel,
    Description,
    /// The rule under the description.
    SecondRule,
    Reviewers,
    /// The button row, with the buttons' margins.
    Buttons,
}

/// The rows each part takes, from the content's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FormRows {
    parts: Vec<(Part, Range<u16>)>,
    height: u16,
}

impl FormRows {
    /// The form with the upstream warning or not, and a Reviewers field `reviewers` rows tall.
    pub(super) fn new(warning: bool, reviewers: u16) -> Self {
        let mut top = 0u16;
        let mut parts = Vec::new();
        let mut place = |part: Option<Part>, rows: u16| {
            if let Some(part) = part {
                parts.push((part, top..top + rows));
            }
            top += rows;
        };
        place(Some(Part::Heading), HEADING_ROWS);
        if warning {
            place(Some(Part::Warning), WARNING_ROWS);
        }
        place(Some(Part::Branches), text_input::HEIGHT);
        place(Some(Part::FirstRule), RULE_ROWS);
        place(Some(Part::TitleLabel), 1);
        place(Some(Part::Title), text_input::HEIGHT);
        // `#pr_title { margin-bottom: 1 }`.
        place(None, 1);
        place(Some(Part::DescriptionLabel), 1);
        place(Some(Part::Description), EDITOR_ROWS);
        // The editor's `margin-bottom: 1` collapses into the rule's top margin.
        place(Some(Part::SecondRule), RULE_ROWS);
        place(Some(Part::Reviewers), reviewers);
        place(Some(Part::Buttons), ButtonRow::HEIGHT);
        let height = parts.last().map_or(0, |(_, rows)| rows.end);
        Self { parts, height }
    }

    /// The content's height: every part's rows.
    pub(super) fn height(&self) -> u16 {
        self.height
    }

    /// The rows `part` takes, if it shows.
    pub(super) fn rows(&self, part: Part) -> Option<Range<u16>> {
        self.parts
            .iter()
            .find_map(|(p, rows)| (*p == part).then(|| rows.clone()))
    }

    /// The part on content row `row`, if any.
    pub(super) fn part_at(&self, row: u16) -> Option<Part> {
        self.parts
            .iter()
            .find_map(|(part, rows)| rows.contains(&row).then_some(*part))
    }

    /// Where `part` goes in `content`, the scroll area's offscreen content: its rows, full width. Empty when it
    /// doesn't show.
    pub(super) fn area(&self, part: Part, content: Rect) -> Rect {
        self.rows(part).map_or_else(Rect::default, |rows| Rect {
            y: content.y + rows.start,
            height: rows.end - rows.start,
            ..content
        })
    }
}

/// The branch row's columns: each label and box, left to right, in a row `area` wide. A `Horizontal` of
/// auto-width labels, 30%-wide inputs and the switch.
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
    pub(super) fn new(area: Rect) -> Self {
        let input = area.width * BRANCH_INPUT_PERCENT / 100;
        let mut x = area.x;
        let mut next = |width: u16| {
            let rect = Rect { x, width, ..area }.intersection(area);
            x = x.saturating_add(width);
            rect
        };
        let width = |text: &str| u16::try_from(ratatui::text::Span::raw(text).width()).unwrap_or(u16::MAX);
        Self {
            base_label: next(width(BASE_LABEL)),
            base: next(input),
            compare_label: next(width(COMPARE_ARROW) + width(COMPARE_LABEL)),
            compare: next(input),
            draft_label: next(width(DRAFT_LABEL)),
            draft: next(Switch::WIDTH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_stack_with_the_collapsed_margin() {
        // Create PR without reviewers: the label and the input.
        let rows = FormRows::new(false, 4);
        assert_eq!(rows.rows(Part::Heading), Some(0..5));
        assert_eq!(rows.rows(Part::Warning), None);
        assert_eq!(rows.rows(Part::Branches), Some(5..8));
        assert_eq!(rows.rows(Part::TitleLabel), Some(11..12));
        assert_eq!(rows.rows(Part::Title), Some(12..15));
        assert_eq!(rows.rows(Part::DescriptionLabel), Some(16..17));
        assert_eq!(rows.rows(Part::Description), Some(17..47));
        assert_eq!(rows.rows(Part::SecondRule), Some(47..50));
        assert_eq!(rows.rows(Part::Reviewers), Some(50..54));
        assert_eq!(rows.rows(Part::Buttons), Some(54..59));
        // Textual's virtual height, which the scrollbar's thumb shows.
        assert_eq!(rows.height(), 59);
        assert_eq!(rows.part_at(15), None);
        assert_eq!(rows.part_at(20), Some(Part::Description));
        let warned = FormRows::new(true, 4);
        assert_eq!(warned.rows(Part::Warning), Some(5..7));
        assert_eq!(warned.rows(Part::Branches), Some(7..10));
        assert_eq!(warned.height(), 61);
    }

    #[test]
    fn the_branch_row_is_labels_and_30_percent_inputs() {
        // At 160x48 the content is 94 columns wide, from column 32.
        let columns = BranchColumns::new(Rect::new(32, 5, 94, 3));
        assert_eq!(columns.base_label, Rect::new(32, 5, 4, 3));
        assert_eq!(columns.base, Rect::new(36, 5, 28, 3));
        assert_eq!(columns.compare_label, Rect::new(64, 5, 9, 3));
        assert_eq!(columns.compare, Rect::new(73, 5, 28, 3));
        assert_eq!(columns.draft_label, Rect::new(101, 5, 5, 3));
        assert_eq!(columns.draft, Rect::new(106, 5, 10, 3));
    }
}
