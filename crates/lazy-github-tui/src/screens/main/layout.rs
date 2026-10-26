//! Where each box of the main screen goes, with Textual's rounding.
//!
//! ratatui's `Layout` solver rounds differently from Textual, so this is plain integer arithmetic. A lone percentage
//! rounds down. The sections stack exact fractional heights (a focused one's 40%, then `1fr` shares) and each ends at
//! the floor of its exact bottom edge, so share *i* of *n* equal ones ends at `floor(i × R / n)`.

use ratatui::layout::Rect;

use super::{FocusTarget, SectionId, SectionState};

/// Rows of the status box.
const STATUS_HEIGHT: u16 = 3;
/// Rows of a compact (`(empty)`) section that isn't focused.
const COMPACT_HEIGHT: u16 = 3;

/// What the layout depends on.
#[derive(Debug, Clone, Copy)]
pub struct LayoutInput {
    /// The whole screen, or the part left of the help panel.
    pub area: Rect,
    /// The sections, in [`SectionId::ALL`] order.
    pub sections: [SectionState; 4],
    /// Where focus is, if on the main screen.
    pub focus: Option<FocusTarget>,
    /// Whether the command log is displayed.
    pub command_log: bool,
}

/// Where each box goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainLayout {
    pub status: Rect,
    /// The sections, in [`SectionId::ALL`] order; `None` for a hidden one.
    pub sections: [Option<Rect>; 4],
    pub details: Rect,
    pub command_log: Option<Rect>,
    pub footer: Rect,
}

/// Lays out the main screen.
pub fn layout(input: &LayoutInput) -> MainLayout {
    let area = input.area;
    let above_footer = area.height.saturating_sub(1);
    let status_height = if above_footer < 2 * STATUS_HEIGHT {
        above_footer / 2
    } else {
        STATUS_HEIGHT
    };
    let main = Rect {
        y: area.y + status_height,
        height: above_footer - status_height,
        ..area
    };
    let selections_width = percent(area.width, 40);
    let selections = Rect {
        width: selections_width,
        ..main
    };
    let details_column = Rect {
        x: area.x + selections_width,
        width: area.width - selections_width,
        ..main
    };

    let focused_section = input.focus.and_then(FocusTarget::section);
    let heights = section_heights(main.height, &input.sections, focused_section);
    let mut sections = [None; 4];
    let mut y = main.y;
    for (slot, height) in sections.iter_mut().zip(heights) {
        if let Some(height) = height {
            *slot = Some(clip(
                Rect {
                    y,
                    height,
                    ..selections
                },
                main,
            ));
            y = y.saturating_add(height);
        }
    }

    let (details, command_log) = if input.command_log {
        let percent_of_main = if input.focus == Some(FocusTarget::CommandLog) {
            40
        } else {
            20
        };
        let log_height = percent(main.height, percent_of_main);
        let details_height = main.height - log_height;
        let log = Rect {
            y: main.y + details_height,
            height: log_height,
            ..details_column
        };
        (
            Rect {
                height: details_height,
                ..details_column
            },
            Some(log),
        )
    } else {
        (details_column, None)
    };

    MainLayout {
        status: Rect {
            height: status_height,
            ..area
        },
        sections,
        details,
        command_log,
        footer: Rect {
            y: area.y + above_footer,
            height: area.height.min(1),
            ..area
        },
    }
}

/// Fractions of a row the section heights are worked out in: a multiple of 5 for the 40% rule and of every share
/// count up to 4, so every exact height below is a whole number of these.
const SUBROWS: u32 = 120;

