//! Textual's `Tree` as the run Jobs tab uses it: a hidden root, expandable nodes that start collapsed, leaves, a line
//! cursor that starts on no line, the `textual.tree.*` keys, clicks and two-way scrolling.
//!
//! The tree is pure state, like the table: keys and clicks change it synchronously, and what the owner should act on
//! comes back as a [`TreeEvent`]. Hover highlighting is a Textual-only cosmetic (PLAN §10.8) and isn't drawn.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::components::command_log::Wheel;
use crate::components::scrollbar::{self, Axis, ScrollbarColors, ScrollbarState};
use crate::keymap::BindingId;
use crate::theme::Theme;

/// The bindings the tree answers while it has focus, in Textual's merged order (which the help panel lists them in):
/// `up` and `down` (`Tree`'s, at the place of the `ScrollView` keys they replace), the rest of the `ScrollView` keys,
/// then the rest of `Tree.BINDINGS`.
pub const TREE_BINDINGS: &[BindingId] = &[
    BindingId::TextualTreeCursorUp,
    BindingId::TextualTreeCursorDown,
    BindingId::TextualScrollLeft,
    BindingId::TextualScrollRight,
    BindingId::TextualScrollHome,
    BindingId::TextualScrollEnd,
    BindingId::TextualScrollPageUp,
    BindingId::TextualScrollPageDown,
    BindingId::TextualScrollPageLeft,
    BindingId::TextualScrollPageRight,
    BindingId::TextualTreeCursorParent,
    BindingId::TextualTreeCursorParentNextSibling,
    BindingId::TextualTreeCursorPreviousSibling,
    BindingId::TextualTreeCursorNextSibling,
    BindingId::TextualTreeSelect,
    BindingId::TextualTreeToggle,
    BindingId::TextualTreeToggleExpandAll,
];

/// `Tree.ICON_NODE`: an expandable node while collapsed.
const ICON_COLLAPSED: &str = "▶ ";
/// `Tree.ICON_NODE_EXPANDED`.
const ICON_EXPANDED: &str = "▼ ";
/// The icons' width in cells.
const ICON_WIDTH: usize = 2;
/// `Tree.guide_depth`: the cells each level indents by.
const GUIDE_DEPTH: usize = 4;
/// `Tree.LINES["default"]` at a guide depth of 4: the continuation of a level whose node has siblings below it, of
/// one whose node is the last, and the guides before a child that isn't the last and before the last.
const GUIDE_VERTICAL: &str = "│   ";
const GUIDE_SPACE: &str = "    ";
const GUIDE_CROSS: &str = "├── ";
const GUIDE_TERMINATOR: &str = "└── ";
/// Textual's `App.scroll_sensitivity_y` and `scroll_sensitivity_x`: how far one wheel notch scrolls.
const WHEEL_ROWS: usize = 2;
const WHEEL_COLUMNS: usize = 4;
/// `Tree:focus { background-tint: $foreground 5% }`.
const FOCUS_TINT_PERCENT: f64 = 5.0;

/// A node of a [`Tree`]: an index into the tree that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

/// What a tree binding does. One per binding in [`TREE_BINDINGS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeAction {
    /// `up`: the line above, or the last line when no line has the cursor.
    CursorUp,
    /// `down`: the line below, or the first line when no line has the cursor.
    CursorDown,
    /// `left`: one column left.
    ScrollLeft,
    /// `right`: one column right.
    ScrollRight,
    /// `home`: the cursor to the first line.
    Home,
    /// `end`: the cursor to the last line.
    End,
    /// `pageup`: the cursor a page up.
    PageUp,
    /// `pagedown`: the cursor a page down.
    PageDown,
    /// `ctrl+pageup`: a page left.
    PageLeft,
    /// `ctrl+pagedown`: a page right.
    PageRight,
    /// `shift+left`: the cursor to its node's parent.
    CursorParent,
    /// `shift+right`: the cursor to its node's parent's next sibling.
    CursorParentNextSibling,
    /// `shift+up`: the cursor to the previous sibling, or else the parent.
    CursorPreviousSibling,
    /// `shift+down`: the cursor to the next sibling, or else the parent's next sibling.
    CursorNextSibling,
    /// `enter`: toggles an expandable cursor node and reports the node as selected.
    Select,
    /// `space`: toggles the cursor node.
    Toggle,
    /// `shift+space`: expands the cursor node's siblings when all are collapsed, and collapses them otherwise.
    ToggleExpandAll,
}

