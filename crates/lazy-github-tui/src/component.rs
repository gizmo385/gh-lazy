//! The `Component` trait every pane, tab and modal implements, and the contexts passed into it.

use std::collections::VecDeque;
use std::sync::Arc;

use crossterm::event::{KeyEvent, MouseEvent};
use lazy_github_core::config::Config;
use ratatui::Frame;
use ratatui::layout::Rect;
use tokio::time::Instant;

use crate::action::Action;
use crate::clock::Clock;
use crate::effect::{Effect, GithubRequest, RequestId};
use crate::keymap::{BindingId, KeyChord, Keymap};
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
    /// printable keys, so `q` types a q instead of quitting.
    fn consumes_key(&self, _key: &KeyChord) -> bool {
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
    /// The details tab strip.
    DetailsTabs,
    /// The content of the active details tab.
    DetailsContent,
    /// The command log.
    CommandLog,
}

impl Owner {
    /// The heading the help panel draws over this link's keys: Textual's `BINDING_GROUP_TITLE`. No M1 owner has
    /// one. Textual's `Input` does ("Input"), so the table search box's owner returns it once it lands.
    pub fn group_title(self) -> Option<&'static str> {
        match self {
            Owner::App
            | Owner::Screen(_)
            | Owner::MainView
            | Owner::SelectionsPane
            | Owner::Section(_)
            | Owner::Table(_)
            | Owner::SearchableTable(_)
            | Owner::DetailsTabs
            | Owner::DetailsContent
            | Owner::CommandLog => None,
        }
    }
}

/// The base screens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenKind {
    /// The "GitHub Authentication" screen.
    Auth,
    /// The main screen.
    Main,
}

impl ScreenKind {
    /// Whether the binding chain stops at this screen (Textual's `ModalScreen`).
    pub fn is_modal(self) -> bool {
        matches!(self, ScreenKind::Auth)
    }
}

/// The state every component may read, owned by `App`.
pub(crate) struct Shared {
    pub(crate) config: Config,
    pub(crate) keymap: Keymap,
    pub(crate) theme: Theme,
    pub(crate) clock: Arc<dyn Clock>,
    next_request: u64,
    config_dirty: bool,
}

impl Shared {
    pub(crate) fn new(config: Config, keymap: Keymap, theme: Theme, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            keymap,
            theme,
            clock,
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
        let id = RequestId::new(self.shared.next_request);
        self.shared.next_request += 1;
        self.out.effects.push(Effect::Github { id, request });
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
