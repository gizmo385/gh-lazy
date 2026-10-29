//! The command palette: its commands, search order, keys, mouse, the "No matches found" timer and its layout.

use std::time::Duration;

use lazy_github_core::config::Config;

use super::*;
use crate::screens::main::{FocusTarget, MainScreen, SectionId};
use crate::test_util::{TestApp, TestDialog, buffer_to_styled_string};

/// The open palette, if it's on top.
fn palette(app: &TestApp) -> Option<&CommandPaletteDialog> {
    app.app().top_dialog::<CommandPaletteDialog>()
}

fn hits(app: &TestApp) -> Vec<&'static str> {
    palette(app).map(CommandPaletteDialog::hit_names).unwrap_or_default()
}

fn highlighted(app: &TestApp) -> Option<&'static str> {
    palette(app).and_then(CommandPaletteDialog::highlighted)
}

fn help_texts(app: &TestApp) -> Vec<&'static str> {
    let palette = palette(app).expect("the palette is open");
    palette
        .hits
        .iter()
        .map(|hit| palette.commands[hit.command].help)
        .collect()
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::CommandPalette]
}

/// Types `text` into the palette, a key per char.
fn type_text(app: &mut TestApp, text: &str) {
    let keys: Vec<String> = text
        .chars()
        .map(|c| if c == ' ' { "space".into() } else { c.to_string() })
        .collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    app.press(&keys);
}

/// An app on the main screen with the palette open.
fn opened() -> TestApp {
    let mut app = TestApp::new(Config::default());
    app.press(&["ctrl+p"]);
    assert!(is_open(&app));
    app
}

fn command(name: &'static str, source: CommandSource) -> PaletteCommand {
    PaletteCommand {
        name,
        help: "Help",
        source,
        run: PaletteRun::Quit,
    }
}

/// An app with a palette of `commands` open, whose reply re-opens the real palette when a command runs: so a run
/// that happened after the first palette closed shows as a palette on top again.
fn with_commands(commands: Vec<PaletteCommand>) -> TestApp {
    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| {
        cx.open_dialog(CommandPaletteDialog::new(commands), |run| {
            if run.is_some() {
                Action::OpenCommandPalette
            } else {
                Action::Consumed
            }
        });
    });
    app
}

/// Forty commands named "Command 01" to "Command 40", from LazyGithub.
fn many() -> Vec<PaletteCommand> {
    let names: Vec<&'static str> = (1..=40)
        .map(|n| &*Box::leak(format!("Command {n:02}").into_boxed_str()))
        .collect();
    names
        .into_iter()
        .map(|name| command(name, CommandSource::App))
        .collect()
}

/// LazyGithub's toggles, as the opening list ends with them.
const TOGGLES: [&str; 4] = [
    "Toggle Command Log",
    "Toggle Workflows",
    "Toggle Issues",
    "Toggle Pull Requests",
];

