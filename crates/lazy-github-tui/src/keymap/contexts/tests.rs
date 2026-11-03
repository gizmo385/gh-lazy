use indexmap::IndexMap;
use lazy_github_core::config::Config;

use super::*;
use crate::action::Action;
use crate::modal::ConfirmDialog;
use crate::test_util::TestApp;

fn keymap(overrides: &[(&str, &str)]) -> Keymap {
    Keymap::new(
        &overrides
            .iter()
            .map(|&(id, keys)| (id.to_owned(), keys.to_owned()))
            .collect::<IndexMap<_, _>>(),
    )
}

/// The conflicts of `id`, as `(other, kind, key, contexts)`.
fn of(conflicts: &[KeyConflict], id: BindingId) -> Vec<(BindingId, ConflictKind, &str, Vec<&'static str>)> {
    conflicts
        .iter()
        .filter(|conflict| conflict.id == id)
        .map(|conflict| {
            (
                conflict.other,
                conflict.kind,
                conflict.key.as_str(),
                conflict.contexts.clone(),
            )
        })
        .collect()
}

const TABLES: [&str; 4] = [
    "Repositories table",
    "Pull requests table",
    "Issues table",
    "Workflows table",
];

#[test]
fn the_default_keymap_has_no_conflicts() {
    assert_eq!(conflicts(&Keymap::defaults()), []);
}

#[test]
fn quit_on_j_meets_the_table_and_diff_cursors() {
    use BindingId as B;
    let found = conflicts(&keymap(&[("app.quit", "j")]));
    assert_eq!(
        of(&found, B::AppQuit),
        [
            (B::TableCursorDown, ConflictKind::ShadowedBy, "j", TABLES.to_vec()),
            (B::DiffCursorDown, ConflictKind::ShadowedBy, "j", vec!["Diff viewer"]),
        ]
    );
    // Both other sides are rebindable, so they're flagged too.
    assert_eq!(
        of(&found, B::TableCursorDown),
        [(B::AppQuit, ConflictKind::Shadows, "j", TABLES.to_vec())]
    );
    assert_eq!(
        of(&found, B::DiffCursorDown),
        [(B::AppQuit, ConflictKind::Shadows, "j", vec!["Diff viewer"])]
    );
    assert_eq!(found.len(), 4);
}

#[test]
fn a_table_key_on_tab_shadows_the_built_in_focus_next() {
    let found = conflicts(&keymap(&[("common.table.cursor_down", "tab")]));
    // Every table: the four sections', the profile's and the notifications'. The built-in has no conflict of its
    // own.
    let contexts = [&TABLES[..], &["User profile", "Notifications"]].concat();
    assert_eq!(
        of(&found, BindingId::TableCursorDown),
        [(BindingId::TextualFocusNext, ConflictKind::Shadows, "tab", contexts)]
    );
    assert_eq!(found.len(), 1);
}

#[test]
fn ctrl_q_away_from_the_app_link_is_taken_by_the_priority_quit() {
    let found = conflicts(&keymap(&[("common.table.cursor_down", "ctrl+q")]));
    let contexts = [&TABLES[..], &["User profile", "Notifications"]].concat();
    assert_eq!(
        of(&found, BindingId::TableCursorDown),
        [(BindingId::TextualQuit, ConflictKind::Priority, "^q", contexts)]
    );
    assert_eq!(found.len(), 1);
}

#[test]
fn ctrl_q_on_the_app_link_takes_the_key_from_the_priority_quit() {
    // In one link the clash rule applies first, as Textual applies the keymap before checking priority: help takes
    // `ctrl+q` and quitting loses it.
    let found = conflicts(&keymap(&[("app.open_help", "ctrl+q")]));
    let help = of(&found, BindingId::AppOpenHelp);
    assert_eq!(help.len(), 1);
    let (other, kind, key, contexts) = &help[0];
    assert_eq!(
        (*other, *kind, *key),
        (BindingId::TextualQuit, ConflictKind::Shares, "^q")
    );
    assert_eq!(contexts[..5], [&TABLES[..], &["Details tabs"]].concat());
    assert!(contexts.iter().all(|name| !name.contains("dialog")), "{contexts:?}");
}

