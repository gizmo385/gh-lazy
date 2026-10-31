//! Link hints: the labels, the keys while they're up, which links get one, and their drawing over the main screen.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crossterm::event::MouseEventKind;
use lazy_github_core::config::Config;
use lazy_github_core::github::FakeGithub;
use lazy_github_core::links::GithubLink;
use lazy_github_core::models::{IssueListItem, Repository};
use rstest::rstest;
use serde_json::Value;

use super::*;
use crate::components::link_map::ScreenLink;
use crate::components::markdown::Markdown;
use crate::effect::{Effect, GithubRequest};
use crate::screens::main::FocusTarget;
use crate::test_util::{TestApp, key_event};
use crate::theme::Theme;
use crate::toast::Severity;

// The codes.

#[test]
fn codes_are_single_letters_up_to_26() {
    assert!(hint_codes(0).is_empty());
    assert_eq!(hint_codes(1), ["a"]);
    assert_eq!(hint_codes(26).concat(), HINT_ALPHABET);
}

#[test]
fn past_26_every_code_is_two_letters_and_none_starts_another() {
    let codes = hint_codes(27);
    assert_eq!(codes.len(), 27);
    assert_eq!(&codes[..4], ["aa", "as", "ad", "af"]);
    assert_eq!(codes[26], "sa");
    assert!(codes.iter().all(|code| code.len() == 2));
    for code in &codes {
        assert!(
            !codes
                .iter()
                .any(|other| other != code && other.starts_with(code.as_str())),
            "{code}"
        );
    }
}

#[test]
fn every_pair_once_and_no_more_than_676() {
    let all = hint_codes(676);
    assert_eq!(all.len(), 676);
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 676);
    assert_eq!(all.last().map(String::as_str), Some("mm"));
    assert_eq!(hint_codes(677), all);
}

// The labels.

fn link(x: u16, y: u16, width: u16, target: &str) -> ScreenLink {
    ScreenLink {
        x,
        y,
        width,
        target: target.into(),
    }
}

fn map(links: &[ScreenLink]) -> LinkMap {
    let mut map = LinkMap::default();
    for link in links {
        map.push(link.clone());
    }
    map
}

#[test]
fn no_links_no_labels() {
    assert!(LinkHintsDialog::new(&LinkMap::default(), false).is_none());
}

#[test]
fn labels_follow_draw_order_and_a_shared_start_gets_one() {
    let links = map(&[
        link(10, 5, 8, "https://b"),
        link(2, 3, 4, "https://a"),
        // Starts where the first did: no label of its own.
        link(10, 5, 3, "https://c"),
        // A wrapped link's second row: a label of its own.
        link(0, 6, 5, "https://b"),
    ]);
    let hints = LinkHintsDialog::new(&links, false).unwrap();
    assert_eq!(hints.labels(), [("a", 10, 5), ("s", 2, 3), ("d", 0, 6)]);
    assert!(!hints.is_external());
}

#[test]
fn a_table_labels_cell_by_cell() {
    // The first cell wraps onto two lines, each with a link; the second has one link on its first line. Python's
    // order is the first cell's two, then the second's; reading order would put the second cell's between them.
    let source = "| a | b |\n|---|---|\n| [one](https://one) [two](https://two) | [three](https://three) |\n";
    let rendered = Markdown::parse(source).layout(20, &Theme::default());
    let targets: Vec<&str> = rendered.links.iter().map(|link| &*link.target).collect();
    let rows: Vec<usize> = rendered.links.iter().map(|link| link.row).collect();
    assert_eq!(targets, ["https://one", "https://two", "https://three"]);
    assert!(rows[1] > rows[2], "the second link wraps below the third: {rows:?}");
}

// The keys.

fn hints(count: usize) -> LinkHintsDialog {
    let links: Vec<ScreenLink> = (0..count)
        .map(|n| link(0, u16::try_from(n).unwrap(), 4, &format!("https://example.com/{n}")))
        .collect();
    LinkHintsDialog::new(&map(&links), false).unwrap()
}

fn press(dialog: &mut LinkHintsDialog, keys: &[&str]) {
    let mut app = TestApp::new(Config::default());
    for key in keys {
        let result = app.with_cx(|cx| dialog.handle_key(key_event(key), cx));
        assert!(matches!(result, Some(Action::Consumed)), "{key} is taken");
    }
}

#[test]
fn a_full_label_picks_its_link() {
    let mut dialog = hints(3);
    press(&mut dialog, &["s"]);
    assert_eq!(
        dialog.take_output(),
        Some(Some((Arc::from("https://example.com/1"), false)))
    );
}

