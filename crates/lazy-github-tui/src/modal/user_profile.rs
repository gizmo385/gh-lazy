//! The User profile dialog: a user's or organization's profile and their recent public repos, which can be opened.
//! Python's `UserProfileModal` around a `UserProfileContainer`.
//!
//! The whole body scrolls as one, as Python's `ScrollableContainer` does: the header, the bio, the info lines, the
//! counts and the repos table are laid out as one column of lines, with blank rows where the table goes. The table is
//! drawn whole off screen and its visible rows copied into those rows.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazy_github_core::models::{FullUser, Repository};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Dialog, DialogAction, FieldKind, FocusMove, FocusRing, ModalKind, dialog_bindings};
use crate::action::Action;
use crate::component::{ChainLink, Component, Cx, RenderCx};
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{self, LinkSpan, Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::components::table::{Cell, CellValue, Column, Handled, SearchableTable, SortSpec, TableEvent, TableItem};
use crate::keymap::BindingId;
use crate::theme::{Palette, Theme, ThemeColor};

/// The dialog's screen bindings: only `modal.close` of its own.
const PROFILE_BINDINGS: &[BindingId] = dialog_bindings![ModalClose];

/// `UserProfileModal > ScrollableContainer { width: 90 }`, border included.
const WIDTH: u16 = 90;

/// `UserProfileContainer { padding: 1 2 }`: columns each side.
const PADDING_X: usize = 2;

/// `UserReposTable { max-height: 20 }`: header, rows and scrollbar.
const MAX_TABLE_HEIGHT: u16 = 20;

/// Python's `private_string`.
const PRIVATE: &str = "✓";
const PUBLIC: &str = "✘";

/// The dialog's own actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserProfileAction {
    /// A scroll key while the body has focus.
    Scroll(Scroll),
    /// A wheel notch over the body.
    Wheel(Wheel),
    /// `modal.close`: done, with nothing picked.
    Close,
    /// `enter` on the table, or a click on its cursor row: done, with this repo.
    Pick(Box<Repository>),
}

/// The dialog's focusable fields, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileField {
    /// The scrolling body.
    Body,
    /// The repos table, when there are repos.
    Repos,
}

/// One row of the repos table: a repo and its place in the API's order.
#[derive(Debug, Clone)]
struct ProfileRepo {
    index: usize,
    repo: Repository,
}

impl TableItem for ProfileRepo {
    type Key = String;
    type Ctx = ();

    fn key(&self) -> String {
        self.repo.full_name.clone()
    }

    fn cells(&self, (): &()) -> Vec<Cell> {
        let repo = &self.repo;
        // The table sorts by its first column's value. Python's table isn't sorted, so the owner cell sorts by the
        // API's order (latest update first) and shows the login.
        let index = i64::try_from(self.index).unwrap_or(i64::MAX);
        vec![
            Cell::new(CellValue::Int(index), repo.owner.login.clone()),
            Cell::text(repo.name.clone()),
            Cell::text(if repo.private { PRIVATE } else { PUBLIC }),
            Cell::text(repo.description.clone().unwrap_or_default()),
        ]
    }
}

/// The scrolling body's content: everything but the table's own rows, which it only makes room for.
struct ProfileBody {
    user: FullUser,
    bio: Option<Markdown>,
    /// `None` when there are no repos: the body says so instead.
    table: Option<SearchableTable<ProfileRepo>>,
    layouts: LayoutCache,
}

impl Layout for ProfileBody {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let ProfileBody {
            user,
            bio,
            table,
            layouts,
        } = self;
        layouts.get(width, theme, || {
            profile_lines(user, bio.as_ref(), table.as_ref(), width, theme)
        })
    }
}

impl ProfileBody {
    /// The bio's links laid out at `width`, the body's text width: those below the header, the only other link.
    /// Textual's `Markdown` keeps `open_links` in the dialog, so a click on one opens it, while the header's link is a
    /// `Label`'s, which Textual doesn't follow.
    fn bio_links(&mut self, width: u16, theme: &Theme) -> Vec<LinkSpan> {
        let inner = content_width(width);
        if self.bio.is_none() || inner == 0 {
            return Vec::new();
        }
        let mut header = Lines::new(inner, theme);
        header.blank();
        header.header(&self.user);
        let below = header.out.lines.len();
        self.layout(width, theme)
            .links
            .iter()
            .filter(|span| span.row >= below)
            .cloned()
            .collect()
    }

