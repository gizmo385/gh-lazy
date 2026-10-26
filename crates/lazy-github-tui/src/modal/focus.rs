//! A dialog's focus: which of its fields has it, and how `tab` and `shift+tab` move it. Textual's
//! `Screen._move_focus` and `Widget.watch_disabled`, over a dialog's own field enum instead of the widget tree.

/// Which way `tab` (`Next`) or `shift+tab` (`Previous`) moves focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMove {
    /// `tab`: the next field in screen order.
    Next,
    /// `shift+tab`: the previous one.
    Previous,
}

/// The focusable fields of a dialog in screen order (top to bottom, left to right), which are disabled, and which one
/// has focus.
///
/// `F` is the dialog's own field enum (`enum LookupField { Input, Track, Submit, Cancel }`), so a dialog names its
/// fields instead of indexing them. Labels, rules and markdown aren't fields: they're never focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusRing<F> {
    order: Vec<F>,
    disabled: Vec<F>,
    focused: Option<F>,
}

impl<F: Copy + Eq> FocusRing<F> {
    /// A ring over `order` with every field enabled and the first one focused, as Textual focuses the first focusable
    /// widget when a screen opens.
    pub fn new(order: impl IntoIterator<Item = F>) -> Self {
        let order: Vec<F> = order.into_iter().collect();
        Self {
            focused: order.first().copied(),
            order,
            disabled: Vec::new(),
        }
    }

    /// The focused field, or `None` when nothing is.
    pub fn focused(&self) -> Option<F> {
        self.focused
    }

    /// Whether `field` has focus.
    pub fn is_focused(&self, field: F) -> bool {
        self.focused == Some(field)
    }

    /// `tab`: the next enabled field, wrapping from the last to the first. From no focus, the first enabled field.
    pub fn next(&mut self) {
        self.step(FocusMove::Next);
    }

    /// `shift+tab`: the previous enabled field, wrapping from the first to the last. From no focus, the first enabled
    /// field, as in Textual.
    pub fn previous(&mut self) {
        self.step(FocusMove::Previous);
    }

    /// [`Self::next`] or [`Self::previous`].
    pub fn step(&mut self, direction: FocusMove) {
        let enabled: Vec<F> = self
            .order
            .iter()
            .copied()
            .filter(|&field| self.is_enabled(field))
            .collect();
        let current = self
            .focused
            .and_then(|focused| enabled.iter().position(|&field| field == focused));
        self.focused = match current {
            None => enabled.first().copied(),
            Some(index) => {
                let len = enabled.len();
                let to = match direction {
                    FocusMove::Next => (index + 1) % len,
                    FocusMove::Previous => (index + len - 1) % len,
                };
                Some(enabled[to])
            }
        };
    }

    /// Focuses `field`, as a click does. False, and no change, when it's disabled or not in the ring.
    pub fn focus(&mut self, field: F) -> bool {
        if !self.order.contains(&field) || !self.is_enabled(field) {
            return false;
        }
        self.focused = Some(field);
        true
    }

    /// Enables or disables `field`. A disabled field is skipped by [`Self::next`] and [`Self::previous`] and can't be
    /// focused; disabling the focused field leaves nothing focused, as Textual blurs a widget when it's disabled.
    pub fn set_disabled(&mut self, field: F, disabled: bool) {
        if disabled {
            if !self.disabled.contains(&field) {
                self.disabled.push(field);
            }
            if self.focused == Some(field) {
                self.focused = None;
            }
        } else {
            self.disabled.retain(|&other| other != field);
        }
    }

    /// Whether `field` isn't disabled.
    pub fn is_enabled(&self, field: F) -> bool {
        !self.disabled.contains(&field)
    }

    /// Replaces the order, for a dialog whose fields come and go (Create PR's reviewers). The focused field keeps
    /// focus if it's still there; otherwise nothing is focused. Disabled fields stay disabled.
    pub fn set_order(&mut self, order: impl IntoIterator<Item = F>) {
        self.order = order.into_iter().collect();
        if self.focused.is_some_and(|focused| !self.order.contains(&focused)) {
            self.focused = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lookup repository's fields.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Field {
        Input,
        Track,
        Submit,
        Cancel,
    }

    use Field::*;

    fn lookup() -> FocusRing<Field> {
        FocusRing::new([Input, Track, Submit, Cancel])
    }

    #[test]
    fn the_first_field_is_focused_on_creation() {
        assert_eq!(lookup().focused(), Some(Input));
        assert_eq!(FocusRing::<Field>::new([]).focused(), None);
    }

    #[test]
    fn next_and_previous_walk_the_order_and_wrap() {
        let mut ring = lookup();
        let mut seen = Vec::new();
        for _ in 0..5 {
            ring.next();
            seen.push(ring.focused().unwrap());
        }
        assert_eq!(seen, [Track, Submit, Cancel, Input, Track]);
        let mut seen = Vec::new();
        for _ in 0..5 {
            ring.previous();
            seen.push(ring.focused().unwrap());
        }
        assert_eq!(seen, [Input, Cancel, Submit, Track, Input]);
    }

    #[test]
    fn disabled_fields_are_skipped() {
        let mut ring = lookup();
        ring.set_disabled(Track, true);
        ring.set_disabled(Cancel, true);
        ring.next();
        assert_eq!(ring.focused(), Some(Submit));
        ring.next();
        assert_eq!(ring.focused(), Some(Input));
        ring.previous();
        assert_eq!(ring.focused(), Some(Submit));
        ring.set_disabled(Cancel, false);
        ring.next();
        assert_eq!(ring.focused(), Some(Cancel));
    }

    #[test]
    fn disabling_the_focused_field_leaves_nothing_focused() {
        let mut ring = lookup();
        ring.next();
        ring.next();
        assert_eq!(ring.focused(), Some(Submit));
        ring.set_disabled(Submit, true);
        assert_eq!(ring.focused(), None);
        // From nothing, both directions go to the first enabled field.
        ring.next();
        assert_eq!(ring.focused(), Some(Input));
        ring.set_disabled(Input, true);
        assert_eq!(ring.focused(), None);
        ring.previous();
        assert_eq!(ring.focused(), Some(Track));
        // Re-enabling doesn't take focus back.
        ring.set_disabled(Submit, false);
        assert_eq!(ring.focused(), Some(Track));
    }

    #[test]
    fn a_ring_with_every_field_disabled_focuses_nothing() {
        let mut ring = FocusRing::new([Submit]);
        ring.set_disabled(Submit, true);
        ring.next();
        ring.previous();
        assert_eq!(ring.focused(), None);
    }

    #[test]
    fn focus_moves_to_an_enabled_field_only() {
        let mut ring = lookup();
        assert!(ring.focus(Cancel));
        assert_eq!(ring.focused(), Some(Cancel));
        ring.set_disabled(Submit, true);
        assert!(!ring.focus(Submit));
        assert_eq!(ring.focused(), Some(Cancel));
        let mut short = FocusRing::new([Input]);
        assert!(!short.focus(Cancel));
        assert_eq!(short.focused(), Some(Input));
    }

    #[test]
    fn a_new_order_keeps_focus_on_a_field_that_stays() {
        let mut ring = lookup();
        ring.focus(Submit);
        ring.set_order([Input, Submit, Cancel]);
        assert_eq!(ring.focused(), Some(Submit));
        ring.next();
        assert_eq!(ring.focused(), Some(Cancel));
        ring.set_order([Input, Submit]);
        assert_eq!(ring.focused(), None);
    }
}