#[test]
fn the_first_letter_of_a_two_letter_label_waits() {
    let mut dialog = hints(30);
    press(&mut dialog, &["s"]);
    assert_eq!(dialog.take_output(), None);
    press(&mut dialog, &["d"]);
    // `sd` is the 29th: `aa` to `am` are 26, then `sa`, `ss`, `sd`.
    assert_eq!(
        dialog.take_output(),
        Some(Some((Arc::from("https://example.com/28"), false)))
    );
}

#[rstest]
#[case::no_label_starts_with_it(&["z"])]
#[case::no_label_continues_with_it(&["s", "g"])]
#[case::escape(&["escape"])]
fn cancelling_closes_with_nothing(#[case] keys: &[&str]) {
    let mut dialog = if keys.len() == 2 { hints(30) } else { hints(3) };
    press(&mut dialog, keys);
    assert_eq!(dialog.take_output(), Some(None));
}

#[test]
fn other_keys_are_ignored_and_the_labels_stay() {
    let mut dialog = hints(3);
    press(
        &mut dialog,
        &["1", "A", "enter", "question_mark", "tab", "ctrl+a", "f1"],
    );
    assert_eq!(dialog.take_output(), None);
    assert_eq!(dialog.typed, "");
}

// On the main screen.

/// The response body of the parity fixture at `path`.
fn fixture_body(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures")
        .join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

fn issue_fixture() -> Repository {
    serde_json::from_value(fixture_body("links/repos/parity-bot/issue-fixture/GET.json")).unwrap()
}

fn readme(path: &str) -> String {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../parity/fixtures")
                .join(path),
        )
        .unwrap(),
    )
    .unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

/// The `link-hints` scenario's GitHub: `issue-fixture` with its README, issue 12 and the issue it links to.
fn github() -> FakeGithub {
    let repo = issue_fixture();
    let issue = |set: &str, number: u64| {
        let body = fixture_body(&format!(
            "{set}/repos/parity-bot/issue-fixture/issues/{number}/GET.json"
        ));
        IssueListItem::from_api_item(body, &repo).unwrap()
    };
    FakeGithub::new()
        .with_repositories(vec![repo.clone()])
        .with_readme(
            "parity-bot/issue-fixture",
            Ok(Some(readme("links/repos/parity-bot/issue-fixture/readme/GET.raw.json"))),
        )
        .with_issue_page("parity-bot/issue-fixture", 1, Ok(vec![issue("issue-details", 12)]))
        .with_repository("parity-bot/issue-fixture", Ok(Some(repo.clone())))
        .with_issue("parity-bot/issue-fixture", 7, Ok(issue("links", 7)))
}

/// The main screen with `issue-fixture` selected and drawn at 160x48.
async fn repo_overview() -> TestApp {
    let mut app = TestApp::with_github(Config::default(), github());
    app.settle().await;
    app.press(&["enter"]);
    app.settle().await;
    app.render(160, 48);
    app
}

fn toasts(app: &TestApp) -> Vec<(Severity, String)> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| (toast.severity, toast.message.to_string()))
        .collect()
}

fn opened(app: &TestApp) -> Vec<String> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::OpenUrl(url) => Some(url.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn with_no_links_f_toasts_and_opens_nothing() {
    let mut app = TestApp::with_github(Config::default(), github());
    app.settle().await;
    app.render(160, 48);
    for key in ["f", "F"] {
        app.press(&[key]);
        assert!(app.app().modals().is_empty(), "{key}");
    }
    assert_eq!(
        toasts(&app),
        [
            (Severity::Information, "No links visible".to_owned()),
            (Severity::Information, "No links visible".to_owned())
        ]
    );
}

#[tokio::test]
async fn f_labels_the_repo_overviews_title_and_owner() {
    let mut app = repo_overview().await;
    app.press(&["f"]);
    assert_eq!(app.app().modals(), [ModalKind::LinkHints]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("aarity-bot/issue-fixture • Public"), "{screen}");
    assert!(screen.contains("Owned by sarity-bot"), "{screen}");
    // The screen underneath is as it was: the footer is the main screen's, focus where it was.
    assert!(screen.contains("f Follow link"), "{screen}");
    assert_eq!(
        app.app().main_screen().unwrap().focus(),
        FocusTarget::Table(crate::screens::main::SectionId::Repos)
    );
    insta::assert_snapshot!("link_hints_repo_overview", screen);
}

#[tokio::test]
async fn labels_are_bold_only_and_drop_the_links_underline() {
    let mut app = repo_overview().await;
    let before = app.render(160, 48);
    app.press(&["f"]);
    let buf = app.render(160, 48);
    let mut labels = 0;
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            if before[(x, y)].modifier.contains(Modifier::UNDERLINED) && cell.symbol() != before[(x, y)].symbol() {
                assert_eq!(
                    cell.modifier,
                    Modifier::BOLD,
                    "label cell ({x}, {y}) is {:?}",
                    cell.modifier
                );
                labels += 1;
            }
        }
    }
    assert_eq!(labels, 2, "the title's and the owner's labels");
}

