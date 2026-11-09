//! The Settings modal: opening from the palette, the tabs, the widgets, the focus ring, list fields, Save's collected
//! config and Cancel.

use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyModifiers};
use lazy_github_core::config::{IssueOwnerFilter, IssueStateFilter, MergeMethod};
use ratatui::style::Color;

use super::*;
use crate::components::footer::footer_items;
use crate::keymap::KeySpec;
use crate::test_util::TestApp;
use crate::toast::Severity;

/// The tabs, by index.
const APPEARANCE: usize = 0;
const REPOSITORIES: usize = 3;
const PULL_REQUESTS: usize = 4;
const ISSUES: usize = 5;
const CACHE: usize = 6;
const CORE: usize = 7;
const API: usize = 8;

/// The default config as loading gives it: with its paths, which Save won't take blank.
fn config() -> Config {
    let mut config = Config::default();
    config.cache.cache_directory = "/home/me/.config/lazy-github/.cache".into();
    config.core.logfile_path = "/home/me/.config/lazy-github/lazy_github.log".into();
    config
}

/// What the modal closed with, once it has.
type Outcome = Arc<Mutex<Option<SettingsOutcome>>>;

/// Types `text` a key per char.
fn type_text(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
        app.press(&[key.as_str()]);
    }
}

/// An app with the modal opened from the palette, as a user opens it.
fn from_palette(config: Config) -> TestApp {
    let mut app = TestApp::new(config);
    app.press(&["ctrl+p"]);
    type_text(&mut app, "change settings");
    app.press(&["enter"]);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    app
}

/// An app with the modal open over the main screen, whose outcome lands in the returned slot. Drawn once at 160x48,
/// so the panes have a layout.
fn opened(config: &Config) -> (TestApp, Outcome) {
    let mut app = TestApp::new(config.clone());
    let outcome: Outcome = Arc::default();
    let slot = Arc::clone(&outcome);
    app.with_cx(|cx| {
        cx.open_dialog(SettingsDialog::new(config), move |closed| {
            *slot.lock().expect("not poisoned") = Some(closed);
            Action::Consumed
        });
    });
    app.render(160, 48);
    (app, outcome)
}

fn dialog(app: &TestApp) -> &SettingsDialog {
    app.app().top_dialog::<SettingsDialog>().expect("the modal is open")
}

fn focused(app: &TestApp) -> Option<SettingsFocus> {
    dialog(app).ring.focused()
}

fn active_tab(app: &TestApp) -> usize {
    dialog(app).tabs.active()
}

/// Switches to tab `index` with `right` from the strip, which has focus.
fn go_to_tab(app: &mut TestApp, index: usize) {
    assert_eq!(focused(app), Some(SettingsFocus::Tabs));
    for _ in 0..index {
        app.press(&["right"]);
    }
    assert_eq!(active_tab(app), index);
    app.render(160, 48);
}

/// Presses `tab` until `stop` has focus.
fn tab_to(app: &mut TestApp, stop: SettingsFocus) {
    tab_to_at(app, stop, 160, 48);
}

/// [`tab_to`], drawing at `width` x `height` after each press.
fn tab_to_at(app: &mut TestApp, stop: SettingsFocus, width: u16, height: u16) {
    for _ in 0..30 {
        if focused(app) == Some(stop) {
            return;
        }
        app.press(&["tab"]);
        app.render(width, height);
    }
    panic!("{stop:?} never got focus");
}

/// The focus stops `tab` visits from the strip until it's back there.
fn ring_forward(app: &mut TestApp) -> Vec<SettingsFocus> {
    let mut seen = vec![focused(app).expect("something is focused")];
    loop {
        app.press(&["tab"]);
        let stop = focused(app).expect("something is focused");
        if stop == seen[0] {
            return seen;
        }
        seen.push(stop);
    }
}

fn footer(app: &mut TestApp) -> Vec<String> {
    footer_items(&app.app_mut().active_bindings())
        .iter()
        .map(|item| format!("{} {}", item.key, item.description))
        .collect()
}

fn saved(outcome: &Outcome) -> Config {
    match outcome.lock().expect("not poisoned").take() {
        Some(SettingsOutcome::Saved(config)) => *config,
        other => panic!("expected a save, got {other:?}"),
    }
}

fn is_cancelled(outcome: &Outcome) -> bool {
    matches!(
        outcome.lock().expect("not poisoned").as_ref(),
        Some(SettingsOutcome::Cancelled)
    )
}

fn widget(app: &TestApp, tab: usize, field: usize) -> &FieldWidget {
    &dialog(app).panes[tab].fields[field].widget
}

// Opening.

#[test]
fn the_palette_opens_the_modal_with_the_strip_focused_and_the_main_screen_under_it() {
    let mut app = from_palette(config());
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
    assert_eq!(active_tab(&app), APPEARANCE);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "q Close", "/ Search"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("LazyGithub Settings"));
    assert!(screen.contains("Appearance  Notifications  Key Bindings  Repositories  Pull Requests  Issues  Cache"));
    // `escape` closes it, and the main screen's focus is where it was.
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(!app.screen(160, 48).contains("LazyGithub Settings"));
}

#[test]
fn each_opening_reads_the_config_afresh() {
    let mut config = config();
    config.cache.default_ttl = 42;
    let mut app = from_palette(config);
    match widget(&app, CACHE, 3) {
        FieldWidget::Text(input) => assert_eq!(input.value(), "42"),
        other => panic!("Default Ttl is a text input, not {other:?}"),
    }
    app.press(&["escape"]);
    app.with_cx(|cx| cx.config_mut().cache.default_ttl = 7);
    app.press(&["ctrl+p"]);
    type_text(&mut app, "change settings");
    app.press(&["enter"]);
    match widget(&app, CACHE, 3) {
        FieldWidget::Text(input) => assert_eq!(input.value(), "7"),
        other => panic!("Default Ttl is a text input, not {other:?}"),
    }
}

#[test]
fn the_theme_field_shows_the_configs_theme_not_the_sessions() {
    // #192: the palette's theme is for the session only and saves nothing.
    let mut app = TestApp::new(config());
    app.press(&["ctrl+p"]);
    type_text(&mut app, "theme");
    app.press(&["enter"]);
    type_text(&mut app, "nord");
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(app.app().theme().name, "nord");
    assert_eq!(app.app().config().appearance.theme, "dark");
    app.press(&["ctrl+p"]);
    type_text(&mut app, "change settings");
    app.press(&["enter"]);
    assert_eq!(theme_value(&app), Some("dark"));
    assert!(app.screen(160, 48).contains("Dark"));
}

/// The text in the theme select's box: the row under its label's, which is the box's top edge.
fn theme_box(app: &mut TestApp) -> String {
    let screen = app.screen(160, 48);
    let lines: Vec<&str> = screen.lines().collect();
    let label = lines
        .iter()
        .position(|line| line.contains("Theme:"))
        .expect("the theme row");
    lines[label + 1].to_owned()
}

/// The theme select's value.
fn theme_value(app: &TestApp) -> Option<&'static str> {
    match widget(app, APPEARANCE, 0) {
        FieldWidget::Select(select) => select.value().copied(),
        other => panic!("Theme is a select, not {other:?}"),
    }
}

/// Python's name for the default theme selects the theme it's an alias of, and an untouched select leaves the name as
/// written, so the Python app reading the file still knows it (app-owned-themes D2).
#[test]
fn an_alias_shows_its_theme_and_survives_save() {
    let mut config = config();
    config.appearance.theme = "textual-dark".into();
    let (mut app, outcome) = opened(&config);
    assert_eq!(theme_value(&app), Some("dark"));
    let row = theme_box(&mut app);
    assert!(row.contains("Dark") && !row.contains("Textual"), "{row}");
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).appearance.theme, "textual-dark");
}

