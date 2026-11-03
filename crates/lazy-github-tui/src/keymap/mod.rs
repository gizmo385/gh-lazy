//! Binding ids, normalized keys and the keymap that matches one to the other.
//!
//! - [`BindingId`] names every binding: the ids of `lazy_github/lib/bindings.py`, with the same defaults, plus the
//!   Textual built-ins under internal `textual.*` ids. [`BindingId::def`] is its registry entry.
//! - [`KeyChord`] is a normalized key press, and [`KeySpec`] a key from a Textual key string (`ctrl+p`,
//!   `q, escape`).
//! - [`Keymap`] applies `bindings.overrides` and answers which binding a key fires. Components never match raw keys.
//! - [`active_bindings`] walks a binding chain for the footer and help panel.
//! - [`BINDING_GROUPS`] groups the rebindable bindings by area, for the Settings Key Bindings list.
//! - [`KEY_CONTEXTS`] lists the focus states and their chains, and [`conflicts`] the bindings that want one key in
//!   one of them, for that list's warnings.

mod active;
mod contexts;
mod groups;
mod ids;
mod key;
#[allow(clippy::module_inception)] // The file layout #73 specifies: ids.rs, key.rs, keymap.rs.
mod keymap;

pub use active::{ActiveBinding, active_bindings};
pub use contexts::{ConflictKind, KEY_CONTEXTS, KeyConflict, KeyContext, conflicts};
pub use groups::{BINDING_GROUPS, BindingGroup};
pub use ids::{BindingDef, BindingId};
pub use key::{KeyChord, KeySpec, textual_name};
pub use keymap::Keymap;
