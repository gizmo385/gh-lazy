//! JSON printed the way Rich's `print_json` prints it, which is what Python's `dump-config` writes.

use serde_json::Value;

/// How `dump-config` styles its JSON, following Rich's rules for its default console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonStyle {
    /// No escape codes: stdout isn't a terminal, or the terminal is dumb.
    Plain,
    /// Rich's default `json.*` theme.
    Color,
    /// `NO_COLOR`: Rich drops the colors and keeps bold and italic.
    NoColor,
}

/// The kinds of text Rich's `JSONHighlighter` styles. Separators, colons and indentation stay plain.
#[derive(Debug, Clone, Copy)]
enum Token {
    Brace,
    Key,
    Str,
    Number,
    True,
    False,
    Null,
}

impl JsonStyle {
    /// Rich's `Console.is_terminal`, `is_dumb_terminal` and `no_color`, from the environment (`env` returns a
    /// variable's value) and whether stdout is a tty.
    pub fn detect(env: impl Fn(&str) -> Option<String>, stdout_is_tty: bool) -> Self {
        let is_terminal = match env("TTY_COMPATIBLE").as_deref() {
            Some("0") => false,
            Some("1") => true,
            // Rich doesn't look at the tty once `FORCE_COLOR` is set, even to an empty value.
            _ => env("FORCE_COLOR").map_or(stdout_is_tty, |force| !force.is_empty()),
        };
        let is_dumb = env("TERM").is_some_and(|term| matches!(term.to_lowercase().as_str(), "dumb" | "unknown"));
        if !is_terminal || is_dumb {
            Self::Plain
        } else if env("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            Self::NoColor
        } else {
            Self::Color
        }
    }

    /// The SGR parameters for `token`, if it's styled.
    fn sgr(self, token: Token) -> Option<&'static str> {
        match self {
            Self::Plain => None,
            Self::Color => Some(match token {
                Token::Brace => "1",
                Token::Key => "1;34",
                Token::Str => "32",
                Token::Number => "1;36",
                Token::True => "3;92",
                Token::False => "3;91",
                Token::Null => "3;35",
            }),
            // The color styles above without their colors.
            Self::NoColor => match token {
                Token::Brace | Token::Key | Token::Number => Some("1"),
                Token::True | Token::False | Token::Null => Some("3"),
                Token::Str => None,
            },
        }
    }
}

/// `value` as Python's `json.dumps(indent=2, ensure_ascii=False)` writes it, each token styled per `style`, and a
/// trailing newline.
pub fn to_rich_json(value: &Value, style: JsonStyle) -> String {
    let mut writer = Writer {
        out: String::new(),
        style,
    };
    writer.value(value, 0);
    writer.out.push('\n');
    writer.out
}

struct Writer {
    out: String,
    style: JsonStyle,
}

impl Writer {
    fn token(&mut self, token: Token, text: &str) {
        match self.style.sgr(token) {
            Some(sgr) => {
                self.out.push_str("\x1b[");
                self.out.push_str(sgr);
                self.out.push('m');
                self.out.push_str(text);
                self.out.push_str("\x1b[0m");
            }
            None => self.out.push_str(text),
        }
    }

    fn newline(&mut self, depth: usize) {
        self.out.push('\n');
        self.out.push_str(&"  ".repeat(depth));
    }

    fn value(&mut self, value: &Value, depth: usize) {
        match value {
            Value::Null => self.token(Token::Null, "null"),
            Value::Bool(true) => self.token(Token::True, "true"),
            Value::Bool(false) => self.token(Token::False, "false"),
            Value::Number(number) => self.token(Token::Number, &number.to_string()),
            Value::String(text) => self.token(Token::Str, &quoted(text)),
            Value::Array(items) => {
                self.token(Token::Brace, "[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.newline(depth + 1);
                    self.value(item, depth + 1);
                }
                if !items.is_empty() {
                    self.newline(depth);
                }
                self.token(Token::Brace, "]");
            }
            Value::Object(fields) => {
                self.token(Token::Brace, "{");
                for (i, (key, item)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.newline(depth + 1);
                    self.token(Token::Key, &quoted(key));
                    self.out.push_str(": ");
                    self.value(item, depth + 1);
                }
                if !fields.is_empty() {
                    self.newline(depth);
                }
                self.token(Token::Brace, "}");
            }
        }
    }
}

