//! One tab of the Settings modal: a config section's docstring and its fields, each a bold label with its widget in
//! the right half, in a scroll area. Python's `SettingsSection` (a `Vertical` with `border: blank` and `padding: 1 0`)
//! of `FieldSetting`s (a two-column grid with `margin-bottom: 2`) inside a `ScrollableContainer`.

use std::ops::Range;

use crossterm::event::MouseEvent;
use lazy_github_core::config::settings::{Field, FieldKind, FieldValue, Section};
use lazy_github_core::config::{Config, TEXTUAL_THEMES};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::SettingsFocus;
use super::list_field::ListField;
use crate::components::markdown::wrap;
use crate::components::scroll_view::ScrollView;
use crate::components::select::{self, DEFAULT_PROMPT, Select};
use crate::components::switch::Switch;
use crate::components::text_input::{self, TextInput};
use crate::fuzzy::fuzzy_match;
use crate::modal::form::FormScroll;
use crate::theme::{Paint, Theme};

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

/// Rich's `dim`, as Textual draws `[dim]` text: the text color at this alpha over its background.
const DIM_ALPHA: f64 = 0.66;

/// The template field's placeholder (Python's `PathInput`).
const PATH_PLACEHOLDER: &str = "Enter file path...";

/// A field's widget, by its kind.
#[derive(Debug, Clone)]
pub(super) enum FieldWidget {
    /// A bool.
    Switch(Switch),
    /// A choice or the theme, with the blank "Select" first. `client_type`'s is disabled and has only its value.
    Select(Select<&'static str>),
    /// Text: numbers, timestamps, paths, the base URL and the template.
    Text(TextInput),
    /// A list of strings.
    List(ListField),
}

/// One field: its descriptor, its label and its widget.
#[derive(Debug, Clone)]
pub(super) struct SettingField {
    pub(super) field: &'static Field,
    label: String,
    pub(super) widget: FieldWidget,
}

impl SettingField {
    fn new(field: &'static Field, config: &Config) -> Self {
        let widget = match field.get(config) {
            FieldValue::Bool(on) => FieldWidget::Switch(Switch::new(on)),
            FieldValue::Choice(value) => FieldWidget::Select(choice_select(field.kind, value)),
            FieldValue::List(items) => FieldWidget::List(ListField::new(items)),
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
    /// B89, answered).
    pub(super) fn value(&self) -> Option<FieldValue> {
        Some(match &self.widget {
            FieldWidget::Switch(switch) => FieldValue::Bool(switch.value()),
            FieldWidget::Select(select) => FieldValue::Choice(select.value().copied()?),
            FieldWidget::Text(input) => FieldValue::Text(input.value().to_owned()),
            FieldWidget::List(list) => FieldValue::List(list.value()),
        })
    }

    /// The text input focus `stop` lands on in this field, if it's one.
    pub(super) fn input_mut(&mut self, stop: SettingsFocus) -> Option<&mut TextInput> {
        match (&mut self.widget, stop) {
            (FieldWidget::Text(input), SettingsFocus::Field(_)) => Some(input),
            (FieldWidget::List(list), SettingsFocus::Field(_)) => Some(&mut list.input),
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
        FieldKind::Theme => TEXTUAL_THEMES.iter().map(|&name| (theme_label(name), name)).collect(),
        FieldKind::Bool | FieldKind::List | FieldKind::OptionalPath | FieldKind::Text(_) => {
            vec![(value.to_owned(), value)]
        }
    };
    let mut select = Select::new(DEFAULT_PROMPT, options, Some(&value));
    select.set_disabled(matches!(kind, FieldKind::ReadOnlyChoice(_)));
    select
}

/// A theme's option label: Python's `name.title().replace("-", " ")`, such as "Textual Dark".
fn theme_label(name: &str) -> String {
    Theme::by_name(name).map_or_else(|| name.to_owned(), |theme| theme.display_name())
}

/// Where a pane's parts go, in rows from the top of its content, at one width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PaneLayout {
    /// The docstring's rows, wrapped.
    doc: Vec<String>,
    /// Each field's first row. A hidden field takes no rows: its top is where the next shown one starts.
    tops: Vec<u16>,
    /// The content's height.
    height: u16,
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
    /// A field's focus stop: a widget, or a list field's input or checklist.
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
    pub(super) scroll: FormScroll,
    /// The layout at the last draw, for revealing a field and for the mouse.
    layout: PaneLayout,
}

impl SettingsPane {
    pub(super) fn new(section: &'static Section, config: &Config) -> Self {
        Self {
            section,
            fields: section
                .fields
                .iter()
                .map(|field| SettingField::new(field, config))
                .collect(),
            shown: vec![true; section.fields.len()],
            scroll: FormScroll::default(),
            layout: PaneLayout::default(),
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
    }

    /// Whether field `index` is shown.
    pub(super) fn is_shown(&self, index: usize) -> bool {
        self.shown.get(index).copied().unwrap_or(false)
    }

    /// The pane's focus stops in order: each shown field's widget, and a list field's checklist after its input.
    pub(super) fn stops(&self) -> Vec<SettingsFocus> {
        let mut stops = Vec::new();
        for (index, field) in self.fields.iter().enumerate() {
            if !self.is_shown(index) {
                continue;
            }
            stops.push(SettingsFocus::Field(index));
            if matches!(field.widget, FieldWidget::List(_)) {
                stops.push(SettingsFocus::ListItems(index));
            }
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
    pub(super) fn open_select_mut(&mut self) -> Option<&mut Select<&'static str>> {
        self.fields.iter_mut().find_map(|field| match &mut field.widget {
            FieldWidget::Select(select) if select.is_open() => Some(select),
            _ => None,
        })
    }

    /// The content rows of focus stop `stop` at the last draw, and the rows with its margin: a list field's input or
    /// checklist alone (the checklist with its `margin-top`), any other widget whole.
    fn stop_rows(&self, stop: SettingsFocus) -> Option<(Range<u16>, Range<u16>)> {
        let (index, items) = match stop {
            SettingsFocus::Field(index) => (index, false),
            SettingsFocus::ListItems(index) => (index, true),
            _ => return None,
        };
        if !self.is_shown(index) {
            return None;
        }
        let top = *self.layout.tops.get(index)?;
        let field = self.fields.get(index)?;
        Some(match &field.widget {
            FieldWidget::List(list) if items => {
                let start = top + ListField::list_top();
                let rows = start..start + list.list_height();
                (rows.clone(), start - 1..rows.end)
            }
            FieldWidget::List(_) => {
                let rows = top..top + text_input::HEIGHT;
                (rows.clone(), rows)
            }
            _ => {
                let rows = top..top + field.height();
                (rows.clone(), rows)
            }
        })
    }

    /// Scrolls focus stop `stop` into view as Textual does when focus moves to it.
    pub(super) fn reveal(&mut self, stop: SettingsFocus) {
        if let Some((rows, with_margin)) = self.stop_rows(stop) {
            self.scroll.center(rows, with_margin);
        }
    }

    /// The layout at content width `width`.
    fn layout(&self, width: u16) -> PaneLayout {
        let inner = width.saturating_sub(2 * BORDER);
        let doc = self
            .section
            .doc
            .map(|doc| wrap(doc, usize::from(inner.saturating_sub(2 * DOC_PADDING))))
            .unwrap_or_default();
        let mut y = PaneLayout::DOC_TOP;
        if !doc.is_empty() {
            y = y
                .saturating_add(u16::try_from(doc.len()).unwrap_or(u16::MAX))
                .saturating_add(GAP);
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
        let column = inner / 2;
        PaneLayout {
            doc,
            tops,
            height: y.saturating_add(PADDING + BORDER),
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
            let stop = match &self.fields[index].widget {
                FieldWidget::List(_) if form_row >= area.y + ListField::list_top() => SettingsFocus::ListItems(index),
                FieldWidget::List(_) if form_row >= area.y + text_input::HEIGHT => return Some(PaneHit::Area),
                _ => SettingsFocus::Field(index),
            };
            return Some(PaneHit::Stop(stop));
        }
        Some(PaneHit::Area)
    }

    /// A left press on focus stop `stop`, which the dialog has just focused: a switch toggles, a select opens or
    /// closes, a text input places its cursor, and a checklist toggles the item under it.
    pub(super) fn press(&mut self, stop: SettingsFocus, event: MouseEvent) {
        let moved = self.scroll.to_content(event);
        let (SettingsFocus::Field(index) | SettingsFocus::ListItems(index)) = stop else {
            return;
        };
        let Some(area) = self.widget_area(index) else { return };
        let input_area = Rect {
            width: area.width.min(MAX_INPUT_WIDTH),
            height: text_input::HEIGHT,
            ..area
        };
        let Some(field) = self.fields.get_mut(index) else {
            return;
        };
        match (&mut field.widget, stop) {
            (FieldWidget::Switch(switch), _) => {
                let _ = switch.handle_mouse(&moved, area);
            }
            (FieldWidget::Select(select), _) => {
                let _ = select.toggle();
            }
            (FieldWidget::Text(input), _) => input.press(input_area, event.column),
            (FieldWidget::List(list), SettingsFocus::Field(_)) => list.input.press(input_area, event.column),
            (FieldWidget::List(list), _) => {
                let _ = list.list.handle_mouse(&moved);
            }
        }
    }

    /// Whether a text input is selecting with the mouse.
    pub(super) fn is_selecting(&self) -> bool {
        self.fields.iter().any(|field| match &field.widget {
            FieldWidget::Text(input) => input.is_selecting(),
            FieldWidget::List(list) => list.input.is_selecting(),
            FieldWidget::Switch(_) | FieldWidget::Select(_) => false,
        })
    }

    /// A mouse event while a text input is selecting: it goes to that input, in the content's coordinates.
    pub(super) fn captured_mouse(&mut self, event: MouseEvent) -> bool {
        let moved = self.scroll.to_content(event);
        self.fields.iter_mut().any(|field| match &mut field.widget {
            FieldWidget::Text(input) => input.captured_mouse(moved),
            FieldWidget::List(list) => list.input.captured_mouse(moved),
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
        let p = theme.palette;
        let mut layout = self.layout(area.width);
        let narrowed = ScrollView::content_width(area, usize::from(layout.height));
        if narrowed != area.width {
            layout = self.layout(narrowed);
        }
        let fill = Style::new().fg(theme.color(p.foreground, under)).bg(theme.paint(under));
        let bold = Style::new()
            .fg(theme.color(p.foreground, under))
            .add_modifier(Modifier::BOLD);
        let doc = if theme.ansi {
            Style::new()
                .fg(theme.color(p.foreground, under))
                .add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(theme.color(p.foreground.multiply_alpha(DIM_ALPHA), under))
        };
        let fields = &mut self.fields;
        let shown = &self.shown;
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
                        FieldWidget::List(list) => {
                            list.input.render(content_buf, input, theme, under, focused);
                            let checklist = Rect {
                                y: y + ListField::list_top(),
                                height: list.list_height(),
                                ..input
                            };
                            let items_focused = focus == Some(SettingsFocus::ListItems(index));
                            list.list
                                .render_solid(content_buf, checklist, theme, under, items_focused, p.primary);
                        }
                    }
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
