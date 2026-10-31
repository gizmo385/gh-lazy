//! The User profile dialog: its lines, focus, keys and layout, against the `user-profile` parity fixtures.

use lazy_github_core::config::Config;
use lazy_github_core::models::{FullUser, Repository};

use super::*;
use crate::components::footer::footer_items;
use crate::test_util::TestApp;
use crate::toast::{Severity, Toast};

/// The `response.body` of a `user-profile` fixture.
pub(crate) fn fixture(file: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parity/fixtures/user-profile")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body"].clone()
}

/// `parity-bot`, with every field set.
pub(crate) fn parity_bot() -> FullUser {
    serde_json::from_value(fixture("users/parity-bot/GET.json")).unwrap()
}

/// `parity-bot`'s four repos, latest update first.
pub(crate) fn parity_bot_repos() -> Vec<Repository> {
    serde_json::from_value(fixture(
        "users/parity-bot/repos/GET.direction-desc_page-1_per_page-30_sort-updated.json",
    ))
    .unwrap()
}

/// `parity-org`: an organization with no name, bio or repos.
pub(crate) fn parity_org() -> FullUser {
    serde_json::from_value(fixture("users/parity-org/GET.json")).unwrap()
}

/// Opens `dialog`, whose reply raises a toast naming the picked repo.
fn open(app: &mut TestApp, dialog: UserProfileDialog) {
    app.with_cx(|cx| {
        cx.open_dialog(dialog, |picked| {
            let name = picked.map_or_else(|| "nothing".to_owned(), |repo| repo.full_name);
            Action::Notify(Toast::new(Severity::Information, format!("picked {name}")))
        });
    });
}

fn app_with(dialog: UserProfileDialog) -> TestApp {
    let mut app = TestApp::new(Config::default());
    open(&mut app, dialog);
    app
}

fn full_app() -> TestApp {
    app_with(UserProfileDialog::new(parity_bot(), parity_bot_repos()))
}

