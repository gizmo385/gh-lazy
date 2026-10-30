//! The Debug Info dialog: the debug markdown (what LazyGithub runs on, and the config), with Copy to Clipboard and
//! Close. Python's `DebugModal`.
//!
//! The text scrolls (Py bug B95's answer): Python clips it at the buttons with no way to see the rest. The rule under
//! it scrolls with it, as Python lays the rule out right after the markdown.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use lazy_github_core::debug::{DebugEnv, debug_markdown};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::button::{ButtonMouse, ButtonRow, Pressed};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::keymap::BindingId;
use crate::theme::Theme;
use crate::toast::Toast;

/// The dialog's screen bindings, which also give the footer's order: `SUBMIT_DIALOG`, `CLOSE_DIALOG`.
const DEBUG_BINDINGS: &[BindingId] = dialog_bindings![ModalSubmit, ModalClose];

/// The dialog's own actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugInfoAction {
    /// Copy to Clipboard: copy the markdown and say so. The dialog stays open.
    Copy,
    /// `modal.close` or Close: done.
    Close,
    /// `modal.submit`: bound and shown, as in Python, where the modal has no `submit` action. Does nothing.
    Submit,
    /// A scroll key while the text has focus.
    Scroll(Scroll),
    /// A wheel notch over the text.
    Wheel(Wheel),
}

/// The dialog's focusable fields, in tab order. The text is last: Python's opening focus is Copy's, and its text
/// isn't a stop at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugField {
    Copy,
    Close,
    Body,
}

/// The scrolling text: the rendered markdown, then the `Rule` under it.
#[derive(Debug)]
struct DebugBody {
    markdown: Markdown,
    layouts: LayoutCache,
}

impl Layout for DebugBody {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let DebugBody { markdown, layouts } = self;
        layouts.get(width, theme, || body_lines(markdown, width, theme))
    }
}

/// The debug info, with a way to copy it. Its output is `()`: nothing waits on it.
#[derive(Debug)]
pub struct DebugInfoDialog {
    /// The markdown as `lazy-github debug` prints it, which Copy copies.
    text: String,
    body: DebugBody,
    scroll: ScrollView,
    buttons: ButtonRow,
    ring: FocusRing<DebugField>,
    /// Where the text was drawn last, for the wheel and clicks.
    body_area: Rect,
    closed: bool,
}

impl DebugInfoDialog {
    /// The debug info for `env` and `config`, as they are now. Focus starts on Copy to Clipboard.
    pub fn new(env: &DebugEnv, config: &Config) -> Self {
        let text = debug_markdown(env, config);
        let mut buttons = ButtonRow::new("Copy to Clipboard");
        buttons.button_mut(Pressed::Cancel).set_label("Close");
        Self {
            body: DebugBody {
                markdown: Markdown::parse(&text),
                layouts: LayoutCache::default(),
            },
            text,
            scroll: ScrollView::default(),
            buttons,
            ring: FocusRing::new([DebugField::Copy, DebugField::Close, DebugField::Body]),
            body_area: Rect::default(),
            closed: false,
        }
    }

    /// The markdown Copy copies.
    pub fn text(&self) -> &str {
        &self.text
    }

    fn action(action: DebugInfoAction) -> Action {
        Action::Dialog(DialogAction::DebugInfo(action))
    }

    fn pressed(which: Pressed) -> Action {
        Self::action(match which {
            Pressed::Submit => DebugInfoAction::Copy,
            Pressed::Cancel => DebugInfoAction::Close,
        })
    }

    /// The first line of the text shown, for tests.
    #[cfg(test)]
    fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }
}

impl Component for DebugInfoDialog {
    type Action = DebugInfoAction;

