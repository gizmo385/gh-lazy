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
