//! The Settings modal, "LazyGithub Settings": a tab per config section (#304's [`SECTIONS`]), each listing its
//! fields with a widget by kind, and Save / Cancel. `/` opens a search box above the tabs whose `enter` filters every
//! tab's fields by a fuzzy match on their names. Save collects every field into a copy of the config; writing and
//! applying it is the opener's. Python's `SettingsModal` and `SettingsContainer`.

mod key_row;
mod list_field;
mod pane;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::config::settings::SECTIONS;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use self::pane::{FieldWidget, PaneHit, SettingsPane};
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::select::SelectOutcome;
use crate::components::selection_list::ListOutcome;
use crate::components::table::py_strip;
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::toast::Toast;

/// The frame's widest: `width: 100`.
const MAX_WIDTH: u16 = 100;

/// The frame's share of the rows above the footer: the modal's `height: 80%`, rounded down.
const HEIGHT_PERCENT: u16 = 80;

const HEADING: &str = "LazyGithub Settings";

/// The heading's block: `MarkdownH1`'s two rows of margin above and one below.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
const HEADING_PADDING: u16 = 2;

/// Below the scroll area: the rule, a blank row and the buttons, docked at the bottom.
const FOOTER_ROWS: u16 = 1 + 1 + ButtonRow::FLUSH_HEIGHT;

/// The search box's first row: below the heading, past the box's `margin-top: 1`.
const SEARCH_TOP: u16 = HEADING_ROWS + 1;

/// The rows the shown search box pushes the tab strip down: its margins and itself.
const SEARCH_ROWS: u16 = 1 + text_input::HEIGHT + 1;

const SEARCH_PLACEHOLDER: &str = "Search settings...";

/// The button row's `padding-left: 35`.
const BUTTONS_LEFT: u16 = 35;

/// The error toast's title when a field won't save (Py bug B90, answered).
const INVALID_TITLE: &str = "Invalid setting";

/// The dialog's screen bindings: submit, close and search, in the footer's order.
const SETTINGS_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose, ModalSearch];

/// How the Settings modal closed.
#[derive(Debug)]
pub enum SettingsOutcome {
    /// Save: the config with every field written back.
    Saved(Box<Config>),
    /// Cancel, `q` or `escape`: nothing changes.
    Cancelled,
}

/// The Settings modal's own action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    /// `modal.submit` or Save: collect the fields and close with them, unless one is invalid.
    Save,
    /// `modal.close` or Cancel: close without saving.
    Cancel,
}

/// The modal's focus stops. A field's index is into the active tab's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsFocus {
    /// The search box, first in the ring while it's shown.
    Search,
    /// The tab strip, focused on opening.
    Tabs,
    /// The active tab's scroll area, which Textual's `ScrollableContainer` makes focusable.
    Scroll,
    /// A field's widget; for a list field, its input.
    Field(usize),
    /// A list field's checklist.
    ListItems(usize),
    /// A Key Bindings row.
    KeyRow(usize),
    Save,
    Cancel,
}

impl SettingsFocus {
    fn button(self) -> Option<Pressed> {
        match self {
            SettingsFocus::Save => Some(Pressed::Submit),
            SettingsFocus::Cancel => Some(Pressed::Cancel),
            _ => None,
        }
    }
}

/// A field that won't save: where it is, and the toast's message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Invalid {
    tab: usize,
    field: usize,
    message: String,
}

/// Where the interior's parts were drawn last, for the mouse.
#[derive(Debug, Clone, Copy, Default)]
struct Areas {
    search: Rect,
    tabs: Rect,
    scroll: Rect,
}

/// The Settings modal. Its output is the [`SettingsOutcome`].
#[derive(Debug)]
pub struct SettingsDialog {
    tabs: TabStrip,
    panes: Vec<SettingsPane>,
    /// The search box, while it's shown (Python's `ToggleableSearchInput`, hidden until `/`).
    search: Option<TextInput>,
    ring: FocusRing<SettingsFocus>,
    buttons: ButtonRow,
    areas: Areas,
    output: Option<SettingsOutcome>,
}

