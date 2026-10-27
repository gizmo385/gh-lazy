//! Numbers typed into a lookup (the pull request and issue lookups): what the input lets through, when it's drawn
//! valid, and the whole number a submit reads.
//!
//! Three rules, as in Python, which differ on purpose: Textual's `type="number"` input lets through any text that
//! could still become a number ([`can_become_number`]), its `Number` validator wants a finite float ([`is_number`]),
//! and the submit reads a whole number with `int()` ([`parse_int`]). So `1.5` is drawn valid but refused at submit.
//!
//! Digits are ASCII: Python's `\d` and `int()` also take other scripts' decimal digits, which a keyboard rarely types.

use std::sync::LazyLock;

use regex::Regex;

/// Textual's `_RESTRICT_TYPES["number"]`, anchored for a full match.
static NUMBER_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    // A fixed pattern that's covered by the tests, so it always compiles.
    Regex::new(r"^[-+]?(?:[0-9]*|[0-9]+_)*\.?(?:[0-9]*|[0-9]+_)*(?:[0-9][eE]?[-+]?(?:[0-9]*|[0-9]+_)*)?$")
        .expect("valid pattern")
});

/// Whether `text` is something a `type="number"` input may hold: a sign, digits with single `_` between them, one
/// `.` and an exponent, each possibly unfinished (`-`, `1_`, `1e`). Every edit whose result isn't is refused, a
/// [`TextInput`](crate::components::text_input::TextInput) filter.
pub fn can_become_number(text: &str) -> bool {
    NUMBER_PREFIX.is_match(text)
}

/// Textual's `Number` validator: `text` reads as a finite number with Python's `float()` (`1`, `-1.5`, `1e3`, `.5`,
/// `1_000`, surrounding whitespace allowed). Anything else, the empty text included, draws the input invalid.
pub fn is_number(text: &str) -> bool {
    let text = text.trim();
    let rest = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some(("", fraction)) => is_digit_part(fraction),
        Some((whole, "")) => is_digit_part(whole),
        Some((whole, fraction)) => is_digit_part(whole) && is_digit_part(fraction),
        None => is_digit_part(mantissa),
    };
    let exponent_ok = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        is_digit_part(digits)
    });
    // The syntax is checked above; Rust's parser only says whether it overflows to infinity (`1e999`), which the
    // validator refuses.
    mantissa_ok && exponent_ok && text.replace('_', "").parse::<f64>().is_ok_and(f64::is_finite)
}

/// A whole number as Python's `int()` reads `text`: surrounding whitespace, a sign, then digits with single `_`
/// between them (`+5`, `-3`, `007`, `1_0`). `None` for anything else (`""`, `1.5`, `1e3`, `1__0`, `-`).
///
/// Python's integers have no bound; past `i128`'s this saturates, which no GitHub number comes near.
pub fn parse_int(text: &str) -> Option<i128> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if !is_digit_part(digits) {
        return None;
    }
    let magnitude = digits.bytes().filter(u8::is_ascii_digit).fold(0i128, |n, digit| {
        n.saturating_mul(10).saturating_add(i128::from(digit - b'0'))
    });
    Some(if negative { -magnitude } else { magnitude })
}

/// Python's `digitpart`: one or more digits, with single underscores between them.
fn is_digit_part(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('_')
        && !text.ends_with('_')
        && !text.contains("__")
        && text.bytes().all(|b| b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("", true)]
    #[case("9", true)]
    #[case("-", true)]
    #[case("+", true)]
    #[case("-3", true)]
    #[case("1_", true)]
    #[case("1_0", true)]
    #[case("1.", true)]
    #[case(".", true)]
    #[case("1.5", true)]
    #[case("1e", true)]
    #[case("1e-", true)]
    #[case("1e3", true)]
    #[case("1.5E+2", true)]
    #[case("a", false)]
    #[case("1a", false)]
    #[case(" 1", false)]
    #[case("1__", false)]
    #[case("_1", false)]
    #[case("1..", false)]
    #[case("1.2.3", false)]
    #[case("--1", false)]
    // Textual's exponent `e` is optional, so a sign after a digit goes in too.
    #[case("1-", true)]
    #[case("12-3", true)]
    #[case("e", false)]
    #[case("1ee", false)]
    fn what_a_number_input_may_hold(#[case] text: &str, #[case] allowed: bool) {
        assert_eq!(can_become_number(text), allowed, "{text:?}");
    }

    #[rstest]
    #[case("9", true)]
    #[case("42", true)]
    #[case("1.5", true)]
    #[case("-3", true)]
    #[case("+5", true)]
    #[case("1e2", true)]
    #[case("1E-2", true)]
    #[case("1.", true)]
    #[case(".5", true)]
    #[case("1_000.0_1", true)]
    #[case(" 7 ", true)]
    #[case("", false)]
    #[case(".", false)]
    #[case("-", false)]
    #[case("1_", false)]
    #[case("1__0", false)]
    #[case("1e", false)]
    #[case("1e999", false)]
    #[case("inf", false)]
    #[case("nan", false)]
    #[case("1._5", false)]
    fn a_number_is_a_finite_python_float(#[case] text: &str, #[case] valid: bool) {
        assert_eq!(is_number(text), valid, "{text:?}");
    }

    #[rstest]
    #[case("", None)]
    #[case("3", Some(3))]
    #[case("+5", Some(5))]
    #[case("-3", Some(-3))]
    #[case("0", Some(0))]
    #[case("007", Some(7))]
    #[case("1_0", Some(10))]
    #[case(" 12\t", Some(12))]
    #[case("1__0", None)]
    #[case("_1", None)]
    #[case("1_", None)]
    #[case("1.", None)]
    #[case("1.5", None)]
    #[case("1e3", None)]
    #[case(".", None)]
    #[case("-", None)]
    #[case("+-1", None)]
    fn a_whole_number_follows_python_int(#[case] text: &str, #[case] number: Option<i128>) {
        assert_eq!(parse_int(text), number, "{text:?}");
    }

    #[test]
    fn a_huge_number_saturates() {
        assert_eq!(parse_int(&"9".repeat(60)), Some(i128::MAX));
        assert_eq!(parse_int(&format!("-{}", "9".repeat(60))), Some(-i128::MAX));
    }
}
