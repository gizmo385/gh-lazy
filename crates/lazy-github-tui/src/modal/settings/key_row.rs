//! One binding of the Key Bindings list: its id and the keys it's set to, as `bindings.overrides` writes them.

use indexmap::IndexMap;

use crate::fuzzy::fuzzy_match;
use crate::keymap::{BINDING_GROUPS, BindingId, KeySpec};

/// One rebindable binding and the key string it's set to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyRow {
    pub(super) id: BindingId,
    /// The `bindings.overrides` key, such as `app.open_help`.
    pub(super) config_id: &'static str,
    /// The override as written, or the default key string; then whatever was captured.
    pub(super) value: String,
}

impl KeyRow {
    /// Every binding `bindings.overrides` can name, in [`BINDING_GROUPS`] order with each binding's group index,
    /// each showing its override from `overrides` or else its default keys.
    pub(super) fn all(overrides: &IndexMap<String, String>) -> Vec<(usize, Self)> {
        BINDING_GROUPS
            .iter()
            .enumerate()
            .flat_map(|(group, g)| g.ids.iter().map(move |&id| (group, id)))
            .filter_map(|(group, id)| {
                let config_id = id.config_id()?;
                let value = overrides
                    .get(config_id)
                    .cloned()
                    .unwrap_or_else(|| id.def().default_keys.to_owned());
                Some((group, Self { id, config_id, value }))
            })
            .collect()
    }

    /// The action column: [`action_label`].
    pub(super) fn action(&self) -> String {
        action_label(self.id)
    }

    /// The keys as the footer shows them, comma-separated (`?`, `^p`, `q, esc`). A binding's `key_display` applies
    /// while it's at its default keys, as [`crate::keymap::Keymap::key_display`] applies it until an override.
    pub(super) fn keys_display(&self) -> String {
        Self::display(self.id, &self.value, self.is_default())
    }

    /// The default keys, displayed the same way.
    pub(super) fn default_display(&self) -> String {
        Self::display(self.id, self.id.def().default_keys, true)
    }

