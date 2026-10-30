use lazy_github_core::config::Config;
use lazy_github_core::github::NotificationQuery;
use ratatui::style::Color;

use super::*;
use crate::keymap::Keymap;
use crate::modal::ModalKind;
use crate::test_util::TestApp;
use crate::theme::{ColorDepth, ColorMode, Theme};

fn config(f: impl FnOnce(&mut AppearanceSettings)) -> Config {
    let mut config = Config::default();
    f(&mut config.appearance);
    config
}

fn screen(config: Config, cols: u16, rows: u16) -> String {
    TestApp::new(config).screen(cols, rows)
}

#[test]
fn main_default_160x48() {
    insta::assert_snapshot!(screen(Config::default(), 160, 48));
}

#[test]
fn main_default_100x30() {
    insta::assert_snapshot!(screen(Config::default(), 100, 30));
}

// The left column is 20 cells wide, too narrow for "[1] Repositories" and "[2] Pull Requests" (#149).
#[test]
fn main_default_50x30() {
    insta::assert_snapshot!(screen(Config::default(), 50, 30));
}

#[test]
fn main_command_log_160x48() {
    insta::assert_snapshot!(screen(config(|a| a.show_command_log = true), 160, 48));
}

#[test]
fn main_no_prs_100x30() {
    insta::assert_snapshot!(screen(config(|a| a.show_pull_requests = false), 100, 30));
}

#[test]
fn main_repos_and_prs_100x30() {
    let config = config(|a| {
        a.show_issues = false;
        a.show_workflows = false;
    });
    insta::assert_snapshot!(screen(config, 100, 30));
}

#[test]
fn main_only_repos_100x30() {
    let config = config(|a| {
        a.show_pull_requests = false;
        a.show_issues = false;
        a.show_workflows = false;
    });
    insta::assert_snapshot!(screen(config, 100, 30));
}

#[test]
fn main_overridden_titles_160x48() {
    let mut config = Config::default();
    let overrides = &mut config.bindings.overrides;
    overrides.insert("main.repos.focus".into(), "9".into());
    overrides.insert("main.pull_requests.focus".into(), "x, ctrl+t".into());
    insta::assert_snapshot!(screen(config, 160, 48));
}

#[test]
fn main_styled_100x30() {
    // The focused repos border in `$success`, the other boxes in `$primary-lighten-3`, the status box in
    // `$secondary`, the details tabs and intro, and the footer row on `$footer-background`.
    insta::assert_debug_snapshot!(TestApp::new(Config::default()).render(100, 30));
}

#[test]
fn main_styled_eight_bit_100x30() {
    // The same screen in a 256-color terminal: every color reduced to Rich's xterm-256 index.
    let mode = ColorMode {
        depth: ColorDepth::EightBit,
        no_color: false,
    };
    let theme = Theme::default().with_color_mode(mode);
    let buffer = TestApp::with_theme(Config::default(), theme).render(100, 30);
    let indexed = |c: Color| matches!(c, Color::Indexed(_) | Color::Reset);
    assert!(buffer.content().iter().all(|cell| indexed(cell.fg) && indexed(cell.bg)));
    insta::assert_debug_snapshot!(buffer);
}

#[test]
fn borders_follow_focus() {
    let theme = crate::theme::Theme::default();
    let keymap = Keymap::new(&indexmap::IndexMap::new());
    let mut screen = MainScreen::new(&AppearanceSettings::default(), &keymap, &theme, AppOptions::default());
    let fg = |screen: &mut MainScreen, x: u16, y: u16| {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                let cx = RenderCx {
                    theme: &theme,
                    keymap: &keymap,
                    now: tokio::time::Instant::now(),
                    focused: true,
                    focus_within: true,
                };
                screen.render(frame, frame.area(), &cx);
            })
            .unwrap();
        terminal.backend().buffer()[(x, y)].fg
    };
    let success = Color::Rgb(0x4e, 0xbf, 0x71);
    let unfocused = Color::Rgb(0x6d, 0xb2, 0xff);
    assert_eq!(screen.focus(), FocusTarget::Table(SectionId::Repos));
    assert_eq!([fg(&mut screen, 0, 3), fg(&mut screen, 40, 3)], [success, unfocused]);
    screen.set_focus(FocusTarget::DetailsContent);
    // Details focused: an even split, so PRs start at row 9.
    assert_eq!(
        [fg(&mut screen, 0, 3), fg(&mut screen, 40, 3), fg(&mut screen, 0, 9)],
        [unfocused, success, unfocused]
    );
    screen.set_focus(FocusTarget::Table(SectionId::Workflows));
    assert_eq!(fg(&mut screen, 0, 22), success);
}

// Focus, the Tab ring, clicks and maximize (#75).

use crate::app::Maximized;
use crate::component::{BindingState, Owner};
use crate::effect::Effect;
use crate::toast::{Severity, Toast};

const REPOS: FocusTarget = FocusTarget::Table(SectionId::Repos);
const PRS: FocusTarget = FocusTarget::Table(SectionId::PullRequests);
const ISSUES: FocusTarget = FocusTarget::Table(SectionId::Issues);
const WORKFLOWS: FocusTarget = FocusTarget::Table(SectionId::Workflows);

fn focus(app: &TestApp) -> FocusTarget {
    app.app().main_screen().unwrap().focus()
}

fn with_log() -> Config {
    config(|a| a.show_command_log = true)
}

/// The focused border color in a buffer: is any cell drawn in `$success`?
fn any_focused_border(app: &mut TestApp, cols: u16, rows: u16) -> bool {
    let success = Color::Rgb(0x4e, 0xbf, 0x71);
    app.render(cols, rows).content.iter().any(|cell| cell.fg == success)
}

#[test]
fn each_focus_key_moves_focus() {
    let mut app = TestApp::new(with_log());
    assert_eq!(focus(&app), REPOS);
    for (key, target) in [
        ("2", PRS),
        ("3", ISSUES),
        ("4", WORKFLOWS),
        ("5", FocusTarget::DetailsTabs),
        ("5", FocusTarget::DetailsTabs),
        ("6", FocusTarget::CommandLog),
        ("1", REPOS),
    ] {
        app.press(&[key]);
        assert_eq!(focus(&app), target, "after {key}");
    }
    assert_eq!(app.effects(), []);
}

#[test]
fn six_needs_the_command_log() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "6"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    let mut app = TestApp::new(with_log());
    app.press(&["6"]);
    assert_eq!(focus(&app), FocusTarget::CommandLog);
}

