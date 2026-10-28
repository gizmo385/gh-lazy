//! Undo and redo: a port of Textual's `EditHistory` (`textual/document/_history.py`), with its batching rules.
//!
//! Edits are grouped into batches, and undo and redo work a batch at a time. A new batch starts when the stack is
//! empty, after a [`EditHistory::checkpoint`] (a cursor move, the editor gaining focus, a redo), for an edit of more
//! than one character or with a line break (which also ends its own batch), when an edit switches between inserting
//! and replacing or deleting, after [`CHECKPOINT_TIMER`] without edits, or once [`CHECKPOINT_MAX_CHARACTERS`] typed
//! characters accumulate. Up to [`MAX_CHECKPOINTS`] batches are kept, and any new edit clears the redo stack.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

use super::Location;

/// Batches kept for undo (`max_checkpoints`).
pub(super) const MAX_CHECKPOINTS: usize = 50;
/// A pause between edits this long ends the batch (`checkpoint_timer`).
pub(super) const CHECKPOINT_TIMER: Duration = Duration::from_secs(2);
/// Characters a batch holds at most (`checkpoint_max_characters`).
pub(super) const CHECKPOINT_MAX_CHARACTERS: usize = 100;

/// One edit as performed: enough to undo it (put `replaced` back between `top` and `end`) and to redo it (put `text`
/// between `top` and `bottom` again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Edit {
    /// Where the replaced range started.
    pub(super) top: Location,
    /// Where it ended, before the edit.
    pub(super) bottom: Location,
    /// What went in.
    pub(super) text: String,
    /// What was there, lines joined with `\n`.
    pub(super) replaced: String,
    /// Where the inserted text ends, after the edit.
    pub(super) end: Location,
    /// The selection (anchor, cursor) before the edit, which undo puts back.
    pub(super) selection_before: (Location, Location),
}

/// The undo and redo stacks.
#[derive(Debug, Clone, Default)]
pub(super) struct EditHistory {
    undo: VecDeque<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
    /// When the last edit was recorded; `None` until one is.
    last_edit: Option<Instant>,
    /// Characters inserted by the current batch.
    characters: usize,
    /// The next edit starts a batch.
    force_end_batch: bool,
    /// The last edit replaced or deleted text.
    previously_replaced: bool,
}

impl EditHistory {
    /// Records an edit made at `now`. An edit that neither inserted nor removed anything isn't kept.
    pub(super) fn record(&mut self, edit: Edit, now: Instant) {
        if edit.text.is_empty() && edit.replaced.is_empty() {
            return;
        }
        let is_replacement = !edit.replaced.is_empty();
        let characters = edit.text.chars().count();
        let contains_newline = edit.text.contains('\n') || edit.replaced.contains('\n');
        let timed_out = self
            .last_edit
            .is_some_and(|last| now.saturating_duration_since(last) > CHECKPOINT_TIMER);
        let new_batch = self.undo.is_empty()
            || self.force_end_batch
            || characters > 1
            || contains_newline
            || is_replacement != self.previously_replaced
            || timed_out
            || self.characters + characters > CHECKPOINT_MAX_CHARACTERS;
        match self.undo.back_mut() {
            Some(batch) if !new_batch => {
                batch.push(edit);
                self.characters += characters;
            }
            _ => {
                self.undo.push_back(vec![edit]);
                if self.undo.len() > MAX_CHECKPOINTS {
                    self.undo.pop_front();
                }
                self.characters = characters;
                self.force_end_batch = false;
            }
        }
        self.last_edit = Some(now);
        self.previously_replaced = is_replacement;
        self.redo.clear();
        if contains_newline || characters > 1 {
            self.checkpoint();
        }
    }

    /// The next edit starts a new batch.
    pub(super) fn checkpoint(&mut self) {
        self.force_end_batch = true;
    }

    /// Takes the newest batch for undo, keeping it for redo. Its edits are in the order they were made.
    pub(super) fn pop_undo(&mut self) -> Option<Vec<Edit>> {
        let batch = self.undo.pop_back()?;
        self.redo.push(batch.clone());
        Some(batch)
    }