/// Picking its own theme again doesn't count as a change either.
#[test]
fn picking_an_aliases_theme_keeps_the_alias() {
    let mut config = config();
    config.appearance.theme = "textual-light".into();
    let (mut app, outcome) = opened(&config);
    open_theme_list(&mut app);
    app.press(&["enter"]);
    assert_eq!(theme_value(&app), Some("light"));
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).appearance.theme, "textual-light");
}

#[test]
fn picking_another_theme_writes_its_name() {
    let mut config = config();
    config.appearance.theme = "textual-dark".into();
    let (mut app, outcome) = opened(&config);
    open_theme_list(&mut app);
    type_text(&mut app, "nord");
    app.press(&["enter"]);
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).appearance.theme, "nord");
}

/// A name no theme has shows the blank option, and Save leaves it as written (app-owned-themes D5).
#[test]
fn an_unknown_theme_shows_the_blank_and_survives_save() {
    let mut config = config();
    config.appearance.theme = "no-such-theme".into();
    let (mut app, outcome) = opened(&config);
    assert_eq!(theme_value(&app), None);
    let row = theme_box(&mut app);
    assert!(row.contains("Select"), "{row}");
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).appearance.theme, "no-such-theme");
}

// Widgets.

#[test]
fn every_field_gets_the_widget_for_its_kind() {
    let (app, _) = opened(&config());
    let kinds: Vec<Vec<&str>> = dialog(&app)
        .panes
        .iter()
        .map(|pane| {
            pane.fields
                .iter()
                .map(|field| match &field.widget {
                    FieldWidget::Switch(_) => "switch",
                    FieldWidget::Select(select) if select.is_disabled() => "disabled select",
                    FieldWidget::Select(_) => "select",
                    FieldWidget::Text(_) => "text",
                    FieldWidget::List(_) => "list",
                })
                .collect()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            vec!["select", "switch", "switch", "switch", "switch", "select"],
            vec!["switch", "switch"],
            vec![],
            vec!["list", "list"],
            vec!["select", "select", "select", "list", "switch", "text"],
            vec!["select", "select"],
            vec!["text", "text", "text", "text", "text", "text"],
            vec!["switch", "text", "text", "text"],
            vec!["text", "disabled select"],
        ]
    );
}

#[test]
fn selects_list_the_blank_first_and_client_type_only_its_value() {
    let (app, _) = opened(&config());
    let FieldWidget::Select(theme) = widget(&app, APPEARANCE, 0) else {
        panic!("Theme is a select");
    };
    assert_eq!(theme.labels().take(3).collect::<Vec<_>>(), ["Dark", "Light", "Nord"]);
    assert_eq!(theme.labels().count(), 21);
    let FieldWidget::Select(merge) = widget(&app, PULL_REQUESTS, 2) else {
        panic!("Preferred Merge Method is a select");
    };
    assert_eq!(merge.labels().collect::<Vec<_>>(), ["Merge", "Squash", "Rebase"]);
    assert_eq!(merge.value(), Some(&"squash"));
    let FieldWidget::Select(client) = widget(&app, API, 1) else {
        panic!("Client Type is a select");
    };
    assert_eq!(client.labels().collect::<Vec<_>>(), ["Github_Cli"]);
    assert!(client.is_disabled());
}

#[test]
fn the_template_input_has_its_placeholder() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, PULL_REQUESTS);
    tab_to(&mut app, SettingsFocus::Field(5));
    let screen = app.screen(160, 48);
    assert!(screen.contains("Pull Request Template:"), "{screen}");
    assert!(screen.contains("Enter file path..."), "{screen}");
}

// Tabs and focus.

#[test]
fn left_and_right_switch_tabs_and_wrap() {
    let (mut app, _) = opened(&config());
    app.press(&["left"]);
    assert_eq!(active_tab(&app), API);
    app.press(&["right"]);
    assert_eq!(active_tab(&app), APPEARANCE);
    app.press(&["right", "right"]);
    assert_eq!(active_tab(&app), 2);
    // `enter` on the strip does nothing.
    app.press(&["enter"]);
    assert_eq!(active_tab(&app), 2);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
}

#[test]
fn a_click_on_a_label_switches_tabs() {
    let (mut app, _) = opened(&config());
    let screen = app.screen(160, 48);
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Pull Requests  Issues"))
        .expect("the strip is drawn");
    let at = line.find("Pull Requests  Issues").expect("the strip is drawn") + "Pull Requests  ".len();
    let column = u16::try_from(line[..at].chars().count()).expect("on screen");
    app.click(column, u16::try_from(row).expect("on screen"));
    assert_eq!(active_tab(&app), ISSUES);
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
}

#[test]
fn the_ring_holds_the_active_tabs_fields_and_wraps_both_ways() {
    use SettingsFocus::*;
    let (mut app, _) = opened(&config());
    assert_eq!(
        ring_forward(&mut app),
        [
            Tabs,
            Scroll,
            Field(0),
            Field(1),
            Field(2),
            Field(3),
            Field(4),
            Field(5),
            Save,
            Cancel
        ]
    );
    app.press(&["shift+tab"]);
    assert_eq!(focused(&app), Some(Cancel));
    app.press(&["shift+tab"]);
    assert_eq!(focused(&app), Some(Save));
    app.press(&["tab", "tab"]);
    assert_eq!(focused(&app), Some(Tabs));

    // Switching tabs from the strip changes the fields: a list field is one stop (#343).
    go_to_tab(&mut app, REPOSITORIES);
    assert_eq!(ring_forward(&mut app), [Tabs, Scroll, Field(0), Field(1), Save, Cancel]);
    // The Key Bindings tab has its list, one stop, instead of fields.
    app.press(&["left"]);
    assert_eq!(ring_forward(&mut app), [Tabs, Scroll, KeyList, Save, Cancel]);
    // Api's disabled select is skipped.
    app.press(&["left", "left", "left"]);
    assert_eq!(active_tab(&app), API);
    assert_eq!(ring_forward(&mut app), [Tabs, Scroll, Field(0), Save, Cancel]);
    // And Field(1) isn't disabled on another tab.
    app.press(&["right"]);
    assert_eq!(
        ring_forward(&mut app),
        [
            Tabs,
            Scroll,
            Field(0),
            Field(1),
            Field(2),
            Field(3),
            Field(4),
            Field(5),
            Save,
            Cancel
        ]
    );
}

