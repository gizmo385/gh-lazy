//! `App`: all UI state, the synchronous `update`, key dispatch and rendering.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::debug::DebugEnv;
use lazy_github_core::github::GhCliOptions;
use lazy_github_core::models::Repository;
use ratatui::Frame;
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear};
use tokio::time::Instant;

use crate::action::Action;
use crate::bug_report::BugReport;
use crate::clock::Clock;
use crate::component::{
    BindingState, ChainLink, Component, Cx, GhVersion, LocalRepo, Outbox, Owner, RenderCx, ScreenKind, Shared,
};
use crate::components::command_log::Wheel;
use crate::components::footer::{Footer, FooterHit, footer_items};
use crate::components::help_panel::{HelpPanel, HelpScroll, help_sections, help_width};
use crate::components::status_bar::{StatusBar, UnreadState};
use crate::components::too_small::{TooSmall, is_too_small};
use crate::effect::{Effect, GitRequest, GitResponse, GithubRequest, GithubResponse, RequestId, TimerId};
use crate::event::Event;
use crate::keymap::{ActiveBinding, BindingId, KeyChord, Keymap, active_bindings};
use crate::modal::{
    CommandPaletteDialog, CommandSource, DebugInfoDialog, DialogFrame, ModalEntry, ModalKind, PaletteCommand,
    PaletteRun, SettingsDialog, SettingsOutcome, dim,
};
use crate::notifications::NotificationPoll;
use crate::options::{AppOptions, TEST_PANIC_ENV, TestPanic};
use crate::rate_limit::RateLimitWatch;
use crate::screens::auth::{AuthAction, AuthReason, AuthScreen};
pub use crate::screens::main::Maximized;
use crate::screens::main::{MainScreen, Toggle};
use crate::screens::startup::{StartupDecision, StartupScreen, naive_now};
use crate::theme::Theme;
use crate::toast::{Severity, Toast, Toasts};

/// The bindings on the app link of every non-modal chain, in Textual order: its built-ins, then `LazyGithub.BINDINGS`.
pub const APP_BINDINGS: &[BindingId] = &[
    BindingId::TextualQuit,
    BindingId::TextualHelpQuit,
    BindingId::AppQuit,
    BindingId::AppCommandPalette,
    BindingId::AppMaximizeWidget,
    BindingId::AppOpenHelp,
];

/// The screen under any modals.
enum BaseScreen {
    /// Blank, until the startup auth check decides.
    Startup(StartupScreen),
    /// `gh` can't be used yet. Modal: only its own bindings and priority ones apply.
    Auth(AuthScreen),
    /// Boxed: it's much bigger than the other screens.
    Main(Box<MainScreen>),
}

/// Who has the mouse between a left press on a text input and the release (Textual's `capture_mouse`). The owner
/// knows which of its inputs it is: the one whose drag is in progress ([`TextInput::is_selecting`]).
///
/// [`TextInput::is_selecting`]: crate::components::text_input::TextInput::is_selecting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseCapture {
    /// An input on the base screen: a section's search box.
    Base,
    /// An input in the dialog at this depth of the stack.
    Dialog(usize),
}

/// The `GhVersion` request in flight, which a bug report waits on too while one runs.
struct GhVersionRequest {
    id: RequestId,
    /// Whether a Debug Info opens when gh answers.
    debug_info: bool,
}

/// The whole UI. One value owns all state; [`App::update`] is synchronous and does no I/O.
pub struct App {
    base: BaseScreen,
    /// The open dialogs above `base`, bottom to top. The top one has the keyboard and the mouse.
    modals: Vec<ModalEntry>,
    shared: Shared,
    toasts: Toasts,
    maximized: Option<Maximized>,
    /// The maximize toast shows once per session. Never saved.
    has_shown_maximize_toast: bool,
    /// Whether the keybindings panel is open. It's drawn only on the main screen, maximized or not. Never saved.
    help_open: bool,
    /// How far the open panel is scrolled.
    help_scroll: HelpScroll,
    /// The footer items drawn in the last frame, for clicks.
    footer_hits: Vec<FooterHit>,
    /// Where the mouse was at the last mouse event, for hover highlights. Each frame checks it against that frame's
    /// layout, as Textual re-checks hover after each layout change. Terminals don't report the pointer leaving the
    /// window, so it stays at its last cell.
    mouse: Option<Position>,
    /// Who has the mouse from a left press on a text input until the release.
    mouse_capture: Option<MouseCapture>,
    /// The `OriginRepo` request asked when the main screen was entered, until git answers it.
    local_repo_request: Option<RequestId>,
    /// The `CurrentBranch` request asked with it, until git answers it.
    current_branch_request: Option<RequestId>,
    /// The `GhVersion` request a Debug Info or a bug report is waiting on, until gh answers it.
    gh_version_request: Option<GhVersionRequest>,
    /// The palette's "Open Bug Report", while it runs.
    bug_report: BugReport,
    /// The unread count in the status box.
    notifications: NotificationPoll,
    /// Whether GitHub is rate limiting the app, which pauses the notifications poll.
    rate_limit: RateLimitWatch,
    /// Warnings from before the main screen, logged once it's up so the command log shows them.
    main_screen_warnings: Vec<String>,
    /// The terminal's size, from the last resize or frame; `None` until the first. Below the minimum, only the
    /// "Terminal too small" message is drawn and only quit keys work.
    size: Option<Size>,
    should_quit: bool,
}

impl App {
    /// An app on the blank startup screen, with the keymap built from `config`'s `bindings.overrides` and the
    /// process-wide `options` the binary read at startup. The clock is the only source of time for the app and its
    /// components. [`Self::init`] decides where it goes next.
    pub fn new(config: Config, theme: Theme, options: AppOptions, clock: impl Clock + 'static) -> Self {
        let keymap = Keymap::new(&config.bindings.overrides);
        Self {
            base: BaseScreen::Startup(StartupScreen::default()),
            modals: Vec::new(),
            shared: Shared::new(config, keymap, theme, options, Arc::new(clock)),
            toasts: Toasts::default(),
            maximized: None,
            has_shown_maximize_toast: false,
            help_open: false,
            help_scroll: HelpScroll::default(),
            footer_hits: Vec::new(),
            mouse: None,
            mouse_capture: None,
            local_repo_request: None,
            current_branch_request: None,
            gh_version_request: None,
            bug_report: BugReport::default(),
            notifications: NotificationPoll::default(),
            rate_limit: RateLimitWatch::default(),
            main_screen_warnings: Vec::new(),
            size: None,
            should_quit: false,
        }
    }

    /// Logs `message` as a warning when the main screen first shows. The command log drops lines from before it, as
    /// Python's does, so a startup problem the user should see (no log file) waits for it.
    pub fn warn_on_main_screen(&mut self, message: impl Into<String>) {
        self.main_screen_warnings.push(message.into());
    }

    /// Effects to run when the event loop starts: the startup auth check, unless a recent one is cached, in which
    /// case the main screen is shown at once.
    pub fn init(&mut self) -> Vec<Effect> {
        let mut out = Outbox::default();
        let BaseScreen::Startup(startup) = &mut self.base else {
            return Vec::new();
        };
        if startup.begin(&mut Cx::new(&mut self.shared, &mut out)) == StartupDecision::Fresh {
            out.actions.push_back(Action::EnterMain);
        }
        self.drain(&mut out);
        out.effects
    }

