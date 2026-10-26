//! The UI test harness: [`TestApp`] drives an [`App`] with keys, clicks, a fake clock and [`FakeGithub`], without a
//! terminal or a tokio runtime of its own.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::RepoName;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;
use tokio::time::Instant;

use crate::action::Action;
use crate::app::App;
use crate::clock::{Clock as _, FakeClock};
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::effect::{Effect, GitRequest, GitResponse, GithubRequest, GithubResponse, RequestId, TimerId};
use crate::event::Event;
use crate::keymap::{BindingId, KeyChord, KeySpec, Keymap};
use crate::modal::{
    Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, TestDialogAction, dialog_bindings,
};
use crate::options::AppOptions;
use crate::theme::Theme;

/// The options every [`TestApp`] runs with unless it says otherwise: no animations.
const TEST_OPTIONS: AppOptions = AppOptions { animations: false };

/// An [`App`] under test.
///
/// Effects are recorded in [`Self::effects`] and performed against fakes: GitHub requests run on [`FakeGithub`] and
/// git requests are answered from [`Self::with_local_repo`] in [`Self::settle`], config saves succeed without
/// touching the disk, and timers fire from [`Self::advance`].
pub struct TestApp {
    app: App,
    gh: FakeGithub,
    /// The working directory's `origin` repo, `None` for "not a repo".
    local_repo: Option<RepoName>,
    /// The main screen's `OriginRepo` request, held back at setup until [`Self::with_local_repo`] or
    /// [`Self::settle`] answers it.
    setup_git: Option<(RequestId, GitRequest)>,
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
    /// The startup auth check is skipped; [`Self::on_startup`] runs it. The repos list the main screen asks for is
    /// answered at once (the fake has no repos unless `gh` says otherwise), so the screen is loaded; [`Self::loading`]
    /// leaves it pending. That request is setup: it's in the fake's call log but not in [`Self::effects`], which
    /// starts with the other startup effects, such as the notifications poll. The git request for the
    /// current-directory repo is setup too, answered by [`Self::with_local_repo`] (or [`Self::settle`]), after the
    /// repos.
    ///
    /// Animations are off, as under the parity harness's `TEXTUAL_ANIMATIONS=none`, so loading overlays are still.
    pub fn new(config: Config) -> Self {
        Self::with_github(config, FakeGithub::new())
    }

    /// Like [`Self::new`], with requests answered by `gh`.
    pub fn with_github(config: Config, gh: FakeGithub) -> Self {
        let mut test = Self::loading(config, gh);
        test.answer_repos();
        test.hold_setup_git();
        test
    }

    /// Like [`Self::with_github`], with the main screen's repos list and git request still in flight, in
    /// [`Self::effects`]: [`Self::settle`] answers them, repos first.
    pub fn loading(config: Config, gh: FakeGithub) -> Self {
        Self::entered(config, Theme::default(), gh, TEST_OPTIONS)
    }

    /// Like [`Self::new`], drawn with `theme` (for example one reduced to a 256-color terminal).
    pub fn with_theme(config: Config, theme: Theme) -> Self {
        let mut test = Self::entered(config, theme, FakeGithub::new(), TEST_OPTIONS);
        test.answer_repos();
        test.hold_setup_git();
        test
    }

    /// Like [`Self::loading`], with `options`.
    pub fn loading_with_options(config: Config, gh: FakeGithub, options: AppOptions) -> Self {
        Self::entered(config, Theme::default(), gh, options)
    }

    fn entered(config: Config, theme: Theme, gh: FakeGithub, options: AppOptions) -> Self {
        let mut test = Self::build(config, theme, gh, options);
        let effects = test.app.skip_startup();
        test.perform(effects);
        test
    }

    /// An app as it launches: on the blank startup screen, with [`App::init`]'s effects performed. Requests it makes
    /// are answered by `gh` on [`Self::settle`].
    pub fn on_startup(config: Config, gh: FakeGithub) -> Self {
        let mut test = Self::build(config, Theme::default(), gh, TEST_OPTIONS);
        let init = test.app.init();
        test.perform(init);
        test
    }

    fn build(config: Config, theme: Theme, gh: FakeGithub, options: AppOptions) -> Self {
        let clock = FakeClock::new();
        let app = App::new(config, theme, options, clock.clone());
        Self {
            app,
            gh,
            local_repo: None,
            setup_git: None,
            clock,
            effects: Vec::new(),
            queued: VecDeque::new(),
            timers: HashMap::new(),
        }
    }

