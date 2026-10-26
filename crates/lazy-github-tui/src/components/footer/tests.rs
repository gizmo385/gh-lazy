use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use lazy_github_core::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::component::{BindingState, ChainLink, ScreenKind};
use crate::event::Event;
use crate::keymap::{Keymap, active_bindings};
use crate::screens::main::{FocusTarget, SectionId};
use crate::test_util::{TestApp, buffer_to_string};

/// The footer row as Textual draws it at an unlimited width, trailing space trimmed.
fn joined(active: &[ActiveBinding]) -> String {
    let items: Vec<String> = footer_items(active)
        .iter()
        .map(|item| format!(" {} {} ", item.key, item.description))
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

fn comma_only_help() -> Config {
    let mut config = Config::default();
    config.bindings.overrides.insert("app.open_help".into(), " , ".into());
    config
}

/// `" , "` binds `app.open_help` to the key `""`, which Textual's footer lists with no key text: Python's footer
/// ends `q Quit   Keybindings` (#158).
#[test]
fn a_comma_only_override_is_listed_with_no_key() {
    assert_eq!(
        footer_after(comma_only_help(), &["5"]),
        " ^v Paste link  f Follow link  F Follow link (browser)  q Quit  ^p Command Palette  M Maximize   Keybindings"
    );
}

#[test]
fn footer_comma_only_override_120x30() {
    let mut app = TestApp::new(comma_only_help());
    app.press(&["5"]);
    let screen = app.screen(120, 30);
    insta::assert_snapshot!(screen.lines().last().unwrap_or_default());
}

#[test]
fn the_auth_screen_shows_only_quit() {
    let chain = [ChainLink::new(Owner::Screen(ScreenKind::Auth), &[BindingId::AppQuit])];
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

fn quit_item(enabled: bool) -> FooterItem {
    FooterItem {
        owner: Owner::App,
        id: BindingId::AppQuit,
        key: "q".into(),
        description: "Quit",
        enabled,
    }
}

/// Textual groups the footer by action across the whole chain, and only among the shown bindings: the hidden
/// `common.table.select` doesn't join `workflows.view_run_details`, and the hidden `ctrl+q` doesn't take `q Quit`.
#[test]
fn items_group_shown_bindings_by_action() {
    let chain = [
        ChainLink::new(
            Owner::Table(SectionId::Workflows),
            &[BindingId::TableSelect, BindingId::WorkflowsViewRunDetails],
        ),
        ChainLink::new(
            Owner::App,
            &[BindingId::TextualQuit, BindingId::AppQuit, BindingId::AppOpenHelp],
        ),
    ];
    let active = active_bindings(&chain, &Keymap::defaults(), |_, _| BindingState::Enabled);
    // `enter` goes to the hidden select, so View Run Details is left with no key.
    assert_eq!(joined(&active), " q Quit  ? Keybindings");
    let chain = [
        ChainLink::new(
            Owner::Table(SectionId::Workflows),
            &[BindingId::WorkflowsViewRunDetails, BindingId::TableSelect],
        ),
        ChainLink::new(Owner::App, &[BindingId::AppQuit]),
    ];
    let active = active_bindings(&chain, &Keymap::defaults(), |_, _| BindingState::Enabled);
    assert_eq!(joined(&active), " ⏎ View Run Details  q Quit");
}

#[test]
fn footer_styles() {
    // ` q ` in the key style, `Quit ` in the description style, the rest of the row on `$footer-background`.
    let theme = Theme::default();
    let items = [quit_item(true)];
    let mut terminal = Terminal::new(TestBackend::new(12, 1)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Footer {
                    items: &items,
                    hovered: None,
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
            items: &[quit_item(enabled)],
            hovered: None,
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
    let items = footer_items(&app.app_mut().active_bindings());
    let theme = Theme::default();
    let footer = Footer {
        items: &items,
        hovered: None,
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
    let screen = app.screen(120, 30);
    assert!(!screen.contains("Keybindings"), "{screen}");
    // Its old cells take no clicks: this would have been `q Quit`'s place.
    app.click(60, 29);
    assert!(!app.app().should_quit());
}

fn help_item() -> FooterItem {
    FooterItem {
        owner: Owner::App,
        id: BindingId::AppOpenHelp,
        key: "?".into(),
        description: "Keybindings",
        enabled: true,
    }
}

#[test]
fn footer_hovered_styles() {
    // ` q Quit ` wholly on `$block-hover-background` over the bar, keeping its colors; ` ? Keybindings ` as before.
    let theme = Theme::default();
    let items = [quit_item(true), help_item()];
    let mut terminal = Terminal::new(TestBackend::new(26, 1)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Footer {
                    items: &items,
                    hovered: Some(0),
                    theme: &theme,
                },
                frame.area(),
            );
        })
        .unwrap();
    insta::assert_debug_snapshot!(terminal.backend().buffer());
}

/// A disabled item under the mouse is both dim and highlighted: Textual applies both rules.
#[test]
fn a_hovered_disabled_item_is_dim_and_highlighted() {
    let theme = Theme::default();
    let draw = |hovered| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        Footer {
            items: &[quit_item(false)],
            hovered,
            theme: &theme,
        }
        .render(buf.area, &mut buf);
        buf
    };
    let (plain, hovered) = (draw(None), draw(Some(0)));
    assert_ne!(hovered[(1, 0)].bg, plain[(1, 0)].bg);
    assert!(hovered[(1, 0)].modifier.contains(Modifier::DIM));
}

fn move_mouse(app: &mut TestApp, column: u16, row: u16) {
    app.send(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

/// The columns of the item `label` in the last row of `buffer`, its padding included.
fn item_columns(buffer: &Buffer, label: &str) -> std::ops::Range<u16> {
    let screen = buffer_to_string(buffer);
    let row = screen.lines().last().unwrap();
    let byte = row.find(&format!(" {label} ")).unwrap();
    let start = u16::try_from(row[..byte].chars().count()).unwrap();
    start..start + u16::try_from(label.chars().count()).unwrap() + 2
}

#[test]
fn the_item_under_the_mouse_is_highlighted() {
    let mut app = TestApp::new(Config::default());
    app.press(&["5"]);
    let before = app.render(100, 30);
    let maximize = item_columns(&before, "M Maximize");
    let item = before[(maximize.start, 29)].bg;
    let (key, description) = (before[(maximize.start, 29)].fg, before[(maximize.end - 2, 29)].fg);

    // Anywhere on the item, key or description, lights the whole item and nothing else.
    for column in [maximize.start, maximize.end - 1] {
        move_mouse(&mut app, column, 29);
        let after = app.render(100, 30);
        let hover = after[(maximize.start, 29)].bg;
        assert_ne!(hover, item);
        for x in maximize.clone() {
            assert_eq!(after[(x, 29)].bg, hover, "column {x}");
        }
        assert_eq!(after[(maximize.start, 29)].fg, key);
        assert_eq!(after[(maximize.end - 2, 29)].fg, description);
        for x in (0..100).filter(|x| !maximize.contains(x)) {
            assert_eq!(after[(x, 29)], before[(x, 29)], "column {x}");
        }
    }

    // Off the footer, nothing is highlighted.
    move_mouse(&mut app, maximize.start, 10);
    assert_eq!(app.render(100, 30), before);
}

/// Hover is checked against each frame's layout: when focus changes the items, the one now under the mouse lights
/// up.
#[test]
fn the_hover_follows_the_layout() {
    let mut app = TestApp::new(Config::default());
    let repos = app.render(160, 48);
    let search = item_columns(&repos, "/ Search");
    move_mouse(&mut app, search.start, 47);
    let hovered = app.render(160, 48)[(search.start, 47)].bg;
    assert_ne!(hovered, repos[(search.start, 47)].bg);
    // On Details the first item is `^v Paste link`, under the same cell.
    app.press(&["5"]);
    let details = app.render(160, 48);
    let paste = item_columns(&details, "^v Paste link");
    assert!(paste.contains(&search.start));
    assert_eq!(details[(paste.start, 47)].bg, hovered);
}
