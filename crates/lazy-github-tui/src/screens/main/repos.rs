//! The repositories section: the user's repos in a searchable table, favorites first. Python's `ReposContainer`.

use lazy_github_core::gh::GhError;
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

/// The repos section's state: the table and the load in flight.
pub struct ReposSection {
    pub(super) table: SearchableTable<Repository>,
    load: Load,
}

/// Where loading the rows is.
#[derive(Debug, Default)]
enum Load {
    /// Nothing in flight.
    #[default]
    Idle,
    /// Waiting for `/user/repos`.
    Listing(RequestId),
    /// The list is in; waiting for the tracked repos.
    Tracking(TrackedFetch),
}

/// The listed repos, held until every tracked repo has answered.
#[derive(Debug)]
struct TrackedFetch {
    listed: Vec<Repository>,
    /// One per name in `repositories.additional_repos_to_track`, in config order.
    slots: Vec<TrackedSlot>,
}

#[derive(Debug)]
struct TrackedSlot {
    name: String,
    id: RequestId,
    answer: TrackedAnswer,
}

/// What one tracked name came back with.
#[derive(Debug)]
enum TrackedAnswer {
    Waiting,
    Found(Box<Repository>),
    /// Not found, or the request failed: left out.
    Missing,
}

impl TrackedFetch {
    /// Whether `id` is a tracked request that hasn't answered yet.
    fn waits_for(&self, id: RequestId) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.id == id && matches!(slot.answer, TrackedAnswer::Waiting))
    }

    /// Fills `id`'s slot. A failure, or a repo that isn't there, is logged at debug and left out.
    fn fill(&mut self, id: RequestId, result: Result<Option<Box<Repository>>, GhError>) {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) else {
            return;
        };
        slot.answer = match result {
            Ok(Some(repo)) => TrackedAnswer::Found(repo),
            Ok(None) => {
                tracing::debug!("Tracked repo {} not found", slot.name);
                TrackedAnswer::Missing
            }
            Err(error) => {
                tracing::debug!("Fetching tracked repo {} failed: {error}", slot.name);
                TrackedAnswer::Missing
            }
        };
    }

    fn is_complete(&self) -> bool {
        !self
            .slots
            .iter()
            .any(|slot| matches!(slot.answer, TrackedAnswer::Waiting))
    }

    /// The listed repos in API order, then the found tracked repos in config order.
    fn into_rows(self) -> impl Iterator<Item = Repository> {
        let found = self.slots.into_iter().filter_map(|slot| match slot.answer {
            TrackedAnswer::Found(repo) => Some(*repo),
            TrackedAnswer::Waiting | TrackedAnswer::Missing => None,
        });
        self.listed.into_iter().chain(found)
    }
}

