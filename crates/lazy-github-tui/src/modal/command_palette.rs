//! The command palette (`app.command_palette`, `ctrl+p`): Textual's `CommandPalette`, a band across the top of the
//! dimmed screen with a search input and the commands whose names match it, best first.
//!
//! The commands are data ([`PaletteCommand`]) built by `App` when the palette opens, from the state at that moment.
//! Picking one closes the palette and hands its [`PaletteRun`] to the opener, which runs it on the main screen.

use std::time::Duration;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Dialog, DialogAction, FieldKind, FocusMove, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::command_log::{WHEEL_ROWS, Wheel};
use crate::components::option_nav;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::components::text_input::{INPUT_BINDINGS, InputOutcome, TextInput};
use crate::effect::{Effect, TimerId};
use crate::fuzzy::fuzzy_match;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::screens::main::Toggle;
use crate::theme::{Paint, TextStyle, Theme, ThemeColor};

/// The palette's screen bindings, in Textual's merged order: the modal screen's, then `CommandPalette.BINDINGS`.
pub const COMMAND_PALETTE_BINDINGS: &[BindingId] = dialog_bindings![
    TextualCommandPaletteLast,
    TextualCommandPaletteFirst,
    TextualCommandPaletteCursorDown,
    TextualCommandPaletteEscape,
    TextualCommandPalettePageDown,
    TextualCommandPalettePageUp,
    TextualCommandPaletteCursorUp,
];

/// The input's placeholder.
const PLACEHOLDER: &str = "Search for commands…";

/// The theme palette's placeholder (Textual's `search_themes`).
const THEMES_PLACEHOLDER: &str = "Search for themes…";

/// The band starts this far down (`margin-top: 3`), so the status summary stays in view above it.
const MARGIN_TOP: u16 = 3;

/// The input's rows: its blank border above and below the text.
const INPUT_ROWS: u16 = 3;

/// The search icon and where it starts: inside `#--input`'s border, after `margin-left: 1`.
const ICON: &str = "🔎";
const ICON_X: u16 = 2;

/// Where the input's text starts: after the icon and the input's blank left border (`padding-left: 0`).
const TEXT_X: u16 = 5;

/// The cells right of the text: the input's `padding-right: 2` and blank border, and `#--input`'s right edge.
const TEXT_RIGHT: u16 = 4;

/// Each hit's `padding: 0 2`.
const OPTION_PADDING: u16 = 2;

/// The list's `max-height: 70vh`, border included.
const MAX_LIST_PERCENT: u32 = 70;

/// How long a search that found nothing waits before saying so (`_NO_MATCHES_COUNTDOWN`).
const NO_MATCHES_AFTER: Duration = Duration::from_millis(500);

/// The row shown then.
const NO_MATCHES: &str = "No matches found";

/// Rich's `dim`, as Textual resolves it: the text color blended 66% of the way from the background.
const DIM_ALPHA: f64 = 0.66;

/// The `hkey black 50%` rules around the input.
const INPUT_RULE_ALPHA: f64 = 0.5;

/// What running a palette command does. The opener turns it into an [`Action`] with [`PaletteRun::action`] once
/// the palette has closed. #195 adds its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteRun {
    /// "Toggle Command Log", "Toggle Workflows", "Toggle Issues", "Toggle Pull Requests".
    Toggle(Toggle),
    /// "Debug Info": open the Debug Info modal.
    DebugInfo,
    /// "Refresh notifications": the poll's refresh, now.
    RefreshNotifications,
    /// "Keys": open the help panel, or close it, as `?` does.
    ToggleHelp,
    /// "Maximize": maximize the focused widget as `M` does, without the one-time toast.
    Maximize,
    /// "Minimize": back to the normal layout, as `escape` does.
    Minimize,
    /// "Quit".
    Quit,
    /// "Theme": open the theme palette.
    ChooseTheme,
    /// A theme palette hit: switch to the theme of that name for the session.
    SetTheme(&'static str),
    /// "Change Settings": open the Settings modal.
    ChangeSettings,
}

