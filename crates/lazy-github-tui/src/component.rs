//! The `Component` trait every pane, tab and modal implements, and the contexts passed into it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::config::Config;
use lazy_github_core::models::{RepoName, Repository};
use ratatui::Frame;
use ratatui::layout::Rect;
use tokio::time::Instant;

use crate::action::Action;
use crate::clock::Clock;
use crate::effect::{Effect, GitRequest, GithubRequest, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::modal::{Dialog, FieldKind, ModalEntry, ModalKind};
use crate::options::AppOptions;
use crate::screens::main::SectionId;
use crate::theme::Theme;
use crate::toast::Toast;

/// A piece of UI with its own state, actions, bindings and rendering.
///
/// `Action` is the component's own enum; its parent wraps it in the global [`Action`]. Components never hold
/// references to each other or to globals: everything shared comes in through [`Cx`] and [`RenderCx`].
pub trait Component {
    /// The component's own actions.
    type Action;

    /// Binding ids this component declares, in Textual `BINDINGS` order. Drives dispatch, the footer and the help
    /// panel.
    fn bindings(&self) -> &'static [BindingId] {
        &[]
    }

    /// A key matched one of [`Self::bindings`]. `None` means not applicable right now, so dispatch falls through to
    /// the next chain link (Textual's `check_action` returning `False`). A feature not built yet returns
    /// `Some(Action::Unimplemented(id))`.
    fn on_binding(&mut self, _id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        None
    }

    /// Whether a binding is shown, shown disabled, or hidden right now, for the footer and help panel.
    fn binding_state(&self, _id: BindingId, _cx: &Cx<'_>) -> BindingState {
        BindingState::Enabled
    }

    /// The focused component sees a key before the binding chain only when this returns true. Text inputs claim
    /// printable keys, so `q` types a q instead of quitting, and the editing keys `keymap` binds for them.
    fn consumes_key(&self, _key: &KeyChord, _keymap: &Keymap) -> bool {
        false
    }

    /// A key that no binding took, or that [`Self::consumes_key`] claimed.
    fn handle_key(&mut self, _key: KeyEvent, _cx: &Cx<'_>) -> Option<Action> {
        None
    }

    /// A mouse event inside `area`, the component's last rendered region.
    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        None
    }

    /// Applies one of the component's own actions.
    fn update(&mut self, action: Self::Action, cx: &mut Cx<'_>);

    /// Draws the component into `area`.
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>);

    /// For maximize (#75): rows wanted at this width, or `None` to fill the screen.
    fn natural_height(&self, _width: u16) -> Option<u16> {
        None
    }
}

/// How the footer and help panel show a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindingState {
    /// Shown and active.
    #[default]
    Enabled,
    /// Shown greyed out.
    Disabled,
    /// Not shown.
    Hidden,
}

/// One link in the binding chain: a component and the bindings it declares. Chains are innermost first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainLink {
    /// Who handles a binding matched at this link.
    pub owner: Owner,
    /// The bindings declared there.
    pub bindings: &'static [BindingId],
    /// A focused text input sits further in, so the keys it types are no longer this link's
    /// ([`crate::keymap::KeyChord::typed_by_input`]). [`crate::keymap::Keymap::link_specs`] drops them.
    pub behind_input: bool,
}

impl ChainLink {
    /// A link for `owner`'s `bindings`, with no input in front of it.
    pub fn new(owner: Owner, bindings: &'static [BindingId]) -> Self {
        Self {
            owner,
            bindings,
            behind_input: false,
        }
    }

    /// Marks every link after the first that types text as [`Self::behind_input`], as Textual filters each outer
    /// namespace by the ones before it.
    pub fn filter_behind_inputs(chain: &mut [ChainLink]) {
        let Some(input) = chain.iter().position(|link| link.owner.types_text()) else {
            return;
        };
        for link in &mut chain[input + 1..] {
            link.behind_input = true;
        }
    }
}

