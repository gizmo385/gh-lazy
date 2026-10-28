//! Each Python class's merged bindings, dumped to `fixtures/contexts.json` by `fixtures/generate_bindings.py`, match
//! the Rust list that mirrors the class: the same bindings in the same order, on the same keys, and the same
//! definitions for the Textual built-ins, which `bindings.json` doesn't cover.
//!
//! Production code never names Python classes: the class-to-list table is here.

use std::collections::BTreeSet;

use lazy_github_tui::app::APP_BINDINGS;
use lazy_github_tui::components::button::BUTTON_BINDINGS;
use lazy_github_tui::components::info::INFO_BINDINGS;
use lazy_github_tui::components::switch::SWITCH_BINDINGS;
use lazy_github_tui::components::table::{SEARCHABLE_BINDINGS, TABLE_BINDINGS};
use lazy_github_tui::components::text_editor::TEXT_EDITOR_BINDINGS;
use lazy_github_tui::components::text_input::INPUT_BINDINGS;
use lazy_github_tui::components::tree::TREE_BINDINGS;
use lazy_github_tui::keymap::{BindingId, KeySpec};
use lazy_github_tui::modal::COMMAND_PALETTE_BINDINGS;
use lazy_github_tui::screens::auth::AUTH_BINDINGS;
use lazy_github_tui::screens::main::{
    LOG_BINDINGS, MAIN_VIEW_BINDINGS, SCREEN_BINDINGS, SELECTIONS_BINDINGS, TABS_BINDINGS,
};
use serde_json::Value;

/// Python classes and the Rust lists that mirror them.
const MAPPED: &[(&str, &[BindingId])] = &[
    ("lazy_github.ui.app.LazyGithub", APP_BINDINGS),
    ("lazy_github.ui.screens.auth.AuthenticationModal", AUTH_BINDINGS),
    ("lazy_github.ui.screens.primary.LazyGithubMainScreen", SCREEN_BINDINGS),
    ("lazy_github.ui.screens.primary.MainViewPane", MAIN_VIEW_BINDINGS),
    ("lazy_github.ui.screens.primary.SelectionsPane", SELECTIONS_BINDINGS),
    ("lazy_github.ui.widgets.common._VimLikeDataTable", TABLE_BINDINGS),
    ("lazy_github.ui.widgets.common.SearchableDataTable", SEARCHABLE_BINDINGS),
    ("textual.containers.VerticalScroll", INFO_BINDINGS),
    ("textual.widgets.Log", LOG_BINDINGS),
    ("textual.widgets.Tabs", TABS_BINDINGS),
    ("textual.widgets.Input", INPUT_BINDINGS),
    ("textual.widgets.Button", BUTTON_BINDINGS),
    ("textual.widgets.Switch", SWITCH_BINDINGS),
    ("textual.widgets.Tree", TREE_BINDINGS),
    ("textual.widgets.TextArea", TEXT_EDITOR_BINDINGS),
    ("textual.command.CommandPalette", COMMAND_PALETTE_BINDINGS),
];

/// An expected difference between a mapped class and its Rust list. The test fails once it's gone, so the entry is
/// dropped with the change that closes it.
struct ContextDiff {
    class: &'static str,
    /// Python bindings the Rust list doesn't have, as `(Textual key, action)`.
    missing: &'static [(&'static str, &'static str)],
    /// Rust ids the Python class doesn't have.
    extra: &'static [BindingId],
}

const CONTEXT_DIFFS: &[ContextDiff] = &[
    // `Screen`'s built-ins, which the auth modal inherits. On Python's auth screen nothing is focusable, so tab and
    // shift+tab do nothing, and with no text selected `copy_text` skips; the modal's chain ends at the screen, so
    // ctrl+c never reaches the app's `help_quit` (checked with Textual's `run_test`, #158). None of them is shown
    // (`show=False`), and the auth screen has no help panel.
    ContextDiff {
        class: "lazy_github.ui.screens.auth.AuthenticationModal",
        missing: &[
            ("tab", "app.focus_next"),
            ("shift+tab", "app.focus_previous"),
            ("ctrl+c", "screen.copy_text"),
            ("super+c", "screen.copy_text"),
        ],
        extra: &[],
    },
];