impl PaletteRun {
    /// The action that runs it on the main screen.
    pub fn action(self) -> Action {
        match self {
            PaletteRun::Toggle(toggle) => Action::Toggle(toggle),
            PaletteRun::DebugInfo => Action::ShowDebugInfo,
            PaletteRun::RefreshNotifications => Action::RefreshNotifications,
            PaletteRun::ToggleHelp => Action::ToggleHelp,
            PaletteRun::Maximize => Action::Maximize { toast: false },
            PaletteRun::Minimize => Action::Minimize,
            PaletteRun::Quit => Action::Quit,
            PaletteRun::ChooseTheme => Action::OpenThemePalette,
            PaletteRun::SetTheme(name) => Action::SetTheme(name),
            PaletteRun::ChangeSettings => Action::OpenSettings,
        }
    }
}

/// Where a command comes from: which ones the empty query lists first, and the order of hits with equal scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    /// LazyGithub's own (the main screen's `MainScreenCommandProvider`).
    App,
    /// Textual's system commands (`App.get_system_commands`).
    System,
}

/// One command the palette offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteCommand {
    /// What the search matches and the hit shows in bold: "Keys".
    pub name: &'static str,
    /// The dimmed line under the name, if it has one. A palette whose commands have none draws one row per hit.
    pub help: Option<&'static str>,
    /// Where it comes from.
    pub source: CommandSource,
    /// What picking it does.
    pub run: PaletteRun,
}

/// The palette's own action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    /// The query changed: search again. `copied` is text a cut put on the clipboard.
    Edited {
        /// The cut's text, for [`Action::Copy`].
        copied: Option<String>,
    },
    /// `enter` or a click on a hit: close and run the highlighted hit, if there is one.
    Run,
    /// `escape` or a click outside the band: close, running nothing.
    Close,
}

/// One listed command.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    /// Its index in the palette's commands.
    command: usize,
    /// The `char` indices of its name drawn underlined: the letters the query matched.
    matched: Vec<usize>,
}

/// The "No matches found" row: hidden, waiting for its timer, or shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoMatches {
    Hidden,
    Pending,
    Shown,
}

/// The palette is done: with the command to run, or `None` when it closed without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Finished(Option<PaletteRun>);

/// Where the band's parts were drawn, for the mouse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Layout {
    /// Every row the band paints.
    band: Rect,
    /// The input's text row.
    text: Rect,
    /// The list's rows in view between its blank top and its `▁` bottom, scrollbar excluded. Empty while it's hidden.
    list: Rect,
    /// The list's scrollbar, when it overflows.
    bar: Option<Rect>,
}

impl Layout {
    /// The band on a screen of `area`, with `rows` rows of list content (none while the list is hidden).
    fn new(area: Rect, rows: usize) -> Self {
        let top = area.y.saturating_add(MARGIN_TOP);
        // `max-height` is a whole number of rows, border included.
        let max_rows = u16::try_from(u32::from(area.height) * MAX_LIST_PERCENT / 100).unwrap_or(u16::MAX);
        let shown = u16::try_from(rows).unwrap_or(u16::MAX).min(max_rows.saturating_sub(2));
        // The `▔` rule, the input, then the list between a blank row and a `▁` rule, or the input's own `▁` rule.
        let height = 1 + INPUT_ROWS + if rows > 0 { shown + 2 } else { 1 };
        let band = Rect::new(area.x, top, area.width, height).intersection(area);
        let text = Rect::new(
            area.x + TEXT_X,
            top + 2,
            area.width.saturating_sub(TEXT_X + TEXT_RIGHT),
            1,
        )
        .intersection(area);
        let mut list = Rect::new(area.x, top + 1 + INPUT_ROWS + 1, area.width, shown).intersection(area);
        let bar = (usize::from(shown) < rows).then(|| {
            list.width = list.width.saturating_sub(scrollbar::VERTICAL_WIDTH);
            Rect {
                x: list.right(),
                width: scrollbar::VERTICAL_WIDTH.min(area.width),
                ..list
            }
            .intersection(area)
        });
        Self { band, text, list, bar }
    }
}