/// A JSON string literal. `serde_json` escapes exactly what `json.dumps(ensure_ascii=False)` does: `"`, `\`, the
/// short forms `\b \f \n \r \t` and other control characters as lowercase `\u00xx`.
fn quoted(text: &str) -> String {
    Value::from(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::{JsonStyle, to_rich_json};
    use serde_json::json;

    fn detect(vars: &[(&str, &str)], tty: bool) -> JsonStyle {
        JsonStyle::detect(
            |name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            },
            tty,
        )
    }

    #[test]
    fn a_tty_is_styled_and_a_pipe_is_not() {
        assert_eq!(detect(&[], true), JsonStyle::Color);
        assert_eq!(detect(&[], false), JsonStyle::Plain);
    }

    #[test]
    fn tty_compatible_overrides_everything_but_term() {
        assert_eq!(detect(&[("TTY_COMPATIBLE", "1")], false), JsonStyle::Color);
        assert_eq!(detect(&[("TTY_COMPATIBLE", "0")], true), JsonStyle::Plain);
        assert_eq!(
            detect(&[("TTY_COMPATIBLE", "0"), ("FORCE_COLOR", "1")], true),
            JsonStyle::Plain
        );
        assert_eq!(
            detect(&[("TTY_COMPATIBLE", "1"), ("FORCE_COLOR", "")], false),
            JsonStyle::Color
        );
        // Any other value falls through to `FORCE_COLOR` and the tty.
        assert_eq!(detect(&[("TTY_COMPATIBLE", "yes")], true), JsonStyle::Color);
        assert_eq!(detect(&[("TTY_COMPATIBLE", "yes")], false), JsonStyle::Plain);
    }

    #[test]
    fn force_color_decides_when_set() {
        assert_eq!(detect(&[("FORCE_COLOR", "1")], false), JsonStyle::Color);
        assert_eq!(detect(&[("FORCE_COLOR", "0")], false), JsonStyle::Color);
        // Empty means "not a terminal" to Rich, even on a tty.
        assert_eq!(detect(&[("FORCE_COLOR", "")], true), JsonStyle::Plain);
        assert_eq!(detect(&[("FORCE_COLOR", "")], false), JsonStyle::Plain);
    }

    #[test]
    fn a_dumb_terminal_is_plain() {
        for term in ["dumb", "unknown", "DUMB"] {
            assert_eq!(detect(&[("TERM", term)], true), JsonStyle::Plain, "{term}");
            assert_eq!(
                detect(&[("TERM", term), ("FORCE_COLOR", "1")], false),
                JsonStyle::Plain,
                "{term}"
            );
        }
        assert_eq!(detect(&[("TERM", "xterm-256color")], true), JsonStyle::Color);
    }

    #[test]
    fn no_color_keeps_bold_and_italic() {
        assert_eq!(detect(&[("NO_COLOR", "1")], true), JsonStyle::NoColor);
        assert_eq!(detect(&[("NO_COLOR", "")], true), JsonStyle::Color);
        assert_eq!(detect(&[("NO_COLOR", "1")], false), JsonStyle::Plain);
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(to_rich_json(&json!({}), JsonStyle::Plain), "{}\n");
        assert_eq!(to_rich_json(&json!([]), JsonStyle::Plain), "[]\n");
        assert_eq!(
            to_rich_json(&json!({"a": {}, "b": []}), JsonStyle::Plain),
            "{\n  \"a\": {},\n  \"b\": []\n}\n"
        );
    }

    #[test]
    fn nested_arrays_indent_by_two() {
        assert_eq!(
            to_rich_json(&json!([1, [2, [true, null]], "x"]), JsonStyle::Plain),
            "[\n  1,\n  [\n    2,\n    [\n      true,\n      null\n    ]\n  ],\n  \"x\"\n]\n"
        );
    }

    #[test]
    fn escapes_match_json_dumps() {
        assert_eq!(
            to_rich_json(&json!("q\" b\\ \n\r\t\u{8}\u{c} \u{1}\u{1f}\u{7f}"), JsonStyle::Plain),
            "\"q\\\" b\\\\ \\n\\r\\t\\b\\f \\u0001\\u001f\u{7f}\"\n"
        );
    }

    #[test]
    fn non_ascii_is_written_as_is() {
        assert_eq!(
            to_rich_json(&json!({"café": "ünïcode ✓ 😀"}), JsonStyle::Plain),
            "{\n  \"café\": \"ünïcode ✓ 😀\"\n}\n"
        );
    }

    #[test]
    fn every_token_is_wrapped_in_its_own_sgr() {
        let value = json!({"k": ["s", 1, true, false, null, {}]});
        assert_eq!(
            to_rich_json(&value, JsonStyle::Color),
            "\x1b[1m{\x1b[0m\n  \x1b[1;34m\"k\"\x1b[0m: \x1b[1m[\x1b[0m\n    \x1b[32m\"s\"\x1b[0m,\n    \
             \x1b[1;36m1\x1b[0m,\n    \x1b[3;92mtrue\x1b[0m,\n    \x1b[3;91mfalse\x1b[0m,\n    \x1b[3;35mnull\x1b[0m,\n    \
             \x1b[1m{\x1b[0m\x1b[1m}\x1b[0m\n  \x1b[1m]\x1b[0m\n\x1b[1m}\x1b[0m\n"
        );
        assert_eq!(
            to_rich_json(&value, JsonStyle::NoColor),
            "\x1b[1m{\x1b[0m\n  \x1b[1m\"k\"\x1b[0m: \x1b[1m[\x1b[0m\n    \"s\",\n    \x1b[1m1\x1b[0m,\n    \
             \x1b[3mtrue\x1b[0m,\n    \x1b[3mfalse\x1b[0m,\n    \x1b[3mnull\x1b[0m,\n    \
             \x1b[1m{\x1b[0m\x1b[1m}\x1b[0m\n  \x1b[1m]\x1b[0m\n\x1b[1m}\x1b[0m\n"
        );
    }

    #[test]
    fn text_inside_a_string_is_never_restyled() {
        assert_eq!(
            to_rich_json(&json!(["owner/12", "true/null", "a{b}[c]"]), JsonStyle::Color),
            "\x1b[1m[\x1b[0m\n  \x1b[32m\"owner/12\"\x1b[0m,\n  \x1b[32m\"true/null\"\x1b[0m,\n  \
             \x1b[32m\"a{b}[c]\"\x1b[0m\n\x1b[1m]\x1b[0m\n"
        );
    }
}
