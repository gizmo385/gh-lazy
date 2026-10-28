//! Modal dialogs: the screen stack above the base screen, the frame every dialog is drawn in, and the [`Dialog`]
//! trait each modal implements. Textual's `ModalScreen`, `push_screen_wait` and `dismiss`.
//!
//! `App` keeps the open dialogs bottom to top. While one is open, the top one has the keyboard and the mouse: its
//! binding chain is the focused field's links, then its screen link, and only priority bindings from the app link
//! (`ctrl+q`). Drawing dims everything below each dialog, then draws its frame, its interior and its footer.

mod add_comment;
mod add_reaction;
mod confirm;
mod edit_issue;
mod focus;
pub mod form;
mod frame;
mod issue_form;
mod lookup;
mod new_comment;
mod new_issue;
mod pull_request;
mod stack;
mod trigger_workflow;
mod user_profile;

use ratatui::layout::Rect;

use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, Owner};
use crate::components::button::BUTTON_BINDINGS;
use crate::components::info::INFO_BINDINGS;
use crate::components::select::{SELECT_BINDINGS, SELECT_OVERLAY_BINDINGS};
use crate::components::selection_list::SELECTION_LIST_BINDINGS;
use crate::components::switch::SWITCH_BINDINGS;
use crate::components::table::TABLE_BINDINGS;
use crate::components::text_editor::TEXT_EDITOR_BINDINGS;
use crate::components::text_input::INPUT_BINDINGS;
use crate::effect::{GitResponse, GithubResponse, RequestId};
use crate::keymap::BindingId;
use crate::theme::{Palette, ThemeColor};

pub use add_comment::{AddCommentAction, AddCommentDialog};
pub use add_reaction::{AddReactionAction, AddReactionDialog};
pub use confirm::{ConfirmAction, ConfirmDialog};
pub use edit_issue::{EditIssueAction, EditIssueDialog};
pub use focus::{FocusMove, FocusRing};
pub use frame::{DIM_FACTOR, DialogFrame, default_border, dim, frame_rect, frame_rect_with_margin};
pub use lookup::{LookupAction, LookupDialog, LookupKind, LookupTarget, RepoLookup, number};
pub use new_comment::{NewCommentAction, NewCommentDialog, PostedComment, ReplyTarget};
pub use new_issue::{NewIssueAction, NewIssueDialog};
pub use pull_request::{
    CreatePrefill, PullRequestAction, PullRequestDialog, PullRequestForm, ReviewersField, ReviewersFocus,
    ReviewersOutcome, removing_toast,
};
pub use stack::ModalEntry;
pub use trigger_workflow::{TriggerWorkflowAction, TriggerWorkflowDialog};
pub use user_profile::{UserProfileAction, UserProfileDialog};

/// Textual's `Screen.BINDINGS`, which every modal screen has before the dialog's own: `tab`, `shift+tab` and the
/// `ctrl+c` copy. A dialog's [`Component::bindings`] starts with these; [`dialog_bindings!`] writes them.
pub const MODAL_SCREEN_BINDINGS: &[BindingId] = &[
    BindingId::TextualFocusNext,
    BindingId::TextualFocusPrevious,
    BindingId::TextualCopyText,
];

/// A dialog's screen bindings: [`MODAL_SCREEN_BINDINGS`], then the dialog's own, in Textual's merged order.
///
/// ```ignore
/// fn bindings(&self) -> &'static [BindingId] {
///     dialog_bindings![ModalSubmit, ModalClose]
/// }
/// ```
macro_rules! dialog_bindings {
    ($($id:ident),* $(,)?) => {
        &[
            $crate::keymap::BindingId::TextualFocusNext,
            $crate::keymap::BindingId::TextualFocusPrevious,
            $crate::keymap::BindingId::TextualCopyText,
            $($crate::keymap::BindingId::$id,)*
        ]
    };
}
pub(crate) use dialog_bindings;

/// Which modal a dialog is: for [`crate::component::ScreenKind::Modal`] and for routing [`DialogAction`]s. Each
/// dialog adds its variant as it lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModalKind {
    /// [`ConfirmDialog`]: a yes/no question.
    Confirm,
    /// A [`LookupDialog`], by what it looks up.
    Lookup(LookupTarget),
    /// [`UserProfileDialog`]: a user's profile and recent repos.
    UserProfile,
    /// [`NewIssueDialog`]: a new issue's title and description.
    NewIssue,
    /// [`EditIssueDialog`]: an issue's title, state and description.
    EditIssue,
    /// [`NewCommentDialog`]: a new comment or a reply.
    NewComment,
    /// [`AddCommentDialog`]: a pending review comment on a diff line.
    AddComment,
    /// [`AddReactionDialog`]: reactions to add to a pull request.
    AddReaction,
    /// [`TriggerWorkflowDialog`]: a workflow to dispatch, and the branch.
    TriggerWorkflow,
    /// [`PullRequestDialog`]: Create PR and Edit PR.
    PullRequest,
    /// The stack's test dialog (`test_util::TestDialog`, which isn't built into the app), by its id. Nothing in the
    /// app opens one.
    Test(u8),
}

