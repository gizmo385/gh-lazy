//! A tiny TUI for the runner integration test (`tests/runner.rs`). It exercises the terminal queries the runner
//! answers, changes its screen for a while and then settles, and echoes keys and resizes:
//!
//! - row 0: `cpr <col>,<row> kbd <bool>`: the cursor position report after moving to (7, 0), and crossterm's kitty
//!   keyboard check, which needs the runner's DA1 reply to return quickly
//! - row 1: `tick <n>` every 40 ms for 8 ticks, then `settled`
//! - row 2: `size <cols>x<rows>`
//! - row 3: `keys: ...`, each key as its char or `<Name>`
//!
//! Ctrl+D exits with code 3.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
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

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
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
    loop {
        match crossterm::event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    terminal::disable_raw_mode()?;
                    std::process::exit(3);
                }
                keys.push_str(&key_name(key.code, key.modifiers));
                line(&mut out, 3, &keys)?;
            }
            Event::Resize(cols, rows) => line(&mut out, 2, &format!("size {cols}x{rows}"))?,
            _ => {}
        }
    }
}