/// The command palette. Its output is the picked command's [`PaletteRun`], or `None` when it closed without one.
#[derive(Debug)]
pub struct CommandPaletteDialog {
    input: TextInput,
    commands: Vec<PaletteCommand>,
    /// The rows each hit takes: its name over its help text, or the name alone when no command has help.
    rows_per_hit: usize,
    /// The listed commands, in order: every command for an empty query, else those whose names match it.
    hits: Vec<Hit>,
    /// The highlighted hit. Meaningless while there are none.
    highlight: usize,
    /// The list's first row in view.
    scroll: usize,
    no_matches: NoMatches,
    layout: Layout,
    /// Set once it's done: the picked command, or `None` for nothing.
    finished: Option<Finished>,
}

impl CommandPaletteDialog {
    /// A palette offering `commands`: LazyGithub's in their order, then the system commands in Textual's. The empty
    /// query lists the system commands sorted by name, then LazyGithub's (Py bug B87's answer), the first
    /// highlighted.
    pub fn new(commands: Vec<PaletteCommand>) -> Self {
        Self::with_placeholder(commands, PLACEHOLDER)
    }

    /// Textual's theme palette (`search_themes`): every built-in theme by name, sorted, with no help lines. The
    /// empty query lists them in that order, the first highlighted rather than the current theme, and equal scores
    /// keep it. Picking one hands back [`PaletteRun::SetTheme`].
    pub fn themes() -> Self {
        let mut commands: Vec<PaletteCommand> = Theme::all()
            .map(|theme| PaletteCommand {
                name: theme.name,
                help: None,
                source: CommandSource::System,
                run: PaletteRun::SetTheme(theme.name),
            })
            .collect();
        commands.sort_by_key(|command| command.name);
        Self::with_placeholder(commands, THEMES_PLACEHOLDER)
    }

    fn with_placeholder(commands: Vec<PaletteCommand>, placeholder: &'static str) -> Self {
        let rows_per_hit = if commands.iter().any(|command| command.help.is_some()) {
            2
        } else {
            1
        };
        let mut palette = Self {
            input: TextInput::new(placeholder).borderless(),
            commands,
            rows_per_hit,
            hits: Vec::new(),
            highlight: 0,
            scroll: 0,
            no_matches: NoMatches::Hidden,
            layout: Layout::default(),
            finished: None,
        };
        palette.search();
        palette
    }