/// The height of each displayed section, in [`SectionId::ALL`] order. As in Textual, the heights are exact fractions
/// (a focused section's 40% of 44 rows is 17.6) and each box ends at the floor of its exact bottom edge, so the
/// leftover fractions go to the boxes below rather than being dropped from the focused one.
fn section_heights(total: u16, sections: &[SectionState; 4], focused: Option<SectionId>) -> [Option<u16>; 4] {
    let total_exact = u32::from(total) * SUBROWS;
    let min_focused = total_exact * 2 / 5;
    let compact_height = u32::from(COMPACT_HEIGHT) * SUBROWS;
    let is_focused = |i: usize| focused.is_some_and(|id| id.index() == i && sections[i].shown());
    let mut exact: [Option<u32>; 4] = [None; 4];

    // Fixed heights first: compact sections, and a focused compact one under the 40% rule.
    let mut fixed = 0u32;
    for (i, section) in sections.iter().enumerate() {
        if section.shown() && section.compact {
            let height = if is_focused(i) {
                compact_height.max(min_focused)
            } else {
                compact_height
            };
            exact[i] = Some(height);
            fixed += height;
        }
    }
    let rest = total_exact.saturating_sub(fixed);
    let flexible: Vec<usize> = (0..4)
        .filter(|&i| sections[i].shown() && !sections[i].compact)
        .collect();

    // A focused flexible section whose share is under 40% takes 40%, and the others share what's left.
    let focused_flexible = flexible.iter().position(|&i| is_focused(i));
    let mut sharing = flexible.clone();
    let mut shared = rest;
    if let Some(position) = focused_flexible {
        let n = u32::try_from(flexible.len()).unwrap_or(u32::MAX);
        if rest < min_focused.saturating_mul(n) {
            let height = min_focused.min(rest);
            exact[sharing.remove(position)] = Some(height);
            shared = rest - height;
        }
    }
    let n = u32::try_from(sharing.len()).unwrap_or(u32::MAX).max(1);
    for i in sharing {
        exact[i] = Some(shared / n);
    }

    // Each box ends at the floor of its exact bottom edge.
    let mut heights = [None; 4];
    let mut bottom = 0u32;
    for (height, exact) in heights.iter_mut().zip(exact) {
        if let Some(exact) = exact {
            let top = bottom / SUBROWS;
            bottom += exact;
            *height = Some(u16::try_from(bottom / SUBROWS - top).unwrap_or(u16::MAX));
        }
    }
    heights
}

/// `floor(value × percent / 100)`, as Textual rounds percentages.
fn percent(value: u16, percent: u32) -> u16 {
    // At most `value`, since `percent` is at most 100.
    u16::try_from(u32::from(value) * percent / 100).unwrap_or(value)
}