/// The kinds of focusable field a dialog has, for [`crate::component::Owner::DialogField`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    /// Textual's `Input`: it types text, so the keys it types are gone from the links behind it.
    TextInput,
    /// Textual's `TextArea` ([`crate::components::text_editor::TextEditor`]): a multi-line editor, which types text
    /// as the input does.
    TextEditor,
    /// Textual's `Button`: `enter` presses it.
    Button,
    /// Textual's `Switch`: `enter` or `space` toggles it.
    Switch,
    /// Textual's `ScrollableContainer`: the scroll keys scroll it.
    Scroll,
    /// Textual's `DataTable`, with the vim keys: `enter` selects the cursor's row.
    Table,
    /// Textual's `Select` with its list closed ([`crate::components::select::Select`]): `enter`, `space`, `up` or
    /// `down` opens it.
    Select,
    /// A select's open list (`SelectOverlay`): it has the focus while open, takes the printable keys to search, and
    /// `escape` dismisses it.
    SelectOverlay,
    /// Textual's `SelectionList` ([`crate::components::selection_list::SelectionList`]): the `OptionList` keys move
    /// its highlight, and `space` or `enter` toggles the highlighted option.
    SelectionList,
}

impl FieldKind {
    /// The chain link of a focused field of this kind in dialog `kind`, for [`Dialog::field_chain`].
    pub fn link(self, kind: ModalKind) -> ChainLink {
        let bindings = match self {
            FieldKind::TextInput => INPUT_BINDINGS,
            FieldKind::TextEditor => TEXT_EDITOR_BINDINGS,
            FieldKind::Button => BUTTON_BINDINGS,
            FieldKind::Switch => SWITCH_BINDINGS,
            FieldKind::Scroll => INFO_BINDINGS,
            FieldKind::Table => TABLE_BINDINGS,
            FieldKind::Select => SELECT_BINDINGS,
            FieldKind::SelectOverlay => SELECT_OVERLAY_BINDINGS,
            FieldKind::SelectionList => SELECTION_LIST_BINDINGS,
        };
        ChainLink::new(Owner::DialogField(kind, self), bindings)
    }
}

/// A dialog's own action, wrapped for [`Action::Dialog`]. Each dialog adds a variant holding its action enum.
#[derive(Debug)]
pub enum DialogAction {
    /// [`ConfirmDialog`]'s.
    Confirm(ConfirmAction),
    /// A [`LookupDialog`]'s, by what it looks up.
    Lookup(LookupTarget, LookupAction),
    /// [`UserProfileDialog`]'s.
    UserProfile(UserProfileAction),
    /// [`NewIssueDialog`]'s.
    NewIssue(NewIssueAction),
    /// [`EditIssueDialog`]'s.
    EditIssue(EditIssueAction),
    /// [`NewCommentDialog`]'s.
    NewComment(NewCommentAction),
    /// [`AddCommentDialog`]'s.
    AddComment(AddCommentAction),
    /// [`AddReactionDialog`]'s.
    AddReaction(AddReactionAction),
    /// [`TriggerWorkflowDialog`]'s.
    TriggerWorkflow(TriggerWorkflowAction),
    /// [`PullRequestDialog`]'s.
    PullRequest(PullRequestAction),
    /// The test dialog's actions, by the dialog's id.
    Test(u8, TestDialogAction),
}

impl DialogAction {
    /// The kind of dialog the action is for: `App` hands it to the topmost open dialog of that kind.
    pub fn kind(&self) -> ModalKind {
        match *self {
            DialogAction::Confirm(_) => ModalKind::Confirm,
            DialogAction::Lookup(target, _) => ModalKind::Lookup(target),
            DialogAction::UserProfile(_) => ModalKind::UserProfile,
            DialogAction::NewIssue(_) => ModalKind::NewIssue,
            DialogAction::EditIssue(_) => ModalKind::EditIssue,
            DialogAction::NewComment(_) => ModalKind::NewComment,
            DialogAction::AddComment(_) => ModalKind::AddComment,
            DialogAction::AddReaction(_) => ModalKind::AddReaction,
            DialogAction::TriggerWorkflow(_) => ModalKind::TriggerWorkflow,
            DialogAction::PullRequest(_) => ModalKind::PullRequest,
            DialogAction::Test(id, _) => ModalKind::Test(id),
        }
    }
}