/// The components that can own bindings, as Python's widget tree nests them. Modals add their variants as they land.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    /// The app itself: quitting, the command palette, maximize and help.
    App,
    /// A whole screen.
    Screen(ScreenKind),
    /// The main screen's `MainViewPane`: the focus keys.
    MainView,
    /// The `SelectionsPane` around the four sections: new issue and new PR.
    SelectionsPane,
    /// A selection section's container, with the section's actions.
    Section(SectionId),
    /// A section's table: Textual's `DataTable` and `_VimLikeDataTable` bindings.
    Table(SectionId),
    /// The `SearchableDataTable` around a section's table: `/` search.
    SearchableTable(SectionId),
    /// A section's `/` search box: Textual's `Input`.
    SearchInput(SectionId),
    /// The details tab strip.
    DetailsTabs,
    /// The content of the active details tab.
    DetailsContent,
    /// A PR's Overview tab (`PrOverviewTabPane`): merge, edit and react, in front of the content's scroll link.
    PrOverview,
    /// An issue's Overview tab (`IssueOverviewTabPane`): edit, in front of the content's scroll link.
    IssueOverview,
    /// A button inside the active details tab's content with focus (the run Overview's re-run buttons), in front of
    /// the content's scroll link.
    DetailsButton,
    /// A focused collapsible section's title inside the details content (Textual's `CollapsibleTitle`).
    CollapsibleTitle,
    /// A focused list inside the details content, such as a section's (Textual's `ListView`).
    ListView,
    /// A comment block with focus on it or inside it (`IssueCommentContainer`): reply.
    CommentBlock,
    /// A focused tree inside the details content (Textual's `Tree`), such as a run's jobs.
    Tree,
    /// A run's Jobs tab (`WorkflowRunJobsTabPane`): view job logs, in front of the content's scroll link.
    RunJobs,
    /// The diff viewer (`SplitDiffViewer`, #236): next and previous hunk, and the file filter key, while focus is
    /// anywhere in the PR Diff tab.
    DiffViewer,
    /// A focused hunk pane in the diff viewer (`UnifiedDiffPane`): the line cursor and add comment.
    HunkPane,
    /// The diff viewer's focused file filter (`FileFilterInput`, an `Input`).
    DiffFilter,
    /// The command log.
    CommandLog,
    /// A field of an open dialog, which [`Dialog::field_chain`](crate::modal::Dialog::field_chain) puts in front of
    /// the dialog's screen link while it has focus.
    DialogField(ModalKind, FieldKind),
}

impl Owner {
    /// The heading the help panel draws over this link's keys: Textual's `BINDING_GROUP_TITLE`. Only Textual's
    /// `Input` and `CollapsibleTitle` have one.
    pub fn group_title(self) -> Option<&'static str> {
        match self {
            Owner::SearchInput(_) | Owner::DiffFilter | Owner::DialogField(_, FieldKind::TextInput) => Some("Input"),
            Owner::CollapsibleTitle => Some("Collapsible"),
            Owner::App
            | Owner::Screen(_)
            | Owner::MainView
            | Owner::SelectionsPane
            | Owner::Section(_)
            | Owner::Table(_)
            | Owner::SearchableTable(_)
            | Owner::DetailsTabs
            | Owner::DetailsContent
            | Owner::PrOverview
            | Owner::IssueOverview
            | Owner::DetailsButton
            | Owner::ListView
            | Owner::CommentBlock
            | Owner::Tree
            | Owner::RunJobs
            | Owner::DiffViewer
            | Owner::HunkPane
            | Owner::CommandLog
            | Owner::DialogField(_, FieldKind::Button | FieldKind::Switch | FieldKind::Scroll | FieldKind::Table) => {
                None
            }
        }
    }

    /// Whether the owner is a text input, which takes the keys it types from every link further out.
    pub fn types_text(self) -> bool {
        matches!(
            self,
            Owner::SearchInput(_) | Owner::DiffFilter | Owner::DialogField(_, FieldKind::TextInput)
        )
    }

    /// The label the footer and help panel give binding `id` at this link, when it isn't the binding's own
    /// description. The hunk pane labels the line cursor keys as Python's pane labels its own `j` / `k` (Py bug B9),
    /// since `diff.cursor_down` and `diff.cursor_up` have no description.
    pub fn binding_label(self, id: BindingId) -> Option<&'static str> {
        match (self, id) {
            (Owner::HunkPane, BindingId::DiffCursorDown) => Some("Line down"),
            (Owner::HunkPane, BindingId::DiffCursorUp) => Some("Line up"),
            _ => None,
        }
    }
}

/// The screens: the base screens, and the dialogs on the modal stack above them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenKind {
    /// The "GitHub Authentication" screen.
    Auth,
    /// The main screen.
    Main,
    /// A dialog on the modal stack.
    Modal(ModalKind),
}

impl ScreenKind {
    /// Whether the binding chain stops at this screen (Textual's `ModalScreen`).
    pub fn is_modal(self) -> bool {
        matches!(self, ScreenKind::Auth | ScreenKind::Modal(_))
    }
}

/// What LazyGithub knows of the repo of the directory it was launched in. Python's
/// `LazyGithubContext.current_directory_repo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LocalRepo {
    /// Git hasn't answered yet.
    #[default]
    Unknown,
    /// Git answered: the `origin` repo, or `None` when there's no current-directory repo, whatever the reason.
    Known(Option<RepoName>),
}

/// The state every component may read, owned by `App`.
pub(crate) struct Shared {
    pub(crate) config: Config,
    pub(crate) keymap: Keymap,
    pub(crate) theme: Theme,
    pub(crate) options: AppOptions,
    pub(crate) clock: Arc<dyn Clock>,
    /// The selected repo. Only `App` writes it, when it applies `Action::SelectRepo`.
    pub(crate) current_repo: Option<Repository>,
    /// The current-directory repo, once git has answered.
    pub(crate) local_repo: LocalRepo,
    next_request: u64,
    config_dirty: bool,
}

