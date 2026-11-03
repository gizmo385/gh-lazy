//! One tab of the Settings modal: a config section's docstring and its fields, each a bold label with its widget in
//! the right half, in a scroll area. Python's `SettingsSection` (a `Vertical` with `border: blank` and `padding: 1 0`)
//! of `FieldSetting`s (a two-column grid with `margin-bottom: 2`) inside a `ScrollableContainer`.

use std::ops::Range;

use crossterm::event::MouseEvent;
use lazy_github_core::config::Config;
use lazy_github_core::config::settings::{Field, FieldKind, FieldValue, Section};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::key_list::{self, KeyList};
use super::{SettingsFocus, refused_toast};
use crate::components::command_log::Wheel;
use crate::components::list_editor::{ListEditor, ListEditorOutcome};
use crate::components::markdown::wrap;
use crate::components::scroll_view::ScrollView;
use crate::components::select::{self, DEFAULT_PROMPT, Select};
use crate::components::switch::Switch;
use crate::components::text_input::{self, TextInput};
use crate::fuzzy::fuzzy_match;
use crate::modal::form::FormScroll;
use crate::theme::{Catalog, Paint, Theme};
use crate::toast::Toast;

/// The section's blank border, each side.
const BORDER: u16 = 1;

/// The section's `padding: 1 0`, above and below its contents.
const PADDING: u16 = 1;

/// The docstring's `padding: 0 1`, each side.
const DOC_PADDING: u16 = 1;

/// The blank rows after the docstring and after each field (`margin-bottom: 2`).
const GAP: u16 = 2;

/// An `Input`'s `width: 70`, which the grid cell narrows.
const MAX_INPUT_WIDTH: u16 = 70;

/// The template field's placeholder (Python's `PathInput`).
const PATH_PLACEHOLDER: &str = "Enter file path...";

/// A list field's add input placeholder: what its items are.
const REPO_PLACEHOLDER: &str = "owner/repo";
const USER_PLACEHOLDER: &str = "username";

/// A field's widget, by its kind.
#[derive(Debug, Clone)]
pub(super) enum FieldWidget {
    /// A bool.
    Switch(Switch),
    /// A choice or the theme, with the blank "Select" first. `client_type`'s is disabled and has only its value.
    Select(Select<&'static str>),
    /// Text: numbers, timestamps, paths, the base URL and the template.
    Text(TextInput),
    /// A list of strings, in a list editor (#343).
    List(ListEditor),
}

/// One field: its descriptor, its label and its widget.
#[derive(Debug, Clone)]
pub(super) struct SettingField {
    pub(super) field: &'static Field,
    label: String,
    pub(super) widget: FieldWidget,
    /// The theme field's stored name, resolved: the option that leaves the name as written, so Save doesn't turn an
    /// untouched alias (`textual-dark`) into its theme's name. `None` for an unknown name and other fields.
    unchanged: Option<&'static str>,
}

impl SettingField {
    fn new(field: &'static Field, config: &Config) -> Self {
        let mut unchanged = None;
        let widget = match field.get(config) {
            FieldValue::Bool(on) => FieldWidget::Switch(Switch::new(on)),
            FieldValue::Choice(value) => FieldWidget::Select(choice_select(field.kind, value)),
            FieldValue::Name(name) => {
                unchanged = Catalog::builtin().resolve(&name).theme().map(|theme| theme.name);
                FieldWidget::Select(theme_select(unchanged))
            }
            FieldValue::List(items) => {
                let placeholder = if field.name == "additional_suggested_pr_reviewers" {
                    USER_PLACEHOLDER
                } else {
                    REPO_PLACEHOLDER
                };
                FieldWidget::List(ListEditor::new(items, placeholder))
            }
            FieldValue::Text(text) => {
                let placeholder = if field.kind == FieldKind::OptionalPath {
                    PATH_PLACEHOLDER
                } else {
                    ""
                };
                let mut input = TextInput::new(placeholder);
                input.set_value(text);
                FieldWidget::Text(input)
            }
        };
        Self {
            field,
            label: field.label(),
            widget,
            unchanged,
        }
    }

    /// The label without its colon, as an error message names the field: "Default Ttl".
    pub(super) fn name(&self) -> &str {
        self.label.trim_end_matches(':')
    }