#[test]
fn a_focused_field_below_the_fold_is_scrolled_into_view() {
    let (mut app, _) = opened(&config());
    let screen = app.screen(100, 30);
    assert!(!screen.contains("Show Issues:"));
    app.press(&["tab", "tab", "tab", "tab", "tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(3)));
    let screen = app.screen(100, 30);
    assert!(screen.contains("Show Issues:"), "{screen}");
    // Textual centers the field: the same offset Python scrolls to.
    assert_eq!(dialog(&app).panes[APPEARANCE].scroll.offset(), 18);
    // Each tab keeps its own scroll position.
    app.press(&["shift+tab", "shift+tab", "shift+tab", "shift+tab", "shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
    let offset = dialog(&app).panes[APPEARANCE].scroll.offset();
    app.press(&["right"]);
    app.screen(100, 30);
    assert_eq!(dialog(&app).panes[1].scroll.offset(), 0);
    app.press(&["left"]);
    app.screen(100, 30);
    assert_eq!(dialog(&app).panes[APPEARANCE].scroll.offset(), offset);
}

// Keys.

#[test]
fn the_rules_top_margin_leaves_the_core_tab_one_row_to_scroll() {
    // The scroll area stops a blank row above the rule, so at 160x48 the Core tab overflows by one row, as Python's
    // does.
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, CORE);
    tab_to(&mut app, SettingsFocus::Scroll);
    app.press(&["end"]);
    app.render(160, 48);
    assert_eq!(dialog(&app).panes[CORE].scroll.offset(), 1);
}

#[test]
fn q_closes_from_a_switch_but_types_in_an_input_and_escape_closes_from_an_input() {
    let (mut app, outcome) = opened(&config());
    tab_to(&mut app, SettingsFocus::Field(1));
    app.press(&["q"]);
    assert!(app.app().modals().is_empty());
    assert!(is_cancelled(&outcome));

    let (mut app, outcome) = opened(&config());
    go_to_tab(&mut app, CACHE);
    tab_to(&mut app, SettingsFocus::Field(3));
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close", "/ Search"]);
    // The input selected its text on focus, so typing replaces it; `q` and `/` are typed.
    app.press(&["q", "/"]);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    match widget(&app, CACHE, 3) {
        FieldWidget::Text(input) => assert_eq!(input.value(), "q/"),
        other => panic!("Default Ttl is a text input, not {other:?}"),
    }
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
    assert!(is_cancelled(&outcome));
}

#[test]
fn a_switch_toggles_with_enter_and_space() {
    let (mut app, _) = opened(&config());
    tab_to(&mut app, SettingsFocus::Field(1));
    let value = |app: &TestApp| match widget(app, APPEARANCE, 1) {
        FieldWidget::Switch(switch) => switch.value(),
        other => panic!("Show Command Log is a switch, not {other:?}"),
    };
    let before = value(&app);
    app.press(&["enter"]);
    assert_eq!(value(&app), !before);
    app.press(&["space"]);
    assert_eq!(value(&app), before);
}

#[test]
fn an_open_select_takes_escape_before_the_modal() {
    let (mut app, _) = opened(&config());
    tab_to(&mut app, SettingsFocus::Field(0));
    app.press(&["enter"]);
    let is_open = |app: &TestApp| matches!(widget(app, APPEARANCE, 0), FieldWidget::Select(s) if s.is_open());
    assert!(is_open(&app));
    app.press(&["escape"]);
    assert!(!is_open(&app));
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
}

// List fields (#343): one list editor per field.

fn list(app: &TestApp, tab: usize, field: usize) -> &ListEditor {
    match widget(app, tab, field) {
        FieldWidget::List(list) => list,
        other => panic!("field {field} of tab {tab} is a list, not {other:?}"),
    }
}

fn list_items(app: &TestApp, tab: usize, field: usize) -> Vec<String> {
    list(app, tab, field).items().to_vec()
}

fn last_toast(app: &TestApp) -> (Option<String>, String) {
    let toast = app.app().toasts().iter().last().cloned().expect("a toast");
    assert_eq!(toast.severity, Severity::Error);
    (toast.title, toast.message.to_string())
}

/// The modal on Repositories with Favorites (`a/one`, `a/two`) focused, its first item highlighted.
fn on_favorites() -> (TestApp, Outcome) {
    let mut config = config();
    config.repositories.favorites = vec!["a/one".into(), "a/two".into()];
    let (mut app, outcome) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    (app, outcome)
}

#[test]
fn a_list_field_is_one_stop_with_its_first_line_highlighted_and_remembered() {
    let (mut app, _) = on_favorites();
    assert_eq!(list(&app, REPOSITORIES, 1).highlighted(), 0);
    app.press(&["down", "tab", "shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(1)));
    assert_eq!(list(&app, REPOSITORIES, 1).highlighted(), 1);
}

#[test]
fn the_footer_offers_the_editors_keys_and_done_and_cancel_while_editing() {
    let (mut app, _) = on_favorites();
    assert_eq!(
        footer(&mut app),
        [
            "⏎ Edit",
            "a Add",
            "d Remove",
            "shift+↑ Move up",
            "shift+↓ Move down",
            "shift+⏎ Submit",
            "q Close",
            "/ Search"
        ]
    );
    app.press(&["enter"]);
    // As for any text input, the footer still shows `/` (Textual's footer doesn't drop the keys an input types).
    assert_eq!(footer(&mut app), ["⏎ Done", "esc Cancel", "shift+⏎ Submit", "/ Search"]);
}

#[test]
fn editing_types_q_and_slash_and_escape_cancels_without_closing() {
    let (mut app, _) = on_favorites();
    app.press(&["enter"]);
    app.press(&["q", "/"]);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    assert_eq!(list(&app, REPOSITORIES, 1).input_value(), Some("q/"));
    app.press(&["escape"]);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/one", "a/two"]);
    // Browsing again, `q` closes.
    app.press(&["q"]);
    assert!(app.app().modals().is_empty());
}

#[test]
fn enter_edits_adds_and_keeps_adding_until_an_empty_enter() {
    let (mut app, _) = on_favorites();
    app.press(&["enter"]);
    type_text(&mut app, "a/zero");
    app.press(&["enter"]);
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/zero", "a/two"]);
    app.press(&["end", "enter"]);
    type_text(&mut app, "b/1");
    app.press(&["enter"]);
    type_text(&mut app, "b/2");
    app.press(&["enter"]);
    assert!(list(&app, REPOSITORIES, 1).is_editing());
    app.press(&["enter"]);
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/zero", "a/two", "b/1", "b/2"]);
    // `d` removes, `shift+up` moves.
    app.press(&["up", "d"]);
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/zero", "a/two", "b/1"]);
    app.press(&["up", "shift+up"]);
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/zero", "b/1", "a/two"]);
}

#[test]
fn a_duplicate_is_refused_with_a_toast_and_the_input_stays_open() {
    let (mut app, _) = on_favorites();
    app.press(&["a"]);
    type_text(&mut app, "a/two");
    app.press(&["enter"]);
    assert_eq!(
        last_toast(&app),
        (Some("Invalid setting".into()), "Favorites already lists a/two".into())
    );
    assert_eq!(list(&app, REPOSITORIES, 1).input_value(), Some("a/two"));
    // `tab` drops it, with the toast again, and focus moves.
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Save));
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    assert_eq!(app.app().toasts().len(), 2);
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/one", "a/two"]);
}

#[test]
fn tab_from_an_open_input_commits_it_without_a_fresh_one() {
    let (mut app, _) = on_favorites();
    app.press(&["a"]);
    type_text(&mut app, "a/three");
    app.press(&["shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(0)));
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/one", "a/two", "a/three"]);
}

#[test]
fn a_multi_line_paste_adds_its_lines() {
    let (mut app, _) = on_favorites();
    app.send(crate::event::Event::Paste("a/two\nb/1\n\nb/2\n".into()));
    assert_eq!(list_items(&app, REPOSITORIES, 1), ["a/one", "a/two", "b/1", "b/2"]);
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
}

#[test]
fn a_press_highlights_then_edits_and_the_add_line_adds() {
    let (mut app, _) = on_favorites();
    let screen = app.screen(160, 48);
    let (column, row) = cell_of(&screen, "a/two");
    // From another stop: focus and the highlight only.
    tab_to(&mut app, SettingsFocus::Save);
    app.click(column, row);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(1)));
    assert_eq!(list(&app, REPOSITORIES, 1).highlighted(), 1);
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    app.render(160, 48);
    app.click(column, row);
    assert_eq!(list(&app, REPOSITORIES, 1).input_value(), Some("a/two"));
    // Favorites' add line, below: a press there while editing commits and highlights it.
    app.render(160, 48);
    app.click(column, row + 1);
    assert_eq!(list(&app, REPOSITORIES, 1).highlighted(), 2);
    assert!(!list(&app, REPOSITORIES, 1).is_editing());
    app.render(160, 48);
    app.click(column, row + 1);
    assert_eq!(list(&app, REPOSITORIES, 1).input_value(), Some(""));
}

#[test]
fn the_tab_follows_the_highlighted_line_of_a_long_list() {
    let mut config = config();
    config.repositories.favorites = (0..6).map(|i| format!("a/{i}")).collect();
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    app.render(100, 30);
    tab_to_at(&mut app, SettingsFocus::Field(1), 100, 30);
    app.render(100, 30);
    for _ in 0..6 {
        app.press(&["down"]);
        app.render(100, 30);
    }
    let screen = app.screen(100, 30);
    assert!(screen.contains("+ Add item..."), "{screen}");
    // The add line is centered, not the whole field.
    let (_, row) = cell_of(&screen, "+ Add item...");
    let scroll = dialog(&app).panes[REPOSITORIES].scroll.area();
    let middle = scroll.y + scroll.height / 2;
    assert!(row.abs_diff(middle) <= 1, "row {row}, middle {middle}\n{screen}");
}

#[test]
fn both_repository_fields_are_reachable_at_100x30() {
    // An empty field takes three rows: Favorites is right below the fold, and `tab` brings it into view.
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, REPOSITORIES);
    let screen = app.screen(100, 30);
    assert!(screen.contains("Additional Repos To Track:"), "{screen}");
    assert_eq!(screen.matches("+ Add item...").count(), 1, "{screen}");
    tab_to_at(&mut app, SettingsFocus::Field(1), 100, 30);
    let screen = app.screen(100, 30);
    assert!(screen.contains("Favorites:"), "{screen}");
    assert!(screen.contains("+ Add item..."), "{screen}");
}

#[test]
fn the_reviewers_list_adds_usernames() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, PULL_REQUESTS);
    tab_to(&mut app, SettingsFocus::Field(3));
    app.press(&["a"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("username"), "{screen}");
}

// Save.

#[test]
fn saving_untouched_fields_writes_the_config_back_unchanged() {
    let mut config = config();
    config.repositories.favorites = vec!["a/b".into()];
    config.pull_requests.pull_request_template = Some("tmpl.md".into());
    let (mut app, outcome) = opened(&config);
    app.press(&["shift+enter"]);
    assert!(app.app().modals().is_empty());
    assert_eq!(saved(&outcome), config);
}

#[test]
fn save_writes_every_edited_field_through_its_section() {
    let mut config = config();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    config.pull_requests.owner_filter = IssueOwnerFilter::Mine;
    let (mut app, outcome) = opened(&config);
    // A switch.
    tab_to(&mut app, SettingsFocus::Field(2));
    app.press(&["enter"]);
    // The theme.
    app.press(&["shift+tab", "shift+tab", "enter"]);
    type_text(&mut app, "nord");
    app.press(&["enter"]);
    // Issues' state filter, which the Pull Requests one doesn't follow (B1).
    tab_to(&mut app, SettingsFocus::Tabs);
    go_to_tab(&mut app, ISSUES);
    tab_to(&mut app, SettingsFocus::Field(0));
    app.press(&["enter"]);
    type_text(&mut app, "clo");
    app.press(&["enter"]);
    // A number.
    tab_to(&mut app, SettingsFocus::Tabs);
    app.press(&["right"]);
    tab_to(&mut app, SettingsFocus::Field(3));
    type_text(&mut app, "42");
    // A path.
    app.press(&["shift+tab"]);
    app.press(&["ctrl+u"]);
    type_text(&mut app, " /tmp/cache ");
    // Pull Requests: a merge method and a reviewer.
    tab_to(&mut app, SettingsFocus::Tabs);
    app.press(&["left", "left"]);
    tab_to(&mut app, SettingsFocus::Field(2));
    app.press(&["enter"]);
    type_text(&mut app, "reb");
    app.press(&["enter"]);
    tab_to(&mut app, SettingsFocus::Field(3));
    app.press(&["a"]);
    type_text(&mut app, "octocat");
    app.press(&["enter"]);
    tab_to(&mut app, SettingsFocus::Save);
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());

    let mut expected = config;
    expected.appearance.show_workflows = !expected.appearance.show_workflows;
    expected.appearance.theme = "nord".into();
    expected.issues.state_filter = IssueStateFilter::Closed;
    expected.cache.default_ttl = 42;
    expected.cache.cache_directory = "/tmp/cache".into();
    expected.pull_requests.preferred_merge_method = MergeMethod::Rebase;
    expected.pull_requests.additional_suggested_pr_reviewers = vec!["octocat".into()];
    let saved = saved(&outcome);
    assert_eq!(saved, expected);
    assert_eq!(saved.pull_requests.state_filter, IssueStateFilter::Open);
    assert_eq!(saved.pull_requests.owner_filter, IssueOwnerFilter::Mine);
    assert_eq!(saved.issues.owner_filter, IssueOwnerFilter::default());
}

