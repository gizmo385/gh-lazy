//! The UI test harness: [`TestApp`] drives an [`App`] with keys, clicks, a fake clock and [`FakeGithub`], without a
//! terminal or a tokio runtime of its own.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tokio::time::Instant;

use crate::app::App;
use crate::clock::{Clock as _, FakeClock};
use crate::component::{Component, Cx, RenderCx};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId, TimerId};
use crate::event::Event;
use crate::keymap::Keymap;
use crate::theme::Theme;

/// An [`App`] under test.
///
/// Effects are recorded in [`Self::effects`] and performed against fakes: GitHub requests run on [`FakeGithub`] in
/// [`Self::settle`], config saves succeed without touching the disk, and timers fire from [`Self::advance`].
pub struct TestApp {
    app: App,
    gh: FakeGithub,
    clock: FakeClock,
    effects: Vec<Effect>,
    queued: VecDeque<Effect>,
    timers: HashMap<TimerId, FakeTimer>,
}

struct FakeTimer {
    next: Instant,
    every: Option<Duration>,
}

impl TestApp {
    /// An app on the main screen with `config`, the default keymap and theme, and a logged-in [`FakeGithub`].
    pub fn new(config: Config) -> Self {
        Self::with_github(config, FakeGithub::new())
    }

    /// Like [`Self::new`], with requests answered by `gh`.
    pub fn with_github(config: Config, gh: FakeGithub) -> Self {
        let clock = FakeClock::new();
        let mut app = App::new(config, Keymap::defaults(), Theme::default(), clock.clone());
        let init = app.init();
        let mut test = Self {
            app,
            gh,
            clock,
            effects: Vec::new(),
            queued: VecDeque::new(),
            timers: HashMap::new(),
        };
        test.perform(init);
        test
    }

    /// Presses each key in turn, in Textual key syntax: `"q"`, `"M"`, `"escape"`, `"enter"`, `"tab"`, `"shift+tab"`,
    /// `"ctrl+q"`.
    pub fn press(&mut self, keys: &[&str]) -> &mut Self {
        for key in keys {
            self.send(Event::Key(key_event(key)));
        }
        self
    }

    /// Left-clicks the cell at `(col, row)`.
    pub fn click(&mut self, col: u16, row: u16) -> &mut Self {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            self.send(Event::Mouse(MouseEvent {
                kind,
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            }));
        }
        self
    }

    /// Feeds one event to the app and performs the effects it returns.
    pub fn send(&mut self, event: Event) -> &mut Self {
        let effects = self.app.handle_event(event);
        self.perform(effects);
        self
    }

    /// Runs queued GitHub requests on the [`FakeGithub`] and completes queued config saves, feeding each result
    /// back, until nothing is left.
    pub async fn settle(&mut self) {
        while let Some(effect) = self.queued.pop_front() {
            let event = match effect {
                Effect::Github { id, request } => Event::Github {
                    id,
                    response: request.run(&self.gh).await,
                },
                Effect::SaveConfig(_) => Event::ConfigSaved(Ok(())),
                _ => continue,
            };
            self.send(event);
        }
    }

    /// Moves the fake clock forward by `d`, firing each timer and deadline that falls due on the way, in order.
    pub fn advance(&mut self, d: Duration) {
        let target = self.clock.now() + d;
        loop {
            let timer = self
                .timers
                .iter()
                .map(|(&id, timer)| (timer.next, Some(id)))
                .min_by_key(|&(next, _)| next);
            let deadline = self.app.next_deadline().map(|at| (at, None));
            let due = [timer, deadline]
                .into_iter()
                .flatten()
                .filter(|&(at, _)| at <= target)
                .min_by_key(|&(at, _)| at);
            let Some((at, timer)) = due else { break };
            self.clock.advance(at.saturating_duration_since(self.clock.now()));
            if let Some(id) = timer {
                if let Some(every) = self.timers.get(&id).and_then(|timer| timer.every) {
                    self.timers.insert(
                        id,
                        FakeTimer {
                            next: at + every,
                            every: Some(every),
                        },
                    );
                } else {
                    self.timers.remove(&id);
                }
                self.send(Event::Timer(id));
            } else {
                self.send(Event::Deadline);
                // A deadline that doesn't move after firing would fire forever.
                if self.app.next_deadline().is_some_and(|next| next <= at) {
                    break;
                }
            }
        }
        self.clock.advance(target.saturating_duration_since(self.clock.now()));
    }

    /// Every effect emitted so far, oldest first.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The app.
    pub fn app(&self) -> &App {
        &self.app
    }

    /// The fake GitHub, for its call log.
    pub fn gh(&self) -> &FakeGithub {
        &self.gh
    }

    /// The fake clock.
    pub fn clock(&self) -> &FakeClock {
        &self.clock
    }

    /// Runs `f` with an update context, as a component's `update` gets it, and performs what it emits. For testing
    /// a component on its own.
    pub fn with_cx<R>(&mut self, f: impl FnOnce(&mut Cx<'_>) -> R) -> R {
        let (result, effects) = self.app.with_cx(f);
        self.perform(effects);
        result
    }

    /// Draws the app at `cols` x `rows`.
    pub fn render(&mut self, cols: u16, rows: u16) -> Buffer {
        // `TestBackend` can't fail.
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("a TestBackend terminal");
        terminal
            .draw(|frame| self.app.render(frame))
            .expect("drawing on a TestBackend");
        terminal.backend().buffer().clone()
    }

    /// Draws the app and returns its characters, one line per row: the standard snapshot.
    pub fn screen(&mut self, cols: u16, rows: u16) -> String {
        buffer_to_string(&self.render(cols, rows))
    }

    fn perform(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match &effect {
                Effect::Github { .. } | Effect::SaveConfig(_) => self.queued.push_back(effect.clone()),
                Effect::Cancel(id) => self
                    .queued
                    .retain(|queued| !matches!(queued, Effect::Github { id: queued, .. } if queued == id)),
                Effect::StartInterval { id, every } => {
                    let next = self.clock.now() + *every;
                    self.timers.entry(*id).or_insert(FakeTimer {
                        next,
                        every: Some(*every),
                    });
                }
                Effect::StartTimeout { id, after } => {
                    let next = self.clock.now() + *after;
                    self.timers.insert(*id, FakeTimer { next, every: None });
                }
                Effect::CancelTimer(id) => {
                    self.timers.remove(id);
                }
                Effect::Quit => {}
            }
            self.effects.push(effect);
        }
    }
}