    /// Whether the widget is disabled, and so out of the focus ring.
    pub(super) fn is_disabled(&self) -> bool {
        matches!(&self.widget, FieldWidget::Select(select) if select.is_disabled())
    }

    /// The rows the widget takes.
    fn height(&self) -> u16 {
        match &self.widget {
            FieldWidget::Switch(_) => Switch::HEIGHT,
            FieldWidget::Select(_) => select::HEIGHT,
            FieldWidget::Text(_) => text_input::HEIGHT,
            FieldWidget::List(list) => list.height(),
        }
    }

    /// The edited value to write back, or `None` to leave the field as it was: a select on its blank option (Py bug
    /// B89, answered), or the theme select still on the theme the stored name stands for.
    pub(super) fn value(&self) -> Option<FieldValue> {
        Some(match &self.widget {
            FieldWidget::Switch(switch) => FieldValue::Bool(switch.value()),
            FieldWidget::Select(select) if self.field.kind == FieldKind::Theme => {
                let name = select.value().copied().filter(|&name| Some(name) != self.unchanged)?;
                FieldValue::Name(name.to_owned())
            }
            FieldWidget::Select(select) => FieldValue::Choice(select.value().copied()?),
            FieldWidget::Text(input) => FieldValue::Text(input.value().to_owned()),
            FieldWidget::List(list) => FieldValue::List(list.items().to_vec()),
        })
    }

    /// The text input focus `stop` lands on in this field, if it's one.
    pub(super) fn input_mut(&mut self, stop: SettingsFocus) -> Option<&mut TextInput> {
        match (&mut self.widget, stop) {
            (FieldWidget::Text(input), SettingsFocus::Field(_)) => Some(input),
            _ => None,
        }
    }
}

/// The select for a choice field showing `value`. `client_type`'s is disabled and offers only the value it shows
/// (Python's disabled select shows only `Github_Cli`).
fn choice_select(kind: FieldKind, value: &'static str) -> Select<&'static str> {
    let options: Vec<(String, &'static str)> = match kind {
        FieldKind::Choice(choices) => choices.iter().map(|c| (c.label.to_owned(), c.value)).collect(),
        FieldKind::ReadOnlyChoice(choices) => choices
            .iter()
            .filter(|c| c.value == value)
            .map(|c| (c.label.to_owned(), c.value))
            .collect(),
        FieldKind::Theme | FieldKind::Bool | FieldKind::List | FieldKind::OptionalPath | FieldKind::Text(_) => {
            vec![(value.to_owned(), value)]
        }
    };
    let mut select = Select::new(DEFAULT_PROMPT, options, Some(&value));
    select.set_disabled(matches!(kind, FieldKind::ReadOnlyChoice(_)));
    select
}

/// The theme select: every theme in catalog order, labeled by its display name, on `selected` (a theme's own name),
/// or on the blank option for a name no theme has.
fn theme_select(selected: Option<&'static str>) -> Select<&'static str> {
    let options = Catalog::builtin()
        .themes()
        .map(|theme| (theme.display_name(), theme.name))
        .collect::<Vec<_>>();
    Select::new(DEFAULT_PROMPT, options, selected.as_ref())
}

/// Where a pane's parts go, in rows from the top of its content, at one width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PaneLayout {
    /// The docstring's rows, wrapped.
    doc: Vec<String>,
    /// Each field's first row. A hidden field takes no rows: its top is where the next shown one starts.
    tops: Vec<u16>,
    /// The Key Bindings list's rows, its detail line included, when the tab has it and the filter shows a binding.
    keys: Option<Range<u16>>,
    /// The content's height.
    height: u16,
    /// The content width it was laid out at.
    width: u16,
    /// The widget column's offset from the content's left edge, and its width.
    column: u16,
    column_width: u16,
}

impl PaneLayout {
    /// The first docstring row.
    const DOC_TOP: u16 = BORDER + PADDING;
}

/// What a press in the pane's scroll area landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaneHit {
    /// A field's focus stop: its widget.
    Stop(SettingsFocus),
    /// The area around the fields, which focuses the scroll area itself.
    Area,
}

/// One tab: its section, its fields, which of them the search shows, and its scroll position.
#[derive(Debug)]
pub(super) struct SettingsPane {
    section: &'static Section,
    pub(super) fields: Vec<SettingField>,
    /// Per field, whether the search filter shows it (Textual's `display`). A hidden field takes no rows and no
    /// focus, but still saves.
    shown: Vec<bool>,
    /// The Key Bindings tab's list, after its (no) fields; `None` on every other tab.
    pub(super) keys: Option<KeyList>,
    pub(super) scroll: FormScroll,
    /// The layout at the last draw, for revealing a field and for the mouse.
    layout: PaneLayout,
    /// A stop to reveal at the next draw, once the layout has caught up with a list field's change.
    reveal_pending: Option<SettingsFocus>,
}

impl SettingsPane {
    pub(super) fn new(section: &'static Section, config: &Config) -> Self {
        Self {
            keys: (section.key == "bindings").then(|| KeyList::new(&config.bindings.overrides)),
            section,
            fields: section
                .fields
                .iter()
                .map(|field| SettingField::new(field, config))
                .collect(),
            shown: vec![true; section.fields.len()],
            scroll: FormScroll::default(),
            layout: PaneLayout::default(),
            reveal_pending: None,
        }
    }

