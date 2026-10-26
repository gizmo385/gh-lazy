//! The footer row: the shown bindings of the focused context, as Textual's `Footer` draws them. Python's
//! `LazyGithubFooter`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::component::Owner;
use crate::keymap::{ActiveBinding, BindingId};
use crate::theme::{Paint, Theme};

/// The footer for `items`, from [`footer_items`]. Each item is `" {key} "` in the key style, then `"{description} "`.
/// Text past the right edge is cut off.
pub struct Footer<'a> {
    /// The items, in order.
    pub items: &'a [FooterItem],
    /// The index in `items` of the one under the mouse, drawn on `$block-hover-background`.
    pub hovered: Option<usize>,
    /// The theme.
    pub theme: &'a Theme,
}

/// One footer entry: the shown bindings of one action, as their first one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterItem {
    /// The chain link that declares the first binding, which handles a click on the item.
    pub owner: Owner,
    /// The first binding: what a click on the item runs.
    pub id: BindingId,
    /// The first binding's key, as displayed.
    pub key: String,
    /// The first binding's label.
    pub description: &'static str,
    /// False for a disabled item, drawn dim.
    pub enabled: bool,
}

/// What a click on a footer item runs: the binding, at the chain link that declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterHit {
    /// The cells the item covers.
    pub rect: Rect,
    /// The chain link.
    pub owner: Owner,
    /// The binding.
    pub id: BindingId,
    /// False for a disabled item, which a click doesn't run.
    pub enabled: bool,
}

/// The footer items for the active keys from [`crate::keymap::active_bindings`], as Textual's `Footer.compose`
/// picks them: the `show` bindings grouped by action across the whole chain, each item at its action's first key.
pub fn footer_items(active: &[ActiveBinding]) -> Vec<FooterItem> {
    let mut items: Vec<FooterItem> = Vec::new();
    for binding in active.iter().filter(|binding| binding.def().show) {
        let def = binding.def();
        if items.iter().any(|item| item.id.def().action == def.action) {
            continue;
        }
        items.push(FooterItem {
            owner: binding.owner,
            id: binding.id,
            key: binding.key.clone(),
            description: def.description,
            enabled: binding.enabled,
        });
    }
    items
}

/// One item's two spans.
fn item_spans(item: &FooterItem) -> [Span<'static>; 2] {
    [
        Span::raw(format!(" {} ", item.key)),
        Span::raw(format!("{} ", item.description)),
    ]
}

impl Footer<'_> {
    /// Where each item is drawn in `area`, cut to the area. Items wholly past the right edge have none.
    pub fn hits(&self, area: Rect) -> Vec<FooterHit> {
        let mut x = area.x;
        let mut hits = Vec::new();
        if area.is_empty() {
            return hits;
        }
        for item in self.items {
            if x >= area.right() {
                break;
            }
            let width: usize = item_spans(item).iter().map(Span::width).sum();
            let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.right() - x);
            hits.push(FooterHit {
                rect: Rect { x, width, ..area },
                owner: item.owner,
                id: item.id,
                enabled: item.enabled,
            });
            x += width;
        }
        hits
    }
}

impl Widget for Footer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let palette = theme.palette;
        let bar = palette.footer_background.over(theme.background());
        // Textual's `FooterKey:hover` swaps the item's background, and the key and description backgrounds are
        // drawn over it, so the whole item lights up. Its `color` changes nothing: both parts set their own.
        let styles = |item_bg: Paint| {
            let key_bg = palette.footer_key_background.over(item_bg);
            let description_bg = palette.footer_description_background.over(item_bg);
            let key = Style::new()
                .fg(theme.color(palette.footer_key_foreground, key_bg))
                .bg(theme.paint(key_bg))
                .add_modifier(Modifier::BOLD);
            let description = Style::new()
                .fg(theme.color(palette.footer_description_foreground, description_bg))
                .bg(theme.paint(description_bg));
            (key, description)
        };
        let plain = styles(palette.footer_item_background.over(bar));
        let hovered = styles(palette.block_hover_background.over(bar));
        let spans: Vec<Span<'static>> = self
            .items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                let (key_style, description_style) = if self.hovered == Some(index) { hovered } else { plain };
                // Textual's `FooterKey.-disabled` is `text-style: dim`.
                let dim = if item.enabled {
                    Style::new()
                } else {
                    Style::new().add_modifier(Modifier::DIM)
                };
                let [key, description] = item_spans(item);
                [
                    key.style(key_style.patch(dim)),
                    description.style(description_style.patch(dim)),
                ]
            })
            .collect();
        Paragraph::new(Line::from(spans))
            .style(Style::new().bg(theme.paint(bar)))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests;