impl SettingsDialog {
    /// The modal showing `config`'s values, on the first tab with the strip focused.
    pub fn new(config: &Config) -> Self {
        let panes: Vec<SettingsPane> = SECTIONS
            .iter()
            .map(|section| SettingsPane::new(section, config))
            .collect();
        let mut dialog = Self {
            tabs: TabStrip::new(panes.iter().map(SettingsPane::title)),
            panes,
            search: None,
            ring: FocusRing::new([SettingsFocus::Tabs]),
            buttons: ButtonRow::flush("Save"),
            areas: Areas::default(),
            output: None,
        };
        dialog.rebuild_ring();
        dialog
    }

    fn action(action: SettingsAction) -> Action {
        Action::Dialog(DialogAction::Settings(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => SettingsAction::Save,
            Pressed::Cancel => SettingsAction::Cancel,
        })
    }

    fn pane(&self) -> &SettingsPane {
        &self.panes[self.tabs.active()]
    }

    fn pane_mut(&mut self) -> &mut SettingsPane {
        &mut self.panes[self.tabs.active()]
    }

    /// The focus ring for the active tab: the search box while it's shown, the strip, the scroll area, the tab's
    /// shown fields (a disabled one skipped), Save and Cancel.
    fn rebuild_ring(&mut self) {
        let pane = self.pane();
        let mut order = Vec::new();
        if self.search.is_some() {
            order.push(SettingsFocus::Search);
        }
        order.extend([SettingsFocus::Tabs, SettingsFocus::Scroll]);
        order.extend(pane.stops());
        order.extend([SettingsFocus::Save, SettingsFocus::Cancel]);
        let disabled: Vec<(SettingsFocus, bool)> = order.iter().map(|&stop| (stop, pane.is_disabled(stop))).collect();
        self.ring.set_order(order);
        for (stop, disabled) in disabled {
            self.ring.set_disabled(stop, disabled);
        }
    }

    /// Makes tab `index` the active one, and the ring follow it.
    fn switch_tab(&mut self, index: usize) {
        if self.tabs.select(index) {
            self.rebuild_ring();
        }
    }