#[test]
fn a_select_on_its_blank_keeps_the_fields_value() {
    let mut config = config();
    config.pull_requests.state_filter = IssueStateFilter::Open;
    let (mut app, outcome) = opened(&config);
    go_to_tab(&mut app, PULL_REQUESTS);
    tab_to(&mut app, SettingsFocus::Field(0));
    app.press(&["enter", "home", "enter"]);
    match widget(&app, PULL_REQUESTS, 0) {
        FieldWidget::Select(select) => assert_eq!(select.value(), None),
        other => panic!("State Filter is a select, not {other:?}"),
    }
    assert!(app.screen(160, 48).contains("Select"));
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome), config);
}

#[test]
fn a_list_saves_its_items_in_the_order_shown() {
    let mut config = config();
    config.repositories.favorites = vec!["a/1".into(), "a/2".into(), "a/3".into()];
    let (mut app, outcome) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    // Remove "a/1", move "a/3" first, then add "a/4".
    app.press(&["d", "down", "shift+up", "a"]);
    type_text(&mut app, "a/4");
    app.press(&["enter", "enter"]);
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).repositories.favorites, ["a/3", "a/2", "a/4"]);
}

#[test]
fn save_commits_an_open_list_input_and_drops_refused_text() {
    let (mut app, outcome) = on_favorites();
    app.press(&["a"]);
    type_text(&mut app, "a/three");
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).repositories.favorites, ["a/one", "a/two", "a/three"]);

    let (mut app, outcome) = on_favorites();
    app.press(&["a"]);
    type_text(&mut app, "a/one");
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).repositories.favorites, ["a/one", "a/two"]);
    assert_eq!(last_toast(&app).1, "Favorites already lists a/one");
}

#[test]
fn an_invalid_field_keeps_the_modal_open_on_it_with_an_error_toast() {
    let (mut app, outcome) = opened(&config());
    go_to_tab(&mut app, CACHE);
    tab_to(&mut app, SettingsFocus::Field(3));
    type_text(&mut app, "ten");
    // Away to another tab, then Save from there.
    tab_to(&mut app, SettingsFocus::Tabs);
    app.press(&["right"]);
    app.press(&["shift+enter"]);
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    assert!(outcome.lock().expect("not poisoned").is_none());
    assert_eq!(active_tab(&app), CACHE);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(3)));
    let toast = app.app().toasts().iter().last().cloned().expect("a toast");
    assert_eq!(toast.severity, Severity::Error);
    assert_eq!(toast.title.as_deref(), Some("Invalid setting"));
    assert_eq!(toast.message.to_string(), "Default Ttl must be a whole number");
}

#[test]
fn cancel_q_and_escape_close_without_saving() {
    let (mut app, outcome) = opened(&config());
    tab_to(&mut app, SettingsFocus::Cancel);
    app.press(&["enter"]);
    assert!(app.app().modals().is_empty());
    assert!(is_cancelled(&outcome));
    for key in ["q", "escape"] {
        let (mut app, outcome) = opened(&config());
        app.press(&[key]);
        assert!(app.app().modals().is_empty(), "{key}");
        assert!(is_cancelled(&outcome), "{key}");
    }
}

#[test]
fn collect_skips_nothing_but_blank_selects() {
    let config = config();
    let dialog = SettingsDialog::new(&config);
    assert_eq!(dialog.collect(&config), Ok(config));
}

// Drawing.

#[test]
fn snapshot_appearance() {
    let (mut app, _) = opened(&config());
    insta::assert_snapshot!("appearance", app.screen(160, 48));
}

