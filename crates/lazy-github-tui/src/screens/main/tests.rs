use lazy_github_core::config::Config;
use ratatui::style::Color;

use super::*;
use crate::keymap::Keymap;
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
    let mut screen = MainScreen::new(&AppearanceSettings::default(), &keymap);
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
    screen.set_focus(FocusTarget::Search(SectionId::Workflows));
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
    assert_eq!(owners[1], Owner::Section(SectionId::PullRequests));
    app.press(&["tab"]);
    assert_eq!(focus(&app), REPOS);
    app.press(&["2", "shift+tab"]);
    assert_eq!(focus(&app), REPOS);
}

#[test]
fn a_section_hidden_by_toggle_ignores_its_key_but_workflows_does_not() {
    let mut app = TestApp::new(Config::default());
    let main = app.app_mut().main_screen_mut().unwrap();
    main.set_hidden_by_toggle(SectionId::PullRequests, true);
    main.set_hidden_by_toggle(SectionId::Workflows, true);
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
    // The PR section is rows 20-28 with repos focused; its border is column 0 and row 20.
    app.click(0, 22).click(5, 20);
    assert_eq!(focus(&app), REPOS);
    app.click(5, 22);
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
    Toast {
        severity: Severity::Information,
        title: Some("View Maximized".into()),
        message: Line::from(vec![
            Span::raw("Current view maximized. Press "),
            Span::styled("escape", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(" to return."),
        ]),
    }
}

#[test]
fn maximize_toasts_once_and_escape_restores() {
    let mut app = TestApp::new(Config::default());
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(Maximized { target: REPOS }));
    assert_eq!(app.app().toasts(), [maximize_toast()]);
    // M while maximized is a no-op.
    app.press(&["M"]);
    assert_eq!(app.app().maximized(), Some(Maximized { target: REPOS }));
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
    assert_eq!(app.app().maximized(), Some(Maximized { target: REPOS }));
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
    // `u` on the repos section, `P` on the selections pane, `f` on the screen: no effect, and nothing quits.
    let mut app = TestApp::new(Config::default());
    app.press(&["u", "P", "f", "ctrl+v"]);
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
    app.press(&["ctrl+n"]);
    assert_eq!(app.effects(), []);
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
    let screen = app.screen(100, 30);
    // The empty table is its header row, centered: row floor((30 - 1) / 2) = 14.
    let rows: Vec<&str> = screen.lines().collect();
    assert_eq!(rows[14], "");
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
