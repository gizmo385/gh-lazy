use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use lazy_github_core::logging::LogLine;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tracing::Level;

use super::*;
use crate::test_util::buffer_to_string;

fn line(level: Level, message: &str) -> LogLine {
    LogLine {
        level,
        timestamp: jiff::civil::date(2026, 10, 25).at(1, 2, 3, 0),
        message: message.into(),
    }
}

fn info(message: &str) -> LogLine {
    line(Level::INFO, message)
}

/// A log with `n` lines, `line 1` to `line n`.
fn log_with(n: usize) -> CommandLog {
    let mut log = CommandLog::new(true);
    for i in 1..=n {
        log.push(&info(&format!("line {i}")));
    }
    log
}

/// Draws `log` into a 60x8 area: 58x7 of text, inside the scrollbars.
fn draw(log: &mut CommandLog, focused: bool) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
    terminal
        .draw(|frame| log.render(frame, frame.area(), &Theme::default(), focused))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    buffer_to_string(buffer)
        .lines()
        .map(|row| row.trim_end().to_owned())
        .collect()
}

#[test]
fn lines_use_the_log_line_format() {
    let mut log = CommandLog::new(true);
    log.push(&info("Selected repo foo/bar"));
    log.push(&line(Level::WARN, "Careful"));
    log.push(&line(Level::ERROR, "Error fetching repositories from Github API"));
    assert_eq!(
        log.lines(),
        [
            "INFO 2026-10-25 01:02:03 - Selected repo foo/bar",
            "WARNING 2026-10-25 01:02:03 - Careful",
            "ERROR 2026-10-25 01:02:03 - Error fetching repositories from Github API",
        ]
    );
}

#[test]
fn multi_line_messages_split_like_python_splitlines() {
    let mut log = CommandLog::new(true);
    log.push(&info("first\nsecond\r\nthird\rfourth\u{2028}fifth\n\nlast\n"));
    assert_eq!(
        log.lines(),
        [
            "INFO 2026-10-25 01:02:03 - first",
            "second",
            "third",
            "fourth",
            "fifth",
            "",
            "last"
        ]
    );
    assert_eq!(split_lines(""), Vec::<&str>::new());
    assert_eq!(
        split_lines("a\x0bb\x0cc\x1cd\x1de\x1ef\u{85}g"),
        ["a", "b", "c", "d", "e", "f", "g"]
    );
}

#[test]
fn tabs_expand_and_control_characters_are_replaced() {
    let mut log = CommandLog::new(true);
    // The prefix is 27 characters, so the tab after "a" stops at column 32.
    log.push(&info("a\tb\x00c\x08d\x14e"));
    assert_eq!(
        log.lines(),
        ["INFO 2026-10-25 01:02:03 - a    b\u{FFFD}c\u{FFFD}d\u{FFFD}e"]
    );
    assert_eq!(process_row("\tx\t"), "        x       ");
    // Beyond Textual: the rest of C0 and DEL, so nothing reaches the terminal raw.
    assert_eq!(process_row("\x15\x1b[31m\x7f"), "\u{FFFD}\u{FFFD}[31m\u{FFFD}");
    assert_eq!(process_row("é ✓"), "é ✓");
}

#[test]
fn the_view_follows_the_tail_only_from_the_bottom() {
    let mut log = log_with(10);
    draw(&mut log, false);
    // 10 lines in 7 rows: the last 7 show.
    assert_eq!(log.offset(), (0, 3));
    log.push(&info("line 11"));
    assert_eq!(log.offset(), (0, 4));
    log.scroll(Scroll::Up);
    log.push(&info("line 12"));
    assert_eq!(log.offset(), (0, 3));
    log.scroll(Scroll::End);
    log.push(&info("line 13"));
    assert_eq!(log.offset(), (0, 6));
    assert_eq!(rows(&draw(&mut log, false))[6], "INFO 2026-10-25 01:02:03 - line 13");
}

#[test]
fn lines_pushed_before_the_first_draw_show_the_tail() {
    let mut log = CommandLog::new(false);
    for i in 1..=10 {
        log.push(&info(&format!("line {i}")));
    }
    assert_eq!(log.lines().len(), 10);
    log.set_visible(true);
    assert!(log.is_visible());
    let screen = rows(&draw(&mut log, false));
    assert_eq!(screen[0], "INFO 2026-10-25 01:02:03 - line 4");
    assert_eq!(log.offset(), (0, 3));
}