#[test]
fn a_section_hidden_by_config_still_takes_focus() {
    let mut app = TestApp::new(config(|a| a.show_pull_requests = false));
    app.press(&["2"]);
    assert_eq!(focus(&app), PRS);
    assert!(!any_focused_border(&mut app, 100, 30));
    // Its section's bindings are active (Py bug B31).
    let owners: Vec<Owner> = app.app().binding_chain().iter().map(|link| link.owner).collect();
    assert_eq!(owners[2], Owner::Section(SectionId::PullRequests));
    app.press(&["tab"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["2", "shift+tab"]);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn a_section_hidden_by_toggle_ignores_its_key_but_workflows_does_not() {
    let mut app = TestApp::new(Config::default());
    app.dispatch(Action::Toggle(Toggle::PullRequests));
    app.dispatch(Action::Toggle(Toggle::Workflows));
    app.press(&["2"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["4"]);
    assert_eq!(focus(&app), WORKFLOWS);
    // Neither is in the ring.
    let ring = app.app().main_screen().unwrap().focus_ring(None);
    assert_eq!(
        ring,
        [REPOS, ISSUES, FocusTarget::DetailsTabs, FocusTarget::DetailsContent]
    );
}

#[test]
fn tab_walks_the_ring_and_wraps() {
    for (config, ring) in [
        (
            Config::default(),
            vec![
                PRS,
                ISSUES,
                WORKFLOWS,
                FocusTarget::DetailsTabs,
                FocusTarget::DetailsContent,
                REPOS,
            ],
        ),
        (
            with_log(),
            vec![
                PRS,
                ISSUES,
                WORKFLOWS,
                FocusTarget::DetailsTabs,
                FocusTarget::DetailsContent,
                FocusTarget::CommandLog,
                REPOS,
            ],
        ),
    ] {
        let mut app = TestApp::new(config);
        for &expected in &ring {
            app.press(&["tab"]);
            assert_eq!(focus(&app), expected);
        }
        // Backwards from the repos table: the end of the ring, then back to the start.
        let mut backwards: Vec<FocusTarget> = ring.iter().rev().skip(1).copied().collect();
        backwards.push(REPOS);
        for expected in backwards {
            app.press(&["shift+tab"]);
            assert_eq!(focus(&app), expected);
        }
    }
}

#[test]
fn a_click_inside_a_box_focuses_it_and_a_border_click_does_not() {
    let mut app = TestApp::new(with_log());
    app.render(160, 48);
    // The PR section is rows 20-28 with repos focused: its border is column 0 and row 20, and its empty table is
    // the header on row 21. Below the header is the table's container, which takes no focus.
    app.click(0, 21).click(5, 20).click(5, 22).click(5, 27);
    assert_eq!(focus(&app), REPOS);
    app.click(5, 21);
    assert_eq!(focus(&app), PRS);
    // Details: the tab strip is rows 4-5, the content below; the log is rows 39-46.
    app.render(160, 48);
    app.click(70, 4);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.render(160, 48);
    app.click(70, 10);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.render(160, 48);
    app.click(70, 42);
    assert_eq!(focus(&app), FocusTarget::CommandLog);
    // The status box and the footer take no clicks.
    app.render(160, 48);
    app.click(5, 1).click(5, 47);
    assert_eq!(focus(&app), FocusTarget::CommandLog);
}

fn maximize_toast() -> Toast {
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    Toast::new(
        Severity::Information,
        Line::from(vec![
            Span::raw("Current view maximized. Press "),
            Span::styled("escape", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(" to return."),
        ]),
    )
    .with_title("View Maximized")
}

#[test]
fn maximize_toasts_once_and_escape_restores() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
    assert!(app.app().toasts().iter().eq([&maximize_toast()]));
    // M while maximized is a no-op.
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(focus(&app), REPOS);
    app.press(&["M"]);
    assert!(app.app().maximized().is_some());
    assert_eq!(app.app().toasts().len(), 1);
    assert_eq!(app.effects(), []);
}

#[test]
fn focus_keys_while_maximized_move_focus_but_not_the_maximized_widget() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M", "2"]);
    assert_eq!(focus(&app), PRS);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
    // Tab is limited to the maximized table: from outside it, it goes to it, then stays.
    app.press(&["tab"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["tab", "shift+tab"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["escape"]);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn tab_while_details_is_maximized_swaps_tabs_and_content() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "M", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn maximize_on_the_details_content_does_nothing_and_keeps_the_toast() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "tab"]);
    let before = app.screen(160, 48);
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert!(app.app().toasts().is_empty());
    assert_eq!(app.screen(160, 48), before);
    // The one-time toast is still unused, so the next maximize that works shows it.
    app.press(&["1", "M"]);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
    assert!(app.app().toasts().iter().eq([&maximize_toast()]));
    assert_eq!(app.effects(), []);
}

#[test]
fn every_other_focus_target_maximizes() {
    let targets = [
        FocusTarget::Table(SectionId::Repos),
        FocusTarget::Table(SectionId::PullRequests),
        FocusTarget::Table(SectionId::Issues),
        FocusTarget::Table(SectionId::Workflows),
        FocusTarget::DetailsTabs,
        FocusTarget::CommandLog,
    ];
    for (keys, target) in [["1"], ["2"], ["3"], ["4"], ["5"], ["6"]].iter().zip(targets) {
        let mut app = TestApp::new(with_log());
        app.press(keys);
        assert_eq!(focus(&app), target);
        app.press(&["M"]);
        assert_eq!(app.app().maximized(), Some(Maximized::widget(target)), "{target:?}");
    }
    // The search box types `M`; a rebound maximize reaches it (`a_rebound_maximize_maximizes_the_search_box`).
    for id in SectionId::ALL {
        assert!(FocusTarget::Search(id).maximizable());
    }
    assert!(!FocusTarget::DetailsContent.maximizable());
}

#[test]
fn escape_does_nothing_unless_maximized() {
    let mut app = TestApp::new(Config::default());
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn the_chain_follows_focus() {
    let mut app = TestApp::new(with_log());
    let owners = |app: &TestApp| -> Vec<Owner> { app.app().binding_chain().iter().map(|link| link.owner).collect() };
    let tail = [Owner::MainView, Owner::Screen(ScreenKind::Main), Owner::App];
    let table = [
        Owner::Table(SectionId::Repos),
        Owner::SearchableTable(SectionId::Repos),
        Owner::Section(SectionId::Repos),
        Owner::SelectionsPane,
    ];
    assert_eq!(owners(&app), [&table[..], &tail[..]].concat());
    for (key, inner) in [
        ("5", Owner::DetailsTabs),
        ("tab", Owner::DetailsContent),
        ("6", Owner::CommandLog),
    ] {
        app.press(&[key]);
        assert_eq!(owners(&app), [&[inner][..], &tail[..]].concat());
    }
    let chain = app.app().binding_chain();
    assert_eq!(
        chain.last().unwrap().bindings,
        [
            BindingId::TextualQuit,
            BindingId::TextualHelpQuit,
            BindingId::AppQuit,
            BindingId::AppCommandPalette,
            BindingId::AppMaximizeWidget,
            BindingId::AppOpenHelp,
        ]
    );
}

#[test]
fn bindings_not_built_yet_are_consumed() {
    // `f` and `ctrl+v` on the screen: no effect, and nothing quits.
    let mut app = TestApp::new(Config::default());
    app.press(&["f", "ctrl+v"]);
    assert_eq!(app.effects(), []);
    // ctrl+c still reaches `help_quit` past the screen's copy binding.
    app.press(&["ctrl+c"]);
    assert_eq!(app.app().toasts().len(), 1);
}

#[test]
fn notifications_open_is_hidden_and_inactive_while_notifications_are_off() {
    let screen = Owner::Screen(ScreenKind::Main);
    let mut app = TestApp::new(Config::default());
    assert!(!app.app().config().notifications.enabled);
    assert_eq!(
        app.app_mut().binding_state(screen, BindingId::NotificationsOpen),
        BindingState::Hidden
    );
    assert_eq!(
        app.app_mut().binding_state(screen, BindingId::LinkPaste),
        BindingState::Enabled
    );
    // Re-read on every call, so turning notifications on shows it.
    app.with_cx(|cx| cx.config_mut().notifications.enabled = true);
    assert_eq!(
        app.app_mut().binding_state(screen, BindingId::NotificationsOpen),
        BindingState::Enabled
    );
}

#[test]
fn notifications_open_falls_through_while_notifications_are_off() {
    // With an app binding moved onto ctrl+n, the key reaches it only while notifications are off.
    let mut config = Config::default();
    config.bindings.overrides.insert("app.quit".into(), "ctrl+n".into());
    let mut app = TestApp::new(config.clone());
    app.press(&["ctrl+n"]);
    assert_eq!(app.effects(), [Effect::Quit]);

    config.notifications.enabled = true;
    let mut app = TestApp::new(config);
    // Entering the main screen starts the notifications poll.
    let started = app.effects().len();
    app.press(&["ctrl+n"]);
    // The screen takes it: the Notifications modal opens and asks for its first page, and nothing quits.
    assert_eq!(app.app().modals(), [ModalKind::Notifications]);
    assert!(matches!(
        app.effects()[started..],
        [Effect::Github {
            request: GithubRequest::ListNotifications(NotificationQuery { all: true, page: 1 }),
            ..
        }]
    ));
}

#[test]
fn scroll_keys_scroll_the_focused_details_content() {
    let mut app = TestApp::new(Config::default());
    app.render(100, 30);
    app.press(&["down"]);
    let before = app.screen(100, 30);
    app.press(&["5", "tab", "down", "down", "down"]);
    let after = app.screen(100, 30);
    assert_ne!(before, after);
    assert!(!after.contains("LazyGithub: A Terminal UI"));
}

#[test]
fn focus_prs_100x30() {
    let mut app = TestApp::new(Config::default());
    app.press(&["2"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn focus_details_160x48() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn focus_command_log_160x48() {
    let mut app = TestApp::new(with_log());
    app.press(&["6"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn maximized_empty_repos_100x30() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M"]);
    // Past the "View Maximized" toast's timeout, so only the layout is on screen; `toasts.rs` covers the toast.
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(100, 30);
    // The empty table is its header row, centered: row floor((30 - 1) / 2) = 14.
    let rows: Vec<&str> = screen.lines().collect();
    assert_eq!(rows[14], " ★  Owner  Name  Private");
    assert!(
        rows.iter()
            .enumerate()
            .all(|(i, row)| i == 14 || row.chars().all(|c| c == '╱'))
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn maximized_details_160x48() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "M"]);
    app.advance(std::time::Duration::from_secs(5));
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn maximized_details_styles() {
    // Tabs on row 0 in the focused style, no border anywhere, and no hatch: the tab area fills the screen.
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "M"]);
    let buffer = app.render(100, 30);
    assert_eq!(buffer[(1, 0)].symbol(), "I");
    assert_eq!(buffer[(1, 0)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(buffer[(0, 1)].fg, Color::Rgb(0x4f, 0x4f, 0x4f));
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| !["┌", "│", "╱"].contains(&cell.symbol()))
    );
}

#[test]
fn clicks_while_maximized_hit_the_maximized_widget() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "M"]);
    app.render(100, 30);
    app.click(10, 10);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    app.render(100, 30);
    app.click(10, 0);
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
}

// The command log (#80).

fn log_line(n: usize) -> crate::event::Event {
    crate::event::Event::CommandLog(lazy_github_core::logging::LogLine {
        level: tracing::Level::INFO,
        timestamp: jiff::civil::date(2026, 10, 25).at(1, 2, 3, 0),
        message: format!("line {n}"),
    })
}

fn log_offset(app: &TestApp) -> (usize, usize) {
    app.app().main_screen().unwrap().command_log().offset()
}

#[test]
fn the_command_log_shows_lines_and_paints_surface() {
    let mut app = TestApp::new(with_log());
    for n in 1..=3 {
        app.send(log_line(n));
    }
    let buffer = app.render(160, 48);
    let screen = crate::test_util::buffer_to_string(&buffer);
    let rows: Vec<&str> = screen.lines().collect();
    // Python at 160x48: the log's interior is rows 40-45 and columns 65-158, with the tracks in the last two
    // columns and the last row.
    assert!(rows[40].contains("│INFO 2026-10-25 01:02:03 - line 1"), "{}", rows[40]);
    let surface = Color::Rgb(0x1e, 0x1e, 0x1e);
    assert_eq!(buffer[(65, 44)].bg, surface);
    assert_eq!(buffer[(156, 44)].bg, surface);
    for (x, y) in [(157, 40), (158, 44), (65, 45), (158, 45)] {
        assert_eq!(buffer[(x, y)].bg, Color::Rgb(0, 0, 0), "({x}, {y})");
    }
}

#[test]
fn hidden_command_log_lines_are_kept() {
    let mut app = TestApp::new(Config::default());
    app.send(log_line(1));
    let log = app.app().main_screen().unwrap().command_log();
    assert!(!log.is_visible());
    assert_eq!(log.lines().len(), 1);
    app.app_mut()
        .main_screen_mut()
        .unwrap()
        .command_log_mut()
        .set_visible(true);
    assert!(app.screen(160, 48).contains("line 1"));
}

#[test]
fn the_focused_command_log_scrolls_with_its_keys() {
    let mut app = TestApp::new(with_log());
    for n in 1..=30 {
        app.send(log_line(n));
    }
    app.press(&["6"]);
    app.render(160, 48);
    // Focused, the log is 40% of the 44 rows: 15 inside the border, 14 of text.
    assert_eq!(log_offset(&app), (0, 16));
    app.press(&["up", "up", "pageup"]);
    assert_eq!(log_offset(&app), (0, 0));
    app.press(&["end", "home"]);
    assert_eq!(log_offset(&app), (0, 0));
    app.press(&["pagedown", "down"]);
    assert_eq!(log_offset(&app), (0, 15));
    // j and k aren't bound here.
    app.press(&["j", "k"]);
    assert_eq!(log_offset(&app), (0, 15));
    assert_eq!(app.effects(), []);
}

#[test]
fn the_wheel_scrolls_the_command_log_under_the_pointer() {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let mut app = TestApp::new(with_log());
    for n in 1..=30 {
        app.send(log_line(n));
    }
    app.render(160, 48);
    assert_eq!(log_offset(&app), (0, 25));
    let wheel = |kind, column, row| {
        crate::event::Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    app.send(wheel(MouseEventKind::ScrollUp, 100, 41));
    assert_eq!(log_offset(&app), (0, 23));
    // Not over the log: nothing scrolls, and focus stays put.
    app.send(wheel(MouseEventKind::ScrollUp, 10, 10));
    assert_eq!(log_offset(&app), (0, 23));
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn the_maximized_command_log_fills_the_screen() {
    let mut app = TestApp::new(with_log());
    app.send(log_line(1));
    app.press(&["6", "M"]);
    let buffer = app.render(100, 30);
    let screen = crate::test_util::buffer_to_string(&buffer);
    assert!(screen.starts_with("INFO 2026-10-25 01:02:03 - line 1"), "{screen}");
    // Focused, so tinted.
    assert_eq!(buffer[(0, 0)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buffer[(99, 29)].bg, Color::Rgb(0, 0, 0));
}

// The repos table (#168).

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::gh::GhError;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::models::Repository;

use crate::action::Action;
use crate::components::table::SearchableTable;
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::test_util::{key_event, logs};

/// `base`'s three repos, in the API's `full_name` order.
fn base_repos() -> Vec<Repository> {
    ["dotfiles", "hello-world", "lazy-demo"]
        .map(|name| repos::repo("parity-bot", name, false))
        .into()
}

fn many_repos(n: usize) -> Vec<Repository> {
    (0..n)
        .map(|i| repos::repo("parity-bot", &format!("repo-{i:02}"), false))
        .collect()
}

fn with_repos(config: Config, repos: Vec<Repository>) -> TestApp {
    TestApp::with_github(config, FakeGithub::new().with_repositories(repos))
}

fn repos_table(app: &TestApp) -> &SearchableTable<Repository> {
    app.app().main_screen().unwrap().repos().table()
}

fn row_keys(app: &TestApp) -> Vec<String> {
    repos_table(app).row_keys().cloned().collect()
}

fn server_error() -> GhError {
    GhError::Http {
        status: 500,
        message: "Server Error".into(),
        body: br#"{"message": "Server Error"}"#.to_vec(),
        headers: Vec::new(),
    }
}

/// Answers every queued request, and returns what that logged.
fn settle_logged(app: &mut TestApp) -> String {
    logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn repos_section_100x30() {
    // A favorite, which sorts first in green, and a private repo.
    let mut repos = base_repos();
    repos[2].private = true;
    let mut config = Config::default();
    config.repositories.favorites = vec!["parity-bot/hello-world".into()];
    let mut app = with_repos(config, repos);
    let buffer = app.render(100, 30);
    let screen = crate::test_util::buffer_to_string(&buffer);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        rows[4].starts_with("│ ★  Owner       Name         Private  │"),
        "{}",
        rows[4]
    );
    assert!(
        rows[5].starts_with("│ ★  parity-bot  hello-world  ✘        │"),
        "{}",
        rows[5]
    );
    assert!(
        rows[6].starts_with("│ ☆  parity-bot  dotfiles     ✘        │"),
        "{}",
        rows[6]
    );
    assert!(
        rows[7].starts_with("│ ☆  parity-bot  lazy-demo    ✓        │"),
        "{}",
        rows[7]
    );
    // Rich's green, in the header; the cursor row draws over the cell's own color until the cursor moves off it.
    let green = Color::Rgb(0x98, 0xe0, 0x24);
    assert_eq!(buffer[(2, 4)].fg, green);
    assert_ne!(buffer[(2, 5)].fg, green);
    app.press(&["j"]);
    assert_eq!(app.render(100, 30)[(2, 5)].fg, green);
    insta::assert_snapshot!(screen);
}

#[test]
fn repos_loading_100x30() {
    let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories(base_repos()));
    assert!(repos_table(&app).is_loading());
    let screen = app.screen(100, 30);
    // Centered in the 38x8 interior, rows 4-11.
    let row = screen.lines().nth(7).unwrap();
    assert!(row.starts_with("│              Loading...              │"), "{row}");
    insta::assert_snapshot!(screen);
    settle_logged(&mut app);
    assert!(!repos_table(&app).is_loading());
    assert!(app.screen(100, 30).contains("dotfiles"));
}

#[test]
fn keys_reach_the_table_under_the_overlay() {
    let mut app = with_repos(Config::default(), base_repos());
    app.app_mut().main_screen_mut().unwrap().repos.table.set_loading(true);
    app.render(100, 30);
    app.press(&["j"]);
    assert_eq!(repos_table(&app).cursor(), 1);
}

#[test]
fn repos_fetch_error_100x30() {
    let gh = FakeGithub::new().with_repositories_error(server_error());
    let mut app = TestApp::loading(Config::default(), gh);
    let out = settle_logged(&mut app);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("ERROR Error fetching repositories from Github API"));
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("DEBUG Fetching repositories failed: ")),
        "{out}"
    );
    assert_eq!(lines.next(), None);
    assert!(app.app().toasts().is_empty());
    assert!(!repos_table(&app).is_loading());
    assert_eq!(repos_table(&app).row_count(), 0);
    let screen = app.screen(100, 30);
    // Only the header, and the section keeps its size.
    let rows: Vec<&str> = screen.lines().collect();
    assert!(rows[4].starts_with("│ ★  Owner  Name  Private"), "{}", rows[4]);
    assert!(rows[3].starts_with("┌─ [1] Repositories"), "{}", rows[3]);
    assert!(rows[12].starts_with("└"), "{}", rows[12]);
    insta::assert_snapshot!(screen);
}

#[test]
fn an_empty_list_draws_the_header_and_keeps_the_section() {
    let mut app = TestApp::new(Config::default());
    assert_eq!(repos_table(&app).row_count(), 0);
    let screen = app.screen(100, 30);
    assert!(screen.contains("[1] Repositories"));
    assert!(!screen.contains("(empty)"));
    assert!(screen.lines().nth(4).unwrap().starts_with("│ ★  Owner  Name  Private"));
}

#[test]
fn only_the_pending_response_fills_the_table() {
    let mut app = TestApp::loading(Config::default(), FakeGithub::new());
    let pending = match app.effects() {
        [Effect::Github { id, .. }, Effect::Git { .. }, Effect::Git { .. }] => *id,
        other => panic!("{other:?}"),
    };
    // A response to another request, even a repos list, is dropped.
    let stale = RequestId::new(999);
    let out = logs(|| {
        app.send(Event::Github {
            id: stale,
            response: GithubResponse::ListRepositories(Ok(base_repos())),
        });
    });
    assert_eq!(
        out,
        format!("DEBUG Dropping the response to request {stale}: nothing is waiting for it\n")
    );
    assert!(repos_table(&app).is_loading());
    assert_eq!(repos_table(&app).row_count(), 0);

    let out = logs(|| {
        app.send(Event::Github {
            id: pending,
            response: GithubResponse::ListRepositories(Ok(base_repos())),
        });
    });
    assert_eq!(out, "");
    assert!(!repos_table(&app).is_loading());
    assert_eq!(
        row_keys(&app),
        ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
    );

    // Answered once: a second answer to the same id is dropped too.
    let out = logs(|| {
        app.send(Event::Github {
            id: pending,
            response: GithubResponse::ListRepositories(Ok(Vec::new())),
        });
    });
    assert!(out.contains("Dropping the response"), "{out}");
    assert_eq!(repos_table(&app).row_count(), 3);
}

#[test]
fn table_keys_move_the_cursor_quietly_and_select_selects() {
    let mut app = with_repos(Config::default(), base_repos());
    app.render(100, 30);
    assert!(matches!(
        app.app_mut().action_for(Event::Key(key_event("j"))),
        Some(Action::Consumed)
    ));
    assert_eq!(repos_table(&app).cursor(), 1);
    let out = logs(|| {
        app.press(&["j", "k", "G"]);
    });
    assert_eq!(out, "");
    assert_eq!(repos_table(&app).cursor(), 2);
    assert!(matches!(
        app.app_mut().action_for(Event::Key(key_event("enter"))),
        Some(Action::SelectRepo(repo)) if repo.full_name == "parity-bot/lazy-demo"
    ));
    let out = logs(|| {
        app.press(&["space"]);
    });
    assert_eq!(out, "INFO Selected repo parity-bot/lazy-demo\n");
    // The columns fit, so `l` falls through the whole chain.
    assert!(app.app_mut().action_for(Event::Key(key_event("l"))).is_none());
    // The only effects are the selection's PR and issue caches and first page, the runs cache, the workflows and
    // runs, and the overview's README.
    let [
        Effect::ReadCache { .. },
        Effect::ReadCache { .. },
        Effect::Github {
            request: GithubRequest::ListIssues { repo, query },
            ..
        },
        Effect::ReadCache { .. },
        Effect::Github {
            request: GithubRequest::ListWorkflows { .. },
            ..
        },
        Effect::Github {
            request: GithubRequest::ListWorkflowRuns { .. },
            ..
        },
        Effect::Github {
            request: GithubRequest::RepoReadme(_),
            ..
        },
    ] = app.effects()
    else {
        panic!("one first-page request: {:?}", app.effects());
    };
    assert_eq!((repo.full_name.as_str(), query.page), ("parity-bot/lazy-demo", 1));
}

#[test]
fn a_click_on_a_row_focuses_the_table_and_moves_the_cursor() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["5"]);
    app.render(100, 30);
    // With details focused, repos is rows 3-8: the header on row 4 and the rows on 5-7.
    app.click(10, 6);
    assert_eq!(focus(&app), REPOS);
    assert_eq!(repos_table(&app).cursor(), 1);
    // A second click on the cursor's cell selects the row.
    app.render(100, 30);
    let out = logs(|| {
        app.click(10, 6);
    });
    assert_eq!(out, "INFO Selected repo parity-bot/hello-world\n");
}

