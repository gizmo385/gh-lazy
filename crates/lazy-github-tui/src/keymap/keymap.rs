//! The keymap: which keys fire which binding.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};

use super::{BindingId, KeyChord};

/// The keys of every binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    keys: HashMap<BindingId, Vec<KeyChord>>,
}

impl Keymap {
    /// Every binding on its default keys.
    pub fn defaults() -> Self {
        let keys = BindingId::ALL.iter().map(|&id| (id, default_keys(id))).collect();
        Self { keys }
    }

    /// The keys that fire `id`.
    pub fn keys(&self, id: BindingId) -> &[KeyChord] {
        self.keys.get(&id).map_or(&[], Vec::as_slice)
    }

    /// The binding in one component's list `ids` that owns `chord`, if any. The first match in list order wins.
    pub fn match_in(&self, ids: &[BindingId], chord: &KeyChord) -> Option<BindingId> {
        ids.iter().copied().find(|&id| self.keys(id).contains(chord))
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

fn default_keys(id: BindingId) -> Vec<KeyChord> {
    let key = match id {
        BindingId::AppQuit => KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE),
        BindingId::TextualQuit => KeyChord::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        BindingId::TextualHelpQuit => KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    };
    vec![key]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_in_only_looks_at_the_listed_ids() {
        let keymap = Keymap::defaults();
        let q = KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            keymap.match_in(&[BindingId::TextualQuit, BindingId::AppQuit], &q),
            Some(BindingId::AppQuit)
        );
        assert_eq!(keymap.match_in(&[BindingId::TextualQuit], &q), None);
    }
}