#[test]
fn every_scroll_key_is_clamped() {
    let mut log = log_with(20);
    // One line 100 cells wide: 42 columns past the 58-column view.
    log.push(&info(&"x".repeat(100 - 27)));
    draw(&mut log, false);
    assert_eq!(log.offset(), (0, 14));
    let mut press = |scroll: Scroll| {
        log.scroll(scroll);
        log.offset()
    };
    assert_eq!(press(Scroll::Down), (0, 14));
    assert_eq!(press(Scroll::Up), (0, 13));
    assert_eq!(press(Scroll::PageUp), (0, 6));
    assert_eq!(press(Scroll::PageUp), (0, 0));
    assert_eq!(press(Scroll::Up), (0, 0));
    assert_eq!(press(Scroll::PageDown), (0, 7));
    assert_eq!(press(Scroll::Left), (0, 7));
    assert_eq!(press(Scroll::Right), (1, 7));
    assert_eq!(press(Scroll::PageRight), (42, 7));
    assert_eq!(press(Scroll::Right), (42, 7));
    assert_eq!(press(Scroll::PageLeft), (0, 7));
    assert_eq!(press(Scroll::PageRight), (42, 7));
    // Home keeps the column unless already at the top; a second Home goes to the first column.
    assert_eq!(press(Scroll::Home), (42, 0));
    assert_eq!(press(Scroll::Home), (0, 0));
    // End goes to the first column from row 0, since Textual compares the row with `False`.
    assert_eq!(press(Scroll::PageRight), (42, 0));
    assert_eq!(press(Scroll::End), (0, 14));
    assert_eq!(press(Scroll::PageRight), (42, 14));
    assert_eq!(press(Scroll::End), (42, 14));
    assert_eq!(press(Scroll::PageDown), (42, 14));
}

#[test]
fn the_wheel_scrolls_by_textual_sensitivity() {
    let mut log = log_with(20);
    log.push(&info(&"x".repeat(100 - 27)));
    draw(&mut log, false);
    log.wheel(Wheel::Up);
    assert_eq!(log.offset(), (0, 12));
    log.wheel(Wheel::Down);
    log.wheel(Wheel::Down);
    assert_eq!(log.offset(), (0, 14));
    log.wheel(Wheel::Right);
    assert_eq!(log.offset(), (4, 14));
    log.wheel(Wheel::Left);
    log.wheel(Wheel::Left);
    assert_eq!(log.offset(), (0, 14));

    let mouse = |kind, modifiers| MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers,
    };
    let none = KeyModifiers::NONE;
    assert_eq!(
        Wheel::from_mouse(&mouse(MouseEventKind::ScrollUp, none)),
        Some(Wheel::Up)
    );
    assert_eq!(
        Wheel::from_mouse(&mouse(MouseEventKind::ScrollDown, KeyModifiers::SHIFT)),
        Some(Wheel::Right)
    );
    assert_eq!(
        Wheel::from_mouse(&mouse(MouseEventKind::ScrollUp, KeyModifiers::CONTROL)),
        Some(Wheel::Left)
    );
    assert_eq!(
        Wheel::from_mouse(&mouse(MouseEventKind::ScrollRight, none)),
        Some(Wheel::Right)
    );
    assert_eq!(Wheel::from_mouse(&mouse(MouseEventKind::Moved, none)), None);
}

#[test]
fn the_empty_log_paints_surface_and_both_tracks() {
    // Python at 160x48: the interior in `$surface`, the tracks and the corner in `$scrollbar-background`.
    let buffer = draw(&mut CommandLog::new(true), false);
    let surface = Color::Rgb(0x1e, 0x1e, 0x1e);
    let black = Color::Rgb(0, 0, 0);
    let text = Color::Rgb(0xe1, 0xe1, 0xe1);
    for y in 0..7 {
        for x in 0..58 {
            assert_eq!((buffer[(x, y)].fg, buffer[(x, y)].bg), (text, surface), "({x}, {y})");
        }
        assert_eq!((buffer[(58, y)].fg, buffer[(58, y)].bg), (text, black));
        assert_eq!(buffer[(59, y)].bg, black);
    }
    for x in 0..58 {
        assert_eq!((buffer[(x, 7)].symbol(), buffer[(x, 7)].bg), (" ", black));
    }
    let corner = &buffer[(59, 7)];
    assert_eq!((corner.fg, corner.bg), (Color::Reset, black));
}