#[test]
fn a_click_below_the_rows_focuses_nothing_and_one_on_the_header_focuses_the_table() {
    let mut app = with_repos(Config::default(), base_repos());
    // Drawn with repos focused: the header on row 4 and the rows on 5-7, in a box down to row 12.
    app.render(100, 30);
    app.app_mut().main_screen_mut().unwrap().set_focus(PRS);
    app.click(10, 9);
    assert_eq!(focus(&app), PRS);
    app.click(10, 4);
    assert_eq!(focus(&app), REPOS);
    assert_eq!(repos_table(&app).cursor(), 0);
}

#[test]
fn the_hit_rect_is_the_header_and_the_rows() {
    let mut app = with_repos(Config::default(), base_repos());
    app.render(100, 30);
    let rect = app.app().main_screen().unwrap().drawn_table_rect(SectionId::Repos);
    assert_eq!(rect, Some(ratatui::layout::Rect::new(1, 4, 38, 4)));
    let mut empty = TestApp::new(Config::default());
    empty.render(100, 30);
    let rect = empty.app().main_screen().unwrap().drawn_table_rect(SectionId::Repos);
    assert_eq!(rect, Some(ratatui::layout::Rect::new(1, 4, 38, 1)));
}

#[test]
fn the_wheel_scrolls_the_unfocused_repos_table() {
    let mut app = with_repos(Config::default(), many_repos(20));
    app.press(&["2"]);
    app.render(100, 30);
    app.send(mouse(MouseEventKind::ScrollDown, 10, 5));
    assert_eq!(repos_table(&app).scroll(), (2, 0));
    assert_eq!(repos_table(&app).cursor(), 0);
    assert_eq!(focus(&app), PRS);
    // Elsewhere, the wheel leaves it alone.
    app.send(mouse(MouseEventKind::ScrollDown, 60, 10));
    assert_eq!(repos_table(&app).scroll(), (2, 0));
    assert_eq!(app.effects(), []);
}