    /// The listed commands' names, in order.
    pub fn hit_names(&self) -> Vec<&'static str> {
        self.hits.iter().map(|hit| self.commands[hit.command].name).collect()
    }

    /// The highlighted command's name, when anything is listed.
    pub fn highlighted(&self) -> Option<&'static str> {
        self.hits.get(self.highlight).map(|hit| self.commands[hit.command].name)
    }

    /// Whether "No matches found" shows.
    pub fn shows_no_matches(&self) -> bool {
        self.no_matches == NoMatches::Shown
    }

    fn action(action: CommandPaletteAction) -> Action {
        Action::Dialog(DialogAction::CommandPalette(action))
    }

    /// The query: the input's text with surrounding whitespace stripped.
    fn query(&self) -> &str {
        self.input.value().trim()
    }

    /// Lists the commands for the current query, the first highlighted and scrolled into view. Equal scores keep the
    /// commands' order (Py bug B86's answer: LazyGithub's, then the system commands).
    fn search(&mut self) {
        let query = self.query();
        self.hits = if query.is_empty() {
            let mut system: Vec<usize> = self.indices(CommandSource::System).collect();
            system.sort_by_key(|&index| self.commands[index].name);
            system
                .into_iter()
                .chain(self.indices(CommandSource::App))
                .map(|command| Hit {
                    command,
                    matched: Vec::new(),
                })
                .collect()
        } else {
            let mut scored: Vec<(f64, Hit)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(command, c)| {
                    let found = fuzzy_match(query, c.name)?;
                    let matched = found.highlights(c.name);
                    Some((found.score, Hit { command, matched }))
                })
                .collect();
            // A stable sort, so ties stay in the commands' order.
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            scored.into_iter().map(|(_, hit)| hit).collect()
        };
        self.highlight = 0;
        self.scroll = 0;
    }

    fn indices(&self, source: CommandSource) -> impl Iterator<Item = usize> + '_ {
        (0..self.commands.len()).filter(move |&index| self.commands[index].source == source)
    }

    /// The query changed: lists its hits, and when a non-blank query found nothing, says so after a while. Any edit
    /// hides the row until then.
    fn research(&mut self, cx: &mut Cx<'_>) {
        self.search();
        let pending = self.no_matches == NoMatches::Pending;
        if self.hits.is_empty() && !self.query().is_empty() {
            self.no_matches = NoMatches::Pending;
            cx.effect(Effect::StartTimeout {
                id: TimerId::PaletteNoMatches,
                after: NO_MATCHES_AFTER,
            });
        } else {
            self.no_matches = NoMatches::Hidden;
            if pending {
                cx.effect(Effect::CancelTimer(TimerId::PaletteNoMatches));
            }
        }
    }

    /// Done, with `run` for the opener. A pending "No matches found" is dropped.
    fn finish(&mut self, run: Option<PaletteRun>, cx: &mut Cx<'_>) {
        if self.no_matches == NoMatches::Pending {
            cx.effect(Effect::CancelTimer(TimerId::PaletteNoMatches));
        }
        self.finished = Some(Finished(run));
    }

    /// The rows of list content: one or two per hit, or the "No matches found" row and none while the list is hidden.
    fn list_rows(&self) -> usize {
        if self.hits.is_empty() {
            usize::from(self.no_matches == NoMatches::Shown)
        } else {
            self.hits.len() * self.rows_per_hit
        }
    }

    /// The list's rows in view at the last draw: every row until then.
    fn window(&self) -> usize {
        if self.layout.list.is_empty() {
            self.list_rows()
        } else {
            usize::from(self.layout.list.height)
        }
    }

    /// Moves the highlight to `highlight` and scrolls its rows into view.
    fn move_to(&mut self, highlight: usize) {
        self.highlight = highlight;
        let rows = self.rows_per_hit;
        self.scroll = option_nav::scroll_into_view(self.scroll, highlight * rows, rows, self.window());
    }

    /// A binding of the palette's screen link that moves the highlight. Nothing moves without hits, so "No matches
    /// found" is never highlighted.
    fn navigate(&mut self, id: BindingId) {
        let len = self.hits.len();
        if len == 0 {
            return;
        }
        let (h, view, rows) = (self.highlight, self.window(), self.rows_per_hit);
        let to = match id {
            BindingId::TextualCommandPaletteCursorDown => option_nav::next(h, len),
            BindingId::TextualCommandPaletteCursorUp => option_nav::previous(h, len),
            BindingId::TextualCommandPalettePageDown => option_nav::page_down(h, len, rows, view),
            BindingId::TextualCommandPalettePageUp => option_nav::page_up(h, rows, view),
            BindingId::TextualCommandPaletteFirst => 0,
            BindingId::TextualCommandPaletteLast => len - 1,
            _ => return,
        };
        self.move_to(to);
    }

    /// The largest scroll offset: the last row at the bottom of the window.
    fn max_scroll(&self) -> usize {
        self.list_rows().saturating_sub(self.window())
    }

    /// Edits the input, and says how: an edit that changed the text searches again.
    fn edit(&mut self, edit: impl FnOnce(&mut TextInput) -> InputOutcome) -> Option<Action> {
        let before = self.input.value().to_owned();
        let outcome = edit(&mut self.input);
        let changed = self.input.value() != before;
        match outcome {
            InputOutcome::Submitted => Some(Self::action(CommandPaletteAction::Run)),
            InputOutcome::Copied(text) if changed => {
                Some(Self::action(CommandPaletteAction::Edited { copied: Some(text) }))
            }
            InputOutcome::Copied(text) => Some(Action::Copy(text)),
            InputOutcome::Changed if changed => Some(Self::action(CommandPaletteAction::Edited { copied: None })),
            InputOutcome::Changed => Some(Action::Consumed),
            InputOutcome::Ignored => None,
        }
    }
}