    /// Answers the queued repos list requests on the fake, leaving every other queued effect in place.
    fn answer_repos(&mut self) {
        let (repos, rest): (VecDeque<Effect>, VecDeque<Effect>) =
            std::mem::take(&mut self.queued).into_iter().partition(|effect| {
                matches!(
                    effect,
                    Effect::Github {
                        request: GithubRequest::ListRepositories(_),
                        ..
                    }
                )
            });
        self.queued = rest;
        self.effects.retain(|effect| !repos.contains(effect));
        for effect in repos {
            if let Effect::Github { id, request } = effect {
                // The fake answers without awaiting anything unless it was given a latency.
                let response = futures_util::FutureExt::now_or_never(request.run(&self.gh))
                    .expect("the fake answers the repos list at once");
                self.send(Event::Github { id, response });
            }
        }
    }

    /// Takes the queued `OriginRepo` request out of [`Self::effects`] and the queue, as setup.
    fn hold_setup_git(&mut self) {
        let position = self.queued.iter().position(|effect| {
            matches!(
                effect,
                Effect::Git {
                    request: GitRequest::OriginRepo,
                    ..
                }
            )
        });
        let Some(Effect::Git { id, request }) = position.and_then(|i| self.queued.remove(i)) else {
            return;
        };
        self.effects
            .retain(|effect| !matches!(effect, Effect::Git { id: held, .. } if *held == id));
        self.setup_git = Some((id, request));
    }