#[test]
fn moves_draw_the_hover_and_leaving_clears_it() {
    let mut app = with_repos(Config::default(), base_repos());
    let before = app.render(100, 30);
    app.send(mouse(MouseEventKind::Moved, 10, 6));
    assert_ne!(app.render(100, 30)[(10, 6)].bg, before[(10, 6)].bg);
    app.send(mouse(MouseEventKind::Moved, 60, 20));
    assert_eq!(app.render(100, 30)[(10, 6)].bg, before[(10, 6)].bg);
}

#[test]
fn the_loading_overlay_takes_clicks() {
    let mut app = TestApp::loading(Config::default(), FakeGithub::new());
    app.press(&["2"]);
    app.render(100, 30);
    app.click(10, 4);
    assert_eq!(focus(&app), PRS);
}

#[test]
fn without_animations_the_loading_table_has_no_deadline() {
    let app = TestApp::loading(Config::default(), FakeGithub::new());
    assert!(repos_table(&app).is_loading());
    assert_eq!(app.app().next_deadline(), None);
    let animated = TestApp::loading_with_options(
        Config::default(),
        FakeGithub::new(),
        crate::options::AppOptions {
            animations: true,
            ..crate::options::AppOptions::default()
        },
    );
    let now = crate::clock::Clock::now(animated.clock());
    assert_eq!(
        animated.app().next_deadline(),
        Some(now + crate::components::loading::FRAME_INTERVAL)
    );
}

#[test]
fn maximized_repos_100x30() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    // The header and 3 rows, centered: rows 13-16.
    assert_eq!(rows[13], " ★  Owner       Name         Private");
    assert_eq!(rows[16], " ☆  parity-bot  lazy-demo    ✘");
    assert!(
        rows.iter()
            .enumerate()
            .all(|(i, row)| (13..=16).contains(&i) || row.chars().all(|c| c == '╱'))
    );
    // The maximized table takes clicks.
    app.click(10, 15);
    assert_eq!(repos_table(&app).cursor(), 1);
    insta::assert_snapshot!(screen);
}