/// Whether the input's binding `id` is one of the scroll keys it inherits, which a one-line input can't use: Textual
/// skips them (`SkipAction`), so `up`, `down` and the page keys reach the palette's bindings.
fn is_scroll(id: BindingId) -> bool {
    matches!(
        id,
        BindingId::TextualScrollUp
            | BindingId::TextualScrollDown
            | BindingId::TextualScrollPageUp
            | BindingId::TextualScrollPageDown
            | BindingId::TextualScrollPageLeft
            | BindingId::TextualScrollPageRight
    )
}

impl Component for CommandPaletteDialog {
    type Action = CommandPaletteAction;

    fn bindings(&self) -> &'static [BindingId] {
        COMMAND_PALETTE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::TextualCommandPaletteEscape => Some(Self::action(CommandPaletteAction::Close)),
            BindingId::TextualCommandPaletteCursorDown
            | BindingId::TextualCommandPaletteCursorUp
            | BindingId::TextualCommandPalettePageDown
            | BindingId::TextualCommandPalettePageUp
            | BindingId::TextualCommandPaletteFirst
            | BindingId::TextualCommandPaletteLast => {
                self.navigate(id);
                Some(Action::Consumed)
            }
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        let scroll = keymap.match_in(INPUT_BINDINGS, key).is_some_and(is_scroll);
        !scroll && self.input.consumes_key(key, keymap)
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        self.edit(|input| input.handle_key(&chord, cx.keymap(), cx.clipboard()))
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        // A drag that started in the input, wherever the pointer is now.
        if self.input.captured_mouse(event) {
            return Some(Action::Consumed);
        }
        let position = Position::new(event.column, event.row);
        let Layout { band, text, list, .. } = self.layout;
        if let Some(wheel) = Wheel::from_mouse(&event) {
            if list.contains(position) {
                self.scroll = match wheel {
                    Wheel::Up => self.scroll.saturating_sub(WHEEL_ROWS),
                    Wheel::Down => (self.scroll + WHEEL_ROWS).min(self.max_scroll()),
                    Wheel::Left | Wheel::Right => self.scroll,
                };
            }
            return Some(Action::Consumed);
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Some(Action::Consumed);
        }
        // Textual's `_on_click`: a click on the palette's own screen, outside the band, dismisses it.
        if !band.contains(position) {
            return Some(Self::action(CommandPaletteAction::Close));
        }
        if text.contains(position) {
            self.input.press(text, event.column);
            return Some(Action::Consumed);
        }
        if list.contains(position) && !self.hits.is_empty() {
            let line = self.scroll + usize::from(event.row - list.y);
            let hit = line / self.rows_per_hit;
            if hit < self.hits.len() {
                self.highlight = hit;
                return Some(Self::action(CommandPaletteAction::Run));
            }
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: CommandPaletteAction, cx: &mut Cx<'_>) {
        match action {
            CommandPaletteAction::Edited { copied } => {
                if let Some(text) = copied {
                    cx.dispatch(Action::Copy(text));
                }
                self.research(cx);
            }
            // Nothing to run (no hits, or only "No matches found"): the palette stays open.
            CommandPaletteAction::Run => {
                if let Some(hit) = self.hits.get(self.highlight) {
                    let run = self.commands[hit.command].run;
                    self.finish(Some(run), cx);
                }
            }
            CommandPaletteAction::Close => self.finish(None, cx),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let rows = self.list_rows();
        self.layout = Layout::new(area, rows);
        self.scroll = self.scroll.min(self.max_scroll());
        let Layout { band, text, list, bar } = self.layout;
        if band.is_empty() {
            return;
        }
        let band_bg = if theme.dark { p.panel_darken_1 } else { p.surface }.over(theme.background());
        let fill = Style::new()
            .fg(theme.color(p.foreground, band_bg))
            .bg(theme.paint(band_bg));
        let buf = frame.buffer_mut();
        for position in band.positions() {
            buf[position].reset();
            buf[position].set_symbol(" ").set_style(fill);
        }
        let rule = |alpha: f64| fill.fg(theme.color(ThemeColor::BLACK.multiply_alpha(alpha), band_bg));
        let rule_row = |buf: &mut Buffer, y: u16, symbol: &str, style: Style| {
            if y < band.bottom() {
                for x in band.left()..band.right() {
                    buf[(x, y)].set_symbol(symbol).set_style(style);
                }
            }
        };
        rule_row(buf, band.y, "▔", rule(INPUT_RULE_ALPHA));

        // `SearchIcon { color: #000 }`.
        let icon_row = band.y + 2;
        if icon_row < band.bottom() && band.width > ICON_X + 1 {
            buf.set_string(
                band.x + ICON_X,
                icon_row,
                ICON,
                fill.fg(theme.color(ThemeColor::BLACK, band_bg)),
            );
        }
        self.input.render(buf, text, theme, band_bg, true);

        let input_bottom = band.y + 1 + INPUT_ROWS;
        if rows == 0 {
            rule_row(buf, input_bottom, "▁", rule(INPUT_RULE_ALPHA));
            return;
        }
        let styles = ListStyles::new(theme, band_bg);
        if self.hits.is_empty() {
            draw_no_matches(buf, list, &styles);
        } else {
            self.draw_hits(buf, list, &styles);
        }
        if let Some(bar) = bar {
            let state = ScrollbarState {
                content: rows,
                window: usize::from(list.height),
                position: self.scroll,
            };
            scrollbar::render(buf, bar, Axis::Vertical, state, styles.scrollbar);
        }
        // `CommandList { border-bottom: hkey black }`.
        rule_row(buf, list.bottom(), "▁", rule(1.0));
    }
}

impl CommandPaletteDialog {
    /// The hits' rows in view: each name, over its help text when hits have two rows, the highlighted hit on the cursor's colors.
    fn draw_hits(&self, buf: &mut Buffer, list: Rect, styles: &ListStyles) {
        let content = Rect {
            x: list.x + OPTION_PADDING,
            width: list.width.saturating_sub(2 * OPTION_PADDING),
            ..list
        };
        for (y, line) in (list.top()..list.bottom()).zip(self.scroll..) {
            let index = line / self.rows_per_hit;
            let Some(hit) = self.hits.get(index) else { break };
            let command = &self.commands[hit.command];
            let row = if index == self.highlight {
                &styles.highlighted
            } else {
                &styles.option
            };
            for x in list.left()..list.right() {
                buf[(x, y)].set_symbol(" ").set_style(row.name);
            }
            let text: Line<'_> = if line % self.rows_per_hit == 0 {
                command
                    .name
                    .chars()
                    .enumerate()
                    .map(|(at, c)| {
                        let style = if hit.matched.contains(&at) {
                            row.name.add_modifier(Modifier::UNDERLINED)
                        } else {
                            row.name
                        };
                        Span::styled(c.to_string(), style)
                    })
                    .collect()
            } else {
                Line::styled(command.help.unwrap_or_default(), row.help)
            };
            buf.set_line(content.x, y, &text, content.width);
        }
    }
}

