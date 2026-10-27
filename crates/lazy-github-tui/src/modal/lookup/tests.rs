//! The lookup dialog through the repository kind: validation, submitting, the answer, tracking, the request in
//! flight, focus, and its drawing.

use lazy_github_core::config::Config;
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::User;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use rstest::rstest;

use super::*;
use crate::effect::Effect;
use crate::event::Event;
use crate::keymap::Keymap;
use crate::modal::DialogFrame;
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::theme::Theme;

fn repo(full_name: &str) -> Repository {
    let (owner, name) = full_name.split_once('/').unwrap();
    Repository {
        name: name.into(),
        full_name: full_name.into(),
        default_branch: Some("main".into()),
        private: false,
        archived: Some(false),
        owner: User {
            login: owner.into(),
            id: 1,
            name: None,
            html_url: format!("https://github.com/{owner}"),
        },
        description: None,
        permissions: None,
    }
}

/// The table's repos, and the lookups' answers: `o/found` exists, `o/missing` is a 404 and `o/broken` fails.
fn github() -> FakeGithub {
    FakeGithub::new()
        .with_repositories(vec![repo("a/one"), repo("b/two"), repo("o/found")])
        .with_repository("o/found", Ok(Some(repo("o/found"))))
        .with_repository("o/new", Ok(Some(repo("o/new"))))
        .with_repository("o/missing", Ok(None))
        .with_repository("o/broken", Err(GhError::NotInstalled))
}

/// The main screen with `O` pressed on the repos table.
fn opened_with(config: Config) -> TestApp {
    let mut app = TestApp::with_github(config, github());
    app.press(&["O"]);
    assert!(is_open(&app), "O opens the dialog");
    app
}

fn opened() -> TestApp {
    opened_with(Config::default())
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::Lookup(LookupTarget::Repository)]
}

/// Types `text` into the focused input, a key at a time.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = match c {
            ' ' => "space".to_owned(),
            '/' => "slash".to_owned(),
            '-' => "minus".to_owned(),
            c => c.to_string(),
        };
        app.press(&[key.as_str()]);
    }
}

/// `(title, message)` of each toast, oldest first.
fn toasts(app: &TestApp) -> Vec<(Option<String>, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.title.clone(), toast.message.to_string()))
        .collect()
}

fn validation_error(message: &str) -> (Option<String>, String) {
    (Some("Validation Error".into()), message.into())
}

/// The repository lookups the app asked for, by name.
fn lookups(app: &TestApp) -> Vec<String> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::Repository(name),
                ..
            } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The id of the last repository lookup.
fn lookup_id(app: &TestApp) -> RequestId {
    app.effects()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::Repository(_),
            } => Some(*id),
            _ => None,
        })
        .expect("a lookup was sent")
}

fn tracked(app: &TestApp) -> Vec<String> {
    app.app().config().repositories.additional_repos_to_track.clone()
}

fn table_keys(app: &TestApp) -> Vec<String> {
    app.app()
        .main_screen()
        .unwrap()
        .repos()
        .table()
        .row_keys()
        .cloned()
        .collect()
}

fn cursor(app: &TestApp) -> usize {
    app.app().main_screen().unwrap().repos().table().cursor()
}

/// The color of the input's top border at 160x48: the dialog's interior starts at (51, 12) and the input's top row
/// is its sixth.
fn input_border(app: &mut TestApp) -> Color {
    app.render(160, 48)[(60, 17)].fg
}

const FOCUSED_BORDER: Color = Color::Rgb(0x01, 0x78, 0xd4);
const BLURRED_BORDER: Color = Color::Rgb(0x19, 0x19, 0x19);
const INVALID_FOCUSED: Color = Color::Rgb(0xb9, 0x3c, 0x5b);
const INVALID_BLURRED: Color = Color::Rgb(0x8e, 0x43, 0x56);

// Validation.

#[rstest]
#[case("a/b", true)]
#[case("a b/c d", true)]
#[case("parity-org/lookup-me", true)]
#[case("", false)]
#[case("a", false)]
#[case("/b", false)]
#[case("a/", false)]
#[case("a/b/c", false)]
#[case("/", false)]
fn repo_names_are_owner_slash_name(#[case] text: &str, #[case] valid: bool) {
    assert_eq!(RepoLookup::is_valid(text), valid, "{text:?}");
}