#[test]
fn maximized_while_loading_100x30() {
    let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories(base_repos()));
    app.press(&["M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(100, 30);
    // The table alone, its header only, with no overlay.
    assert!(!screen.contains("Loading"));
    assert_eq!(screen.lines().nth(14), Some(" ★  Owner  Name  Private"));
    insta::assert_snapshot!(screen);
    // The rows appear when the load lands.
    settle_logged(&mut app);
    let screen = app.screen(100, 30);
    assert_eq!(screen.lines().nth(13), Some(" ★  Owner       Name         Private"));
}

#[test]
fn a_maximized_table_of_60_repos_fills_the_screen() {
    let mut app = with_repos(Config::default(), many_repos(60));
    app.press(&["M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(100, 30);
    assert!(!screen.contains('╱'));
    assert!(screen.starts_with(" ★  Owner"));
}

// The search box on the main screen (#169).

use crate::components::table::TableFocus;

const SEARCH: FocusTarget = FocusTarget::Search(SectionId::Repos);

/// Types `text` one key at a time.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        app.press(&[c.encode_utf8(&mut [0; 4])]);
    }
}

/// The screen's focus and the table's own agree.
fn assert_focus_in_step(app: &TestApp) {
    let table = repos_table(app).focus();
    if focus(app) == SEARCH {
        assert_eq!(table, TableFocus::Search);
    } else {
        assert_eq!(table, TableFocus::Table);
    }
}

fn footer(app: &mut TestApp) -> String {
    app.screen(100, 30).lines().last().unwrap().trim_end().to_owned()
}

#[test]
fn the_focus_path_keeps_the_table_in_step() {
    let mut app = with_repos(with_log(), base_repos());
    app.render(100, 30);
    let steps: [(&[&str], FocusTarget); 7] = [
        (&["/"], SEARCH),
        (&["d", "o", "t"], SEARCH),
        (&["enter"], REPOS),
        (&["shift+tab"], SEARCH),
        (&["tab"], REPOS),
        (&["shift+tab", "shift+tab"], FocusTarget::CommandLog),
        (&["1"], REPOS),
    ];
    for (keys, expected) in steps {
        app.press(keys);
        app.render(100, 30);
        assert_eq!(focus(&app), expected, "{keys:?}");
        assert_focus_in_step(&app);
    }
    for target in [SEARCH, FocusTarget::CommandLog] {
        app.app_mut().update(Action::Focus(target));
        assert_eq!(focus(&app), target);
        assert_focus_in_step(&app);
    }
    // Clicks: on the box (rows 4-6), then on a row of the table under it.
    app.press(&["1"]);
    app.render(100, 30);
    app.click(10, 5);
    assert_eq!(focus(&app), SEARCH);
    assert_focus_in_step(&app);
    app.render(100, 30);
    app.click(10, 9);
    assert_eq!(focus(&app), REPOS);
    assert_focus_in_step(&app);
}

/// Where `text` starts on screen row `row`.
fn column_of(screen: &str, row: usize, text: &str) -> u16 {
    let line = screen.lines().nth(row).unwrap();
    let byte = line
        .find(text)
        .unwrap_or_else(|| panic!("{text:?} isn't on row {row}: {line:?}"));
    u16::try_from(line[..byte].chars().count()).unwrap()
}

#[test]
fn a_drag_from_the_search_box_keeps_the_mouse_until_the_release() {
    let mut app = with_repos(with_log(), base_repos());
    app.press(&["/"]);
    app.send(Event::Paste("foo-bar baz".into()));
    // The box takes rows 4-6; its text is on row 5.
    let screen = app.screen(100, 30);
    let bar = column_of(&screen, 5, "foo-bar baz") + 4;
    // Pressed on the `b` of `bar`, dragged right out of the Repositories box over the Pull Requests table and back
    // left over the rows below the box, and released down there: the release clicks no row.
    let cursor = repos_table(&app).cursor();
    app.drag((bar, 5), &[(90, 12), (bar + 3, 9)]);
    assert_eq!(focus(&app), SEARCH);
    assert_eq!(repos_table(&app).cursor(), cursor);
    type_text(&mut app, "x");
    assert_eq!(repos_table(&app).search_text(), "foo-x baz");
    // The release ended the drag: moves with the button down select nothing until the next press.
    app.mouse(MouseEventKind::Drag(MouseButton::Left), bar, 5);
    type_text(&mut app, "y");
    assert_eq!(repos_table(&app).search_text(), "foo-xy baz");
    // A drag past the box's left edge selects to the start.
    app.render(100, 30);
    app.drag((bar, 5), &[(0, 20)]);
    type_text(&mut app, "z");
    assert_eq!(repos_table(&app).search_text(), "zxy baz");
}

/// The titles of the toasts up now.
fn toast_titles(app: &TestApp) -> Vec<Option<String>> {
    app.app().toasts().iter().map(|toast| toast.title.clone()).collect()
}

fn clipboard_writes(app: &TestApp) -> Vec<&str> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::SetClipboard(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_c_in_the_search_box_copies_a_selection_or_else_hints_at_quitting() {
    let mut app = with_repos(with_log(), base_repos());
    app.press(&["/"]);
    app.send(Event::Paste("foo-bar baz".into()));
    // Nothing selected: the key isn't the box's, and reaches the app's `help_quit`.
    app.press(&["ctrl+c"]);
    assert_eq!(toast_titles(&app), [Some("Do you want to quit?".to_owned())]);
    assert_eq!((app.app().clipboard(), clipboard_writes(&app)), ("", vec![]));
    // With a selection the box copies it, keeps it, and nothing else sees the key.
    app.press(&["shift+left", "shift+left", "shift+left", "ctrl+c"]);
    assert_eq!(toast_titles(&app).len(), 1);
    assert_eq!((app.app().clipboard(), clipboard_writes(&app)), ("baz", vec!["baz"]));
    assert_eq!(focus(&app), SEARCH);
    // Cut and paste it back twice.
    app.press(&["ctrl+x", "ctrl+v", "ctrl+v"]);
    assert_eq!(repos_table(&app).search_text(), "foo-bar bazbaz");
    assert_eq!(clipboard_writes(&app), ["baz", "baz"]);
}

#[test]
fn the_ring_has_the_box_before_its_table_while_it_is_shown() {
    let mut app = with_repos(Config::default(), base_repos());
    let ring = |app: &TestApp| app.app().main_screen().unwrap().focus_ring(None);
    let rest = [
        PRS,
        ISSUES,
        WORKFLOWS,
        FocusTarget::DetailsTabs,
        FocusTarget::DetailsContent,
    ];
    assert_eq!(ring(&app), [&[REPOS][..], &rest].concat());
    app.press(&["/"]);
    assert_eq!(ring(&app), [&[SEARCH, REPOS][..], &rest].concat());
    // Shift+Tab from the blank box wraps to the end of the ring, and the box hides.
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(ring(&app), [&[REPOS][..], &rest].concat());
    // With text, it stays a stop.
    app.press(&["1", "/", "x", "shift+tab", "tab"]);
    assert_eq!(focus(&app), SEARCH);
    app.press(&["tab"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), SEARCH);
    // While the table is maximized, the ring holds only the table: the box is outside the `DataTable`.
    app.press(&["tab", "M"]);
    assert_eq!(app.app().main_screen().unwrap().focus_ring(Some(REPOS)), [REPOS]);
    app.press(&["shift+tab"]);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn a_blank_box_hides_whichever_way_focus_leaves_it() {
    let leave: [&[&str]; 3] = [&["enter"], &["tab"], &["shift+tab"]];
    for keys in leave {
        let mut app = with_repos(Config::default(), base_repos());
        app.press(&["/", "space", "space"]);
        app.press(keys);
        assert_ne!(focus(&app), SEARCH, "{keys:?}");
        assert!(!repos_table(&app).search_visible(), "{keys:?}");
    }
    // A click elsewhere: the details content.
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["/"]);
    app.render(100, 30);
    app.click(70, 10);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert!(!repos_table(&app).search_visible());
    // Text left unsubmitted keeps the box, and filters nothing.
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["/"]);
    type_text(&mut app, "dot");
    app.press(&["tab"]);
    assert!(repos_table(&app).search_visible());
    assert_eq!(repos_table(&app).row_count(), 3);
}

#[test]
fn printable_keys_type_into_the_box() {
    let mut app = with_repos(with_log(), base_repos());
    app.press(&["/"]);
    type_text(&mut app, "1a/b?qM6");
    assert_eq!(focus(&app), SEARCH);
    assert_eq!(repos_table(&app).search_text(), "1a/b?qM6");
    assert_eq!(app.app().maximized(), None);
    assert!(!app.app().help_open());
    assert!(!app.app().should_quit());
    assert_eq!(app.effects(), []);
    // The editing keys edit, and enter filters and focuses the table.
    app.press(&["ctrl+u"]);
    type_text(&mut app, "dot");
    app.press(&["enter"]);
    assert_eq!(focus(&app), REPOS);
    assert_eq!(row_keys(&app), ["parity-bot/dotfiles"]);
}

#[test]
fn paste_goes_into_the_box_only_while_it_has_focus() {
    let mut app = with_repos(Config::default(), base_repos());
    app.send(Event::Paste("dot".into()));
    assert_eq!(repos_table(&app).search_text(), "");
    app.press(&["/"]);
    app.send(Event::Paste("hello\nworld".into()));
    assert_eq!(repos_table(&app).search_text(), "hello");
    app.press(&["tab"]);
    app.send(Event::Paste("more".into()));
    assert_eq!(repos_table(&app).search_text(), "hello");
}

/// The input's inherited scroll keys stop at it, as in Python: they don't move the table's cursor behind it.
#[test]
fn scroll_keys_in_the_search_box_leave_the_table_cursor() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["j", "/"]);
    let cursor = repos_table(&app).cursor();
    assert_eq!(cursor, 1);
    app.press(&[
        "up",
        "down",
        "down",
        "pageup",
        "pagedown",
        "ctrl+pageup",
        "ctrl+pagedown",
    ]);
    assert_eq!(focus(&app), SEARCH);
    assert_eq!(repos_table(&app).cursor(), cursor);
    assert_eq!(app.effects(), []);
}

