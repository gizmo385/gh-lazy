//! A row of the Key Bindings tab: a binding's label and the key it's set to, which takes the next key pressed while
//! it's focused. Python's `KeySelectionInput`.

use crossterm::event::{KeyCode, KeyModifiers};
use indexmap::IndexMap;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget};

use crate::fuzzy::fuzzy_match;
use crate::keymap::{BindingId, KeyChord, textual_name};
use crate::theme::{Paint, Theme};

/// A row's height: the label line and a blank one (`height: 2`).
pub(super) const HEIGHT: u16 = 2;

/// The focused row's height: one line inside a `solid $accent` border (`height: 3`).
pub(super) const FOCUSED_HEIGHT: u16 = 3;

/// One rebindable binding and the key string it's set to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyRow {
    pub(super) id: BindingId,
    /// The `bindings.overrides` key, such as `app.open_help`.
    config_id: &'static str,
    /// The override as written, or the default key string; then whatever was captured.
    pub(super) value: String,
}

impl KeyRow {
    /// Every binding `bindings.overrides` can name, sorted by id, each showing its override from `overrides` or else
    /// its default keys. Python's `BindingsSettingsSection.compose`.
    pub(super) fn all(overrides: &IndexMap<String, String>) -> Vec<Self> {
        let mut rows: Vec<Self> = BindingId::ALL
            .iter()
            .filter_map(|&id| {
                let config_id = id.config_id()?;
                let value = overrides
                    .get(config_id)
                    .cloned()
                    .unwrap_or_else(|| id.def().default_keys.to_owned());
                Some(Self { id, config_id, value })
            })
            .collect();
        rows.sort_by_key(|row| row.config_id);
        rows
    }

    /// The bold part: the description, or the id when it has none, and a colon.
    pub(super) fn label(&self) -> String {
        let description = self.id.def().description;
        let name = if description.is_empty() {
            self.config_id
        } else {
            description
        };
        format!("{name}: ")
    }

    /// A key pressed on the focused row: `escape` resets it to the default key string, and any other key Textual
    /// names replaces the value with that name. Returns whether the row took the key.
    pub(super) fn capture(&mut self, chord: &KeyChord) -> bool {
        if chord.code == KeyCode::Esc && chord.mods == KeyModifiers::NONE {
            self.id.def().default_keys.clone_into(&mut self.value);
            return true;
        }
        match textual_name(chord) {
            Some(name) => {
                self.value = name;
                true
            }
            None => false,
        }
    }

    /// Whether the search `query` shows the row: it fuzzy-matches the description or the id.
    pub(super) fn matches(&self, query: &str) -> bool {
        let description = self.id.def().description;
        (!description.is_empty() && fuzzy_match(query, description).is_some())
            || fuzzy_match(query, self.config_id).is_some()
    }

    /// Writes the row into `overrides` as Save does: a value that differs from the default key string, as text, is
    /// the id's override, kept in place if there was one; an equal one removes the override. Python's
    /// `_update_settings`, keybinding part.
    pub(super) fn save(&self, overrides: &mut IndexMap<String, String>) {
        if self.value == self.id.def().default_keys {
            overrides.shift_remove(self.config_id);
        } else {
            overrides.insert(self.config_id.to_owned(), self.value.clone());
        }
    }

    /// The rows the row takes.
    pub(super) fn height(focused: bool) -> u16 {
        if focused { FOCUSED_HEIGHT } else { HEIGHT }
    }

    /// Draws the row into `area` (its full height) over `under`: the bold label and the value, boxed in `$accent`
    /// while focused.
    pub(super) fn render(&self, buf: &mut Buffer, area: Rect, theme: &Theme, under: Paint, focused: bool) {
        let p = theme.palette;
        let text = Style::new().fg(theme.color(p.foreground, under));
        let mut line = area;
        if focused {
            Block::bordered()
                .border_style(Style::new().fg(theme.color(p.accent, under)))
                .render(area, buf);
            line = Rect {
                x: area.x.saturating_add(1),
                y: area.y.saturating_add(1),
                width: area.width.saturating_sub(2),
                height: 1,
            };
        }
        let line = line.intersection(area);
        if line.is_empty() {
            return;
        }
        let label = self.label();
        let width = usize::from(line.width);
        let (x, _) = buf.set_stringn(line.x, line.y, &label, width, text.add_modifier(Modifier::BOLD));
        let used = usize::from(x - line.x);
        buf.set_stringn(x, line.y, &self.value, width.saturating_sub(used), text);
    }
}