impl TreeAction {
    /// The action a binding stands for, or `None` for a binding that isn't the tree's.
    pub fn from_binding(id: BindingId) -> Option<Self> {
        use BindingId as B;
        Some(match id {
            B::TextualTreeCursorUp => Self::CursorUp,
            B::TextualTreeCursorDown => Self::CursorDown,
            B::TextualScrollLeft => Self::ScrollLeft,
            B::TextualScrollRight => Self::ScrollRight,
            B::TextualScrollHome => Self::Home,
            B::TextualScrollEnd => Self::End,
            B::TextualScrollPageUp => Self::PageUp,
            B::TextualScrollPageDown => Self::PageDown,
            B::TextualScrollPageLeft => Self::PageLeft,
            B::TextualScrollPageRight => Self::PageRight,
            B::TextualTreeCursorParent => Self::CursorParent,
            B::TextualTreeCursorParentNextSibling => Self::CursorParentNextSibling,
            B::TextualTreeCursorPreviousSibling => Self::CursorPreviousSibling,
            B::TextualTreeCursorNextSibling => Self::CursorNextSibling,
            B::TextualTreeSelect => Self::Select,
            B::TextualTreeToggle => Self::Toggle,
            B::TextualTreeToggleExpandAll => Self::ToggleExpandAll,
            _ => return None,
        })
    }
}

/// What the tree tells its owner.
#[derive(Debug, PartialEq, Eq)]
pub enum TreeEvent<'a, T> {
    /// `enter`, or a click on a line: the node was selected (Textual's `Tree.NodeSelected`). An expandable node has
    /// already been toggled.
    Selected(NodeId, &'a T),
}

#[derive(Debug)]
struct Node<T> {
    label: Line<'static>,
    data: T,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// Added with `add` rather than `add_leaf`: it has an icon and can expand, with or without children.
    expandable: bool,
    expanded: bool,
}

/// A tree of nodes carrying `T`, under a hidden root.
#[derive(Debug)]
pub struct Tree<T> {
    nodes: Vec<Node<T>>,
    /// The hidden root's children.
    roots: Vec<NodeId>,
    /// The visible nodes, top to bottom: rebuilt when a node is added, expands or collapses.
    lines: Vec<NodeId>,
    /// Each node's line, or `None` while it's hidden.
    line_of: Vec<Option<usize>>,
    /// The widest line in cells, guides and icon included.
    content_width: usize,
    /// The node with the cursor. `None` until a key or click puts it on a line (Textual's `cursor_line = -1`).
    cursor: Option<NodeId>,
    scroll_x: usize,
    scroll_y: usize,
    /// The columns and rows the lines were drawn in last, left of and above any scrollbars: what paging, keeping
    /// the cursor in view and scrolling are measured against.
    viewport: (usize, usize),
}

impl<T> Default for Tree<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            lines: Vec::new(),
            line_of: Vec::new(),
            content_width: 0,
            cursor: None,
            scroll_x: 0,
            scroll_y: 0,
            viewport: (0, 0),
        }
    }
}