/// Appearance scrolled to its last field, Color Depth (Rust-only, decision `color-depth-detection`), focused and then
/// with its list open, at both sizes.
#[test]
fn snapshot_appearance_color_depth() {
    // Each size drawn after every tab, as a focused field is scrolled into view in the last frame's layout.
    let (mut narrow, _) = opened(&config());
    narrow.render(100, 30);
    tab_to_at(&mut narrow, SettingsFocus::Field(5), 100, 30);
    insta::assert_snapshot!("appearance_color_depth_narrow", narrow.screen(100, 30));
    let (mut app, _) = opened(&config());
    tab_to(&mut app, SettingsFocus::Field(5));
    insta::assert_snapshot!("appearance_color_depth", app.screen(160, 48));
    app.press(&["enter"]);
    insta::assert_snapshot!("appearance_color_depth_open", app.screen(160, 48));
    // Down to "256 colors" and Save: the config holds the setting's value.
    let FieldWidget::Select(select) = widget(&app, APPEARANCE, 5) else {
        panic!("Color Depth is a select");
    };
    assert_eq!(
        select.labels().collect::<Vec<_>>(),
        ["Auto", "Truecolor", "256 colors", "16 colors"]
    );
    assert_eq!(select.value(), Some(&"auto"));
}

#[test]
fn color_depth_saves_the_picked_setting() {
    let (mut app, outcome) = opened(&config());
    tab_to(&mut app, SettingsFocus::Field(5));
    app.press(&["enter", "down", "down", "enter"]);
    app.press(&["shift+enter"]);
    assert_eq!(
        saved(&outcome).appearance.color_depth,
        lazy_github_core::config::ColorDepthSetting::EightBit
    );
}

#[test]
fn snapshot_pull_requests() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, PULL_REQUESTS);
    insta::assert_snapshot!("pull_requests", app.screen(160, 48));
}

#[test]
fn snapshot_cache() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, CACHE);
    insta::assert_snapshot!("cache", app.screen(160, 48));
}

#[test]
fn snapshot_narrow() {
    let (mut app, _) = opened(&config());
    insta::assert_snapshot!("narrow", app.screen(100, 30));
    app.press(&["tab", "tab", "tab", "tab", "tab"]);
    insta::assert_snapshot!("narrow_scrolled", app.screen(100, 30));
}

#[test]
fn snapshot_select_open() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, PULL_REQUESTS);
    tab_to(&mut app, SettingsFocus::Field(2));
    app.press(&["enter"]);
    insta::assert_snapshot!("select_open", app.screen(160, 48));
}

/// Opens Appearance's Theme list.
fn open_theme_list(app: &mut TestApp) {
    tab_to(app, SettingsFocus::Field(0));
    app.press(&["enter"]);
}

