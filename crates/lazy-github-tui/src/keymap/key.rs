//! Normalized keys, and Textual's key strings: parsing them into keys and displaying them as the footer does.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::components::table::is_printable;

/// A key press with its modifiers, normalized so matching a binding is plain equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// The key.
    pub code: KeyCode,
    /// The modifiers that remain after normalization.
    pub mods: KeyModifiers,
}

impl KeyChord {
    /// A chord, normalized as [`Self::from_event`] would.
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        normalize(code, mods)
    }

    /// The chord for a terminal key event, or `None` for a release.
    ///
    /// - `BackTab` is `Tab` with SHIFT.
    /// - An ASCII letter without CTRL or ALT folds SHIFT into the character: `M`, `shift+m` and `Char('M')` with
    ///   SHIFT are all `Char('M')` with no modifiers.
    /// - Space keeps SHIFT, so `shift+space` (kitty only) never fires on plain space.
    /// - Other characters drop SHIFT: it's what typed them (`?`, `!`), and Textual reports them without it.
    /// - A character with CTRL or ALT is lowercased, keeping SHIFT if it was uppercase or SHIFT was set.
    /// - Other keys keep their modifiers, so plain Enter is never `ctrl+m`, Tab never `ctrl+i` and Escape never
    ///   `ctrl+[`: unlike Textual, there are no event aliases.
    pub fn from_event(event: &KeyEvent) -> Option<Self> {
        // `Repeat` counts as a press. Repeats only arrive with kitty's REPORT_EVENT_TYPES, which isn't pushed.
        if event.kind == KeyEventKind::Release {
            return None;
        }
        Some(normalize(event.code, event.modifiers))
    }

    /// Whether a focused text input takes this key away from the bindings around it, by Textual's rule: the input
    /// consumes a key whose name `key_to_character` turns into a printable character (`Input.check_consume_key`),
    /// and the screen drops that key from every outer link of the binding chain (`Screen._binding_chain`).
    ///
    /// That's a plain printable character, `space` included. Textual names `+ - / @ \ _` `plus`, `minus`, `slash`,
    /// `at`, `backslash` and `underscore`, which `key_to_character` doesn't resolve, so those stay: `/` still
    /// searches while the search box has focus. Keys with modifiers and named keys (`enter`, `f2`) stay too.
    /// Non-ASCII characters count as printable here; Textual misses a few whose Unicode names hold a hyphen.
    pub fn typed_by_input(&self) -> bool {
        match self.code {
            KeyCode::Char(c) if self.mods.is_empty() => {
                is_printable(c) && !matches!(c, '+' | '-' | '/' | '@' | '\\' | '_')
            }
            _ => false,
        }
    }

    /// The key event a terminal would send for this chord. `shift+tab` is `BackTab`, as crossterm reports it.
    pub fn to_event(self) -> KeyEvent {
        match (self.code, self.mods) {
            (KeyCode::Tab, mods) if mods.contains(KeyModifiers::SHIFT) => KeyEvent::new(KeyCode::BackTab, mods),
            (code, mods) => KeyEvent::new(code, mods),
        }
    }
}

fn normalize(code: KeyCode, mods: KeyModifiers) -> KeyChord {
    match code {
        KeyCode::BackTab => KeyChord {
            code: KeyCode::Tab,
            mods: mods | KeyModifiers::SHIFT,
        },
        KeyCode::Char(' ') if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => KeyChord { code, mods },
        KeyCode::Char(c) if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
            let c = if mods.contains(KeyModifiers::SHIFT) {
                c.to_ascii_uppercase()
            } else {
                c
            };
            KeyChord {
                code: KeyCode::Char(c),
                mods: mods - KeyModifiers::SHIFT,
            }
        }
        KeyCode::Char(c) => {
            let mods = if c.is_ascii_uppercase() {
                mods | KeyModifiers::SHIFT
            } else {
                mods
            };
            KeyChord {
                code: KeyCode::Char(c.to_ascii_lowercase()),
                mods,
            }
        }
        code => KeyChord { code, mods },
    }
}

/// One key from a Textual key string: a chord, or a token that names no key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySpec {
    /// A key that can be pressed.
    Chord(KeyChord),
    /// A token that isn't a key (`bogus`, `Enter`, `ctrl+`), as written. It never matches, but it's displayed.
    Unknown(String),
}

