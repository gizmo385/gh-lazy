//! Terminal hyperlinks (OSC 8): text the terminal itself opens on ctrl+click (cmd+click), as Textual writes for
//! `[link=…]` markup (#356). Only a few spans are terminal hyperlinks (the run Overview's `(#N)`, the user profile's
//! `@login`); the in-app links are [`LinkMap`]'s.
//!
//! ratatui's buffer has no room for a hyperlink, so they're written beside it:
//!
//! 1. Views record them into the frame's hyperlink sink as they draw ([`RenderCx::hyperlinks`]).
//! 2. [`Hyperlinks`] keeps the cells each one drew, and drops the cells anything drew over afterwards (a dialog, the
//!    dim under it, a toast).
//! 3. Once the frame is drawn, [`HyperlinkWriter`] draws those cells again, wrapped in `ESC ] 8 ; … ; url ESC \` and
//!    `ESC ] 8 ; ; ESC \`, through a [`HyperlinkBackend`].
//!
//! Nothing reaches the buffer, so snapshots never hold an escape, and ratatui's `TestBackend` writes nothing for them.
//!
//! [`RenderCx::hyperlinks`]: crate::component::RenderCx::hyperlinks

use std::ffi::OsStr;
use std::io::Write as _;
use std::sync::Arc;

use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
use ratatui::buffer::{Buffer, Cell, CellWidth as _};
use ratatui::layout::Rect;

use crate::backend::WideEmojiBackend;
use crate::components::link_map::LinkMap;

/// Ends the hyperlink the cells written since the last open belong to.
const CLOSE: &str = "\x1b]8;;\x1b\\";

/// The cells one terminal hyperlink covers on one screen row, as they were drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct HyperlinkRun {
    /// The column of its first cell.
    pub x: u16,
    /// Its row.
    pub y: u16,
    /// Where it goes.
    pub url: Arc<str>,
    /// Its cells, left to right from `x`.
    pub cells: Vec<Cell>,
}

impl HyperlinkRun {
    /// Whether the cell at `(x, y)` is one of the run's.
    fn contains(&self, x: u16, y: u16) -> bool {
        y == self.y && x >= self.x && usize::from(x - self.x) < self.cells.len()
    }

    /// The run's columns.
    fn columns(&self) -> std::ops::Range<u16> {
        // A run is cut to the buffer, so its length fits a u16.
        let width = u16::try_from(self.cells.len()).unwrap_or(u16::MAX);
        self.x..self.x.saturating_add(width)
    }
}

/// The terminal hyperlinks one frame shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hyperlinks {
    runs: Vec<HyperlinkRun>,
}

impl Hyperlinks {
    /// Takes the hyperlinks a layer recorded into `links`, with the cells of `buf` they cover as the layer drew them.
    /// Call it right after the layer draws, before anything is drawn over it.
    pub fn capture(&mut self, buf: &Buffer, links: &LinkMap) {
        for link in links.iter() {
            let row = Rect::new(link.x, link.y, link.width, 1).intersection(buf.area);
            if row.is_empty() {
                continue;
            }
            let cells = (row.left()..row.right()).map(|x| buf[(x, row.y)].clone()).collect();
            self.runs.push(HyperlinkRun {
                x: row.x,
                y: row.y,
                url: Arc::clone(&link.target),
                cells,
            });
        }
    }

    /// Keeps only the cells `buf` still shows as they were captured: what was drawn over since (a dialog, the dim
    /// under one, a toast, a hint label) is no longer the link's text, so it isn't linked. A run broken in the middle
    /// becomes two.
    pub fn retain_drawn(&mut self, buf: &Buffer) {
        let mut kept = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            let mut part: Option<HyperlinkRun> = None;
            for (x, cell) in run.columns().zip(run.cells) {
                if buf.cell((x, run.y)) != Some(&cell) {
                    kept.extend(part.take());
                    continue;
                }
                match &mut part {
                    Some(part) => part.cells.push(cell),
                    None => {
                        part = Some(HyperlinkRun {
                            x,
                            y: run.y,
                            url: Arc::clone(&run.url),
                            cells: vec![cell],
                        });
                    }
                }
            }
            kept.extend(part);
        }
        self.runs = kept;
    }

    /// The runs, in the order they were recorded.
    pub fn runs(&self) -> &[HyperlinkRun] {
        &self.runs
    }

    /// Whether the frame shows no terminal hyperlink.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// One write after a frame: cells, drawn as a hyperlink when `open` is the sequence that starts one.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkWrite {
    /// `ESC ] 8 ; id=… ; url ESC \`, or `None` to draw the cells with no link.
    pub open: Option<String>,
    /// The cells, as ratatui hands them to a backend.
    pub cells: Vec<(u16, u16, Cell)>,
}

/// Writes each frame's terminal hyperlinks after ratatui draws it, and unlinks the cells that were linked in the frame
/// before and no longer are: a terminal keeps a cell's link until the cell is written again.
#[derive(Debug)]
pub struct HyperlinkWriter {
    enabled: bool,
    /// The cells linked by the last writes, as `(x, y)`.
    linked: Vec<(u16, u16)>,
}

impl HyperlinkWriter {
    /// A writer that writes hyperlinks when `enabled`, else nothing at all.
    pub const fn new(enabled: bool) -> Self {
        Self {
            enabled,
            linked: Vec::new(),
        }
    }