    /// The tab's title.
    pub(super) fn title(&self) -> &'static str {
        self.section.title
    }

    /// Shows only the fields whose field name `query` fuzzy-matches (`show_command_log`, not the label), or
    /// every field for `None`. Python's `SettingsSection.filter_field_settings`.
    pub(super) fn filter(&mut self, query: Option<&str>) {
        for (shown, field) in self.shown.iter_mut().zip(&self.fields) {
            *shown = query.is_none_or(|query| fuzzy_match(query, field.field.name).is_some());
        }
        if let Some(keys) = &mut self.keys {
            keys.filter(query);
        }
    }

    /// Whether the Key Bindings list is waiting for a key.
    pub(super) fn is_capturing(&self) -> bool {
        self.keys.as_ref().is_some_and(KeyList::is_capturing)
    }

    /// Whether field `index` is shown.
    pub(super) fn is_shown(&self, index: usize) -> bool {
        self.shown.get(index).copied().unwrap_or(false)
    }

    /// The pane's focus stops in order: each shown field's widget, a list field's editor included.
    pub(super) fn stops(&self) -> Vec<SettingsFocus> {
        let mut stops: Vec<SettingsFocus> = (0..self.fields.len())
            .filter(|&index| self.is_shown(index))
            .map(SettingsFocus::Field)
            .collect();
        if self.keys.as_ref().is_some_and(KeyList::has_lines) {
            stops.push(SettingsFocus::KeyList);
        }
        stops
    }

    /// Whether `stop` is a disabled widget.
    pub(super) fn is_disabled(&self, stop: SettingsFocus) -> bool {
        match stop {
            SettingsFocus::Field(index) => self.fields.get(index).is_some_and(SettingField::is_disabled),
            _ => false,
        }
    }

    /// Closes every open select: focus left it.
    pub(super) fn close_selects(&mut self) {
        for field in &mut self.fields {
            if let FieldWidget::Select(select) = &mut field.widget {
                select.close();
            }
        }
    }

    /// The open select, if any.
    /// Whether screen cell `(column, row)` is on an open select's list.
    pub(super) fn overlay_contains(&self, column: u16, row: u16) -> bool {
        self.fields.iter().any(|field| match &field.widget {
            FieldWidget::Select(select) => select.overlay_contains(column, row),
            _ => false,
        })
    }

