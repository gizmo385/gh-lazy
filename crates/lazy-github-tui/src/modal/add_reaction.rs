//! The Add reaction dialog (`react.add` on the PR Overview): a "Add a new reaction" heading, a checklist of the
//! reactions the user hasn't given, then Submit and Cancel. It closes with the checked reactions, in the order they
//! were checked; the opener posts them. Python's `AddReactionsModal` around an `AddReactionsContainer`.

use crossterm::event::MouseEvent;
use lazy_github_core::models::ReactionKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::selection_list::{ListOutcome, SelectionList};
use crate::keymap::BindingId;

/// `AddReactionsContainer { width: 50; height: 20 }`, border included.
const SIZE: (u16, u16) = (50, 20);

/// The heading: `Markdown("# Add a new reaction")`.
const HEADING: &str = "Add a new reaction";

/// The heading's block: `MarkdownH1`'s two rows of margin above and one below.
const HEADING_ROWS: u16 = 4;
const HEADING_MARGIN_TOP: u16 = 2;

/// The `Markdown` widget's `padding: 0 2`, inside which the heading is centered.
const HEADING_PADDING: u16 = 2;

/// The dialog's screen bindings: Python binds only `modal.close`.
const ADD_REACTION_BINDINGS: &[BindingId] = dialog_bindings![ModalClose];

/// The dialog's own actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddReactionAction {
    /// Submit: close with the checked reactions.
    Submit,
    /// `modal.close` or Cancel: close with none.
    Close,
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReactionField {
    List,
    Submit,
    Cancel,
}

impl ReactionField {
    fn button(self) -> Option<Pressed> {
        match self {
            ReactionField::Submit => Some(Pressed::Submit),
            ReactionField::Cancel => Some(Pressed::Cancel),
            ReactionField::List => None,
        }
    }
}

/// Where each part goes in the interior: the heading, the list and the buttons stacked and centered vertically (the
/// container's `align: center middle`), from the top when they don't fit.
struct Areas {
    heading: Rect,
    list: Rect,
    buttons: Rect,
}

impl Areas {
    fn new(area: Rect, list_height: u16) -> Self {
        let total = HEADING_ROWS + list_height + ButtonRow::HEIGHT;
        let mut y = area.y + area.height.saturating_sub(total) / 2;
        let mut next = |height: u16| {
            let rect = Rect { y, height, ..area }.intersection(area);
            y = y.saturating_add(height);
            rect
        };
        let block = next(HEADING_ROWS);
        let heading = Rect {
            x: area.x + HEADING_PADDING.min(area.width),
            y: block.y.saturating_add(HEADING_MARGIN_TOP),
            width: area.width.saturating_sub(2 * HEADING_PADDING),
            height: 1,
        }
        .intersection(area);
        let list = next(list_height);
        let buttons = next(ButtonRow::HEIGHT);
        Self { heading, list, buttons }
    }
}

/// The Add reaction dialog. Its output is the checked reactions in the order they were checked, empty when it was
/// cancelled.
#[derive(Debug)]
pub struct AddReactionDialog {
    ring: FocusRing<ReactionField>,
    list: SelectionList<ReactionKind>,
    buttons: ButtonRow,
    output: Option<Vec<ReactionKind>>,
}

impl AddReactionDialog {
    /// The dialog listing every kind of reaction but those in `given`, in GitHub's order, none checked, the list
    /// focused with its first row highlighted.
    pub fn new(given: &[ReactionKind]) -> Self {
        let options = ReactionKind::ALL
            .into_iter()
            .filter(|kind| !given.contains(kind))
            .map(|kind| (kind.emoji(), kind, false));
        Self {
            ring: FocusRing::new([ReactionField::List, ReactionField::Submit, ReactionField::Cancel]),
            list: SelectionList::with_options(options),
            buttons: ButtonRow::new("Submit"),
            output: None,
        }
    }

    /// The kinds listed, in order.
    pub fn listed(&self) -> Vec<ReactionKind> {
        (0..self.list.len())
            .filter_map(|index| self.list.value(index).copied())
            .collect()
    }

    /// The checked kinds, in the order they were checked.
    pub fn checked(&self) -> Vec<ReactionKind> {
        self.list.selected().into_iter().copied().collect()
    }

    fn action(action: AddReactionAction) -> Action {
        Action::Dialog(DialogAction::AddReaction(action))
    }

    fn pressed(button: Pressed) -> Action {
        Self::action(match button {
            Pressed::Submit => AddReactionAction::Submit,
            Pressed::Cancel => AddReactionAction::Close,
        })
    }
}

impl Component for AddReactionDialog {
    type Action = AddReactionAction;

    fn bindings(&self) -> &'static [BindingId] {
        ADD_REACTION_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        (id == BindingId::ModalClose).then(|| Self::action(AddReactionAction::Close))
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        if let Some((button, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            return Some(match outcome {
                ButtonMouse::Focus => {
                    let field = match button {
                        Pressed::Submit => ReactionField::Submit,
                        Pressed::Cancel => ReactionField::Cancel,
                    };
                    self.ring.focus(field);
                    Action::Consumed
                }
                ButtonMouse::Pressed => Self::pressed(button),
                ButtonMouse::Missed => Action::Consumed,
            });
        }
        if self.list.handle_mouse(&event) != ListOutcome::Ignored {
            self.ring.focus(ReactionField::List);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: AddReactionAction, _cx: &mut Cx<'_>) {
        self.output = Some(match action {
            AddReactionAction::Submit => self.checked(),
            AddReactionAction::Close => Vec::new(),
        });
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let p = theme.palette;
        let under = self.background(p).over(theme.background());
        let areas = Areas::new(area, self.list.height());
        let buf = frame.buffer_mut();

        // `MarkdownH1`: centered, in `$markdown-h1-color` and its text style, the whole row styled.
        let heading = p
            .markdown_h1_text_style
            .apply(Style::new().fg(theme.color(p.markdown_h1_color, under)));
        let width = usize::from(areas.heading.width);
        let text = format!("{HEADING:^width$}");
        buf.set_stringn(areas.heading.x, areas.heading.y, text, width, heading);

        let focused = self.ring.focused();
        self.list
            .render(buf, areas.list, theme, under, focused == Some(ReactionField::List));
        self.buttons
            .render(buf, areas.buttons, theme, focused.and_then(ReactionField::button));
    }
}

impl Dialog for AddReactionDialog {
    type Output = Vec<ReactionKind>;

    fn kind(&self) -> ModalKind {
        ModalKind::AddReaction
    }

    fn own_action(&self, action: DialogAction) -> Option<AddReactionAction> {
        match action {
            DialogAction::AddReaction(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.ring.step(direction);
    }

    fn size(&self, _available: Rect) -> (u16, u16) {
        SIZE
    }

    fn take_output(&mut self) -> Option<Vec<ReactionKind>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(ReactionField::List) => FieldKind::SelectionList,
            Some(ReactionField::Submit | ReactionField::Cancel) => FieldKind::Button,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match field {
            // Every key of the list's link is the list's, whether or not it changed anything.
            FieldKind::SelectionList => {
                let _ = self.list.on_binding(id);
                Some(Action::Consumed)
            }
            FieldKind::Button if id == BindingId::TextualButtonPress => {
                let button = self.ring.focused().and_then(ReactionField::button)?;
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(button, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