    /// The cells `@login` covers in the layout `width` wide: the header's link, a terminal hyperlink to the profile,
    /// as Python's. The bio's links aren't, as Textual's `Markdown` follows those itself.
    fn login_links(&mut self, width: u16, theme: &Theme) -> Vec<LinkSpan> {
        let inner = content_width(width);
        if inner == 0 {
            return Vec::new();
        }
        let lines = Lines::new(inner, theme);
        // The header is the body's first linked text, so its spans lead the body's.
        let count = markdown::wrap_text(&lines.header_runs(&self.user), lines.width)
            .links
            .len();
        self.layout(width, theme).links.iter().take(count).cloned().collect()
    }

    /// The content lines the table takes at `width` (the body's text width), as laid out.
    fn table_rows(&mut self, width: u16, theme: &Theme) -> Option<std::ops::Range<usize>> {
        let height = usize::from(table_height(self.table.as_ref()?, content_width(width)));
        // The table is last, above the bottom padding row.
        let end = self.layout(width, theme).lines.len().checked_sub(1)?;
        Some(end.saturating_sub(height)..end)
    }
}

/// Where the last draw put the visible part of the table.
#[derive(Debug, Clone, Copy)]
struct TableView {
    /// The table's visible rows on screen.
    screen: Rect,
    /// The table's first visible row, counted from its header.
    first_row: u16,
    /// The whole table's size.
    size: (u16, u16),
}

/// A user's profile, their recent repos and a way to open one. Its output is the picked repo, or `None` for
/// `modal.close`.
pub struct UserProfileDialog {
    body: ProfileBody,
    scroll: ScrollView,
    ring: FocusRing<ProfileField>,
    /// The table's content lines at the last draw, to scroll it into view when it takes focus.
    last_table_rows: Option<std::ops::Range<usize>>,
    table_view: Option<TableView>,
    // `Some(None)` is a close with nothing picked, as `take_output` hands it on.
    #[allow(clippy::option_option)]
    output: Option<Option<Repository>>,
}

impl std::fmt::Debug for UserProfileDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserProfileDialog")
            .field("login", &self.body.user.user.login)
            .field("focused", &self.ring.focused())
            .finish_non_exhaustive()
    }
}

impl UserProfileDialog {
    /// The profile of `user`, with `repos` (in the API's order) in its table. Focus starts on the body.
    pub fn new(user: FullUser, repos: Vec<Repository>) -> Self {
        let bio = user.bio.as_deref().filter(|bio| !bio.is_empty()).map(Markdown::parse);
        let table = (!repos.is_empty()).then(|| {
            let columns = ["Owner", "Name", "Private", "Description"].map(Column::new).into();
            let mut table = SearchableTable::new(columns, SortSpec::ascending(0));
            let rows = repos
                .into_iter()
                .enumerate()
                .map(|(index, repo)| ProfileRepo { index, repo });
            // The table isn't lazy, so its first rows never ask for a page.
            let _ = table.insert_many(rows, &());
            table
        });
        let fields = if table.is_some() {
            vec![ProfileField::Body, ProfileField::Repos]
        } else {
            vec![ProfileField::Body]
        };
        Self {
            body: ProfileBody {
                user,
                bio,
                table,
                layouts: LayoutCache::default(),
            },
            scroll: ScrollView::default(),
            ring: FocusRing::new(fields),
            last_table_rows: None,
            table_view: None,
            output: None,
        }
    }

    fn action(action: UserProfileAction) -> Action {
        Action::Dialog(DialogAction::UserProfile(action))
    }

    /// What the table's answer to an input means: a picked row closes the dialog with its repo.
    fn table_outcome(&self, handled: Handled<TableEvent<String>>) -> Option<Action> {
        match handled {
            Handled::Ignored => None,
            Handled::Consumed | Handled::Event(TableEvent::LoadMore { .. }) => Some(Action::Consumed),
            Handled::Event(TableEvent::Copied(text)) => Some(Action::Copy(text)),
            Handled::Event(TableEvent::Selected(key)) => {
                let repo = self.body.table.as_ref()?.get(&key)?.repo.clone();
                Some(Self::action(UserProfileAction::Pick(Box::new(repo))))
            }
        }
    }

    /// The text lines of the body as last laid out, for tests.
    #[cfg(test)]
    fn lines(&mut self, width: u16) -> Vec<String> {
        let theme = Theme::default();
        self.body
            .layout(width, &theme)
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }
}

impl Component for UserProfileDialog {
    type Action = UserProfileAction;