#[test]
fn the_border_is_normal_until_a_check() {
    let mut app = opened();
    assert_eq!(input_border(&mut app), FOCUSED_BORDER);
    // A cursor key changes nothing, so it checks nothing.
    app.press(&["left", "home"]);
    assert_eq!(input_border(&mut app), FOCUSED_BORDER);
}

#[test]
fn each_change_checks_the_input() {
    let mut app = opened();
    app.press(&["a"]);
    assert_eq!(input_border(&mut app), INVALID_FOCUSED);
    type_text(&mut app, "/b");
    assert_eq!(input_border(&mut app), FOCUSED_BORDER);
    app.press(&["backspace"]);
    assert_eq!(input_border(&mut app), INVALID_FOCUSED);
}

#[test]
fn blurring_checks_the_input() {
    let mut app = opened();
    app.press(&["tab"]);
    assert_eq!(input_border(&mut app), INVALID_BLURRED);
    // Back on it, still red until a check passes.
    app.press(&["shift+tab"]);
    assert_eq!(input_border(&mut app), INVALID_FOCUSED);
    type_text(&mut app, "a/b");
    app.press(&["tab"]);
    assert_eq!(input_border(&mut app), BLURRED_BORDER);
}

#[test]
fn enter_in_the_input_checks_it_and_doesnt_look_up() {
    let mut app = opened();
    app.press(&["enter"]);
    assert_eq!(input_border(&mut app), INVALID_FOCUSED);
    type_text(&mut app, "o/found");
    app.press(&["enter"]);
    assert_eq!(input_border(&mut app), FOCUSED_BORDER);
    assert!(lookups(&app).is_empty());
    assert!(is_open(&app));
}

#[test]
fn focusing_the_input_selects_its_text() {
    let mut app = opened();
    type_text(&mut app, "nope");
    app.press(&["tab", "shift+tab"]);
    type_text(&mut app, "o/found");
    app.press(&["shift+enter"]);
    assert_eq!(lookups(&app), ["o/found"]);
}

// Submitting.

#[test]
fn an_invalid_submit_toasts_and_sends_nothing() {
    for text in ["", "nope", "a/b/c"] {
        let mut app = opened();
        type_text(&mut app, text);
        app.press(&["shift+enter"]);
        assert!(lookups(&app).is_empty(), "{text:?}");
        assert_eq!(
            toasts(&app),
            [validation_error("You must enter a repo name!")],
            "{text:?}"
        );
        assert!(is_open(&app), "{text:?}");
    }
}

#[test]
fn the_lookup_button_and_modal_submit_look_up() {
    for keys in [&["shift+enter"][..], &["tab", "tab", "enter"]] {
        let mut app = opened();
        type_text(&mut app, "o/found");
        app.press(keys);
        assert_eq!(lookups(&app), ["o/found"], "{keys:?}");
    }
}

#[tokio::test]
async fn not_found_toasts_and_keeps_the_text() {
    for name in ["o/missing", "o/broken"] {
        let mut app = opened();
        type_text(&mut app, name);
        app.press(&["shift+enter"]);
        app.settle().await;
        assert_eq!(toasts(&app), [validation_error("Could not find repo!")], "{name}");
        assert!(is_open(&app), "{name}");
        assert!(app.screen(160, 48).contains(name), "{name}");
        // It can look up again.
        app.press(&["shift+enter"]);
        assert_eq!(lookups(&app), [name, name]);
    }
}

#[tokio::test]
async fn a_found_repo_is_added_and_selected() {
    let mut app = opened();
    let before = cursor(&app);
    type_text(&mut app, "o/new");
    app.press(&["shift+enter"]);
    app.settle().await;
    assert!(app.app().modals().is_empty());
    assert!(toasts(&app).is_empty());
    assert_eq!(app.app().current_repo().map(|r| r.full_name.as_str()), Some("o/new"));
    assert!(table_keys(&app).contains(&"o/new".to_owned()));
    let main = app.app().main_screen().unwrap();
    assert_eq!(main.focus(), FocusTarget::Table(SectionId::Repos));
    assert_eq!(cursor(&app), before);
    // The table cache has it too.
    assert!(
        app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::WriteCache { text, .. } if text.contains("o/new")))
    );
    // Without the switch, nothing is tracked.
    assert!(tracked(&app).is_empty());
    assert!(
        !app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::SaveConfig(_)))
    );
}