#[test]
fn the_footer_with_the_box_focused_is_search_and_the_palette() {
    let mut app = with_repos(Config::default(), base_repos());
    let row = footer(&mut app);
    assert!(row.contains("^f Toggle Favorite"), "{row}");
    app.press(&["/"]);
    let row = footer(&mut app);
    assert_eq!(row, " / Search  ^p Command Palette");
    insta::assert_snapshot!(row);
    // The help panel lists the box's keys under its heading, and none of the keys it types.
    app.app_mut().update(Action::ToggleHelp);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Input"), "{screen}");
    assert!(screen.contains("Delete character left"), "{screen}");
    assert!(!screen.contains("Maximize"), "{screen}");
    assert!(!screen.contains("Lookup Repository"), "{screen}");
}

#[test]
fn search_box_open_100x30() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["/"]);
    type_text(&mut app, "lazy");
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(rows[5].starts_with("│▊  lazy"), "{}", rows[5]);
    assert!(rows[8].starts_with("│ ★  Owner"), "{}", rows[8]);
    // The section keeps its focused border while the box has focus.
    let buffer = app.render(100, 30);
    assert_eq!(buffer[(0, 3)].fg, Color::Rgb(0x4e, 0xbf, 0x71));
    insta::assert_snapshot!(screen);
}

#[test]
fn slash_while_maximized_filters_the_maximized_table() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["M", "/"]);
    app.advance(std::time::Duration::from_secs(5));
    assert_eq!(focus(&app), SEARCH);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
    // The box takes focus and typing, but isn't drawn.
    type_text(&mut app, "dot");
    let screen = app.screen(100, 30);
    assert!(!screen.contains("Search"), "{screen}");
    assert_eq!(screen.lines().nth(13), Some(" ★  Owner       Name         Private"));
    app.press(&["enter"]);
    assert_eq!(focus(&app), REPOS);
    // The header and one row, re-centered: rows 14-15. The columns keep the widths of the rows the search hid.
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    assert_eq!(rows[14], " ★  Owner       Name         Private");
    assert_eq!(rows[15], " ☆  parity-bot  dotfiles     ✘");
    insta::assert_snapshot!(screen);
}

#[test]
fn escape_with_the_box_focused_while_maximized_minimizes() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["M", "/"]);
    type_text(&mut app, "lazy");
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(focus(&app), SEARCH);
    assert_eq!(repos_table(&app).search_text(), "lazy");
    // Not maximized, escape does nothing in the box.
    app.press(&["escape"]);
    assert_eq!(focus(&app), SEARCH);
}

/// Py bug B38, answered by Chris on #169: behind the box, outer keys are dropped by their keys after overrides, so
/// a maximize rebound off `M` works from the box, and maximizes the box alone (#156).
#[test]
fn a_rebound_maximize_maximizes_the_search_box() {
    let mut config = Config::default();
    config
        .bindings
        .overrides
        .insert("app.maximize_widget".into(), "f2".into());
    let mut app = with_repos(config, base_repos());
    app.press(&["/"]);
    assert_eq!(footer(&mut app), " / Search  ^p Command Palette  f2 Maximize");
    // `M` is typed, as with the default keys.
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), None);
    app.press(&["f2"]);
    assert_eq!(app.app().maximized(), Some(Maximized::widget(SEARCH)));
    app.advance(std::time::Duration::from_secs(5));
    // The input alone, at its 3 rows, centered on the hatch.
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(rows[14].starts_with("▊  M"), "{screen}");
    assert!(rows[12].chars().all(|c| c == '╱'), "{screen}");
    assert!(rows[16].chars().all(|c| c == '╱'), "{screen}");
    assert!(!screen.contains("Owner"), "{screen}");
    // The ring holds only the box.
    assert_eq!(app.app().main_screen().unwrap().focus_ring(Some(SEARCH)), [SEARCH]);
    app.press(&["tab", "shift+tab"]);
    assert_eq!(focus(&app), SEARCH);
    app.press(&["x"]);
    assert_eq!(repos_table(&app).search_text(), "Mx");
    app.press(&["escape"]);
    assert_eq!(app.app().maximized(), None);
    assert_eq!(focus(&app), SEARCH);
}

// Selecting a repo, and the current-directory default (#170).

use lazy_github_core::models::RepoName;

use crate::component::{Cx, LocalRepo, Outbox, Shared};
use crate::effect::GitResponse;

fn octo_repos() -> Vec<Repository> {
    vec![repos::repo("octo", "one", false), repos::repo("octo", "two", false)]
}

fn current_repo(app: &TestApp) -> Option<&str> {
    app.app().current_repo().map(|repo| repo.full_name.as_str())
}

fn details_title(app: &TestApp) -> &str {
    app.app().main_screen().unwrap().details_title()
}

fn details_tabs(app: &TestApp) -> Vec<String> {
    app.app().main_screen().unwrap().details_tabs().to_vec()
}

/// Applies `Action::SelectRepo(repo)`, and returns what that logged.
fn select(app: &mut TestApp, repo: Repository) -> String {
    logs(|| {
        app.app_mut().update(Action::SelectRepo(Box::new(repo)));
    })
}

/// The `Selected repo` lines in `out`.
fn selected_lines(out: &str) -> Vec<&str> {
    out.lines().filter(|line| line.contains("Selected repo")).collect()
}

/// A directory whose `origin` is `full_name`, cloned over SSH.
fn local(full_name: &str) -> Option<RepoName> {
    RepoName::from_remote_url(&format!("git@github.com:{full_name}.git"))
}

/// The id of the main screen's `OriginRepo` request in `app`'s effects.
fn origin_request(app: &TestApp) -> RequestId {
    app.effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Git { id, .. } => Some(*id),
            _ => None,
        })
        .expect("the main screen asks git for the current-directory repo")
}

#[test]
fn enter_and_space_select_the_cursor_repo() {
    for key in ["enter", "space"] {
        let mut app = with_repos(Config::default(), octo_repos());
        app.press(&["j"]);
        assert!(
            matches!(
                app.app_mut().action_for(Event::Key(key_event(key))),
                Some(Action::SelectRepo(repo)) if repo.full_name == "octo/two"
            ),
            "{key}"
        );
        // The cursor stays where it was.
        assert_eq!(repos_table(&app).cursor(), 1);
    }
}

#[test]
fn select_cursor_selects_the_cursor_repo() {
    // `textual.table.select_cursor` is shadowed by `common.table.select` in the chain, but means the same.
    let theme = Theme::default();
    let keymap = Keymap::new(&indexmap::IndexMap::new());
    let mut screen = MainScreen::new(&AppearanceSettings::default(), &keymap, &theme, AppOptions::default());
    let cells = repos::RepoCells {
        favorites: Vec::new(),
        star: Color::Reset,
    };
    screen.repos.table.replace_all(octo_repos(), &cells);
    let clock = std::sync::Arc::new(crate::clock::FakeClock::new());
    let mut shared = Shared::new(Config::default(), keymap, theme, AppOptions::default(), clock);
    let mut out = Outbox::default();
    let cx = Cx::new(&mut shared, &mut out);
    let action = screen.on_link_binding(Owner::Table(SectionId::Repos), BindingId::TextualTableSelect, &cx);
    assert!(matches!(action, Some(Action::SelectRepo(repo)) if repo.full_name == "octo/one"));
}

#[test]
fn nothing_is_selected_while_the_table_is_loading_or_empty() {
    let mut loading = TestApp::loading(Config::default(), FakeGithub::new().with_repositories(octo_repos()));
    let mut empty = TestApp::new(Config::default());
    for app in [&mut loading, &mut empty] {
        for key in ["enter", "space"] {
            assert!(
                matches!(
                    app.app_mut().action_for(Event::Key(key_event(key))),
                    Some(Action::Consumed)
                ),
                "{key}"
            );
        }
        let out = logs(|| {
            app.press(&["enter", "space"]);
        });
        assert_eq!(out, "");
        assert_eq!(current_repo(app), None);
    }
}

#[test]
fn selecting_logs_and_publishes_the_repo() {
    let mut app = with_repos(Config::default(), octo_repos());
    assert_eq!(app.with_cx(|cx| cx.current_repo().cloned()), None);
    let out = select(&mut app, octo_repos()[0].clone());
    assert_eq!(out, "INFO Selected repo octo/one\n");
    assert_eq!(
        app.with_cx(|cx| cx.current_repo().map(|repo| repo.full_name.clone())),
        Some("octo/one".into())
    );
    assert_eq!(current_repo(&app), Some("octo/one"));
    assert_eq!(details_title(&app), "[5] octo/one Details");
    assert_eq!(details_tabs(&app), ["Repo Overview"]);
    let shown = app
        .app()
        .main_screen()
        .unwrap()
        .details_repo()
        .map(|repo| repo.full_name.as_str());
    assert_eq!(shown, Some("octo/one"));
    // Nothing to fetch yet: #171 to #174 add the loads.
    assert_eq!(app.effects(), []);
}

