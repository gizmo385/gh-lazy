//! Which cells a frame links, what the writer writes for them across frames, and the bytes a terminal gets.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::{Terminal, TerminalOptions, Viewport};

use super::*;
use crate::components::link_map::ScreenLink;

const URL: &str = "https://github.com/o/r/actions/runs/7";

fn buffer(rows: &[&str]) -> Buffer {
    let width = rows
        .iter()
        .map(|row| unicode_width::UnicodeWidthStr::width(*row))
        .max()
        .unwrap_or(0);
    let mut buf = Buffer::empty(Rect::new(
        0,
        0,
        u16::try_from(width).unwrap(),
        u16::try_from(rows.len()).unwrap(),
    ));
    for (y, row) in rows.iter().enumerate() {
        buf.set_string(0, u16::try_from(y).unwrap(), row, Style::new());
    }
    buf
}

fn links(spans: &[(u16, u16, u16, &str)]) -> LinkMap {
    let mut map = LinkMap::default();
    for &(x, y, width, target) in spans {
        map.push(ScreenLink {
            x,
            y,
            width,
            target: target.into(),
        });
    }
    map
}

/// Each run as `(x, y, text, url)`.
fn runs(hyperlinks: &Hyperlinks) -> Vec<(u16, u16, String, String)> {
    hyperlinks
        .runs()
        .iter()
        .map(|run| {
            let text = run.cells.iter().map(Cell::symbol).collect();
            (run.x, run.y, text, run.url.to_string())
        })
        .collect()
}

/// Each write as `(linked, text at its cells)`.
fn writes(writes: &[LinkWrite]) -> Vec<(bool, String)> {
    writes
        .iter()
        .map(|write| {
            let text = write.cells.iter().map(|(_, _, cell)| cell.symbol()).collect();
            (write.open.is_some(), text)
        })
        .collect()
}

fn captured(buf: &Buffer, spans: &[(u16, u16, u16, &str)]) -> Hyperlinks {
    let mut hyperlinks = Hyperlinks::default();
    hyperlinks.capture(buf, &links(spans));
    hyperlinks
}

#[test]
fn a_capture_keeps_the_cells_and_cuts_them_to_the_buffer() {
    let buf = buffer(&["run (#7)", "@login"]);
    let hyperlinks = captured(
        &buf,
        &[(4, 0, 4, URL), (0, 1, 20, "https://github.com/login"), (0, 5, 3, URL)],
    );
    assert_eq!(
        runs(&hyperlinks),
        [
            (4, 0, "(#7)".to_owned(), URL.to_owned()),
            (0, 1, "@login  ".to_owned(), "https://github.com/login".to_owned()),
        ]
    );
}

#[test]
fn cells_drawn_over_after_the_capture_are_no_longer_linked() {
    let mut buf = buffer(&["(#1234)", "@login"]);
    let mut hyperlinks = captured(&buf, &[(0, 0, 7, URL), (0, 1, 6, URL)]);
    // A toast's edge over the middle of the first, and a dim (a new color) over the whole second.
    buf.set_string(3, 0, "X", Style::new());
    buf.set_style(Rect::new(0, 1, 6, 1), Style::new().fg(Color::DarkGray));
    hyperlinks.retain_drawn(&buf);
    assert_eq!(
        runs(&hyperlinks),
        [
            (0, 0, "(#1".to_owned(), URL.to_owned()),
            (4, 0, "34)".to_owned(), URL.to_owned())
        ]
    );
}

#[test]
fn every_run_is_written_each_frame_and_unlinked_once_gone() {
    let buf = buffer(&["(#7) and @login"]);
    let both = captured(&buf, &[(0, 0, 4, URL), (9, 0, 6, "https://github.com/login")]);
    let mut writer = HyperlinkWriter::new(true);
    let first = writer.prepare(&buf, &both);
    assert_eq!(writes(&first), [(true, "(#7)".to_owned()), (true, "@login".to_owned())]);
    assert_eq!(
        first,
        writer.prepare(&buf, &both),
        "the same frame writes the same again"
    );

    // `@login` is gone: its cells are written once with no link, then left alone.
    let one = captured(&buf, &[(0, 0, 4, URL)]);
    assert_eq!(
        writes(&writer.prepare(&buf, &one)),
        [(false, "@login".to_owned()), (true, "(#7)".to_owned())]
    );
    assert_eq!(writes(&writer.prepare(&buf, &one)), [(true, "(#7)".to_owned())]);
    assert!(
        writer
            .prepare(&buf, &Hyperlinks::default())
            .iter()
            .all(|write| write.open.is_none())
    );
    assert!(writer.prepare(&buf, &Hyperlinks::default()).is_empty());
}

#[test]
fn a_disabled_writer_writes_nothing() {
    let buf = buffer(&["(#7)"]);
    let mut writer = HyperlinkWriter::new(false);
    assert!(writer.prepare(&buf, &captured(&buf, &[(0, 0, 4, URL)])).is_empty());
}