    /// A writer for the terminal `TERM` names.
    pub fn for_terminal() -> Self {
        Self::new(supports_hyperlinks(std::env::var_os("TERM").as_deref()))
    }

    /// What to write after `buf` was drawn showing `links`: the cells linked last time that aren't now, with no link,
    /// then each run as a hyperlink. Every run is written each frame: ratatui's diff writes over a link's cells when
    /// they change, and redraws the whole screen after a resize, either of which unlinks them.
    pub fn prepare(&mut self, buf: &Buffer, links: &Hyperlinks) -> Vec<LinkWrite> {
        if !self.enabled {
            return Vec::new();
        }
        let runs: Vec<(&HyperlinkRun, String)> = links
            .runs()
            .iter()
            .filter_map(|run| open_sequence(&run.url).map(|open| (run, open)))
            .collect();
        let linked_now = |x: u16, y: u16| runs.iter().any(|(run, _)| run.contains(x, y));

        let mut writes = Vec::with_capacity(runs.len() + 1);
        let stale: Vec<(u16, u16)> = self
            .linked
            .iter()
            .copied()
            .filter(|&(x, y)| !linked_now(x, y))
            .collect();
        let unlinked = drawable(buf, stale);
        if !unlinked.is_empty() {
            writes.push(LinkWrite {
                open: None,
                cells: unlinked,
            });
        }
        self.linked.clear();
        for (run, open) in runs {
            let cells = drawable(buf, run.columns().map(|x| (x, run.y)));
            if cells.is_empty() {
                continue;
            }
            self.linked.extend(cells.iter().map(|&(x, y, _)| (x, y)));
            writes.push(LinkWrite {
                open: Some(open),
                cells,
            });
        }
        writes
    }

    /// Writes `writes` ([`Self::prepare`]) through `backend`, and flushes.
    ///
    /// # Errors
    ///
    /// The backend's, when it can't write.
    pub fn write<B: HyperlinkBackend>(backend: &mut B, writes: &[LinkWrite]) -> Result<(), B::Error> {
        if writes.is_empty() {
            return Ok(());
        }
        for write in writes {
            backend.draw_linked(write.open.as_deref(), &write.cells)?;
        }
        backend.flush()
    }
}

/// Whether a terminal whose `TERM` is `term` can take OSC 8. Most terminals either show the link or skip the sequence,
/// as they skip any OSC they don't know (Windows Terminal, conhost and tmux do too); the Linux console instead prints
/// the rest of it as text, and a dumb terminal has no escapes at all.
pub fn supports_hyperlinks(term: Option<&OsStr>) -> bool {
    !term.is_some_and(|term| term == "linux" || term == "dumb")
}

/// The sequence that starts a hyperlink to `url`, or `None` for a URL that can't go in one: OSC 8 takes printable
/// ASCII only, and a control character would end the sequence early and let the rest through as escapes.
///
/// The `id` is the URL's hash, so the rows of a wrapped link highlight together, as Textual's `id` does.
pub fn open_sequence(url: &str) -> Option<String> {
    let printable = !url.is_empty() && url.bytes().all(|byte| (0x21..=0x7e).contains(&byte));
    printable.then(|| format!("\x1b]8;id={:x};{url}\x1b\\", fnv1a(url)))
}

/// FNV-1a: a short hash that's the same in every run.
fn fnv1a(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The cells of `buf` at `positions`, as ratatui's diff would write them: inside the buffer, and leaving out the
/// cells a wide character to their left covers, as writing those would cut it in half.
fn drawable(buf: &Buffer, positions: impl IntoIterator<Item = (u16, u16)>) -> Vec<(u16, u16, Cell)> {
    positions
        .into_iter()
        .filter(|&(x, y)| !covered(buf, x, y))
        .filter_map(|(x, y)| buf.cell((x, y)).map(|cell| (x, y, cell.clone())))
        .collect()
}

/// Whether a wide character left of `(x, y)` covers that cell.
fn covered(buf: &Buffer, x: u16, y: u16) -> bool {
    // No character is wider than a few cells.
    (1..=3u16).any(|dx| {
        x.checked_sub(dx)
            .and_then(|left| buf.cell((left, y)))
            .is_some_and(|cell| cell.cell_width() > dx)
    })
}

/// A backend that can draw cells as a terminal hyperlink.
pub trait HyperlinkBackend: Backend {
    /// Draws `cells`, inside the hyperlink `open` starts when there is one, else with no link.
    ///
    /// # Errors
    ///
    /// The backend's, when it can't write.
    fn draw_linked(&mut self, open: Option<&str>, cells: &[(u16, u16, Cell)]) -> Result<(), Self::Error>;
}

/// The terminal: the sequences go out around the cells, which go through the emoji-safe draw.
impl<W: std::io::Write> HyperlinkBackend for WideEmojiBackend<CrosstermBackend<W>> {
    fn draw_linked(&mut self, open: Option<&str>, cells: &[(u16, u16, Cell)]) -> std::io::Result<()> {
        if let Some(open) = open {
            self.inner_mut().write_all(open.as_bytes())?;
        }
        self.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))?;
        if open.is_some() {
            self.inner_mut().write_all(CLOSE.as_bytes())?;
        }
        Ok(())
    }
}

/// Tests draw into a buffer, which has no hyperlinks, and holds these cells already.
impl HyperlinkBackend for TestBackend {
    fn draw_linked(&mut self, _open: Option<&str>, _cells: &[(u16, u16, Cell)]) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