    /// Moves focus with `f`. An open list closes when focus leaves it, and so does a blank search box. A stop that
    /// gains focus is scrolled into view, and a text input selects its text (Textual's `select_on_focus`, so typing
    /// replaces it).
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<SettingsFocus>)) {
        let before = self.ring.focused();
        f(&mut self.ring);
        let Some(stop) = self.ring.focused().filter(|&stop| Some(stop) != before) else {
            return;
        };
        if before == Some(SettingsFocus::Search) {
            self.blur_search();
        }
        if stop == SettingsFocus::Search {
            if let Some(search) = &mut self.search {
                search.select_all();
            }
            return;
        }
        let pane = self.pane_mut();
        pane.close_selects();
        if let SettingsFocus::Field(index) = stop
            && let Some(input) = pane.fields.get_mut(index).and_then(|field| field.input_mut(stop))
        {
            input.select_all();
        }
        pane.reveal(stop);
    }

    fn focus(&mut self, stop: SettingsFocus) {
        self.refocus(|ring| {
            ring.focus(stop);
        });
    }

    /// `/`: shows the search box if it's hidden, and focuses it. Python's `action_search`.
    fn open_search(&mut self) {
        if self.search.is_none() {
            self.search = Some(TextInput::new(SEARCH_PLACEHOLDER));
            self.rebuild_ring();
        }
        self.pane_mut().close_selects();
        self.focus(SettingsFocus::Search);
    }

    /// The search box lost focus: it hides when only whitespace is left in it (`ToggleableSearchInput._on_blur`),
    /// leaving the filter as it was.
    fn blur_search(&mut self) {
        if self
            .search
            .as_ref()
            .is_some_and(|search| py_strip(search.value()).is_empty())
        {
            self.search = None;
            self.rebuild_ring();
        }
    }

    /// `enter` in the search box: filters every tab by the stripped, lowercased query; a blank one shows everything.
    /// Focus stays in the box. Python's `handle_submitted_search`.
    fn submit_search(&mut self) {
        let Some(search) = &self.search else { return };
        let query = py_strip(search.value()).to_lowercase();
        let filter = (!query.is_empty()).then_some(query);
        for pane in &mut self.panes {
            pane.filter(filter.as_deref());
        }
        self.rebuild_ring();
    }

    /// The focused field's widget, with its focus stop.
    fn focused_widget(&self) -> Option<(SettingsFocus, &FieldWidget)> {
        let stop = self.ring.focused()?;
        let (SettingsFocus::Field(index) | SettingsFocus::ListItems(index)) = stop else {
            return None;
        };
        Some((stop, &self.pane().fields.get(index)?.widget))
    }

    fn focused_widget_mut(&mut self) -> Option<(SettingsFocus, &mut FieldWidget)> {
        let stop = self.ring.focused()?;
        let (SettingsFocus::Field(index) | SettingsFocus::ListItems(index)) = stop else {
            return None;
        };
        Some((stop, &mut self.pane_mut().fields.get_mut(index)?.widget))
    }

    /// Every field written into a copy of `base` through its descriptor, tab by tab. A select on its blank option
    /// leaves its field alone (Py bug B89, answered); the first field that won't take its value stops it (Py bug
    /// B90, answered).
    fn collect(&self, base: &Config) -> Result<Config, Invalid> {
        let mut config = base.clone();
        for row in self.panes.iter().flat_map(|pane| &pane.keys) {
            row.save(&mut config.bindings.overrides);
        }
        for (tab, pane) in self.panes.iter().enumerate() {
            for (index, field) in pane.fields.iter().enumerate() {
                let Some(value) = field.value() else { continue };
                if let Err(error) = field.field.set(&mut config, value) {
                    return Err(Invalid {
                        tab,
                        field: index,
                        message: format!("{} {error}", field.name()),
                    });
                }
            }
        }
        Ok(config)
    }

    /// Save: closes with the edited config, or stays open on the first invalid field with an error toast.
    fn save(&mut self, cx: &mut Cx<'_>) {
        match self.collect(cx.config()) {
            Ok(config) => self.output = Some(SettingsOutcome::Saved(Box::new(config))),
            Err(invalid) => {
                self.switch_tab(invalid.tab);
                self.focus(SettingsFocus::Field(invalid.field));
                cx.notify(Toast::error(invalid.message).with_title(INVALID_TITLE));
            }
        }
    }

    /// A key for the focused text input: the search box's `enter` filters, a list field's adds its item, and a plain
    /// field's does nothing.
    fn input_key(&mut self, chord: &KeyChord, cx: &Cx<'_>) -> Option<Action> {
        if self.ring.focused() == Some(SettingsFocus::Search) {
            let outcome = self.search.as_mut()?.handle_key(chord, cx.keymap(), cx.clipboard());
            if outcome == InputOutcome::Submitted {
                self.submit_search();
            }
            return Self::input_outcome(outcome);
        }
        let (stop, widget) = self.focused_widget_mut()?;
        let outcome = match (widget, stop) {
            (FieldWidget::Text(input), _) => input.handle_key(chord, cx.keymap(), cx.clipboard()),
            (FieldWidget::List(list), SettingsFocus::Field(_)) => {
                let outcome = list.input.handle_key(chord, cx.keymap(), cx.clipboard());
                if outcome == InputOutcome::Submitted {
                    list.submit();
                }
                outcome
            }
            _ => return None,
        };
        Self::input_outcome(outcome)
    }

    fn input_outcome(outcome: InputOutcome) -> Option<Action> {
        match outcome {
            InputOutcome::Changed | InputOutcome::Submitted => Some(Action::Consumed),
            InputOutcome::Copied(text) => Some(Action::Copy(text)),
            InputOutcome::Ignored => None,
        }
    }
}

