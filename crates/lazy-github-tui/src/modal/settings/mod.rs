//! The Settings modal, "LazyGithub Settings": a tab per config section (#304's [`SECTIONS`]), each listing its
//! fields with a widget by kind, and Save / Cancel. `/` opens a search box above the tabs whose `enter` filters every
//! tab's fields by a fuzzy match on their names. Save collects every field into a copy of the config; writing and
//! applying it is the opener's. Python's `SettingsModal` and `SettingsContainer`.

mod key_list;
mod key_row;
mod pane;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::config::settings::SECTIONS;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use self::key_list::KeyListOutcome;
pub(crate) use self::key_list::{KEY_CAPTURE_BINDINGS, KEY_LIST_BINDINGS};
use self::pane::{FieldWidget, PaneHit, SettingsPane};
use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{BindingState, ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::list_editor::{ListEditor, ListEditorOutcome};
use crate::components::select::SelectOutcome;
use crate::components::table::py_strip;
use crate::components::tabs::{TAB_STRIP_HEIGHT, TabStrip};
use crate::components::text_input::{self, InputOutcome, TextInput};
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::Catalog;
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

/// The `Rule`'s `margin: 1 0`: a blank row above it and one below.
const RULE_MARGIN: u16 = 1;

/// Below the scroll area: the rule and its margins, then the buttons, docked at the bottom.
const FOOTER_ROWS: u16 = RULE_MARGIN + 1 + RULE_MARGIN + ButtonRow::FLUSH_HEIGHT;

/// The search box's first row: below the heading, past the box's `margin-top: 1`.
const SEARCH_TOP: u16 = HEADING_ROWS + 1;

/// The rows the shown search box pushes the tab strip down: its margins and itself.
const SEARCH_ROWS: u16 = 1 + text_input::HEIGHT + 1;

const SEARCH_PLACEHOLDER: &str = "Search settings...";

/// The button row's `padding-left: 35`.
const BUTTONS_LEFT: u16 = 35;

/// The error toast's title when a field won't save (Py bug B90, answered).
const INVALID_TITLE: &str = "Invalid setting";

/// The title of [`key_conflicts_toast`].
const KEY_CONFLICTS_TITLE: &str = "Key conflicts";

/// The dialog's screen bindings: submit, close and search, in the footer's order.
pub(crate) const SETTINGS_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose, ModalSearch];

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
    /// A field's widget, a list field's editor included (one stop since #343).
    Field(usize),
    /// The Key Bindings list, one stop (#381).
    KeyList,
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

/// The error toast for text a list field's editor won't take because the field lists it already.
pub(super) fn refused_toast(name: &str, text: &str) -> Toast {
    Toast::error(format!("{name} already lists {text}")).with_title(INVALID_TITLE)
}