#[test]
fn selecting_the_same_repo_again_does_it_all_again() {
    let mut app = with_repos(Config::default(), octo_repos());
    let out = logs(|| {
        app.press(&["enter", "enter"]);
    });
    assert_eq!(out, "INFO Selected repo octo/one\n".repeat(2));
    assert_eq!(details_tabs(&app), ["Repo Overview"]);
}

#[test]
fn selecting_another_repo_replaces_the_title_and_tab() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["enter", "j", "enter"]);
    assert_eq!(current_repo(&app), Some("octo/two"));
    assert_eq!(details_title(&app), "[5] octo/two Details");
    assert_eq!(details_tabs(&app), ["Repo Overview"]);
    let screen = app.screen(100, 30);
    assert!(screen.contains("Current repo: octo/two"), "{screen}");
    assert!(screen.contains("[5] octo/two Details"), "{screen}");
    assert!(!screen.contains("octo/one Details"), "{screen}");
}

#[test]
fn the_details_title_takes_its_key_from_the_overrides() {
    let mut config = Config::default();
    config
        .bindings
        .overrides
        .insert("main.details.focus".into(), "f5".into());
    let mut app = with_repos(config, octo_repos());
    app.press(&["enter"]);
    assert_eq!(details_title(&app), "[f5] octo/one Details");
}

#[test]
fn a_click_on_the_cursor_cell_focuses_the_repos_table_and_selects() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["2"]);
    app.render(100, 30);
    // The header is the rect's first row and the cursor's cell, column 0 of row 0, is under it.
    let rect = app
        .app()
        .main_screen()
        .unwrap()
        .drawn_table_rect(SectionId::Repos)
        .unwrap();
    let out = logs(|| {
        app.click(rect.x + 1, rect.y + 1);
    });
    assert_eq!(selected_lines(&out), ["INFO Selected repo octo/one"]);
    assert_eq!(focus(&app), REPOS);
    assert_eq!(current_repo(&app), Some("octo/one"));
}

#[test]
fn a_click_on_another_row_moves_the_cursor_without_selecting() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["2"]);
    app.render(100, 30);
    let rect = app
        .app()
        .main_screen()
        .unwrap()
        .drawn_table_rect(SectionId::Repos)
        .unwrap();
    let out = logs(|| {
        app.click(rect.x + 10, rect.y + 2);
    });
    assert_eq!(out, "");
    assert_eq!(focus(&app), REPOS);
    assert_eq!(repos_table(&app).cursor(), 1);
    assert_eq!(current_repo(&app), None);
}

#[test]
fn focus_in_the_details_content_moves_to_the_tab_strip() {
    let mut app = with_repos(with_log(), octo_repos());
    app.press(&["5", "tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    select(&mut app, octo_repos()[0].clone());
    assert_eq!(focus(&app), FocusTarget::DetailsTabs);
    // Everywhere else, focus stays.
    for (keys, target) in [
        (&["5"][..], FocusTarget::DetailsTabs),
        (&["6"], FocusTarget::CommandLog),
        (&["1"], REPOS),
    ] {
        app.press(keys);
        select(&mut app, octo_repos()[1].clone());
        assert_eq!(focus(&app), target);
    }
}

#[test]
fn a_maximized_widget_stays_maximized() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["M", "enter"]);
    assert_eq!(current_repo(&app), Some("octo/one"));
    assert_eq!(app.app().maximized(), Some(Maximized::widget(REPOS)));
}

#[test]
fn repo_selected_100x30() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["enter"]);
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(rows[1].starts_with("│Current repo: octo/one"), "{}", rows[1]);
    assert!(rows[3].contains("[5] octo/one Details"), "{}", rows[3]);
    assert!(rows[4].contains("│ Repo Overview "), "{}", rows[4]);
    assert!(!screen.contains("Introduction"), "{screen}");
    assert!(!screen.contains("LazyGithub: A Terminal UI"), "{screen}");
    assert_eq!(focus(&app), REPOS);
    insta::assert_snapshot!(screen);
}

#[test]
fn repo_selected_long_name_100x30() {
    let repo = repos::repo(
        "octo",
        "a-repository-with-a-name-far-too-long-for-the-details-title",
        false,
    );
    let mut app = with_repos(Config::default(), vec![repo]);
    app.press(&["enter"]);
    let screen = app.screen(100, 30);
    let title_row = screen.lines().nth(3).unwrap();
    assert!(title_row.contains("[5] octo/a-repository-with"), "{title_row}");
    assert!(title_row.contains("… ─┐"), "{title_row}");
    insta::assert_snapshot!(screen);
}

#[test]
fn repo_selected_from_the_details_content_tabs_styled() {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["5", "tab"]);
    select(&mut app, octo_repos()[0].clone());
    let buffer = app.render(100, 30);
    // The details box's tab strip: its two rows inside the border.
    let screen = crate::test_util::buffer_to_string(&buffer);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Repo Overview"))
        .unwrap();
    // The box's left border is the `│` before the label; its right border is the screen's last column.
    let byte = line.find("│ Repo Overview").unwrap();
    let x = u16::try_from(line[..byte].chars().count()).unwrap() + 1;
    let area = ratatui::layout::Rect::new(x, u16::try_from(row).unwrap(), 99 - x, 2);
    let mut strip = ratatui::buffer::Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            strip[(x, y)] = buffer[(x, y)].clone();
        }
    }
    // The active label in the block cursor colors: focus moved to the strip.
    assert_eq!(strip[(area.x + 1, area.y)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    insta::assert_debug_snapshot!(strip);
}

#[test]
fn the_current_directory_repo_is_selected_when_git_answers_after_the_rows() {
    let mut app = with_repos(Config::default(), base_repos());
    let out = logs(|| app.set_local_repo(local("parity-bot/hello-world")));
    assert_eq!(selected_lines(&out), ["INFO Selected repo parity-bot/hello-world"]);
    assert_eq!(current_repo(&app), Some("parity-bot/hello-world"));
    assert_eq!(details_title(&app), "[5] parity-bot/hello-world Details");
    assert_eq!(details_tabs(&app), ["Repo Overview"]);
    assert_eq!(focus(&app), REPOS);
    // The cursor doesn't move.
    assert_eq!(repos_table(&app).cursor(), 0);
}

#[test]
fn the_current_directory_repo_is_selected_when_git_answers_before_the_rows() {
    let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories(base_repos()));
    let id = origin_request(&app);
    app.send(Event::Git {
        id,
        response: GitResponse::OriginRepo(local("parity-bot/lazy-demo")),
    });
    assert_eq!(current_repo(&app), None);
    assert_eq!(
        app.with_cx(|cx| cx.local_repo().clone()),
        LocalRepo::Known(local("parity-bot/lazy-demo"))
    );
    let out = settle_logged(&mut app);
    assert_eq!(selected_lines(&out), ["INFO Selected repo parity-bot/lazy-demo"]);
    assert_eq!(current_repo(&app), Some("parity-bot/lazy-demo"));
    assert_eq!(repos_table(&app).cursor(), 0);
}

#[test]
fn the_rows_then_the_git_answer_select_once() {
    // `loading` answers the repos list first, then git.
    let mut app = TestApp::loading(Config::default(), FakeGithub::new().with_repositories(base_repos()))
        .with_local_repo(local("parity-bot/dotfiles"));
    let out = settle_logged(&mut app);
    assert_eq!(selected_lines(&out), ["INFO Selected repo parity-bot/dotfiles"]);
    assert_eq!(current_repo(&app), Some("parity-bot/dotfiles"));
}

#[test]
fn no_current_directory_repo_selects_nothing() {
    for repo in [local("someone/else"), None] {
        let mut app = with_repos(with_log(), base_repos());
        let out = logs(|| app.set_local_repo(repo.clone()));
        assert_eq!(out, "", "{repo:?}");
        assert_eq!(current_repo(&app), None);
        assert_eq!(details_title(&app), "[5] Details");
        assert!(app.app().toasts().is_empty());
        assert!(app.app().main_screen().unwrap().command_log().lines().is_empty());
    }
}

#[test]
fn a_failed_fetch_selects_nothing() {
    let gh = FakeGithub::new().with_repositories_error(server_error());
    let mut app = TestApp::loading(Config::default(), gh).with_local_repo(local("parity-bot/dotfiles"));
    let out = settle_logged(&mut app);
    assert!(selected_lines(&out).is_empty(), "{out}");
    assert_eq!(current_repo(&app), None);
}

#[test]
fn a_repo_selected_by_hand_before_the_git_answer_stays() {
    let mut app = with_repos(Config::default(), base_repos());
    app.press(&["j", "enter"]);
    assert_eq!(current_repo(&app), Some("parity-bot/hello-world"));
    let out = logs(|| app.set_local_repo(local("parity-bot/dotfiles")));
    assert_eq!(out, "");
    assert_eq!(current_repo(&app), Some("parity-bot/hello-world"));
}