#[tokio::test]
async fn a_repo_already_in_the_table_is_replaced() {
    let mut app = opened();
    let rows = table_keys(&app).len();
    type_text(&mut app, "o/found");
    app.press(&["shift+enter"]);
    app.settle().await;
    assert_eq!(table_keys(&app).len(), rows);
    assert_eq!(app.app().current_repo().map(|r| r.full_name.as_str()), Some("o/found"));
}

#[tokio::test]
async fn closing_gives_nothing() {
    for keys in [&["escape"][..], &["tab", "q"], &["tab", "tab", "tab", "enter"]] {
        let mut app = opened();
        app.press(keys);
        assert!(app.app().modals().is_empty(), "{keys:?}");
        app.settle().await;
        assert_eq!(app.app().current_repo(), None, "{keys:?}");
        assert!(lookups(&app).is_empty());
    }
}

// Tracking.

#[tokio::test]
async fn tracking_appends_once_and_only_on_success() {
    let mut app = opened();
    // The switch on, a repo that isn't there: nothing is tracked.
    app.press(&["tab", "space", "shift+tab"]);
    type_text(&mut app, "o/missing");
    app.press(&["shift+enter"]);
    app.settle().await;
    assert!(tracked(&app).is_empty());
    assert!(
        !app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::SaveConfig(_)))
    );
    // One that is: tracked as typed, and saved. Back onto the input selects its text, so typing replaces it.
    app.press(&["tab", "shift+tab"]);
    type_text(&mut app, "o/new");
    app.press(&["shift+enter"]);
    app.settle().await;
    assert!(app.app().modals().is_empty());
    assert_eq!(tracked(&app), ["o/new"]);
    assert!(
        app.effects()
            .iter()
            .any(|effect| matches!(effect, Effect::SaveConfig(_)))
    );
    // Again: not twice.
    app.press(&["O", "tab", "space", "shift+tab"]);
    type_text(&mut app, "o/new");
    app.press(&["shift+enter"]);
    app.settle().await;
    assert_eq!(tracked(&app), ["o/new"]);
}

// In flight (Py bug B51).

#[tokio::test]
async fn closing_in_flight_cancels_and_a_late_answer_does_nothing() {
    let mut app = opened();
    type_text(&mut app, "o/new");
    app.press(&["tab", "space", "shift+tab", "shift+enter"]);
    let id = lookup_id(&app);
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(app.effects().contains(&Effect::Cancel(id)));
    app.settle().await;
    app.send(Event::Github {
        id,
        response: GithubResponse::Repository(Ok(Some(Box::new(repo("o/new"))))),
    });
    assert_eq!(app.app().current_repo(), None);
    assert!(!table_keys(&app).contains(&"o/new".to_owned()));
    assert!(tracked(&app).is_empty());
}

#[tokio::test]
async fn a_second_submit_in_flight_is_ignored_and_typing_works() {
    let mut app = opened();
    type_text(&mut app, "o/new");
    app.press(&["shift+enter", "shift+enter", "tab", "tab", "enter"]);
    assert_eq!(lookups(&app), ["o/new"]);
    app.press(&["shift+tab", "shift+tab"]);
    type_text(&mut app, "x");
    // Focusing the input selected its text, so the x replaced it.
    assert!(app.screen(160, 48).contains("█▊  x "));
    app.settle().await;
    // The answer is for the text it was sent for.
    assert_eq!(app.app().current_repo().map(|r| r.full_name.as_str()), Some("o/new"));
}

// Focus.

#[derive(Debug)]
enum NoSwitch {}

impl LookupKind for NoSwitch {
    type Found = String;
    const TARGET: LookupTarget = LookupTarget::Issue;
    const HEADING: &'static str = "Search for an issue by number:";
    const LABEL: &'static str = "Issue Number:";
    const PLACEHOLDER: &'static str = "Issue number";

    fn is_valid(text: &str) -> bool {
        text.parse::<u64>().is_ok()
    }

    fn request(_text: &str, _cx: &Cx<'_>) -> Result<GithubRequest, Toast> {
        Err(Toast::error("unused"))
    }

    fn found(response: GithubResponse) -> Result<Result<String, Toast>, GithubResponse> {
        Err(response)
    }
}