/// "No matches found", centered and dimmed, on the list's one row.
fn draw_no_matches(buf: &mut Buffer, list: Rect, styles: &ListStyles) {
    if list.is_empty() {
        return;
    }
    let width = u16::try_from(NO_MATCHES.chars().count()).unwrap_or(u16::MAX);
    let x = list.x + list.width.saturating_sub(width) / 2;
    buf.set_stringn(
        x,
        list.y,
        NO_MATCHES,
        usize::from(list.right().saturating_sub(x)),
        styles.disabled,
    );
}

/// One hit's two rows.
struct HitStyles {
    /// The name, and the row's background.
    name: Style,
    /// The help text: `dim not bold`.
    help: Style,
}

/// The list's styles, from `CommandList.DEFAULT_CSS` and `CommandPalette`'s component classes.
struct ListStyles {
    option: HitStyles,
    highlighted: HitStyles,
    /// The disabled "No matches found" option: `$text-disabled`, not bold.
    disabled: Style,
    scrollbar: ScrollbarColors,
}

impl ListStyles {
    fn new(theme: &Theme, band: Paint) -> Self {
        let p = theme.palette;
        let hit = |fg: ThemeColor, bg: ThemeColor, text_style: Option<TextStyle>| {
            let bg = bg.over(band);
            // `CommandList > .option-list--option { color: $foreground; text-style: bold }`, then the highlight's.
            let mut name = Style::new()
                .fg(theme.color(fg, bg))
                .bg(theme.paint(bg))
                .add_modifier(Modifier::BOLD);
            if let Some(text_style) = text_style {
                name = text_style.apply(name);
            }
            let help = match fg {
                // Textual can't blend the terminal's own colors, so the terminal dims them.
                ThemeColor::Ansi(_) => name.add_modifier(Modifier::DIM),
                fg => name.fg(theme.color(fg.multiply_alpha(DIM_ALPHA), bg)),
            }
            .remove_modifier(Modifier::BOLD);
            HitStyles { name, help }
        };
        // `CommandList:ansi` highlights with the focused cursor's colors.
        let highlighted = if theme.ansi {
            hit(
                p.block_cursor_foreground,
                p.block_cursor_background,
                Some(p.block_cursor_text_style),
            )
        } else {
            hit(
                p.block_cursor_blurred_foreground,
                p.block_cursor_blurred_background,
                Some(p.block_cursor_blurred_text_style),
            )
        };
        let back = p.scrollbar_background.over(band);
        Self {
            option: hit(p.foreground, ThemeColor::Transparent, None),
            highlighted,
            disabled: Style::new()
                .fg(theme.color(p.text_disabled, band))
                .bg(theme.paint(band)),
            scrollbar: ScrollbarColors {
                bar: theme.color(p.scrollbar, back),
                back: theme.paint(back),
                fg: theme.color(p.foreground, band),
                bg: theme.paint(band),
            },
        }
    }
}

