//! The footer row: the shown bindings of the focused context, as Textual's `Footer` draws them. Python's
//! `LazyGithubFooter`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::component::Owner;
use crate::keymap::{ActiveGroup, BindingId};
use crate::theme::Theme;

/// The footer for `groups`, from [`crate::keymap::active_bindings`]. Each shown group is one item: `" {key} "` in the
/// key style, then `"{description} "`. Text past the right edge is cut off.
pub struct Footer<'a> {
    /// The active bindings, innermost first.
    pub groups: &'a [ActiveGroup],
    /// The theme.
    pub theme: &'a Theme,
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
}

/// The `(key, description)` of each footer item, in order.
pub fn footer_items(groups: &[ActiveGroup]) -> Vec<(String, String)> {
    shown(groups)
        .map(|group| (group.first_key().to_owned(), group.description.to_owned()))
        .collect()
}

fn shown(groups: &[ActiveGroup]) -> impl Iterator<Item = &ActiveGroup> {
    groups.iter().filter(|group| group.show)
}

/// One item's two spans.
fn item_spans(group: &ActiveGroup) -> [Span<'static>; 2] {
    [
        Span::raw(format!(" {} ", group.first_key())),
        Span::raw(format!("{} ", group.description)),
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
        for group in shown(self.groups) {
            if x >= area.right() {
                break;
            }
            let width: usize = item_spans(group).iter().map(Span::width).sum();
            let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.right() - x);
            hits.push(FooterHit {
                rect: Rect { x, width, ..area },
                owner: group.owner,
                id: group.id,
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
        let item = palette.footer_item_background.over(bar);
        let key_bg = palette.footer_key_background.over(item);
        let description_bg = palette.footer_description_background.over(item);
        let key_style = Style::new()
            .fg(theme.color(palette.footer_key_foreground, key_bg))
            .bg(theme.paint(key_bg))
            .add_modifier(Modifier::BOLD);
        let description_style = Style::new()
            .fg(theme.color(palette.footer_description_foreground, description_bg))
            .bg(theme.paint(description_bg));
        let spans: Vec<Span<'static>> = shown(self.groups)
            .flat_map(|group| {
                // Textual's `FooterKey.-disabled` is `text-style: dim`.
                let dim = if group.enabled {
                    Style::new()
                } else {
                    Style::new().add_modifier(Modifier::DIM)
                };
                let [key, description] = item_spans(group);
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
