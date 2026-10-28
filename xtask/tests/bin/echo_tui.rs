//! A tiny TUI for the runner integration test (`tests/runner.rs`). It exercises the terminal queries the runner
//! answers, changes its screen for a while and then settles, and echoes keys and resizes:
//!
//! - row 0: `cpr <col>,<row> kbd <bool>`: the cursor position report after moving to (7, 0), and crossterm's kitty
//!   keyboard check, which needs the runner's DA1 reply to return quickly
//! - row 1: `tick <n>` every 40 ms for 8 ticks, then `settled`
//! - row 2: `size <cols>x<rows>`
//! - row 3: `keys: ...`, each key as its char or `<Name>`
//! - row 4: `mouse: ...`, the last few mouse events as `<kind>@<col>,<row>` (0-based)
//!
//! Mouse capture is on (crossterm's any-motion SGR mode) unless `ECHO_TUI_MOUSE` says otherwise: `off` leaves it off,
//! and `press` turns on press/release reporting without motion (`?1000h ?1006h`).
//!
//! Ctrl+B draws a box titled with the process id at rows 5 to 7, so two runs differ (`tests/record.rs`). Ctrl+D
//! exits with code 3. `--version` prints a version and exits, as fake-gh's installer and `--record` expect of the
//! programs it stands in for.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::{cursor, execute, queue, style, terminal};

fn line(out: &mut impl Write, row: u16, text: &str) -> io::Result<()> {
    queue!(
        out,
        cursor::MoveTo(0, row),
        terminal::Clear(terminal::ClearType::CurrentLine),
        style::Print(text)
    )?;
    out.flush()
}

fn key_name(code: KeyCode, mods: KeyModifiers) -> String {
    match code {
        KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => format!("<C-{c}>"),
        KeyCode::Char(c) if mods.contains(KeyModifiers::ALT) => format!("<A-{c}>"),
        KeyCode::Char(c) => c.to_string(),
        other => format!("<{other:?}>"),
    }
}

fn mouse_name(kind: MouseEventKind) -> String {
    match kind {
        MouseEventKind::Down(MouseButton::Left) => "down".into(),
        MouseEventKind::Up(MouseButton::Left) => "up".into(),
        MouseEventKind::Moved => "move".into(),
        MouseEventKind::ScrollUp => "wheel-up".into(),
        MouseEventKind::ScrollDown => "wheel-down".into(),
        MouseEventKind::ScrollLeft => "wheel-left".into(),
        MouseEventKind::ScrollRight => "wheel-right".into(),
        other => format!("{other:?}"),
    }
}

/// How many mouse events row 4 shows, so it stays on one row of an 80-column screen.
const MOUSE_EVENTS: usize = 4;

fn main() -> io::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("echo_tui 1.0");
        return Ok(());
    }
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    match std::env::var("ECHO_TUI_MOUSE").as_deref() {
        Ok("off") => {}
        Ok("press") => out.write_all(b"\x1b[?1000h\x1b[?1006h")?,
        _ => execute!(out, crossterm::event::EnableMouseCapture)?,
    }
    execute!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(7, 0))?;
    let (col, row) = cursor::position()?;
    let kbd = terminal::supports_keyboard_enhancement()?;
    line(&mut out, 0, &format!("cpr {col},{row} kbd {kbd}"))?;

    for n in 1..=8 {
        line(&mut out, 1, &format!("tick {n}"))?;
        std::thread::sleep(Duration::from_millis(40));
    }
    line(&mut out, 1, "settled")?;
    let (cols, rows) = terminal::size()?;
    line(&mut out, 2, &format!("size {cols}x{rows}"))?;

    let mut keys = String::from("keys: ");
    line(&mut out, 3, &keys)?;
    let mut mouse: Vec<String> = Vec::new();
    line(&mut out, 4, "mouse:")?;
    loop {
        match crossterm::event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    terminal::disable_raw_mode()?;
                    std::process::exit(3);
                }
                if key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    let title = format!("┌─ pid {} ", std::process::id());
                    let pad = 23usize.saturating_sub(title.chars().count());
                    line(&mut out, 5, &format!("{title}{}┐", "─".repeat(pad)))?;
                    line(&mut out, 6, &format!("│{:22}│", " boxed"))?;
                    line(&mut out, 7, &format!("└{}┘", "─".repeat(22)))?;
                    continue;
                }
                keys.push_str(&key_name(key.code, key.modifiers));
                line(&mut out, 3, &keys)?;
            }
            Event::Mouse(event) => {
                mouse.push(format!("{}@{},{}", mouse_name(event.kind), event.column, event.row));
                let skip = mouse.len().saturating_sub(MOUSE_EVENTS);
                line(&mut out, 4, &format!("mouse: {}", mouse[skip..].join(" ")))?;
            }
            Event::Resize(cols, rows) => line(&mut out, 2, &format!("size {cols}x{rows}"))?,
            _ => {}
        }
    }
}
