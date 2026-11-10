//! The compose sheet (decision compose-dialogs): the parts every dialog that writes markdown shares, laid out to fit
//! what's typed instead of Python's fixed boxes.
//!
//! A sheet is a centered dialog frame holding, top to bottom, a one-row heading, an optional [`Quote`], the dialog's
//! own fields, a `Write  Preview` tabs row and the [`ComposeBody`]. The body grows with its wrapped text and the sheet
//! with it ([`ComposeLayout`]), up to the screen less the footer and a margin row; only then does the body scroll, and
//! only then is its scrollbar drawn. [`SheetPlacement`] keeps the frame's top still while it grows. There are no
//! button rows: the footer carries [`COMPOSE_BINDINGS`].
//!
//! Each dialog keeps its own state, request and output, and describes its parts to the layout. Its action enum
//! carries a [`ComposeAction`] for the keys the sheet shares.

mod body;
mod layout;
mod quote;

use crate::keymap::BindingId;
use crate::modal::dialog_bindings;

pub use body::{BodyMode, BodyMouse, ComposeBody};
pub use layout::{ComposeLayout, ComposeParts, SheetGeometry, SheetPlacement, sheet_geometry};
pub use quote::Quote;

/// A compose dialog's screen bindings: submit (`ctrl+s` and `shift+enter`), Preview, Expand, then `modal.close`, which
/// leaves a field or, from the sheet, closes the dialog.
pub(crate) const COMPOSE_BINDINGS: &[BindingId] = dialog_bindings![
    ComposeSubmit,
    ModalSubmit,
    ComposeTogglePreview,
    ComposeExpand,
    ModalClose
];

/// What the sheet's shared keys do. Each compose dialog's action carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeAction {
    /// `compose.submit` or `modal.submit`: send what's written. Ignored while a request is in flight.
    Submit,
    /// `modal.close` with the sheet focused: close with nothing, cancelling a request in flight.
    Close,
    /// `modal.close` in a field or the body: the sheet itself takes focus, and nothing is lost.
    Leave,
    /// `compose.toggle_preview` or a click on a tab: swap the body between the editor and its rendering.
    TogglePreview,
    /// `compose.expand`: the sheet fills the screen above the footer, or goes back to its content's size.
    ToggleExpand,
}