#[test]
fn a_url_that_cant_go_in_the_sequence_is_left_unlinked() {
    let buf = buffer(&["abcdef"]);
    let mut writer = HyperlinkWriter::new(true);
    let hyperlinks = captured(
        &buf,
        &[
            (0, 0, 2, "https://x\x1b]0;pwned\x07"),
            (2, 0, 2, "https://a b"),
            (4, 0, 2, URL),
        ],
    );
    assert_eq!(writes(&writer.prepare(&buf, &hyperlinks)), [(true, "ef".to_owned())]);
}

#[test]
fn the_cell_a_wide_character_covers_is_not_written() {
    let buf = buffer(&["漢字ab"]);
    let mut writer = HyperlinkWriter::new(true);
    let written = writer.prepare(&buf, &captured(&buf, &[(0, 0, 5, URL)]));
    let columns: Vec<u16> = written[0].cells.iter().map(|&(x, _, _)| x).collect();
    assert_eq!(columns, [0, 2, 4]);
}

#[test]
fn the_open_sequence_carries_a_stable_id_and_only_printable_ascii() {
    let open = open_sequence(URL).unwrap();
    assert!(open.starts_with("\x1b]8;id="), "{open:?}");
    assert!(open.ends_with(&format!(";{URL}\x1b\\")), "{open:?}");
    assert_eq!(Some(open), open_sequence(URL));
    assert_ne!(open_sequence(URL), open_sequence("https://github.com/other"));
    for bad in [
        "",
        "https://a b",
        "https://a\x1b\\",
        "https://a\x07",
        "https://ä.example",
    ] {
        assert_eq!(open_sequence(bad), None, "{bad:?}");
    }
}

#[test]
fn the_linux_console_and_dumb_terminals_get_no_hyperlinks() {
    assert!(supports_hyperlinks(None));
    assert!(supports_hyperlinks(Some(OsStr::new("xterm-256color"))));
    assert!(supports_hyperlinks(Some(OsStr::new("xterm-kitty"))));
    assert!(!supports_hyperlinks(Some(OsStr::new("linux"))));
    assert!(!supports_hyperlinks(Some(OsStr::new("dumb"))));
}

/// Collects what the backend writes.
#[derive(Clone, Default)]
struct Bytes(Rc<RefCell<Vec<u8>>>);

impl io::Write for Bytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_terminal_gets_the_cells_between_the_open_and_close_sequences() {
    let bytes = Bytes::default();
    let mut backend = WideEmojiBackend::new(CrosstermBackend::new(bytes.clone()));
    let buf = buffer(&["x (#7)"]);
    let mut writer = HyperlinkWriter::new(true);
    let written = writer.prepare(&buf, &captured(&buf, &[(2, 0, 4, URL)]));
    HyperlinkWriter::write(&mut backend, &written).unwrap();

    let out = String::from_utf8(bytes.0.borrow().clone()).unwrap();
    let open = open_sequence(URL).unwrap();
    let start = out.find(&open).expect("the open sequence");
    let end = out.rfind(CLOSE).expect("the close sequence");
    assert!(start < end);
    let inside = &out[start + open.len()..end];
    // The cursor moves to the run, and its text follows; nothing outside the pair is printed text.
    assert!(inside.contains("\x1b[1;3H"), "{inside:?}");
    assert!(inside.contains("(#7)"), "{inside:?}");
    assert!(
        !out[..start].contains("(#7)") && !out[end..].contains("(#7)"),
        "{out:?}"
    );
}

/// Feeds what the backend writes to a `vt100` parser.
struct VtWriter(Rc<RefCell<vt100::Parser>>);

impl io::Write for VtWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_terminal_without_hyperlinks_shows_just_the_frame() {
    let parser = Rc::new(RefCell::new(vt100::Parser::new(
        2.try_into().unwrap(),
        12.try_into().unwrap(),
        0,
    )));
    let backend = WideEmojiBackend::new(CrosstermBackend::new(VtWriter(Rc::clone(&parser))));
    let viewport = Viewport::Fixed(Rect::new(0, 0, 12, 2));
    let mut terminal = Terminal::with_options(backend, TerminalOptions { viewport }).unwrap();
    let mut writer = HyperlinkWriter::new(true);
    for text in ["Run (#7)", "Run (#7)", "Run (#12)"] {
        let mut written = Vec::new();
        terminal
            .draw(|frame| {
                frame.buffer_mut().set_string(0, 0, text, Style::new().fg(Color::Green));
                frame.buffer_mut().set_string(0, 1, "@login", Style::new());
                let width = u16::try_from(text.len()).unwrap() - 4;
                let hyperlinks = captured(frame.buffer_mut(), &[(4, 0, width, URL), (0, 1, 6, URL)]);
                written = writer.prepare(frame.buffer_mut(), &hyperlinks);
            })
            .unwrap();
        HyperlinkWriter::write(terminal.backend_mut(), &written).unwrap();
        let screen = parser.borrow().screen().contents();
        assert_eq!(screen, format!("{text}\n@login"));
        assert_eq!(
            parser.borrow().screen().cell(0, 4).unwrap().fgcolor(),
            vt100::Color::Idx(2)
        );
    }
}