impl std::fmt::Debug for ReposSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReposSection")
            .field("rows", &self.table.row_count())
            .field("loading", &self.table.is_loading())
            .field("load", &self.load)
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
            load: Load::Idle,
        }
    }

    /// Python's `on_mount`: the loading overlay, and the request for every repo.
    pub fn start(&mut self, cx: &mut Cx<'_>) {
        self.table.set_loading(true);
        self.load = Load::Listing(cx.request(GithubRequest::ListRepositories(RepoListQuery::all())));
    }

    /// Takes the responses to the pending list and tracked-repo requests, and hands back any other.
    ///
    /// Python's `load_repos`: once every page has arrived, each of `repositories.additional_repos_to_track` is
    /// fetched at once, and when all have answered the listed repos, then the found tracked ones, replace whatever
    /// was there. A failed list is logged, with no toast, fetches nothing tracked and leaves the table empty.
    pub fn on_response(&mut self, id: RequestId, response: GithubResponse, cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match &mut self.load {
            Load::Listing(pending) if *pending == id => {
                let GithubResponse::ListRepositories(result) = response else {
                    return Some(response);
                };
                self.load = Load::Idle;
                match result {
                    Ok(repos) => self.fetch_tracked(repos, cx),
                    Err(error) => {
                        tracing::error!("Error fetching repositories from Github API");
                        tracing::debug!("Fetching repositories failed: {error}");
                        self.table.set_loading(false);
                    }
                }
                None
            }
            Load::Tracking(fetch) if fetch.waits_for(id) => {
                let GithubResponse::Repository(result) = response else {
                    return Some(response);
                };
                fetch.fill(id, result);
                if fetch.is_complete()
                    && let Load::Tracking(fetch) = std::mem::take(&mut self.load)
                {
                    self.show(fetch.into_rows(), cx);
                }
                None
            }
            _ => Some(response),
        }
    }

    /// Asks for every tracked repo, as the config has them now, keeping `listed` and the overlay until they answer.
    /// With nothing tracked, `listed` is shown at once.
    fn fetch_tracked(&mut self, listed: Vec<Repository>, cx: &mut Cx<'_>) {
        let names = cx.config().repositories.additional_repos_to_track.clone();
        if names.is_empty() {
            self.show(listed, cx);
            return;
        }
        let slots = names
            .into_iter()
            .map(|name| TrackedSlot {
                id: cx.request(GithubRequest::Repository(name.clone())),
                name,
                answer: TrackedAnswer::Waiting,
            })
            .collect();
        self.load = Load::Tracking(TrackedFetch { listed, slots });
    }

    /// Replaces the rows with `repos`, drops the overlay, and offers the current-directory repo the new rows.
    fn show(&mut self, repos: impl IntoIterator<Item = Repository>, cx: &mut Cx<'_>) {
        self.table.replace_all(repos, &RepoCells::from_cx(cx));
        self.table.set_loading(false);
        self.check_current_directory_repo(cx);
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

    /// Python's `action_toggle_favorite_repo`: adds the repo under the cursor to `repositories.favorites`, or
    /// removes it, then re-sorts. The cursor keeps its row index, so the repo under it can change. Rows a search
    /// hides get their stars from the config too. Nothing happens on an empty table (Py bug B13).
    pub fn toggle_favorite(&mut self, cx: &mut Cx<'_>) {
        let Some(name) = self.table.selected().map(|repo| repo.full_name.clone()) else {
            return;
        };
        let favorites = &mut cx.config_mut().repositories.favorites;
        if favorites.contains(&name) {
            tracing::info!("Unfavoriting repo {name}");
            favorites.retain(|favorite| *favorite != name);
        } else {
            tracing::info!("Favoriting repo {name}");
            favorites.push(name);
        }
        self.table.refresh_rows(&RepoCells::from_cx(cx));
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
    fn replacing_with_a_duplicate_keeps_the_last_and_moves_it_to_the_end_of_its_tie_group() {
        let mut section = ReposSection::new(&Theme::default(), AppOptions::default());
        let mut again = repo("octo", "a", true);
        again.description = Some("again".into());
        let repos = [
            repo("octo", "a", false),
            repo("octo", "b", false),
            repo("octo", "c", false),
            again.clone(),
        ];
        section.table.replace_all(repos, &cells(&["octo/a", "octo/c"]));
        let keys: Vec<&str> = section.table.row_keys().map(String::as_str).collect();
        assert_eq!(keys, ["octo/c", "octo/a", "octo/b"]);
        assert_eq!(section.table.get(&"octo/a".to_owned()), Some(&again));
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

/// The section in the app: loading the list and the tracked repos, and toggling favorites.
#[cfg(test)]
mod app_tests {
    use lazy_github_core::config::Config;
    use lazy_github_core::github::{FakeGithub, GithubCall};
    use lazy_github_core::models::RepoName;

    use super::*;
    use crate::effect::Effect;
    use crate::event::Event;
    use crate::test_util::{TestApp, logs};

    /// `base`'s three repos, in the API's order.
    fn base_repos() -> Vec<Repository> {
        ["dotfiles", "hello-world", "lazy-demo"]
            .map(|name| repo("parity-bot", name, false))
            .into()
    }

    fn tracking(names: &[&str]) -> Config {
        let mut config = Config::default();
        config.repositories.additional_repos_to_track = names.iter().map(|&name| name.to_owned()).collect();
        config
    }

    fn server_error() -> GhError {
        GhError::Http {
            status: 500,
            message: "Server Error".into(),
            body: Vec::new(),
        }
    }

    fn section(app: &TestApp) -> &ReposSection {
        app.app().main_screen().unwrap().repos()
    }

    fn row_keys(app: &TestApp) -> Vec<String> {
        section(app).table().row_keys().cloned().collect()
    }

    /// Answers every queued request, and returns what that logged.
    fn settle(app: &mut TestApp) -> String {
        logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
    }

    /// The tracked-repo requests among the app's effects, in order.
    fn tracked_requests(app: &TestApp) -> Vec<(RequestId, String)> {
        app.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Github {
                    id,
                    request: GithubRequest::Repository(name),
                } => Some((*id, name.clone())),
                _ => None,
            })
            .collect()
    }

    fn repository_calls(app: &TestApp) -> Vec<String> {
        app.gh()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                GithubCall::Repository(name) => Some(name),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_tracked_makes_no_extra_request() {
        let mut app = TestApp::with_github(Config::default(), FakeGithub::new().with_repositories(base_repos()));
        settle(&mut app);
        assert_eq!(tracked_requests(&app), []);
        assert_eq!(repository_calls(&app), Vec::<String>::new());
        assert_eq!(
            row_keys(&app),
            ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
        );
    }

    #[test]
    fn tracked_repos_come_after_the_list_in_config_order() {
        let gh = FakeGithub::new()
            .with_repositories(base_repos())
            .with_repository("octo/b", Ok(Some(repo("octo", "b", false))))
            .with_repository("octo/a", Ok(Some(repo("octo", "a", true))));
        let mut app = TestApp::with_github(tracking(&["octo/b", "octo/a"]), gh);
        // Asked for at once, in config order.
        let requested: Vec<String> = tracked_requests(&app).into_iter().map(|(_, name)| name).collect();
        assert_eq!(requested, ["octo/b", "octo/a"]);
        let out = settle(&mut app);
        assert_eq!(out, "");
        assert_eq!(
            row_keys(&app),
            [
                "parity-bot/dotfiles",
                "parity-bot/hello-world",
                "parity-bot/lazy-demo",
                "octo/b",
                "octo/a"
            ]
        );
        assert!(!section(&app).table().is_loading());
    }

    #[test]
    fn a_tracked_repo_already_listed_replaces_its_row_at_the_end_of_its_tie_group() {
        let mut tracked = repo("parity-bot", "dotfiles", false);
        tracked.description = Some("the tracked answer".into());
        let gh = FakeGithub::new()
            .with_repositories(base_repos())
            .with_repository("parity-bot/dotfiles", Ok(Some(tracked.clone())));
        let mut config = tracking(&["parity-bot/dotfiles"]);
        config.repositories.favorites = vec!["parity-bot/dotfiles".into(), "parity-bot/lazy-demo".into()];
        let mut app = TestApp::with_github(config, gh);
        settle(&mut app);
        // Still a favorite, but after the favorite that was added before it.
        assert_eq!(
            row_keys(&app),
            ["parity-bot/lazy-demo", "parity-bot/dotfiles", "parity-bot/hello-world"]
        );
        let table = section(&app).table();
        assert_eq!(table.get(&"parity-bot/dotfiles".to_owned()), Some(&tracked));
        assert_eq!(table.row_count(), 3);
    }

    #[test]
    fn a_failed_or_missing_tracked_repo_is_left_out_quietly() {
        let gh = FakeGithub::new()
            .with_repositories(base_repos())
            .with_repository("octo/broken", Err(server_error()))
            .with_repository("octo/a", Ok(Some(repo("octo", "a", false))));
        // `octo/gone` isn't set, so the fake finds nothing, as a 404 does.
        let mut app = TestApp::with_github(tracking(&["octo/gone", "octo/broken", "octo/a"]), gh);
        let out = settle(&mut app);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out}");
        assert_eq!(lines[0], "DEBUG Tracked repo octo/gone not found");
        assert!(
            lines[1].starts_with("DEBUG Fetching tracked repo octo/broken failed: "),
            "{out}"
        );
        assert!(app.app().toasts().is_empty());
        assert_eq!(
            row_keys(&app),
            [
                "parity-bot/dotfiles",
                "parity-bot/hello-world",
                "parity-bot/lazy-demo",
                "octo/a"
            ]
        );
    }

    #[test]
    fn a_failed_list_fetches_nothing_tracked() {
        let gh = FakeGithub::new()
            .with_repositories_error(server_error())
            .with_repository("octo/a", Ok(Some(repo("octo", "a", false))));
        let mut app = TestApp::with_github(tracking(&["octo/a"]), gh);
        settle(&mut app);
        assert_eq!(tracked_requests(&app), []);
        assert_eq!(repository_calls(&app), Vec::<String>::new());
        assert_eq!(section(&app).table().row_count(), 0);
        assert!(!section(&app).table().is_loading());
    }

    #[test]
    fn the_overlay_stays_until_the_last_answer() {
        let mut app = TestApp::with_github(
            tracking(&["octo/a", "octo/b"]),
            FakeGithub::new().with_repositories(base_repos()),
        );
        let requests = tracked_requests(&app);
        assert_eq!(requests.len(), 2);
        // The list is in, but held.
        assert!(section(&app).table().is_loading());
        assert_eq!(section(&app).table().row_count(), 0);

        // Answers may come in any order.
        app.send(Event::Github {
            id: requests[1].0,
            response: GithubResponse::Repository(Ok(Some(Box::new(repo("octo", "b", false))))),
        });
        assert!(section(&app).table().is_loading());
        assert_eq!(section(&app).table().row_count(), 0);

        // A second answer to the same request isn't the section's.
        let out = logs(|| {
            app.send(Event::Github {
                id: requests[1].0,
                response: GithubResponse::Repository(Ok(None)),
            });
        });
        assert!(out.contains("Dropping the response"), "{out}");
        assert!(section(&app).table().is_loading());

        app.send(Event::Github {
            id: requests[0].0,
            response: GithubResponse::Repository(Ok(Some(Box::new(repo("octo", "a", false))))),
        });
        assert!(!section(&app).table().is_loading());
        // Config order, whichever answered first.
        let keys = row_keys(&app);
        assert_eq!(keys[3..], ["octo/a", "octo/b"]);
    }

    #[test]
    fn the_current_directory_check_sees_tracked_repos() {
        let gh = FakeGithub::new()
            .with_repositories(base_repos())
            .with_repository("octo/a", Ok(Some(repo("octo", "a", false))));
        let mut app = TestApp::loading(tracking(&["octo/a"]), gh)
            .with_local_repo(RepoName::from_remote_url("git@github.com:octo/a.git"));
        // Git answers after the list but before the tracked repo, so the first check finds nothing.
        let out = settle(&mut app);
        assert!(out.contains("INFO Selected repo octo/a"), "{out}");
        assert_eq!(
            app.app().current_repo().map(|repo| repo.full_name.as_str()),
            Some("octo/a")
        );
        // The cursor stays on the first row.
        assert_eq!(section(&app).table().cursor(), 0);
    }

    #[test]
    fn repos_tracked_favorite_100x30() {
        let gh = FakeGithub::new().with_repositories(base_repos()).with_repository(
            "parity-org/tracked-tool",
            Ok(Some(repo("parity-org", "tracked-tool", false))),
        );
        let mut config = tracking(&["parity-org/tracked-tool"]);
        config.repositories.favorites = vec!["parity-org/tracked-tool".into()];
        let mut app = TestApp::with_github(config, gh);
        settle(&mut app);
        let screen = app.screen(100, 30);
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[5].starts_with("│ ★  parity-org  tracked-tool"), "{}", rows[5]);
        insta::assert_snapshot!(screen);
    }

    /// The favorites in each config save among the app's effects, oldest first.
    fn saved_favorites(app: &TestApp) -> Vec<Vec<String>> {
        app.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::SaveConfig(config) => Some(config.repositories.favorites.clone()),
                _ => None,
            })
            .collect()
    }

    /// Presses `keys`, and returns what that logged.
    fn press_logged(app: &mut TestApp, keys: &[&str]) -> String {
        logs(|| {
            app.press(keys);
        })
    }

    fn stars(app: &mut TestApp) -> Vec<String> {
        app.screen(100, 30)
            .lines()
            .skip(5)
            .take(3)
            .map(|line| line.chars().skip(2).take(30).collect::<String>().trim_end().to_owned())
            .collect()
    }

    #[test]
    fn toggling_favorites_saves_logs_and_resorts_as_the_scenario_does() {
        let mut app = TestApp::with_github(Config::default(), FakeGithub::new().with_repositories(base_repos()));
        let favorites = |app: &mut TestApp| app.with_cx(|cx| cx.config().repositories.favorites.clone());

        // first: the first row, which stays where it is.
        let out = press_logged(&mut app, &["ctrl+f"]);
        assert_eq!(out, "INFO Favoriting repo parity-bot/dotfiles\n");
        assert_eq!(favorites(&mut app), ["parity-bot/dotfiles"]);
        assert_eq!(saved_favorites(&app), [vec!["parity-bot/dotfiles".to_owned()]]);
        assert_eq!(
            row_keys(&app),
            ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
        );
        assert_eq!(section(&app).table().cursor(), 0);

        // second: the third row moves up; the cursor stays on the third row, now hello-world.
        let out = press_logged(&mut app, &["j", "j", "ctrl+f"]);
        assert_eq!(out, "INFO Favoriting repo parity-bot/lazy-demo\n");
        assert_eq!(favorites(&mut app), ["parity-bot/dotfiles", "parity-bot/lazy-demo"]);
        assert_eq!(saved_favorites(&app).len(), 2);
        assert_eq!(
            row_keys(&app),
            ["parity-bot/dotfiles", "parity-bot/lazy-demo", "parity-bot/hello-world"]
        );
        assert_eq!(section(&app).table().cursor(), 2);

        // unfavorite: the first row drops to the top of the rest.
        let out = press_logged(&mut app, &["k", "k", "ctrl+f"]);
        assert_eq!(out, "INFO Unfavoriting repo parity-bot/dotfiles\n");
        assert_eq!(favorites(&mut app), ["parity-bot/lazy-demo"]);
        assert_eq!(
            row_keys(&app),
            ["parity-bot/lazy-demo", "parity-bot/dotfiles", "parity-bot/hello-world"]
        );
        assert_eq!(section(&app).table().cursor(), 0);

        // none-left: back to the order the rows were added, not the order shown.
        let out = press_logged(&mut app, &["ctrl+f"]);
        assert_eq!(out, "INFO Unfavoriting repo parity-bot/lazy-demo\n");
        assert_eq!(favorites(&mut app), Vec::<String>::new());
        assert_eq!(
            row_keys(&app),
            ["parity-bot/dotfiles", "parity-bot/hello-world", "parity-bot/lazy-demo"]
        );
        assert_eq!(section(&app).table().cursor(), 0);

        // One save per toggle, each with the favorites as they were then.
        assert_eq!(
            saved_favorites(&app),
            [
                vec!["parity-bot/dotfiles".to_owned()],
                vec!["parity-bot/dotfiles".to_owned(), "parity-bot/lazy-demo".to_owned()],
                vec!["parity-bot/lazy-demo".to_owned()],
                vec![],
            ]
        );
        assert!(app.app().toasts().is_empty());
    }

    #[test]
    fn toggling_on_an_empty_table_does_nothing() {
        let mut app = TestApp::new(Config::default());
        let out = press_logged(&mut app, &["ctrl+f"]);
        assert_eq!(out, "");
        assert_eq!(saved_favorites(&app), Vec::<Vec<String>>::new());
        assert!(app.app().toasts().is_empty());
    }

    #[test]
    fn with_a_search_filter_hidden_rows_take_their_stars_from_the_config() {
        let mut config = Config::default();
        config.repositories.favorites = vec!["parity-bot/lazy-demo".into()];
        let mut app = TestApp::with_github(config, FakeGithub::new().with_repositories(base_repos()));
        app.press(&["/", "d", "o", "t", "enter"]);
        assert_eq!(row_keys(&app), ["parity-bot/dotfiles"]);
        let out = press_logged(&mut app, &["ctrl+f"]);
        assert_eq!(out, "INFO Favoriting repo parity-bot/dotfiles\n");
        assert_eq!(row_keys(&app), ["parity-bot/dotfiles"]);
        // Clearing the search shows every row, starred from the config, favorites first in the order added.
        app.press(&["/", "ctrl+u", "enter"]);
        assert_eq!(
            row_keys(&app),
            ["parity-bot/dotfiles", "parity-bot/lazy-demo", "parity-bot/hello-world"]
        );
        assert_eq!(
            stars(&mut app),
            [
                "★  parity-bot  dotfiles     ✘",
                "★  parity-bot  lazy-demo    ✘",
                "☆  parity-bot  hello-world  ✘"
            ]
        );
    }

    #[test]
    fn ctrl_f_in_the_search_box_edits_the_box() {
        let mut app = TestApp::with_github(Config::default(), FakeGithub::new().with_repositories(base_repos()));
        app.press(&["/", "a", "b", "home"]);
        let out = press_logged(&mut app, &["ctrl+f"]);
        assert_eq!(out, "");
        assert_eq!(section(&app).table().search_text(), "");
        assert_eq!(saved_favorites(&app), Vec::<Vec<String>>::new());
    }

    #[test]
    fn repos_favorite_toggled_100x30() {
        let mut app = TestApp::with_github(Config::default(), FakeGithub::new().with_repositories(base_repos()));
        app.press(&["j", "j", "ctrl+f", "j"]);
        let buffer = app.render(100, 30);
        // The star is Rich's green, now that the cursor is off its row.
        assert_eq!(buffer[(2, 5)].symbol(), "★");
        assert_eq!(buffer[(2, 5)].fg, Color::Rgb(0x98, 0xe0, 0x24));
        let screen = crate::test_util::buffer_to_string(&buffer);
        insta::assert_snapshot!(screen);
    }
}
