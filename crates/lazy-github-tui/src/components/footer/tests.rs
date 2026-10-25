use lazy_github_core::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::component::{BindingState, ChainLink, ScreenKind};
use crate::keymap::{Keymap, active_bindings};
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::TestApp;

/// The footer row as Textual draws it at an unlimited width, trailing space trimmed.
fn joined(groups: &[ActiveGroup]) -> String {
    let items: Vec<String> = footer_items(groups)
        .iter()
        .map(|(key, description)| format!(" {key} {description} "))
        .collect();
    items.concat().trim_end().to_owned()
}

fn footer_after(config: Config, keys: &[&str]) -> String {
    let mut app = TestApp::new(config);
    app.press(keys);
    joined(&app.app_mut().active_bindings())
}

fn notifications_on() -> Config {
    let mut config = Config::default();
    config.notifications.enabled = true;
    config
}

/// The screen's and the app's items, which every main-screen footer ends with.
const TAIL: &str = "^v Paste link  f Follow link  F Follow link (browser)  q Quit  ^p Command Palette  M Maximize  \
                    ? Keybindings";

/// Python's footers from #107, with B14's spelling fixed.
#[test]
fn footers_match_python_for_each_focus() {
    let new = "I New Issue  P New PR";
    for (keys, expected) in [
        (
            &[][..],
            format!(" / Search  ^f Toggle Favorite  O Lookup Repository  u View User  {new}  {TAIL}"),
        ),
        (
            &["2"][..],
            format!(" / Search  O Lookup Pull Request  E Edit PR  u View User  {new}  {TAIL}"),
        ),
        (
            &["3"][..],
            format!(" / Search  O Lookup Issue  E Edit Issue  u View User  {new}  {TAIL}"),
        ),
        (&["4"][..], format!(" / Search  T Trigger Workflow  {new}  {TAIL}")),
        (&["5"][..], format!(" {TAIL}")),
        (&["5", "tab"][..], format!(" {TAIL}")),
    ] {
        assert_eq!(footer_after(Config::default(), keys), expected, "after {keys:?}");
    }
}

/// B7: `^n Notifications` is listed only while `notifications.enabled`.
#[test]
fn notifications_show_only_while_enabled() {
    assert_eq!(
        footer_after(notifications_on(), &[]),
        format!(
            " / Search  ^f Toggle Favorite  O Lookup Repository  u View User  I New Issue  P New PR  \
             ^n Notifications  {TAIL}"
        )
    );
    let mut config = notifications_on();
    config.appearance.show_command_log = true;
    assert_eq!(footer_after(config, &["6"]), format!(" ^n Notifications  {TAIL}"));
}

#[test]
fn overrides_change_keys_and_the_table_takes_h() {
    let mut config = Config::default();
    let overrides = &mut config.bindings.overrides;
    for (id, keys) in [
        ("main.repos.focus", "9"),
        ("main.pull_requests.focus", "x, ctrl+t"),
        ("common.table.search", "question_mark"),
        ("app.open_help", "h"),
        ("app.maximize_widget", "bogus"),
    ] {
        overrides.insert(id.into(), keys.into());
    }
    assert_eq!(
        footer_after(config, &[]),
        " ? Search  ^f Toggle Favorite  O Lookup Repository  u View User  I New Issue  P New PR  ^v Paste link  \
         f Follow link  F Follow link (browser)  q Quit  ^p Command Palette  bogus Maximize"
    );
}

#[test]
fn the_auth_screen_shows_only_quit() {
    let chain = [ChainLink {
        owner: Owner::Screen(ScreenKind::Auth),
        bindings: &[BindingId::AppQuit],
    }];
    let groups = active_bindings(&chain, &Keymap::defaults(), |_, _| BindingState::Enabled);
    assert_eq!(joined(&groups), " q Quit");
}

/// The last row of the main screen at `cols` x `rows`, after `keys`.
fn footer_row(keys: &[&str], cols: u16, rows: u16) -> String {
    let mut app = TestApp::new(Config::default());
    app.press(keys);
    let screen = app.screen(cols, rows);
    screen.lines().last().unwrap_or_default().to_owned()
}

#[test]
fn footer_repos_160x48() {
    insta::assert_snapshot!(footer_row(&[], 160, 48));
}

#[test]
fn footer_details_100x30() {
    insta::assert_snapshot!(footer_row(&["5"], 100, 30));
}

#[test]
fn long_footers_are_cut_at_the_edge() {
    assert!(footer_row(&[], 160, 48).ends_with("  q Quit  ^p Comman"));
    assert!(footer_row(&[], 100, 30).ends_with("  P New PR  ^v Paste lin"));
}

fn quit_group(enabled: bool) -> ActiveGroup {
    ActiveGroup {
        owner: Owner::App,
        id: BindingId::AppQuit,
        action: "quit",
        description: "Quit",
        tooltip: "",
        keys: vec!["q".into()],
        show: true,
        enabled,
    }
}

#[test]
fn footer_styles() {
    // ` q ` in the key style, `Quit ` in the description style, the rest of the row on `$footer-background`.
    let theme = Theme::default();
    let groups = [quit_group(true)];
    let mut terminal = Terminal::new(TestBackend::new(12, 1)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Footer {
                    groups: &groups,
                    theme: &theme,
                },
                frame.area(),
            );
        })
        .unwrap();
    insta::assert_debug_snapshot!(terminal.backend().buffer());
}

#[test]
fn disabled_items_are_dim() {
    let theme = Theme::default();
    for enabled in [true, false] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        Footer {
            groups: &[quit_group(enabled)],
            theme: &theme,
        }
        .render(buf.area, &mut buf);
        for x in [1, 4] {
            assert_eq!(buf[(x, 0)].modifier.contains(Modifier::DIM), !enabled);
        }
    }
}

#[test]
fn hits_cover_each_item_and_stop_at_the_edge() {
    let mut app = TestApp::new(Config::default());
    let groups = app.app_mut().active_bindings();
    let theme = Theme::default();
    let footer = Footer {
        groups: &groups,
        theme: &theme,
    };
    // " / Search " is 10 wide and " ^f Toggle Favorite " 20, cut to the 25 columns left.
    let hits = footer.hits(Rect::new(5, 29, 35, 1));
    assert_eq!(
        hits.iter()
            .map(|hit| (hit.rect.x, hit.rect.width, hit.id))
            .collect::<Vec<_>>(),
        [
            (5, 10, BindingId::TableSearch),
            (15, 20, BindingId::RepositoriesFavorite),
            (35, 5, BindingId::RepositoriesLookup)
        ]
    );
    assert_eq!(hits[0].owner, Owner::SearchableTable(SectionId::Repos));
    assert!(footer.hits(Rect::new(0, 0, 0, 1)).is_empty());
}

#[test]
fn clicking_maximize_maximizes() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5"]);
    let screen = app.screen(160, 48);
    let row = screen.lines().last().unwrap();
    let byte = row.find(" M Maximize").unwrap();
    let col = u16::try_from(row[..byte].chars().count()).unwrap() + 1;
    app.click(col, 47);
    assert_eq!(app.app().maximized().map(|m| m.target), Some(FocusTarget::DetailsTabs));
    // The same toast as pressing M.
    assert_eq!(app.app().toasts().len(), 1);
}

#[test]
fn the_footer_is_hidden_while_maximized() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5", "M"]);
    let screen = app.screen(100, 30);
    assert!(!screen.contains("Keybindings"), "{screen}");
    // Its old cells take no clicks: this would have been `q Quit`'s place.
    app.click(60, 29);
    assert!(!app.app().should_quit());
}
