use std::path::{Path, PathBuf};

use lazy_github_core::config::Config;
use lazy_github_core::screen::{self, CellStyle};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::*;
use crate::event::Event;
use crate::modal::{CommandPaletteDialog, ModalKind};
use crate::test_util::TestApp;
use crate::toast::Severity;

/// The palette's hits after typing `query`.
fn search(app: &mut TestApp, query: &str) -> Vec<String> {
    app.press(&["ctrl+p"]);
    let keys: Vec<String> = query.chars().map(|c| c.to_string()).collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    app.press(&keys);
    app.app()
        .top_dialog::<CommandPaletteDialog>()
        .map(|palette| palette.hit_names().into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The bottom-right corner of the screen, where toasts are, as text.
fn toast_corner(app: &mut TestApp) -> String {
    let screen = app.screen(160, 48);
    let rows: Vec<&str> = screen.lines().collect();
    rows[rows.len() - 9..]
        .iter()
        .map(|row| row.chars().skip(96).collect::<String>().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_buffer_becomes_a_screen_cell_for_cell() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 2));
    buffer.set_string(0, 0, "a", Style::new().fg(Color::Red).bg(Color::Indexed(200)));
    buffer.set_string(1, 0, "🔎b", Style::new().fg(Color::Rgb(1, 2, 3)));
    buffer.set_string(
        0,
        1,
        "x",
        Style::new()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD | Modifier::DIM | Modifier::ITALIC)
            .add_modifier(Modifier::UNDERLINED | Modifier::REVERSED | Modifier::CROSSED_OUT),
    );
    let screen = screen_from_buffer(&buffer);
    assert_eq!((screen.width(), screen.height()), (6, 2));
    assert_eq!(screen.to_text(), "a🔎b\nx");

    let cell = |x, y| screen.cell(x, y).unwrap().clone();
    assert_eq!(cell(0, 0).style.fg, screen::Color::Indexed(1));
    assert_eq!(cell(0, 0).style.bg, screen::Color::Indexed(200));
    assert_eq!(cell(1, 0).symbol, "🔎");
    assert_eq!(cell(1, 0).style.fg, screen::Color::Rgb(1, 2, 3));
    // The emoji's second cell is empty, as ratatui's diff skips it.
    assert_eq!(cell(2, 0).symbol, "");
    assert_eq!(cell(3, 0).symbol, "b");
    assert_eq!(cell(4, 0).style, CellStyle::default());
    assert_eq!(
        cell(0, 1).style,
        CellStyle {
            fg: screen::Color::Indexed(15),
            bg: screen::Color::Indexed(8),
            bold: true,
            dim: true,
            italic: true,
            underline: true,
            reversed: true,
            crossed_out: true,
        }
    );
}

#[test]
fn every_named_color_is_its_xterm_index() {
    let named = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    for (index, color) in (0u8..).zip(named) {
        assert_eq!(screen_color(color), screen::Color::Indexed(index), "{color:?}");
    }
    assert_eq!(screen_color(Color::Reset), screen::Color::Default);
}

