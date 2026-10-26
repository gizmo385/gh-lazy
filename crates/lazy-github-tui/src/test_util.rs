//! The UI test harness: [`TestApp`] drives an [`App`] with keys, clicks, a fake clock and [`FakeGithub`], without a
//! terminal or a tokio runtime of its own.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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
use crate::keymap::KeySpec;
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
    /// An app on the main screen with `config` (and its keymap), the default theme, and a logged-in [`FakeGithub`].
    /// The startup auth check is skipped; [`Self::on_startup`] runs it.
    pub fn new(config: Config) -> Self {
        Self::with_github(config, FakeGithub::new())
    }

    /// Like [`Self::new`], with requests answered by `gh`.
    pub fn with_github(config: Config, gh: FakeGithub) -> Self {
        let mut test = Self::build(config, Theme::default(), gh);
        let effects = test.app.skip_startup();
        test.perform(effects);
        test
    }

    /// Like [`Self::new`], drawn with `theme` (for example one reduced to a 256-color terminal).
    pub fn with_theme(config: Config, theme: Theme) -> Self {
        let mut test = Self::build(config, theme, FakeGithub::new());
        let effects = test.app.skip_startup();
        test.perform(effects);
        test
    }

    /// An app as it launches: on the blank startup screen, with [`App::init`]'s effects performed. Requests it makes
    /// are answered by `gh` on [`Self::settle`].
    pub fn on_startup(config: Config, gh: FakeGithub) -> Self {
        let mut test = Self::build(config, Theme::default(), gh);
        let init = test.app.init();
        test.perform(init);
        test
    }

    fn build(config: Config, theme: Theme, gh: FakeGithub) -> Self {
        let clock = FakeClock::new();
        let app = App::new(config, theme, clock.clone());
        Self {
            app,
            gh,
            clock,
            effects: Vec::new(),
            queued: VecDeque::new(),
            timers: HashMap::new(),
        }
    }

    /// Presses each key in turn, in Textual key syntax ([`KeySpec::parse`]): `"q"`, `"M"`, `"escape"`, `"shift+tab"`,
    /// `"ctrl+q"`, `"question_mark"`.
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

    /// The app, for setting up state that no key reaches yet.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
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
                Effect::Bell | Effect::Quit => {}
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

/// What `f` logs at `DEBUG` and above, as `LEVEL message` lines.
#[cfg(test)]
pub fn logs(f: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
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
        .with_max_level(tracing::Level::DEBUG)
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

/// The key event a terminal sends for `key`, in Textual key syntax ([`KeySpec::parse`]).
///
/// # Panics
///
/// If `key` isn't a key, so a typo fails the test instead of pressing nothing.
pub fn key_event(key: &str) -> KeyEvent {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord.to_event(),
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
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