impl Component for SettingsDialog {
    type Action = SettingsAction;

    fn bindings(&self) -> &'static [BindingId] {
        SETTINGS_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(SettingsAction::Save)),
            BindingId::ModalClose => Some(Self::action(SettingsAction::Cancel)),
            // A focused text input, the box included, consumes `/` before this fires.
            BindingId::ModalSearch => {
                self.open_search();
                Some(Action::Consumed)
            }
            _ => None,
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        // A key row takes every key but the focus moves (priority bindings have already run).
        if let Some(SettingsFocus::KeyRow(_)) = self.ring.focused() {
            return key.code != KeyCode::Tab || !(key.mods - KeyModifiers::SHIFT).is_empty();
        }
        if self.ring.focused() == Some(SettingsFocus::Search) {
            return self
                .search
                .as_ref()
                .is_some_and(|search| search.consumes_key(key, keymap));
        }
        match self.focused_widget() {
            Some((_, FieldWidget::Select(select))) => select.consumes_key(key, keymap),
            Some((_, FieldWidget::Text(input))) => input.consumes_key(key, keymap),
            Some((SettingsFocus::Field(_), FieldWidget::List(list))) => list.input.consumes_key(key, keymap),
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        if let Some(SettingsFocus::KeyRow(index)) = self.ring.focused() {
            let row = self.pane_mut().keys.get_mut(index)?;
            row.capture(&chord);
            return Some(Action::Consumed);
        }
        if let Some((_, FieldWidget::Select(select))) = self.focused_widget_mut() {
            return match select.handle_key(&chord, cx.keymap(), cx.now()) {
                SelectOutcome::Ignored => None,
                _ => Some(Action::Consumed),
            };
        }
        self.input_key(&chord, cx)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        // A drag that started in a text input, wherever the pointer is now.
        if self.search.as_mut().is_some_and(|search| search.captured_mouse(event))
            || self.pane_mut().captured_mouse(event)
        {
            return Some(Action::Consumed);
        }
        let press = event.kind == MouseEventKind::Down(MouseButton::Left);
        let at = Position::new(event.column, event.row);
        let hit = self.pane().hit(event.column, event.row);
        if let Some(select) = self.pane_mut().open_select_mut() {
            if select.overlay_contains(event.column, event.row) {
                let _ = select.handle_overlay_mouse(&event);
                return Some(Action::Consumed);
            }
            // A press elsewhere takes the focus from the list, which closes it; a press on its box toggles it below.
            let on_box = matches!(hit, Some(PaneHit::Stop(_)));
            if press && !on_box {
                select.close();
            }
        }
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    self.focus(match button {
                        Pressed::Submit => SettingsFocus::Save,
                        Pressed::Cancel => SettingsFocus::Cancel,
                    });
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            if self.areas.scroll.contains(at) {
                self.pane_mut().scroll.wheel(wheel);
            }
            return Some(Action::Consumed);
        }
        if !press {
            return Some(Action::Consumed);
        }
        if self.search.is_some() && self.areas.search.contains(at) {
            self.pane_mut().close_selects();
            self.focus(SettingsFocus::Search);
            if let Some(search) = &mut self.search {
                search.press(self.areas.search, event.column);
            }
            return Some(Action::Consumed);
        }
        if self.areas.tabs.contains(at) {
            self.focus(SettingsFocus::Tabs);
            if let Some(index) = self.tabs.hit(event.column - self.areas.tabs.x) {
                self.switch_tab(index);
            }
            return Some(Action::Consumed);
        }
        match hit {
            Some(PaneHit::Stop(stop)) if self.ring.is_enabled(stop) => {
                // Focus first (an input selects its text), then the press lands.
                self.focus(stop);
                self.pane_mut().press(stop, event);
            }
            Some(PaneHit::Stop(_) | PaneHit::Area) => self.focus(SettingsFocus::Scroll),
            None => {}
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: SettingsAction, cx: &mut Cx<'_>) {
        match action {
            SettingsAction::Save => self.save(cx),
            SettingsAction::Cancel => self.output = Some(SettingsOutcome::Cancelled),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let under = self.background(p).over(theme.background());
        let focus = self.ring.focused();
        let screen = frame.area();

        // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style.
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: area.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        let style = p
            .markdown_h1_text_style
            .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
        let width = usize::from(heading.width);
        frame
            .buffer_mut()
            .set_stringn(heading.x, heading.y, format!("{HEADING:^width$}"), width, style);

        // The search box, while shown, between the heading and the strip, which moves down for it.
        let mut strip_top = HEADING_ROWS;
        let mut search_area = Rect::default();
        if let Some(search) = &mut self.search {
            search_area = Rect {
                y: area.y.saturating_add(SEARCH_TOP),
                height: text_input::HEIGHT,
                ..area
            }
            .intersection(area);
            search.render(
                frame.buffer_mut(),
                search_area,
                theme,
                under,
                focus == Some(SettingsFocus::Search),
            );
            strip_top += SEARCH_ROWS;
        }

        let tabs = Rect {
            y: area.y.saturating_add(strip_top),
            height: TAB_STRIP_HEIGHT,
            ..area
        }
        .intersection(area);
        self.tabs
            .render_over(frame, tabs, theme, focus == Some(SettingsFocus::Tabs), under);

        // The scroll area keeps the height it has without the box: Python's column overflows instead, so the box
        // pushes the area down under the docked buttons and the rule off the frame.
        let unshifted = area.y.saturating_add(HEADING_ROWS + TAB_STRIP_HEIGHT);
        let scroll = Rect {
            y: area.y.saturating_add(strip_top + TAB_STRIP_HEIGHT),
            height: area.bottom().saturating_sub(unshifted).saturating_sub(FOOTER_ROWS),
            ..area
        }
        .intersection(area);
        let buf = frame.buffer_mut();
        self.pane_mut().render(buf, scroll, theme, under, focus);

        // `Rule`: a `─` line in `$secondary` across the interior, right below the scroll area.
        let rule_style = Style::new().fg(theme.color(p.secondary, under));
        if scroll.bottom() < area.bottom() {
            let width = usize::from(area.width);
            buf.set_stringn(area.x, scroll.bottom(), "─".repeat(width), width, rule_style);
        }

        // The buttons, docked at the bottom, `padding-left: 35` in.
        let buttons_width =
            self.buttons.button(Pressed::Submit).width(theme) + self.buttons.button(Pressed::Cancel).width(theme);
        let buttons = Rect {
            x: area.x.saturating_add(BUTTONS_LEFT),
            y: area.bottom().saturating_sub(ButtonRow::FLUSH_HEIGHT),
            width: buttons_width,
            height: ButtonRow::FLUSH_HEIGHT,
        }
        .intersection(area);
        self.buttons
            .render(buf, buttons, theme, focus.and_then(SettingsFocus::button));

        // An open select's list last, over everything below it.
        self.pane_mut().render_overlay(buf, screen, theme, under);
        self.areas = Areas {
            search: search_area,
            tabs,
            scroll,
        };
    }
}