fn tab_order<K: LookupKind>(dialog: &mut LookupDialog<K>, direction: FocusMove) -> Vec<LookupField> {
    (0..5)
        .map(|_| {
            dialog.move_focus(direction);
            dialog.ring.focused().unwrap()
        })
        .collect()
}

#[test]
fn tab_walks_the_fields_with_the_switch() {
    use LookupField::*;
    let mut dialog = LookupDialog::<RepoLookup>::new();
    assert_eq!(dialog.ring.focused(), Some(Input));
    assert_eq!(
        tab_order(&mut dialog, FocusMove::Next),
        [Track, Submit, Cancel, Input, Track]
    );
    assert_eq!(
        tab_order(&mut dialog, FocusMove::Previous),
        [Input, Cancel, Submit, Track, Input]
    );
}

#[test]
fn tab_walks_the_fields_without_the_switch() {
    use LookupField::*;
    let mut dialog = LookupDialog::<NoSwitch>::new();
    assert_eq!(dialog.ring.focused(), Some(Input));
    assert_eq!(
        tab_order(&mut dialog, FocusMove::Next),
        [Submit, Cancel, Input, Submit, Cancel]
    );
    assert_eq!(
        tab_order(&mut dialog, FocusMove::Previous),
        [Submit, Input, Cancel, Submit, Input]
    );
}

#[test]
fn the_footer_shows_submit_and_close() {
    let mut app = opened();
    let footer = |app: &mut TestApp| -> Vec<String> {
        crate::components::footer::footer_items(&app.app_mut().active_bindings())
            .iter()
            .map(|item| format!("{} {}", item.key, item.description))
            .collect()
    };
    assert!(footer(&mut app).contains(&"esc Close".to_owned()));
    app.press(&["tab"]);
    assert!(footer(&mut app).contains(&"q Close".to_owned()));
}

// Drawing.

/// The dialog on its own, framed, at its 60 x 25.
fn draw<K: LookupKind>(dialog: &mut LookupDialog<K>) -> String {
    let theme = Theme::default();
    let keymap = Keymap::defaults();
    let mut terminal = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let background = dialog.background(theme.palette);
            frame.render_widget(
                DialogFrame {
                    theme: &theme,
                    background,
                },
                area,
            );
            let cx = RenderCx {
                theme: &theme,
                keymap: &keymap,
                now: tokio::time::Instant::now(),
                focused: true,
                focus_within: true,
            };
            dialog.render(frame, DialogFrame::interior(area), &cx);
        })
        .unwrap();
    buffer_to_styled_string(terminal.backend().buffer())
}

fn typed(text: &str) -> LookupDialog<RepoLookup> {
    let mut dialog = LookupDialog::<RepoLookup>::new();
    let keymap = Keymap::defaults();
    for c in text.chars() {
        let chord = KeyChord::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
        let _ = dialog.edit(|input| input.handle_key(&chord, &keymap));
    }
    dialog
}

#[test]
fn lookup_open() {
    insta::assert_snapshot!(draw(&mut LookupDialog::<RepoLookup>::new()));
}

#[test]
fn lookup_invalid_focused() {
    insta::assert_snapshot!(draw(&mut typed("nope")));
}

#[test]
fn lookup_invalid_blurred() {
    let mut dialog = typed("nope");
    dialog.move_focus(FocusMove::Next);
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn lookup_switch_on() {
    let mut dialog = LookupDialog::<RepoLookup>::new();
    dialog.move_focus(FocusMove::Next);
    if let Some(track) = &mut dialog.track {
        track.toggle();
    }
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn lookup_submit_focused() {
    let mut dialog = typed("parity-org/lookup-me");
    dialog.move_focus(FocusMove::Next);
    dialog.move_focus(FocusMove::Next);
    insta::assert_snapshot!(draw(&mut dialog));
}

#[test]
fn lookup_without_the_switch_sits_lower() {
    let screen = draw(&mut LookupDialog::<NoSwitch>::new());
    let rows: Vec<&str> = screen.lines().collect();
    let heading = rows
        .iter()
        .position(|row| row.contains("Search for an issue by number:"))
        .unwrap();
    // Python's issue lookup: the heading on the interior's sixth row, the frame's seventh.
    assert_eq!(heading, 6);
    let buttons = rows.iter().position(|row| row.contains("Lookup")).unwrap();
    assert_eq!(buttons, 17);
}
