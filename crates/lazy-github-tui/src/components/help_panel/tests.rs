use lazy_github_core::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::component::{BindingState, ChainLink, Owner, ScreenKind};
use crate::keymap::{BindingId, Keymap, active_bindings};
use crate::screens::main::SectionId;
use crate::test_util::TestApp;

/// The panel's lines as plain text, trailing blanks trimmed, and the trailing blank rows dropped.
fn text(lines: &[Line<'_>]) -> String {
    let mut rows: Vec<String> = lines
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows.join("\n")
}

fn sections_after(config: Config, keys: &[&str]) -> Vec<HelpSection> {
    let mut app = TestApp::new(config);
    app.press(keys);
    help_sections(&app.app_mut().active_bindings())
}

fn notifications_on() -> Config {
    let mut config = Config::default();
    config.notifications.enabled = true;
    config
}

/// Python's panel at 160x110, where it fits without a scrollbar, 49 columns inside the border and padding. With
/// notifications on, so B7 doesn't drop `^n`; B14 is applied.
const PYTHON_REPOS: &str = "
        ↑ k Cursor up
        ↓ j Cursor down
          ← Cursor left
          → Cursor right
       home Home
        end End
     pgup K Page up
     pgdn J Page down
    ^pgup ^ Page Left
    ^pgdn $ Page Right
    ⏎ space Select table entry
    ^home g Top
     ^end G Bottom
          l Table scroll right
          h Table scroll left

          / Search Search in the table

         ^f Toggle Favorite Mark/unmark a
            repository as favorited, which are
            displayed at the top of the repos
            table
          O Lookup Repository
          u View User Open the GitHub profile
            for the author/owner of the
            highlighted row

          I New Issue Open a new issue in the
            current repository
          P New PR Open a new pull request in
            the current repository

          1 Focus repos table Focus the table of
            accessible repositories
          2 Focus PRs table Focus the table of
            pull requests for the repo
          3 Focus issues table Focus the table
            of issues for the repo
          4 Focus workflows table Focus and swap
            between the workflow tabs
          5 Focus details tabs Focus and swap
            between the selection detail tabs on
            the right
          6 Focus command log Focus the command
            log, where actions taken by
            LazyGithub are shown

        tab Focus Next
  shift+tab Focus Previous
 ^c super+c Copy selected text
         ^n Notifications Open the notifications
            modal
         ^v Paste link
          f Follow link Show hints on visible
            links and pick one
          F Follow link (browser) Show hints on
            visible links and open the pick in
            the system browser

       ^q q Quit Quit the app and return to the
            command prompt.
         ^p Command Palette
          M Maximize Fullscreen the currently
            focused element
          ? Keybindings Toggle this window";

#[test]
fn help_width_is_a_third_clamped() {
    assert_eq!(help_width(160), 52);
    assert_eq!(help_width(100), 33);
    assert_eq!(help_width(80), 30);
    assert_eq!(help_width(200), 60);
    assert_eq!(help_width(20), 20);
}

#[test]
fn the_repos_table_matches_python() {
    let sections = sections_after(notifications_on(), &[]);
    assert_eq!(text(&help_lines(&sections, 49, &HelpStyles::default())), PYTHON_REPOS);
}

#[test]
fn b7_hides_notifications_while_disabled() {
    let lines = help_lines(&sections_after(Config::default(), &[]), 49, &HelpStyles::default());
    let expected = PYTHON_REPOS.replace(
        "\n         ^n Notifications Open the notifications\n            modal",
        "",
    );
    assert_eq!(text(&lines), expected);
}

#[test]
fn details_and_content_list_their_own_bindings_first() {
    let first_section = |keys: &[&str]| -> Vec<String> {
        let sections = sections_after(Config::default(), keys);
        sections[0]
            .rows
            .iter()
            .map(|row| format!("{} {}", row.keys.join(" "), row.description))
            .collect()
    };
    assert_eq!(first_section(&["5"]), ["← Previous tab", "→ Next tab"]);
    assert_eq!(
        first_section(&["5", "tab"]),
        [
            "↑ Scroll Up",
            "↓ Scroll Down",
            "← Scroll Left",
            "→ Scroll Right",
            "home Scroll Home",
            "end Scroll End",
            "pgup Page Up",
            "pgdn Page Down",
            "^pgup Page Left",
            "^pgdn Page Right"
        ]
    );
    // The details links: the tabs, the focus keys, the screen and the app.
    assert_eq!(sections_after(Config::default(), &["5"]).len(), 4);
}

#[test]
fn hidden_groups_are_absent_and_unshown_ones_present() {
    let chain = [
        ChainLink {
            owner: Owner::Screen(ScreenKind::Main),
            bindings: &[BindingId::NotificationsOpen, BindingId::TextualFocusNext],
        },
        ChainLink {
            owner: Owner::App,
            bindings: &[BindingId::TextualHelpQuit, BindingId::AppQuit],
        },
    ];
    let state = |_: Owner, id: BindingId| match id {
        BindingId::NotificationsOpen => BindingState::Hidden,
        _ => BindingState::Enabled,
    };
    let sections = help_sections(&active_bindings(&chain, &Keymap::defaults(), state));
    // `tab` is `show = false` and listed; `^n` is hidden; `^c` is a system binding.
    assert_eq!(
        text(&help_lines(&sections, 40, &HelpStyles::default())),
        "\n tab Focus Next\n\n   q Quit Exit LazyGithub"
    );
}

/// `" , "` leaves `app.open_help` on the key `""`: Textual's panel still lists "Keybindings", with no key (#158).
#[test]
fn a_comma_only_override_is_listed_with_no_key() {
    let mut config = Config::default();
    config.bindings.overrides.insert("app.open_help".into(), " , ".into());
    let rows: Vec<HelpRow> = sections_after(config, &[])
        .into_iter()
        .flat_map(|section| section.rows)
        .filter(|row| row.description == "Keybindings")
        .collect();
    assert_eq!(
        rows,
        [HelpRow {
            keys: vec![String::new()],
            description: "Keybindings",
            tooltip: "Toggle this window",
        }]
    );
}

#[test]
fn one_action_is_one_row_per_link() {
    // The workflow runs table: `workflows.view_run_details` and `common.table.select` are both `select_cursor`.
    let chain = [ChainLink {
        owner: Owner::Table(SectionId::Workflows),
        bindings: &[BindingId::WorkflowsViewRunDetails, BindingId::TableSelect],
    }];
    let sections = help_sections(&active_bindings(&chain, &Keymap::defaults(), |_, _| {
        BindingState::Enabled
    }));
    assert_eq!(
        text(&help_lines(&sections, 40, &HelpStyles::default())),
        "\n ⏎ space View Run Details"
    );
}

#[test]
fn long_text_wraps_with_a_hanging_indent() {
    let section = HelpSection {
        title: None,
        rows: vec![
            HelpRow {
                keys: vec!["^f".into()],
                description: "Toggle Favorite",
                tooltip: "Mark/unmark a repository as favorited,",
            },
            HelpRow {
                keys: vec!["x".into()],
                description: "Supercalifragilistic",
                tooltip: "",
            },
        ],
    };
    // 2 key columns plus 3 leave 10 for the text; the long word is broken.
    assert_eq!(
        text(&help_lines(&[section], 15, &HelpStyles::default())),
        "
 ^f Toggle
    Favorite
    Mark/unmar
    k a
    repository
    as
    favorited,
  x Supercalif
    ragilistic"
    );
}

#[test]
fn a_group_title_heads_its_section() {
    let sections = [HelpSection {
        title: Some("Input"),
        rows: vec![HelpRow {
            keys: vec!["←".into()],
            description: "Move cursor left",
            tooltip: "",
        }],
    }];
    assert_eq!(
        text(&help_lines(&sections, 30, &HelpStyles::default())),
        "\n   Input\n ← Move cursor left"
    );
}

/// The panel column of each screen row: the right `help_width` cells.
fn panel(keys: &[&str], cols: u16, rows: u16) -> String {
    let mut app = TestApp::new(Config::default());
    app.press(keys);
    let buffer = app.render(cols, rows);
    let x = cols - help_width(cols);
    (0..rows)
        .map(|y| {
            (x..cols)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn help_panel_repos_160x48() {
    insta::assert_snapshot!(panel(&["?"], 160, 48));
}

#[test]
fn help_panel_details_100x30() {
    insta::assert_snapshot!(panel(&["?", "5"], 100, 30));
}

#[test]
fn help_panel_styles() {
    // Border, keys, description and tooltip, on the screen background; a heading row too.
    let theme = Theme::default();
    let sections = [
        HelpSection {
            title: Some("Input"),
            rows: Vec::new(),
        },
        HelpSection {
            title: None,
            rows: vec![HelpRow {
                keys: vec!["/".into()],
                description: "Search",
                tooltip: "Search in the table",
            }],
        },
    ];
    let mut terminal = Terminal::new(TestBackend::new(34, 5)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                HelpPanel {
                    sections: &sections,
                    theme: &theme,
                },
                frame.area(),
            );
        })
        .unwrap();
    insta::assert_debug_snapshot!(terminal.backend().buffer());
}

#[test]
fn an_overflowing_panel_makes_room_for_a_scrollbar() {
    // At 160x48 the repos panel is taller than the screen: Python's text wraps at 34 columns, not 36.
    let screen = panel(&["?"], 160, 48);
    assert!(
        screen.contains("\n▏          ^f Toggle Favorite Mark/unmark a\n"),
        "{screen}"
    );
    assert!(
        screen.contains("\n▏             repository as favorited, which are\n"),
        "{screen}"
    );
    // At 160x110 it fits, and there's no scrollbar.
    let screen = panel(&["?"], 160, 110);
    assert!(
        screen.contains("\n▏           1 Focus repos table Focus the table of\n"),
        "{screen}"
    );
}
