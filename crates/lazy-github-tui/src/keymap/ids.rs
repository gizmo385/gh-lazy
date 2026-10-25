//! Binding ids.

/// A binding, by the id Python uses for it (`app.quit`) or, for Textual built-ins, an internal `textual.*` id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BindingId {
    /// `app.quit`: `q`.
    AppQuit,
    /// `textual.app.quit`: `ctrl+q`, a priority binding.
    TextualQuit,
    /// `textual.app.help_quit`: `ctrl+c`, which explains how to quit.
    TextualHelpQuit,
}

impl BindingId {
    /// Every id, in registry order.
    pub const ALL: &'static [BindingId] = &[BindingId::AppQuit, BindingId::TextualQuit, BindingId::TextualHelpQuit];

    /// The id as `bindings.overrides` spells it, or `None` for a Textual built-in, which can't be overridden.
    pub fn config_id(self) -> Option<&'static str> {
        match self {
            BindingId::AppQuit => Some("app.quit"),
            BindingId::TextualQuit | BindingId::TextualHelpQuit => None,
        }
    }

    /// Whether the binding is checked before the focused component sees the key (Textual's `priority=True`).
    pub fn is_priority(self) -> bool {
        matches!(self, BindingId::TextualQuit)
    }
}