/// The opening list: `system` sorted by name, then [`TOGGLES`] (Py bug B87's answer).
fn opening(system: &[&'static str]) -> Vec<&'static str> {
    system.iter().copied().chain(TOGGLES).collect()
}

fn styled(app: &mut TestApp) -> String {
    buffer_to_styled_string(&app.render(160, 48))
}

// Opening.

#[test]
fn ctrl_p_opens_the_palette_with_the_input_focused() {
    let mut app = opened();
    assert_eq!(hits(&app), opening(&["Keys", "Maximize", "Quit"]));
    assert_eq!(highlighted(&app), Some("Keys"));
    // The input takes printable keys, `q` and `?` included.
    type_text(&mut app, "q?1");
    assert!(is_open(&app));
    assert_eq!(
        palette(&app).map(|p| p.input.value().to_owned()).as_deref(),
        Some("q?1")
    );
    assert!(!app.app().should_quit());
    assert!(!app.app().help_open());
}

#[test]
fn ctrl_p_does_nothing_under_a_modal() {
    let mut app = opened();
    app.press(&["ctrl+p"]);
    assert_eq!(app.app().modals(), [ModalKind::CommandPalette]);

    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| cx.open_dialog(TestDialog::new(1), |_| Action::Consumed));
    app.press(&["ctrl+p"]);
    assert_eq!(app.app().modals(), [ModalKind::Test(1)]);
}

#[test]
fn the_palette_has_no_footer_and_tab_does_nothing() {
    let mut app = opened();
    let before = app.screen(160, 48);
    app.press(&["tab", "shift+tab"]);
    assert!(is_open(&app));
    assert_eq!(app.screen(160, 48), before);
    // The main screen's footer stays on the bottom row.
    assert!(before.lines().last().is_some_and(|row| row.contains("q Quit")));
}

// Availability.

#[test]
fn keys_says_whether_it_shows_or_hides_the_help_panel() {
    let app = opened();
    assert_eq!(
        help_texts(&app)[0],
        "Show help for the focused widget and a summary of available keys"
    );
    let mut app = TestApp::new(Config::default());
    app.press(&["?", "ctrl+p"]);
    assert_eq!(hits(&app)[0], "Keys");
    assert_eq!(help_texts(&app)[0], "Hide the keys and widget help panel");
}

#[test]
fn maximize_is_offered_when_m_would_maximize_and_minimize_while_maximized() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M", "ctrl+p"]);
    assert_eq!(hits(&app), opening(&["Keys", "Minimize", "Quit"]));
    assert_eq!(help_texts(&app)[1], "Minimize the widget and restore to normal size");

    // `M` does nothing in the details content, so neither is offered.
    let mut app = TestApp::new(Config::default());
    app.dispatch(Action::Focus(FocusTarget::DetailsContent));
    assert_eq!(
        app.app().main_screen().map(MainScreen::focus),
        Some(FocusTarget::DetailsContent)
    );
    app.press(&["ctrl+p"]);
    assert_eq!(hits(&app), opening(&["Keys", "Quit"]));
}

#[test]
fn lazygithubs_commands_are_in_their_order_before_the_system_ones() {
    let app = opened();
    let palette = palette(&app).expect("the palette is open");
    let names: Vec<_> = palette.commands.iter().map(|c| (c.name, c.source)).collect();
    assert_eq!(
        names,
        [
            ("Toggle Command Log", CommandSource::App),
            ("Toggle Workflows", CommandSource::App),
            ("Toggle Issues", CommandSource::App),
            ("Toggle Pull Requests", CommandSource::App),
            ("Quit", CommandSource::System),
            ("Keys", CommandSource::System),
            ("Maximize", CommandSource::System),
        ]
    );
    let helps: Vec<_> = palette.commands[..4].iter().map(|c| c.help).collect();
    assert_eq!(
        helps,
        [
            "Toggle showing or hiding the command log",
            "Toggle showing or hiding repo actions",
            "Toggle showing or hiding repo issues",
            "Toggle showing or hiding repo pull requests",
        ]
    );
}

#[test]
fn tog_lists_the_four_toggles_and_lazygithubs_win_ties() {
    let mut app = opened();
    type_text(&mut app, "tog");
    assert_eq!(hits(&app), TOGGLES);
    // Every query's hits are best first, and on equal scores LazyGithub's come first (Py bug B86's answer).
    for query in ["t", "i", "o", "e", "s", "m"] {
        app.press(&["ctrl+u"]);
        type_text(&mut app, query);
        let palette = palette(&app).expect("the palette is open");
        let order: Vec<(f64, bool)> = palette
            .hits
            .iter()
            .map(|hit| {
                let command = &palette.commands[hit.command];
                let score = fuzzy_match(query, command.name).expect("a hit matches").score;
                (score, command.source == CommandSource::System)
            })
            .collect();
        assert!(
            order
                .windows(2)
                .all(|w| w[0].0.total_cmp(&w[1].0).then(w[1].1.cmp(&w[0].1)).is_ge()),
            "{query}: {order:?}"
        );
    }
}

#[test]
fn refresh_notifications_is_listed_only_while_notifications_are_on() {
    let mut app = opened();
    type_text(&mut app, "ref");
    assert!(hits(&app).is_empty());
    app.advance(Duration::from_secs(1));
    assert!(palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));

    let mut config = Config::default();
    config.notifications.enabled = true;
    let mut app = TestApp::new(config);
    futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once");
    app.press(&["ctrl+p"]);
    // Last among LazyGithub's, after the system commands in the opening list.
    assert_eq!(hits(&app).last(), Some(&"Refresh notifications"));
    type_text(&mut app, "ref");
    assert_eq!(hits(&app), ["Refresh notifications"]);
    assert_eq!(help_texts(&app), ["Refresh the unread notifications count"]);
    let checks = |app: &TestApp| {
        app.effects()
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::Github {
                        request: crate::effect::GithubRequest::CheckAuth,
                        ..
                    }
                )
            })
            .count()
    };
    let before = checks(&app);
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(checks(&app), before + 1);
    assert!(app.app().notifications_refreshing());
}