#[test]
fn running_it_closes_the_palette_and_asks_for_the_next_frame() {
    let mut app = TestApp::new(Config::default());
    assert_eq!(search(&mut app, "screensh"), ["Screenshot"]);
    let focus = app.app().main_screen().map(crate::screens::main::MainScreen::focus);
    app.press(&["enter"]);
    assert_eq!(app.app().modals(), Vec::<ModalKind>::new());
    assert_eq!(
        app.app().main_screen().map(crate::screens::main::MainScreen::focus),
        focus
    );
    // Nothing runs until the loop has drawn the frame without the palette.
    assert!(!app.effects().iter().any(|e| matches!(e, Effect::SaveScreenshot { .. })));
    let request = app.app_mut().take_screenshot_request().expect("a pending request");
    // The fake clock's local time, 2026-10-25 12:00:00 exactly.
    assert_eq!(request.file_name, "LazyGithub_2026-10-25T12_00_00.svg");
    assert_eq!(request.options.title, "LazyGithub");
    // The theme's background and foreground: textual-dark's `#121212` and `#e0e0e0`.
    assert_eq!(request.options.default_bg, (0x12, 0x12, 0x12));
    assert_eq!(request.options.default_fg, (0xe0, 0xe0, 0xe0));
    // Taken once.
    assert!(app.app_mut().take_screenshot_request().is_none());

    let frame = app.render(160, 48);
    match request.into_effect(&frame) {
        Effect::SaveScreenshot { file_name, screen, .. } => {
            assert_eq!(file_name, "LazyGithub_2026-10-25T12_00_00.svg");
            assert_eq!((screen.width(), screen.height()), (160, 48));
            assert!(!screen.to_text().contains("Search for commands"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_saved_screenshot_raises_the_success_toast() {
    let mut app = TestApp::new(Config::default());
    let path = PathBuf::from("/home/u/Downloads/LazyGithub_2026-10-25T12_00_00.svg");
    app.send(Event::ScreenshotSaved(Ok(path)));
    let toast = app.app().toasts().iter().last().expect("a toast").clone();
    assert_eq!(toast.severity, Severity::Information);
    assert_eq!(toast.title.as_deref(), Some("Screenshot"));
    assert_eq!(
        toast.message.to_string(),
        "Saved screenshot to '/home/u/Downloads/LazyGithub_2026-10-25T12_00_00.svg'"
    );
    // The quoted path in `$text-success` (textual-dark's `#8ad4a1`); the rest is the toast's own.
    assert_eq!(toast.message.spans[0].style.fg, None);
    assert_eq!(toast.message.spans[1].style.fg, Some(Color::Rgb(0x8a, 0xd4, 0xa1)));
    insta::assert_snapshot!("saved_toast", toast_corner(&mut app));
}

#[test]
fn a_failed_screenshot_raises_the_error_toast() {
    let mut app = TestApp::new(Config::default());
    let error = ScreenshotError {
        path: PathBuf::from("/home/u/Downloads/LazyGithub_2026-10-25T12_00_00.svg"),
        message: "No such file or directory (os error 2)".to_owned(),
    };
    app.send(Event::ScreenshotSaved(Err(error)));
    let toast = app.app().toasts().iter().last().expect("a toast").clone();
    assert_eq!(toast.severity, Severity::Error);
    assert_eq!(toast.title.as_deref(), Some("Screenshot"));
    assert_eq!(
        toast.message.to_string(),
        "Failed to save screenshot to '/home/u/Downloads/LazyGithub_2026-10-25T12_00_00.svg': \
         No such file or directory (os error 2)"
    );
    insta::assert_snapshot!("failed_toast", toast_corner(&mut app));
}

#[test]
fn save_writes_the_svg_and_never_creates_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = Screen::new(3, 1);
    screen.cell_mut(0, 0).unwrap().symbol = "a".to_owned();
    let options = svg_options(&Theme::default());

    let path = save(dir.path(), "shot.svg", &screen, &options).unwrap();
    assert_eq!(path, dir.path().join("shot.svg"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), screen.to_svg(&options));
    // An existing file is replaced.
    std::fs::write(&path, "old").unwrap();
    save(dir.path(), "shot.svg", &screen, &options).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("<svg"));

    let missing = dir.path().join("Downloads");
    let error = save(&missing, "shot.svg", &screen, &options).unwrap_err();
    assert_eq!(error.path, missing.join("shot.svg"));
    assert!(!error.message.is_empty());
    assert!(!missing.exists());
}

#[test]
fn without_a_home_the_save_fails_with_a_message() {
    let env = DownloadsEnv {
        home: None,
        xdg_config_home: None,
    };
    let error = env
        .save("shot.svg", &Screen::new(1, 1), &svg_options(&Theme::default()))
        .unwrap_err();
    assert_eq!(error.path, Path::new("shot.svg"));
    assert_eq!(error.message, "could not determine your home directory");
}

#[test]
fn ansi_themes_fall_back_to_fixed_default_colors() {
    let options = svg_options(&Theme::by_name("ansi-dark").unwrap());
    assert_eq!(options.default_fg, (0xd4, 0xd4, 0xd4));
    assert_eq!(options.default_bg, (0x1e, 0x1e, 0x1e));
    let options = svg_options(&Theme::by_name("ansi-light").unwrap());
    assert_eq!(options.default_fg, (0x1e, 0x1e, 0x1e));
    assert_eq!(options.default_bg, (0xff, 0xff, 0xff));
}