    fn bindings(&self) -> &'static [BindingId] {
        PROFILE_BINDINGS
    }

    fn on_binding(&mut self, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        (id == BindingId::ModalClose).then(|| Self::action(UserProfileAction::Close))
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect, _cx: &Cx<'_>) -> Option<Action> {
        let at = (event.column, event.row).into();
        if let (Some(view), Some(table)) = (self.table_view, self.body.table.as_mut()) {
            // In the table's own coordinates, as if it were drawn whole at the origin.
            let scratch = Rect::new(0, 0, view.size.0, view.size.1);
            let translated = if view.screen.contains(at) {
                MouseEvent {
                    column: event.column - view.screen.x,
                    row: event.row - view.screen.y + view.first_row,
                    ..event
                }
            } else {
                // Off the table: a move still clears its hover.
                MouseEvent {
                    column: u16::MAX,
                    row: u16::MAX,
                    ..event
                }
            };
            let handled = table.handle_mouse(translated, scratch);
            if view.screen.contains(at) {
                if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                    self.ring.focus(ProfileField::Repos);
                }
                if let Some(action) = self.table_outcome(handled) {
                    return Some(action);
                }
            }
        }
        if !area.contains(at) {
            return None;
        }
        if let Some(wheel) = Wheel::from_mouse(&event) {
            return Some(Self::action(UserProfileAction::Wheel(wheel)));
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && self.table_view.is_none_or(|view| !view.screen.contains(at))
        {
            self.ring.focus(ProfileField::Body);
        }
        Some(Action::Consumed)
    }

    fn update(&mut self, action: UserProfileAction, _cx: &mut Cx<'_>) {
        match action {
            UserProfileAction::Scroll(scroll) => self.scroll.scroll(scroll),
            UserProfileAction::Wheel(wheel) => self.scroll.wheel(wheel),
            UserProfileAction::Close => self.output = Some(None),
            UserProfileAction::Pick(repo) => self.output = Some(Some(*repo)),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &RenderCx<'_>) {
        let theme = cx.theme;
        self.scroll.render(frame, area, theme, &mut self.body);
        if let Some(hyperlinks) = cx.hyperlinks {
            let login = self.body.login_links(self.scroll.text_width(area), theme);
            self.scroll.record_spans(area, &login, hyperlinks);
        }
        self.table_view = None;
        self.last_table_rows = None;

        let (content, window) = self.scroll.last_size();
        // The scroll view laid the body out beside its scrollbar when it overflowed.
        let width = if content > window {
            area.width.saturating_sub(crate::components::scrollbar::VERTICAL_WIDTH)
        } else {
            area.width
        };
        if let Some(links) = cx.links {
            let bio = self.body.bio_links(width, theme);
            links
                .borrow_mut()
                .record(Rect { width, ..area }, self.scroll.offset(), &bio);
        }
        let Some(rows) = self.body.table_rows(width, theme) else {
            return;
        };
        self.last_table_rows = Some(rows.clone());
        let Some(table) = self.body.table.as_mut() else {
            return;
        };
        let size = (
            u16::try_from(content_width(width)).unwrap_or(u16::MAX),
            u16::try_from(rows.len()).unwrap_or(u16::MAX),
        );
        let scratch_area = Rect::new(0, 0, size.0, size.1);
        let mut scratch = Buffer::empty(scratch_area);
        let focused = self.ring.is_focused(ProfileField::Repos);
        let table_cx = RenderCx {
            focused,
            focus_within: focused,
            ..*cx
        };
        table.render_table_into(&mut scratch, scratch_area, &table_cx);

        let offset = self.scroll.offset();
        let visible = rows.start.max(offset)..rows.end.min(offset + window);
        if visible.is_empty() {
            return;
        }
        // Content lines and table rows are bounded by the screen's u16 height once visible.
        let to_u16 = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        let screen = Rect {
            x: area.x + to_u16(PADDING_X),
            y: area.y + to_u16(visible.start - offset),
            width: size.0,
            height: to_u16(visible.len()),
        }
        .intersection(area);
        let first_row = to_u16(visible.start - rows.start);
        let buf = frame.buffer_mut();
        for dy in 0..screen.height {
            for dx in 0..screen.width {
                buf[(screen.x + dx, screen.y + dy)] = scratch[(dx, first_row + dy)].clone();
            }
        }
        self.table_view = Some(TableView {
            screen,
            first_row,
            size,
        });
    }
}

impl Dialog for UserProfileDialog {
    type Output = Option<Repository>;

    fn kind(&self) -> ModalKind {
        ModalKind::UserProfile
    }