    pub(super) fn open_select_mut(&mut self) -> Option<&mut Select<&'static str>> {
        self.fields.iter_mut().find_map(|field| match &mut field.widget {
            FieldWidget::Select(select) if select.is_open() => Some(select),
            _ => None,
        })
    }

    /// The content rows of focus stop `stop` in `layout`, and the rows with its margin: a list field's highlighted
    /// line alone (its open input while editing), so a long list stays usable in a short tab; any other widget whole.
    fn stop_rows(&self, layout: &PaneLayout, stop: SettingsFocus) -> Option<(Range<u16>, Range<u16>)> {
        let index = match stop {
            SettingsFocus::Field(index) => index,
            SettingsFocus::KeyList => {
                let rows = layout.keys.clone()?;
                return Some((rows.clone(), rows));
            }
            _ => return None,
        };
        if !self.is_shown(index) {
            return None;
        }
        let top = *layout.tops.get(index)?;
        let field = self.fields.get(index)?;
        let rows = match &field.widget {
            FieldWidget::List(list) => {
                let line = list.highlighted_rows();
                top + line.start..top + line.end
            }
            _ => top..top + field.height(),
        };
        Some((rows.clone(), rows))
    }

    /// Scrolls focus stop `stop` into view as Textual does when focus moves to it.
    pub(super) fn reveal(&mut self, stop: SettingsFocus) {
        if let Some((rows, with_margin)) = self.stop_rows(&self.layout, stop) {
            self.scroll.center(rows, with_margin);
        }
    }

    /// [`Self::reveal`] at the next draw: a list field's highlight moved, and the field may have grown or shrunk.
    pub(super) fn reveal_later(&mut self, stop: SettingsFocus) {
        self.reveal_pending = Some(stop);
    }

    /// The list editor of field `index`, if it's a list field.
    pub(super) fn list_mut(&mut self, index: usize) -> Option<&mut ListEditor> {
        match &mut self.fields.get_mut(index)?.widget {
            FieldWidget::List(list) => Some(list),
            _ => None,
        }
    }

    /// Commits field `index`'s open list input, as when focus leaves it: the toast for text it refused.
    pub(super) fn commit_list(&mut self, index: usize) -> Option<Toast> {
        let field = self.fields.get_mut(index)?;
        let FieldWidget::List(list) = &mut field.widget else {
            return None;
        };
        match list.commit() {
            ListEditorOutcome::Refused { text } => Some(refused_toast(field.name(), &text)),
            _ => None,
        }
    }

    /// Commits every open list input, as Save does first: the toasts for text they refused.
    pub(super) fn commit_lists(&mut self) -> Vec<Toast> {
        (0..self.fields.len())
            .filter_map(|index| self.commit_list(index))
            .collect()
    }

    /// The layout at content width `width` in a scroll area `rows` high. The Key Bindings list fills the area below
    /// the docstring, down to its last row, unless that leaves it fewer than [`key_list::MIN_LINES`] lines: then the
    /// tab grows and scrolls.
    fn layout(&self, width: u16, rows: u16) -> PaneLayout {
        let inner = width.saturating_sub(2 * BORDER);
        let doc = self
            .section
            .doc
            .map(|doc| wrap(doc, usize::from(inner.saturating_sub(2 * DOC_PADDING))))
            .unwrap_or_default();
        let mut y = PaneLayout::DOC_TOP;
        let doc_end = y.saturating_add(u16::try_from(doc.len()).unwrap_or(u16::MAX));
        if !doc.is_empty() {
            y = doc_end.saturating_add(GAP);
        }
        let tops = self
            .fields
            .iter()
            .zip(&self.shown)
            .map(|(field, &shown)| {
                let top = y;
                if shown {
                    y = y.saturating_add(field.height()).saturating_add(GAP);
                }
                top
            })
            .collect();
        let mut height = y.saturating_add(PADDING + BORDER);
        let mut keys = None;
        if self.keys.as_ref().is_some_and(KeyList::has_lines) {
            // One blank row below the docstring, and the detail line in the area's last row.
            let top = doc_end.saturating_add(1);
            let bottom = rows.max(top.saturating_add(key_list::MIN_LINES + 1));
            keys = Some(top..bottom);
            height = bottom;
        }
        let column = inner / 2;
        PaneLayout {
            doc,
            tops,
            keys,
            width,
            height,
            column,
            column_width: inner - column,
        }
    }

    /// The widget area of field `index` in the content's coordinates (from row 0), as of the last draw. `None` for a
    /// hidden field.
    fn widget_area(&self, index: usize) -> Option<Rect> {
        if !self.is_shown(index) {
            return None;
        }
        let content = self.scroll.content();
        let top = *self.layout.tops.get(index)?;
        let field = self.fields.get(index)?;
        Some(Rect {
            x: content.x + BORDER + self.layout.column,
            y: content.y + top,
            width: self.layout.column_width,
            height: field.height(),
        })
    }

    /// What a press at screen cell `(column, row)` landed on, if it's in the scroll area.
    pub(super) fn hit(&self, column: u16, row: u16) -> Option<PaneHit> {
        let form_row = self.scroll.content_row(row)?;
        let content = self.scroll.content();
        if column >= content.right() {
            return Some(PaneHit::Area);
        }
        for index in 0..self.fields.len() {
            let Some(area) = self.widget_area(index) else { continue };
            if !(area.x..area.right()).contains(&column) || !(area.y..area.bottom()).contains(&form_row) {
                continue;
            }
            return Some(PaneHit::Stop(SettingsFocus::Field(index)));
        }
        if self
            .keys
            .as_ref()
            .is_some_and(|keys| self.layout.keys.is_some() && keys.contains(column, form_row))
        {
            return Some(PaneHit::Stop(SettingsFocus::KeyList));
        }
        Some(PaneHit::Area)
    }

    /// A left press on focus stop `stop`, which the dialog has just focused (`focused` says whether it had focus
    /// before): a switch toggles, a select opens or closes, a text input places its cursor, and a list editor
    /// highlights, edits or adds ([`ListEditor::handle_mouse`]). The toast for text a list editor refused, if any.
    pub(super) fn press(&mut self, stop: SettingsFocus, event: MouseEvent, focused: bool) -> Option<Toast> {
        let moved = self.scroll.to_content(event);
        let SettingsFocus::Field(index) = stop else {
            return None;
        };
        let area = self.widget_area(index)?;
        let input_area = Rect {
            width: area.width.min(MAX_INPUT_WIDTH),
            height: text_input::HEIGHT,
            ..area
        };
        let field = self.fields.get_mut(index)?;
        match &mut field.widget {
            FieldWidget::Switch(switch) => {
                let _ = switch.handle_mouse(&moved, area);
            }
            FieldWidget::Select(select) => {
                let _ = select.toggle();
            }
            FieldWidget::Text(input) => input.press(input_area, event.column),
            FieldWidget::List(list) => {
                let outcome = list.handle_mouse(&moved, focused);
                self.reveal_later(stop);
                if let ListEditorOutcome::Refused { text } = outcome {
                    return Some(refused_toast(self.fields[index].name(), &text));
                }
            }
        }
        None
    }

    /// A left press on the Key Bindings list, which the dialog has just focused; `focused` says whether it had focus
    /// before the press.
    pub(super) fn press_keys(&mut self, event: MouseEvent, focused: bool) {
        let moved = self.scroll.to_content(event);
        if let Some(keys) = &mut self.keys {
            let _ = keys.press(moved, focused);
        }
    }

    /// A wheel notch at screen cell `(column, row)` in the scroll area: the Key Bindings list or a list editor scrolls
    /// under it, and the tab elsewhere.
    pub(super) fn wheel(&mut self, wheel: Wheel, column: u16, row: u16) {
        let form_row = self.scroll.content_row(row);
        if let Some(form_row) = form_row {
            let shown = &self.shown;
            let under = self
                .fields
                .iter_mut()
                .zip(shown)
                .find_map(|(field, &shown)| match &mut field.widget {
                    FieldWidget::List(list) if shown && list.contains(column, form_row) => Some(list),
                    _ => None,
                });
            if let Some(list) = under {
                list.wheel(wheel);
                return;
            }
        }
        match (&mut self.keys, form_row) {
            (Some(keys), Some(form_row)) if self.layout.keys.is_some() && keys.contains(column, form_row) => {
                keys.wheel(wheel);
            }
            _ => self.scroll.wheel(wheel),
        }
    }

    /// Whether a text input is selecting with the mouse.
    pub(super) fn is_selecting(&self) -> bool {
        self.fields.iter().any(|field| match &field.widget {
            FieldWidget::Text(input) => input.is_selecting(),
            FieldWidget::List(list) => list.is_selecting(),
            FieldWidget::Switch(_) | FieldWidget::Select(_) => false,
        })
    }

    /// A mouse event while a text input is selecting: it goes to that input, in the content's coordinates.
    pub(super) fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        let moved = self.scroll.to_content(event);
        self.fields.iter_mut().any(|field| match &mut field.widget {
            FieldWidget::Text(input) => input.captured_mouse(moved),
            FieldWidget::List(list) => list.captured_mouse(moved),
            FieldWidget::Switch(_) | FieldWidget::Select(_) => false,
        })
    }

    /// Draws the pane into its scroll area `area` over the dialog's background `under`, with `focus` drawn focused.
    pub(super) fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        theme: &Theme,
        under: Paint,
        focus: Option<SettingsFocus>,
    ) {
        let r = theme.roles();
        let mut layout = self.layout(area.width, area.height);
        let narrowed = ScrollView::content_width(area, usize::from(layout.height));
        if narrowed != area.width {
            layout = self.layout(narrowed, area.height);
        }
        if let Some(stop) = self.reveal_pending.take()
            && let Some((rows, with_margin)) = self.stop_rows(&layout, stop)
        {
            self.scroll.center_at_draw(rows, with_margin);
        }
        let fill = Style::new().fg(theme.color(r.text.plain, under)).bg(theme.paint(under));
        let bold = Style::new()
            .fg(theme.color(r.text.plain, under))
            .add_modifier(Modifier::BOLD);
        let doc = theme.rich_dim(Style::new().fg(theme.color(r.text.plain, under)), r.text.plain, under);
        let fields = &mut self.fields;
        let shown = &self.shown;
        let keys = &mut self.keys;
        let plan = &layout;
        self.scroll
            .render(buf, area, theme, fill, layout.height, |content_buf, content| {
                let doc_x = content.x + BORDER + DOC_PADDING;
                let doc_width = usize::from(content.width.saturating_sub(2 * (BORDER + DOC_PADDING)));
                for (row, line) in (PaneLayout::DOC_TOP..).zip(&plan.doc) {
                    content_buf.set_stringn(doc_x, content.y + row, line, doc_width, doc);
                }
                for (index, ((field, &top), &shown)) in fields.iter_mut().zip(&plan.tops).zip(shown).enumerate() {
                    if !shown {
                        continue;
                    }
                    let y = content.y + top;
                    content_buf.set_stringn(content.x + BORDER, y, &field.label, usize::from(plan.column), bold);
                    let widget = Rect {
                        x: content.x + BORDER + plan.column,
                        y,
                        width: plan.column_width,
                        height: field.height(),
                    };
                    let input = Rect {
                        width: widget.width.min(MAX_INPUT_WIDTH),
                        height: text_input::HEIGHT,
                        ..widget
                    };
                    let focused = focus == Some(SettingsFocus::Field(index));
                    match &mut field.widget {
                        FieldWidget::Switch(switch) => switch.render(content_buf, widget, theme, under, focused),
                        // The box loses its focus to the open list.
                        FieldWidget::Select(select) => {
                            let focused = focused && !select.is_open();
                            select.render_box(content_buf, widget, theme, under, focused);
                        }
                        FieldWidget::Text(text) => text.render(content_buf, input, theme, under, focused),
                        FieldWidget::List(list) => list.render(content_buf, widget, theme, under, focused),
                    }
                }
                if let (Some(keys), Some(rows)) = (keys, &plan.keys) {
                    let area = Rect::new(
                        content.x + BORDER,
                        content.y + rows.start,
                        content.width.saturating_sub(2 * BORDER),
                        rows.end - rows.start,
                    );
                    keys.render(content_buf, area, theme, under, focus == Some(SettingsFocus::KeyList));
                }
            });
        self.layout = layout;
    }

    /// Draws an open select's list over everything, below its box on screen.
    pub(super) fn render_overlay(&mut self, buf: &mut Buffer, screen: Rect, theme: &Theme, under: Paint) {
        let open = self
            .fields
            .iter()
            .position(|field| matches!(&field.widget, FieldWidget::Select(select) if select.is_open()));
        let Some(index) = open else { return };
        let (Some(area), Some(top)) = (self.widget_area(index), self.layout.tops.get(index).copied()) else {
            return;
        };
        let Some(below) = self.scroll.screen_rows(top..top + select::HEIGHT) else {
            return;
        };
        let below = Rect {
            x: area.x,
            width: area.width,
            ..below
        };
        if let FieldWidget::Select(select) = &mut self.fields[index].widget {
            select.render_overlay(buf, below, screen, theme, under);
        }
    }
}