/// The warning that follows "Settings updated" when the saved keymap still has bindings in a conflict (#382): Save
/// isn't blocked, but says so. `None` without any.
pub fn key_conflicts_toast(keymap: &Keymap) -> Option<Toast> {
    let mut ids: Vec<BindingId> = crate::keymap::conflicts(keymap)
        .into_iter()
        .map(|conflict| conflict.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let message = match ids.len() {
        0 => return None,
        1 => "1 binding shares a key with another binding.".to_owned(),
        n => format!("{n} bindings share a key with another binding."),
    };
    let message = format!("{message} Open Settings → Key Bindings to see them.");
    Some(Toast::warning(message).with_title(KEY_CONFLICTS_TITLE))
}

/// The toast to show, if any, as the action that showed it; otherwise the key or press is simply consumed.
fn notify_or_consumed(toast: Option<Toast>) -> Action {
    toast.map_or(Action::Consumed, Action::Notify)
}

/// The focus move `chord` is bound to on the modal screen, if any.
fn focus_move(chord: &KeyChord, keymap: &Keymap) -> Option<FocusMove> {
    match keymap.match_in(&[BindingId::TextualFocusNext, BindingId::TextualFocusPrevious], chord)? {
        BindingId::TextualFocusNext => Some(FocusMove::Next),
        _ => Some(FocusMove::Previous),
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
    /// The modal showing `config`'s values, on the first tab with the strip focused. The theme select offers
    /// `themes`.
    pub fn new(config: &Config, themes: &'static Catalog) -> Self {
        let panes: Vec<SettingsPane> = SECTIONS
            .iter()
            .map(|section| SettingsPane::new(section, config, themes))
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

    /// Moves focus with `f`. An open list closes when focus leaves it, and so does a blank search box; a list
    /// editor's open input commits. A stop that gains focus is scrolled into view, and a text input selects its text
    /// (Textual's `select_on_focus`, so typing replaces it). The toast for text the list editor refused, if any.
    #[must_use]
    fn refocus(&mut self, f: impl FnOnce(&mut FocusRing<SettingsFocus>)) -> Option<Toast> {
        let before = self.ring.focused();
        f(&mut self.ring);
        let stop = self.ring.focused().filter(|&stop| Some(stop) != before)?;
        let refused = match before {
            Some(SettingsFocus::Field(index)) => self.pane_mut().commit_list(index),
            _ => None,
        };
        if before == Some(SettingsFocus::Search) {
            self.blur_search();
        }
        if stop == SettingsFocus::Search {
            if let Some(search) = &mut self.search {
                search.select_all();
            }
            return refused;
        }
        let pane = self.pane_mut();
        pane.close_selects();
        if let SettingsFocus::Field(index) = stop
            && let Some(input) = pane.fields.get_mut(index).and_then(|field| field.input_mut(stop))
        {
            input.select_all();
        }
        pane.reveal(stop);
        refused
    }

    #[must_use]
    fn focus(&mut self, stop: SettingsFocus) -> Option<Toast> {
        self.refocus(|ring| {
            ring.focus(stop);
        })
    }

    /// `/`: shows the search box if it's hidden, and focuses it. Python's `action_search`.
    #[must_use]
    fn open_search(&mut self) -> Option<Toast> {
        if self.search.is_none() {
            self.search = Some(TextInput::new(SEARCH_PLACEHOLDER));
            self.rebuild_ring();
        }
        self.pane_mut().close_selects();
        self.focus(SettingsFocus::Search)
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
        let SettingsFocus::Field(index) = stop else {
            return None;
        };
        Some((stop, &self.pane().fields.get(index)?.widget))
    }

    fn focused_widget_mut(&mut self) -> Option<(SettingsFocus, &mut FieldWidget)> {
        let stop = self.ring.focused()?;
        let SettingsFocus::Field(index) = stop else {
            return None;
        };
        Some((stop, &mut self.pane_mut().fields.get_mut(index)?.widget))
    }

    /// The focused field's list editor, if it's a list field.
    fn focused_list(&self) -> Option<&ListEditor> {
        match self.focused_widget()? {
            (_, FieldWidget::List(list)) => Some(list),
            _ => None,
        }
    }

    fn focused_list_mut(&mut self) -> Option<&mut ListEditor> {
        match self.focused_widget_mut()? {
            (_, FieldWidget::List(list)) => Some(list),
            _ => None,
        }
    }

    /// Whether the focused field is a list editor with its input open.
    fn is_editing_list(&self) -> bool {
        self.focused_list().is_some_and(ListEditor::is_editing)
    }

    /// What the focused list editor's `outcome` means for the dialog: refused text is an error toast naming the
    /// field, a copy goes to the clipboard, and anything else but `Ignored` is consumed. The tab follows the
    /// highlight.
    fn list_outcome(&mut self, outcome: ListEditorOutcome) -> Option<Action> {
        let Some(stop @ SettingsFocus::Field(index)) = self.ring.focused() else {
            return None;
        };
        let action = match outcome {
            ListEditorOutcome::Ignored => return None,
            ListEditorOutcome::Refused { text } => {
                Action::Notify(refused_toast(self.pane().fields.get(index)?.name(), &text))
            }
            ListEditorOutcome::Copied(text) => Action::Copy(text),
            ListEditorOutcome::Moved
            | ListEditorOutcome::Changed
            | ListEditorOutcome::Editing
            | ListEditorOutcome::Cancelled => Action::Consumed,
        };
        self.pane_mut().reveal_later(stop);
        Some(action)
    }

    /// Every field written into a copy of `base` through its descriptor, tab by tab. A select on its blank option
    /// leaves its field alone (Py bug B89, answered); the first field that won't take its value stops it (Py bug
    /// B90, answered).
    fn collect(&self, base: &Config) -> Result<Config, Invalid> {
        let mut config = base.clone();
        for keys in self.panes.iter().filter_map(|pane| pane.keys.as_ref()) {
            keys.save(&mut config.bindings.overrides);
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

    /// Save: an open list input commits first (text it refuses is dropped, with its toast), then it closes with the
    /// edited config, or stays open on the first invalid field with an error toast.
    fn save(&mut self, cx: &mut Cx<'_>) {
        for toast in self.panes.iter_mut().flat_map(SettingsPane::commit_lists) {
            cx.notify(toast);
        }
        match self.collect(cx.config()) {
            Ok(config) => self.output = Some(SettingsOutcome::Saved(Box::new(config))),
            Err(invalid) => {
                self.switch_tab(invalid.tab);
                let _ = self.focus(SettingsFocus::Field(invalid.field));
                cx.notify(Toast::error(invalid.message).with_title(INVALID_TITLE));
            }
        }
    }

    /// Whether the active tab's Key Bindings list is waiting for a key.
    fn is_capturing(&self) -> bool {
        self.pane().is_capturing()
    }

    /// A key for the focused text input: the search box's `enter` filters, a plain field's does nothing, and a list
    /// editor's open input takes what it consumes. `tab` and `shift+tab` from that input move focus here, so the
    /// toast for text the commit refuses can go out.
    fn input_key(&mut self, chord: &KeyChord, cx: &Cx<'_>) -> Option<Action> {
        if self.ring.focused() == Some(SettingsFocus::Search) {
            let outcome = self.search.as_mut()?.handle_key(chord, cx.keymap(), cx.clipboard());
            if outcome == InputOutcome::Submitted {
                self.submit_search();
            }
            return Self::input_outcome(outcome);
        }
        if self.is_editing_list() {
            if let Some(direction) = focus_move(chord, cx.keymap()) {
                return Some(notify_or_consumed(self.step_focus(direction)));
            }
            let outcome = self.focused_list_mut()?.handle_key(chord, cx.keymap(), cx.clipboard());
            return self.list_outcome(outcome);
        }
        let (_, widget) = self.focused_widget_mut()?;
        let FieldWidget::Text(input) = widget else {
            return None;
        };
        Self::input_outcome(input.handle_key(chord, cx.keymap(), cx.clipboard()))
    }

    /// `tab` or `shift+tab`: an open list closes (it loses focus), then focus moves on. The toast for text a list
    /// editor's open input refused as it committed, if any.
    #[must_use]
    fn step_focus(&mut self, direction: FocusMove) -> Option<Toast> {
        self.pane_mut().close_selects();
        self.refocus(|ring| ring.step(direction))
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
            BindingId::ModalSearch => Some(notify_or_consumed(self.open_search())),
            _ => None,
        }
    }

    /// While the Key Bindings list waits for a key, the footer offers only its cancel key: submit, close and search
    /// are keys it records.
    fn binding_state(&self, id: BindingId, _cx: &Cx<'_>) -> BindingState {
        let screen = matches!(
            id,
            BindingId::ModalSubmit | BindingId::ModalClose | BindingId::ModalSearch
        );
        if screen && self.is_capturing() {
            BindingState::Hidden
        } else {
            BindingState::Enabled
        }
    }

    fn consumes_key(&self, key: &KeyChord, keymap: &Keymap) -> bool {
        // The Key Bindings list, waiting for a key, takes every key (priority bindings have already run).
        if self.ring.focused() == Some(SettingsFocus::KeyList) {
            return self.is_capturing();
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
            // An open list input also takes `tab` and `shift+tab`, to commit before focus moves.
            Some((_, FieldWidget::List(list))) => {
                list.consumes_key(key, keymap) || (list.is_editing() && focus_move(key, keymap).is_some())
            }
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cx: &Cx<'_>) -> Option<Action> {
        let chord = KeyChord::from_event(&key)?;
        if self.ring.focused() == Some(SettingsFocus::KeyList) {
            let keys = self.pane_mut().keys.as_mut()?;
            return (keys.capture(&chord, cx.keymap()) == KeyListOutcome::Consumed).then_some(Action::Consumed);
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
        // While the Key Bindings list waits for a key, the mouse does nothing but end the wait.
        if self.is_capturing() {
            if press && let Some(keys) = &mut self.pane_mut().keys {
                keys.cancel_capture();
            }
            return Some(Action::Consumed);
        }
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
                ButtonMouse::Focus => notify_or_consumed(self.focus(match button {
                    Pressed::Submit => SettingsFocus::Save,
                    Pressed::Cancel => SettingsFocus::Cancel,
                })),
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            if self.areas.scroll.contains(at) {
                self.pane_mut().wheel(wheel, event.column, event.row);
            }
            return Some(Action::Consumed);
        }
        if !press {
            return Some(Action::Consumed);
        }
        if self.search.is_some() && self.areas.search.contains(at) {
            self.pane_mut().close_selects();
            let refused = self.focus(SettingsFocus::Search);
            if let Some(search) = &mut self.search {
                search.press(self.areas.search, event.column);
            }
            return Some(notify_or_consumed(refused));
        }
        if self.areas.tabs.contains(at) {
            let refused = self.focus(SettingsFocus::Tabs);
            let scroll = self.tabs.scroll(self.areas.tabs.width);
            if let Some(index) = self.tabs.hit(event.column - self.areas.tabs.x + scroll) {
                self.switch_tab(index);
            }
            return Some(notify_or_consumed(refused));
        }
        let refused = match hit {
            Some(PaneHit::Stop(SettingsFocus::KeyList)) if self.ring.is_enabled(SettingsFocus::KeyList) => {
                let focused = self.ring.focused() == Some(SettingsFocus::KeyList);
                let refused = self.focus(SettingsFocus::KeyList);
                self.pane_mut().press_keys(event, focused);
                refused
            }
            Some(PaneHit::Stop(stop)) if self.ring.is_enabled(stop) => {
                // Focus first (an input selects its text), then the press lands.
                let focused = self.ring.focused() == Some(stop);
                let refused = self.focus(stop);
                let pressed = self.pane_mut().press(stop, event, focused);
                refused.or(pressed)
            }
            Some(PaneHit::Stop(_) | PaneHit::Area) => self.focus(SettingsFocus::Scroll),
            None => None,
        };
        Some(notify_or_consumed(refused))
    }

    fn update(&mut self, action: SettingsAction, cx: &mut Cx<'_>) {
        match action {
            SettingsAction::Save => self.save(cx),
            SettingsAction::Cancel => self.output = Some(SettingsOutcome::Cancelled),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let r = theme.roles();
        let under = self.background(r).over(theme.background());
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
        let h1 = r.markdown.h1;
        let style = h1.style.apply(Style::new().fg(theme.color(h1.fg, under)));
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

        // `Rule`: a `─` line in `$secondary` across the interior, past its top margin below the scroll area.
        let rule_style = Style::new().fg(theme.color(r.border.rule, under));
        let rule_y = scroll.bottom().saturating_add(RULE_MARGIN);
        if rule_y < area.bottom() {
            let width = usize::from(area.width);
            buf.set_stringn(area.x, rule_y, "─".repeat(width), width, rule_style);
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
        // The docked row (`width: auto`) covers its `padding-left` too: with the search box shown, the scroll area
        // reaches under it, and the row hides what's there.
        let docked = Rect {
            x: area.x,
            width: buttons.right().saturating_sub(area.x),
            ..buttons
        };
        let fill = Style::new().fg(theme.color(r.text.plain, under)).bg(theme.paint(under));
        for position in docked.positions() {
            buf[position].reset();
            buf[position].set_style(fill);
        }
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

    /// `tab` or `shift+tab`: an open list closes (it loses focus), then focus moves on. A list editor's open input
    /// takes these keys first ([`Self::input_key`]), so there's never a refused commit to tell of here.
    fn move_focus(&mut self, direction: FocusMove) {
        let _ = self.step_focus(direction);
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
            (Some(SettingsFocus::KeyList), _) if self.is_capturing() => FieldKind::KeyCapture,
            (Some(SettingsFocus::KeyList), _) => FieldKind::KeyList,
            (_, Some((_, FieldWidget::Switch(_)))) => FieldKind::Switch,
            (_, Some((_, FieldWidget::Select(select)))) if select.is_open() => FieldKind::SelectOverlay,
            (_, Some((_, FieldWidget::Select(_)))) => FieldKind::Select,
            (_, Some((_, FieldWidget::List(list)))) if list.is_editing() => FieldKind::ListEditorInput,
            (_, Some((_, FieldWidget::List(_)))) => FieldKind::ListEditor,
            (_, Some((_, FieldWidget::Text(_)))) => FieldKind::TextInput,
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
            FieldKind::ListEditor | FieldKind::ListEditorInput => {
                let outcome = self.focused_list_mut()?.on_binding(id);
                match (field, outcome) {
                    // The input's own keys that `consumes_key` left over (copy with nothing selected) go nowhere.
                    (FieldKind::ListEditorInput, ListEditorOutcome::Ignored) => Some(Action::Consumed),
                    (_, outcome) => self.list_outcome(outcome),
                }
            }
            FieldKind::KeyList => {
                let keys = self.pane_mut().keys.as_mut()?;
                (keys.on_binding(id) == KeyListOutcome::Consumed).then_some(Action::Consumed)
            }
            // `consumes_key` already gave the input every key it binds.
            FieldKind::TextInput => Some(Action::Consumed),
            FieldKind::TextEditor | FieldKind::Table | FieldKind::KeyCapture | FieldKind::SelectionList => None,
        }
    }

    fn captures_mouse(&self) -> bool {
        self.search.as_ref().is_some_and(TextInput::is_selecting) || self.pane().is_selecting()
    }

    fn popup_contains(&self, column: u16, row: u16) -> bool {
        self.pane().overlay_contains(column, row)
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
        if let Some(list) = self.pane_mut().list_mut(index) {
            let outcome = list.handle_paste(text);
            return self.list_outcome(outcome);
        }
        let input = self.pane_mut().fields.get_mut(index)?.input_mut(stop)?;
        input.handle_paste(text);
        Some(Action::Consumed)
    }
}

#[cfg(test)]
mod tests;