impl Dialog for SettingsDialog {
    type Output = SettingsOutcome;

    fn kind(&self) -> ModalKind {
        ModalKind::Settings
    }

    fn own_action(&self, action: DialogAction) -> Option<SettingsAction> {
        match action {
            DialogAction::Settings(action) => Some(action),
            _ => None,
        }
    }

    /// `tab` or `shift+tab`: an open list closes (it loses focus), then focus moves on.
    fn move_focus(&mut self, direction: FocusMove) {
        self.pane_mut().close_selects();
        self.refocus(|ring| ring.step(direction));
    }

    /// 100 columns (the whole width when narrower) and 80% of the rows above the footer, rounded down.
    fn size(&self, available: Rect) -> (u16, u16) {
        (available.width.min(MAX_WIDTH), available.height * HEIGHT_PERCENT / 100)
    }

    fn take_output(&mut self) -> Option<SettingsOutcome> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match (self.ring.focused(), self.focused_widget()) {
            (Some(SettingsFocus::Search), _) => FieldKind::TextInput,
            (Some(SettingsFocus::Tabs), _) => FieldKind::Tabs,
            (Some(SettingsFocus::Scroll), _) => FieldKind::Scroll,
            (Some(SettingsFocus::Save | SettingsFocus::Cancel), _) => FieldKind::Button,
            (Some(SettingsFocus::KeyRow(_)), _) => FieldKind::KeyCapture,
            (_, Some((_, FieldWidget::Switch(_)))) => FieldKind::Switch,
            (_, Some((_, FieldWidget::Select(select)))) if select.is_open() => FieldKind::SelectOverlay,
            (_, Some((_, FieldWidget::Select(_)))) => FieldKind::Select,
            (_, Some((SettingsFocus::ListItems(_), FieldWidget::List(_)))) => FieldKind::SelectionList,
            (_, Some((_, FieldWidget::Text(_) | FieldWidget::List(_)))) => FieldKind::TextInput,
            (_, None) => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match field {
            // `left` / `right` on the strip switch tabs, wrapping; focus stays on the strip.
            FieldKind::Tabs => {
                let mut strip = self.tabs.clone();
                match id {
                    BindingId::TextualTabsNext => strip.select_next(),
                    BindingId::TextualTabsPrevious => strip.select_previous(),
                    _ => return None,
                };
                self.switch_tab(strip.active());
                Some(Action::Consumed)
            }
            FieldKind::Scroll => {
                let scroll = Scroll::from_binding(id)?;
                self.pane_mut().scroll.scroll(scroll);
                Some(Action::Consumed)
            }
            FieldKind::Button => {
                let button = self.ring.focused().and_then(SettingsFocus::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            FieldKind::Switch => match self.focused_widget_mut() {
                Some((_, FieldWidget::Switch(switch))) if id == BindingId::TextualSwitchToggle => {
                    switch.toggle();
                    Some(Action::Consumed)
                }
                _ => None,
            },
            FieldKind::Select | FieldKind::SelectOverlay => match self.focused_widget_mut() {
                Some((_, FieldWidget::Select(select))) => {
                    (select.on_binding(id) != SelectOutcome::Ignored).then_some(Action::Consumed)
                }
                _ => None,
            },
            FieldKind::SelectionList => match self.focused_widget_mut() {
                Some((_, FieldWidget::List(list))) => {
                    (list.list.on_binding(id) != ListOutcome::Ignored).then_some(Action::Consumed)
                }
                _ => None,
            },
            // `consumes_key` already gave the input every key it binds.
            FieldKind::TextInput => Some(Action::Consumed),
            FieldKind::TextEditor | FieldKind::Table | FieldKind::KeyCapture => None,
        }
    }

    fn captures_mouse(&self) -> bool {
        self.search.as_ref().is_some_and(TextInput::is_selecting) || self.pane().is_selecting()
    }

    fn handle_paste(&mut self, text: &str, _cx: &Cx<'_>) -> Option<Action> {
        let stop = self.ring.focused()?;
        if stop == SettingsFocus::Search {
            self.search.as_mut()?.handle_paste(text);
            return Some(Action::Consumed);
        }
        let SettingsFocus::Field(index) = stop else {
            return None;
        };
        let input = self.pane_mut().fields.get_mut(index)?.input_mut(stop)?;
        input.handle_paste(text);
        Some(Action::Consumed)
    }
}

#[cfg(test)]
mod tests;
