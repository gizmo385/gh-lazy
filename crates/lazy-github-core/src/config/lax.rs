//! Serde helpers that coerce values the way pydantic's lax mode does, so a hand-edited `config.json` that the Python
//! app accepts also loads here. Saving always writes the canonical JSON type.

use std::fmt;
use std::path::PathBuf;

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serializer};

/// A bool given as a JSON bool, the numbers `0`/`1`, or one of pydantic's bool strings in any case.
pub(super) fn bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    struct LaxBool;

    impl Visitor<'_> for LaxBool {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a boolean")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<bool, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<bool, E> {
            match v {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(E::invalid_value(de::Unexpected::Unsigned(v), &self)),
            }
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<bool, E> {
            u64::try_from(v)
                .map_err(|_| E::invalid_value(de::Unexpected::Signed(v), &self))
                .and_then(|v| self.visit_u64(v))
        }

        #[allow(clippy::float_cmp)] // Only the exact values 0.0 and 1.0 are accepted.
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<bool, E> {
            if v == 0.0 {
                Ok(false)
            } else if v == 1.0 {
                Ok(true)
            } else {
                Err(E::invalid_value(de::Unexpected::Float(v), &self))
            }
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<bool, E> {
            match v.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "t" | "y" | "1" => Ok(true),
                "false" | "no" | "off" | "f" | "n" | "0" => Ok(false),
                _ => Err(E::invalid_value(de::Unexpected::Str(v), &self)),
            }
        }
    }

    deserializer.deserialize_any(LaxBool)
}

/// An integer given as a JSON integer, a float with no fractional part, a bool, or a string accepted by
/// [`parse_int_str`]. Negative values are allowed, as pydantic allows them; the accessors that use a value clamp it.
pub(super) fn i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    struct LaxI64;

    impl Visitor<'_> for LaxI64 {
        type Value = i64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an integer")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<i64, E> {
            Ok(i64::from(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(|_| E::invalid_value(de::Unexpected::Unsigned(v), &self))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> {
            // The range and fraction checks make the cast exact. `i64::MAX as f64` rounds up to 2^63, one past the
            // largest `i64`, so the range is half-open.
            if v.fract() == 0.0 && (i64::MIN as f64..i64::MAX as f64).contains(&v) {
                Ok(v as i64)
            } else {
                Err(E::invalid_value(de::Unexpected::Float(v), &self))
            }
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            parse_int_str(v).ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
        }
    }

    deserializer.deserialize_any(LaxI64)
}

/// Parses an integer string the way pydantic does: surrounding whitespace and a leading `+` or `-` are allowed,
/// single underscores may separate digits (`1_000`), and a fractional part must be all zeros (`1.00`). Exponents
/// (`1e3`) and other float spellings are rejected.
fn parse_int_str(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", s.strip_prefix('+').unwrap_or(s)),
    };
    let (int_part, frac) = match s.split_once('.') {
        Some((int_part, frac)) => (int_part, Some(frac)),
        None => (s, None),
    };
    if frac.is_some_and(|frac| frac.is_empty() || !frac.bytes().all(|b| b == b'0')) {
        return None;
    }
    // Every underscore-separated group must be a non-empty run of digits.
    if !int_part
        .split('_')
        .all(|group| !group.is_empty() && group.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    // Parsing with the sign attached keeps `i64::MIN` in range.
    format!("{sign}{}", int_part.replace('_', "")).parse().ok()
}

/// A string enum, such as [`super::MergeMethod`]. Read as a string first: `serde_json` reports a number where an enum is
/// expected as a JSON syntax error, which would blame the file rather than the field.
pub(super) fn string_enum<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let name = String::deserialize(deserializer)?;
    T::deserialize(de::value::StringDeserializer::new(name))
}

/// A theme name. Python's `mode="before"` validator looks the value up in Textual's built-in themes and falls back to
/// `textual-dark`, so an unknown name (matched exactly, case included) or a non-string loads as `textual-dark`, which
/// the next save writes back.
pub(super) fn theme<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(name) if super::TEXTUAL_THEMES.contains(&name.as_str()) => name,
        _ => super::DEFAULT_THEME.to_owned(),
    })
}

/// A path. An empty string becomes `.`, which is what Python's `Path("")` does; `~` is not expanded.
pub(super) fn path<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    let s = String::deserialize(deserializer)?;
    Ok(path_from(s))
}

/// An optional path, `null` being `None`.
pub(super) fn optional_path<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<PathBuf>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.map(path_from))
}

fn path_from(s: String) -> PathBuf {
    if s.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(s)
    }
}

/// Paths are written as strings. A non-UTF-8 path can't come from JSON, so the lossy conversion only matters for one
/// set in code, and it keeps serialization infallible.
pub(super) fn serialize_path<S: Serializer>(path: &std::path::Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

#[allow(clippy::ref_option)] // serde passes `serialize_with` a reference to the field.
pub(super) fn serialize_optional_path<S: Serializer>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => serialize_path(path, serializer),
        None => serializer.serialize_none(),
    }
}

/// `auth_last_checked` in pydantic's format for a naive datetime: `YYYY-MM-DDTHH:MM:SS[.ffffff]`.
pub(super) mod naive_datetime {
    use super::{DateTime, Deserialize, Deserializer, Serializer, TimeZone, Timestamp, ToSpan, de};

