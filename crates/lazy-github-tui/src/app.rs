//! `App`: all UI state, the synchronous `update`, key dispatch and rendering.

use std::collections::VecDeque;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::logging::LogLine;
use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;
use tokio::time::Instant;

use crate::action::Action;
use crate::clock::Clock;
use crate::component::{BindingState, ChainLink, Component, Cx, Outbox, Owner, RenderCx, ScreenKind, Shared};
use crate::effect::{Effect, GithubResponse, RequestId, TimerId};
use crate::event::Event;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::screens::auth::{AuthAction, AuthReason, AuthScreen};
use crate::screens::main::{FocusTarget, MainScreen};
use crate::screens::startup::{StartupDecision, StartupScreen, naive_now};
use crate::theme::Theme;
use crate::toast::{Severity, Toast};

/// How many command-log lines `App` keeps for the command log pane (#80).
pub const COMMAND_LOG_LIMIT: usize = 1000;

/// The bindings on the app link of every non-modal chain, in Textual order: its built-ins, then `LazyGithub.BINDINGS`.
const APP_BINDINGS: &[BindingId] = &[
    BindingId::TextualQuit,
    BindingId::TextualHelpQuit,
    BindingId::AppQuit,
    BindingId::AppCommandPalette,
    BindingId::AppMaximizeWidget,
    BindingId::AppOpenHelp,
];

/// The widget shown alone while maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Maximized {
    /// What had focus when it was maximized. It stays fixed until minimized, even when focus keys move focus.
    pub target: FocusTarget,
}

/// The screen under any modals.
enum BaseScreen {
    /// Blank, until the startup auth check decides.
    Startup(StartupScreen),
    /// `gh` can't be used yet. Modal: only its own bindings and priority ones apply.
    Auth(AuthScreen),
    /// Boxed: it's much bigger than the other screens.
    Main(Box<MainScreen>),
}

/// The whole UI. One value owns all state; [`App::update`] is synchronous and does no I/O.
pub struct App {
    base: BaseScreen,
    shared: Shared,
    toasts: Vec<Toast>,
    command_log: VecDeque<LogLine>,
    maximized: Option<Maximized>,
    /// The maximize toast shows once per session. Never saved.
    has_shown_maximize_toast: bool,
    should_quit: bool,
}

impl App {
    /// An app on the blank startup screen, with the keymap built from `config`'s `bindings.overrides`. The clock is
    /// the only source of time for the app and its components. [`Self::init`] decides where it goes next.
    pub fn new(config: Config, theme: Theme, clock: impl Clock + 'static) -> Self {
        let keymap = Keymap::new(&config.bindings.overrides);
        Self {
            base: BaseScreen::Startup(StartupScreen::default()),
            shared: Shared::new(config, keymap, theme, Arc::new(clock)),
            toasts: Vec::new(),
            command_log: VecDeque::new(),
            maximized: None,
            has_shown_maximize_toast: false,
            should_quit: false,
        }
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

    /// Shows the main screen without the startup check, for tests of everything after it.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn skip_startup(&mut self) {
        self.base = BaseScreen::Main(Box::new(self.new_main_screen()));
    }

    fn new_main_screen(&self) -> MainScreen {
        MainScreen::new(&self.shared.config.appearance, &self.shared.keymap)
    }

    /// Maps `event` to actions and applies them.
    pub fn handle_event(&mut self, event: Event) -> Vec<Effect> {
        match self.action_for(event) {
            Some(action) => self.update(action),
            None => Vec::new(),
        }
    }