/// Classes whose component isn't built yet, with the matrix row or issue it lands with. That change maps the class
/// in [`MAPPED`] and adds its keybinding rows to `parity/goldens/keybindings.toml`.
const NOT_BUILT: &[(&str, &str)] = &[
    (
        "lazy_github.ui.screens.add_reactions_modal.AddReactionsModal",
        "modals.add-reaction",
    ),
    ("lazy_github.ui.screens.confirm.ConfirmDialog", "modals.confirm"),
    (
        "lazy_github.ui.screens.create_or_edit_pull_request.CreateOrEditPullRequestContainer",
        "modals.create-pr-submit",
    ),
    (
        "lazy_github.ui.screens.create_or_edit_pull_request.CreateOrEditPullRequestModal",
        "modals.create-edit-pr",
    ),
    ("lazy_github.ui.screens.debug.DebugModal", "modals.debug-bug-report"),
    ("lazy_github.ui.screens.edit_issue.EditIssueModal", "modals.edit-issue"),
    ("lazy_github.ui.screens.link_paste.PasteLinkModal", "modals.paste-link"),
    (
        "lazy_github.ui.screens.lookup_issue.LookupIssueModal",
        "modals.lookup-issue",
    ),
    (
        "lazy_github.ui.screens.lookup_pull_request.LookupPullRequestModal",
        "modals.lookup-pr",
    ),
    (
        "lazy_github.ui.screens.lookup_repository.LookupRepositoryModal",
        "modals.lookup-repository",
    ),
    (
        "lazy_github.ui.screens.new_comment.NewCommentModal",
        "modals.new-comment",
    ),
    ("lazy_github.ui.screens.new_issue.NewIssueContainer", "modals.new-issue"),
    ("lazy_github.ui.screens.new_issue.NewIssueModal", "modals.new-issue"),
    (
        "lazy_github.ui.screens.notifications.NotificationsContainer",
        "modals.notifications-keys",
    ),
    (
        "lazy_github.ui.screens.notifications.NotificationsModal",
        "modals.notifications",
    ),
    (
        "lazy_github.ui.screens.notifications.UnreadNotificationTabPane",
        "modals.notifications-keys",
    ),
    (
        "lazy_github.ui.screens.settings.SettingsContainer",
        "modals.settings-save",
    ),
    (
        "lazy_github.ui.screens.trigger_workflow.TriggerWorkflowModal",
        "modals.trigger-workflow",
    ),
    (
        "lazy_github.ui.screens.user_profile.UserProfileModal",
        "modals.user-profile",
    ),
    (
        "lazy_github.ui.widgets.conversations.IssueCommentContainer",
        "pull-requests.comment-block",
    ),
    (
        "lazy_github.ui.widgets.conversations.ReviewContainer",
        "pull-requests.conversation-reply",
    ),
    (
        "lazy_github.ui.widgets.issues.IssueConversationTabPane",
        "issues.comments-tab",
    ),
    (
        "lazy_github.ui.widgets.issues.IssueOverviewTabPane",
        "issues.overview-edit",
    ),
    // The section's footer list exists (`SectionId::bindings`); the table's issue maps it.
    ("lazy_github.ui.widgets.issues.IssuesContainer", "issues.table, #172"),
    (
        "lazy_github.ui.widgets.pull_requests.PrConversationTabPane",
        "pull-requests.conversation-new-comment",
    ),
    (
        "lazy_github.ui.widgets.pull_requests.PrOverviewTabPane",
        "pull-requests.overview-merge",
    ),
    (
        "lazy_github.ui.widgets.pull_requests.PullRequestsContainer",
        "pull-requests.table, #171",
    ),
    (
        "lazy_github.ui.widgets.repositories.ReposContainer",
        "repositories.table, #168",
    ),
    (
        "lazy_github.ui.widgets.split_diff_viewer.AddCommentModal",
        "diff-viewer.add-comment",
    ),
    (
        "lazy_github.ui.widgets.split_diff_viewer.SplitDiffViewer",
        "diff-viewer.hunk-pane",
    ),
    (
        "lazy_github.ui.widgets.split_diff_viewer.UnifiedDiffPane",
        "diff-viewer.line-cursor",
    ),
    (
        "lazy_github.ui.widgets.workflow_run_details.WorkflowRunJobsTabPane",
        "workflows.jobs-tab",
    ),
    (
        "lazy_github.ui.widgets.workflows.WorkflowsContainer",
        "workflows.runs-table, #173",
    ),
];