    #[allow(clippy::ref_option)] // serde passes `with` a reference to the field.
    pub fn serialize<S: Serializer>(value: &Option<DateTime>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(dt) => serializer.serialize_str(&format(*dt)),
            None => serializer.serialize_none(),
        }
    }

    const EXPECTING: &str = "a datetime like 2024-01-31T12:34:56[.123456]";

    /// A datetime string, `null`, or a Unix timestamp as a number or numeric string, as pydantic accepts.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime>, D::Error> {
        use serde_json::Value;

        match Value::deserialize(deserializer)? {
            Value::Null => Ok(None),
            // The Python settings modal saves an unset datetime as the string "None" (Py bug B2).
            Value::String(s) if s == "None" => Ok(None),
            Value::String(s) => parse(&s)
                .or_else(|| unix_number_str(&s).and_then(from_unix))
                .map(Some)
                .ok_or_else(|| de::Error::invalid_value(de::Unexpected::Str(&s), &EXPECTING)),
            Value::Number(n) => n
                .as_f64()
                .and_then(from_unix)
                .map(Some)
                .ok_or_else(|| de::Error::invalid_value(de::Unexpected::Other("number"), &EXPECTING)),
            Value::Bool(b) => Err(de::Error::invalid_type(de::Unexpected::Bool(b), &EXPECTING)),
            Value::Array(_) => Err(de::Error::invalid_type(de::Unexpected::Seq, &EXPECTING)),
            Value::Object(_) => Err(de::Error::invalid_type(de::Unexpected::Map, &EXPECTING)),
        }
    }

    /// A string pydantic reads as a Unix timestamp: an optional sign, then digits with at most one `.` (`5.`, `.5`).
    fn unix_number_str(s: &str) -> Option<f64> {
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        let (int_part, frac) = digits.split_once('.').unwrap_or((digits, ""));
        let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
        if (int_part.is_empty() && frac.is_empty()) || !all_digits(int_part) || !all_digits(frac) {
            return None;
        }
        s.parse().ok()
    }

    /// A Unix timestamp in local civil time, as Python's aware UTC datetime is converted for Py bug B37. As in
    /// pydantic, a value beyond ±2e10 is in milliseconds rather than seconds; precision is microseconds.
    #[allow(clippy::cast_possible_truncation)] // Saturating: an out-of-range value fails in `from_microsecond`.
    fn from_unix(value: f64) -> Option<DateTime> {
        let micros = if value.abs() > 2e10 { value * 1e3 } else { value * 1e6 };
        let ts = Timestamp::from_microsecond(micros.round() as i64).ok()?;
        Some(ts.to_zoned(TimeZone::system()).datetime())
    }

    /// Parses a naive datetime, or an RFC 3339 timestamp with an offset converted to local civil time. Precision
    /// is truncated to microseconds, as Python's `datetime` holds no more.
    pub(crate) fn parse(s: &str) -> Option<DateTime> {
        // A timestamp needs an offset; try it first because the civil parser would silently drop one.
        let dt = match s.parse::<Timestamp>() {
            Ok(ts) => ts.to_zoned(TimeZone::system()).datetime(),
            Err(_) => s.parse::<DateTime>().ok()?,
        };
        let sub_micro = dt.subsec_nanosecond() % 1000;
        dt.checked_sub(i64::from(sub_micro).nanoseconds()).ok()
    }

    pub(crate) fn format(dt: DateTime) -> String {
        let base = dt.strftime("%Y-%m-%dT%H:%M:%S").to_string();
        let micros = dt.subsec_nanosecond() / 1000;
        if micros == 0 {
            base
        } else {
            format!("{base}.{micros:06}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TimeZone, Timestamp, naive_datetime, parse_int_str};

    #[test]
    fn unix_timestamps_load_as_local_time() {
        let local = |s: &str| s.parse::<Timestamp>().unwrap().to_zoned(TimeZone::system()).datetime();
        for (raw, utc) in [
            ("0", "1970-01-01T00:00:00Z"),
            ("-5", "1969-12-31T23:59:55Z"),
            ("1.5", "1970-01-01T00:00:01.5Z"),
            ("\"01\"", "1970-01-01T00:00:01Z"),
            ("\"+5\"", "1970-01-01T00:00:05Z"),
            ("\"5.\"", "1970-01-01T00:00:05Z"),
            ("\".5\"", "1970-01-01T00:00:00.5Z"),
            // Beyond 2e10 the value is in milliseconds.
            ("1700000000123", "2023-11-14T22:13:20.123Z"),
            ("20000000001", "1970-08-20T11:33:20.001Z"),
            ("1700000000123.5", "2023-11-14T22:13:20.1235Z"),
        ] {
            let mut de = serde_json::Deserializer::from_str(raw);
            let loaded = naive_datetime::deserialize(&mut de).unwrap();
            assert_eq!(loaded, Some(local(utc)), "{raw}");
        }
        for raw in ["\"1e3\"", "\" 5\"", "\".\"", "\"-\"", "true", "99999999999999999999"] {
            let mut de = serde_json::Deserializer::from_str(raw);
            assert!(naive_datetime::deserialize(&mut de).is_err(), "{raw}");
        }
    }

    #[test]
    fn int_strings_match_pydantic() {
        for (raw, expected) in [
            ("1_000", Some(1000)),
            ("1_000.0", Some(1000)),
            ("1.00", Some(1)),
            ("+5", Some(5)),
            (" 7 ", Some(7)),
            ("00012", Some(12)),
            ("_1", None),
            ("1__0", None),
            ("1_", None),
            ("1.", None),
            ("1.5", None),
            ("1e3", None),
            (".0", None),
            ("0x10", None),
            ("-1", Some(-1)),
            (" -1_000.0 ", Some(-1000)),
            ("-9223372036854775808", Some(i64::MIN)),
            ("9223372036854775808", None),
            ("-+1", None),
            ("+-1", None),
            ("-", None),
            ("", None),
        ] {
            assert_eq!(parse_int_str(raw), expected, "{raw:?}");
        }
    }
}