#[test]
fn a_toggle_runs_on_the_main_screen() {
    let mut app = opened();
    type_text(&mut app, "toggle issues");
    assert_eq!(highlighted(&app), Some("Toggle Issues"));
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());
    let main = app.app().main_screen().expect("on the main screen");
    assert!(!main.section(SectionId::Issues).shown());
    app.press(&["ctrl+p"]);
    type_text(&mut app, "command log");
    app.press(&["enter"]);
    assert!(
        app.app()
            .main_screen()
            .is_some_and(|main| main.command_log().is_visible())
    );
}

// Search.

#[test]
fn the_empty_query_lists_the_system_commands_by_name_then_lazygithubs() {
    // Py bug B87's answer: LazyGithub's commands too, after the system ones, in their own order.
    let app = with_commands(vec![
        command("Toggle Issues", CommandSource::App),
        command("Change Settings", CommandSource::App),
        command("Theme", CommandSource::System),
        command("Quit", CommandSource::System),
        command("Keys", CommandSource::System),
    ]);
    assert_eq!(
        hits(&app),
        ["Keys", "Quit", "Theme", "Toggle Issues", "Change Settings"]
    );
    assert_eq!(highlighted(&app), Some("Keys"));
}

#[test]
fn hits_are_best_first_and_ties_keep_the_commands_order() {
    // Py bug B86's answer. `t` scores 6.0 on "Theme" and the toggles, 3.0 on "Quit" and "Change Settings".
    let commands = vec![
        command("Toggle Command Log", CommandSource::App),
        command("Toggle Workflows", CommandSource::App),
        command("Change Settings", CommandSource::App),
        command("Theme", CommandSource::System),
        command("Quit", CommandSource::System),
        command("Keys", CommandSource::System),
    ];
    let mut app = with_commands(commands);
    type_text(&mut app, "t");
    assert_eq!(
        hits(&app),
        [
            "Toggle Command Log",
            "Toggle Workflows",
            "Theme",
            "Change Settings",
            "Quit"
        ]
    );
    assert_eq!(highlighted(&app), Some("Toggle Command Log"));
    type_text(&mut app, "heme");
    assert_eq!(hits(&app), ["Theme"]);
}

