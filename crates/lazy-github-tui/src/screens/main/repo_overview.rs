//! The Repo Overview tab's body: the repo's header lines, a rule, and its README, scrolling as one. Python's
//! `RepoOverviewTabPane`.

use std::sync::LazyLock;

use lazy_github_core::gh::GhError;
use lazy_github_core::models::Repository;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::component::Cx;
use crate::components::command_log::Wheel;
use crate::components::info::Scroll;
use crate::components::markdown::{Markdown, Rendered, TextRun};
use crate::components::scroll_view::{Layout, LayoutCache, ScrollView};
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::theme::{Accent, Theme};

use super::details::DetailsBody;
use super::overview_text::{HeaderStyles, accent, append, push_label, push_rule};

/// What shows below the rule when there's no README, or it couldn't be fetched: Python's `*No README found.*`.
static NO_README: LazyLock<Markdown> = LazyLock::new(|| Markdown::parse("*No README found.*"));

/// The overview of one repo, and its scroll position.
#[derive(Debug)]
pub(super) struct RepoOverview {
    scroll: ScrollView,
    body: Body,
}

/// Everything that scrolls, with its layouts.
#[derive(Debug)]
struct Body {
    repo: Repository,
    readme: Readme,
    layouts: LayoutCache,
}

/// Where the README is.
#[derive(Debug)]
enum Readme {
    /// Requested; nothing shows below the rule yet.
    Loading(RequestId),
    /// Fetched and parsed.
    Loaded(Markdown),
    /// None, empty, or the request failed.
    Missing,
}

impl RepoOverview {
    /// The overview of `repo`, scrolled to the top, with its README requested.
    pub(super) fn new(repo: &Repository, cx: &mut Cx<'_>) -> Self {
        let id = cx.request(GithubRequest::RepoReadme(repo.full_name.clone()));
        Self {
            scroll: ScrollView::default(),
            body: Body {
                repo: repo.clone(),
                readme: Readme::Loading(id),
                layouts: LayoutCache::default(),
            },
        }
    }

    /// The repo shown.
    pub(super) fn repo(&self) -> &Repository {
        &self.body.repo
    }

    /// The README request still running, if any.
    pub(super) fn pending(&self) -> Option<RequestId> {
        match self.body.readme {
            Readme::Loading(id) => Some(id),
            Readme::Loaded(_) | Readme::Missing => None,
        }
    }

    /// The overview is going away: cancels its README request if it's still running.
    pub(super) fn close(&self, cx: &mut Cx<'_>) {
        if let Some(id) = self.pending() {
            cx.effect(Effect::Cancel(id));
        }
    }

    /// The README request finished. A failure is only logged: it shows as no README, as in Python.
    pub(super) fn on_readme(&mut self, result: Result<Option<String>, GhError>) {
        self.body.readme = match result {
            Ok(Some(text)) if !text.is_empty() => Readme::Loaded(Markdown::parse(&text)),
            Ok(_) => Readme::Missing,
            Err(error) => {
                tracing::debug!("Fetching the README of {} failed: {error}", self.body.repo.full_name);
                Readme::Missing
            }
        };
        self.body.layouts.clear();
    }

    /// The first visible line.
    #[cfg(test)]
    pub(super) fn offset(&self) -> usize {
        self.scroll.offset()
    }

    pub(super) fn scroll(&mut self, scroll: Scroll) {
        self.scroll.scroll(scroll);
    }

    pub(super) fn wheel(&mut self, wheel: Wheel) {
        self.scroll.wheel(wheel);
    }

    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.scroll.render(frame, area, theme, &mut self.body);
    }
}

impl DetailsBody for RepoOverview {
    fn tab_names(&self) -> &'static [&'static str] {
        &["Repo Overview"]
    }

    fn on_response(&mut self, id: RequestId, response: GithubResponse, _cx: &mut Cx<'_>) -> Option<GithubResponse> {
        match response {
            GithubResponse::RepoReadme(result) if self.pending() == Some(id) => {
                self.on_readme(result);
                None
            }
            other => Some(other),
        }
    }

    fn close(&self, cx: &mut Cx<'_>) {
        RepoOverview::close(self, cx);
    }

    fn scroll(&mut self, _tab: usize, scroll: Scroll) {
        RepoOverview::scroll(self, scroll);
    }

    fn wheel(&mut self, _tab: usize, wheel: Wheel) {
        RepoOverview::wheel(self, wheel);
    }

    fn render(&mut self, _tab: usize, frame: &mut Frame<'_>, area: Rect, theme: &Theme, _focused: bool) {
        RepoOverview::render(self, frame, area, theme);
    }
}