/// One `(key, binding)` of a class's merged bindings.
struct PythonBinding<'a> {
    key: &'a str,
    id: Option<&'a str>,
    action: &'a str,
    value: &'a Value,
}

fn python_contexts() -> Vec<(String, Vec<Value>)> {
    let json = include_str!("fixtures/contexts.json");
    let contexts: Vec<Value> = serde_json::from_str(json).expect("contexts.json is a JSON array");
    contexts
        .into_iter()
        .map(|context| {
            let class = context["class"].as_str().expect("class is a string").to_owned();
            let bindings = context["bindings"].as_array().expect("bindings is an array").clone();
            (class, bindings)
        })
        .collect()
}

fn text<'a>(binding: &'a Value, field: &str) -> &'a str {
    binding[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} in {binding}"))
}

fn flag(binding: &Value, field: &str) -> bool {
    binding[field]
        .as_bool()
        .unwrap_or_else(|| panic!("{field} in {binding}"))
}

fn parse(bindings: &[Value]) -> Vec<PythonBinding<'_>> {
    bindings
        .iter()
        .map(|value| PythonBinding {
            key: text(value, "key"),
            id: value["id"].as_str(),
            action: text(value, "action"),
            value,
        })
        .collect()
}

fn default_specs(id: BindingId) -> Vec<KeySpec> {
    KeySpec::parse_list(id.def().default_keys)
        .into_iter()
        .map(|(_, spec)| spec)
        .collect()
}

/// The id in `rust` that a Python binding is: by its id, or for a Textual built-in (no id), the one `textual.*` def
/// with its action and key. `None` when the list has none.
fn resolve(class: &str, binding: &PythonBinding<'_>, rust: &[BindingId]) -> Option<BindingId> {
    if let Some(id) = binding.id {
        let id = BindingId::from_config_id(id).unwrap_or_else(|| panic!("{class}: {id} is missing from BindingId"));
        return rust.contains(&id).then_some(id);
    }
    let spec = KeySpec::from_event_name(binding.key);
    let found: Vec<BindingId> = rust
        .iter()
        .copied()
        .filter(|id| {
            let def = id.def();
            def.name.starts_with("textual.") && def.action == binding.action && default_specs(*id).contains(&spec)
        })
        .collect();
    assert!(
        found.len() <= 1,
        "{class}: {} {} is ambiguous: {found:?}",
        binding.key,
        binding.action
    );
    found.first().copied()
}

fn diff_for(class: &str) -> Option<&'static ContextDiff> {
    CONTEXT_DIFFS.iter().find(|diff| diff.class == class)
}

#[test]
fn every_class_is_mapped_or_not_built() {
    let python: Vec<String> = python_contexts().into_iter().map(|(class, _)| class).collect();
    assert_eq!(python.len(), 49);
    for class in &python {
        let mapped = MAPPED.iter().any(|&(mapped, _)| mapped == class);
        let not_built = NOT_BUILT.iter().any(|&(not_built, _)| not_built == class);
        assert!(
            mapped || not_built,
            "{class} defines bindings: map it to its Rust list in MAPPED, or add it to NOT_BUILT with the row or \
             issue its component lands with"
        );
        assert!(!(mapped && not_built), "{class} is in both MAPPED and NOT_BUILT");
    }
    let tables = MAPPED
        .iter()
        .map(|&(class, _)| class)
        .chain(NOT_BUILT.iter().map(|&(class, _)| class));
    for class in tables.chain(CONTEXT_DIFFS.iter().map(|diff| diff.class)) {
        assert!(
            python.iter().any(|python| python == class),
            "{class} isn't in contexts.json"
        );
    }
    for diff in CONTEXT_DIFFS {
        assert!(
            MAPPED.iter().any(|&(class, _)| class == diff.class),
            "{}: CONTEXT_DIFFS is for mapped classes",
            diff.class
        );
    }
}