#[test]
fn the_query_is_stripped_and_a_blank_one_lists_everything() {
    let mut app = opened();
    type_text(&mut app, "  keys ");
    assert_eq!(hits(&app), ["Keys"]);
    app.press(&["ctrl+u"]);
    type_text(&mut app, "   ");
    assert_eq!(hits(&app), opening(&["Keys", "Maximize", "Quit"]));
    // A blank query never says it found nothing.
    app.advance(Duration::from_secs(1));
    assert!(!palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
}

#[test]
fn matched_letters_are_underlined() {
    let mut app = opened();
    type_text(&mut app, "qt");
    let buffer = app.render(160, 48);
    // "Quit" on row 8, from column 2: Q and t matched.
    let underlined: Vec<bool> = (2..6)
        .map(|x| buffer[(x, 8)].modifier.contains(Modifier::UNDERLINED))
        .collect();
    assert_eq!(underlined, [true, false, false, true]);
    assert!(buffer[(2, 8)].modifier.contains(Modifier::BOLD));
}

// No matches.

#[test]
fn no_matches_shows_half_a_second_after_a_search_that_found_nothing() {
    let mut app = opened();
    type_text(&mut app, "zzz");
    assert!(hits(&app).is_empty());
    app.advance(Duration::from_millis(499));
    assert!(!palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
    app.advance(Duration::from_millis(1));
    assert!(palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
    // Any edit hides it again, and it comes back only for a query that still finds nothing.
    type_text(&mut app, "z");
    assert!(!palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
    app.advance(Duration::from_millis(300));
    type_text(&mut app, "z");
    app.advance(Duration::from_millis(300));
    assert!(!palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
    app.advance(Duration::from_millis(200));
    assert!(palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
}

#[test]
fn no_matches_isnt_shown_once_the_query_finds_something_again() {
    let mut app = opened();
    type_text(&mut app, "kz");
    app.press(&["backspace"]);
    assert_eq!(hits(&app), ["Keys", "Toggle Workflows"]);
    app.advance(Duration::from_secs(1));
    assert!(!palette(&app).is_some_and(CommandPaletteDialog::shows_no_matches));
    assert!(app.effects().contains(&Effect::CancelTimer(TimerId::PaletteNoMatches)));
}

#[test]
fn closing_drops_a_pending_no_matches() {
    let mut app = opened();
    type_text(&mut app, "zzz");
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(app.effects().contains(&Effect::CancelTimer(TimerId::PaletteNoMatches)));
}

// Keys.

#[test]
fn up_and_down_wrap() {
    let mut app = opened();
    app.press(&["down"]);
    assert_eq!(highlighted(&app), Some("Maximize"));
    app.press(&["down"; 6]);
    assert_eq!(highlighted(&app), Some("Keys"));
    app.press(&["up"]);
    assert_eq!(highlighted(&app), Some("Toggle Pull Requests"));
}

#[test]
fn page_keys_move_by_the_rows_in_view_and_stop_at_the_ends() {
    let mut app = with_commands(many());
    // 31 rows in view at 160x48: a page down is 15 hits.
    app.render(160, 48);
    app.press(&["pagedown"]);
    assert_eq!(highlighted(&app), Some("Command 16"));
    app.press(&["pagedown", "pagedown"]);
    assert_eq!(highlighted(&app), Some("Command 40"));
    app.press(&["pageup"]);
    assert_eq!(highlighted(&app), Some("Command 24"));
    app.press(&["pageup", "pageup"]);
    assert_eq!(highlighted(&app), Some("Command 01"));
}

#[test]
fn ctrl_home_and_ctrl_end_go_to_the_ends_and_shift_selects_instead() {
    let mut app = with_commands(many());
    app.render(160, 48);
    app.press(&["ctrl+end"]);
    assert_eq!(highlighted(&app), Some("Command 40"));
    app.press(&["ctrl+home"]);
    assert_eq!(highlighted(&app), Some("Command 01"));
    // The input takes `shift+end` and `shift+home` first.
    type_text(&mut app, "c");
    app.press(&["shift+home"]);
    assert_eq!(highlighted(&app), Some("Command 01"));
    assert_eq!(palette(&app).and_then(|p| p.input.selection()), Some((0, 1)));
}

#[test]
fn the_list_scrolls_to_keep_the_highlight_in_view() {
    let mut app = with_commands(many());
    app.render(160, 48);
    app.press(&["ctrl+end"]);
    let palette = palette(&app).expect("open");
    // 80 rows, 31 in view: the last hit's two rows at the bottom.
    assert_eq!(palette.scroll, 49);
    app.press(&["down"]);
    assert_eq!(highlighted(&app), Some("Command 01"));
    assert_eq!(self::palette(&app).map(|p| p.scroll), Some(0));
}

#[test]
fn enter_runs_the_highlighted_hit_after_the_palette_closes() {
    let mut app = opened();
    type_text(&mut app, "keys");
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());
    assert!(app.app().help_open());
    // Focus is where it was.
    assert_eq!(
        app.app().main_screen().map(MainScreen::focus),
        Some(FocusTarget::Table(SectionId::Repos))
    );

    // The reply runs once the stack has closed the palette: here it opens the real palette, which only opens when
    // no dialog is open.
    let mut app = with_commands(many());
    app.press(&["down", "enter"]);
    assert_eq!(app.app().modals(), [ModalKind::CommandPalette]);
    assert_eq!(hits(&app), opening(&["Keys", "Maximize", "Quit"]));
}

#[test]
fn enter_with_no_hits_runs_nothing_and_escape_closes() {
    let mut app = opened();
    type_text(&mut app, "zzz");
    app.press(&["enter"]);
    assert!(is_open(&app));
    app.advance(Duration::from_secs(1));
    app.press(&["enter"]);
    assert!(is_open(&app));
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(!app.app().help_open());
    assert!(!app.app().should_quit());
    assert_eq!(
        app.app().main_screen().map(MainScreen::focus),
        Some(FocusTarget::Table(SectionId::Repos))
    );
}

#[test]
fn quit_quits() {
    let mut app = opened();
    type_text(&mut app, "quit");
    app.press(&["enter"]);
    assert!(app.app().should_quit());
}

#[test]
fn maximize_and_minimize_run_as_m_and_escape_without_the_toast() {
    let mut app = opened();
    type_text(&mut app, "max");
    app.press(&["enter"]);
    assert!(app.app().maximized().is_some());
    assert!(
        app.app()
            .toasts()
            .iter()
            .all(|toast| toast.title.as_deref() != Some("View Maximized"))
    );
    app.press(&["ctrl+p"]);
    type_text(&mut app, "min");
    app.press(&["enter"]);
    assert!(app.app().maximized().is_none());
    assert_eq!(
        app.app().main_screen().map(MainScreen::focus),
        Some(FocusTarget::Table(SectionId::Repos))
    );
    // The one-time toast is still there for the next `M`.
    app.press(&["M"]);
    assert!(
        app.app()
            .toasts()
            .iter()
            .any(|toast| toast.title.as_deref() == Some("View Maximized"))
    );
}

#[test]
fn ctrl_q_still_quits() {
    let mut app = opened();
    app.press(&["ctrl+q"]);
    assert!(app.app().should_quit());
}

// Mouse.

#[test]
fn a_click_on_a_hit_runs_it() {
    let mut app = opened();
    app.render(160, 48);
    // "Keys" is on rows 8 and 9; its help row runs it too.
    app.click(40, 9);
    assert!(app.app().modals().is_empty());
    assert!(app.app().help_open());
}

#[test]
fn a_click_outside_the_band_closes_the_palette() {
    // The dimmed screen below the band, the status row above it, and the footer row.
    for (col, row) in [(40, 30), (5, 1), (3, 47)] {
        let mut app = opened();
        app.render(160, 48);
        app.click(col, row);
        assert!(app.app().modals().is_empty(), "a click at {col}, {row}");
        assert!(!app.app().help_open());
    }
    // Inside the band, off the hits, it stays open.
    let mut app = opened();
    app.render(160, 48);
    app.click(40, 3);
    app.click(40, 7);
    assert!(is_open(&app));
}

#[test]
fn a_click_in_the_input_moves_its_cursor() {
    let mut app = opened();
    type_text(&mut app, "keys");
    app.render(160, 48);
    app.click(6, 5);
    assert_eq!(palette(&app).map(|p| p.input.cursor()), Some(1));
    assert!(is_open(&app));
}

#[test]
fn the_wheel_scrolls_the_list() {
    let mut app = with_commands(many());
    app.render(160, 48);
    app.mouse(crossterm::event::MouseEventKind::ScrollDown, 40, 20);
    assert_eq!(palette(&app).map(|p| p.scroll), Some(WHEEL_ROWS));
    assert_eq!(highlighted(&app), Some("Command 01"));
}

// Layout.

#[test]
fn opening_list_160x48() {
    insta::assert_snapshot!(styled(&mut opened()));
}

#[test]
fn query_with_the_second_hit_highlighted_160x48() {
    let mut app = opened();
    type_text(&mut app, "i");
    app.press(&["down"]);
    insta::assert_snapshot!(styled(&mut app));
}

#[test]
fn no_matches_160x48() {
    let mut app = opened();
    type_text(&mut app, "zzz");
    app.advance(Duration::from_secs(1));
    insta::assert_snapshot!(styled(&mut app));
}

#[test]
fn a_tall_list_scrolled_to_the_bottom_160x48() {
    let mut app = with_commands(many());
    app.render(160, 48);
    app.press(&["ctrl+end"]);
    insta::assert_snapshot!(styled(&mut app));
}

#[test]
fn opening_list_in_other_themes() {
    for name in ["textual-light", "ansi-dark"] {
        let theme = crate::theme::Theme::by_name(name).expect("a built-in theme");
        let mut app = TestApp::with_theme(Config::default(), theme);
        app.press(&["ctrl+p"]);
        insta::assert_snapshot!(name, styled(&mut app));
    }
}