/// Textual's names for punctuation keys, from `textual.keys._character_to_key`.
const PUNCTUATION: &[(&str, char)] = &[
    ("exclamation_mark", '!'),
    ("quotation_mark", '"'),
    ("number_sign", '#'),
    ("dollar_sign", '$'),
    ("percent_sign", '%'),
    ("ampersand", '&'),
    ("apostrophe", '\''),
    ("left_parenthesis", '('),
    ("right_parenthesis", ')'),
    ("asterisk", '*'),
    ("plus", '+'),
    ("comma", ','),
    ("minus", '-'),
    ("full_stop", '.'),
    ("slash", '/'),
    ("colon", ':'),
    ("semicolon", ';'),
    ("less_than_sign", '<'),
    ("equals_sign", '='),
    ("greater_than_sign", '>'),
    ("question_mark", '?'),
    ("at", '@'),
    ("left_square_bracket", '['),
    ("backslash", '\\'),
    ("right_square_bracket", ']'),
    ("circumflex_accent", '^'),
    ("underscore", '_'),
    ("grave_accent", '`'),
    ("left_curly_bracket", '{'),
    ("vertical_line", '|'),
    ("right_curly_bracket", '}'),
    ("tilde", '~'),
];

/// Textual's `KEY_DISPLAY_ALIASES`.
const DISPLAY_ALIASES: &[(&str, &str)] = &[
    ("up", "↑"),
    ("down", "↓"),
    ("left", "←"),
    ("right", "→"),
    ("backspace", "⌫"),
    ("escape", "esc"),
    ("enter", "⏎"),
    ("minus", "-"),
    ("space", "space"),
    ("pagedown", "pgdn"),
    ("pageup", "pgup"),
    ("delete", "del"),
];

impl KeySpec {
    /// [`KeyChord::typed_by_input`] for a key; a token that names no key is never typed.
    pub fn typed_by_input(&self) -> bool {
        matches!(self, KeySpec::Chord(chord) if chord.typed_by_input())
    }

    /// The keys in a comma-separated list, each with its trimmed token as written: `"q, escape"` gives two.
    /// Empty tokens are skipped, so `""` and `"  "` give none. A literal comma is written `comma`.
    pub fn parse_list(s: &str) -> Vec<(String, KeySpec)> {
        s.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| (token.to_owned(), Self::parse(token)))
            .collect()
    }

    /// One key in Textual's syntax: a single character (`M`, `?`), or modifiers and a key name joined by `+`
    /// (`ctrl+p`, `shift+enter`, `question_mark`). Names are case-sensitive, so `Enter` is unknown.
    pub fn parse(token: &str) -> KeySpec {
        let token = token.trim();
        parse_chord(token).map_or_else(|| KeySpec::Unknown(token.to_owned()), KeySpec::Chord)
    }

    /// The key a Textual key-event name stands for, as the Settings screen records them (`M`, `ctrl+f`,
    /// `question_mark`, `shift+enter`). Event names use the same grammar as bindings.
    pub fn from_event_name(name: &str) -> KeySpec {
        Self::parse(name)
    }

    /// How the footer shows `token`, as Textual's `App.get_key_display` does without a `key_display`: `ctrl+p` is
    /// `^p`, `question_mark` is `?`, `enter` is `⏎`, and anything it doesn't know stays as written.
    pub fn display(token: &str) -> String {
        let token = token.trim();
        if token.chars().count() == 1 {
            return token.to_owned();
        }
        let mut modifiers: Vec<&str> = token.split('+').collect();
        // `split` always yields at least one part.
        let key = modifiers.pop().unwrap_or_default();
        let mut key = format_key(key);
        if let Some(ctrl) = modifiers.iter().position(|&modifier| modifier == "ctrl") {
            modifiers.remove(ctrl);
            key.insert(0, '^');
        }
        modifiers.push(&key);
        modifiers.join("+")
    }
}

fn parse_chord(token: &str) -> Option<KeyChord> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyChord::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let mut parts: Vec<&str> = token.split('+').collect();
    let name = parts.pop()?;
    let mut mods = KeyModifiers::NONE;
    for modifier in parts {
        mods |= MODIFIERS.iter().find(|&&(name, _)| name == modifier)?.1;
    }
    let code = key_code(name)?;
    if let KeyCode::Char(c) = code
        && mods == KeyModifiers::SHIFT
        && !c.is_ascii_alphabetic()
        && c != ' '
    {
        // `shift+1`, `shift+?`: a terminal sends the shifted character instead (`!`, `?`), and Textual never reports
        // these, so the chord keeps SHIFT and matches nothing, rather than firing on the plain key.
        return Some(KeyChord { code, mods });
    }
    Some(KeyChord::new(code, mods))
}