/// The replies the opener got, oldest first.
fn replies(app: &TestApp) -> Vec<String> {
    app.app()
        .toasts()
        .iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

fn is_open(app: &TestApp) -> bool {
    app.app().modals() == [ModalKind::UserProfile]
}

fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

/// The body's lines at 84 columns of content, the width at 160x48, with the table's rows blank.
fn lines(user: FullUser, repos: Vec<Repository>) -> Vec<String> {
    UserProfileDialog::new(user, repos).lines(88)
}

// Lines.

#[test]
fn the_full_profile_lines() {
    let lines = lines(parity_bot(), parity_bot_repos());
    let rule = format!("  {}", "─".repeat(84));
    let mut expected: Vec<String> = [
        "",
        "  Parity Bot (@parity-bot) — user",
        "",
        &rule,
        "",
        "    Fictional account for LazyGithub parity fixtures.",
        "",
        "    Keeps dotfiles and a few demos.",
        "",
        "",
        &rule,
        "",
        "  Company: @parity-org",
        "",
        "  Location: Testville",
        "",
        "  Email: user@example.com",
        "",
        "  Blog: https://example.com/parity-bot",
        "",
        "  Twitter: @paritybot",
        "",
        &rule,
        "",
        "  Public repos: 4    Gists: 2    Followers: 12    Following: 3",
        "",
        "  Joined: 2024-01-02",
        "",
        &rule,
        "",
        "  Recent repositories (showing 4 of 4, press enter to open)",
        "",
    ]
    .map(str::to_owned)
    .into();
    // The table: its header, four rows and the scrollbar the long description needs, then the padding.
    expected.extend(std::iter::repeat_n(String::new(), 7));
    assert_eq!(lines, expected);
}

#[test]
fn the_organization_lines() {
    let lines = lines(parity_org(), Vec::new());
    let rule = format!("  {}", "─".repeat(84));
    assert_eq!(
        lines,
        [
            "",
            "  parity-org (@parity-org) — organization",
            "",
            &rule,
            "",
            "  Location: Remote",
            "",
            &rule,
            "",
            "  Public repos: 0    Gists: 0    Followers: 1    Following: 0",
            "",
            "  Joined: 2025-05-06",
            "",
            &rule,
            "",
            "  Recent repositories",
            "",
            "  No public repositories.",
            "",
            "",
        ]
    );
}

#[test]
fn missing_fields_fall_back() {
    let mut user = parity_org();
    user.created_at = None;
    user.kind = None;
    user.location = Some(String::new());
    user.user.name = Some(String::new());
    let lines = lines(user, Vec::new());
    assert_eq!(lines[1], "  parity-org (@parity-org) — user");
    assert!(lines.contains(&"  Joined: unknown".to_owned()), "{lines:#?}");
    // No info line, so no rule after them either: the header's rule, the counts' rule.
    assert!(!lines.iter().any(|line| line.contains("Location")));
    assert_eq!(lines.iter().filter(|line| line.contains('─')).count(), 2);
}

#[test]
fn fields_are_literal_text() {
    // Py bug B49: markup in a name or a field is shown as typed.
    let mut user = parity_bot();
    user.user.name = Some("Acme [b]Labs[/b]".into());
    user.company = Some("[red]Evil[/x]".into());
    let lines = lines(user, Vec::new());
    assert_eq!(lines[1], "  Acme [b]Labs[/b] (@parity-bot) — user");
    assert!(lines.contains(&"  Company: [red]Evil[/x]".to_owned()), "{lines:#?}");
}

#[test]
fn the_login_links_to_the_profile() {
    let mut dialog = UserProfileDialog::new(parity_bot(), Vec::new());
    let theme = Theme::default();
    let links = &dialog.body.layout(88, &theme).links;
    let login = links.first().unwrap();
    assert_eq!(&*login.target, "https://github.com/parity-bot");
    assert_eq!(login.row, 1);
    // "  Parity Bot (" is 14 columns.
    assert_eq!(login.columns, 14..25);
}

// Links (#355): the bio is a Textual `Markdown`, which keeps `open_links` in the dialog, so a click on one of its links
// opens it in the browser. The header's link is a `Label`'s, which Textual doesn't follow.

#[test]
fn a_click_on_a_link_in_the_bio_opens_it_in_the_browser() {
    let mut user = parity_bot();
    user.bio = Some("Docs at [zzdocs](https://example.com/docs).".into());
    let mut app = app_with(UserProfileDialog::new(user, parity_bot_repos()));
    let buffer = app.render(160, 48);
    let links: Vec<_> = app.app().dialog_links().iter().cloned().collect();
    assert_eq!(links.len(), 1, "only the bio's: {links:?}");
    let link = &links[0];
    assert_eq!(&*link.target, "https://example.com/docs");
    let text: String = (link.x..link.x + link.width)
        .map(|x| buffer[(x, link.y)].symbol().to_owned())
        .collect();
    assert_eq!(text, "zzdocs");

    app.click(link.x, link.y);
    let opened: Vec<_> = app
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            crate::effect::Effect::OpenUrl(url) => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["https://example.com/docs"]);
    assert!(is_open(&app));
    assert!(replies(&app).is_empty());
}

#[test]
fn without_a_bio_nothing_in_the_dialog_is_clickable() {
    let mut app = app_with(UserProfileDialog::new(parity_org(), Vec::new()));
    app.render(160, 48);
    assert!(app.app().dialog_links().is_empty());
}

// Focus and keys.

#[test]
fn focus_starts_on_the_body_where_enter_does_nothing() {
    let mut app = full_app();
    app.render(160, 48);
    assert_eq!(footer(&mut app), ["q Close"]);
    app.press(&["enter", "space"]);
    assert!(is_open(&app));
    assert!(replies(&app).is_empty());
}

#[test]
fn the_scroll_keys_scroll_the_body() {
    let mut app = full_app();
    let top = app.screen(100, 30);
    app.press(&["down", "down"]);
    let scrolled = app.screen(100, 30);
    // The body's text, inside the frame and left of its scrollbar.
    let row = |screen: &str, i: usize| {
        screen
            .lines()
            .nth(i)
            .unwrap()
            .chars()
            .skip(6)
            .take(80)
            .collect::<String>()
    };
    assert_eq!(row(&scrolled, 3), row(&top, 5));
    app.press(&["end"]);
    assert!(app.screen(100, 30).contains("Recent repositories"));
}

#[test]
fn tab_goes_to_the_table_and_scrolls_it_into_view() {
    let mut app = full_app();
    let before = app.screen(100, 30);
    assert!(!before.contains("lazy-demo"));
    app.press(&["tab"]);
    let after = app.screen(100, 30);
    // The whole table shows, down to the container's padding.
    for name in ["dotfiles", "profile-only", "hello-world", "lazy-demo"] {
        assert!(after.contains(name), "{after}");
    }
    // shift+tab goes back to the body, which stays where it is.
    app.press(&["shift+tab"]);
    assert_eq!(app.screen(100, 30), after);
}