impl Layout for Body {
    fn layout(&mut self, width: u16, theme: &Theme) -> &Rendered {
        let Body { repo, readme, layouts } = self;
        layouts.get(width, theme, || overview(repo, readme, width, theme))
    }
}

/// The whole body laid out `width` cells wide: the header lines at the left edge, the rule, then the README.
fn overview(repo: &Repository, readme: &Readme, width: u16, theme: &Theme) -> Rendered {
    let mut out = Rendered::default();
    if width == 0 {
        return out;
    }
    let HeaderStyles { text, link } = HeaderStyles::new(theme);

    let mut title = vec![
        TextRun::link(
            repo.full_name.clone(),
            link.add_modifier(Modifier::BOLD),
            format!("https://github.com/{}", repo.full_name),
        ),
        TextRun::new(" • ", text),
    ];
    title.push(if repo.private {
        TextRun::new("Private", accent(theme, Accent::Red))
    } else {
        TextRun::new("Public", accent(theme, Accent::GreenYellow))
    });
    if repo.archived == Some(true) {
        title.push(TextRun::new(" • ", text));
        title.push(TextRun::new("Archived", accent(theme, Accent::Orchid)));
    }
    push_label(&mut out, &title, width);

    let owner = &repo.owner;
    let owned_by = [
        TextRun::new("Owned by ", text),
        TextRun::link(owner.login.clone(), link, owner.html_url.clone()),
    ];
    push_label(&mut out, &owned_by, width);

    if let Some(branch) = repo.default_branch.as_deref().filter(|branch| !branch.is_empty()) {
        let branch_line = [
            TextRun::new("Default branch: ", text),
            TextRun::new(branch, text.add_modifier(Modifier::BOLD)),
        ];
        push_label(&mut out, &branch_line, width);
    }

    // Literal text, not markup (Py bug B48).
    if let Some(description) = repo.description.as_deref().filter(|d| !d.is_empty()) {
        out.lines.push(Line::default());
        push_label(&mut out, &[TextRun::new(description, text)], width);
    }

    push_rule(&mut out, width, theme);

    match readme {
        Readme::Loading(_) => {}
        Readme::Loaded(markdown) => append(&mut out, markdown.layout(width, theme)),
        Readme::Missing => append(&mut out, NO_README.layout(width, theme)),
    }
    out
}

#[cfg(test)]
mod tests {
    use lazy_github_core::models::User;

    use super::*;

    fn repo() -> Repository {
        Repository {
            name: "hello-world".into(),
            full_name: "parity-bot/hello-world".into(),
            default_branch: Some("main".into()),
            private: false,
            archived: Some(false),
            owner: User {
                login: "parity-bot".into(),
                id: 1,
                name: None,
                html_url: "https://github.com/parity-bot".into(),
            },
            description: Some("A first repository.".into()),
            permissions: None,
        }
    }

    fn text(rendered: &Rendered) -> Vec<String> {
        rendered
            .lines
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect()
    }

    fn header(repo: &Repository, readme: &Readme) -> Vec<String> {
        text(&overview(repo, readme, 40, &Theme::default()))
    }