    fn bindings(&self) -> &'static [BindingId] {
        DEBUG_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match id {
            BindingId::ModalSubmit => Some(Self::action(DebugInfoAction::Submit)),
            BindingId::ModalClose => Some(Self::action(DebugInfoAction::Close)),
            _ => None,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect, cx: &Cx<'_>) -> Option<Action> {
        if let Some((which, outcome)) = self.buttons.handle_mouse(&event, cx.now()) {
            let field = match which {
                Pressed::Submit => DebugField::Copy,
                Pressed::Cancel => DebugField::Close,
            };
            self.ring.focus(field);
            return Some(match outcome {
                ButtonMouse::Pressed => Self::pressed(which),
                ButtonMouse::Focus | ButtonMouse::Missed => Action::Consumed,
            });
        }
        if self.body_area.contains((event.column, event.row).into()) {
            if let Some(wheel) = Wheel::from_mouse(&event) {
                return Some(Self::action(DebugInfoAction::Wheel(wheel)));
            }
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.ring.focus(DebugField::Body);
            }
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: DebugInfoAction, cx: &mut Cx<'_>) {
        match action {
            // B96's answer: the markdown itself, as `lazy-github debug` prints it, with nothing left selected.
            DebugInfoAction::Copy => {
                cx.dispatch(Action::Copy(self.text.clone()));
                cx.notify(Toast::info("Debug info copied to clipboard").with_title("Copied"));
            }
            DebugInfoAction::Close => self.closed = true,
            DebugInfoAction::Submit => {}
            DebugInfoAction::Scroll(scroll) => self.scroll.scroll(scroll),
            DebugInfoAction::Wheel(wheel) => self.scroll.wheel(wheel),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        // `DebugButtons { dock: bottom }`: the buttons and their margins take the bottom rows, the text the rest.
        let rows = self.buttons.height().min(area.height);
        let text_area = Rect {
            height: area.height - rows,
            ..area
        };
        let buttons_area = Rect {
            y: text_area.bottom(),
            height: rows,
            ..area
        };
        // `DebugContainer { align: center middle }`: text shorter than its room sits in the middle of it.
        let height = self.body.layout(text_area.width, theme).lines.len();
        let spare = usize::from(text_area.height).saturating_sub(height);
        // Less than the area's height, which is a u16.
        let top = u16::try_from(spare / 2).unwrap_or(0);
        self.body_area = Rect {
            y: text_area.y + top,
            height: text_area.height - top,
            ..text_area
        };
        self.scroll.render(frame, self.body_area, theme, &mut self.body);

        let focused = match self.ring.focused() {
            Some(DebugField::Copy) => Some(Pressed::Submit),
            Some(DebugField::Close) => Some(Pressed::Cancel),
            Some(DebugField::Body) | None => None,
        };
        self.buttons.render(frame.buffer_mut(), buttons_area, theme, focused);
    }
}

impl Dialog for DebugInfoDialog {
    type Output = ();

    fn kind(&self) -> ModalKind {
        ModalKind::DebugInfo
    }

    fn own_action(&self, action: DialogAction) -> Option<DebugInfoAction> {
        match action {
            DialogAction::DebugInfo(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.ring.step(direction);
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        // `max-width: 80%; max-height: 80%`, rounded down as Textual does; the container's `1fr` reaches both.
        (available.width * 4 / 5, available.height * 4 / 5)
    }

    fn take_output(&mut self) -> Option<()> {
        std::mem::take(&mut self.closed).then_some(())
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(DebugField::Copy | DebugField::Close) => FieldKind::Button,
            Some(DebugField::Body) => FieldKind::Scroll,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, cx: &Cx<'_>) -> Option<Action> {
        match (field, self.ring.focused()?) {
            (FieldKind::Button, DebugField::Copy | DebugField::Close) if id == BindingId::TextualButtonPress => {
                let which = if self.ring.is_focused(DebugField::Copy) {
                    Pressed::Submit
                } else {
                    Pressed::Cancel
                };
                // A press inside the debounce still takes the key.
                Some(
                    self.buttons
                        .press(which, cx.now())
                        .map_or(Action::Consumed, Self::pressed),
                )
            }
            (FieldKind::Scroll, DebugField::Body) => {
                Scroll::from_binding(id).map(|scroll| Self::action(DebugInfoAction::Scroll(scroll)))
            }
            _ => None,
        }
    }
}

/// The markdown laid out `width` wide, then the rule: the markdown's last margin, which collapses with the rule's
/// `margin: 1 0`, the rule across the width in `$secondary`, and its bottom margin.
fn body_lines(markdown: &Markdown, width: u16, theme: &Theme) -> Rendered {
    let mut out = markdown.layout(width, theme);
    if width == 0 {
        return out;
    }
    let palette = theme.palette;
    let under = palette.surface_lighten_3.over(theme.background());
    let rule = Style::new().fg(theme.color(palette.secondary, under));
    out.lines.push(Line::default());
    out.lines
        .push(Line::from(Span::styled("─".repeat(usize::from(width)), rule)));
    out.lines.push(Line::default());
    out
}

#[cfg(test)]
mod tests;
