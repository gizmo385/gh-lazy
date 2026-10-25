//! `<capture>.diff.txt`: a unified diff of the two apps' plain-text captures. Informational only; the structural
//! check (#93) is what gates.

use similar::TextDiff;

/// A unified diff (3 lines of context) of the Python capture against the Rust one, headed `--- python` and
/// `+++ rust`. Empty when the two are identical.
pub fn text_diff(py: &str, rs: &str) -> String {
    TextDiff::from_lines(py, rs)
        .unified_diff()
        .context_radius(3)
        .header("python", "rust")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_two_small_grids() {
        let py =
            "┌─ [1] Repositories ─┐\n│ alpha              │\n│ beta               │\n└────────────────────┘\nq Quit\n";
        let rs = "┌─ [1] Repositories ─┐\n│ alpha              │\n│ gamma              │\n└────────────────────┘\nq Quit  ? Help\n";
        insta::assert_snapshot!(text_diff(py, rs));
    }

    #[test]
    fn identical_captures_diff_to_nothing() {
        assert_eq!(text_diff("a\nb\n", "a\nb\n"), "");
    }
}