    /// The action an event stands for, if any.
    pub(crate) fn action_for(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Key(key) => self.action_for_key(key),
            Event::Github { id, response } => Some(Action::Github(id, response)),
            Event::ConfigSaved(result) => Some(Action::ConfigSaved(result)),
            Event::Timer(id) => Some(Action::Timer(id)),
            Event::CommandLog(line) => Some(Action::CommandLog(line)),
            Event::Signal(signal) => {
                tracing::debug!("Received {signal:?}; quitting");
                Some(Action::Quit)
            }
            Event::Mouse(mouse) => self.action_for_mouse(mouse),
            // Resizes need no state: the next draw uses the new size. Nothing animates yet, so a deadline has
            // nothing to advance.
            Event::Paste(_) | Event::Resize(..) | Event::FocusGained | Event::FocusLost | Event::Deadline => None,
        }
    }

    /// Key dispatch, in Textual's order.
    fn action_for_key(&mut self, key: KeyEvent) -> Option<Action> {
        // 1. Normalize; releases are ignored.
        let chord = KeyChord::from_event(&key)?;
        // 2. Escape minimizes a maximized widget before any binding (Textual's `ESCAPE_TO_MINIMIZE`).
        if self.maximized.is_some() && chord == KeyChord::new(KeyCode::Esc, KeyModifiers::NONE) {
            return Some(Action::Minimize);
        }
        let chain = self.binding_chain();
        // 3. Priority bindings, from the app link inward, past any modal screen. The clash rule sees the whole link,
        //    as in Textual.
        for link in self.full_binding_chain().iter().rev() {
            let matches: Vec<BindingId> = self.shared.keymap.priority_matches_in(link.bindings, &chord).collect();
            if let Some(action) = self.route_first(link.owner, matches) {
                return Some(action);
            }
        }
        // 4. A focused component that consumes the key: none can have focus yet (#75).
        // 5. The chain, innermost first. Within a link, each binding the key fires is tried in turn; the first link
        //    whose component accepts one wins.
        for link in &chain {
            let matches: Vec<BindingId> = self.shared.keymap.matches_in(link.bindings, &chord).collect();
            if let Some(action) = self.route_first(link.owner, matches) {
                return Some(action);
            }
        }
        // 6. Unbound keys go to the focused component's `handle_key`, once something can have focus.
        None
    }

    /// A left click focuses the target under the pointer, as of the last frame.
    fn action_for_mouse(&self, mouse: MouseEvent) -> Option<Action> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        match &self.base {
            BaseScreen::Main(main) => main.hit(mouse.column, mouse.row).map(Action::Focus),
            BaseScreen::Startup(_) | BaseScreen::Auth(_) => None,
        }
    }

    /// Tries each binding a key fired in one link, in order, until one applies (Textual's `_check_bindings`).
    fn route_first(&mut self, owner: Owner, matches: Vec<BindingId>) -> Option<Action> {
        matches.into_iter().find_map(|id| self.route_binding(owner, id))
    }

    /// The binding chain for the current focus, innermost first, ending at a modal screen as Textual's
    /// `_modal_binding_chain` does. Otherwise the app link comes last.
    pub fn binding_chain(&self) -> Vec<ChainLink> {
        let mut chain = self.full_binding_chain();
        let is_modal = |link: &ChainLink| matches!(link.owner, Owner::Screen(kind) if kind.is_modal());
        if let Some(end) = chain.iter().position(is_modal) {
            chain.truncate(end + 1);
        }
        chain
    }

    /// The whole chain, modal screens included, which priority bindings are looked up in. On the main screen it's
    /// the focus chain from [`MainScreen::binding_chain`].
    fn full_binding_chain(&self) -> Vec<ChainLink> {
        let mut chain = match &self.base {
            BaseScreen::Startup(_) => Vec::new(),
            BaseScreen::Auth(auth) => vec![ChainLink {
                owner: Owner::Screen(ScreenKind::Auth),
                bindings: auth.bindings(),
            }],
            BaseScreen::Main(main) => main.binding_chain(),
        };
        chain.push(ChainLink {
            owner: Owner::App,
            bindings: APP_BINDINGS,
        });
        chain
    }

    /// How the footer and help panel (#76) show binding `id` of the chain link `owner`.
    pub fn binding_state(&mut self, owner: Owner, id: BindingId) -> BindingState {
        let mut scratch = Outbox::default();
        let cx = Cx::new(&mut self.shared, &mut scratch);
        match (&self.base, owner) {
            (_, Owner::App) => BindingState::Enabled,
            (BaseScreen::Main(main), owner) => main.link_binding_state(owner, id, &cx),
            (BaseScreen::Auth(auth), Owner::Screen(ScreenKind::Auth)) => auth.binding_state(id, &cx),
            _ => BindingState::Enabled,
        }
    }

    /// Asks the owner of a matched binding what it means right now.
    fn route_binding(&mut self, owner: Owner, id: BindingId) -> Option<Action> {
        match owner {
            Owner::App => match id {
                BindingId::AppQuit | BindingId::TextualQuit => Some(Action::Quit),
                BindingId::TextualHelpQuit => Some(Action::QuitHint),
                BindingId::AppMaximizeWidget => Some(Action::Maximize),
                other => Some(Action::Unimplemented(other)),
            },
            owner => {
                let mut scratch = Outbox::default();
                let cx = Cx::new(&mut self.shared, &mut scratch);
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
        while let Some(action) = out.actions.pop_front() {
            self.apply(action, out);
        }
        // One save per processed action, like Python's `Config.to_edit()`.
        if self.shared.take_config_dirty() {
            out.effects
                .push(Effect::SaveConfig(Box::new(self.shared.config.clone())));
        }
    }

    fn apply(&mut self, action: Action, out: &mut Outbox) {
        match action {
            Action::Quit => {
                self.should_quit = true;
                out.effects.push(Effect::Quit);
            }
            Action::QuitHint => out.actions.push_back(Action::Notify(quit_hint())),
            Action::Notify(toast) => self.toasts.push(toast),
            Action::Focus(target) => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.try_focus(target);
                }
            }
            Action::FocusNext | Action::FocusPrevious => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.focus_step(matches!(action, Action::FocusNext), self.maximized.map(|m| m.target));
                }
            }
            Action::Maximize => self.maximize(out),
            Action::Minimize => self.maximized = None,
            Action::Main(action) => {
                if let BaseScreen::Main(main) = &mut self.base {
                    main.update(action, &mut Cx::new(&mut self.shared, out));
                }
            }
            Action::EnterMain => {
                if matches!(self.base, BaseScreen::Auth(_)) {
                    out.effects.push(Effect::CancelTimer(TimerId::AuthPoll));
                }
                // A new main screen starts unmaximized, focused on repos.
                self.maximized = None;
                self.base = BaseScreen::Main(Box::new(self.new_main_screen()));
            }
            Action::Auth(action) => {
                if let BaseScreen::Auth(auth) = &mut self.base {
                    auth.update(action, &mut Cx::new(&mut self.shared, out));
                } else {
                    tracing::debug!("Dropping {action:?}: the authentication screen is gone");
                }
            }
            Action::Github(id, response) => self.route_response(id, response, out),
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
            Action::Timer(id) => tracing::debug!("Ignoring timer {id:?}: nothing owns it"),
            Action::CommandLog(line) => {
                if self.command_log.len() == COMMAND_LOG_LIMIT {
                    self.command_log.pop_front();
                }
                self.command_log.push_back(line);
            }
            Action::Unimplemented(id) => tracing::debug!("{id:?} is not implemented yet"),
        }
    }

    /// Shows only the focused widget, with the one-time toast. A no-op while something is maximized: `M` doesn't
    /// toggle back.
    fn maximize(&mut self, out: &mut Outbox) {
        if self.maximized.is_some() {
            return;
        }
        let BaseScreen::Main(main) = &self.base else { return };
        let target = main.focus();
        self.maximized = Some(Maximized { target });
        if !self.has_shown_maximize_toast {
            self.has_shown_maximize_toast = true;
            out.actions.push_back(Action::Notify(maximize_toast()));
        }
    }

    /// What is maximized, if anything.
    pub fn maximized(&self) -> Option<Maximized> {
        self.maximized
    }

    /// Hands a response to the screen waiting for it.
    fn route_response(&mut self, id: RequestId, response: GithubResponse, out: &mut Outbox) {
        // Screens and panes that make other requests (M2) claim their responses here by id.
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

    /// Draws the whole UI: the base screen, then (later) modals and toasts.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
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
        // While maximized, the widget replaces the whole screen, footer and help panel included.
        match (&mut self.base, self.maximized) {
            (BaseScreen::Startup(_), _) => {}
            (BaseScreen::Auth(auth), _) => auth.render(frame, area, &cx),
            (BaseScreen::Main(main), Some(maximized)) => main.render_maximized(frame, area, maximized.target, &cx),
            (BaseScreen::Main(main), None) => main.render(frame, area, &cx),
        }
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

    /// The next time something on screen changes by itself (toast expiry, a spinner frame), or `None` when nothing
    /// animates. Toasts (#79) and spinners (#78) feed it.
    pub fn next_deadline(&self) -> Option<Instant> {
        None
    }

    /// The most recent command-log lines, oldest first, at most [`COMMAND_LOG_LIMIT`].
    pub fn command_log(&self) -> &VecDeque<LogLine> {
        &self.command_log
    }

    /// Every toast raised so far, oldest first. #79 renders and expires them.
    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    /// The config as the app currently holds it.
    pub fn config(&self) -> &Config {
        &self.shared.config
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
    use std::sync::Mutex;
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    use lazy_github_core::github::GithubCall;
    use lazy_github_core::models::User;
    use tracing::Level;

    use super::*;
    use crate::clock::FakeClock;
    use crate::effect::{GithubRequest, GithubResponse, TimerId};
    use crate::event::Signal;
    use crate::test_util::{LatestWins, LatestWinsAction, TestApp, key_event};

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

    /// What `f` logs at `DEBUG` and above, as `LEVEL message` lines.
    fn logs(f: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::DEBUG)
            .without_time()
            .with_target(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        // The fmt layer pads levels to five columns.
        out.lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ") + "\n")
            .collect()
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

    #[test]
    fn ctrl_c_explains_how_to_quit() {
        let mut app = App::new(Config::default(), Theme::default(), FakeClock::new());
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
        };
        assert_eq!(test.app().toasts(), [expected]);
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

    #[test]
    fn signals_quit() {
        for signal in [Signal::Term, Signal::Hup] {
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
        assert_eq!(
            app.app().toasts(),
            [Toast::new(Severity::Error, "Failed to save settings: disk full")]
        );
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
    fn the_command_log_keeps_the_last_thousand_lines() {
        let mut app = test_app();
        let line = |n: usize| LogLine {
            level: Level::INFO,
            timestamp: jiff::civil::date(2026, 10, 25).at(1, 2, 3, 0),
            message: format!("line {n}"),
        };
        app.send(Event::CommandLog(line(0)));
        assert_eq!(app.app().command_log().iter().collect::<Vec<_>>(), [&line(0)]);
        for n in 1..=COMMAND_LOG_LIMIT {
            app.send(Event::CommandLog(line(n)));
        }
        let log = app.app().command_log();
        assert_eq!(log.len(), COMMAND_LOG_LIMIT);
        assert_eq!(log.front(), Some(&line(1)));
        assert_eq!(log.back(), Some(&line(COMMAND_LOG_LIMIT)));
    }

    #[test]
    fn settle_runs_requests_on_the_fake() {
        let mut app = test_app();
        let id = app.with_cx(|cx| cx.request(GithubRequest::CheckAuth));
        let out = logs(|| futures_util::FutureExt::now_or_never(app.settle()).unwrap());
        assert_eq!(app.gh().calls(), [GithubCall::CheckAuth]);
        assert_eq!(
            out,
            format!("DEBUG Dropping the response to request {id}: nothing is waiting for it\n")
        );
    }

    #[test]
    fn advance_fires_due_timers() {
        let mut app = test_app();
        app.with_cx(|cx| {
            cx.effect(Effect::StartInterval {
                id: TimerId::Notifications,
                every: Duration::from_secs(10),
            });
        });
        let out = logs(|| app.advance(Duration::from_secs(25)));
        assert_eq!(out, "DEBUG Ignoring timer Notifications: nothing owns it\n".repeat(2));
    }
}