#[test]
fn an_override_onto_a_neighbour_s_key_shares_its_link() {
    use BindingId as B;
    let found = conflicts(&keymap(&[("pull_request.lookup", "E")]));
    assert_eq!(
        found,
        [
            KeyConflict {
                id: B::PullRequestLookup,
                other: B::PullRequestEdit,
                key: "E".to_owned(),
                kind: ConflictKind::Shares,
                contexts: vec!["Pull requests table"],
            },
            KeyConflict {
                id: B::PullRequestEdit,
                other: B::PullRequestLookup,
                key: "E".to_owned(),
                kind: ConflictKind::Shares,
                contexts: vec!["Pull requests table"],
            },
        ]
    );
}

#[test]
fn bindings_that_never_meet_dont_conflict() {
    // The issue lookup is the Issues table's; a new comment is the conversations'.
    assert_eq!(conflicts(&keymap(&[("issue.lookup", "n")])), []);
}

#[test]
fn resetting_or_moving_to_a_free_key_clears_it() {
    assert_eq!(conflicts(&keymap(&[("app.quit", "q")])), []);
    assert_eq!(conflicts(&keymap(&[("app.quit", "f9")])), []);
}

/// Every overlap in the default keymap with a rebindable side, changed or not. Overlaps between built-ins alone
/// (Textual's own `up` on a tree and on the scroll view around it) are Textual's and left out. A new binding or
/// context that overlaps by default changes this list, which makes it a reviewed choice.
const DEFAULT_OVERLAPS: &[&str] = &[
    "conversation.comment.reply Shadows conversation.review.reply on r: \
     Pull request conversation, on a review comment",
    "conversation.review.reply ShadowedBy conversation.comment.reply on r: \
     Pull request conversation, on a review comment",
];

#[test]
fn the_default_overlaps_are_the_reviewed_list() {
    let mut lines: Vec<String> = overlaps(&Keymap::defaults())
        .into_iter()
        .filter(|c| c.id.config_id().is_some() || c.other.config_id().is_some())
        .map(|c| {
            format!(
                "{} {:?} {} on {}: {}",
                c.id.def().name,
                c.kind,
                c.other.def().name,
                c.key,
                c.contexts.join(", ")
            )
        })
        .collect();
    lines.sort();
    assert_eq!(lines, DEFAULT_OVERLAPS);
}

#[test]
fn every_rebindable_id_is_in_a_context() {
    let missing: Vec<&str> = BindingId::ALL
        .iter()
        .filter(|id| id.config_id().is_some())
        .filter(|&id| {
            !KEY_CONTEXTS
                .iter()
                .any(|context| context.links.iter().any(|link| link.contains(id)))
        })
        .map(|id| id.def().name)
        .collect();
    // Py bug B9: rebindable, but nothing binds them.
    assert_eq!(missing, ["workflows.view_run_details", "diff.clear_selection"]);
}

/// The names of the contexts whose links are exactly the app's binding chain now. The command log and the details
/// content have the same scroll keys, so a chain can match more than one.
fn contexts_of(app: &TestApp) -> Vec<&'static str> {
    let chain: Vec<&[BindingId]> = app.app().binding_chain().iter().map(|link| link.bindings).collect();
    KEY_CONTEXTS
        .iter()
        .filter(|context| context.links == chain.as_slice())
        .map(|context| context.name)
        .collect()
}

#[test]
fn the_contexts_match_the_app_s_chains() {
    let mut config = Config::default();
    config.appearance.show_command_log = true;
    let mut app = TestApp::new(config);
    assert_eq!(contexts_of(&app), ["Repositories table"]);
    for (key, expected) in [
        ("2", "Pull requests table"),
        ("3", "Issues table"),
        ("4", "Workflows table"),
        ("5", "Details tabs"),
        ("tab", "Details"),
        ("6", "Command log"),
    ] {
        app.press(&[key]);
        assert!(
            contexts_of(&app).contains(&expected),
            "after {key}: {:?}",
            contexts_of(&app)
        );
    }
    app.with_cx(|cx| cx.open_dialog(ConfirmDialog::new("Sure?"), |_| Action::Consumed));
    assert_eq!(contexts_of(&app), ["Confirm dialog"]);
}
