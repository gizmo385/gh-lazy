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

/// A non-negative integer given as a JSON integer, a float with no fractional part, a bool, or a string accepted by
/// [`parse_int_str`].
pub(super) fn u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    struct LaxU64;

    impl Visitor<'_> for LaxU64 {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a non-negative integer")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<u64, E> {
            Ok(u64::from(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
            u64::try_from(v).map_err(|_| E::invalid_value(de::Unexpected::Signed(v), &self))
        }

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<u64, E> {
            // The range and fraction checks make the cast exact.
            if v.fract() == 0.0 && (0.0..=u64::MAX as f64).contains(&v) {
                Ok(v as u64)
            } else {
                Err(E::invalid_value(de::Unexpected::Float(v), &self))
            }
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            parse_int_str(v).ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
        }
    }

    deserializer.deserialize_any(LaxU64)
}

/// Parses an integer string the way pydantic does: surrounding whitespace and a leading `+` are allowed, single
/// underscores may separate digits (`1_000`), and a fractional part must be all zeros (`1.00`). Exponents (`1e3`)
/// and other float spellings are rejected.
fn parse_int_str(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
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
    int_part.replace('_', "").parse().ok()
}

/// A theme name. Python's `mode="before"` validator turns any value into a theme, falling back to `textual-dark`,
/// so a non-string loads as `textual-dark`. An unknown string is kept as written.
pub(super) fn theme<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(name) => name,
        _ => "textual-dark".to_owned(),
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

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            // The Python settings modal saves an unset datetime as the string "None".
            None => Ok(None),
            Some(s) if s == "None" => Ok(None),
            Some(s) => parse(&s).map(Some).ok_or_else(|| {
                de::Error::invalid_value(de::Unexpected::Str(&s), &"a datetime like 2024-01-31T12:34:56[.123456]")
            }),
        }
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
    use super::parse_int_str;

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
            ("-1", None),
            ("", None),
        ] {
            assert_eq!(parse_int_str(raw), expected, "{raw:?}");
        }
    }
}
