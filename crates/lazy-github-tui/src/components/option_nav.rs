//! The highlight arithmetic of Textual's `OptionList`, shared by the lists built on it: a select's open list,
//! `SelectionList` and the command palette's hits. `up` and `down` wrap at the ends; the page keys move by the rows in
//! view and stop there (`OptionList._move_page`).

/// `down` from `highlight` in a list of `len` options: the next one, wrapping to the first.
pub fn next(highlight: usize, len: usize) -> usize {
    if highlight + 1 >= len { 0 } else { highlight + 1 }
}

/// `up` from `highlight` in a list of `len` options: the previous one, wrapping to the last.
pub fn previous(highlight: usize, len: usize) -> usize {
    highlight.checked_sub(1).unwrap_or(len.saturating_sub(1))
}

/// `pagedown` from `highlight` in a list of `len` options, each `rows` rows tall, with `view` rows in view: the option
/// on the row `view` below the highlight's first, or the last one.
pub fn page_down(highlight: usize, len: usize, rows: usize, view: usize) -> usize {
    let rows = rows.max(1);
    let last_row = (len * rows).saturating_sub(1);
    (highlight * rows + view).min(last_row) / rows
}

/// `pageup` from `highlight` in a list of options `rows` rows tall, with `view` rows in view: the option on the row
/// `view` above the highlight's first, or the first one.
pub fn page_up(highlight: usize, rows: usize, view: usize) -> usize {
    let rows = rows.max(1);
    (highlight * rows).saturating_sub(view) / rows
}

/// The scroll offset that brings rows `first..first + height` into a window of `window` rows now scrolled to
/// `scroll`, moving it as little as possible. When they don't fit, their first row is at the top.
pub fn scroll_into_view(scroll: usize, first: usize, height: usize, window: usize) -> usize {
    if first < scroll {
        first
    } else if first + height > scroll + window {
        (first + height).saturating_sub(window).min(first)
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_and_down_wrap() {
        assert_eq!(next(0, 3), 1);
        assert_eq!(next(2, 3), 0);
        assert_eq!(previous(1, 3), 0);
        assert_eq!(previous(0, 3), 2);
        assert_eq!(next(0, 0), 0);
        assert_eq!(previous(0, 0), 0);
    }

    #[test]
    fn pages_move_by_the_rows_in_view_and_stop_at_the_ends() {
        // One row per option, five in view.
        assert_eq!(page_down(0, 20, 1, 5), 5);
        assert_eq!(page_down(17, 20, 1, 5), 19);
        assert_eq!(page_up(7, 1, 5), 2);
        assert_eq!(page_up(3, 1, 5), 0);
        // Two rows per option, 31 rows in view: a page is 15 options down, and 16 up (the row lands mid-option).
        assert_eq!(page_down(0, 40, 2, 31), 15);
        assert_eq!(page_down(30, 40, 2, 31), 39);
        assert_eq!(page_up(20, 2, 31), 4);
        assert_eq!(page_up(10, 2, 31), 0);
        assert_eq!(page_down(0, 0, 2, 31), 0);
    }

    #[test]
    fn scrolling_brings_the_rows_into_view() {
        assert_eq!(scroll_into_view(0, 3, 1, 5), 0);
        assert_eq!(scroll_into_view(0, 5, 1, 5), 1);
        assert_eq!(scroll_into_view(4, 2, 1, 5), 2);
        // A two-row option half below the window.
        assert_eq!(scroll_into_view(0, 30, 2, 31), 1);
        // Taller than the window: its first row at the top.
        assert_eq!(scroll_into_view(0, 4, 3, 2), 4);
    }
}
