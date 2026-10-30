//! The Settings modal: opening from the palette, the tabs, the widgets, the focus ring, list fields, Save's collected
//! config and Cancel.

use std::sync::{Arc, Mutex};

use lazy_github_core::config::{IssueOwnerFilter, IssueStateFilter, MergeMethod};

use super::*;
use crate::components::footer::footer_items;
use crate::test_util::TestApp;
use crate::toast::Severity;

/// The tabs, by index.
const APPEARANCE: usize = 0;
const REPOSITORIES: usize = 3;
const PULL_REQUESTS: usize = 4;
const ISSUES: usize = 5;
const CACHE: usize = 6;
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
    for _ in 0..30 {
        if focused(app) == Some(stop) {
            return;
        }
        app.press(&["tab"]);
        app.render(160, 48);
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
    assert_eq!(app.app().config().appearance.theme, "textual-dark");
    app.press(&["ctrl+p"]);
    type_text(&mut app, "change settings");
    app.press(&["enter"]);
    match widget(&app, APPEARANCE, 0) {
        FieldWidget::Select(select) => assert_eq!(select.value(), Some(&"textual-dark")),
        other => panic!("Theme is a select, not {other:?}"),
    }
    assert!(app.screen(160, 48).contains("Textual Dark"));
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
            vec!["select", "switch", "switch", "switch", "switch"],
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
    assert_eq!(
        theme.labels().take(3).collect::<Vec<_>>(),
        ["Textual Dark", "Textual Light", "Nord"]
    );
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

    // Switching tabs from the strip changes the fields: a list field is two stops.
    go_to_tab(&mut app, REPOSITORIES);
    assert_eq!(
        ring_forward(&mut app),
        [
            Tabs,
            Scroll,
            Field(0),
            ListItems(0),
            Field(1),
            ListItems(1),
            Save,
            Cancel
        ]
    );
    // The Key Bindings tab has none until #194.
    app.press(&["left"]);
    assert_eq!(ring_forward(&mut app), [Tabs, Scroll, Save, Cancel]);
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

#[test]
fn a_list_fields_checklist_moves_from_no_highlight_and_toggles() {
    let mut config = config();
    config.repositories.favorites = vec!["a/one".into(), "a/two".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::ListItems(1));
    let list = |app: &TestApp| match widget(app, REPOSITORIES, 1) {
        FieldWidget::List(list) => (list.list.highlighted(), list.value()),
        other => panic!("Favorites is a list, not {other:?}"),
    };
    assert_eq!(list(&app), (None, vec!["a/one".into(), "a/two".into()]));
    app.press(&["up"]);
    assert_eq!(list(&app).0, Some(1));
    app.press(&["down"]);
    assert_eq!(list(&app).0, Some(0));
    app.press(&["space"]);
    assert_eq!(list(&app).1, ["a/two"]);
    app.press(&["enter"]);
    assert_eq!(list(&app).1, ["a/two", "a/one"]);
}

#[test]
fn a_list_fields_input_adds_items_by_the_rules() {
    let (mut app, _) = opened(&config());
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    let state = |app: &TestApp| match widget(app, REPOSITORIES, 1) {
        FieldWidget::List(list) => (
            list.input.value().to_owned(),
            list.list.labels().map(str::to_owned).collect(),
        ),
        other => panic!("Favorites is a list, not {other:?}"),
    };
    // Blank text stays and adds nothing.
    type_text(&mut app, "  ");
    app.press(&["enter"]);
    assert_eq!(state(&app), ("  ".to_owned(), Vec::<String>::new()));
    app.press(&["ctrl+u"]);
    type_text(&mut app, " o/r ");
    app.press(&["enter"]);
    assert_eq!(state(&app), (String::new(), vec!["o/r".to_owned()]));
    // A duplicate stays in the input.
    type_text(&mut app, "o/r");
    app.press(&["enter"]);
    assert_eq!(state(&app), ("o/r".to_owned(), vec!["o/r".to_owned()]));
    // So does one of an unchecked item.
    app.press(&["tab", "down", "space", "shift+tab"]);
    assert_eq!(focused(&app), Some(SettingsFocus::Field(1)));
    match widget(&app, REPOSITORIES, 1) {
        FieldWidget::List(list) => assert!(list.value().is_empty()),
        other => panic!("Favorites is a list, not {other:?}"),
    }
    type_text(&mut app, "o/r");
    app.press(&["enter"]);
    assert_eq!(state(&app), ("o/r".to_owned(), vec!["o/r".to_owned()]));
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
fn a_list_saves_its_checked_items_in_the_order_they_were_checked() {
    let mut config = config();
    config.repositories.favorites = vec!["a/1".into(), "a/2".into(), "a/3".into()];
    let (mut app, outcome) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::Field(1));
    type_text(&mut app, "a/4");
    app.press(&["enter", "tab"]);
    // Uncheck "a/1" and "a/3", then check "a/1" again.
    app.press(&["down", "space", "down", "down", "space", "up", "up", "space"]);
    app.press(&["shift+enter"]);
    assert_eq!(saved(&outcome).repositories.favorites, ["a/2", "a/4", "a/1"]);
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

#[test]
fn snapshot_list_field() {
    let mut config = config();
    config.repositories.favorites = vec!["octo/one".into(), "octo/two".into()];
    let (mut app, _) = opened(&config);
    go_to_tab(&mut app, REPOSITORIES);
    tab_to(&mut app, SettingsFocus::ListItems(1));
    app.press(&["down", "space"]);
    insta::assert_snapshot!("list_field", app.screen(160, 48));
    insta::assert_snapshot!(
        "list_field_styled",
        crate::test_util::buffer_to_styled_string(&app.render(160, 48))
    );
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