    /// Takes the newest undone batch for redo, keeping it for undo, and ends the current batch.
    pub(super) fn pop_redo(&mut self) -> Option<Vec<Edit>> {
        let batch = self.redo.pop()?;
        self.undo.push_back(batch.clone());
        self.checkpoint();
        Some(batch)
    }

    /// Forgets everything, as loading new text does.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    #[cfg(test)]
    pub(super) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[cfg(test)]
    pub(super) fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str, at: usize) -> Edit {
        Edit {
            top: (0, at),
            bottom: (0, at),
            text: text.to_owned(),
            replaced: String::new(),
            end: (0, at + text.chars().count()),
            selection_before: ((0, at), (0, at)),
        }
    }

    fn deleted(replaced: &str, at: usize) -> Edit {
        Edit {
            top: (0, at),
            bottom: (0, at + replaced.chars().count()),
            text: String::new(),
            replaced: replaced.to_owned(),
            end: (0, at),
            selection_before: ((0, at), (0, at)),
        }
    }

    /// The size of each batch on the undo stack, oldest first.
    fn batches(history: &EditHistory) -> Vec<usize> {
        history.undo.iter().map(Vec::len).collect()
    }

    #[test]
    fn typing_batches_until_a_pause() {
        let start = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), start);
        history.record(typed("b", 1), start + Duration::from_secs(1));
        history.record(typed("c", 2), start + Duration::from_millis(3001));
        // Two seconds exactly isn't a pause; more is.
        history.record(typed("d", 3), start + Duration::from_millis(5001));
        history.record(typed("e", 4), start + Duration::from_millis(7002));
        assert_eq!(batches(&history), [2, 2, 1]);
    }

    #[test]
    fn a_batch_holds_100_characters() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        for i in 0..105 {
            history.record(typed("x", i), now);
        }
        assert_eq!(batches(&history), [100, 5]);
    }

    #[test]
    fn switching_between_inserting_and_deleting_starts_a_batch() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), now);
        history.record(typed("b", 1), now);
        history.record(deleted("b", 1), now);
        history.record(deleted("a", 0), now);
        history.record(typed("c", 0), now);
        assert_eq!(batches(&history), [2, 2, 1]);
    }

    #[test]
    fn line_breaks_and_pastes_are_batches_of_their_own() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), now);
        history.record(typed("\n", 1), now);
        history.record(typed("b", 0), now);
        history.record(typed("pasted", 1), now);
        history.record(typed("c", 7), now);
        // Deleting a line break too.
        history.record(deleted("\n", 8), now);
        history.record(deleted("d", 8), now);
        assert_eq!(batches(&history), [1, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn checkpoints_start_a_batch() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), now);
        // A cursor move, or the editor gaining focus.
        history.checkpoint();
        history.record(typed("b", 1), now);
        history.record(typed("c", 2), now);
        assert_eq!(batches(&history), [1, 2]);
    }

    #[test]
    fn edits_that_change_nothing_are_not_kept() {
        let mut history = EditHistory::default();
        history.record(typed("", 0), Instant::now());
        assert_eq!(history.undo_len(), 0);
    }

    #[test]
    fn fifty_batches_are_kept() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        for i in 0..60 {
            history.record(typed("\n", i), now);
        }
        assert_eq!(history.undo_len(), MAX_CHECKPOINTS);
        // The oldest went first.
        assert_eq!(history.undo.front().map(|batch| batch[0].top), Some((0, 10)));
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), now);
        history.checkpoint();
        history.record(typed("b", 1), now);
        assert!(history.pop_undo().is_some());
        assert_eq!((history.undo_len(), history.redo_len()), (1, 1));
        history.record(typed("c", 1), now);
        assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
        assert!(history.pop_redo().is_none());
    }

    #[test]
    fn redo_ends_the_batch() {
        let now = Instant::now();
        let mut history = EditHistory::default();
        history.record(typed("a", 0), now);
        let _ = history.pop_undo();
        let _ = history.pop_redo();
        history.record(typed("b", 1), now);
        assert_eq!(batches(&history), [1, 1]);
    }
}