    fn own_action(&self, action: DialogAction) -> Option<UserProfileAction> {
        match action {
            DialogAction::UserProfile(action) => Some(action),
            _ => None,
        }
    }

    fn move_focus(&mut self, direction: FocusMove) {
        self.ring.step(direction);
        if self.ring.is_focused(ProfileField::Repos)
            && let Some(rows) = self.last_table_rows.clone()
        {
            // The table with its `margin-top: 1` above and the container's bottom padding below, as Python scrolls.
            self.scroll.reveal(rows.start.saturating_sub(1)..rows.end + 1);
        }
    }

    fn size(&self, available: Rect) -> (u16, u16) {
        // `max-height: 90%`, rounded down as Textual does; the `1fr` height always reaches it.
        (WIDTH, available.height * 9 / 10)
    }

    fn background(&self, palette: &Palette) -> ThemeColor {
        palette.surface_lighten_1
    }

    fn take_output(&mut self) -> Option<Option<Repository>> {
        self.output.take()
    }

    fn field_chain(&self) -> Vec<ChainLink> {
        let field = match self.ring.focused() {
            Some(ProfileField::Body) => FieldKind::Scroll,
            Some(ProfileField::Repos) => FieldKind::Table,
            None => return Vec::new(),
        };
        vec![field.link(self.kind())]
    }

    fn on_field_binding(&mut self, field: FieldKind, id: BindingId, _cx: &Cx<'_>) -> Option<Action> {
        match field {
            FieldKind::Scroll => Scroll::from_binding(id).map(|scroll| Self::action(UserProfileAction::Scroll(scroll))),
            FieldKind::Table => {
                let handled = self.body.table.as_mut()?.on_binding(id);
                self.table_outcome(handled)
            }
            FieldKind::TextInput
            | FieldKind::TextEditor
            | FieldKind::Button
            | FieldKind::Switch
            | FieldKind::Select
            | FieldKind::SelectOverlay
            | FieldKind::SelectionList
            | FieldKind::Tabs
            | FieldKind::KeyCapture => None,
        }
    }
}

/// The columns inside the container's padding, for a body `width` wide.
fn content_width(width: u16) -> usize {
    usize::from(width).saturating_sub(2 * PADDING_X)
}

/// The table's height at `width`: header, rows and any horizontal scrollbar, up to [`MAX_TABLE_HEIGHT`].
fn table_height(table: &SearchableTable<ProfileRepo>, width: usize) -> u16 {
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    table.bare_table_rect(Rect::new(0, 0, width, MAX_TABLE_HEIGHT)).height
}

/// The body laid out `width` cells wide: the padding row, then each label with the blank row below it (its
/// `margin-bottom: 1`, which collapses with a rule's `margin: 1 0`), the rules, the bio, room for the table, and the
/// padding row. Every field is literal text (Py bug B49).
fn profile_lines(
    user: &FullUser,
    bio: Option<&Markdown>,
    table: Option<&SearchableTable<ProfileRepo>>,
    width: u16,
    theme: &Theme,
) -> Rendered {
    let inner = content_width(width);
    if inner == 0 {
        return Rendered::default();
    }
    let mut lines = Lines::new(inner, theme);
    // The container's top padding.
    lines.blank();
    lines.header(user);
    if let Some(bio) = bio {
        lines.push(bio.layout(lines.width, theme));
        // The markdown's last margin, then the rule's.
        lines.blank();
        lines.rule();
    }
    lines.info(user);
    lines.counts(user);
    lines.repos(user, table);
    // The container's bottom padding.
    lines.blank();
    lines.out
}

/// The body's lines as they're built, with the styles they're drawn in.
struct Lines {
    out: Rendered,
    /// The columns inside the padding.
    width: u16,
    text: Style,
    bold: Style,
    dim: Style,
    rule: Style,
}

impl Lines {
    fn new(width: usize, theme: &Theme) -> Self {
        let palette = theme.palette;
        let under = palette.surface_lighten_1.over(theme.background());
        let text = Style::new().fg(theme.color(palette.foreground, under));
        let dim = theme.rich_dim(text, palette.foreground, under);
        Self {
            out: Rendered::default(),
            width: u16::try_from(width).unwrap_or(u16::MAX),
            text,
            bold: text.add_modifier(Modifier::BOLD),
            dim,
            // `Rule { color: $secondary }`.
            rule: Style::new().fg(theme.color(palette.secondary, under)),
        }
    }

    fn blank(&mut self) {
        self.out.lines.push(Line::default());
    }