    /// Answers the held setup git request from [`Self::with_local_repo`], if it's still held.
    fn answer_setup_git(&mut self) {
        if let Some((id, request)) = self.setup_git.take() {
            let response = self.answer_git(&request);
            self.send(Event::Git { id, response });
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

    /// Applies one action and performs the effects it returns.
    pub fn dispatch(&mut self, action: Action) -> &mut Self {
        let effects = self.app.update(action);
        self.perform(effects);
        self
    }

    /// Feeds one event to the app and performs the effects it returns.
    pub fn send(&mut self, event: Event) -> &mut Self {
        let effects = self.app.handle_event(event);
        self.perform(effects);
        self
    }

    /// This app with its working directory in a git repo whose `origin` is `repo`; `None` (the default) means the
    /// directory isn't a repo. The main screen's setup request is answered from it now, after the repos list, and
    /// later git requests in [`Self::settle`].
    #[must_use]
    pub fn with_local_repo(mut self, repo: Option<RepoName>) -> Self {
        self.set_local_repo(repo);
        self
    }

    /// [`Self::with_local_repo`] on a borrowed app.
    pub fn set_local_repo(&mut self, repo: Option<RepoName>) {
        self.local_repo = repo;
        self.answer_setup_git();
    }

    /// What git would answer `request` with in the [`Self::with_local_repo`] directory.
    fn answer_git(&self, request: &GitRequest) -> GitResponse {
        match request {
            GitRequest::OriginRepo => GitResponse::OriginRepo(self.local_repo.clone()),
        }
    }

    /// Runs queued GitHub requests on the [`FakeGithub`], answers queued git requests from
    /// [`Self::with_local_repo`] and completes queued config saves, feeding each result back, until nothing is left.
    pub async fn settle(&mut self) {
        self.answer_setup_git();
        while let Some(effect) = self.queued.pop_front() {
            let event = match effect {
                Effect::Github { id, request } => Event::Github {
                    id,
                    response: request.run(&self.gh).await,
                },
                Effect::Git { id, request } => Event::Git {
                    id,
                    response: self.answer_git(&request),
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
                Effect::Github { .. } | Effect::Git { .. } | Effect::SaveConfig(_) => {
                    self.queued.push_back(effect.clone());
                }
                Effect::Cancel(id) => self.queued.retain(|queued| {
                    !matches!(queued, Effect::Github { id: queued, .. } | Effect::Git { id: queued, .. } if queued == id)
                }),
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

/// The test dialog's fields, in Lookup repository's order: the input, another field (Lookup's switch), then the
/// button row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestField {
    /// A text input, which types `q`.
    Input,
    /// A field without bindings, like a switch.
    Plain,
    /// The submit button.
    Submit,
    /// The Cancel button.
    Cancel,
}

impl TestField {
    /// The row's button, for the buttons.
    fn button(self) -> Option<Pressed> {
        match self {
            TestField::Submit => Some(Pressed::Submit),
            TestField::Cancel => Some(Pressed::Cancel),
            TestField::Input | TestField::Plain => None,
        }
    }
}

/// A dialog for testing the modal stack: a text input, a plain field and a Lookup/Cancel [`ButtonRow`], with
/// `modal.submit` and `modal.close`. Its output is the input's text on submit, or `None` on close. By default it's
/// Lookup repository's 60 x 25 with its input focused, and its buttons where Lookup's are.
#[derive(Debug)]
pub struct TestDialog {
    id: u8,
    size: (u16, u16),
    footer: bool,
    ring: FocusRing<TestField>,
    input: TextInput,
    buttons: ButtonRow,
    // The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<String>>,
    /// Mouse events that reached the dialog.
    pub mouse_events: usize,
}

/// The test dialog's screen bindings.
const TEST_DIALOG_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// The rows of the test dialog's interior that each field takes, from the interior's top.
const INPUT_ROW: u16 = 4;
const PLAIN_ROW: u16 = 8;
/// Lookup's button row: its buttons are on interior rows 19 to 21.
const BUTTON_ROW: u16 = 18;

impl TestDialog {
    /// The dialog with id `id`: [`ModalKind::Test`]`(id)`.
    pub fn new(id: u8) -> Self {
        Self {
            id,
            size: (60, 25),
            footer: true,
            ring: FocusRing::new([TestField::Input, TestField::Plain, TestField::Submit, TestField::Cancel]),
            input: TextInput::new("Name"),
            buttons: ButtonRow::new("Lookup"),
            output: None,
            mouse_events: 0,
        }
    }

    /// This dialog at a fixed outer size.
    #[must_use]
    pub fn with_size(mut self, width: u16, height: u16) -> Self {
        self.size = (width, height);
        self
    }

    /// This dialog with its plain field focused.
    #[must_use]
    pub fn focused_on_plain(self) -> Self {
        self.focused_on(TestField::Plain)
    }

    /// This dialog with `field` focused.
    #[must_use]
    pub fn focused_on(mut self, field: TestField) -> Self {
        self.ring.focus(field);
        self
    }

    /// This dialog with `button` disabled, and so blurred if it had focus.
    #[must_use]
    pub fn with_disabled(mut self, field: TestField) -> Self {
        self.set_disabled(field, true);
        self
    }

    /// Disables or enables `field`.
    pub fn set_disabled(&mut self, field: TestField, disabled: bool) {
        if let Some(button) = field.button() {
            self.buttons.button_mut(button).set_disabled(disabled);
        }
        self.ring.set_disabled(field, disabled);
    }

    /// This dialog without a footer, as Create PR has none.
    #[must_use]
    pub fn without_footer(mut self) -> Self {
        self.footer = false;
        self
    }

    fn action(&self, action: TestDialogAction) -> Action {
        Action::Dialog(DialogAction::Test(self.id, action))
    }

    /// What a counted press of `button` does: the same as `modal.submit` or `modal.close`.
    fn pressed(&self, button: Pressed) -> Action {
        self.action(match button {
            Pressed::Submit => TestDialogAction::Submit,
            Pressed::Cancel => TestDialogAction::Close,
        })
    }
}

impl Component for TestDialog {
    type Action = TestDialogAction;

    fn bindings(&self) -> &'static [BindingId] {
        TEST_DIALOG_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalClose => Some(self.action(TestDialogAction::Close)),
            BindingId::ModalSubmit => Some(self.pressed(Pressed::Submit)),
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        self.ring.is_focused(TestField::Input) && self.input.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        match self.input.handle_key(&chord, cx.keymap()) {
            // Like Python's dialogs, `enter` in a single-line input doesn't submit.
            InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
            InputOutcome::Ignored => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, cx: &Cx<'_>) -> Option<Action> {
        self.mouse_events += 1;
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            let field = match button {
                Pressed::Submit => TestField::Submit,
                Pressed::Cancel => TestField::Cancel,
            };
            return Some(match outcome {
                ButtonMouse::Focus => {
                    self.ring.focus(field);
                    Action::Consumed
                }
                ButtonMouse::Pressed => self.pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let row = event.row - area.y;
        if (INPUT_ROW..INPUT_ROW + text_input::HEIGHT).contains(&row) {
            self.ring.focus(TestField::Input);
        } else if row == PLAIN_ROW {
            self.ring.focus(TestField::Plain);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: TestDialogAction, _cx: &mut Cx<'_>) {
        match action {
            TestDialogAction::Close => self.output = Some(None),
            TestDialogAction::Submit => self.output = Some(Some(self.input.value().to_owned())),
            TestDialogAction::Replied(reply) => self.input.set_value(reply.unwrap_or_default()),
        }
    }

    fn render(&mut self, frame: &mut ratatui::Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = self.background(theme.palette).over(theme.background());
        let text = Style::new().fg(theme.color(theme.palette.foreground, under));
        let row = |offset: u16| Rect {
            y: area.y + offset,
            height: 1,
            ..area
        };
        let rows = |offset: u16, height: u16| {
            Rect {
                y: area.y.saturating_add(offset),
                height,
                ..area
            }
            .intersection(area)
        };
        frame.render_widget(
            Paragraph::new("Test dialog").style(text).alignment(Alignment::Center),
            row(1).intersection(area),
        );
        frame.render_widget(
            Paragraph::new("Name:").style(text),
            row(INPUT_ROW - 1).intersection(area),
        );
        let input_area = rows(INPUT_ROW, text_input::HEIGHT);
        self.input.render(
            frame.buffer_mut(),
            input_area,
            theme,
            under,
            self.ring.is_focused(TestField::Input),
        );
        let plain = if self.ring.is_focused(TestField::Plain) {
            text.add_modifier(Modifier::REVERSED)
        } else {
            text
        };
        frame.render_widget(
            Paragraph::new("Plain field").style(plain),
            row(PLAIN_ROW).intersection(area),
        );
        let focused = self.ring.focused().and_then(TestField::button);
        self.buttons
            .render(frame.buffer_mut(), rows(BUTTON_ROW, ButtonRow::HEIGHT), theme, focused);
    }
}

impl Dialog for TestDialog {
    type Output = Option<String>;

    fn kind(&self) -> ModalKind {
        ModalKind::Test(self.id)
    }

    fn own_action(&self, action: DialogAction) -> Option<TestDialogAction> {
        match action {
            DialogAction::Test(id, action) if id == self.id => Some(action),
            DialogAction::Test(..) | DialogAction::Confirm(_) => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.ring.step(direction);
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        self.size
    }

    fn shows_footer(&self) -> bool {
        self.footer
    }

    fn take_output(&mut self) -> Option<Option<String>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        match self.ring.focused() {
            Some(TestField::Input) => vec![FieldKind::TextInput.link(self.kind())],
            Some(TestField::Submit | TestField::Cancel) => vec![FieldKind::Button.link(self.kind())],
            Some(TestField::Plain) | None => Vec::new(),
        }
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, id) {
            (FieldKind::Button, BindingId::TextualButtonPress) => {
                let button = self.ring.focused().and_then(TestField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, |pressed| self.pressed(pressed)),
                )
            }
            // `consumes_key` already gave the input every key it binds.
            _ => Some(Action::Consumed),
        }
    }

    fn handle_paste(&mut self, text: &str) -> Option<Action> {
        self.ring.is_focused(TestField::Input).then(|| {
            self.input.handle_paste(text);
            Action::Consumed
        })
    }
}

/// The characters of `buffer` as [`buffer_to_string`] gives them, then each cell's style as a letter, then what each
/// letter stands for: the parity harness's `.screen` layout, for snapshots where colors matter.
pub fn buffer_to_styled_string(buffer: &Buffer) -> String {
    const LETTERS: &str = ".abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let area = buffer.area;
    let mut legend: Vec<String> = Vec::new();
    let mut grid = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &buffer[(x, y)];
            let style = describe_style(cell.fg, cell.bg, cell.modifier);
            let index = legend.iter().position(|seen| *seen == style).unwrap_or_else(|| {
                legend.push(style);
                legend.len() - 1
            });
            grid.push(LETTERS.chars().nth(index).unwrap_or('?'));
        }
        grid.push('\n');
    }
    let mut out = buffer_to_string(buffer);
    out.push_str("--- styles\n");
    out.push_str(&grid);
    out.push_str("--- legend\n");
    out.extend(
        LETTERS
            .chars()
            .zip(&legend)
            .map(|(letter, style)| format!("{letter} {style}\n")),
    );
    out
}

fn describe_style(fg: Color, bg: Color, modifier: Modifier) -> String {
    let color = |color: Color| match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}"),
    };
    let mut parts = vec![format!("fg={}", color(fg)), format!("bg={}", color(bg))];
    if !modifier.is_empty() {
        parts.push(format!("{modifier:?}"));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_requests_are_answered_from_the_local_repo() {
        let repo = RepoName::from_remote_url("https://github.com/o/n.git");
        assert!(repo.is_some());
        let app = TestApp::new(Config::default());
        assert_eq!(app.answer_git(&GitRequest::OriginRepo), GitResponse::OriginRepo(None));
        let app = app.with_local_repo(repo.clone());
        assert_eq!(app.answer_git(&GitRequest::OriginRepo), GitResponse::OriginRepo(repo));
    }

    #[test]
    fn settle_answers_queued_git_requests_and_cancel_drops_them() {
        let mut app = TestApp::new(Config::default()).with_local_repo(RepoName::from_remote_url("git@h:o/n"));
        let (kept, cancelled) = app.with_cx(|cx| {
            let kept = cx.git(GitRequest::OriginRepo);
            let cancelled = cx.git(GitRequest::OriginRepo);
            cx.effect(Effect::Cancel(cancelled));
            (kept, cancelled)
        });
        let queued = |app: &TestApp, wanted: RequestId| {
            app.queued
                .iter()
                .any(|effect| matches!(effect, Effect::Git { id, .. } if *id == wanted))
        };
        assert!(queued(&app, kept));
        assert!(!queued(&app, cancelled));
        futures_util::FutureExt::now_or_never(app.settle()).unwrap();
        assert!(app.queued.is_empty());
    }
}