/// The test dialog's actions (`test_util::TestDialog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestDialogAction {
    /// `modal.close` or the Cancel button: done, with no result.
    Close,
    /// `modal.submit` or the submit button: done, with the input's text.
    Submit,
    /// The reply from a dialog it opened, as a dialog lower in the stack gets one.
    Replied(Option<String>),
}

/// A modal dialog: a [`Component`] the stack draws in a frame, with a result for whoever opened it.
///
/// The component's [`Component::bindings`] are its modal screen's link and must start with
/// [`MODAL_SCREEN_BINDINGS`] ([`dialog_bindings!`]). Its `on_binding`, `binding_state`, `consumes_key`, `handle_key`,
/// `handle_mouse`, `update` and `render` work as for any component: `render` and `handle_mouse` get the frame's
/// interior, and `consumes_key` and `handle_key` see keys while the dialog is on top.
pub trait Dialog: Component + Send + 'static {
    /// What the opener gets back: Confirm's `bool`, Lookup's `Option<Repository>`.
    type Output: Send + 'static;

    /// Which modal this is, for [`crate::component::ScreenKind::Modal`] and for routing [`Action::Dialog`].
    fn kind(&self) -> ModalKind;

    /// Its own action out of a routed [`DialogAction`] of its [`Self::kind`]. `None` drops it.
    fn own_action(&self, action: DialogAction) -> Option<Self::Action>;

    /// `tab` or `shift+tab` (the modal screen's `TextualFocusNext` and `TextualFocusPrevious`): moves focus, usually
    /// with [`FocusRing::step`] on the dialog's ring. The dialog's `on_binding` never sees those two bindings.
    fn move_focus(&mut self, direction: FocusMove);

    /// The frame's outer size, border included, for a screen whose area above the footer row is `available`. A
    /// size larger than `available` is clipped to it.
    fn size(&self, available: Rect) -> (u16, u16);

    /// Where the frame goes on a screen of `area` (the footer row included): [`frame_rect`] of [`Self::size`] unless
    /// the dialog says otherwise ([`frame_rect_with_margin`] for one with a `margin`).
    fn frame(&self, area: Rect) -> Rect {
        let available = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };
        frame_rect(area, self.size(available))
    }

    /// The interior's background: `$surface-lighten-3` unless the dialog says otherwise.
    fn background(&self, palette: &Palette) -> ThemeColor {
        palette.surface_lighten_3
    }

    /// The frame's border color: `$background 80%` ([`default_border`]) unless the dialog says otherwise.
    fn border(&self, palette: &Palette) -> ThemeColor {
        default_border(palette)
    }

    /// Whether the bottom row shows the dialog's footer. False for Create/Edit PR.
    fn shows_footer(&self) -> bool {
        true
    }

    /// Set once the dialog is done (a button, `modal.close`, a finished lookup). The stack checks after every
    /// action, closes the dialog and dispatches the opener's reply.
    fn take_output(&mut self) -> Option<Self::Output>;

    /// The links of the focused field, innermost first, in front of the screen link: an
    /// [`crate::component::Owner::DialogField`] for each. Empty when the focused field binds nothing.
    fn field_chain(&self) -> Vec<ChainLink> {
        Vec::new()
    }

    /// A binding matched at one of [`Self::field_chain`]'s links. `None` lets the key fall through.
    fn on_field_binding(&mut self, _field: FieldKind, _id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        None
    }

    /// How the footer and help panel show a binding of one of [`Self::field_chain`]'s links.
    fn field_binding_state(&self, _field: FieldKind, _id: BindingId, _cx: &Cx<'_>) -> BindingState {
        BindingState::Enabled
    }

    /// Whether one of the dialog's fields has the mouse: a left press on a text input or editor started a drag that
    /// hasn't been released (Textual's `capture_mouse`). While it does, `App` hands the left drags and the release to
    /// [`Component::handle_mouse`] wherever they are, even outside the interior.
    fn captures_mouse(&self) -> bool {
        false
    }

    /// A bracketed paste while the dialog is on top. `cx` gives the time an editor's undo batches by.
    fn handle_paste(&mut self, _text: &str, _cx: &Cx<'_>) -> Option<Action> {
        None
    }

    /// Takes a response to one of the dialog's requests, and hands back any other.
    fn on_response(&mut self, _id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        Some(response)
    }

    /// Takes the answer to one of the dialog's git requests ([`Cx::git`]), and hands back any other.
    fn on_git_response(&mut self, _id: RequestId, response: GitResponse, _cx: &mut Cx<'_>) -> Option<GitResponse> {
        Some(response)
    }
}

#[cfg(test)]
mod tests;