    /// Adds `more` below, indented by the container's padding, moving its links with it.
    fn push(&mut self, more: Rendered) {
        let shift = self.out.lines.len();
        let indent = u16::try_from(PADDING_X).unwrap_or(0);
        self.out.links.extend(more.links.into_iter().map(|mut span| {
            span.row += shift;
            span.columns = span.columns.start + indent..span.columns.end + indent;
            span
        }));
        self.out.lines.extend(more.lines.into_iter().map(|mut line| {
            if !line.spans.is_empty() {
                line.spans.insert(0, Span::raw(" ".repeat(PADDING_X)));
            }
            line
        }));
    }

    /// A `Label`: the runs wrapped to the width, then its bottom margin.
    fn label(&mut self, runs: &[TextRun]) {
        self.push(markdown::wrap_text(runs, self.width));
        self.blank();
    }

    /// A `Rule` across the width, then its bottom margin. Its top margin is the blank row above it.
    fn rule(&mut self) {
        let line = Line::from(Span::styled("─".repeat(usize::from(self.width)), self.rule));
        self.push(Rendered {
            lines: vec![line],
            links: Vec::new(),
        });
        self.blank();
    }

    /// "<name> (@login) — <type>", and a rule.
    fn header(&mut self, user: &FullUser) {
        self.label(&self.header_runs(user));
        self.rule();
    }

    /// The header's text, `@login` linking to the profile.
    fn header_runs(&self, user: &FullUser) -> [TextRun; 4] {
        let login = &user.user.login;
        let name = user
            .user
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(login);
        let kind = user
            .kind
            .as_deref()
            .filter(|kind| !kind.is_empty())
            .unwrap_or("User")
            .to_lowercase();
        [
            TextRun::new(name, self.bold),
            TextRun::new(" (", self.text),
            TextRun::link(format!("@{login}"), self.text, user.user.html_url.clone()),
            TextRun::new(format!(") — {kind}"), self.text),
        ]
    }

    /// Company, Location, Email, Blog and Twitter, each when set and not empty, and a rule when any is.
    fn info(&mut self, user: &FullUser) {
        let twitter = user
            .twitter_username
            .as_deref()
            .filter(|handle| !handle.is_empty())
            .map(|handle| format!("@{handle}"));
        let fields = [
            ("Company", user.company.clone()),
            ("Location", user.location.clone()),
            ("Email", user.email.clone()),
            ("Blog", user.blog.clone()),
            ("Twitter", twitter),
        ];
        let mut any = false;
        for (name, value) in fields {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                continue;
            };
            any = true;
            self.label(&[
                TextRun::new(format!("{name}:"), self.bold),
                TextRun::new(format!(" {value}"), self.text),
            ]);
        }
        if any {
            self.rule();
        }
    }

    /// The four counts on one line, four spaces apart, then the join date, and a rule.
    fn counts(&mut self, user: &FullUser) {
        let counts = [
            ("Public repos:", user.public_repos),
            ("Gists:", user.public_gists),
            ("Followers:", user.followers),
            ("Following:", user.following),
        ];
        let mut runs = Vec::new();
        for (i, (name, count)) in counts.into_iter().enumerate() {
            let gap = if i + 1 < counts.len() { "    " } else { "" };
            runs.push(TextRun::new(name, self.bold));
            runs.push(TextRun::new(format!(" {count}{gap}"), self.text));
        }
        self.label(&runs);
        let joined = user
            .created_at
            .map_or_else(|| "unknown".to_owned(), |at| at.strftime("%Y-%m-%d").to_string());
        self.label(&[
            TextRun::new("Joined:", self.bold),
            TextRun::new(format!(" {joined}"), self.text),
        ]);
        self.rule();
    }

    /// The "Recent repositories" heading, and blank rows the table is drawn over, or "No public repositories.".
    fn repos(&mut self, user: &FullUser, table: Option<&SearchableTable<ProfileRepo>>) {
        let Some(table) = table else {
            self.label(&[TextRun::new("Recent repositories", self.bold)]);
            self.label(&[TextRun::new("No public repositories.", self.dim)]);
            return;
        };
        let heading = format!(
            "(showing {} of {}, press enter to open)",
            table.row_count(),
            user.public_repos
        );
        // The heading's margin is the table's `margin-top: 1`.
        self.label(&[
            TextRun::new("Recent repositories", self.bold),
            TextRun::new(" ", self.text),
            TextRun::new(heading, self.dim),
        ]);
        let height = usize::from(table_height(table, usize::from(self.width)));
        self.out.lines.extend(std::iter::repeat_n(Line::default(), height));
    }
}

#[cfg(test)]
mod tests;