#[tokio::test]
async fn escape_cancels_and_the_main_screen_keeps_focus() {
    let mut app = repo_overview().await;
    app.press(&["f", "escape"]);
    assert!(app.app().modals().is_empty());
    let screen = app.screen(160, 48);
    assert!(screen.contains("Owned by parity-bot"), "{screen}");
    assert!(opened(&app).is_empty());
    assert!(toasts(&app).is_empty());
}

#[tokio::test]
async fn capital_f_opens_the_pick_externally_with_a_toast() {
    let mut app = repo_overview().await;
    app.press(&["F", "s"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(opened(&app), ["https://github.com/parity-bot"]);
    assert_eq!(
        toasts(&app),
        [(
            Severity::Information,
            "Opening https://github.com/parity-bot externally".to_owned()
        )]
    );
}

#[tokio::test]
async fn f_follows_the_pick_as_a_click_would() {
    let mut app = repo_overview().await;
    // The repo title: a bare repo, so the browser, with nothing shown.
    app.press(&["f", "a"]);
    assert_eq!(opened(&app), ["https://github.com/parity-bot/issue-fixture"]);
    assert!(toasts(&app).is_empty());
    // Issue 12's #7 resolves in the app.
    app.press(&["3", "enter"]);
    app.settle().await;
    app.render(160, 48);
    app.press(&["f"]);
    let screen = app.screen(160, 48);
    insta::assert_snapshot!("link_hints_issue_overview", screen);
    app.press(&["d"]);
    let resolved: Vec<GithubLink> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github {
                request: GithubRequest::ResolveLink(link),
                ..
            } => Some(link.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        resolved,
        [GithubLink::Issue {
            owner: "parity-bot".into(),
            repo: "issue-fixture".into(),
            number: 7
        }]
    );
    app.settle().await;
    assert_eq!(
        app.app()
            .main_screen()
            .unwrap()
            .details_issue()
            .map(|issue| issue.number),
        Some(7)
    );
}

#[tokio::test]
async fn the_mouse_does_nothing_while_the_labels_are_up() {
    let mut app = repo_overview().await;
    app.press(&["f"]);
    app.click(5, 5);
    app.mouse(MouseEventKind::ScrollDown, 100, 20);
    assert_eq!(app.app().modals(), [ModalKind::LinkHints]);
    assert_eq!(
        app.app().main_screen().unwrap().focus(),
        FocusTarget::Table(crate::screens::main::SectionId::Repos)
    );
}

/// A repo whose README lists 30 links, more than the screen shows.
async fn many_links() -> TestApp {
    let repo = issue_fixture();
    let readme = (1..=40).fold(String::from("# Links\n\n"), |mut readme, n| {
        // Writing to a `String` can't fail.
        let _ = writeln!(readme, "- [link {n}](https://example.com/{n})");
        readme
    });
    let gh = FakeGithub::new()
        .with_repositories(vec![repo])
        .with_readme("parity-bot/issue-fixture", Ok(Some(readme)));
    let mut app = TestApp::with_github(Config::default(), gh);
    app.settle().await;
    app.press(&["enter"]);
    app.settle().await;
    app.render(160, 48);
    app
}

#[tokio::test]
async fn more_than_26_links_get_two_letters_each() {
    let mut app = many_links().await;
    app.press(&["f"]);
    let screen = app.screen(160, 48);
    // The title and owner, then the README's links on screen: more than 26, so `aa`, `as`, ..., each two cells.
    assert!(screen.contains("aarity-bot/issue-fixture"), "{screen}");
    assert!(screen.contains("Owned by asrity-bot"), "{screen}");
    assert!(screen.contains("• adnk 1"), "{screen}");
    insta::assert_snapshot!("link_hints_two_letters", screen);
}

#[tokio::test]
async fn only_the_links_on_screen_get_labels() {
    let mut app = many_links().await;
    // Scrolled to the end: the header and the first links are gone, and only what's drawn is labelled (Py bug
    // B99's answer).
    app.press(&["5", "tab", "end"]);
    app.render(160, 48);
    let drawn = app.app().links().iter().count();
    assert!(drawn < 42, "not every link is on screen: {drawn}");
    app.press(&["f"]);
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Owned by"), "{screen}");
    insta::assert_snapshot!("link_hints_scrolled", screen);
}
