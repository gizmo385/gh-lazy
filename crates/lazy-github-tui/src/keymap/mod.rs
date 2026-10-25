//! Binding ids, normalized keys and the keymap that matches one to the other.
//!
//! This is the minimal slice the runtime (#105) needs: the app-level ids and their default keys. #73 replaces it
//! with the full registry from `lazy_github/lib/bindings.py`, the Textual key grammar and `bindings.overrides`,
//! behind the same `BindingId`, `KeyChord` and `Keymap::match_in` seams.

mod ids;
mod key;
#[allow(clippy::module_inception)] // The file layout #73 specifies: ids.rs, key.rs, keymap.rs.
mod keymap;

pub use ids::BindingId;
pub use key::KeyChord;
pub use keymap::Keymap;
