//! Unified diffs of plain text: a run against its snapshot, or one accept run against another.

use similar::TextDiff;

/// A unified diff (3 lines of context) of `a` against `b`, headed `--- <a_label>` and `+++ <b_label>`. Empty when
/// the two are identical.
pub fn labeled_diff(a: &str, b: &str, a_label: &str, b_label: &str) -> String {
    TextDiff::from_lines(a, b)
        .unified_diff()
        .context_radius(3)
        .header(a_label, b_label)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_two_small_grids() {
        let a =
            "┌─ [1] Repositories ─┐\n│ alpha              │\n│ beta               │\n└────────────────────┘\nq Quit\n";
        let b = "┌─ [1] Repositories ─┐\n│ alpha              │\n│ gamma              │\n└────────────────────┘\nq Quit  ? Help\n";
        insta::assert_snapshot!(labeled_diff(a, b, "snapshot", "run"));
    }

    #[test]
    fn identical_text_diffs_to_nothing() {
        assert_eq!(labeled_diff("a\nb\n", "a\nb\n", "snapshot", "run"), "");
    }
}