    /// Shows the main screen without the startup check, for tests of everything after it, and returns the effects
    /// entering it emits.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn skip_startup(&mut self) -> Vec<Effect> {
        self.update(Action::EnterMain)
    }

    /// Replaces the base screen with a new main screen, unmaximized and focused on repos, starts loading the repos
    /// and starts the notifications poll, as the Python widgets' `on_mount`s do (children first). The first time, it
    /// also asks git for the current-directory repo, which the repos section selects once both have arrived.
    fn enter_main(&mut self, out: &mut Outbox) {
        self.maximized = None;
        let mut main = Box::new(MainScreen::new(
            &self.shared.config.appearance,
            &self.shared.keymap,
            &self.shared.theme,
            self.shared.options,
        ));
        let mut cx = Cx::new(&mut self.shared, out);
        main.start(&mut cx);
        if *cx.local_repo() == LocalRepo::Unknown && self.local_repo_request.is_none() {
            self.local_repo_request = Some(cx.git(GitRequest::OriginRepo));
            // Read once, as Python's `current_directory_branch`, for Create PR's head branch.
            self.current_branch_request = Some(cx.git(GitRequest::CurrentBranch));
        }
        self.base = BaseScreen::Main(main);
        self.notifications.start(&mut Cx::new(&mut self.shared, out));
        for warning in self.main_screen_warnings.drain(..) {
            tracing::warn!("{warning}");
        }
    }

    /// Maps `event` to actions and applies them.
    pub fn handle_event(&mut self, event: Event) -> Vec<Effect> {
        if matches!(event, Event::Key(_)) {
            self.test_panic(TestPanic::Update);
        }
        // Toasts first: one past its timeout is gone before anything sees the event, and a click on a live one is
        // consumed. A deadline needs nothing else.
        self.toasts.expire(self.shared.clock.now());
        if let Event::Mouse(mouse) = &event {
            self.mouse = Some(Position::new(mouse.column, mouse.row));
            // Toasts sit over the screen, but not over a drag that a text input has the mouse for.
            if self.mouse_capture.is_none() && self.toasts.handle_mouse(mouse) {
                return Vec::new();
            }
        }
        if let Event::Resize(width, height) = event {
            self.size = Some(Size::new(width, height));
        }
        // Drained even without an action: a key or click may have finished a dialog by itself.
        let mut out = Outbox::default();
        let action = if self.too_small() {
            self.action_while_too_small(event)
        } else {
            self.action_for(event)
        };
        out.actions.extend(action);
        self.drain(&mut out);
        out.effects
    }

    /// Whether the terminal is below the minimum size, as of the last resize or frame.
    pub fn too_small(&self) -> bool {
        self.size.is_some_and(is_too_small)
    }

    /// The action for `event` while only "Terminal too small" is on screen: a key that quits the app right now
    /// quits, and every other key, click and paste is dropped, since nothing they'd act on can be seen. Everything
    /// else carries on as usual.
    ///
    /// `ctrl+q` always quits. `q` quits only where it would at full size: with no dialog open and no text field
    /// focused, so a `q` typed into an editor while the terminal is briefly small can't throw the text away.
    fn action_while_too_small(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Key(key) => {
                let chord = KeyChord::from_event(&key)?;
                let chain = self.binding_chain();
                let typing = chain.first().is_some_and(|link| link.owner.types_text());
                let app_link = chain.iter().any(|link| link.owner == Owner::App);
                let quits: &[BindingId] = if app_link && !typing {
                    &[BindingId::AppQuit, BindingId::TextualQuit]
                } else {
                    &[BindingId::TextualQuit]
                };
                self.shared.keymap.match_in(quits, &chord).map(|_| Action::Quit)
            }
            Event::Mouse(_) | Event::Paste(_) => None,
            event => self.action_for(event),
        }
    }

    /// Panics at `place` while a dialog is open, when the debug-only [`TestPanic`] trigger asks for it there.
    fn test_panic(&self, place: TestPanic) {
        assert!(
            self.shared.options.test_panic != Some(place) || self.modals.is_empty(),
            "{TEST_PANIC_ENV} asked for a panic in {place:?}"
        );
    }

    /// The action an event stands for, if any.
    pub(crate) fn action_for(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Key(key) => self.action_for_key(key),
            Event::Github { id, response } => Some(Action::Github(id, response)),
            Event::Git { id, response } => Some(Action::Git(id, response)),
            Event::CacheRead { id, result } => Some(Action::CacheRead(id, result)),
            Event::ConfigSaved(result) => Some(Action::ConfigSaved(result)),
            Event::Timer(id) => Some(Action::Timer(id)),
            Event::CommandLog(line) => Some(Action::CommandLog(line)),
            Event::Signal(signal) => {
                tracing::debug!("Received {signal:?}; quitting");
                Some(Action::Quit)
            }
            Event::Mouse(mouse) => self.action_for_mouse(mouse),
            // A bracketed paste goes to the top dialog, or else to the focused search box, and nowhere else.
            Event::Paste(text) => match (self.modals.last_mut(), &mut self.base) {
                (Some(top), _) => {
                    let mut scratch = Outbox::default();
                    top.handle_paste(&text, &Cx::new(&mut self.shared, &mut scratch))
                }
                (None, BaseScreen::Main(main)) => main.handle_paste(&text),
                (None, _) => None,
            },
            // Resizes need no state: the next draw uses the new size. A deadline only needs the redraw that follows
            // every event: animations take their frame from the clock.
            Event::Resize(..) | Event::FocusGained | Event::FocusLost | Event::Deadline => None,
        }
    }

    /// Key dispatch, in Textual's order.
    fn action_for_key(&mut self, key: KeyEvent) -> Option<Action> {
        // 1. Normalize; releases are ignored.
        let chord = KeyChord::from_event(&key)?;
        // 2. Escape minimizes a maximized widget before any binding (Textual's `ESCAPE_TO_MINIMIZE`). That belongs to
        //    the main screen, so not while a dialog has the keyboard.
        if self.modals.is_empty()
            && self.maximized.is_some()
            && chord == KeyChord::new(KeyCode::Esc, KeyModifiers::NONE)
        {
            return Some(Action::Minimize);
        }
        let full = self.full_binding_chain();
        // 3. Priority bindings, from the app link inward, past any modal screen. The clash rule sees the whole link,
        //    as in Textual.
        for link in full.iter().rev() {
            let matches = self.shared.keymap.link_priority_matches(link, &chord);
            if let Some(action) = self.route_first(link.owner, matches) {
                return Some(action);
            }
        }
        // 4. A focused component that consumes the key: a text input types printable keys and takes its editing
        //    keys (Textual's `Input._on_key`, then the input's own bindings, innermost in the chain).
        if let Some(top) = self.modals.last_mut() {
            if top.consumes_key(&chord, &self.shared.keymap) {
                let mut scratch = Outbox::default();
                let cx = Cx::new(&mut self.shared, &mut scratch);
                return Some(top.handle_key(key, &cx).unwrap_or(Action::Consumed));
            }
        } else if let BaseScreen::Main(main) = &mut self.base
            && let Some(action) = main.handle_focused_key(
                &chord,
                &self.shared.keymap,
                &self.shared.clipboard,
                self.shared.clock.now(),
            )
        {
            return Some(action);
        }
        let chain = truncate_at_modal(full);
        // 5. The chain, innermost first. Within a link, each binding the key fires is tried in turn; the first link
        //    whose component accepts one wins. Behind the search box, the keys it types are gone from every link.
        for link in &chain {
            let matches = self.shared.keymap.link_matches(link, &chord);
            if let Some(action) = self.route_first(link.owner, matches) {
                return Some(action);
            }
        }
        // 6. Nothing takes the key.
        None
    }

    /// A mouse event. While a text input has the mouse, its left drags and the release go to it wherever they are
    /// (Textual's `capture_mouse`); a left press that lands on one gives it the mouse. Anything else is routed by
    /// position ([`Self::route_mouse`]).
    fn action_for_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        let dragging = matches!(
            mouse.kind,
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        );
        if dragging && let Some(target) = self.mouse_capture {
            if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                self.mouse_capture = None;
            }
            if let Some(action) = self.captured_mouse(target, mouse) {
                return Some(action);
            }
            // The input that had the mouse is gone: it's an ordinary event.
            self.mouse_capture = None;
        }
        let press = mouse.kind == MouseEventKind::Down(MouseButton::Left);
        if press {
            // The release went missing (terminals report none outside the window), so the drag ends here.
            self.end_mouse_capture();
        }
        let action = self.route_mouse(mouse);
        if press {
            self.mouse_capture = match (self.modals.last(), &self.base) {
                (Some(top), _) => top
                    .captures_mouse()
                    .then(|| MouseCapture::Dialog(self.modals.len() - 1)),
                (None, BaseScreen::Main(main)) => main.captures_mouse().then_some(MouseCapture::Base),
                (None, _) => None,
            };
        }
        action
    }

    /// Hands a mouse event to the input that has the mouse. `None` when it's gone, or no longer has it.
    fn captured_mouse(&mut self, target: MouseCapture, mouse: MouseEvent) -> Option<Action> {
        match target {
            MouseCapture::Base => match &mut self.base {
                BaseScreen::Main(main) => main.captured_mouse(mouse),
                _ => None,
            },
            MouseCapture::Dialog(depth) => {
                let entry = self.modals.get_mut(depth).filter(|entry| entry.captures_mouse())?;
                let mut scratch = Outbox::default();
                entry.captured_mouse(mouse, &Cx::new(&mut self.shared, &mut scratch))
            }
        }
    }

    /// Ends a drag whose release never came: the input that had the mouse gets one.
    fn end_mouse_capture(&mut self) {
        if let Some(target) = self.mouse_capture.take() {
            let release = MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            };
            let _ = self.captured_mouse(target, release);
        }
    }

    /// A left click on a footer item runs its binding, or rings the bell if it's disabled, as Textual's `FooterKey`
    /// does; elsewhere it focuses the target under the pointer, and the wheel scrolls it. All as of the last frame.
    /// While a dialog is open the footer is the dialog's, and every other event goes to the dialog alone, which only
    /// sees the ones inside its frame.
    fn route_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let position = Position::new(mouse.column, mouse.row);
            if let Some(hit) = self.footer_hits.iter().find(|hit| hit.rect.contains(position)).copied() {
                if !hit.enabled {
                    return Some(Action::Bell);
                }
                return self.route_binding(hit.owner, hit.id);
            }
        }
        if let Some(top) = self.modals.last_mut() {
            let mut scratch = Outbox::default();
            return top.handle_mouse(mouse, &Cx::new(&mut self.shared, &mut scratch));
        }
        let BaseScreen::Main(main) = &mut self.base else {
            return None;
        };
        // The panel docks over the screen's right edge, and the wheel over it scrolls it.
        if self.help_open
            && self.help_scroll.contains(Position::new(mouse.column, mouse.row))
            && let Some(wheel) = Wheel::from_mouse(&mouse)
        {
            self.help_scroll.wheel(wheel);
            return Some(Action::Consumed);
        }
        main.handle_mouse(mouse)
    }

    /// Tries each binding a key fired in one link, in order, until one applies (Textual's `_check_bindings`).
    fn route_first(&mut self, owner: Owner, matches: Vec<BindingId>) -> Option<Action> {
        matches.into_iter().find_map(|id| self.route_binding(owner, id))
    }

    /// The binding chain for the current focus, innermost first, ending at a modal screen as Textual's
    /// `_modal_binding_chain` does. Otherwise the app link comes last.
    pub fn binding_chain(&self) -> Vec<ChainLink> {
        truncate_at_modal(self.full_binding_chain())
    }

    /// The whole chain, modal screens included, which priority bindings are looked up in: the top dialog's while
    /// one is open, else the base screen's. On the main screen it's the focus chain from
    /// [`MainScreen::binding_chain`].
    fn full_binding_chain(&self) -> Vec<ChainLink> {
        match self.modals.last() {
            Some(top) => with_app_link(top.chain()),
            None => self.base_full_chain(),
        }
    }

    /// [`Self::full_binding_chain`] for the base screen, as it stands under any dialogs.
    fn base_full_chain(&self) -> Vec<ChainLink> {
        with_app_link(match &self.base {
            BaseScreen::Startup(_) => Vec::new(),
            BaseScreen::Auth(auth) => vec![ChainLink::new(Owner::Screen(ScreenKind::Auth), auth.bindings())],
            BaseScreen::Main(main) => main.binding_chain(),
        })
    }

    /// How the footer and help panel (#76) show binding `id` of the chain link `owner`.
    pub fn binding_state(&mut self, owner: Owner, id: BindingId) -> BindingState {
        let mut scratch = Outbox::default();
        let cx = Cx::new(&mut self.shared, &mut scratch);
        if let Some(kind) = modal_owner(owner) {
            return self
                .modals
                .iter()
                .rev()
                .find(|entry| entry.kind() == kind)
                .map_or(BindingState::Enabled, |entry| entry.binding_state(owner, id, &cx));
        }
        match (&self.base, owner) {
            (_, Owner::App) => BindingState::Enabled,
            (BaseScreen::Main(main), owner) => main.link_binding_state(owner, id, &cx),
            (BaseScreen::Auth(auth), Owner::Screen(ScreenKind::Auth)) => auth.binding_state(id, &cx),
            _ => BindingState::Enabled,
        }
    }

    /// The keys active for the current focus, for the footer and help panel, read fresh each frame: the top
    /// dialog's while one is open.
    pub fn active_bindings(&mut self) -> Vec<ActiveBinding> {
        let chain = self.binding_chain();
        self.chain_bindings(&chain)
    }

    /// The active keys of `chain`, as [`Self::active_bindings`] reads them.
    fn chain_bindings(&mut self, chain: &[ChainLink]) -> Vec<ActiveBinding> {
        let mut states = Vec::new();
        for link in chain {
            for &id in link.bindings {
                states.push((link.owner, id, self.binding_state(link.owner, id)));
            }
        }
        let state = |owner: Owner, id: BindingId| {
            states
                .iter()
                .find(|&&(o, i, _)| o == owner && i == id)
                .map_or(BindingState::Enabled, |&(_, _, state)| state)
        };
        active_bindings(chain, &self.shared.keymap, state)
    }

    /// Asks the owner of a matched binding what it means right now.
    fn route_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        match owner {
            Owner::App => match id {
                BindingId::AppQuit | BindingId::TextualQuit => Some(Action::Quit),
                BindingId::TextualHelpQuit => Some(Action::QuitHint),
                BindingId::AppMaximizeWidget => Some(Action::Maximize { toast: true }),
                BindingId::AppOpenHelp => Some(Action::ToggleHelp),
                BindingId::AppCommandPalette => Some(Action::OpenCommandPalette),
                other => Some(Action::Unimplemented(other)),
            },
            // A text input or editor takes `copy` before the chain when it has a selection. Reaching its link means
            // it has none, so the key goes on to the next link (Textual's `SkipAction`).
            owner
                if owner.types_text() && matches!(id, BindingId::TextualInputCopy | BindingId::TextualTextAreaCopy) =>
            {
                None
            }
            owner => {
                let mut scratch = Outbox::default();
                let cx = Cx::new(&mut self.shared, &mut scratch);
                if let Some(kind) = modal_owner(owner) {
                    let entry = self.modals.iter_mut().rev().find(|entry| entry.kind() == kind)?;
                    return entry.on_binding(owner, id, &cx);
                }
                match (&mut self.base, owner) {
                    (BaseScreen::Main(main), owner) => main.on_link_binding(owner, id, &cx),
                    (BaseScreen::Auth(auth), Owner::Screen(ScreenKind::Auth)) => auth.on_binding(id, &cx),
                    _ => None,
                }
            }
        }
    }

    /// Applies `action` and every follow-up action it queues, then returns the effects to run.
    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let mut out = Outbox::default();
        out.actions.push_back(action);
        self.drain(&mut out);
        out.effects
    }

    /// Runs `f` with an update context, as a component's `update` would get, then processes what it queued.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn with_cx<R>(&mut self, f: impl FnOnce(&mut Cx<'_>) -> R) -> (R, Vec<Effect>) {
        let mut out = Outbox::default();
        let result = f(&mut Cx::new(&mut self.shared, &mut out));
        self.drain(&mut out);
        (result, out.effects)
    }

    fn drain(&mut self, out: &mut Outbox) {
        self.close_finished(out);
        while let Some(action) = out.actions.pop_front() {
            self.apply(action, out);
            self.close_finished(out);
        }
        self.restore_if_section_gone();
        // Toasts raised by this action time out from now.
        self.toasts.stamp(self.shared.clock.now());
        // One save per processed action, like Python's `Config.to_edit()`.
        if self.shared.take_config_dirty() {
            out.effects
                .push(Effect::SaveConfig(Box::new(self.shared.config.clone())));
        }
    }

    /// Closes every dialog that has its output, top first, and queues the opener's reply to each. This is where
    /// Python's `push_screen_wait` callers continue.
    fn close_finished(&mut self, out: &mut Outbox) {
        let mut index = self.modals.len();
        while index > 0 {
            index -= 1;
            if let Some(reply) = self.modals[index].take_reply() {
                let entry = self.modals.remove(index);
                tracing::debug!("Closed {:?}", entry.kind());
                out.actions.push_back(reply);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "a flat table with one arm per action, each handing off to its owner"
    )]
    fn apply(&mut self, action: Action, out: &mut Outbox) {
        match action {
            Action::Quit => {
                self.should_quit = true;
                out.effects.push(Effect::Quit);
            }
            Action::QuitHint => out.actions.push_back(Action::Notify(quit_hint())),
            Action::Notify(toast) => self.toasts.push(toast),
            Action::Bell => out.effects.push(Effect::Bell),
            Action::Copy(text) => {
                self.shared.clipboard.clone_from(&text);
                out.effects.push(Effect::SetClipboard(text));
            }
            Action::Focus(target) => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.try_focus(target);
                }
            }
            Action::FocusNext | Action::FocusPrevious => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.focus_step(matches!(action, Action::FocusNext), self.maximized);
                }
            }
            Action::Maximize { toast } => self.maximize(toast, out),
            Action::OpenCommandPalette => self.open_command_palette(out),
            Action::OpenThemePalette => self.open_theme_palette(out),
            Action::SetTheme(name) => {
                let theme = Theme::from_config(name, self.shared.theme.color_mode());
                self.set_theme(theme, out);
            }
            Action::OpenSettings => self.open_settings(out),
            Action::SettingsClosed(SettingsOutcome::Saved(config)) => self.apply_settings(*config, out),
            Action::ShowDebugInfo => self.show_debug_info(out),
            Action::OpenBugReport => self.open_bug_report(out),
            Action::Toggle(toggle) => self.toggle(toggle, out),
            Action::Minimize => self.minimize(),
            // Python's `action_open_help`: only this closes the panel, and it doesn't move focus.
            // Only the main screen has the panel: Python mounts it on the startup screen, which the main screen
            // then covers.
            Action::ToggleHelp if matches!(self.base, BaseScreen::Main(_)) => {
                self.help_open = !self.help_open;
                self.help_scroll = HelpScroll::default();
            }
            // `Consumed` was applied where it was produced.
            // Cancel applies nothing.
            Action::ToggleHelp | Action::Consumed | Action::SettingsClosed(SettingsOutcome::Cancelled) => {}
            Action::Main(action) => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.update(action, &mut Cx::new(&mut self.shared, out));
                }
            }
            Action::EnterMain => {
                if matches!(self.base, BaseScreen::Auth(_)) {
                    out.effects.push(Effect::CancelTimer(TimerId::AuthPoll));
                }
                self.enter_main(out);
            }
            Action::OpenDialog(entry) => {
                tracing::debug!("Opened {:?}", entry.kind());
                self.modals.push(entry);
            }
            Action::Dialog(action) => {
                let kind = action.kind();
                if let Some(entry) = self.modals.iter_mut().rev().find(|entry| entry.kind() == kind) {
                    entry.update(action, &mut Cx::new(&mut self.shared, out));
                } else {
                    tracing::debug!("Dropping {action:?}: no {kind:?} dialog is open");
                }
            }
            Action::RefreshNotifications => self.notifications.refresh(&mut Cx::new(&mut self.shared, out)),
            Action::SelectRepo(repo) => self.select_repo(&repo, out),
            Action::Auth(action) => {
                if let BaseScreen::Auth(auth) = &mut self.base {
                    auth.update(action, &mut Cx::new(&mut self.shared, out));
                } else {
                    tracing::debug!("Dropping {action:?}: the authentication screen is gone");
                }
            }
            Action::Github(id, response) => self.route_response(id, response, out),
            Action::Git(id, response) => self.route_git(id, response, out),
            Action::CacheRead(id, result) => self.route_cache_read(id, result, out),
            Action::ConfigSaved(Ok(())) => tracing::debug!("Saved the config"),
            Action::ConfigSaved(Err(message)) => {
                tracing::error!("Failed to save config: {message}");
                self.toasts.push(Toast::new(
                    Severity::Error,
                    format!("Failed to save settings: {message}"),
                ));
            }
            Action::Timer(TimerId::AuthPoll) if matches!(self.base, BaseScreen::Auth(_)) => {
                out.actions.push_back(Action::Auth(AuthAction::Poll));
            }
            // The poll waits out a rate limit; the user's own refreshes still run.
            Action::Timer(TimerId::Notifications)
                if self.rate_limit.is_limited(self.shared.clock.local_now().timestamp()) =>
            {
                tracing::debug!("Skipping the notifications poll: GitHub is rate limiting requests");
            }
            Action::Timer(TimerId::Notifications) => out.actions.push_back(Action::RefreshNotifications),
            Action::Timer(id) => {
                // The other timers belong to the open dialogs, top first.
                let taken = self
                    .modals
                    .iter_mut()
                    .rev()
                    .any(|entry| entry.on_timer(id, &mut Cx::new(&mut self.shared, out)));
                if !taken {
                    tracing::debug!("Ignoring timer {id:?}: nothing owns it");
                }
            }
            // Python attaches the pane's handler when the main screen mounts, so lines from before it (startup and
            // auth) never show.
            Action::CommandLog(line) => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.command_log_mut().push(&line);
                }
            }
            Action::Unimplemented(id) => tracing::debug!("{id:?} is not implemented yet"),
        }
    }

    /// Shows only the focused widget, with the one-time toast if `toast` says so. A no-op while something is maximized
    /// (`M` doesn't toggle back) or when the focused widget isn't maximizable, which leaves the toast for the next
    /// maximize.
    fn maximize(&mut self, toast: bool, out: &mut Outbox) {
        if self.maximized.is_some() {
            return;
        }
        let BaseScreen::Main(main) = &mut self.base else { return };
        let Some(maximized) = main.maximize_target() else {
            return;
        };
        main.begin_maximize(maximized);
        self.maximized = Some(maximized);
        if toast && !self.has_shown_maximize_toast {
            self.has_shown_maximize_toast = true;
            out.actions.push_back(Action::Notify(maximize_toast()));
        }
    }

    /// Opens the command palette with the commands that apply now. Only the main screen has it, and the app link
    /// that binds it is cut off under any dialog, the palette included.
    fn open_command_palette(&mut self, out: &mut Outbox) {
        if !self.modals.is_empty() {
            return;
        }
        let BaseScreen::Main(main) = &self.base else { return };
        let mut commands = self.app_commands();
        commands.extend(self.system_commands(main));
        Cx::new(&mut self.shared, out).open_dialog(CommandPaletteDialog::new(commands), |run| {
            run.map_or(Action::Consumed, PaletteRun::action)
        });
    }

    /// The "Theme" command's palette: every theme by name, as [`Self::open_command_palette`] opens the commands. The
    /// first palette has closed by the time its reply opens this one.
    fn open_theme_palette(&mut self, out: &mut Outbox) {
        if !self.modals.is_empty() || !matches!(self.base, BaseScreen::Main(_)) {
            return;
        }
        Cx::new(&mut self.shared, out).open_dialog(CommandPaletteDialog::themes(), |run| {
            run.map_or(Action::Consumed, PaletteRun::action)
        });
    }

    /// [`Action::OpenSettings`]: "Change Settings" opens the Settings modal over the main screen, built from the
    /// config as it is now (not the session's palette theme); the palette has closed by the time its reply opens it.
    fn open_settings(&mut self, out: &mut Outbox) {
        if !self.modals.is_empty() || !matches!(self.base, BaseScreen::Main(_)) {
            return;
        }
        let dialog = SettingsDialog::new(&self.shared.config);
        Cx::new(&mut self.shared, out).open_dialog(dialog, Action::SettingsClosed);
    }

    /// Settings' Save: `config` becomes the app's and is written once (by [`Self::drain`]), and what the app reads
    /// from it as it runs follows at once. Python's `on_settings_modal_dismissed` and `handle_settings_update`, toast
    /// included, which shows on every Save, changed or not.
    ///
    /// The theme replaces any the palette picked for the session, keeping the terminal's color mode. The keymap is
    /// rebuilt, so the footer, the help panel and every key follow; the section titles keep the keys they were built
    /// with, as in Python. Hiding the maximized section restores the layout (Py bug B88's answer). Everything else is
    /// read where it's next used, except the `gh` options, which are baked in at startup and so are rebuilt here when
    /// their TTLs or host changed.
    fn apply_settings(&mut self, config: Config, out: &mut Outbox) {
        let gh_options = GhCliOptions::from(&config);
        let reconfigure = gh_options != GhCliOptions::from(&self.shared.config);
        *Cx::new(&mut self.shared, out).config_mut() = config;
        // First, so the requests below already use the new options.
        if reconfigure {
            out.effects.push(Effect::ReconfigureGithub(gh_options));
        }
        out.actions
            .push_back(Action::Notify(Toast::new(Severity::Information, "Settings updated")));
        let config = &self.shared.config;
        let theme = Theme::from_config(&config.appearance.theme, self.shared.theme.color_mode());
        self.shared.keymap = Keymap::new(&config.bindings.overrides);
        self.set_theme(theme, out);
        if let BaseScreen::Main(main) = &mut self.base {
            main.apply_appearance(&mut Cx::new(&mut self.shared, out));
            if self
                .maximized
                .and_then(|maximized| maximized.target.section())
                .is_some_and(|id| !main.section(id).shown())
            {
                self.minimize();
            }
        }
        self.notifications.settings_saved(&mut Cx::new(&mut self.shared, out));
    }

    /// Switches the whole app to `theme`, which the caller has given the terminal's color mode. Only what's drawn
    /// changes: `Config` isn't touched, so nothing is saved. What cached colors from the old theme takes them from
    /// the new one.
    fn set_theme(&mut self, theme: Theme, out: &mut Outbox) {
        tracing::debug!("Switching to the {} theme", theme.name);
        self.shared.theme = theme;
        if let BaseScreen::Main(main) = &mut self.base {
            main.retheme(&Cx::new(&mut self.shared, out));
        }
    }

    /// The palette's "Debug Info": opens the Debug Info modal with the config as it is now. The first time in a
    /// session it asks gh for its version and opens once gh answers; a second "Debug Info" meanwhile doesn't ask
    /// again.
    fn show_debug_info(&mut self, out: &mut Outbox) {
        if let Some(env) = self.debug_env() {
            let dialog = DebugInfoDialog::new(&env, &self.shared.config);
            Cx::new(&mut self.shared, out).open_dialog(dialog, |()| Action::Consumed);
        } else {
            self.ask_gh_version(out).debug_info = true;
        }
    }

    /// The palette's "Open Bug Report": asks for LazyGithub's repo, and for gh's version the first time in a session
    /// (sharing a request a Debug Info is waiting on). New issue opens once both have answered.
    fn open_bug_report(&mut self, out: &mut Outbox) {
        self.bug_report.start(&mut Cx::new(&mut self.shared, out));
        if self.shared.gh_version == GhVersion::Unknown {
            self.ask_gh_version(out);
        }
    }

    /// Asks gh for its version, unless a request is in flight already.
    fn ask_gh_version(&mut self, out: &mut Outbox) -> &mut GhVersionRequest {
        self.gh_version_request.get_or_insert_with(|| GhVersionRequest {
            id: Cx::new(&mut self.shared, out).request(GithubRequest::GhVersion),
            debug_info: false,
        })
    }

    /// What the debug info shows about this build and machine, once gh has said its version.
    fn debug_env(&self) -> Option<DebugEnv> {
        let GhVersion::Known(version) = &self.shared.gh_version else {
            return None;
        };
        // The terminal was read at startup; nothing here reads the environment.
        Some(DebugEnv {
            terminal: self.shared.options.terminal.to_owned(),
            ..DebugEnv::detect(|_| None, version.clone())
        })
    }

    /// LazyGithub's own commands as they apply now, in `MainScreenCommandProvider.commands` order. They come before
    /// the system commands, which keeps them first among equal scores (Py bug B86's answer).
    fn app_commands(&self) -> Vec<PaletteCommand> {
        let command = |name, help, run| PaletteCommand {
            name,
            help: Some(help),
            source: CommandSource::App,
            run,
        };
        let toggle = |name, help, toggle| command(name, help, PaletteRun::Toggle(toggle));
        let mut commands = vec![
            toggle(
                "Toggle Command Log",
                "Toggle showing or hiding the command log",
                Toggle::CommandLog,
            ),
            toggle(
                "Toggle Workflows",
                "Toggle showing or hiding repo actions",
                Toggle::Workflows,
            ),
            toggle("Toggle Issues", "Toggle showing or hiding repo issues", Toggle::Issues),
            toggle(
                "Toggle Pull Requests",
                "Toggle showing or hiding repo pull requests",
                Toggle::PullRequests,
            ),
            command(
                "Change Settings",
                "Adjust LazyGithub settings",
                PaletteRun::ChangeSettings,
            ),
            command("Debug Info", "View Debug Information", PaletteRun::DebugInfo),
            command("Open Bug Report", "Open Bug Report", PaletteRun::OpenBugReport),
        ];
        if self.shared.config.notifications.enabled {
            commands.push(command(
                "Refresh notifications",
                "Refresh the unread notifications count",
                PaletteRun::RefreshNotifications,
            ));
        }
        commands
    }

    /// Textual's system commands as they apply on `main` now, in `App.get_system_commands` order. "Screenshot" is
    /// deferred (#291).
    fn system_commands(&self, main: &MainScreen) -> Vec<PaletteCommand> {
        let command = |name, help, run| PaletteCommand {
            name,
            help: Some(help),
            source: CommandSource::System,
            run,
        };
        let mut commands = vec![
            command("Theme", "Change the current theme", PaletteRun::ChooseTheme),
            command("Quit", "Quit the application as soon as possible", PaletteRun::Quit),
        ];
        commands.push(if self.help_open {
            command("Keys", "Hide the keys and widget help panel", PaletteRun::ToggleHelp)
        } else {
            command(
                "Keys",
                "Show help for the focused widget and a summary of available keys",
                PaletteRun::ToggleHelp,
            )
        });
        if self.maximized.is_some() {
            commands.push(command(
                "Minimize",
                "Minimize the widget and restore to normal size",
                PaletteRun::Minimize,
            ));
        } else if main.maximize_target().is_some() {
            commands.push(command("Maximize", "Maximize the focused widget", PaletteRun::Maximize));
        }
        commands
    }

    /// A palette toggle on the main screen. Hiding the maximized section or command log restores the layout too
    /// (Py bug B88's answer, where Python keeps drawing it maximized until `escape`); the screen has already moved
    /// focus out of it.
    fn toggle(&mut self, toggle: Toggle, out: &mut Outbox) {
        let BaseScreen::Main(main) = &mut self.base else { return };
        let shown = main.toggle(toggle, &mut Cx::new(&mut self.shared, out));
        if !shown && self.maximized.is_some_and(|maximized| toggle.covers(maximized.target)) {
            self.minimize();
        }
    }

    /// Back to the normal layout, focus unchanged.
    fn minimize(&mut self) {
        self.maximized = None;
        if let BaseScreen::Main(main) = &mut self.base {
            main.end_maximize();
        }
    }

    /// Py bug B78's recommendation, answered by Chris: when the maximized section is no longer in the details
    /// (another selection, the details reloaded), the layout is restored as `escape` would, where Python leaves a
    /// blank hatch until `escape`.
    fn restore_if_section_gone(&mut self) {
        if let (Some(maximized), BaseScreen::Main(main)) = (self.maximized, &self.base)
            && !main.shows_maximized(maximized)
        {
            tracing::debug!("The maximized section is gone; restoring the layout");
            self.minimize();
        }
    }

    /// What is maximized, if anything.
    pub fn maximized(&self) -> Option<Maximized> {
        self.maximized
    }

    /// Whether the keybindings panel is open.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// `Action::SelectRepo`: makes `repo` the current repo, shown in the status box, and has the main screen load
    /// it. Python's `handle_repo_selection`. Selecting the current repo again does it all again.
    fn select_repo(&mut self, repo: &Repository, out: &mut Outbox) {
        tracing::info!("Selected repo {}", repo.full_name);
        self.shared.current_repo = Some(repo.clone());
        if let BaseScreen::Main(main) = &mut self.base {
            main.on_repo_selected(repo, &mut Cx::new(&mut self.shared, out));
        }
    }

    /// The current repo, if one is selected.
    pub fn current_repo(&self) -> Option<&Repository> {
        self.shared.current_repo.as_ref()
    }

    /// The unread count the status box shows.
    pub fn unread(&self) -> UnreadState {
        self.notifications.unread()
    }

    /// Whether a notifications refresh is running.
    pub fn notifications_refreshing(&self) -> bool {
        self.notifications.is_refreshing()
    }

    /// Hands a git response to whatever is waiting for it: the current-directory repo and branch are kept for every
    /// component, and the main screen may select the repo. Any other answer goes to the open dialogs, top first.
    fn route_git(&mut self, id: RequestId, mut response: GitResponse, out: &mut Outbox) {
        if self.local_repo_request != Some(id) && self.current_branch_request != Some(id) {
            for entry in self.modals.iter_mut().rev() {
                let Some(unclaimed) = entry.on_git_response(id, response, &mut Cx::new(&mut self.shared, out)) else {
                    return;
                };
                response = unclaimed;
            }
        }
        match response {
            GitResponse::OriginRepo(repo) if self.local_repo_request == Some(id) => {
                self.local_repo_request = None;
                self.shared.local_repo = LocalRepo::Known(repo);
                if let BaseScreen::Main(main) = &mut self.base {
                    main.on_local_repo(&mut Cx::new(&mut self.shared, out));
                }
            }
            GitResponse::CurrentBranch(branch) if self.current_branch_request == Some(id) => {
                self.current_branch_request = None;
                self.shared.current_branch = branch;
            }
            _ => tracing::debug!("Dropping the response to request {id}: nothing is waiting for it"),
        }
    }

    /// Hands a cache read to whatever is waiting for it: the open dialogs, top first, then the base screen.
    fn route_cache_read(&mut self, id: RequestId, mut result: Result<Option<String>, String>, out: &mut Outbox) {
        for entry in self.modals.iter_mut().rev() {
            let Some(unclaimed) = entry.on_cache_read(id, result, &mut Cx::new(&mut self.shared, out)) else {
                return;
            };
            result = unclaimed;
        }
        let unclaimed = match &mut self.base {
            BaseScreen::Main(main) => main.on_cache_read(id, result, &mut Cx::new(&mut self.shared, out)),
            _ => Some(result),
        };
        if unclaimed.is_some() {
            tracing::debug!("Dropping the cache read {id}: nothing is waiting for it");
        }
    }

    /// Hands a response to the screen waiting for it.
    fn route_response(&mut self, id: RequestId, response: GithubResponse, out: &mut Outbox) {
        // Every response counts, even one nothing waits for any more.
        if let Some(error) = response.error() {
            self.rate_limit.observe(error, &mut Cx::new(&mut self.shared, out));
        }
        if let Some(request) = self.gh_version_request.take_if(|request| request.id == id) {
            if let GithubResponse::GhVersion(result) = response {
                let version = result
                    .inspect_err(|error| tracing::warn!("Couldn't read gh's version: {error}"))
                    .ok();
                self.shared.gh_version = GhVersion::Known(version);
                if request.debug_info {
                    self.show_debug_info(out);
                }
                if let Some(env) = self.debug_env() {
                    self.bug_report.gh_known(&env, &mut Cx::new(&mut self.shared, out));
                }
            }
            return;
        }
        if self.bug_report.waits_for(id) {
            let env = self.debug_env();
            self.bug_report
                .on_repo(response, env, &mut Cx::new(&mut self.shared, out));
            return;
        }
        // Screens and panes that make other requests claim their responses here by id.
        let Some(mut response) = self
            .notifications
            .on_response(id, response, &mut Cx::new(&mut self.shared, out))
        else {
            return;
        };
        // Open dialogs, top first, then the base screen.
        for entry in self.modals.iter_mut().rev() {
            let Some(unclaimed) = entry.on_response(id, response, &mut Cx::new(&mut self.shared, out)) else {
                return;
            };
            response = unclaimed;
        }
        if let BaseScreen::Main(main) = &mut self.base {
            let Some(unclaimed) = main.on_response(id, response, &mut Cx::new(&mut self.shared, out)) else {
                return;
            };
            response = unclaimed;
        }
        let GithubResponse::CheckAuth(result) = response else {
            tracing::debug!("Dropping the response to request {id}: nothing is waiting for it");
            return;
        };
        let claimed = match &mut self.base {
            BaseScreen::Startup(startup) => startup.take_pending(id),
            BaseScreen::Auth(_) => {
                out.actions.push_back(Action::Auth(AuthAction::Checked(id, result)));
                return;
            }
            BaseScreen::Main(_) => false,
        };
        if !claimed {
            tracing::debug!("Dropping the response to request {id}: nothing is waiting for it");
            return;
        }
        let mut cx = Cx::new(&mut self.shared, out);
        match result {
            Ok(()) => {
                let now = naive_now(&cx.local_now());
                let config = cx.config_mut();
                config.cache.auth_last_checked = Some(now);
                // Rust has no backend picker, whose save cleared this in Python.
                config.core.first_start = false;
                cx.dispatch(Action::EnterMain);
            }
            Err(error) => {
                tracing::debug!("Triggering auth with github");
                self.base = BaseScreen::Auth(AuthScreen::open(AuthReason::from_error(&error), &mut cx));
            }
        }
    }

    /// Draws the whole UI: the base screen, then the dialogs and the toasts.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        self.size = Some(area.as_size());
        if self.too_small() {
            // Nothing drawn this frame takes a click.
            self.footer_hits.clear();
            frame.render_widget(
                TooSmall {
                    theme: &self.shared.theme,
                },
                area,
            );
            return;
        }
        self.render_screens(frame);
        self.test_panic(TestPanic::Render);
        self.render_modals(frame);
        // `handle_event` already dropped the expired ones: the loop wakes for each deadline.
        self.toasts.render(frame, frame.area(), &self.shared.theme);
    }

    /// The base screen, with the footer and help panel it has under any dialogs.
    fn render_screens(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let base_chain = truncate_at_modal(self.base_full_chain());
        let active = self.chain_bindings(&base_chain);
        // The screen below a dialog gets no mouse events, so nothing on it is hovered.
        let mouse = if self.modals.is_empty() { self.mouse } else { None };
        let theme = &self.shared.theme;
        // Textual's `Screen`: `background: $background; color: $foreground`.
        let screen = theme.background();
        frame.render_widget(
            Block::new().style(
                Style::new()
                    .fg(theme.color(theme.palette.foreground, screen))
                    .bg(theme.paint(screen)),
            ),
            area,
        );
        let cx = RenderCx {
            theme,
            keymap: &self.shared.keymap,
            now: self.shared.clock.now(),
            focused: true,
            focus_within: true,
        };
        let footer_area = match &mut self.base {
            BaseScreen::Startup(_) => None,
            BaseScreen::Auth(auth) => {
                auth.render(frame, area, &cx);
                AuthScreen::footer_area(area)
            }
            BaseScreen::Main(main) => {
                // The panel docks on the right over the full height, footer row included, maximized or not; the
                // screen lays out in what's left.
                let mut area = area;
                if self.help_open {
                    let width = help_width(area.width);
                    area.width -= width;
                    let panel = Rect {
                        x: area.right(),
                        width,
                        ..area
                    };
                    let sections = help_sections(&active);
                    frame.render_stateful_widget(
                        HelpPanel {
                            sections: &sections,
                            theme,
                        },
                        panel,
                        &mut self.help_scroll,
                    );
                }
                // While maximized, the widget replaces the rest of the screen, footer included.
                if let Some(maximized) = self.maximized {
                    main.render_maximized(frame, area, maximized, &cx);
                    None
                } else {
                    main.render(frame, area, &cx);
                    let status = StatusBar {
                        repo: self.shared.current_repo.as_ref(),
                        unread: self.notifications.unread(),
                        theme,
                    };
                    frame.render_widget(status, main.status_area(area));
                    Some(main.footer_area(area))
                }
            }
        };
        self.footer_hits = match footer_area {
            Some(footer_area) => {
                let items = footer_items(&active);
                let mut footer = Footer {
                    items: &items,
                    hovered: None,
                    theme,
                };
                let hits = footer.hits(footer_area);
                // Hits are in item order, so the hit's index is the item's.
                footer.hovered = mouse.and_then(|mouse| hits.iter().position(|hit| hit.rect.contains(mouse)));
                frame.render_widget(footer, footer_area);
                hits
            }
            None => Vec::new(),
        };
    }

    /// Each dialog, bottom to top, over everything drawn before it, dimmed: its frame centered above the bottom row,
    /// its interior, and its footer on the bottom row; a frameless one draws on the whole screen. Only the top footer's
    /// items take clicks.
    fn render_modals(&mut self, frame: &mut Frame<'_>) {
        if self.modals.is_empty() {
            return;
        }
        let area = frame.area();
        let actives: Vec<Vec<ActiveBinding>> = (0..self.modals.len())
            .map(|index| {
                let chain = truncate_at_modal(with_app_link(self.modals[index].chain()));
                self.chain_bindings(&chain)
            })
            .collect();
        let theme = self.shared.theme;
        let cx = RenderCx {
            theme: &theme,
            keymap: &self.shared.keymap,
            now: self.shared.clock.now(),
            focused: true,
            focus_within: true,
        };
        let available = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };
        let footer_area = Rect {
            y: available.bottom(),
            height: area.height.min(1),
            ..area
        };
        let top = self.modals.len() - 1;
        let mut hits = Vec::new();
        for (index, (entry, active)) in self.modals.iter_mut().zip(&actives).enumerate() {
            dim(frame.buffer_mut(), area, &theme);
            if entry.frameless() {
                entry.interior = area;
                entry.render(frame, area, &cx);
                continue;
            }
            let rect = entry.frame(area);
            if rect.bottom() > available.bottom() {
                screen_scrollbar(frame.buffer_mut(), available, rect, &theme);
            }
            let background = entry.background(theme.palette);
            frame.render_widget(
                DialogFrame {
                    theme: &theme,
                    background,
                    border: entry.border(theme.palette),
                },
                rect,
            );
            entry.interior = DialogFrame::interior(rect);
            entry.render(frame, entry.interior, &cx);
            if entry.shows_footer() {
                let items = footer_items(active);
                let mut footer = Footer {
                    items: &items,
                    hovered: None,
                    theme: &theme,
                };
                let footer_hits = footer.hits(footer_area);
                if index == top {
                    footer.hovered = self
                        .mouse
                        .and_then(|mouse| footer_hits.iter().position(|hit| hit.rect.contains(mouse)));
                    hits = footer_hits;
                }
                // The footer paints its row's background but not its symbols, and the main footer is under it.
                frame.render_widget(Clear, footer_area);
                frame.render_widget(footer, footer_area);
            }
        }
        self.footer_hits = hits;
    }

    /// The open dialogs' kinds, bottom to top.
    pub fn modals(&self) -> Vec<ModalKind> {
        self.modals.iter().map(ModalEntry::kind).collect()
    }

    /// The top dialog, when it's a `D`.
    #[cfg(test)]
    pub(crate) fn top_dialog<D: crate::modal::Dialog>(&self) -> Option<&D> {
        self.modals.last()?.dialog()
    }

    /// Where the top dialog's interior was in the last frame.
    #[cfg(test)]
    pub(crate) fn top_dialog_interior(&self) -> Option<Rect> {
        Some(self.modals.last()?.interior)
    }

    /// The main screen, while it's the base screen.
    pub fn main_screen(&self) -> Option<&MainScreen> {
        match &self.base {
            BaseScreen::Main(main) => Some(main.as_ref()),
            _ => None,
        }
    }

    /// The main screen, for changing state that later milestones drive (runtime section toggles).
    pub fn main_screen_mut(&mut self) -> Option<&mut MainScreen> {
        match &mut self.base {
            BaseScreen::Main(main) => Some(main.as_mut()),
            _ => None,
        }
    }

    /// The base screen showing, or `None` for the blank startup screen.
    pub fn screen(&self) -> Option<ScreenKind> {
        match self.base {
            BaseScreen::Startup(_) => None,
            BaseScreen::Auth(_) => Some(ScreenKind::Auth),
            BaseScreen::Main(_) => Some(ScreenKind::Main),
        }
    }

    /// The authentication screen, while it's up.
    pub fn auth_screen(&self) -> Option<&AuthScreen> {
        match &self.base {
            BaseScreen::Auth(auth) => Some(auth),
            _ => None,
        }
    }

    /// Whether the event loop should stop after the current batch.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// The next time something on screen changes by itself (toast expiry, a loading animation frame), or `None` when
    /// nothing animates.
    pub fn next_deadline(&self) -> Option<Instant> {
        let main = match &self.base {
            BaseScreen::Main(main) => main.next_deadline(self.shared.clock.now()),
            _ => None,
        };
        let now = self.shared.clock.now();
        let dialogs = self.modals.iter().filter_map(|entry| entry.next_deadline(now));
        [self.toasts.next_deadline(), main]
            .into_iter()
            .flatten()
            .chain(dialogs)
            .min()
    }

    /// The in-app clipboard: the last text copied or cut in a text input, which `ctrl+v` pastes.
    pub fn clipboard(&self) -> &str {
        &self.shared.clipboard
    }

    /// The toasts up now.
    pub fn toasts(&self) -> &Toasts {
        &self.toasts
    }

    /// The config as the app currently holds it.
    pub fn config(&self) -> &Config {
        &self.shared.config
    }

    /// The theme everything is drawn with now: the config's at startup, or one picked from the theme palette since.
    pub fn theme(&self) -> &Theme {
        &self.shared.theme
    }
}

