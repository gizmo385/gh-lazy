//! Encoding scenario key specs (Textual's binding spelling) as the bytes an xterm sends.

use thiserror::Error;

const ESC: u8 = 0x1b;

/// Why a key spec can't be sent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error("unknown key {0:?}")]
    Unknown(String),
    /// Keys a terminal can't send distinctly without the kitty keyboard protocol (parity matrix,
    /// `docs/rust-rewrite/parity/areas/terminal-caveats.md`).
    #[error("key {0:?} can't be sent distinctly by a terminal without the kitty keyboard protocol")]
    Indistinct(String),
}

/// Whether `spec` is the escape key, which needs a longer pause after it so Textual's escape-sequence timeout
/// doesn't fuse it with the next key.
pub fn is_escape(spec: &str) -> bool {
    matches!(spec, "escape" | "esc")
}

/// Encodes one key spec. `app_cursor` is the app's DECCKM mode (`vt100::Screen::application_cursor`), which
/// switches arrows and Home/End to their `ESC O` forms.
pub fn encode(spec: &str, app_cursor: bool) -> Result<Vec<u8>, KeyError> {
    let mut chars = spec.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && !c.is_control()
    {
        return Ok(c.to_string().into_bytes());
    }

    // `ESC O x` in application cursor mode, `ESC [ x` otherwise.
    let cursor = |final_byte: u8| vec![ESC, if app_cursor { b'O' } else { b'[' }, final_byte];
    let csi = |s: &str| [&[ESC, b'['], s.as_bytes()].concat();
    let ss3 = |final_byte: u8| vec![ESC, b'O', final_byte];

    let bytes = match spec {
        "enter" => vec![b'\r'],
        "tab" => vec![b'\t'],
        "shift+tab" => csi("Z"),
        "escape" | "esc" => vec![ESC],
        "backspace" => vec![0x7f],
        "space" => vec![b' '],
        "up" => cursor(b'A'),
        "down" => cursor(b'B'),
        "right" => cursor(b'C'),
        "left" => cursor(b'D'),
        "home" => cursor(b'H'),
        "end" => cursor(b'F'),
        "insert" => csi("2~"),
        "delete" => csi("3~"),
        "pageup" => csi("5~"),
        "pagedown" => csi("6~"),
        "f1" => ss3(b'P'),
        "f2" => ss3(b'Q'),
        "f3" => ss3(b'R'),
        "f4" => ss3(b'S'),
        "f5" => csi("15~"),
        "f6" => csi("17~"),
        "f7" => csi("18~"),
        "f8" => csi("19~"),
        "f9" => csi("20~"),
        "f10" => csi("21~"),
        "f11" => csi("23~"),
        "f12" => csi("24~"),
        // Ctrl+M is CR, the same byte as Enter; Shift+Enter is Enter on a legacy terminal.
        "ctrl+m" | "shift+enter" => return Err(KeyError::Indistinct(spec.to_owned())),
        _ => return modified(spec),
    };
    Ok(bytes)
}

fn modified(spec: &str) -> Result<Vec<u8>, KeyError> {
    let unknown = || KeyError::Unknown(spec.to_owned());
    if let Some(rest) = spec.strip_prefix("ctrl+") {
        return match rest.as_bytes() {
            [c @ b'a'..=b'z'] => Ok(vec![c - b'a' + 1]),
            _ => Err(unknown()),
        };
    }
    if let Some(rest) = spec.strip_prefix("alt+") {
        let mut chars = rest.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_control() => Ok([&[ESC], c.to_string().as_bytes()].concat()),
            _ => Err(unknown()),
        };
    }
    Err(unknown())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(spec: &str) -> Vec<u8> {
        encode(spec, false).unwrap()
    }

    #[test]
    fn printable_characters_are_their_utf8() {
        assert_eq!(enc("j"), b"j");
        assert_eq!(enc("J"), b"J");
        assert_eq!(enc("/"), b"/");
        assert_eq!(enc("?"), b"?");
        assert_eq!(enc("é"), "é".as_bytes());
    }

    #[test]
    fn named_keys() {
        assert_eq!(enc("enter"), b"\r");
        assert_eq!(enc("tab"), b"\t");
        assert_eq!(enc("shift+tab"), b"\x1b[Z");
        assert_eq!(enc("escape"), b"\x1b");
        assert_eq!(enc("esc"), b"\x1b");
        assert_eq!(enc("backspace"), b"\x7f");
        assert_eq!(enc("space"), b" ");
        assert_eq!(enc("insert"), b"\x1b[2~");
        assert_eq!(enc("delete"), b"\x1b[3~");
        assert_eq!(enc("pageup"), b"\x1b[5~");
        assert_eq!(enc("pagedown"), b"\x1b[6~");
    }

    #[test]
    fn cursor_keys_follow_decckm() {
        let normal: [(&str, &[u8]); 6] = [
            ("up", b"\x1b[A"),
            ("down", b"\x1b[B"),
            ("right", b"\x1b[C"),
            ("left", b"\x1b[D"),
            ("home", b"\x1b[H"),
            ("end", b"\x1b[F"),
        ];
        for (spec, bytes) in normal {
            assert_eq!(encode(spec, false).unwrap(), bytes, "{spec}");
            let mut app = bytes.to_vec();
            app[1] = b'O';
            assert_eq!(encode(spec, true).unwrap(), app, "{spec} with DECCKM on");
        }
    }

    #[test]
    fn function_keys() {
        let expected: [&[u8]; 12] = [
            b"\x1bOP",
            b"\x1bOQ",
            b"\x1bOR",
            b"\x1bOS",
            b"\x1b[15~",
            b"\x1b[17~",
            b"\x1b[18~",
            b"\x1b[19~",
            b"\x1b[20~",
            b"\x1b[21~",
            b"\x1b[23~",
            b"\x1b[24~",
        ];
        for (n, bytes) in (1..=12).zip(expected) {
            assert_eq!(enc(&format!("f{n}")), bytes, "f{n}");
            // DECCKM doesn't change function keys.
            assert_eq!(encode(&format!("f{n}"), true).unwrap(), bytes, "f{n}");
        }
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(enc("ctrl+a"), [0x01]);
        assert_eq!(enc("ctrl+p"), [0x10]);
        assert_eq!(enc("ctrl+z"), [0x1a]);
        assert_eq!(encode("ctrl+A", false), Err(KeyError::Unknown("ctrl+A".into())));
        assert_eq!(encode("ctrl+1", false), Err(KeyError::Unknown("ctrl+1".into())));
    }

    #[test]
    fn alt_characters() {
        assert_eq!(enc("alt+x"), b"\x1bx");
        assert_eq!(enc("alt+X"), b"\x1bX");
        assert_eq!(enc("alt+/"), b"\x1b/");
        assert!(encode("alt+xy", false).is_err());
    }

    #[test]
    fn indistinct_and_unknown_keys_are_rejected() {
        assert_eq!(encode("ctrl+m", false), Err(KeyError::Indistinct("ctrl+m".into())));
        assert_eq!(
            encode("shift+enter", false),
            Err(KeyError::Indistinct("shift+enter".into()))
        );
        assert_eq!(encode("f13", false), Err(KeyError::Unknown("f13".into())));
        assert_eq!(encode("", false), Err(KeyError::Unknown(String::new())));
        assert_eq!(encode("\n", false), Err(KeyError::Unknown("\n".into())));
    }

    #[test]
    fn escape_detection() {
        assert!(is_escape("escape"));
        assert!(is_escape("esc"));
        assert!(!is_escape("alt+x"));
    }
}