impl<T> Tree<T> {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a collapsed node under `parent`, or at the top level for `None`, after its existing siblings. An
    /// `expandable` node (Textual's `add`) shows `▶`/`▼` whether or not it has children; any other (`add_leaf`)
    /// shows no icon and never expands.
    pub fn add(&mut self, parent: Option<NodeId>, label: Line<'static>, data: T, expandable: bool) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            label,
            data,
            parent,
            children: Vec::new(),
            expandable,
            expanded: false,
        });
        self.line_of.push(None);
        let shown = if let Some(parent) = parent {
            self.nodes[parent.0].children.push(id);
            self.nodes[parent.0].expanded && self.line_of[parent.0].is_some()
        } else {
            self.roots.push(id);
            true
        };
        if shown {
            self.rebuild();
        }
        id
    }

    /// Replaces the nodes' labels, in the order they were added, for a theme switch. The text must stay the same:
    /// the content width isn't measured again. Extra labels are ignored, and nodes past the last label keep theirs.
    pub fn restyle_labels(&mut self, labels: impl IntoIterator<Item = Line<'static>>) {
        for (node, label) in self.nodes.iter_mut().zip(labels) {
            node.label = label;
        }
    }

    /// Whether the tree has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The data of `node`.
    pub fn data(&self, node: NodeId) -> &T {
        &self.nodes[node.0].data
    }

    /// The node with the cursor and its data, or `None` when no line has the cursor.
    pub fn cursor_node(&self) -> Option<(NodeId, &T)> {
        self.cursor.map(|id| (id, self.data(id)))
    }

    /// The line the cursor is on, from the top of the tree.
    pub fn cursor_line(&self) -> Option<usize> {
        self.cursor.and_then(|id| self.line_of[id.0])
    }

    /// Whether `node` is expanded.
    pub fn is_expanded(&self, node: NodeId) -> bool {
        self.nodes[node.0].expanded
    }

    /// Expands or collapses `node`, if it's expandable. A collapse that hides the cursor's node leaves no line with
    /// the cursor.
    pub fn set_expanded(&mut self, node: NodeId, expanded: bool) {
        let node = &mut self.nodes[node.0];
        if node.expandable && node.expanded != expanded {
            node.expanded = expanded;
            self.rebuild();
        }
    }

    /// The first visible line and column.
    pub fn scroll_offset(&self) -> (usize, usize) {
        (self.scroll_y, self.scroll_x)
    }

    /// A binding's action. `Selected` comes back for `enter` on a line.
    pub fn handle(&mut self, action: TreeAction) -> Option<TreeEvent<'_, T>> {
        let last = self.lines.len().checked_sub(1);
        let page = self.viewport.1.saturating_sub(1);
        match action {
            TreeAction::CursorUp => {
                let line = match self.cursor_line() {
                    Some(line) => Some(line.saturating_sub(1)),
                    None => last,
                };
                self.move_to_line(line);
            }
            TreeAction::CursorDown => {
                let line = match self.cursor_line() {
                    Some(line) => last.map(|last| (line + 1).min(last)),
                    None => last.map(|_| 0),
                };
                self.move_to_line(line);
            }
            TreeAction::PageUp => {
                let line = self.cursor_line().or(last).map(|line| line.saturating_sub(page));
                self.move_to_line(line);
            }
            TreeAction::PageDown => {
                let line = last.map(|last| (self.cursor_line().unwrap_or(0) + page).min(last));
                self.move_to_line(line);
            }
            TreeAction::Home => self.move_to_line(last.map(|_| 0)),
            TreeAction::End => self.move_to_line(last),
            TreeAction::ScrollLeft => self.scroll_x_by(-1),
            TreeAction::ScrollRight => self.scroll_x_by(1),
            TreeAction::PageLeft => self.scroll_x_by(-signed(self.viewport.0)),
            TreeAction::PageRight => self.scroll_x_by(signed(self.viewport.0)),
            TreeAction::CursorParent => {
                let parent = self.cursor.and_then(|id| self.nodes[id.0].parent);
                self.move_to_node(parent);
            }
            TreeAction::CursorParentNextSibling => {
                let parent = self.cursor.and_then(|id| self.nodes[id.0].parent);
                let target = parent.and_then(|parent| self.sibling(parent, 1));
                self.move_to_node(target);
            }
            TreeAction::CursorPreviousSibling => {
                let target = self
                    .cursor
                    .and_then(|id| self.sibling(id, -1).or(self.nodes[id.0].parent));
                self.move_to_node(target);
            }
            TreeAction::CursorNextSibling => {
                let target = self.cursor.and_then(|id| {
                    self.sibling(id, 1)
                        .or_else(|| self.nodes[id.0].parent.and_then(|parent| self.sibling(parent, 1)))
                });
                self.move_to_node(target);
            }
            TreeAction::Select => return self.select(),
            TreeAction::Toggle => {
                if let Some(id) = self.cursor {
                    self.toggle(id);
                }
            }
            TreeAction::ToggleExpandAll => self.toggle_expand_all(),
        }
        None
    }

    /// A mouse event against the tree drawn in `area`. A click on a line moves the cursor there and selects it, so
    /// `Selected` comes back; a click on a `▶`/`▼` icon only toggles its node. The wheel scrolls without moving the
    /// cursor. Clicks complete on the release, as Textual's do.
    pub fn handle_mouse(&mut self, event: &MouseEvent, area: Rect) -> Option<TreeEvent<'_, T>> {
        let at = Position::new(event.column, event.row);
        if !area.contains(at) {
            return None;
        }
        if let Some(wheel) = Wheel::from_mouse(event) {
            match wheel {
                Wheel::Up => self.scroll_y = self.scroll_y.saturating_sub(WHEEL_ROWS),
                Wheel::Down => self.scroll_y = (self.scroll_y + WHEEL_ROWS).min(self.max_scroll_y()),
                Wheel::Left => self.scroll_x_by(-signed(WHEEL_COLUMNS)),
                Wheel::Right => self.scroll_x_by(signed(WHEEL_COLUMNS)),
            }
            return None;
        }
        if event.kind != MouseEventKind::Up(MouseButton::Left) {
            return None;
        }
        let layout = ScrollLayout::new(area, self.content_width, self.lines.len());
        if !layout.lines.contains(at) {
            return None;
        }
        let line = self.scroll_y + usize::from(at.y - area.y);
        let &id = self.lines.get(line)?;
        let x = self.scroll_x + usize::from(at.x - area.x);
        let icon = self.guide_width(id);
        if self.nodes[id.0].expandable && (icon..icon + ICON_WIDTH).contains(&x) {
            self.toggle(id);
            return None;
        }
        self.cursor = Some(id);
        self.select()
    }

    /// Draws the visible lines into `area`, with scrollbars when the lines overflow it.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, focused: bool) {
        self.render_to(frame.buffer_mut(), area, theme, focused);
    }

    /// [`Self::render`] into a buffer: an owner that scrolls the tree with other content draws it whole off screen,
    /// then copies the rows that show.
    pub fn render_to(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, focused: bool) {
        let layout = ScrollLayout::new(area, self.content_width, self.lines.len());
        self.viewport = (usize::from(layout.lines.width), usize::from(layout.lines.height));
        self.scroll_y = self.scroll_y.min(self.max_scroll_y());
        self.scroll_x = self.scroll_x.min(self.max_scroll_x());

        let styles = Styles::new(theme, focused);
        buf.set_style(area.intersection(buf.area), styles.base);
        let scroll_x = u16::try_from(self.scroll_x).unwrap_or(u16::MAX);
        let visible = self.lines.iter().skip(self.scroll_y).take(self.viewport.1);
        for (row, &id) in (layout.lines.y..).zip(visible) {
            let line = self.line(id, &styles, focused);
            let rect = Rect {
                y: row,
                height: 1,
                ..layout.lines
            }
            .intersection(buf.area);
            Paragraph::new(line).scroll((0, scroll_x)).render(rect, buf);
        }
        self.render_scrollbars(buf, &layout, &styles);
    }

    fn render_scrollbars(&self, buf: &mut Buffer, layout: &ScrollLayout, styles: &Styles) {
        if let Some(bar) = layout.vertical_bar {
            let state = ScrollbarState {
                content: self.lines.len(),
                window: self.viewport.1,
                position: self.scroll_y,
            };
            scrollbar::render(buf, bar, Axis::Vertical, state, styles.scrollbar);
        }
        if let Some(bar) = layout.horizontal_bar {
            let state = ScrollbarState {
                content: self.content_width,
                window: self.viewport.0,
                position: self.scroll_x,
            };
            scrollbar::render(buf, bar, Axis::Horizontal, state, styles.scrollbar);
        }
        if let (Some(vertical), Some(horizontal)) = (layout.vertical_bar, layout.horizontal_bar) {
            let corner = Rect::new(vertical.x, horizontal.y, vertical.width, horizontal.height);
            for pos in corner.intersection(buf.area).positions() {
                buf[pos].set_symbol(" ").set_style(styles.corner);
            }
        }
    }

    /// A node's line as Textual's `_render_line` builds it: the guides, the icon, then the label, which alone takes
    /// the cursor style. The guides below a focused cursor's node take the selected style.
    fn line(&self, id: NodeId, styles: &Styles, focused: bool) -> Line<'static> {
        let path = self.path(id);
        let mut spans = Vec::new();
        // Whether the cursor's node is on the path so far: its descendants' guides are drawn selected.
        let mut selected = false;
        for (depth, &node) in path.iter().enumerate() {
            if depth > 0 {
                let style = if selected {
                    styles.guides_selected
                } else {
                    styles.guides
                };
                let last = self.is_last(node);
                let guide = match (node == id, last) {
                    (true, true) => GUIDE_TERMINATOR,
                    (true, false) => GUIDE_CROSS,
                    (false, true) => GUIDE_SPACE,
                    (false, false) => GUIDE_VERTICAL,
                };
                spans.push(Span::styled(guide, style));
            }
            selected |= focused && self.cursor == Some(node);
        }
        let node = &self.nodes[id.0];
        if node.expandable {
            let icon = if node.expanded { ICON_EXPANDED } else { ICON_COLLAPSED };
            spans.push(Span::styled(icon, styles.base));
        }
        let cursor = (self.cursor == Some(id)).then_some(styles.cursor);
        for span in &node.label.spans {
            let mut style = styles.base.patch(node.label.style).patch(span.style);
            if let Some(cursor) = cursor {
                style = style.patch(cursor);
            }
            spans.push(Span::styled(span.content.clone(), style));
        }
        Line::from(spans)
    }

    /// The nodes from the top level down to `id`.
    fn path(&self, id: NodeId) -> Vec<NodeId> {
        let mut path = vec![id];
        let mut node = id;
        while let Some(parent) = self.nodes[node.0].parent {
            path.push(parent);
            node = parent;
        }
        path.reverse();
        path
    }

    fn depth(&self, id: NodeId) -> usize {
        self.path(id).len() - 1
    }

    /// The cells before a node's icon or label.
    fn guide_width(&self, id: NodeId) -> usize {
        self.depth(id) * GUIDE_DEPTH
    }

    fn siblings(&self, id: NodeId) -> &[NodeId] {
        match self.nodes[id.0].parent {
            Some(parent) => &self.nodes[parent.0].children,
            None => &self.roots,
        }
    }

    fn is_last(&self, id: NodeId) -> bool {
        self.siblings(id).last() == Some(&id)
    }

    /// The sibling `offset` places after `id` (before it, when negative).
    fn sibling(&self, id: NodeId, offset: isize) -> Option<NodeId> {
        let siblings = self.siblings(id);
        let index = siblings.iter().position(|&sibling| sibling == id)?;
        siblings.get(index.checked_add_signed(offset)?).copied()
    }

    /// Recomputes the visible lines and the content width. The cursor stays on its node, or leaves the tree when its
    /// node is hidden.
    fn rebuild(&mut self) {
        let mut lines = Vec::with_capacity(self.lines.len());
        let mut stack: Vec<NodeId> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            lines.push(id);
            let node = &self.nodes[id.0];
            if node.expanded {
                stack.extend(node.children.iter().rev());
            }
        }
        self.line_of.fill(None);
        for (line, &id) in lines.iter().enumerate() {
            self.line_of[id.0] = Some(line);
        }
        self.content_width = lines.iter().map(|&id| self.line_width(id)).max().unwrap_or(0);
        self.lines = lines;
        if self.cursor.is_some_and(|id| self.line_of[id.0].is_none()) {
            self.cursor = None;
        }
        self.scroll_y = self.scroll_y.min(self.max_scroll_y());
        self.scroll_x = self.scroll_x.min(self.max_scroll_x());
    }

    fn line_width(&self, id: NodeId) -> usize {
        let node = &self.nodes[id.0];
        let icon = if node.expandable { ICON_WIDTH } else { 0 };
        self.guide_width(id) + icon + node.label.width()
    }

    fn toggle(&mut self, id: NodeId) {
        let expanded = self.nodes[id.0].expanded;
        self.set_expanded(id, !expanded);
    }

    /// Textual's `action_select_cursor` with `auto_expand`: the cursor's node toggles and is reported.
    fn select(&mut self) -> Option<TreeEvent<'_, T>> {
        let id = self.cursor?;
        self.toggle(id);
        Some(TreeEvent::Selected(id, self.data(id)))
    }

    fn toggle_expand_all(&mut self) {
        let Some(id) = self.cursor else {
            return;
        };
        // A leaf is never expanded, so it counts as collapsed, as in Textual.
        let expand = self.siblings(id).iter().all(|&sibling| !self.nodes[sibling.0].expanded);
        for index in 0..self.siblings(id).len() {
            let sibling = self.siblings(id)[index];
            let node = &mut self.nodes[sibling.0];
            if node.expandable {
                node.expanded = expand;
            }
        }
        self.rebuild();
        self.scroll_to_cursor();
    }

    fn move_to_line(&mut self, line: Option<usize>) {
        if let Some(&id) = line.and_then(|line| self.lines.get(line)) {
            self.cursor = Some(id);
            self.scroll_to_cursor();
        }
    }

    /// Moves the cursor to a visible node; `None` leaves it where it is.
    fn move_to_node(&mut self, node: Option<NodeId>) {
        if let Some(node) = node.filter(|node| self.line_of[node.0].is_some()) {
            self.cursor = Some(node);
            self.scroll_to_cursor();
        }
    }

    /// Scrolls vertically, as little as it takes, so the cursor's line is in view. Textual doesn't scroll sideways
    /// for it. Before the first draw there's no view to scroll.
    fn scroll_to_cursor(&mut self) {
        let height = self.viewport.1;
        let Some(line) = self.cursor_line().filter(|_| height > 0) else {
            return;
        };
        if line < self.scroll_y {
            self.scroll_y = line;
        } else if line >= self.scroll_y + height {
            self.scroll_y = line + 1 - height;
        }
    }

    fn scroll_x_by(&mut self, delta: isize) {
        self.scroll_x = self.scroll_x.saturating_add_signed(delta).min(self.max_scroll_x());
    }

    fn max_scroll_y(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport.1)
    }

    fn max_scroll_x(&self) -> usize {
        self.content_width.saturating_sub(self.viewport.0)
    }
}