/// `chain` with the app link appended, and every link behind a text input marked so.
fn with_app_link(mut chain: Vec<ChainLink>) -> Vec<ChainLink> {
    chain.push(ChainLink::new(Owner::App, APP_BINDINGS));
    ChainLink::filter_behind_inputs(&mut chain);
    chain
}

/// The vertical scrollbar a modal screen shows in the two rightmost columns of `available` when its dialog's frame
/// (`rect`) runs past the bottom, as Textual's screen scrolls then. The screen isn't scrolled: the dialog keeps its
/// own focused field in view.
fn screen_scrollbar(buf: &mut ratatui::buffer::Buffer, available: Rect, rect: Rect, theme: &Theme) {
    use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState, VERTICAL_WIDTH};

    let palette = theme.palette;
    let bg = theme.background();
    let back = palette.scrollbar_background.over(bg);
    let colors = ScrollbarColors {
        bar: theme.paint(palette.scrollbar.over(back)),
        back: theme.paint(back),
        fg: theme.color(palette.foreground, bg),
        bg: theme.paint(bg),
    };
    let bar = Rect {
        x: available.right().saturating_sub(VERTICAL_WIDTH),
        width: VERTICAL_WIDTH.min(available.width),
        ..available
    };
    let state = ScrollbarState {
        // The frame and the margin row below it.
        content: usize::from(rect.bottom().saturating_sub(available.y)) + 1,
        window: usize::from(available.height),
        position: 0,
    };
    scrollbar::render(buf, bar, Axis::Vertical, state, colors);
}