/// Textual's names for the keys that aren't characters, shared by the parser and [`textual_name`].
const NAMED_KEYS: &[(&str, KeyCode)] = &[
    ("enter", KeyCode::Enter),
    ("escape", KeyCode::Esc),
    ("tab", KeyCode::Tab),
    ("space", KeyCode::Char(' ')),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
];

/// Textual's modifier names, in the sorted order it writes them before the key.
const MODIFIERS: &[(&str, KeyModifiers)] = &[
    ("alt", KeyModifiers::ALT),
    ("ctrl", KeyModifiers::CONTROL),
    ("hyper", KeyModifiers::HYPER),
    ("meta", KeyModifiers::META),
    ("shift", KeyModifiers::SHIFT),
    ("super", KeyModifiers::SUPER),
];

fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    if let Some(&(_, code)) = NAMED_KEYS.iter().find(|&&(named, _)| named == name) {
        return Some(code);
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&n)
        && !name.starts_with("f0")
    {
        return Some(KeyCode::F(n));
    }
    let &(_, c) = PUNCTUATION.iter().find(|&&(punctuation, _)| punctuation == name)?;
    Some(KeyCode::Char(c))
}

/// The name Textual gives a key press, as the Settings screen records it: the inverse of [`KeySpec::parse`]. A
/// letter or digit is itself (`a`, `M`, `é`), ASCII punctuation and named keys go by Textual's names
/// (`question_mark`, `space`, `pagedown`, `f5`), and modifiers come first in Textual's sorted order (`ctrl+a`,
/// `alt+shift+x`). Other non-ASCII characters are written as themselves (`£`, where Textual writes `pound_sign`;
/// decision `key-names-non-ascii-literal`).
///
/// `None` for `tab` and `escape`, which a Settings row doesn't record, and for keys Textual has no name for.
pub fn textual_name(chord: &KeyChord) -> Option<String> {
    let key = match chord.code {
        KeyCode::Tab | KeyCode::Esc => return None,
        KeyCode::Char(c) => match PUNCTUATION.iter().find(|&&(_, punctuation)| punctuation == c) {
            Some(&(name, _)) => name.to_owned(),
            None if c == ' ' => "space".to_owned(),
            None => c.to_string(),
        },
        KeyCode::F(n) if (1..=24).contains(&n) => format!("f{n}"),
        code => NAMED_KEYS.iter().find(|&&(_, named)| named == code)?.0.to_owned(),
    };
    let mut parts: Vec<&str> = MODIFIERS
        .iter()
        .filter(|&&(_, modifier)| chord.mods.contains(modifier))
        .map(|&(name, _)| name)
        .collect();
    parts.push(&key);
    Some(parts.join("+"))
}