#[test]
fn mapped_classes_match_their_rust_lists() {
    let contexts = python_contexts();
    for &(class, rust) in MAPPED {
        let (_, bindings) = contexts
            .iter()
            .find(|(python, _)| python == class)
            .unwrap_or_else(|| panic!("{class} isn't in contexts.json"));
        let python = parse(bindings);
        let diff = diff_for(class);

        // Python bindings with no Rust counterpart, each once (B9 lists two table bindings twice).
        let mut missing: Vec<(&str, &str)> = Vec::new();
        // Each id at its first appearance, which is its place in Textual's merged order.
        let mut order: Vec<BindingId> = Vec::new();
        for binding in &python {
            match resolve(class, binding, rust) {
                Some(id) if !order.contains(&id) => order.push(id),
                None if !missing.contains(&(binding.key, binding.action)) => {
                    missing.push((binding.key, binding.action));
                }
                Some(_) | None => {}
            }
        }
        let extra: Vec<BindingId> = rust.iter().copied().filter(|id| !order.contains(id)).collect();
        let (expected_missing, expected_extra) = diff.map_or((&[][..], &[][..]), |diff| (diff.missing, diff.extra));
        assert_eq!(
            missing, expected_missing,
            "{class}: Python bindings missing from the Rust list (update CONTEXT_DIFFS once they're expected or \
             gone)"
        );
        assert_eq!(
            extra, expected_extra,
            "{class}: Rust ids the Python class doesn't have (update CONTEXT_DIFFS once they're expected or gone)"
        );
        assert_eq!(order, rust, "{class}: order");

        for &id in &order {
            let mut python_keys: Vec<KeySpec> = Vec::new();
            for binding in python
                .iter()
                .filter(|binding| resolve(class, binding, rust) == Some(id))
            {
                let spec = KeySpec::from_event_name(binding.key);
                if !python_keys.contains(&spec) {
                    python_keys.push(spec);
                }
            }
            let rust_keys = default_specs(id);
            assert_eq!(
                python_keys.len(),
                rust_keys.len(),
                "{class}: {} keys {python_keys:?}, Rust {rust_keys:?}",
                id.def().name
            );
            assert!(
                python_keys.iter().all(|spec| rust_keys.contains(spec)),
                "{class}: {} keys {python_keys:?}, Rust {rust_keys:?}",
                id.def().name
            );
        }

        for binding in python.iter().filter(|binding| binding.id.is_none()) {
            let Some(id) = resolve(class, binding, rust) else {
                continue;
            };
            let (def, value, name) = (id.def(), binding.value, id.def().name);
            assert_eq!(
                def.description,
                text(value, "description"),
                "{class}: {name} description"
            );
            assert_eq!(def.show, flag(value, "show"), "{class}: {name} show");
            assert_eq!(def.priority, flag(value, "priority"), "{class}: {name} priority");
            assert_eq!(def.system, flag(value, "system"), "{class}: {name} system");
            assert_eq!(def.tooltip, text(value, "tooltip"), "{class}: {name} tooltip");
            assert_eq!(
                def.key_display,
                value["key_display"].as_str(),
                "{class}: {name} key_display"
            );
        }
    }
}

/// The priority and system flags in contexts.json agree with bindings.json for the ids both have, so a class can't
/// change a registry binding's flags unnoticed.
#[test]
fn context_flags_agree_with_the_registry() {
    let ids: BTreeSet<BindingId> = python_contexts()
        .iter()
        .flat_map(|(_, bindings)| bindings.iter())
        .filter_map(|binding| {
            let id = BindingId::from_config_id(binding["id"].as_str()?)?;
            assert_eq!(
                id.def().priority,
                flag(binding, "priority"),
                "{} priority",
                id.def().name
            );
            assert_eq!(id.def().system, flag(binding, "system"), "{} system", id.def().name);
            Some(id)
        })
        .collect();
    assert!(!ids.is_empty());
}