#[test]
fn focus_tints_the_background() {
    let mut log = log_with(1);
    let buffer = draw(&mut log, true);
    let cell = &buffer[(0, 0)];
    // Python's focused log: `$surface` tinted with `$foreground` at 5%.
    assert_eq!(
        (cell.fg, cell.bg),
        (Color::Rgb(0xe2, 0xe2, 0xe2), Color::Rgb(0x27, 0x27, 0x27))
    );
    assert_eq!(buffer[(58, 0)].fg, Color::Rgb(0xe2, 0xe2, 0xe2));
}

#[test]
fn a_long_line_gets_a_horizontal_thumb() {
    // Python's 100x30 log: a 97-cell line in a 56-column view puts the thumb's `▍` tail at column 32.
    let mut log = CommandLog::new(true);
    log.push(&line(
        Level::ERROR,
        "Cannot load notifications - please login to the gh CLI: gh auth login",
    ));
    assert_eq!(Span::raw(log.lines()[0].as_str()).width(), 97);
    let mut terminal = Terminal::new(TestBackend::new(58, 4)).unwrap();
    terminal
        .draw(|frame| log.render(frame, frame.area(), &Theme::default(), false))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let bar: String = (0..56).map(|x| buffer[(x, 3)].symbol().to_owned()).collect();
    assert_eq!(bar.trim_end(), format!("{}▍", " ".repeat(32)));
    assert_eq!(buffer[(0, 3)].modifier, Modifier::REVERSED);
    assert_eq!(buffer[(0, 3)].fg, Color::Rgb(0x00, 0x30, 0x54));
}

#[test]
fn a_cut_through_a_wide_character_leaves_a_blank() {
    // The #137 review's repro: the prefix is 27 cells, so column 28 is the right half of the first `日`. Textual's
    // `crop_extend` draws a blank there and `本` at column 1.
    let mut log = CommandLog::new(true);
    log.push(&info("日本語日本語"));
    log.push(&info(&"x".repeat(40)));
    let row = |log: &mut CommandLog, x: usize| {
        log.x = x;
        // A fresh backend each time: `TestBackend` doesn't model a wide character covering the cell after it.
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        terminal
            .draw(|frame| log.render(frame, frame.area(), &Theme::default(), false))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // A wide character's second cell holds a blank symbol.
        let cells: String = (0..18).map(|col| buffer[(col, 0)].symbol().to_owned()).collect();
        cells.trim_end().to_owned()
    };
    assert_eq!(row(&mut log, 28), " 本 語 日 本 語");
    assert_eq!(row(&mut log, 29), "本 語 日 本 語");
    assert_eq!(crop_left("ab", 5), "");
    assert_eq!(crop_left("e\u{301}x", 1), "x");
}

#[test]
fn tiny_areas_do_not_panic() {
    for (w, h) in [(0, 0), (1, 1), (2, 1), (3, 2), (5, 40)] {
        let mut log = log_with(3);
        let mut terminal = Terminal::new(TestBackend::new(w.max(1), h.max(1))).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                log.render(frame, area, &Theme::default(), true);
            })
            .unwrap();
        log.scroll(Scroll::End);
        log.scroll(Scroll::PageRight);
    }
}

#[test]
fn command_log_empty() {
    insta::assert_debug_snapshot!(draw(&mut CommandLog::new(true), false));
}

#[test]
fn command_log_three_lines() {
    let mut log = CommandLog::new(true);
    log.push(&info("Selected repo gizmo385/gh-lazy"));
    log.push(&line(Level::WARN, "Selected PR: #12"));
    log.push(&line(Level::ERROR, "Error fetching repositories from Github API"));
    insta::assert_debug_snapshot!(draw(&mut log, false));
}

#[test]
fn command_log_overflow_tail() {
    let mut log = log_with(20);
    draw(&mut log, false);
    insta::assert_debug_snapshot!(draw(&mut log, false));
}

#[test]
fn command_log_scrolled_up() {
    let mut log = log_with(20);
    draw(&mut log, false);
    log.scroll(Scroll::PageUp);
    log.push(&info("line 21"));
    insta::assert_debug_snapshot!(draw(&mut log, false));
}

#[test]
fn command_log_focused() {
    let mut log = log_with(3);
    insta::assert_debug_snapshot!(draw(&mut log, true));
}

#[test]
fn command_log_long_line_hscroll() {
    let mut log = log_with(2);
    log.push(&info(&format!("{}|end", "wide ".repeat(15))));
    draw(&mut log, false);
    log.scroll(Scroll::PageRight);
    insta::assert_debug_snapshot!(draw(&mut log, false));
}
