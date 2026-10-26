//! The repositories section: the user's repos in a searchable table, favorites first. Python's `ReposContainer`.

use lazy_github_core::github::RepoListQuery;
use lazy_github_core::models::Repository;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::action::Action;
use crate::component::{Cx, LocalRepo};
use crate::components::table::{Cell, CellValue, Column, SearchableTable, SortSpec, TableItem};
use crate::effect::{GithubRequest, GithubResponse, RequestId};
use crate::options::AppOptions;
use crate::theme::{RichAnsi, Theme};

/// The favorite cell's raw value, which the rows sort by: Python's `IS_FAVORITED` markup. `[` sorts before `☆`, so
/// favorites come first.
const FAVORITE_VALUE: &str = "[green]★[/]";
/// The star drawn for a favorite, and as the column's header, in Rich's green.
const FAVORITE: &str = "★";
/// Python's `IS_NOT_FAVORITED`.
const NOT_FAVORITE: &str = "☆";
/// Python's `private_string(True)`.
const PRIVATE: &str = "✓";
/// Python's `private_string(False)`.
const PUBLIC: &str = "✘";

/// What a repo's cells need besides the repo.
#[derive(Debug, Clone)]
pub struct RepoCells {
    /// `repositories.favorites`: full names.
    pub favorites: Vec<String>,
    /// The favorite star's color.
    pub star: Color,
}

impl RepoCells {
    /// The favorites from `cx`'s config and the star color from its theme.
    fn from_cx(cx: &Cx<'_>) -> Self {
        Self {
            favorites: cx.config().repositories.favorites.clone(),
            star: star_color(cx.theme()),
        }
    }
}

fn star_color(theme: &Theme) -> Color {
    theme.rich_ansi(RichAnsi::Green)
}

impl TableItem for Repository {
    type Key = String;
    type Ctx = RepoCells;

    fn key(&self) -> String {
        self.full_name.clone()
    }

    /// Python's `repo_to_row`.
    fn cells(&self, cx: &RepoCells) -> Vec<Cell> {
        let favorite = if cx.favorites.contains(&self.full_name) {
            Cell::new(
                CellValue::Str(FAVORITE_VALUE.into()),
                Span::styled(FAVORITE, Style::new().fg(cx.star)),
            )
        } else {
            Cell::text(NOT_FAVORITE)
        };
        vec![
            favorite,
            Cell::text(self.owner.login.clone()),
            Cell::text(self.name.clone()),
            Cell::text(if self.private { PRIVATE } else { PUBLIC }),
        ]
    }
}

/// The repos section's state: the table and the list request in flight.
pub struct ReposSection {
    pub(super) table: SearchableTable<Repository>,
    pending: Option<RequestId>,
}

impl std::fmt::Debug for ReposSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReposSection")
            .field("rows", &self.table.row_count())
            .field("loading", &self.table.is_loading())
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl ReposSection {
    /// An empty table with Python's columns, sorted by the favorite column.
    pub fn new(theme: &Theme, options: AppOptions) -> Self {
        let columns = vec![
            Column::new(Span::styled(FAVORITE, Style::new().fg(star_color(theme)))),
            Column::new("Owner"),
            Column::new("Name"),
            Column::new("Private"),
        ];
        Self {
            table: SearchableTable::new(columns, SortSpec::ascending(0)).with_options(options),
            pending: None,
        }
    }

    /// Python's `on_mount`: the loading overlay, and the request for every repo.
    pub fn start(&mut self, cx: &mut Cx<'_>) {
        self.table.set_loading(true);
        self.pending = Some(cx.request(GithubRequest::ListRepositories(RepoListQuery::all())));
    }

    /// Takes the response to the pending list request, and hands back any other.
    ///
    /// Python's `load_repos`: the rows replace whatever was there once every page has arrived. A failure is logged,
    /// with no toast, and leaves the table empty.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        if self.pending != Some(id) {
            return Some(response);
        }
        let GithubResponse::ListRepositories(result) = response else {
            return Some(response);
        };
        self.pending = None;
        match result {
            Ok(repos) => {
                self.table.replace_all(repos, &RepoCells::from_cx(cx));
                self.check_current_directory_repo(cx);
            }
            Err(error) => {
                tracing::error!("Error fetching repositories from Github API");
                tracing::debug!("Fetching repositories failed: {error}");
            }
        }
        self.table.set_loading(false);
        None
    }

    /// Python's `check_current_directory_repo`: when nothing is selected yet and the current-directory repo is in
    /// the table, selects it as `enter` would, without moving the cursor. Called when the rows land and when git
    /// answers, so it works whichever comes first. The match ignores ASCII case (Py bug B42).
    pub fn check_current_directory_repo(&self, cx: &mut Cx<'_>) {
        let LocalRepo::Known(Some(name)) = cx.local_repo() else {
            return;
        };
        if cx.current_repo().is_some() {
            return;
        }
        let Some(repo) = self.table.items().find(|repo| name.matches(&repo.full_name)) else {
            return;
        };
        let action = Action::SelectRepo(Box::new(repo.clone()));
        cx.dispatch(action);
    }

    /// The table, for reading its rows and cursor.
    pub fn table(&self) -> &SearchableTable<Repository> {
        &self.table
    }
}

/// A repo for tests: public unless `private`, owned by the user `owner`.
#[cfg(test)]
pub(super) fn repo(owner: &str, name: &str, private: bool) -> Repository {
    Repository {
        name: name.into(),
        full_name: format!("{owner}/{name}"),
        default_branch: Some("main".into()),
        private,
        archived: Some(false),
        owner: lazy_github_core::models::User {
            login: owner.into(),
            id: 1,
            name: None,
            html_url: format!("https://github.com/{owner}"),
        },
        description: None,
        permissions: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: Color = Color::Rgb(0x98, 0xe0, 0x24);

    fn cells(favorites: &[&str]) -> RepoCells {
        RepoCells {
            favorites: favorites.iter().map(|&f| f.to_owned()).collect(),
            star: GREEN,
        }
    }

    #[test]
    fn cells_for_favorite_and_private_repos() {
        let cx = cells(&["octo/secret"]);
        let favorite_private = repo("octo", "secret", true).cells(&cx);
        assert_eq!(
            favorite_private,
            [
                Cell::new(
                    CellValue::Str("[green]★[/]".into()),
                    Span::styled("★", Style::new().fg(GREEN))
                ),
                Cell::text("octo"),
                Cell::text("secret"),
                Cell::text("✓"),
            ]
        );
        let plain_public = repo("octo", "open", false).cells(&cx);
        assert_eq!(
            plain_public,
            [Cell::text("☆"), Cell::text("octo"), Cell::text("open"), Cell::text("✘")]
        );
        assert_eq!(repo("octo", "open", false).key(), "octo/open");
    }

    #[test]
    fn favorites_come_first_and_ties_keep_api_order() {
        let mut section = ReposSection::new(&Theme::default(), AppOptions::default());
        let repos = ["a", "b", "c", "d", "e"].map(|name| repo("octo", name, false));
        section.table.replace_all(repos, &cells(&["octo/b", "octo/d"]));
        let keys: Vec<&str> = section.table.row_keys().map(String::as_str).collect();
        assert_eq!(keys, ["octo/b", "octo/d", "octo/a", "octo/c", "octo/e"]);
    }
}