    #[test]
    fn a_public_repo_with_everything() {
        let rendered = overview(&repo(), &Readme::Loading(RequestId::new(1)), 40, &Theme::default());
        assert_eq!(
            text(&rendered),
            [
                "parity-bot/hello-world • Public",
                "Owned by parity-bot",
                "Default branch: main",
                "",
                "A first repository.",
                "",
                &"─".repeat(40),
                "",
            ]
        );
        let theme = Theme::default();
        let spans = &rendered.lines[0].spans;
        assert!(
            spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert_eq!(spans[2].style.fg, Some(theme.accent(Accent::GreenYellow)));
        assert_eq!(
            rendered.lines[6].spans[0].style.fg,
            Some(theme.color(theme.palette.secondary, theme.background()))
        );
        let links: Vec<(usize, std::ops::Range<u16>, &str)> = rendered
            .links
            .iter()
            .map(|link| (link.row, link.columns.clone(), &*link.target))
            .collect();
        assert_eq!(
            links,
            [
                (0, 0..22, "https://github.com/parity-bot/hello-world"),
                (1, 9..19, "https://github.com/parity-bot"),
            ]
        );
    }

    #[test]
    fn private_and_archived_without_branch_or_description() {
        let repo = Repository {
            private: true,
            archived: Some(true),
            default_branch: Some(String::new()),
            description: None,
            ..repo()
        };
        let rendered = overview(&repo, &Readme::Missing, 40, &Theme::default());
        assert_eq!(
            text(&rendered),
            [
                "parity-bot/hello-world • Private •",
                "Archived",
                "Owned by parity-bot",
                "",
                &"─".repeat(40),
                "",
                "  No README found.",
                "",
            ]
        );
        let theme = Theme::default();
        assert_eq!(rendered.lines[0].spans[2].style.fg, Some(theme.accent(Accent::Red)));
        assert_eq!(rendered.lines[1].spans[0].style.fg, Some(theme.accent(Accent::Orchid)));
        assert!(rendered.lines[6].spans[1].style.add_modifier.contains(Modifier::ITALIC));
        let no_branch = Repository {
            default_branch: None,
            ..repo
        };
        assert_eq!(header(&no_branch, &Readme::Missing)[2], "Owned by parity-bot");
    }

    #[test]
    fn the_description_is_literal_and_wraps() {
        let repo = Repository {
            description: Some("a [b]bold[/b] c [/] and a long tail that wraps past forty".into()),
            ..repo()
        };
        assert_eq!(
            header(&repo, &Readme::Missing)[4..6],
            ["a [b]bold[/b] c [/] and a long tail that", "wraps past forty"]
        );
    }

    #[test]
    fn the_readme_follows_the_rule() {
        let readme = Readme::Loaded(Markdown::parse("# Title\n\nText with a [link](https://example.com)."));
        let rendered = overview(&repo(), &readme, 40, &Theme::default());
        let lines = text(&rendered);
        assert_eq!(
            lines[7..],
            ["", "", "", "                 Title", "", "  Text with a link.", ""]
        );
        let readme_link = rendered.links.last().unwrap();
        assert_eq!((readme_link.row, &*readme_link.target), (12, "https://example.com"));
    }

    #[test]
    fn zero_width_is_empty() {
        assert!(
            overview(&repo(), &Readme::Missing, 0, &Theme::default())
                .lines
                .is_empty()
        );
    }
}

/// The overview in the app: selecting, the README request, and scrolling.
#[cfg(test)]
mod app_tests {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    use lazy_github_core::config::Config;
    use lazy_github_core::github::{FakeGithub, GithubCall};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::super::repos;
    use super::*;
    use crate::event::Event;
    use crate::screens::main::{FocusTarget, SectionId};
    use crate::test_util::{TestApp, buffer_to_styled_string, logs};

    /// The `repo-overview` fixtures' README of `name`, verbatim.
    fn fixture_readme(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../parity/fixtures/repo-overview/repos/parity-bot")
            .join(name)
            .join("readme/GET.raw.json");
        let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        fixture["response"]["body_text"].as_str().unwrap().to_owned()
    }

    /// The `repo-overview` set's three repos, in table order.
    fn parity_repos() -> Vec<Repository> {
        let mut repos = ["dotfiles", "hello-world", "lazy-demo"].map(|name| repos::repo("parity-bot", name, false));
        repos[0].description = Some("Configuration files.".into());
        repos[1].description = Some("A first repository for trying things out.".into());
        repos[2].archived = Some(true);
        repos.into()
    }

    fn gh() -> FakeGithub {
        FakeGithub::new()
            .with_repositories(parity_repos())
            .with_readme("parity-bot/hello-world", Ok(Some(fixture_readme("hello-world"))))
            .with_readme("parity-bot/dotfiles", Ok(Some(fixture_readme("dotfiles"))))
    }

    fn app(gh: FakeGithub) -> TestApp {
        TestApp::with_github(Config::default(), gh)
    }