/// Textual's `format_key`: display aliases, punctuation names as their character, anything else as written.
fn format_key(name: &str) -> String {
    if let Some(&(_, alias)) = DISPLAY_ALIASES.iter().find(|&&(key, _)| key == name) {
        return alias.to_owned();
    }
    if let Some(&(_, c)) = PUNCTUATION.iter().find(|&&(punctuation, _)| punctuation == name) {
        return c.to_string();
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::keymap::BindingId;

    fn chord(event: KeyEvent) -> Option<KeyChord> {
        KeyChord::from_event(&event)
    }

    fn parsed(token: &str) -> KeyChord {
        match KeySpec::parse(token) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token:?} didn't parse"),
        }
    }

    fn plain(code: KeyCode) -> KeyChord {
        KeyChord::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyChord {
        KeyChord::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn event(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[rstest]
    #[case("q", plain(KeyCode::Char('q')))]
    #[case("ctrl+p", ctrl('p'))]
    #[case("M", plain(KeyCode::Char('M')))]
    #[case("?", plain(KeyCode::Char('?')))]
    #[case("I", plain(KeyCode::Char('I')))]
    #[case("ctrl+m", ctrl('m'))]
    #[case("ctrl+f", ctrl('f'))]
    #[case("/", plain(KeyCode::Char('/')))]
    #[case("$", plain(KeyCode::Char('$')))]
    #[case("^", plain(KeyCode::Char('^')))]
    #[case("escape", plain(KeyCode::Esc))]
    #[case("ctrl+n", ctrl('n'))]
    #[case("ctrl+r", ctrl('r'))]
    #[case("shift+enter", KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT))]
    #[case("ctrl+v", ctrl('v'))]
    #[case("1", plain(KeyCode::Char('1')))]
    #[case("ctrl+q", ctrl('q'))]
    #[case("tab", plain(KeyCode::Tab))]
    #[case("shift+tab", KeyChord::new(KeyCode::Tab, KeyModifiers::SHIFT))]
    #[case("super+c", KeyChord::new(KeyCode::Char('c'), KeyModifiers::SUPER))]
    #[case("ctrl+home", KeyChord::new(KeyCode::Home, KeyModifiers::CONTROL))]
    #[case("ctrl+pagedown", KeyChord::new(KeyCode::PageDown, KeyModifiers::CONTROL))]
    #[case("f12", plain(KeyCode::F(12)))]
    #[case("f1", plain(KeyCode::F(1)))]
    #[case("f24", plain(KeyCode::F(24)))]
    #[case("ctrl+pageup", KeyChord::new(KeyCode::PageUp, KeyModifiers::CONTROL))]
    #[case("space", plain(KeyCode::Char(' ')))]
    #[case("alt+x", KeyChord::new(KeyCode::Char('x'), KeyModifiers::ALT))]
    fn parses(#[case] token: &str, #[case] expected: KeyChord) {
        assert_eq!(KeySpec::parse(token), KeySpec::Chord(expected));
    }

    /// Textual's `check_consume_key(key, key_to_character(key))` for each key name, from Textual 8.2.5.
    #[rstest]
    #[case("M", true)]
    #[case("q", true)]
    #[case("1", true)]
    #[case("?", true)]
    #[case("question_mark", true)]
    #[case("full_stop", true)]
    #[case("space", true)]
    #[case("é", true)]
    #[case("/", false)]
    #[case("slash", false)]
    #[case("+", false)]
    #[case("minus", false)]
    #[case("@", false)]
    #[case("backslash", false)]
    #[case("_", false)]
    #[case("ctrl+p", false)]
    #[case("ctrl+f", false)]
    #[case("shift+space", false)]
    #[case("enter", false)]
    #[case("tab", false)]
    #[case("escape", false)]
    #[case("f2", false)]
    #[case("up", false)]
    #[case("bogus", false)]
    fn input_typed_keys(#[case] token: &str, #[case] typed: bool) {
        assert_eq!(KeySpec::parse(token).typed_by_input(), typed, "{token}");
    }

    #[test]
    fn every_default_key_parses() {
        for &id in BindingId::ALL {
            let def = id.def();
            let keys = KeySpec::parse_list(def.default_keys);
            assert!(!keys.is_empty(), "{} has no keys", def.name);
            for (token, spec) in keys {
                assert!(
                    matches!(spec, KeySpec::Chord(_)),
                    "{}: {token:?} didn't parse",
                    def.name
                );
            }
        }
    }

    #[test]
    fn punctuation_names_and_characters_are_the_same_key() {
        assert_eq!(PUNCTUATION.len(), 32);
        for &(name, c) in PUNCTUATION {
            let literal = parsed(&c.to_string());
            assert_eq!(literal, plain(KeyCode::Char(c)));
            assert_eq!(parsed(name), literal, "{name}");
            assert_eq!(KeySpec::from_event_name(name), KeySpec::Chord(literal), "{name}");
            assert_eq!(KeySpec::display(name), c.to_string(), "{name}");
        }
        assert_eq!(
            parsed("ctrl+question_mark"),
            KeyChord::new(KeyCode::Char('?'), KeyModifiers::CONTROL)
        );
    }

    #[rstest]
    #[case("q, escape", ["q", "escape"])]
    #[case("q,escape", ["q", "escape"])]
    #[case(" enter , space ", ["enter", "space"])]
    #[case("x, ctrl+t", ["x", "ctrl+t"])]
    #[case("enter,space", ["enter", "space"])]
    fn lists_split_on_commas(#[case] list: &str, #[case] tokens: [&str; 2]) {
        let keys = KeySpec::parse_list(list);
        let expected: Vec<_> = tokens
            .iter()
            .map(|&token| (token.to_owned(), KeySpec::parse(token)))
            .collect();
        assert_eq!(keys, expected);
        assert!(keys.iter().all(|(_, spec)| matches!(spec, KeySpec::Chord(_))));
    }

    #[rstest]
    #[case("")]
    #[case("  ")]
    #[case(" , ")]
    fn blank_lists_have_no_keys(#[case] list: &str) {
        assert_eq!(KeySpec::parse_list(list), []);
    }

    #[rstest]
    #[case("M", "shift+m")]
    #[case("ctrl+F", "ctrl+shift+f")]
    #[case("ctrl+i", "ctrl+i")]
    fn spellings_of_the_same_key(#[case] a: &str, #[case] b: &str) {
        assert_eq!(parsed(a), parsed(b));
    }

    #[rstest]
    #[case("bogus")]
    #[case("ctrl+")]
    #[case("hyperspace+x")]
    #[case("Enter")]
    #[case("f0")]
    #[case("f25")]
    #[case("f01")]
    #[case("ctrl++")]
    fn unknown_tokens(#[case] token: &str) {
        assert_eq!(KeySpec::parse(token), KeySpec::Unknown(token.to_owned()));
    }

    #[test]
    fn uppercase_letters_match_with_or_without_shift() {
        let upper = parsed("M");
        assert_eq!(upper, plain(KeyCode::Char('M')));
        assert_eq!(chord(event(KeyCode::Char('M'), KeyModifiers::NONE)), Some(upper));
        assert_eq!(chord(event(KeyCode::Char('M'), KeyModifiers::SHIFT)), Some(upper));
        assert_eq!(chord(event(KeyCode::Char('m'), KeyModifiers::SHIFT)), Some(upper));
        assert_ne!(chord(event(KeyCode::Char('m'), KeyModifiers::NONE)), Some(upper));
    }

    #[test]
    fn shift_stays_on_space_and_never_matches_shifted_characters() {
        let shift_space = parsed("shift+space");
        assert_eq!(chord(event(KeyCode::Char(' '), KeyModifiers::SHIFT)), Some(shift_space));
        assert_ne!(chord(event(KeyCode::Char(' '), KeyModifiers::NONE)), Some(shift_space));
        for (spec, plain_key) in [("shift+1", '1'), ("shift+?", '?'), ("shift+question_mark", '?')] {
            let spec = parsed(spec);
            for mods in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
                assert_ne!(chord(event(KeyCode::Char(plain_key), mods)), Some(spec));
            }
        }
        // A shifted character arrives as itself, with or without SHIFT reported.
        assert_eq!(chord(event(KeyCode::Char('!'), KeyModifiers::SHIFT)), Some(parsed("!")));
        assert_eq!(
            chord(event(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Some(parsed("question_mark"))
        );
        assert_eq!(KeySpec::display("shift+1"), "shift+1");
    }

    #[test]
    fn ctrl_letters_are_lowercased() {
        assert_eq!(
            chord(event(KeyCode::Char('F'), KeyModifiers::CONTROL)),
            Some(parsed("ctrl+shift+f"))
        );
        assert_eq!(
            chord(event(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            Some(parsed("ctrl+f"))
        );
    }

    #[test]
    fn back_tab_is_shift_tab() {
        assert_eq!(
            chord(event(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(parsed("shift+tab"))
        );
        assert_eq!(
            chord(event(KeyCode::BackTab, KeyModifiers::NONE)),
            Some(parsed("shift+tab"))
        );
        assert_eq!(
            parsed("shift+tab").to_event(),
            event(KeyCode::BackTab, KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn enter_is_never_ctrl_m() {
        let ctrl_m = parsed("ctrl+m");
        assert_ne!(chord(event(KeyCode::Enter, KeyModifiers::NONE)), Some(ctrl_m));
        assert_eq!(chord(event(KeyCode::Char('m'), KeyModifiers::CONTROL)), Some(ctrl_m));
    }

    #[test]
    fn shift_enter_needs_shift() {
        let shift_enter = parsed("shift+enter");
        assert_eq!(chord(event(KeyCode::Enter, KeyModifiers::SHIFT)), Some(shift_enter));
        assert_ne!(chord(event(KeyCode::Enter, KeyModifiers::NONE)), Some(shift_enter));
    }

    #[test]
    fn tab_and_escape_have_no_ctrl_aliases() {
        assert_ne!(chord(event(KeyCode::Tab, KeyModifiers::NONE)), Some(parsed("ctrl+i")));
        assert_ne!(
            chord(event(KeyCode::Esc, KeyModifiers::NONE)),
            Some(parsed("ctrl+left_square_bracket"))
        );
    }

    #[test]
    fn release_is_ignored() {
        let mut release = event(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(chord(release), None);
        let mut repeat = event(KeyCode::Char('q'), KeyModifiers::NONE);
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(chord(repeat), Some(parsed("q")));
    }

    #[rstest]
    #[case("ctrl+p", "^p")]
    #[case("?", "?")]
    #[case("question_mark", "?")]
    #[case("slash", "/")]
    #[case("ctrl+f", "^f")]
    #[case("ctrl+v", "^v")]
    #[case("$", "$")]
    #[case("^", "^")]
    #[case("enter", "⏎")]
    #[case("escape", "esc")]
    #[case("shift+enter", "shift+⏎")]
    #[case("ctrl+n", "^n")]
    #[case("ctrl+pageup", "^pgup")]
    #[case("ctrl+shift+m", "shift+^m")]
    #[case("super+c", "super+c")]
    #[case("shift+tab", "shift+tab")]
    #[case("up", "↑")]
    #[case("backspace", "⌫")]
    #[case("minus", "-")]
    #[case("delete", "del")]
    #[case("pagedown", "pgdn")]
    #[case("M", "M")]
    #[case("f5", "f5")]
    #[case("home", "home")]
    #[case("bogus", "bogus")]
    #[case(" h ", "h")]
    fn displays(#[case] token: &str, #[case] expected: &str) {
        assert_eq!(KeySpec::display(token), expected);
    }

    fn named(chord: KeyChord) -> String {
        textual_name(&chord).unwrap_or_else(|| panic!("{chord:?} has no name"))
    }

    /// Every key a Settings row can record: printable ASCII, `space`, the named keys and `f1`-`f24`.
    fn recordable_codes() -> Vec<KeyCode> {
        let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
        codes.extend(
            NAMED_KEYS
                .iter()
                .map(|&(_, code)| code)
                .filter(|code| !matches!(code, KeyCode::Tab | KeyCode::Esc | KeyCode::Char(_))),
        );
        codes.extend((1..=24).map(KeyCode::F));
        codes
    }

    #[test]
    fn textual_names_round_trip_with_the_parser() {
        let mods = [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ];
        for code in recordable_codes() {
            for mods in mods {
                let chord = KeyChord::new(code, mods);
                let name = named(chord);
                assert_eq!(parsed(&name), chord, "{name}");
                assert_eq!(named(parsed(&name)), name);
            }
        }
    }

    #[rstest]
    #[case(plain(KeyCode::Char('a')), "a")]
    #[case(plain(KeyCode::Char('A')), "A")]
    #[case(plain(KeyCode::Char('1')), "1")]
    #[case(plain(KeyCode::Char('é')), "é")]
    #[case(plain(KeyCode::Char('£')), "£")]
    #[case(plain(KeyCode::Char('!')), "exclamation_mark")]
    #[case(plain(KeyCode::Char('?')), "question_mark")]
    #[case(plain(KeyCode::Char('/')), "slash")]
    #[case(plain(KeyCode::Char('^')), "circumflex_accent")]
    #[case(plain(KeyCode::Char(' ')), "space")]
    #[case(plain(KeyCode::Enter), "enter")]
    #[case(plain(KeyCode::PageDown), "pagedown")]
    #[case(plain(KeyCode::F(12)), "f12")]
    #[case(ctrl('a'), "ctrl+a")]
    #[case(KeyChord::new(KeyCode::Char('X'), KeyModifiers::ALT), "alt+shift+x")]
    #[case(KeyChord::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SHIFT), "ctrl+shift+up")]
    #[case(KeyChord::new(KeyCode::Char(' '), KeyModifiers::SHIFT), "shift+space")]
    #[case(KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT), "shift+enter")]
    fn textual_names(#[case] chord: KeyChord, #[case] expected: &str) {
        assert_eq!(named(chord), expected);
    }

    #[test]
    fn non_ascii_characters_read_back_to_the_same_key() {
        let pound = plain(KeyCode::Char('£'));
        assert_eq!(parsed(&named(pound)), pound);
    }

    #[rstest]
    #[case(plain(KeyCode::Tab))]
    #[case(KeyChord::new(KeyCode::BackTab, KeyModifiers::NONE))]
    #[case(plain(KeyCode::Esc))]
    #[case(plain(KeyCode::CapsLock))]
    #[case(plain(KeyCode::F(25)))]
    fn keys_a_row_does_not_record(#[case] chord: KeyChord) {
        assert_eq!(textual_name(&chord), None);
    }
}
