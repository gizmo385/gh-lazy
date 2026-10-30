//! A list setting's editor: an "Add new item..." input over a checklist of the items, every one checked to start
//! with. `enter` in the input adds its text as a new checked item; unchecking an item drops it on Save. Python's
//! `ListSettingWidget`.

use crate::components::selection_list::SelectionList;
use crate::components::text_input::{self, TextInput};

/// The input's placeholder.
const PLACEHOLDER: &str = "Add new item...";

/// The checklist's `max-height: 10`, border included.
const MAX_LIST_HEIGHT: u16 = 10;

/// The blank row between the input and the checklist (`margin-top: 1`).
const LIST_MARGIN: u16 = 1;

/// The input and the checklist of one list setting.
#[derive(Debug, Clone)]
pub(super) struct ListField {
    pub(super) input: TextInput,
    pub(super) list: SelectionList<String>,
}

impl ListField {
    /// The editor for `items`, all checked, in order. The first item is highlighted (nothing, when there are none),
    /// as Textual's `OptionList` built with options is, so `space` on a freshly focused list toggles it.
    pub(super) fn new(items: Vec<String>) -> Self {
        Self {
            input: TextInput::new(PLACEHOLDER),
            list: SelectionList::with_options(items.into_iter().map(|item| (item.clone(), item, true))),
        }
    }

    /// `enter` in the input: its text, stripped, becomes a new checked item at the end, and the input clears. Blank
    /// text, or an item already in the list (checked or not), changes nothing and the text stays. Says whether an
    /// item was added.
    pub(super) fn submit(&mut self) -> bool {
        let item = self.input.value().trim().to_owned();
        if item.is_empty() || self.list.contains(&item) {
            return false;
        }
        self.list.add(item.clone(), item, true);
        self.input.set_value("");
        true
    }

    /// The checked items in the order they were checked: the original items first, in config order, then each
    /// newly checked one (Textual's `SelectionList.selected`).
    pub(super) fn value(&self) -> Vec<String> {
        self.list.selected().into_iter().cloned().collect()
    }

    /// The checklist's rows: its items and border, at most [`MAX_LIST_HEIGHT`].
    pub(super) fn list_height(&self) -> u16 {
        self.list.height().min(MAX_LIST_HEIGHT)
    }

    /// The rows from the input's top to the checklist's bottom.
    pub(super) fn height(&self) -> u16 {
        text_input::HEIGHT + LIST_MARGIN + self.list_height()
    }

    /// The checklist's first row, from the field's top.
    pub(super) fn list_top() -> u16 {
        text_input::HEIGHT + LIST_MARGIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::BindingId;

    fn field(items: &[&str]) -> ListField {
        ListField::new(items.iter().map(|&item| item.to_owned()).collect())
    }

    #[test]
    fn enter_adds_the_stripped_text_checked_at_the_end_and_clears_the_input() {
        let mut list = field(&["a"]);
        list.input.set_value("  b  ");
        assert!(list.submit());
        assert_eq!(list.input.value(), "");
        assert_eq!(list.list.labels().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(list.value(), ["a", "b"]);
    }

    #[test]
    fn blank_or_duplicate_text_adds_nothing_and_stays() {
        let mut list = field(&["a", "b"]);
        for text in ["", "   ", "a", " b "] {
            list.input.set_value(text);
            assert!(!list.submit(), "{text:?}");
            assert_eq!(list.input.value(), text);
            assert_eq!(list.list.len(), 2);
        }
        // An unchecked item is still an item.
        let _ = list.list.on_binding(BindingId::TextualSelectionListSelect);
        assert_eq!(list.value(), ["b"]);
        list.input.set_value("a");
        assert!(!list.submit());
        assert_eq!(list.input.value(), "a");
        assert_eq!(list.value(), ["b"]);
    }

    #[test]
    fn the_value_keeps_the_order_items_were_checked_in() {
        let mut list = field(&["a", "b", "c"]);
        // Uncheck "a", then check it again: it goes last.
        let _ = list.list.on_binding(BindingId::TextualSelectionListSelect);
        assert_eq!(list.value(), ["b", "c"]);
        let _ = list.list.on_binding(BindingId::TextualSelectionListSelect);
        assert_eq!(list.value(), ["b", "c", "a"]);
        list.input.set_value("d");
        assert!(list.submit());
        assert_eq!(list.value(), ["b", "c", "a", "d"]);
    }

    #[test]
    fn the_list_starts_on_its_first_item_and_caps_its_height() {
        let list = field(&["a", "b"]);
        assert_eq!(list.list.highlighted(), Some(0));
        assert_eq!(field(&[]).list.highlighted(), None);
        assert_eq!(list.list_height(), 4);
        assert_eq!(list.height(), 8);
        assert_eq!(field(&[]).height(), 6);
        let many: Vec<String> = (0..20).map(|n| n.to_string()).collect();
        assert_eq!(ListField::new(many).list_height(), 10);
    }
}