    fn settle(app: &mut TestApp) -> String {
        logs(|| futures_util::FutureExt::now_or_never(app.settle()).expect("the fake answers at once"))
    }

    /// The README requests among the app's effects, by repo.
    fn readme_requests(app: &TestApp) -> Vec<(RequestId, String)> {
        app.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Github {
                    id,
                    request: GithubRequest::RepoReadme(name),
                } => Some((*id, name.clone())),
                _ => None,
            })
            .collect()
    }

    fn cancels(app: &TestApp) -> Vec<RequestId> {
        app.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Cancel(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn overview(app: &TestApp) -> &RepoOverview {
        app.app().main_screen().unwrap().details.overview().unwrap()
    }

    fn focus(app: &TestApp) -> FocusTarget {
        app.app().main_screen().unwrap().focus()
    }

    fn wheel(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn selecting_requests_the_readme_once() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        let requests = readme_requests(&app);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "parity-bot/hello-world");
        assert_eq!(overview(&app).pending(), Some(requests[0].0));
        settle(&mut app);
        assert_eq!(overview(&app).pending(), None);
        let readme_calls = app
            .gh()
            .calls()
            .into_iter()
            .filter(|call| matches!(call, GithubCall::RepoReadme(_)))
            .count();
        assert_eq!(readme_calls, 1);
        assert!(app.screen(160, 48).contains("Hello World"));
    }

    #[test]
    fn selecting_the_same_repo_again_requests_again() {
        let mut app = app(gh());
        app.press(&["enter"]);
        settle(&mut app);
        app.press(&["enter"]);
        let requests = readme_requests(&app);
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|(_, name)| name == "parity-bot/dotfiles"));
        // The first had finished, so there's nothing to cancel.
        assert_eq!(cancels(&app), []);
        settle(&mut app);
        assert!(app.screen(160, 48).contains("The first paragraph."));
    }

    #[test]
    fn a_missing_empty_or_failed_readme_shows_no_readme_found() {
        let cases: [(&str, Result<Option<String>, GhError>); 4] = [
            ("none", Ok(None)),
            ("empty", Ok(Some(String::new()))),
            (
                "server error",
                Err(GhError::Http {
                    status: 500,
                    message: "Server Error".into(),
                    body: Vec::new(),
                    headers: Vec::new(),
                }),
            ),
            (
                "timeout",
                Err(GhError::Timeout {
                    after: std::time::Duration::from_secs(30),
                }),
            ),
        ];
        for (case, result) in cases {
            let mut app = app(gh().with_readme("parity-bot/hello-world", result));
            app.press(&["j", "enter"]);
            let out = settle(&mut app);
            let screen = app.screen(160, 48);
            assert!(screen.contains("No README found."), "{case}\n{screen}");
            // A hung gh toasts that it timed out (`gh_health`); the README itself raises none.
            let expected = usize::from(case == "timeout");
            assert_eq!(app.app().toasts().len(), expected, "{case}");
            // Only debug lines: nothing reaches the command log.
            assert!(out.lines().all(|line| line.starts_with("DEBUG")), "{case}: {out}");
        }
    }

    #[test]
    fn a_new_selection_cancels_the_old_readme_and_drops_its_late_response() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        let (old, _) = readme_requests(&app)[0].clone();
        app.press(&["j", "enter"]);
        assert!(cancels(&app).contains(&old));
        assert_eq!(overview(&app).repo().full_name, "parity-bot/lazy-demo");
        // The old response arrives anyway, as one already sent can.
        let out = logs(|| {
            app.send(Event::Github {
                id: old,
                response: GithubResponse::RepoReadme(Ok(Some("# Hello World\n".into()))),
            });
        });
        assert!(out.contains("DEBUG Dropping the README response"), "{out}");
        let screen = app.screen(160, 48);
        assert!(screen.contains("parity-bot/lazy-demo • Public • Archived"), "{screen}");
        assert!(!screen.contains("Hello World"), "{screen}");
        assert!(overview(&app).pending().is_some());
        settle(&mut app);
        assert!(app.screen(160, 48).contains("No README found."));
    }

    #[test]
    fn scroll_keys_and_the_wheel_scroll_and_clamp() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        app.press(&["5", "tab"]);
        assert_eq!(focus(&app), FocusTarget::DetailsContent);
        app.press(&["down", "down", "left", "right"]);
        assert_eq!(overview(&app).offset(), 2);
        app.press(&["pagedown"]);
        let window = overview(&app).scroll.last_size().1;
        assert_eq!(overview(&app).offset(), 2 + window);
        app.press(&["end"]);
        let (content, window) = overview(&app).scroll.last_size();
        let max = content - window;
        assert_eq!(overview(&app).offset(), max);
        app.press(&["down"]);
        assert_eq!(overview(&app).offset(), max);
        app.press(&["home", "up"]);
        assert_eq!(overview(&app).offset(), 0);

        // The wheel scrolls it unfocused, two rows a notch.
        app.press(&["1"]);
        app.send(wheel(MouseEventKind::ScrollDown, 70, 15));
        assert_eq!(overview(&app).offset(), 2);
        app.send(wheel(MouseEventKind::ScrollUp, 70, 15));
        app.send(wheel(MouseEventKind::ScrollUp, 70, 15));
        assert_eq!(overview(&app).offset(), 0);
        assert_eq!(focus(&app), FocusTarget::Table(SectionId::Repos));
    }

    #[test]
    fn a_new_selection_starts_at_the_top() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        app.press(&["5", "tab", "end"]);
        assert!(overview(&app).offset() > 0);
        app.press(&["1", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        assert_eq!(overview(&app).offset(), 0);
    }

    #[test]
    fn a_resize_rewraps_and_keeps_the_offset_clamped() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        app.press(&["5", "tab", "end"]);
        let narrow = overview(&app).offset();
        app.render(160, 48);
        let (content, window) = overview(&app).scroll.last_size();
        assert_eq!(overview(&app).offset(), content - window);
        assert!(overview(&app).offset() < narrow);
    }

    #[test]
    fn a_click_on_the_body_focuses_it_and_maximize_there_does_nothing() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        app.click(70, 15);
        assert_eq!(focus(&app), FocusTarget::DetailsContent);
        app.press(&["M"]);
        assert_eq!(app.app().maximized(), None);
    }

    #[test]
    fn the_description_is_literal_in_the_app() {
        let mut repos = parity_repos();
        repos[0].description = Some("a [b]bold[/b] c [/] d".into());
        let mut app = app(FakeGithub::new().with_repositories(repos));
        app.press(&["enter"]);
        let screen = app.screen(160, 48);
        assert!(screen.contains("│a [b]bold[/b] c [/] d"), "{screen}");
    }

    #[test]
    fn repo_overview_loading_160x48() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        let screen = app.screen(160, 48);
        assert!(screen.contains("│parity-bot/hello-world • Public"), "{screen}");
        assert!(!screen.contains("Hello World"), "{screen}");
        insta::assert_snapshot!(screen);
    }

    #[test]
    fn repo_overview_readme_160x48() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        insta::assert_snapshot!(app.screen(160, 48));
    }

    #[test]
    fn repo_overview_readme_100x30() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        insta::assert_snapshot!(app.screen(100, 30));
    }

    #[test]
    fn repo_overview_no_readme() {
        let mut app = app(gh());
        app.press(&["j", "j", "enter"]);
        settle(&mut app);
        insta::assert_snapshot!(app.screen(160, 48));
    }

    #[test]
    fn repo_overview_scrolled_end_100x30() {
        let mut app = app(gh());
        app.press(&["j", "enter"]);
        settle(&mut app);
        app.render(100, 30);
        app.press(&["5", "tab", "end"]);
        let screen = app.screen(100, 30);
        assert!(screen.contains("The last paragraph."), "{screen}");
        insta::assert_snapshot!(screen);
    }

    #[test]
    fn repo_overview_light_theme() {
        let mut app = app(gh());
        let repo = parity_repos()[1].clone();
        let mut overview = app.with_cx(|cx| RepoOverview::new(&repo, cx));
        overview.on_readme(Ok(Some(fixture_readme("hello-world"))));
        let theme = Theme::by_name("textual-light").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        terminal
            .draw(|frame| overview.render(frame, frame.area(), &theme))
            .unwrap();
        insta::assert_snapshot!(buffer_to_styled_string(terminal.backend().buffer()));
    }
}