/// A count of cells as a signed step. Screen sizes are far below `isize::MAX`.
fn signed(cells: usize) -> isize {
    isize::try_from(cells).unwrap_or(isize::MAX)
}

/// Where the lines and any scrollbars go in an area, by Textual's box model for a scrolling widget: each scrollbar
/// takes space from the lines, and each can make the other necessary, so the check runs twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScrollLayout {
    lines: Rect,
    vertical_bar: Option<Rect>,
    horizontal_bar: Option<Rect>,
}

impl ScrollLayout {
    fn new(area: Rect, content_width: usize, content_height: usize) -> Self {
        let (width, height) = (usize::from(area.width), usize::from(area.height));
        let vbar = usize::from(scrollbar::VERTICAL_WIDTH);
        let hbar = usize::from(scrollbar::HORIZONTAL_HEIGHT);
        let horizontal = content_width > width;
        let vertical = content_height > height.saturating_sub(if horizontal { hbar } else { 0 });
        let horizontal = content_width > width.saturating_sub(if vertical { vbar } else { 0 });
        let vertical = content_height > height.saturating_sub(if horizontal { hbar } else { 0 });

        let lines = Rect {
            width: area
                .width
                .saturating_sub(if vertical { scrollbar::VERTICAL_WIDTH } else { 0 }),
            height: area
                .height
                .saturating_sub(if horizontal { scrollbar::HORIZONTAL_HEIGHT } else { 0 }),
            ..area
        };
        Self {
            lines,
            vertical_bar: vertical.then(|| Rect {
                x: lines.right(),
                width: area.width - lines.width,
                ..lines
            }),
            horizontal_bar: horizontal.then(|| Rect {
                y: lines.bottom(),
                height: area.height - lines.height,
                ..lines
            }),
        }
    }
}