#[test]
fn enter_on_the_table_closes_with_that_repo() {
    let mut app = full_app();
    app.render(160, 48);
    app.press(&["tab", "j", "enter"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(replies(&app), ["picked parity-bot/profile-only"]);
}

#[test]
fn modal_close_closes_with_nothing() {
    for key in ["q", "escape"] {
        let mut app = full_app();
        app.render(160, 48);
        app.press(&["tab", key]);
        assert!(app.app().modals().is_empty(), "{key}");
        assert_eq!(replies(&app), ["picked nothing"], "{key}");
    }
}

#[test]
fn without_repos_tab_stays_on_the_body() {
    let mut app = app_with(UserProfileDialog::new(parity_org(), Vec::new()));
    app.render(160, 48);
    app.press(&["tab", "enter"]);
    assert!(is_open(&app));
    assert!(replies(&app).is_empty());
}

#[test]
fn a_click_on_the_cursor_row_picks_it() {
    let mut app = full_app();
    let screen = app.screen(160, 48);
    let row = screen
        .lines()
        .position(|line| line.contains("hello-world   ✘"))
        .unwrap();
    let row = u16::try_from(row).unwrap();
    // The first click moves the cursor there, the second picks it.
    app.click(60, row);
    assert!(is_open(&app));
    app.click(60, row);
    assert!(app.app().modals().is_empty());
    assert_eq!(replies(&app), ["picked parity-bot/hello-world"]);
}

#[test]
fn the_size_is_90_wide_and_nine_tenths_of_the_rows() {
    let dialog = UserProfileDialog::new(parity_org(), Vec::new());
    assert_eq!(dialog.size(Rect::new(0, 0, 160, 47)), (90, 42));
    assert_eq!(dialog.size(Rect::new(0, 0, 100, 29)), (90, 26));
}

// Snapshots.

#[test]
fn user_profile_160x48() {
    let mut app = full_app();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn user_profile_tabbed_100x30() {
    let mut app = full_app();
    app.render(100, 30);
    app.press(&["tab"]);
    insta::assert_snapshot!(app.screen(100, 30));
}

#[test]
fn user_profile_organization_160x48() {
    let mut app = app_with(UserProfileDialog::new(parity_org(), Vec::new()));
    insta::assert_snapshot!(app.screen(160, 48));
}

// Terminal hyperlinks (#356).

#[test]
fn the_login_is_a_terminal_hyperlink_and_the_bio_links_are_not() {
    let mut user = parity_bot();
    user.bio = Some("See [the docs](https://example.com/docs).".to_owned());
    let mut app = app_with(UserProfileDialog::new(user, parity_bot_repos()));
    let screen = app.screen(120, 40);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("(@parity-bot)"))
        .unwrap();
    let column = line[..line.find("@parity-bot").unwrap()].chars().count();
    assert_eq!(
        app.hyperlinks(120, 40),
        [(
            (u16::try_from(column).unwrap(), u16::try_from(row).unwrap()),
            "@parity-bot".to_owned(),
            "https://github.com/parity-bot".to_owned()
        )]
    );
}

#[test]
fn a_login_scrolled_out_of_view_is_not_linked() {
    let mut app = full_app();
    app.press(&["j", "j", "j", "j"]);
    let screen = app.screen(120, 12);
    assert!(!screen.contains("@parity-bot)"), "{screen}");
    assert!(app.hyperlinks(120, 12).is_empty());
}

// Colors (#359).

#[test]
fn inline_code_in_the_bio_blends_over_the_dialogs_background() {
    use ratatui::style::Color;

    let mut app = full_app();
    let buf = app.render(160, 48);
    let screen = crate::test_util::buffer_to_string(&buf);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Keeps dotfiles"))
        .unwrap();
    let column = line[..line.find("dotfiles").unwrap()].chars().count();
    let (x, y) = (u16::try_from(column).unwrap(), u16::try_from(row).unwrap());
    // Python's cells for `dotfiles` in the `user-profile` and `link-hints` scenarios (textual-dark): `$warning 10%`
    // blended into the container's `$surface-lighten-1` (#2d2d2d), not into the screen's `$background`.
    assert_eq!(buf[(x, y)].fg, Color::Rgb(0xf5, 0xbc, 0x6f));
    assert_eq!(buf[(x, y)].bg, Color::Rgb(0x3d, 0x36, 0x2c));
    assert_eq!(buf[(x - 1, y)].bg, Color::Rgb(0x2d, 0x2d, 0x2d));
}