impl Shared {
    pub(crate) fn new(
        config: Config,
        keymap: Keymap,
        theme: Theme,
        options: AppOptions,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            keymap,
            theme,
            options,
            clock,
            current_repo: None,
            local_repo: LocalRepo::Unknown,
            next_request: 1,
            config_dirty: false,
        }
    }

    /// Whether the config changed since the last call.
    pub(crate) fn take_config_dirty(&mut self) -> bool {
        std::mem::take(&mut self.config_dirty)
    }
}

/// Follow-up actions and effects collected while processing one action.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    pub(crate) actions: VecDeque<Action>,
    pub(crate) effects: Vec<Effect>,
}

/// The context passed into [`Component::update`] and the other event handlers.
pub struct Cx<'a> {
    shared: &'a mut Shared,
    out: &'a mut Outbox,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(shared: &'a mut Shared, out: &'a mut Outbox) -> Self {
        Self { shared, out }
    }

    /// The config.
    pub fn config(&self) -> &Config {
        &self.shared.config
    }

    /// The config, for changing it. `App` saves it once the current action is processed.
    pub fn config_mut(&mut self) -> &mut Config {
        self.shared.config_dirty = true;
        &mut self.shared.config
    }

    /// The keymap.
    pub fn keymap(&self) -> &Keymap {
        &self.shared.keymap
    }

    /// The theme.
    pub fn theme(&self) -> &Theme {
        &self.shared.theme
    }

    /// The selected repo, if any: the one the PR, issue and workflow sections show. Python's
    /// `LazyGithubContext.current_repo`.
    pub fn current_repo(&self) -> Option<&Repository> {
        self.shared.current_repo.as_ref()
    }

    /// The repo of the directory LazyGithub was launched in, as far as git has said.
    pub fn local_repo(&self) -> &LocalRepo {
        &self.shared.local_repo
    }

    /// Monotonic time from the app's clock.
    pub fn now(&self) -> Instant {
        self.shared.clock.now()
    }

    /// Local wall-clock time from the app's clock.
    pub fn local_now(&self) -> jiff::Zoned {
        self.shared.clock.local_now()
    }

    /// Starts `request` in the background and returns its id. The response comes back as [`Action::Github`] with
    /// that id; store it to tell the response apart from a stale one.
    pub fn request(&mut self, request: GithubRequest) -> RequestId {
        let id = self.next_request_id();
        self.out.effects.push(Effect::Github { id, request });
        id
    }

    /// Starts `request` in the user's working directory and returns its id. The response comes back as
    /// [`Action::Git`] with that id. Ids come from the same counter as [`Self::request`]'s, so the two never collide.
    pub fn git(&mut self, request: GitRequest) -> RequestId {
        let id = self.next_request_id();
        self.out.effects.push(Effect::Git { id, request });
        id
    }

    /// Reads the cache file at `path` in the background and returns the read's id. The text comes back as
    /// [`Action::CacheRead`] with that id. Ids come from the same counter as [`Self::request`]'s.
    pub fn read_cache(&mut self, path: PathBuf) -> RequestId {
        let id = self.next_request_id();
        self.out.effects.push(Effect::ReadCache { id, path });
        id
    }

    fn next_request_id(&mut self) -> RequestId {
        let id = RequestId::new(self.shared.next_request);
        self.shared.next_request += 1;
        id
    }

    /// Queues a follow-up action, processed before `App::update` returns.
    pub fn dispatch(&mut self, action: Action) {
        self.out.actions.push_back(action);
    }

    /// Emits an effect.
    pub fn effect(&mut self, effect: Effect) {
        self.out.effects.push(effect);
    }

    /// Shows a toast.
    pub fn notify(&mut self, toast: Toast) {
        self.dispatch(Action::Notify(toast));
    }

    /// Opens `dialog` on top of everything showing, Textual's `push_screen_wait`. Once the dialog has an output
    /// ([`Dialog::take_output`]) it closes, and `reply` turns the output into the opener's own action, which is then
    /// dispatched.
    pub fn open_dialog<D: Dialog>(&mut self, dialog: D, reply: impl FnOnce(D::Output) -> Action + Send + 'static) {
        self.dispatch(Action::OpenDialog(ModalEntry::new(dialog, reply)));
    }
}

/// The context passed into [`Component::render`].
pub struct RenderCx<'a> {
    /// The theme.
    pub theme: &'a Theme,
    /// The keymap, for key hints in titles.
    pub keymap: &'a Keymap,
    /// The time this frame is drawn at, read from the clock once per frame.
    pub now: Instant,
    /// Whether this component has focus.
    pub focused: bool,
    /// Whether this component or one inside it has focus (Textual's `:focus-within`).
    pub focus_within: bool,
}
