//! Normalized keys.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

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
    /// - A character without CTRL or ALT folds SHIFT into the character: `M`, `shift+m` and `Char('M')` with SHIFT
    ///   are all `Char('M')` with no modifiers.
    /// - A character with CTRL or ALT is lowercased, keeping SHIFT if it was uppercase or SHIFT was set.
    /// - Other keys keep their modifiers, so plain Enter is never `ctrl+m`.
    pub fn from_event(event: &KeyEvent) -> Option<Self> {
        // `Repeat` counts as a press. Repeats only arrive with kitty's REPORT_EVENT_TYPES, which isn't pushed.
        if event.kind == KeyEventKind::Release {
            return None;
        }
        Some(normalize(event.code, event.modifiers))
    }
}

fn normalize(code: KeyCode, mods: KeyModifiers) -> KeyChord {
    match code {
        KeyCode::BackTab => KeyChord {
            code: KeyCode::Tab,
            mods: mods | KeyModifiers::SHIFT,
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(event: KeyEvent) -> Option<KeyChord> {
        KeyChord::from_event(&event)
    }

    #[test]
    fn release_is_ignored() {
        let mut event = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        event.kind = KeyEventKind::Release;
        assert_eq!(chord(event), None);
    }

    #[test]
    fn shift_folds_into_characters() {
        let upper = KeyChord::new(KeyCode::Char('M'), KeyModifiers::NONE);
        assert_eq!(upper.mods, KeyModifiers::NONE);
        assert_eq!(
            chord(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
            Some(upper)
        );
        assert_eq!(
            chord(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::SHIFT)),
            Some(upper)
        );
    }

    #[test]
    fn ctrl_letters_are_lowercased() {
        let ctrl_shift_f = KeyChord::new(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(
            chord(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::CONTROL)),
            Some(ctrl_shift_f)
        );
    }

    #[test]
    fn back_tab_is_shift_tab() {
        assert_eq!(
            chord(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(KeyChord {
                code: KeyCode::Tab,
                mods: KeyModifiers::SHIFT
            })
        );
    }
}
