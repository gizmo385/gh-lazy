//! Date formats the app shows.

use jiff::Timestamp;

/// `ts` as Python's `datetime.strftime("%c")` under the C locale, in UTC: `Sun Feb  1 08:30:00 2026`, the day of the
/// month padded with a space. Python shows API times in UTC (Py bug B26), and so does Rust.
pub fn c_format(ts: Timestamp) -> String {
    ts.strftime("%a %b %e %H:%M:%S %Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn a_one_digit_day_is_space_padded() {
        assert_eq!(c_format(at("2026-02-01T08:30:00Z")), "Sun Feb  1 08:30:00 2026");
    }

    #[test]
    fn a_two_digit_day_is_not() {
        assert_eq!(c_format(at("2026-01-12T16:20:00Z")), "Mon Jan 12 16:20:00 2026");
        assert_eq!(c_format(at("2026-01-10T14:00:00+05:00")), "Sat Jan 10 09:00:00 2026");
    }
}