/// `rect` cut to fit inside `within`, so tiny screens draw what fits instead of overflowing.
fn clip(rect: Rect, within: Rect) -> Rect {
    let y = rect.y.min(within.bottom());
    Rect {
        y,
        height: rect.height.min(within.bottom() - y),
        ..rect
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const SHOWN: SectionState = SectionState {
        displayed: true,
        hidden_by_toggle: false,
        compact: false,
    };
    const HIDDEN: SectionState = SectionState {
        displayed: false,
        ..SHOWN
    };
    const COMPACT: SectionState = SectionState { compact: true, ..SHOWN };

    fn rows(rect: Option<Rect>) -> Option<(u16, u16)> {
        rect.map(|r| (r.y, r.bottom() - 1))
    }

    fn run(size: (u16, u16), sections: [SectionState; 4], focus: FocusTarget, command_log: bool) -> MainLayout {
        layout(&LayoutInput {
            area: Rect::new(0, 0, size.0, size.1),
            sections,
            focus: Some(focus),
            command_log,
        })
    }

    type Rows = Option<(u16, u16)>;

    /// The geometry table in #74: rows (inclusive) of each section, details and the log.
    #[rstest]
    #[case::default_repos_160((160, 48), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), false,
        [Some((3, 19)), Some((20, 28)), Some((29, 37)), Some((38, 46))], (3, 46), None)]
    // 40% of 44 is 17.6 and the others share 26.4: Python's 8, 9, 18 and 9 rows, from the edges' floors.
    #[case::default_issues_160((160, 48), [SHOWN; 4], FocusTarget::Table(SectionId::Issues), false,
        [Some((3, 10)), Some((11, 19)), Some((20, 37)), Some((38, 46))], (3, 46), None)]
    // repo-lists' `issues-page-2`: 13.2, 13.2 and 17.6 rows end up 13, 13 and 18.
    #[case::no_workflows_issues_160((160, 48), [SHOWN, SHOWN, SHOWN, HIDDEN], FocusTarget::Table(SectionId::Issues),
        false, [Some((3, 15)), Some((16, 28)), Some((29, 46)), None], (3, 46), None)]
    #[case::default_details_160((160, 48), [SHOWN; 4], FocusTarget::DetailsTabs, false,
        [Some((3, 13)), Some((14, 24)), Some((25, 35)), Some((36, 46))], (3, 46), None)]
    #[case::log_repos_160((160, 48), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), true,
        [Some((3, 19)), Some((20, 28)), Some((29, 37)), Some((38, 46))], (3, 38), Some((39, 46)))]
    #[case::log_focused_160((160, 48), [SHOWN; 4], FocusTarget::CommandLog, true,
        [Some((3, 13)), Some((14, 24)), Some((25, 35)), Some((36, 46))], (3, 29), Some((30, 46)))]
    #[case::default_repos_100((100, 30), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), false,
        [Some((3, 12)), Some((13, 17)), Some((18, 22)), Some((23, 28))], (3, 28), None)]
    #[case::default_prs_100((100, 30), [SHOWN; 4], FocusTarget::Table(SectionId::PullRequests), false,
        [Some((3, 7)), Some((8, 17)), Some((18, 22)), Some((23, 28))], (3, 28), None)]
    #[case::default_details_100((100, 30), [SHOWN; 4], FocusTarget::DetailsContent, false,
        [Some((3, 8)), Some((9, 15)), Some((16, 21)), Some((22, 28))], (3, 28), None)]
    #[case::log_repos_100((100, 30), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), true,
        [Some((3, 12)), Some((13, 17)), Some((18, 22)), Some((23, 28))], (3, 23), Some((24, 28)))]
    #[case::no_prs_100((100, 30), [SHOWN, HIDDEN, SHOWN, SHOWN], FocusTarget::Table(SectionId::Repos), false,
        [Some((3, 12)), None, Some((13, 20)), Some((21, 28))], (3, 28), None)]
    #[case::repos_and_prs_100((100, 30), [SHOWN, SHOWN, HIDDEN, HIDDEN], FocusTarget::Table(SectionId::Repos),
        false, [Some((3, 15)), Some((16, 28)), None, None], (3, 28), None)]
    #[case::only_repos_100((100, 30), [SHOWN, HIDDEN, HIDDEN, HIDDEN], FocusTarget::Table(SectionId::Repos),
        false, [Some((3, 28)), None, None, None], (3, 28), None)]
    fn geometry_table(
        #[case] size: (u16, u16),
        #[case] sections: [SectionState; 4],
        #[case] focus: FocusTarget,
        #[case] command_log: bool,
        #[case] expected: [Rows; 4],
        #[case] details: (u16, u16),
        #[case] log: Rows,
    ) {
        let got = run(size, sections, focus, command_log);
        assert_eq!(got.sections.map(rows), expected);
        assert_eq!(rows(Some(got.details)), Some(details));
        assert_eq!(rows(got.command_log), log);
        assert_eq!(got.status, Rect::new(0, 0, size.0, 3));
        assert_eq!(got.footer, Rect::new(0, size.1 - 1, size.0, 1));
    }

    #[rstest]
    #[case::wide(160, 64)]
    #[case::narrow(100, 40)]
    #[case::help_panel_open(108, 43)]
    fn the_selections_take_40_percent_of_the_width(#[case] width: u16, #[case] selections: u16) {
        let got = run((width, 48), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), true);
        for rect in got.sections.into_iter().flatten() {
            assert_eq!((rect.x, rect.width), (0, selections));
        }
        assert_eq!((got.details.x, got.details.width), (selections, width - selections));
        let log = got.command_log.unwrap();
        assert_eq!((log.x, log.width), (selections, width - selections));
    }

    #[test]
    fn compact_sections_are_three_rows_and_the_others_share_the_rest() {
        let sections = [SHOWN, COMPACT, COMPACT, SHOWN];
        let got = run((100, 30), sections, FocusTarget::DetailsTabs, false);
        // 26 rows: 3 + 3 for the compact ones, then 20 split in two.
        assert_eq!(
            got.sections.map(rows),
            [Some((3, 12)), Some((13, 15)), Some((16, 18)), Some((19, 28))]
        );
        // With repos focused, its exact share (10) is already 40% of 26.
        let got = run((100, 30), sections, FocusTarget::Table(SectionId::Repos), false);
        assert_eq!(
            got.sections.map(rows),
            [Some((3, 12)), Some((13, 15)), Some((16, 18)), Some((19, 28))]
        );
    }

    #[test]
    fn a_focused_compact_section_gets_40_percent() {
        let sections = [SHOWN, COMPACT, COMPACT, SHOWN];
        let got = run((100, 30), sections, FocusTarget::Search(SectionId::PullRequests), false);
        // PRs take floor(0.4 × 26) = 10; issues 3; repos and workflows share 13.
        assert_eq!(
            got.sections.map(rows),
            [Some((3, 8)), Some((9, 18)), Some((19, 21)), Some((22, 28))]
        );
    }

    #[test]
    fn focus_on_a_hidden_section_focuses_nothing() {
        let sections = [SHOWN, HIDDEN, SHOWN, SHOWN];
        let got = run((100, 30), sections, FocusTarget::Table(SectionId::PullRequests), false);
        // An even three-way split of 26: 8, 9, 9.
        assert_eq!(
            got.sections.map(rows),
            [Some((3, 10)), None, Some((11, 19)), Some((20, 28))]
        );
    }

    #[test]
    fn a_section_hidden_by_toggle_leaves_the_layout() {
        let toggled = SectionState {
            hidden_by_toggle: true,
            ..SHOWN
        };
        let got = run(
            (100, 30),
            [SHOWN, toggled, toggled, toggled],
            FocusTarget::Table(SectionId::Repos),
            false,
        );
        assert_eq!(got.sections.map(rows), [Some((3, 28)), None, None, None]);
    }

    #[rstest]
    #[case(20, 8)]
    #[case(10, 4)]
    #[case(1, 1)]
    #[case(0, 0)]
    #[case(5, 2)]
    fn tiny_sizes_stay_on_screen(#[case] width: u16, #[case] height: u16) {
        for (sections, log) in [
            ([SHOWN; 4], true),
            ([COMPACT; 4], false),
            ([SHOWN, COMPACT, HIDDEN, SHOWN], true),
        ] {
            for focus in [FocusTarget::Table(SectionId::Issues), FocusTarget::CommandLog] {
                let got = run((width, height), sections, focus, log);
                let screen = Rect::new(0, 0, width, height);
                let rects = [got.status, got.details, got.footer]
                    .into_iter()
                    .chain(got.sections.into_iter().flatten())
                    .chain(got.command_log);
                for rect in rects {
                    assert_eq!(rect.intersection(screen), rect, "{rect:?} at {width}x{height}");
                }
            }
        }
    }

    #[test]
    fn short_screens_halve_the_status_box() {
        let got = run((10, 4), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), false);
        assert_eq!(got.status.height, 1);
        assert_eq!(got.details, Rect::new(4, 1, 6, 2));
        let got = run((20, 8), [SHOWN; 4], FocusTarget::Table(SectionId::Repos), false);
        assert_eq!(got.status.height, 3);
        assert_eq!(got.details, Rect::new(8, 3, 12, 4));
    }
}