/// The tree's styles for one frame, from `Tree.DEFAULT_CSS`.
struct Styles {
    /// The tree's own text and background: `$foreground` on `$surface`, tinted while focused. The icons and the space
    /// past each line take it too.
    base: Style,
    /// `.tree--guides`.
    guides: Style,
    /// `.tree--guides-selected`: the guides below the cursor's node, while focused.
    guides_selected: Style,
    /// `.tree--cursor`, over the label only.
    cursor: Style,
    scrollbar: ScrollbarColors,
    corner: Style,
}

impl Styles {
    fn new(theme: &Theme, focused: bool) -> Self {
        let p = theme.palette;
        let surface = p.surface.over(theme.background());
        let bg = if focused {
            theme.tint(surface, p.foreground, FOCUS_TINT_PERCENT)
        } else {
            surface
        };
        let base = Style::new().fg(theme.color(p.foreground, bg)).bg(theme.paint(bg));
        // `Tree:ansi` comes last, then `&:light`, then `&:focus`.
        let guide_color = if theme.ansi {
            p.ansi_background
        } else if !theme.dark {
            p.surface_darken_1
        } else if focused {
            p.surface_lighten_3
        } else {
            p.surface_lighten_2
        };
        let guides = Style::new().fg(theme.color(guide_color, bg));
        let guides_selected = Style::new().fg(theme.color(p.block_cursor_background, bg));

        // Focused, the block cursor's colors and text style; blurred, its blurred background and text style under the
        // tree's own `$foreground`, which overrides the label's colors as the focused cursor's does.
        let cursor = if focused {
            let cursor_bg = p.block_cursor_background.over(bg);
            p.block_cursor_text_style.apply(
                Style::new()
                    .fg(theme.color(p.block_cursor_foreground, cursor_bg))
                    .bg(theme.paint(cursor_bg)),
            )
        } else {
            let cursor_bg = p.block_cursor_blurred_background.over(bg);
            p.block_cursor_blurred_text_style.apply(
                Style::new()
                    .fg(theme.color(p.foreground, cursor_bg))
                    .bg(theme.paint(cursor_bg)),
            )
        };

        let back = p.scrollbar_background.over(surface);
        let scrollbar = ScrollbarColors {
            bar: theme.color(p.scrollbar, back),
            back: theme.paint(back),
            fg: theme.color(p.foreground, bg),
            bg: theme.paint(bg),
        };
        let corner = Style::new()
            .fg(Color::Reset)
            .bg(theme.color(p.scrollbar_corner_color, surface));
        Self {
            base,
            guides,
            guides_selected,
            cursor,
            scrollbar,
            corner,
        }
    }
}

#[cfg(test)]
mod tests;
