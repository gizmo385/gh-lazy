//! Link hints (`link.follow`, `link.follow_external`): a label over every link drawn on the main screen, and typing a
//! label follows that link. Python's `LinkHintScreen`, a transparent modal screen over the unchanged main screen.

use std::convert::Infallible;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::{Dialog, DialogAction, FocusMove, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{Component, Cx, RenderCx};
use crate::components::link_map::LinkMap;
use crate::keymap::{BindingId, KeyChord, Keymap};
use crate::theme::ThemeColor;

/// The letters labels are made of, in the order they're handed out: the home row first.
pub const HINT_ALPHABET: &str = "asdfghjklqwertyuiopzxcvbnm";

/// `count` labels: one letter each up to 26, otherwise two letters each (first letter major: `aa`, `as`, …, `sa`), so
/// no label is the start of another (Py bug B100's answer: Python mixes lengths, which leaves the two-letter ones
/// unreachable). At most 676.
pub fn hint_codes(count: usize) -> Vec<String> {
    let letters: Vec<char> = HINT_ALPHABET.chars().collect();
    if count <= letters.len() {
        return letters[..count].iter().map(char::to_string).collect();
    }
    letters
        .iter()
        .flat_map(|&first| letters.iter().map(move |&second| format!("{first}{second}")))
        .take(count)
        .collect()
}

/// One label: its letters, where it's drawn, and the link it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hint {
    code: String,
    x: u16,
    y: u16,
    target: Arc<str>,
}

/// The labels over the links drawn in the last frame. Its output is the picked link with whether it goes to the
/// browser whatever it is (`F`), or `None` when it was cancelled.
#[derive(Debug)]
pub struct LinkHintsDialog {
    hints: Vec<Hint>,
    /// The letters typed so far.
    typed: String,
    external: bool,
    /// The outer `Option` is whether it's done, the inner the dialog's output.
    #[allow(clippy::option_option)]
    output: Option<Option<(Arc<str>, bool)>>,
}

impl LinkHintsDialog {
    /// Labels for the links in `links`, in the order they were drawn, or `None` when there are none. Two links starting
    /// at the same cell get one label. `external` is `F`'s: the pick goes to the browser.
    pub fn new(links: &LinkMap, external: bool) -> Option<Self> {
        let mut starts: Vec<(u16, u16)> = Vec::new();
        let labelled: Vec<_> = links
            .iter()
            .filter(|link| {
                let start = (link.x, link.y);
                let first = !starts.contains(&start);
                starts.push(start);
                first
            })
            .collect();
        if labelled.is_empty() {
            return None;
        }
        let codes = hint_codes(labelled.len());
        // Past 676 links, the rest get no label.
        let hints = codes
            .into_iter()
            .zip(labelled)
            .map(|(code, link)| Hint {
                code,
                x: link.x,
                y: link.y,
                target: Arc::clone(&link.target),
            })
            .collect();
        Some(Self {
            hints,
            typed: String::new(),
            external,
            output: None,
        })
    }

    /// Each label's letters and cell, in the order they were handed out.
    pub fn labels(&self) -> Vec<(&str, u16, u16)> {
        self.hints
            .iter()
            .map(|hint| (hint.code.as_str(), hint.x, hint.y))
            .collect()
    }

    /// Whether the pick goes to the browser whatever it is (`F`).
    pub fn is_external(&self) -> bool {
        self.external
    }

    /// One letter of a label typed: the link whose label it completes is picked; a letter no label continues with
    /// cancels; anything else waits for the next letter.
    fn type_letter(&mut self, letter: char) {
        self.typed.push(letter);
        if let Some(hint) = self.hints.iter().find(|hint| hint.code == self.typed) {
            self.output = Some(Some((Arc::clone(&hint.target), self.external)));
        } else if !self.hints.iter().any(|hint| hint.code.starts_with(&self.typed)) {
            self.output = Some(None);
        }
    }
}

impl Component for LinkHintsDialog {
    type Action = Infallible;

    fn bindings(&self) -> &'static [BindingId] {
        dialog_bindings![]
    }

    /// Every key: while the labels are up, the keyboard is theirs. Only the priority bindings that pass every modal
    /// (`ctrl+q`, `ctrl+c`) come first.
    fn consumes_key(&self, _key: &KeyChord, _keymap: &Keymap) -> bool {
        true
    }

    /// `escape` cancels and a letter of [`HINT_ALPHABET`] types; every other key is ignored and the labels stay.
    /// Python's `LinkHintScreen.on_key` reads these keys itself, with no binding ids, so a rebinding doesn't move them.
    fn handle_key(&mut self, key: KeyEvent, _cx: &Cx<'_>) -> Option<Action> {
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Esc if key.modifiers.is_empty() => self.output = Some(None),
            KeyCode::Char(letter) if plain && HINT_ALPHABET.contains(letter) => self.type_letter(letter),
            _ => {}
        }
        Some(Action::Consumed)
    }

    /// The pointer does nothing while the labels are up.
    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        Some(Action::Consumed)
    }

    fn update(&mut self, action: Infallible, _cx: &mut Cx<'_>) {
        match action {}
    }

    /// Each label over its link's first cells, padded to the longest: bold black on `$warning`, cut at the screen's
    /// edge.
    fn render(&mut self, frame: &mut Frame<'_>, _area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        let under = theme.palette.warning.over(theme.background());
        let style = Style::new()
            .fg(theme.color(ThemeColor::BLACK, under))
            .bg(theme.paint(under))
            .add_modifier(Modifier::BOLD);
        let width = self.hints.iter().map(|hint| hint.code.len()).max().unwrap_or(0);
        let screen = frame.area();
        let buf = frame.buffer_mut();
        for hint in &self.hints {
            if !screen.contains((hint.x, hint.y).into()) {
                continue;
            }
            let room = usize::from(screen.right() - hint.x);
            buf.set_stringn(hint.x, hint.y, format!("{:<width$}", hint.code), room, style);
        }
    }
}

impl Dialog for LinkHintsDialog {
    type Output = Option<(Arc<str>, bool)>;

    fn kind(&self) -> ModalKind {
        ModalKind::LinkHints
    }

    fn own_action(&self, _action: DialogAction) -> Option<Infallible> {
        None
    }

    fn move_focus(&mut self, _direction: FocusMove) {}

    fn size(&self, available: Rect) -> (u16, u16) {
        (available.width, available.height)
    }

    fn shows_footer(&self) -> bool {
        false
    }

    fn overlay(&self) -> bool {
        true
    }

    fn take_output(&mut self) -> Option<Option<(Arc<str>, bool)>> {
        self.output.take()
    }
}

#[cfg(test)]
mod tests;