/// `chain` up to and including its first modal screen, as Textual's `_modal_binding_chain` ends it.
fn truncate_at_modal(mut chain: Vec<ChainLink>) -> Vec<ChainLink> {
    let is_modal = |link: &ChainLink| matches!(link.owner, Owner::Screen(kind) if kind.is_modal());
    if let Some(end) = chain.iter().position(is_modal) {
        chain.truncate(end + 1);
    }
    chain
}

/// The dialog a chain link belongs to, if it's a dialog's.
fn modal_owner(owner: Owner) -> Option<ModalKind> {
    match owner {
        Owner::DialogField(kind, _) | Owner::DialogPane(kind, _) | Owner::Screen(ScreenKind::Modal(kind)) => Some(kind),
        _ => None,
    }
}

/// Python's `action_maximize` toast.
fn maximize_toast() -> Toast {
    let message = Line::from(vec![
        Span::raw("Current view maximized. Press "),
        Span::styled("escape", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" to return."),
    ]);
    Toast::new(Severity::Information, message).with_title("View Maximized")
}

/// Textual's `help_quit` toast.
fn quit_hint() -> Toast {
    let message = Line::from(vec![
        Span::raw("Press "),
        Span::styled("ctrl+q", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" to quit the app"),
    ]);
    Toast::new(Severity::Information, message).with_title("Do you want to quit?")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use lazy_github_core::github::GithubCall;
    use lazy_github_core::logging::LogLine;
    use lazy_github_core::models::User;
    use tracing::Level;

    use super::*;
    use crate::clock::FakeClock;
    use crate::effect::{GitRequest, GithubRequest, GithubResponse, TimerId};
    use crate::event::Signal;
    use crate::screens::main::FocusTarget;
    use crate::test_util::{LatestWins, LatestWinsAction, TestApp, key_event, logs};

    fn test_app() -> TestApp {
        TestApp::new(Config::default())
    }

    fn user(login: &str) -> User {
        User {
            login: login.into(),
            id: 1,
            name: None,
            html_url: format!("https://github.com/{login}"),
        }
    }

    #[test]
    fn q_quits() {
        let mut app = test_app();
        app.press(&["q"]);
        assert_eq!(app.effects(), [Effect::Quit]);
        assert!(app.app().should_quit());
    }

    #[test]
    fn ctrl_q_quits() {
        let mut app = test_app();
        app.press(&["ctrl+q"]);
        assert_eq!(app.effects(), [Effect::Quit]);
        assert!(app.app().should_quit());
    }

    #[test]
    fn the_keymap_comes_from_the_config_overrides() {
        let mut config = Config::default();
        config.bindings.overrides.insert("app.quit".into(), "x, ctrl+t".into());
        let mut app = TestApp::new(config.clone());
        app.press(&["q"]);
        assert_eq!(app.effects(), []);
        app.press(&["ctrl+t"]);
        assert_eq!(app.effects(), [Effect::Quit]);
        assert_eq!(app.app().config(), &config);
    }

    /// Whether the help panel shows at 160x48: its border runs down column 108. Only the top rows are checked, since
    /// toasts draw over the bottom right.
    fn help_shown(app: &mut TestApp) -> bool {
        app.screen(160, 48)
            .lines()
            .take(20)
            .all(|row| row.chars().nth(108) == Some('▏'))
    }

    #[test]
    fn question_mark_toggles_help() {
        let mut app = test_app();
        assert!(!help_shown(&mut app));
        app.press(&["?"]);
        assert!(app.app().help_open());
        assert!(help_shown(&mut app));
        // The main screen lays out in the 108 columns left: the footer stops before the panel.
        let screen = app.screen(160, 48);
        let footer = screen.lines().last().unwrap();
        assert_eq!(footer.find('▏').map(|byte| footer[..byte].chars().count()), Some(108));
        app.press(&["?"]);
        assert!(!app.app().help_open());
        assert!(!help_shown(&mut app));
        assert_eq!(app.effects(), []);
    }

    #[test]
    fn escape_leaves_help_open() {
        let mut app = test_app();
        app.press(&["?", "escape"]);
        assert!(app.app().help_open());
        assert!(help_shown(&mut app));
    }

    #[test]
    fn help_follows_focus() {
        let mut app = test_app();
        app.press(&["?"]);
        assert!(app.screen(160, 48).contains("O Lookup Repository"));
        app.press(&["2"]);
        let screen = app.screen(160, 48);
        assert!(screen.contains("O Lookup Pull Request Lookup a pull"), "{screen}");
        assert!(!screen.contains("O Lookup Repository"), "{screen}");
        // Focus didn't move when the panel opened.
        assert_eq!(
            app.app().main_screen().unwrap().focus(),
            FocusTarget::Table(crate::screens::main::SectionId::PullRequests)
        );
    }

    #[test]
    fn an_override_moves_the_toggle() {
        let mut config = Config::default();
        config.bindings.overrides.insert("app.open_help".into(), "f1".into());
        let mut app = TestApp::new(config);
        app.press(&["?"]);
        assert!(!app.app().help_open());
        app.press(&["f1"]);
        assert!(app.app().help_open());
        let screen = app.screen(160, 110);
        assert!(screen.contains("f1 Keybindings Toggle this window"), "{screen}");
        app.press(&["f1"]);
        assert!(!app.app().help_open());
    }

    #[test]
    fn help_stays_docked_while_maximized() {
        let mut app = test_app();
        app.press(&["?", "M"]);
        assert!(app.app().maximized().is_some());
        assert!(help_shown(&mut app));
        // `?` toggles the panel while maximized, and escape minimizes without touching it.
        app.press(&["?"]);
        assert!(!help_shown(&mut app));
        app.press(&["?", "escape"]);
        assert!(app.app().maximized().is_none());
        assert!(help_shown(&mut app));
    }

    #[test]
    fn question_mark_during_startup_opens_nothing() {
        let mut app = TestApp::on_startup(Config::default(), lazy_github_core::github::FakeGithub::new());
        assert_eq!(app.app().screen(), None);
        app.press(&["?"]);
        futures_util::FutureExt::now_or_never(app.settle()).unwrap();
        assert_eq!(app.app().screen(), Some(ScreenKind::Main));
        assert!(!app.app().help_open());
    }

    #[test]
    fn clicks_on_the_panel_do_nothing() {
        let mut app = test_app();
        app.press(&["?", "2"]);
        app.screen(160, 48);
        app.click(140, 47);
        app.click(140, 5);
        assert_eq!(
            app.app().main_screen().unwrap().focus(),
            FocusTarget::Table(crate::screens::main::SectionId::PullRequests)
        );
        assert!(!app.app().should_quit());
        assert!(app.app().help_open());
    }

    #[test]
    fn ctrl_c_explains_how_to_quit() {
        let mut app = App::new(
            Config::default(),
            Theme::default(),
            AppOptions::default(),
            FakeClock::new(),
        );
        assert!(matches!(
            app.action_for(Event::Key(key_event("ctrl+c"))),
            Some(Action::QuitHint)
        ));

        let mut test = test_app();
        test.press(&["ctrl+c"]);
        assert_eq!(test.effects(), []);
        assert!(!test.app().should_quit());
        let expected = Toast {
            severity: Severity::Information,
            title: Some("Do you want to quit?".into()),
            message: Line::from(vec![
                Span::raw("Press "),
                Span::styled("ctrl+q", Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(" to quit the app"),
            ]),
            timeout: crate::toast::DEFAULT_TIMEOUT,
        };
        assert!(test.app().toasts().iter().eq([&expected]));
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = test_app();
        for (code, mods) in [
            (KeyCode::Char('q'), KeyModifiers::NONE),
            (KeyCode::Char('q'), KeyModifiers::CONTROL),
            (KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut release = KeyEvent::new(code, mods);
            release.kind = KeyEventKind::Release;
            app.send(Event::Key(release));
        }
        assert_eq!(app.effects(), []);
        assert!(!app.app().should_quit());
        assert!(app.app().toasts().is_empty());
    }

    #[test]
    fn unbound_keys_and_clicks_do_nothing() {
        let mut app = test_app();
        app.press(&["x", "Q", "escape", "enter", "tab", "shift+tab", "ctrl+x"]);
        app.click(3, 3);
        assert_eq!(app.effects(), []);
        assert!(!app.app().should_quit());
    }

    /// Textual's `FooterKey.on_mouse_down`: a click on a disabled item rings the bell instead of running its binding.
    #[test]
    fn clicking_a_disabled_footer_item_rings_the_bell() {
        let mut app = test_app();
        app.render(160, 48);
        // Nothing is disabled in M1, so disable `q Quit` where the last frame drew it.
        let hit = app
            .app_mut()
            .footer_hits
            .iter_mut()
            .find(|hit| hit.id == BindingId::AppQuit)
            .unwrap();
        hit.enabled = false;
        let (column, row) = (hit.rect.x + 1, hit.rect.y);
        app.click(column, row);
        assert_eq!(app.effects(), [Effect::Bell]);
        assert!(!app.app().should_quit());
    }

    #[test]
    fn signals_quit() {
        for signal in [Signal::Term, Signal::Hup, Signal::Int] {
            let mut app = test_app();
            app.send(Event::Signal(signal));
            assert_eq!(app.effects(), [Effect::Quit]);
            assert!(app.app().should_quit());
        }
    }

    #[test]
    fn a_failed_save_logs_and_raises_an_error_toast() {
        let mut app = test_app();
        let out = logs(|| {
            app.send(Event::ConfigSaved(Err("disk full".into())));
        });
        assert_eq!(out, "ERROR Failed to save config: disk full\n");
        let expected = Toast::new(Severity::Error, "Failed to save settings: disk full");
        assert!(app.app().toasts().iter().eq([&expected]));
    }

    #[test]
    fn changing_the_config_saves_it_once_per_action() {
        let mut app = test_app();
        app.with_cx(|cx| {
            "nord".clone_into(&mut cx.config_mut().appearance.theme);
            cx.config_mut().appearance.show_command_log = false;
        });
        let saved: Vec<_> = app
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::SaveConfig(config) => Some(config.appearance.theme.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(saved, ["nord"]);
        assert_eq!(app.app().config().appearance.theme, "nord");
    }

    #[test]
    fn stale_responses_lose_to_the_latest_request() {
        let mut app = test_app();
        let mut component = LatestWins::default();
        app.with_cx(|cx| component.update(LatestWinsAction::Load, cx));
        app.with_cx(|cx| component.update(LatestWinsAction::Load, cx));
        let request_id = |effect: &Effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::CurrentUser,
            } => *id,
            other => panic!("unexpected {other:?}"),
        };
        let first = request_id(&app.effects()[0]);
        let second = request_id(&app.effects()[1]);
        assert_ne!(first, second);
        assert_eq!(app.effects()[2..], [Effect::Cancel(first)]);

        let out = logs(|| {
            app.with_cx(|cx| {
                let response = GithubResponse::CurrentUser(Ok(user("second")));
                component.update(LatestWinsAction::Loaded(second, response), cx);
                let response = GithubResponse::CurrentUser(Ok(user("first")));
                component.update(LatestWinsAction::Loaded(first, response), cx);
            });
        });
        assert_eq!(component.login.as_deref(), Some("second"));
        assert_eq!(out, format!("DEBUG Dropping the stale response to request {first}\n"));
    }

    #[test]
    fn command_log_lines_before_the_main_screen_are_dropped() {
        let line = |message: &str| LogLine {
            level: Level::INFO,
            timestamp: jiff::civil::date(2026, 10, 25).at(1, 2, 3, 0),
            message: message.into(),
        };
        let mut config = Config::default();
        config.appearance.show_command_log = true;
        let mut app = TestApp::on_startup(config, lazy_github_core::github::FakeGithub::new());
        assert_eq!(app.app().screen(), None);
        app.send(Event::CommandLog(line("during startup")));
        futures_util::FutureExt::now_or_never(app.settle()).unwrap();
        assert_eq!(app.app().screen(), Some(ScreenKind::Main));
        app.send(Event::CommandLog(line("on the main screen")));
        let log = app.app().main_screen().unwrap().command_log();
        assert_eq!(log.lines(), ["INFO 2026-10-25 01:02:03 - on the main screen"]);
    }

    #[test]
    fn a_startup_warning_is_logged_once_the_main_screen_is_up() {
        let mut app = TestApp::on_startup(Config::default(), lazy_github_core::github::FakeGithub::new());
        app.app_mut()
            .warn_on_main_screen("Could not open log file x: denied. Continuing without a log file.");
        let out = logs(|| futures_util::FutureExt::now_or_never(app.settle()).unwrap());
        assert_eq!(app.app().screen(), Some(ScreenKind::Main));
        let warnings: Vec<&str> = out.lines().filter(|line| line.starts_with("WARN ")).collect();
        assert_eq!(
            warnings,
            ["WARN Could not open log file x: denied. Continuing without a log file."]
        );
        // Only the first time.
        let out = logs(|| {
            app.app_mut().update(Action::EnterMain);
        });
        assert!(!out.contains("WARN "), "{out}");
    }

    #[test]
    fn settle_runs_requests_on_the_fake() {
        let mut app = test_app();
        let id = app.with_cx(|cx| cx.request(GithubRequest::CheckAuth));
        let out = logs(|| futures_util::FutureExt::now_or_never(app.settle()).unwrap());
        assert_eq!(
            app.gh().calls(),
            [
                GithubCall::ListRepositories(lazy_github_core::github::RepoListQuery::all()),
                GithubCall::CheckAuth
            ]
        );
        assert_eq!(
            out,
            format!("DEBUG Dropping the response to request {id}: nothing is waiting for it\n")
        );
    }

    #[test]
    fn the_current_branch_is_read_once_on_entering_main() {
        let mut app = TestApp::new(Config::default()).with_local_branch(Some("new-feature"));
        assert_eq!(app.with_cx(|cx| cx.current_branch().map(str::to_owned)), None);
        futures_util::FutureExt::now_or_never(app.settle()).unwrap();
        assert_eq!(
            app.with_cx(|cx| cx.current_branch().map(str::to_owned)).as_deref(),
            Some("new-feature")
        );
        // Entering the main screen again doesn't ask again.
        let before = app.effects().len();
        app.dispatch(Action::EnterMain);
        assert!(
            !app.effects()[before..]
                .iter()
                .any(|effect| matches!(effect, Effect::Git { .. })),
            "{:?}",
            &app.effects()[before..]
        );
    }

    #[test]
    fn an_unclaimed_git_response_is_dropped() {
        let repo = lazy_github_core::models::RepoName::from_remote_url("git@github.com:o/n.git");
        let mut app = TestApp::new(Config::default()).with_local_repo(repo.clone());
        let id = app.with_cx(|cx| cx.git(GitRequest::OriginRepo));
        assert_eq!(
            app.effects().last(),
            Some(&Effect::Git {
                id,
                request: GitRequest::OriginRepo
            })
        );
        let out = logs(|| futures_util::FutureExt::now_or_never(app.settle()).unwrap());
        assert_eq!(
            out,
            format!("DEBUG Dropping the response to request {id}: nothing is waiting for it\n")
        );

        // Fed straight in, as the runtime would deliver it: dropped the same way, with nothing else happening.
        let before = app.effects().len();
        let event = Event::Git {
            id,
            response: GitResponse::OriginRepo(repo),
        };
        let out = logs(|| {
            app.send(event);
        });
        assert_eq!(
            out,
            format!("DEBUG Dropping the response to request {id}: nothing is waiting for it\n")
        );
        assert_eq!(app.effects().len(), before);
    }

    #[test]
    fn git_and_github_requests_share_the_id_counter() {
        let mut app = test_app();
        let (github, git) = app.with_cx(|cx| (cx.request(GithubRequest::CheckAuth), cx.git(GitRequest::OriginRepo)));
        assert!(git > github);
    }

    #[test]
    fn advance_fires_due_timers() {
        let mut app = test_app();
        app.with_cx(|cx| {
            cx.effect(Effect::StartInterval {
                id: TimerId::AuthPoll,
                every: Duration::from_secs(10),
            });
        });
        let out = logs(|| app.advance(Duration::from_secs(25)));
        assert_eq!(out, "DEBUG Ignoring timer AuthPoll: nothing owns it\n".repeat(2));
    }
}