/// Py bug B42: a clone of `Parity-Bot/DOTFILES` selects the table's `parity-bot/dotfiles`, spelled as the API does.
#[test]
fn the_current_directory_match_ignores_ascii_case() {
    let mut app = with_repos(Config::default(), base_repos());
    app.set_local_repo(local("Parity-Bot/DOTFILES"));
    assert_eq!(current_repo(&app), Some("parity-bot/dotfiles"));
    assert!(app.screen(100, 30).contains("Current repo: parity-bot/dotfiles"));
}

/// A repo selected whose first page is empty, so the PR section is compact.
fn compact_prs() -> TestApp {
    let mut app = with_repos(Config::default(), octo_repos());
    app.press(&["enter"]);
    settle_logged(&mut app);
    assert!(
        app.app()
            .main_screen()
            .unwrap()
            .section(SectionId::PullRequests)
            .compact
    );
    app
}

/// The cells of the PR box's title row that are dim, as `d`s.
fn dim_title(app: &mut TestApp) -> String {
    let buffer = app.render(100, 30);
    let row = (0..30)
        .find(|&y| buffer[(3, y)].symbol() == "[" && buffer[(4, y)].symbol() == "2")
        .expect("the PR box's title row");
    (0..100)
        .map(|x| {
            if buffer[(x, row)].modifier.contains(ratatui::style::Modifier::DIM) {
                'd'
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn compact_prs_100x30() {
    let mut app = compact_prs();
    insta::assert_snapshot!(app.screen(100, 30));
    // `[2] Pull Requests ` is plain and `(empty)` dim.
    assert_eq!(dim_title(&mut app), format!("{}ddddddd", " ".repeat(21)));
}

#[test]
fn compact_prs_focused_100x30() {
    let mut app = compact_prs();
    app.press(&["2"]);
    insta::assert_snapshot!(app.screen(100, 30));
    assert_eq!(dim_title(&mut app), format!("{}ddddddd", " ".repeat(21)));
}

// The pull requests section (#171).

use lazy_github_core::models::IssueListItem;

use super::pull_requests::{Kind, item};

/// `octo/one` selected, with an open, a closed and a draft PR among issues on its first page.
fn prs_loaded() -> TestApp {
    let repo = repos::repo("octo", "one", false);
    let page: Vec<IssueListItem> = vec![
        item(&repo, 14, Kind::Issue),
        item(&repo, 13, Kind::OpenPr),
        item(&repo, 12, Kind::ClosedPr),
        item(&repo, 11, Kind::DraftPr),
    ];
    let gh = FakeGithub::new()
        .with_repositories(octo_repos())
        .with_issue_page("octo/one", 1, Ok(page));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle_logged(&mut app);
    app
}

#[test]
fn prs_section_100x30() {
    let mut app = prs_loaded();
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    let header = rows
        .iter()
        .position(|row| row.contains("#   Status  Author"))
        .expect("the PR header");
    assert!(
        rows[header - 1].starts_with("┌─ [2] Pull Requests ─"),
        "{}",
        rows[header - 1]
    );
    assert!(
        rows[header + 1].starts_with("│ 13  open    parity-bot  PR 13"),
        "{}",
        rows[header + 1]
    );
    assert!(
        rows[header + 2].starts_with("│ 12  closed  parity-bot  PR 12"),
        "{}",
        rows[header + 2]
    );
    // The draft is below the fold here, and drawn in `prs_maximized_100x30`; the issue isn't a row at all.
    let prs = app.app().main_screen().unwrap().issue_lists.prs_table();
    assert_eq!(prs.row_keys().copied().collect::<Vec<u64>>(), [13, 12, 11]);
    insta::assert_snapshot!(screen);
}

#[test]
fn prs_loading_100x30() {
    let gh = FakeGithub::new().with_repositories(octo_repos());
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    let screen = app.screen(100, 30);
    assert!(screen.contains("Loading..."), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn prs_maximized_100x30() {
    let mut app = prs_loaded();
    app.press(&["2", "M"]);
    app.advance(std::time::Duration::from_secs(5));
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    // The header and 3 rows, centered: rows 13-16.
    assert!(rows[13].starts_with(" #   Status  Author      Title"), "{}", rows[13]);
    assert!(rows[16].starts_with(" 11  open    parity-bot  PR 11"), "{}", rows[16]);
    insta::assert_snapshot!(screen);
}

// The issues section (#172).

/// `octo/one` selected, with an open and a closed issue among PRs on its first page.
fn issues_loaded() -> TestApp {
    let repo = repos::repo("octo", "one", false);
    let page: Vec<IssueListItem> = vec![
        item(&repo, 9, Kind::Issue),
        item(&repo, 8, Kind::OpenPr),
        item(&repo, 7, Kind::ClosedIssue),
    ];
    let gh = FakeGithub::new()
        .with_repositories(octo_repos())
        .with_issue_page("octo/one", 1, Ok(page));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle_logged(&mut app);
    app
}

#[test]
fn issues_section_100x30() {
    let mut app = issues_loaded();
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    let title = rows
        .iter()
        .position(|row| row.starts_with("┌─ [3] Issues ─"))
        .expect("the Issues box");
    assert!(
        rows[title + 1].starts_with("│ #  Status  Author      Title"),
        "{}",
        rows[title + 1]
    );
    assert!(
        rows[title + 2].starts_with("│ 9  open    parity-bot  Issue 9"),
        "{}",
        rows[title + 2]
    );
    assert!(
        rows[title + 3].starts_with("│ 7  closed  parity-bot  Issue 7"),
        "{}",
        rows[title + 3]
    );
    let issues = app.app().main_screen().unwrap().issue_lists.issues_table();
    assert_eq!(issues.row_keys().copied().collect::<Vec<u64>>(), [9, 7]);
    insta::assert_snapshot!(screen);
}

#[test]
fn compact_issues_100x30() {
    // The first page is empty, so the Issues section is ` (empty)` too.
    let mut app = compact_prs();
    let screen = app.screen(100, 30);
    assert!(screen.contains("┌─ [3] Issues (empty) ─"), "{screen}");
    assert!(app.app().main_screen().unwrap().section(SectionId::Issues).compact);
    insta::assert_snapshot!(screen);
}

#[test]
fn issues_focus_key_focuses_the_table() {
    let mut app = issues_loaded();
    app.press(&["3"]);
    assert_eq!(
        app.app().main_screen().unwrap().focus(),
        FocusTarget::Table(SectionId::Issues)
    );
    app.press(&["j"]);
    let issues = app.app().main_screen().unwrap().issue_lists.issues_table();
    assert_eq!(issues.selected().map(|issue| issue.number), Some(7));
}

// The workflows section (#173).

use super::workflows::run;

/// `octo/one` selected, with an in-progress, a completed and a queued run.
fn runs_loaded() -> TestApp {
    let repo = repos::repo("octo", "one", false);
    let mut queued = run(&repo, 3, "Lint", 7, "2026-10-19T23:59:00Z", None, "Queued lint");
    queued.status = "queued".into();
    let page = vec![
        run(&repo, 1, "CI", 13, "2026-10-21T09:00:05Z", None, "In flight"),
        run(
            &repo,
            2,
            "CI",
            12,
            "2026-10-20T10:15:00Z",
            Some("success"),
            "Fix the thing",
        ),
        queued,
    ];
    let gh = FakeGithub::new()
        .with_repositories(octo_repos())
        .with_run_page("octo/one", 1, Ok(page));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    settle_logged(&mut app);
    app
}

#[test]
fn workflows_section_100x30() {
    let mut app = runs_loaded();
    // Maximized, every column shows.
    app.press(&["4", "M"]);
    app.advance(std::time::Duration::from_secs(5));
    let maximized = app.screen(100, 30);
    for row in [
        "Time              Result       Job Name  Run Name       Run #",
        "2026-10-21 09:00  in_progress  CI        In flight      13",
        "2026-10-20 10:15  success      CI        Fix the thing  12",
        "2026-10-19 23:59  queued       Lint      Queued lint    7",
    ] {
        assert!(maximized.contains(row), "{row}\n{maximized}");
    }
    app.press(&["escape"]);
    let screen = app.screen(100, 30);
    assert!(screen.contains("┌─ [4] Workflows ──"), "{screen}");
    assert!(!screen.contains("Workflows (empty)"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn workflows_compact_100x30() {
    // `octo/two` has no runs.
    let gh = FakeGithub::new().with_repositories(octo_repos());
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["j", "enter"]);
    settle_logged(&mut app);
    let screen = app.screen(100, 30);
    assert!(screen.contains("┌─ [4] Workflows (empty) ──"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn workflows_loading_100x30() {
    let gh = FakeGithub::new().with_repositories(octo_repos());
    let mut app = TestApp::with_github(Config::default(), gh);
    app.press(&["enter"]);
    let screen = app.screen(100, 30);
    let rows: Vec<&str> = screen.lines().collect();
    let title = rows
        .iter()
        .position(|row| row.starts_with("┌─ [4] Workflows"))
        .expect("the Workflows box");
    assert!(
        rows[title + 1..].iter().take(5).any(|row| row.contains("Loading...")),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}