#[test]
fn snapshot_theme_list_with_a_scrollbar() {
    // More themes than the list's ten rows: Python's `theme-list` capture of `settings-save` at 160x48 has the
    // scrollbar in columns 122-123, the thumb down to a `▃` on row 26, and the options stopping at column 121.
    let (mut app, _) = opened(&config());
    open_theme_list(&mut app);
    let buf = app.render(160, 48);
    let bar: String = (22..32).map(|y| buf[(122, y)].symbol().to_owned()).collect();
    assert_eq!(bar, "    ▃     ");
    assert_eq!(buf[(123, 26)].symbol(), "▃");
    assert_eq!(buf[(122, 22)].fg, Color::Rgb(0x00, 0x30, 0x54));
    assert_eq!(buf[(122, 22)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buf[(122, 26)].bg, Color::Rgb(0x00, 0x00, 0x00));
    assert_eq!(buf[(122, 27)].bg, Color::Rgb(0x00, 0x00, 0x00));
    // The highlighted Dark row ends before the scrollbar; the list's padding and border follow it.
    assert_eq!(buf[(121, 23)].bg, Color::Rgb(0x01, 0x78, 0xd4));
    assert_eq!(buf[(124, 23)].bg, Color::Rgb(0x27, 0x27, 0x27));
    assert_eq!(buf[(125, 23)].symbol(), "▎");
    insta::assert_snapshot!("theme_list_styled", crate::test_util::buffer_to_styled_string(&buf));
}

#[test]
fn snapshot_theme_list_over_the_footer() {
    // At 100x30 the list moves up to fit the screen and its bottom border takes the footer's row, as in Python.
    let (mut app, _) = opened(&config());
    open_theme_list(&mut app);
    let buf = app.render(100, 30);
    assert_eq!(buf[(49, 29)].symbol(), "▊");
    assert_eq!(buf[(50, 29)].symbol(), "▁");
    assert_eq!(buf[(95, 29)].symbol(), "▎");
    insta::assert_snapshot!("theme_list_narrow", app.screen(100, 30));
}

#[test]
fn a_click_on_the_list_over_the_footer_is_the_lists() {
    let (mut app, _) = opened(&config());
    open_theme_list(&mut app);
    let _ = app.render(100, 30);
    let is_open = |app: &TestApp| matches!(widget(app, APPEARANCE, 0), FieldWidget::Select(s) if s.is_open());
    // The bottom border, on the footer's row: the list takes the click, and nothing happens.
    app.click(60, 29);
    assert!(is_open(&app));
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    // Flexoki's row, over the frame's bottom border, outside the dialog's interior: still the list's, and picks it.
    let _ = app.render(100, 30);
    app.click(60, 28);
    assert!(!is_open(&app));
    match widget(&app, APPEARANCE, 0) {
        FieldWidget::Select(select) => assert_eq!(select.value(), Some(&"flexoki")),
        other => panic!("Theme is a select, not {other:?}"),
    }
}

#[test]
fn snapshot_list_editor() {
    // One empty field and one filled, focused.
    let mut config = config();
    config.repositories.favorites = vec!["octo/one".into(), "octo/two".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    app.press(&["down"]);
    insta::assert_snapshot!("list_editor", app.screen(160, 48));
    insta::assert_snapshot!(
        "list_editor_styled",
        crate::test_util::buffer_to_styled_string(&app.render(160, 48))
    );
}

#[test]
fn snapshot_list_editor_editing() {
    let mut config = config();
    config.repositories.favorites = vec!["octo/one".into(), "octo/two".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    app.press(&["down", "enter", "end"]);
    insta::assert_snapshot!("list_editor_editing", app.screen(160, 48));
}

#[test]
fn snapshot_list_editor_adding() {
    let mut config = config();
    config.repositories.favorites = vec!["octo/one".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    app.press(&["a"]);
    insta::assert_snapshot!("list_editor_adding", app.screen(160, 48));
}

#[test]
fn snapshot_list_editor_narrow() {
    let mut config = config();
    config.repositories.favorites = vec!["octo/one".into(), "octo/two".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    app.render(100, 30);
    tab_to_at(&mut app, SettingsFocus::Field(1), 100, 30);
    insta::assert_snapshot!("list_editor_narrow", app.screen(100, 30));
}

#[test]
fn snapshot_api_disabled_select_styled() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, API);
    insta::assert_snapshot!(
        "api_styled",
        crate::test_util::buffer_to_styled_string(&app.render(160, 48))
    );
}

// Search.

/// The field names of tab `tab`'s fields the filter shows.
fn shown(app: &TestApp, tab: usize) -> Vec<&'static str> {
    let pane = &dialog(app).panes[tab];
    pane.fields
        .iter()
        .enumerate()
        .filter(|&(index, _)| pane.is_shown(index))
        .map(|(_, field)| field.field.name)
        .collect()
}

/// The search box's text, while it's shown.
fn search_value(app: &TestApp) -> Option<String> {
    dialog(app).search.as_ref().map(|search| search.value().to_owned())
}

/// Opens the box with `/`, replaces its text with `query` and submits it.
fn search(app: &mut TestApp, query: &str) {
    app.press(&["/"]);
    assert_eq!(focused(app), Some(SettingsFocus::Search));
    app.press(&["ctrl+u"]);
    type_text(app, query);
    app.press(&["enter"]);
    app.render(160, 48);
}

/// The screen cell where `text` starts on the row that holds it.
fn cell_of(screen: &str, text: &str) -> (u16, u16) {
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(text))
        .unwrap_or_else(|| panic!("{text} isn't drawn:\n{screen}"));
    let at = line.find(text).expect("the line holds it");
    (
        u16::try_from(line[..at].chars().count()).expect("on screen"),
        u16::try_from(row).expect("on screen"),
    )
}

#[test]
fn slash_opens_and_focuses_the_box_from_every_stop_but_a_text_input() {
    use SettingsFocus::*;
    let mut config = config();
    config.repositories.favorites = vec!["a/b".into()];
    // The strip, the scroll area, a switch, a select, a list editor, Save and Cancel.
    let stops = [
        (APPEARANCE, Tabs),
        (APPEARANCE, Scroll),
        (APPEARANCE, Field(1)),
        (APPEARANCE, Field(0)),
        (REPOSITORIES, Field(1)),
        (APPEARANCE, Save),
        (APPEARANCE, Cancel),
    ];
    for (tab, stop) in stops {
        let (mut app, _) = opened(&config);
        go_to_tab(&mut app, tab);
        tab_to(&mut app, stop);
        assert!(search_value(&app).is_none(), "{stop:?}");
        app.press(&["/"]);
        assert_eq!(focused(&app), Some(Search), "{stop:?}");
        assert_eq!(search_value(&app).as_deref(), Some(""), "{stop:?}");
        assert_eq!(app.app().modals(), [ModalKind::Settings]);
    }

    // A text input types it: a plain one and a list editor's open one.
    for (tab, index, open) in [(CACHE, 3, None), (REPOSITORIES, 1, Some("a"))] {
        let (mut app, _) = opened(&config);
        go_to_tab(&mut app, tab);
        tab_to(&mut app, Field(index));
        app.press(&open.into_iter().collect::<Vec<_>>());
        app.press(&["ctrl+u", "/"]);
        assert_eq!(focused(&app), Some(Field(index)));
        assert!(search_value(&app).is_none());
        let value = match widget(&app, tab, index) {
            FieldWidget::Text(input) => input.value(),
            FieldWidget::List(list) => list.input_value().expect("an open input"),
            other => panic!("not a text input: {other:?}"),
        };
        assert_eq!(value, "/");
    }
    // And so does the box itself.
    let (mut app, _) = opened(&config);
    app.press(&["/", "a", "/"]);
    assert_eq!(search_value(&app).as_deref(), Some("a/"));
}

#[test]
fn the_box_shows_above_the_strip_with_its_footer() {
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    assert_eq!(footer(&mut app), ["shift+⏎ Submit", "esc Close", "/ Search"]);
    let screen = app.screen(160, 48);
    let (_, heading) = cell_of(&screen, "LazyGithub Settings");
    // As the reference draws it: the box's text 4 rows below the heading and the strip 3 below that.
    assert_eq!(cell_of(&screen, "Search settings...").1, heading + 4);
    assert_eq!(cell_of(&screen, "Appearance  Notifications").1, heading + 7);
    // `q` types; `escape` closes.
    app.press(&["q"]);
    assert_eq!(search_value(&app).as_deref(), Some("q"));
    app.press(&["escape"]);
    assert!(app.app().modals().is_empty());
}

#[test]
fn the_docked_buttons_hide_the_scroll_area_under_their_padding() {
    // With the box shown the scroll area reaches under the buttons; Python's docked row (padding included) hides
    // what's there, so the Appearance tab's labels don't show left of Save.
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    let screen = app.screen(160, 48);
    let (save_x, save_y) = cell_of(&screen, "Save");
    let lines: Vec<&str> = screen.lines().collect();
    let left = cell_of(&screen, "Appearance  Notifications").0;
    for row in save_y - 1..=save_y + 1 {
        let line: String = lines[usize::from(row)]
            .chars()
            .skip(usize::from(left))
            .take(usize::from(save_x - left))
            .collect();
        assert!(
            line.chars().all(|c| c == ' ' || c == '▔' || c == '▁'),
            "row {row}: {line:?}\n{screen}"
        );
    }
}

#[test]
fn nothing_filters_before_enter_and_focus_stays_in_the_box() {
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    type_text(&mut app, "filter");
    assert_eq!(shown(&app, PULL_REQUESTS).len(), 6);
    assert_eq!(shown(&app, APPEARANCE).len(), 6);
    app.press(&["enter"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    assert_eq!(search_value(&app).as_deref(), Some("filter"));
    assert_eq!(shown(&app, APPEARANCE), Vec::<&str>::new());
    assert_eq!(shown(&app, PULL_REQUESTS), ["state_filter", "owner_filter"]);
}

#[test]
fn the_filter_spans_every_tab_and_matches_the_snake_case_name() {
    let (mut app, _) = opened(&config());
    search(&mut app, "filter");
    let all: Vec<Vec<&str>> = (0..dialog(&app).panes.len()).map(|tab| shown(&app, tab)).collect();
    assert_eq!(
        all,
        [
            vec![],
            vec![],
            vec![],
            vec![],
            vec!["state_filter", "owner_filter"],
            vec!["state_filter", "owner_filter"],
            vec![],
            vec![],
            vec![],
        ]
    );
    // The name, not the label: an underscore matches, and so do scattered letters.
    search(&mut app, "command_log");
    assert_eq!(shown(&app, APPEARANCE), ["show_command_log"]);
    search(&mut app, "shcl");
    assert_eq!(shown(&app, APPEARANCE), ["show_command_log"]);
}

#[test]
fn the_query_is_stripped_and_lowercased() {
    let (mut app, _) = opened(&config());
    search(&mut app, "  FiLTer  ");
    assert_eq!(shown(&app, PULL_REQUESTS), ["state_filter", "owner_filter"]);
    // The box keeps what was typed.
    assert_eq!(search_value(&app).as_deref(), Some("  FiLTer  "));
}

#[test]
fn a_blank_query_shows_every_field_again() {
    let (mut app, _) = opened(&config());
    search(&mut app, "filter");
    assert_eq!(shown(&app, CACHE), Vec::<&str>::new());
    search(&mut app, "   ");
    for (tab, pane) in dialog(&app).panes.iter().enumerate() {
        assert_eq!(shown(&app, tab).len(), pane.fields.len(), "{tab}");
    }
}

#[test]
fn the_ring_puts_the_box_first_and_skips_hidden_fields() {
    use SettingsFocus::*;
    let (mut app, _) = opened(&config());
    // Unsubmitted text keeps the box shown as focus goes round, and filters nothing.
    app.press(&["/", "x"]);
    assert_eq!(
        ring_forward(&mut app),
        [
            Search,
            Tabs,
            Scroll,
            Field(0),
            Field(1),
            Field(2),
            Field(3),
            Field(4),
            Field(5),
            Save,
            Cancel
        ]
    );
    app.press(&["ctrl+u"]);
    type_text(&mut app, "filter");
    app.press(&["enter"]);
    assert_eq!(ring_forward(&mut app), [Search, Tabs, Scroll, Save, Cancel]);
    // `tab` leaves for the strip with the filter kept, and the other tabs are filtered too.
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(Tabs));
    app.press(&["right", "right", "right", "right"]);
    assert_eq!(active_tab(&app), PULL_REQUESTS);
    assert_eq!(
        ring_forward(&mut app),
        [Tabs, Scroll, Field(0), Field(1), Save, Cancel, Search]
    );
    let screen = app.screen(160, 48);
    assert!(screen.contains("State Filter:"), "{screen}");
    assert!(screen.contains("Owner Filter:"), "{screen}");
    assert!(!screen.contains("Preferred Merge Method:"), "{screen}");
    assert!(!screen.contains("Pull Request Template:"), "{screen}");
}

#[test]
fn a_tab_with_nothing_left_shows_only_its_docstring() {
    let (mut app, _) = opened(&config());
    search(&mut app, "filter");
    let screen = app.screen(160, 48);
    assert!(
        screen.contains("Settings focused on altering the appearance"),
        "{screen}"
    );
    for label in ["Theme:", "Show Command Log:", "Show Pull Requests:"] {
        assert!(!screen.contains(label), "{label}\n{screen}");
    }
}

#[test]
fn hidden_fields_are_still_saved_as_they_are() {
    let (mut app, outcome) = opened(&config());
    // Edit Show Workflows, then hide it.
    tab_to(&mut app, SettingsFocus::Field(2));
    app.press(&["enter"]);
    search(&mut app, "theme");
    assert_eq!(shown(&app, APPEARANCE), ["theme"]);
    app.press(&["shift+enter"]);
    let mut expected = config();
    expected.appearance.show_workflows = !expected.appearance.show_workflows;
    assert_eq!(saved(&outcome), expected);
}

#[test]
fn the_box_hides_on_blur_when_blank_and_stays_when_not() {
    let (mut app, _) = opened(&config());
    // Only whitespace in it: it hides, and leaves the ring.
    app.press(&["/", "space", "tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
    assert!(search_value(&app).is_none());
    assert!(!app.screen(160, 48).contains("Search settings..."));
    app.press(&["shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Cancel));

    // With text in it, it stays, first in the ring, even unsubmitted.
    app.press(&["/", "x", "shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Cancel));
    assert_eq!(search_value(&app).as_deref(), Some("x"));
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    // `/` from elsewhere focuses it again, its text selected so typing replaces it.
    app.press(&["tab", "/"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    app.press(&["y"]);
    assert_eq!(search_value(&app).as_deref(), Some("y"));

    // Emptied after a filter: it hides and the filter stays.
    search(&mut app, "filter");
    app.press(&["ctrl+u", "tab"]);
    assert!(search_value(&app).is_none());
    assert_eq!(shown(&app, PULL_REQUESTS), ["state_filter", "owner_filter"]);
}

#[test]
fn a_click_focuses_the_box_and_a_click_away_hides_it_when_blank() {
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    type_text(&mut app, "ab");
    app.press(&["tab"]);
    let screen = app.screen(160, 48);
    let (column, row) = cell_of(&screen, "ab ");
    // Past the end of the text, so the cursor lands there and `ctrl+u` clears it all.
    app.click(column + 4, row);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    app.press(&["ctrl+u"]);
    let (column, row) = cell_of(&screen, "Notifications  Key Bindings");
    app.click(column, row);
    assert_eq!(focused(&app), Some(SettingsFocus::Tabs));
    assert_eq!(active_tab(&app), 1);
    assert!(search_value(&app).is_none());
}

#[test]
fn a_paste_lands_in_the_box() {
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    app.send(crate::event::Event::Paste("filter".into()));
    assert_eq!(search_value(&app).as_deref(), Some("filter"));
}

#[test]
fn snapshot_search_pull_requests() {
    let (mut app, _) = opened(&config());
    app.press(&["/"]);
    type_text(&mut app, "filter");
    app.press(&["enter", "tab", "right", "right", "right", "right", "shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    insta::assert_snapshot!("search_pull_requests", app.screen(160, 48));
}

// The Key Bindings list (#194, #381).

const KEY_BINDINGS: usize = 2;

fn key_list(app: &TestApp) -> &key_list::KeyList {
    dialog(app).panes[KEY_BINDINGS].keys.as_ref().expect("the tab's list")
}

fn key_value(app: &TestApp, config_id: &str) -> String {
    let id = BindingId::from_config_id(config_id).expect("a binding");
    key_list(app)
        .rows()
        .iter()
        .find(|row| row.id == id)
        .expect("a row")
        .value
        .clone()
}

fn highlighted_id(app: &TestApp) -> Option<&'static str> {
    key_list(app).highlighted().map(|row| row.config_id)
}

fn is_capturing(app: &TestApp) -> bool {
    key_list(app).is_capturing()
}

/// The modal on the Key Bindings tab with the list focused.
fn on_list(config: &Config) -> (TestApp, Outcome) {
    let (mut app, outcome) = opened(config);
    go_to_tab(&mut app, KEY_BINDINGS);
    tab_to(&mut app, SettingsFocus::KeyList);
    (app, outcome)
}

/// [`on_list`] with "Keybindings" (`app.open_help`, the fourth binding) highlighted.
fn on_help(config: &Config) -> (TestApp, Outcome) {
    let (mut app, outcome) = on_list(config);
    app.press(&["down", "down", "down"]);
    assert_eq!(highlighted_id(&app), Some("app.open_help"));
    (app, outcome)
}

fn written(config: &Config) -> Vec<(&str, &str)> {
    config
        .bindings
        .overrides
        .iter()
        .map(|(id, keys)| (id.as_str(), keys.as_str()))
        .collect()
}

#[test]
fn the_list_is_every_rebindable_binding_in_group_order_with_the_first_highlighted() {
    let (app, _) = on_list(&config());
    let ids: Vec<&str> = key_list(&app).rows().iter().map(|row| row.config_id).collect();
    // Python's 59 ids, the eight Rust-only diff ids, `main.refresh` and `search.open`.
    assert_eq!(ids.len(), 69);
    assert_eq!(
        &ids[..4],
        [
            "app.quit",
            "app.command_palette",
            "app.maximize_widget",
            "app.open_help"
        ]
    );
    assert_eq!(highlighted_id(&app), Some("app.quit"));
}

#[test]
fn a_binding_shows_its_override_as_written() {
    let mut config = config();
    config.bindings.overrides.insert("app.quit".into(), "ctrl+x, f9".into());
    let (app, _) = opened(&config);
    assert_eq!(key_value(&app, "app.quit"), "ctrl+x, f9");
}

#[test]
fn the_footer_offers_change_add_and_reset_and_only_cancel_while_waiting() {
    let (mut app, _) = on_help(&config());
    assert_eq!(
        footer(&mut app),
        [
            "⏎ Change key",
            "a Add key",
            "r Reset",
            "shift+⏎ Submit",
            "q Close",
            "/ Search"
        ]
    );
    app.press(&["enter"]);
    assert!(is_capturing(&app));
    assert_eq!(footer(&mut app), ["esc Cancel"]);
    app.press(&["escape"]);
    assert!(!is_capturing(&app));
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
    assert_eq!(footer(&mut app)[0], "⏎ Change key");
}

#[test]
fn the_list_consumes_keys_only_while_waiting() {
    let (mut app, _) = on_help(&config());
    let keymap = Keymap::defaults();
    let q = KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(!dialog(&app).consumes_key(&q, &keymap));
    app.press(&["enter"]);
    assert!(dialog(&app).consumes_key(&q, &keymap));
}

#[test]
fn waiting_records_every_key_but_escape_and_the_modal_stays_open() {
    let (mut app, outcome) = on_help(&config());
    for (key, name) in [
        ("q", "q"),
        ("/", "slash"),
        ("shift+enter", "shift+enter"),
        ("enter", "enter"),
        ("space", "space"),
        ("tab", "tab"),
        ("shift+tab", "shift+tab"),
        ("ctrl+c", "ctrl+c"),
        ("!", "exclamation_mark"),
    ] {
        app.press(&["enter", key]);
        assert_eq!(app.app().modals(), [ModalKind::Settings], "{key} closed the modal");
        assert_eq!(focused(&app), Some(SettingsFocus::KeyList), "{key} moved focus");
        assert!(!is_capturing(&app));
        assert_eq!(key_value(&app, "app.open_help"), name);
    }
    assert!(dialog(&app).search.is_none());
    assert!(outcome.lock().expect("not poisoned").is_none());
}

#[test]
fn add_appends_a_key_and_reset_restores_the_defaults() {
    let (mut app, _) = on_help(&config());
    app.press(&["enter", "!", "a", "x"]);
    assert_eq!(key_value(&app, "app.open_help"), "exclamation_mark, x");
    // A key it already has changes nothing.
    app.press(&["a", "x"]);
    assert_eq!(key_value(&app, "app.open_help"), "exclamation_mark, x");
    app.press(&["r"]);
    assert_eq!(key_value(&app, "app.open_help"), "?");
}

#[test]
fn out_of_the_wait_q_and_escape_close_and_slash_searches() {
    let (mut app, outcome) = on_list(&config());
    app.press(&["/"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Search));
    assert!(outcome.lock().expect("not poisoned").is_none());
    for key in ["q", "escape"] {
        let (mut app, outcome) = on_list(&config());
        app.press(&[key]);
        assert!(is_cancelled(&outcome), "{key}");
    }
}

#[test]
fn ctrl_q_still_quits_while_waiting() {
    let (mut app, _) = on_help(&config());
    app.press(&["enter", "ctrl+q"]);
    assert_eq!(key_value(&app, "app.open_help"), "?");
    assert!(app.app().should_quit());
}

#[test]
fn save_writes_overrides_in_place_appends_new_ones_and_removes_defaults() {
    let mut config = config();
    let overrides = &mut config.bindings.overrides;
    overrides.insert("app.quit".into(), "x".into());
    overrides.insert("not.a_binding".into(), "z".into());
    overrides.insert("app.maximize_widget".into(), "f2".into());
    let (mut app, outcome) = on_list(&config);
    // `app.quit` back to its default; `app.maximize_widget` changed in place; `app.open_help` added.
    app.press(&["r", "down", "down", "enter", "f3", "down", "enter", "!"]);
    app.press(&["tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Save));
    app.press(&["enter"]);
    assert_eq!(
        written(&saved(&outcome)),
        [
            ("not.a_binding", "z"),
            ("app.maximize_widget", "f3"),
            ("app.open_help", "exclamation_mark")
        ]
    );
}

#[test]
fn a_differently_spelled_default_removes_the_override() {
    // Decision settings-key-bindings-list, D5: revisits Py bug B93's "keep parity" for the reworked tab.
    let mut config = config();
    config.bindings.overrides.insert("app.open_help".into(), "h".into());
    let (mut app, _) = on_help(&config);
    app.press(&["enter", "?"]);
    assert_eq!(key_value(&app, "app.open_help"), "question_mark");
    assert_eq!(key_list(&app).detail(), "app.open_help · default ?");
    let collected = dialog(&app).collect(&config).expect("valid");
    assert_eq!(collected.bindings.overrides.get("app.open_help"), None);
}

#[test]
fn a_non_ascii_key_is_saved_as_itself_and_reads_back() {
    let (mut app, _) = on_help(&config());
    app.press(&["enter", "£"]);
    let collected = dialog(&app).collect(&config()).expect("valid");
    assert_eq!(
        collected.bindings.overrides.get("app.open_help").map(String::as_str),
        Some("£")
    );
    let keymap = Keymap::new(&collected.bindings.overrides);
    let pound = KeySpec::Chord(KeyChord::new(KeyCode::Char('£'), KeyModifiers::NONE));
    assert!(keymap.keys(BindingId::AppOpenHelp).iter().any(|(_, key)| *key == pound));
}

#[test]
fn search_filters_the_list_and_an_empty_one_leaves_the_ring() {
    let (mut app, _) = on_help(&config());
    search(&mut app, "keybindings");
    assert!(
        dialog(&app).panes[KEY_BINDINGS]
            .stops()
            .contains(&SettingsFocus::KeyList)
    );
    assert_eq!(highlighted_id(&app), Some("app.open_help"));
    search(&mut app, "app.quit");
    assert_eq!(highlighted_id(&app), Some("app.quit"));
    search(&mut app, "zzzzzz");
    assert!(dialog(&app).panes[KEY_BINDINGS].stops().is_empty());
}

#[test]
fn a_click_highlights_a_binding_and_a_second_click_waits_for_a_key() {
    let (mut app, _) = on_list(&config());
    let screen = app.screen(160, 48);
    let (col, row) = cell_of(&screen, "Keybindings");
    app.click(col, row);
    assert_eq!(focused(&app), Some(SettingsFocus::KeyList));
    assert_eq!(highlighted_id(&app), Some("app.open_help"));
    assert!(!is_capturing(&app));
    app.render(160, 48);
    app.click(col, row);
    assert!(is_capturing(&app));
    // A press anywhere ends the wait, and does nothing else.
    let (col, row) = cell_of(&app.screen(160, 48), "Cancel");
    app.click(col, row);
    assert!(!is_capturing(&app));
    assert_eq!(app.app().modals(), [ModalKind::Settings]);
}

#[test]
fn a_click_from_another_stop_focuses_the_list_without_waiting() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, KEY_BINDINGS);
    let (col, row) = cell_of(&app.screen(160, 48), "Quit");
    app.click(col, row);
    assert_eq!(focused(&app), Some(SettingsFocus::KeyList));
    assert_eq!(highlighted_id(&app), Some("app.quit"));
    assert!(!is_capturing(&app));
}

#[test]
fn snapshot_key_bindings() {
    let (mut app, _) = on_list(&config());
    insta::assert_snapshot!("key_bindings", app.screen(160, 48));
}

#[test]
fn snapshot_key_bindings_narrow() {
    let (mut app, _) = on_list(&config());
    insta::assert_snapshot!("key_bindings_narrow", app.screen(100, 30));
}

#[test]
fn snapshot_key_bindings_capturing() {
    let (mut app, _) = on_help(&config());
    app.press(&["enter"]);
    insta::assert_snapshot!("key_bindings_capturing", app.screen(160, 48));
}

#[test]
fn snapshot_key_bindings_changed() {
    let mut config = config();
    config
        .bindings
        .overrides
        .insert("app.maximize_widget".into(), "f2".into());
    let (mut app, _) = on_help(&config);
    app.press(&["a", "!"]);
    insta::assert_snapshot!("key_bindings_changed", app.screen(160, 48));
    insta::assert_snapshot!(
        "key_bindings_changed_styled",
        crate::test_util::buffer_to_styled_string(&app.render(160, 48))
    );
}

#[test]
fn snapshot_key_bindings_search() {
    let (mut app, _) = on_list(&config());
    search(&mut app, "issue");
    insta::assert_snapshot!("key_bindings_search", app.screen(160, 48));
}

// Conflict warnings (#382).

/// The list with Quit on `j` (the table and diff cursors' key) and Command Palette on `tab` (the built-in focus
/// key), Quit highlighted.
fn with_conflicts() -> (TestApp, Outcome) {
    let mut config = config();
    let overrides = &mut config.bindings.overrides;
    overrides.insert("app.quit".into(), "j".into());
    overrides.insert("app.command_palette".into(), "tab".into());
    on_list(&config)
}

#[test]
fn conflicts_are_checked_on_opening() {
    let (mut app, _) = with_conflicts();
    // Quit, the table cursor, the diff cursor and Command Palette.
    assert_eq!(key_list(&app).conflict_count(), 4);
    let screen = app.screen(160, 48);
    assert!(screen.contains("General  2 conflicts"), "{screen}");
    assert!(screen.contains("!  Quit                       j "), "{screen}");
    assert!(screen.contains("!  Command Palette            tab "), "{screen}");
    assert!(
        screen.contains("app.quit · default q · changed · ! j also Table cursor down (Repositories table"),
        "{screen}"
    );
}

#[test]
fn a_capture_onto_a_taken_key_warns_and_save_still_closes() {
    let (mut app, outcome) = on_list(&config());
    app.press(&["enter", "j"]);
    assert_eq!(key_list(&app).conflict_count(), 3);
    app.press(&["r"]);
    assert_eq!(key_list(&app).conflict_count(), 0);
    app.press(&["enter", "j"]);
    tab_to(&mut app, SettingsFocus::Save);
    app.press(&["enter"]);
    assert_eq!(written(&saved(&outcome)), [("app.quit", "j")]);
}

#[test]
fn snapshot_key_bindings_conflict() {
    let (mut app, _) = with_conflicts();
    insta::assert_snapshot!("key_bindings_conflict", app.screen(160, 48));
    insta::assert_snapshot!(
        "key_bindings_conflict_styled",
        crate::test_util::buffer_to_styled_string(&app.render(160, 48))
    );
}