/// The characters of `buffer`, one line per row, with trailing spaces trimmed.
pub fn buffer_to_string(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// The key event a terminal sends for `key`, in Textual key syntax.
///
/// A minimal parser for single characters, `escape`, `enter`, `tab`, `shift+tab` and `ctrl+<letter>`, until #73's
/// `KeySpec::parse` replaces it.
pub fn key_event(key: &str) -> KeyEvent {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let mods = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        return KeyEvent::new(KeyCode::Char(c), mods);
    }
    match key {
        "escape" => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        "enter" => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        "tab" => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        "shift+tab" => KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        _ => match key
            .strip_prefix("ctrl+")
            .map(|rest| rest.chars().collect::<Vec<_>>())
            .as_deref()
        {
            Some(&[c]) if c.is_ascii_alphabetic() => KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            _ => panic!("TestApp can't parse the key {key:?} yet; the full parser arrives with #73"),
        },
    }
}

/// A component that loads the current user, for testing the latest-wins rule: it keeps only the response to the
/// request it is waiting for.
#[derive(Debug, Default)]
pub struct LatestWins {
    pending: Option<RequestId>,
    /// The login from the last response that was applied.
    pub login: Option<String>,
}

/// [`LatestWins`]'s actions.
#[derive(Debug)]
pub enum LatestWinsAction {
    /// Start loading, replacing any request in flight.
    Load,
    /// A response arrived.
    Loaded(RequestId, GithubResponse),
}

impl Component for LatestWins {
    type Action = LatestWinsAction;

    fn update(&mut self, action: LatestWinsAction, cx: &mut Cx<'_>) {
        match action {
            LatestWinsAction::Load => {
                let id = cx.request(GithubRequest::CurrentUser);
                if let Some(old) = self.pending.replace(id) {
                    cx.effect(Effect::Cancel(old));
                }
            }
            LatestWinsAction::Loaded(id, response) => {
                if self.pending != Some(id) {
                    tracing::debug!("Dropping the stale response to request {id}");
                    return;
                }
                self.pending = None;
                if let GithubResponse::CurrentUser(Ok(user)) = response {
                    self.login = Some(user.login);
                }
            }
        }
    }

    fn render(&mut self, _frame: &mut ratatui::Frame<'_>, _area: Rect, _cx: &RenderCx<'_>) {}
}