    fn display(id: BindingId, keys: &str, default: bool) -> String {
        let key_display = id.def().key_display.filter(|_| default);
        KeySpec::parse_list(keys)
            .iter()
            .map(|(token, _)| key_display.map_or_else(|| KeySpec::display(token), str::to_owned))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Replace mode: the binding's keys become the recorded key `name` alone.
    pub(super) fn replace(&mut self, name: String) {
        self.value = name;
    }

    /// Add mode: appends the recorded key `name`, unless the binding already has that key. Returns whether it
    /// changed.
    pub(super) fn add(&mut self, name: &str) -> bool {
        let spec = KeySpec::parse(name);
        let keys = KeySpec::parse_list(&self.value);
        if keys.iter().any(|(_, key)| *key == spec) {
            return false;
        }
        self.value = if keys.is_empty() {
            name.to_owned()
        } else {
            format!("{}, {name}", self.value.trim_end())
        };
        true
    }

    /// Sets the binding back to its default keys. Returns whether it changed.
    pub(super) fn reset(&mut self) -> bool {
        if self.is_default() {
            return false;
        }
        self.id.def().default_keys.clone_into(&mut self.value);
        true
    }

    /// Whether the keys parse to the default keys, in the same order: `question_mark` is the default `?`, and
    /// `escape, q` isn't `q, escape` (decision settings-key-bindings-list, D5).
    pub(super) fn is_default(&self) -> bool {
        let specs =
            |keys: &str| -> Vec<KeySpec> { KeySpec::parse_list(keys).into_iter().map(|(_, key)| key).collect() };
        specs(&self.value) == specs(self.id.def().default_keys)
    }

    /// Whether the search `query` shows the binding: it fuzzy-matches the description or the id.
    pub(super) fn matches(&self, query: &str) -> bool {
        let description = self.id.def().description;
        (!description.is_empty() && fuzzy_match(query, description).is_some())
            || fuzzy_match(query, self.config_id).is_some()
    }

    /// Writes the binding into `overrides` as Save does: one at its default keys removes the override; any other
    /// writes its keys as text, in place if there was one.
    pub(super) fn save(&self, overrides: &mut IndexMap<String, String>) {
        if self.is_default() {
            overrides.shift_remove(self.config_id);
        } else {
            overrides.insert(self.config_id.to_owned(), self.value.clone());
        }
    }
}

/// What the list calls binding `id`: its description, or for a binding with none its id's last part humanized
/// (`diff.cursor_down` is "Cursor down").
pub(super) fn action_label(id: BindingId) -> String {
    let def = id.def();
    if !def.description.is_empty() {
        return def.description.to_owned();
    }
    let last = def.name.rsplit('.').next().unwrap_or(def.name);
    let mut words = last.replace('_', " ");
    if let Some(first) = words.get(..1) {
        let upper = first.to_uppercase();
        words.replace_range(..1, &upper);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: BindingId, value: &str) -> KeyRow {
        KeyRow {
            id,
            config_id: id.config_id().expect("rebindable"),
            value: value.to_owned(),
        }
    }

    #[test]
    fn rows_come_in_group_order() {
        let rows = KeyRow::all(&IndexMap::new());
        // Python's 59 ids, the nine Rust-only diff ids, `main.refresh`, `search.open`, `search.save_query`,
        // `issue.milestone` and the three compose keys.
        assert_eq!(rows.len(), 75);
        assert_eq!(rows[0], (0, row(BindingId::AppQuit, "q")));
        assert_eq!(rows[3].1.id, BindingId::AppOpenHelp);
        assert_eq!(
            rows.last().map(|(group, row)| (*group, row.id)),
            Some((11, BindingId::ConfirmNo))
        );
    }

    #[test]
    fn a_binding_without_a_description_is_named_by_its_id() {
        assert_eq!(row(BindingId::DiffCursorDown, "j").action(), "Cursor down");
        assert_eq!(row(BindingId::AppOpenHelp, "?").action(), "Keybindings");
    }

    #[test]
    fn keys_display_as_the_footer_shows_them() {
        assert_eq!(row(BindingId::ModalClose, "q, escape").keys_display(), "q, esc");
        assert_eq!(row(BindingId::AppCommandPalette, "ctrl+p").keys_display(), "^p");
        assert_eq!(row(BindingId::AppOpenHelp, "exclamation_mark").keys_display(), "!");
        assert_eq!(row(BindingId::AppOpenHelp, "question_mark").keys_display(), "?");
    }

    #[test]
    fn the_default_check_compares_parsed_keys_in_order() {
        assert!(row(BindingId::AppOpenHelp, "question_mark").is_default());
        assert!(row(BindingId::AppOpenHelp, "?").is_default());
        assert!(row(BindingId::ModalClose, "q,escape").is_default());
        assert!(!row(BindingId::ModalClose, "escape, q").is_default());
        assert!(!row(BindingId::AppOpenHelp, "!").is_default());
    }

    #[test]
    fn add_appends_a_new_key_and_skips_one_it_has() {
        let mut quit = row(BindingId::AppQuit, "q");
        assert!(quit.add("x"));
        assert_eq!(quit.value, "q, x");
        assert!(!quit.add("q"));
        assert!(!quit.add("x"));
        assert_eq!(quit.value, "q, x");
        let mut help = row(BindingId::AppOpenHelp, "?");
        assert!(!help.add("question_mark"));
    }

    #[test]
    fn reset_restores_the_default_string() {
        let mut quit = row(BindingId::AppQuit, "x");
        assert!(quit.reset());
        assert_eq!(quit.value, "q");
        assert!(!quit.reset());
    }

    #[test]
    fn save_removes_defaults_writes_changes_and_keeps_unknown_ids() {
        let mut overrides: IndexMap<String, String> = [
            ("not.a_binding".to_owned(), "z".to_owned()),
            ("app.open_help".to_owned(), "h".to_owned()),
            ("app.quit".to_owned(), "x".to_owned()),
        ]
        .into_iter()
        .collect();
        row(BindingId::AppOpenHelp, "question_mark").save(&mut overrides);
        row(BindingId::AppQuit, "q, x").save(&mut overrides);
        row(BindingId::ModalClose, "escape, q").save(&mut overrides);
        let written: Vec<(&str, &str)> = overrides
            .iter()
            .map(|(id, keys)| (id.as_str(), keys.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                ("not.a_binding", "z"),
                ("app.quit", "q, x"),
                ("modal.close", "escape, q")
            ]
        );
    }
}