impl Dialog for CommandPaletteDialog {
    type Output = Option<PaletteRun>;

    fn kind(&self) -> ModalKind {
        ModalKind::CommandPalette
    }

    fn own_action(&self, action: DialogAction) -> Option<CommandPaletteAction> {
        match action {
            DialogAction::CommandPalette(action) => Some(action),
            _ => None,
        }
    }

    /// The input is the only focus stop: `tab` and `shift+tab` do nothing.
    fn move_focus(&mut self, _direction: FocusMove) {}

    fn size(&self, available: Rect) -> (u16, u16) {
        (available.width, available.height)
    }

    fn frameless(&self) -> bool {
        true
    }

    /// The main screen's footer stays on the bottom row, dimmed.
    fn shows_footer(&self) -> bool {
        false
    }

    fn take_output(&mut self) -> Option<Option<PaletteRun>> {
        self.finished.take().map(|Finished(run)| run)
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        vec![FieldKind::TextInput.link(self.kind())]
    }

    fn captures_mouse(&self) -> bool {
        self.input.is_selecting()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        self.edit(|input| {
            input.handle_paste(text);
            InputOutcome::Changed
        })
    }

    fn on_timer(&mut self, id: TimerId, _cx: &mut Cx<'_>) -> bool {
        if id != TimerId::PaletteNoMatches {
            return false;
        }
        // A tick from before the last edit finds the row no longer pending.
        if self.no_matches == NoMatches::Pending {
            self.no_matches = NoMatches::Shown;
        }
        true
    }
}

#[cfg(test)]
mod tests;
