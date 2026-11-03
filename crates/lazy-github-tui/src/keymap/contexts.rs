//! Which bindings can be active at the same moment, and which of them want the same key: the Settings Key Bindings
//! list's conflict warnings (#382, decision settings-key-bindings-list, D4).
//!
//! A [`KeyContext`] is one focus state: the binding chain the app builds there, innermost link first. Within one
//! link the clash rule decides which binding a key fires ([`Keymap::effective_specs`]); across links the one nearer
//! the focus wins; and the priority bindings on the app link (`ctrl+q`) run before any of them. [`conflicts`] reports
//! every pair of bindings that want one key in one context, where at least one of the two was changed from its
//! default, so the overlaps the app ships with never warn.
//!
//! Focus states inside a text input, an editor or an open select list are left out: they type printable keys before
//! any binding sees them. So is the Key Bindings list waiting for a key, which takes every key.

use super::{BindingId, KeyChord, KeySpec, Keymap};
use crate::app::APP_BINDINGS;
use crate::components::button::BUTTON_BINDINGS;
use crate::components::comment::{COMMENT_BINDINGS, LIST_BINDINGS, TITLE_BINDINGS};
use crate::components::diff_view::{DIFF_BODY_BINDINGS, DIFF_VIEWER_BINDINGS};
use crate::components::info::INFO_BINDINGS;
use crate::components::list_editor::LIST_EDITOR_BINDINGS;
use crate::components::select::SELECT_BINDINGS;
use crate::components::selection_list::SELECTION_LIST_BINDINGS;
use crate::components::switch::SWITCH_BINDINGS;
use crate::components::table::{SEARCHABLE_BINDINGS, TABLE_BINDINGS};
use crate::components::tabs::TABS_BINDINGS;
use crate::components::tree::TREE_BINDINGS;
use crate::modal::{
    ADD_COMMENT_BINDINGS, ADD_REACTION_BINDINGS, CONFIRM_BINDINGS, CONTAINER_BINDINGS, DEBUG_BINDINGS,
    EDIT_ISSUE_BINDINGS, KEY_LIST_BINDINGS, LOOKUP_BINDINGS, NEW_COMMENT_BINDINGS, NEW_ISSUE_BINDINGS,
    NOTIFICATIONS_BINDINGS, PASTE_LINK_BINDINGS, PR_BINDINGS, PROFILE_BINDINGS, SETTINGS_BINDINGS, TRIGGER_BINDINGS,
    UNREAD_BINDINGS,
};
use crate::screens::main::{
    CONVERSATION_BINDINGS, ISSUE_COMMENTS_BINDINGS, ISSUE_OVERVIEW_BINDINGS, JOBS_BINDINGS, LOG_BINDINGS,
    MAIN_VIEW_BINDINGS, PR_OVERVIEW_BINDINGS, REVIEW_BINDINGS, SCREEN_BINDINGS, SELECTIONS_BINDINGS, SectionId,
};

/// One focus state and the binding chain the app builds for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyContext {
    /// Where the focus is, as the Key Bindings detail line names it. Several focus states of one place share a
    /// name (a dialog's button and its switch are both the dialog).
    pub name: &'static str,
    /// Each link's bindings, innermost first, like a binding chain.
    pub links: &'static [&'static [BindingId]],
}

/// A main-screen context: `links`, then the main view's focus keys, the screen's and the app's.
macro_rules! main {
    ($name:literal, [$($link:expr),* $(,)?]) => {
        KeyContext {
            name: $name,
            links: &[$($link,)* MAIN_VIEW_BINDINGS, SCREEN_BINDINGS, APP_BINDINGS],
        }
    };
}

/// A context with focus inside the details pane's content: `links`, then the content's scroll link and the main
/// screen's tail.
macro_rules! details {
    ($name:literal, [$($link:expr),* $(,)?]) => {
        main!($name, [$($link,)* INFO_BINDINGS])
    };
}

/// A dialog's context: its focused field's link, then the dialog's screen link. A dialog's chain stops there, as
/// Textual's modal chain does; only the priority bindings reach past it.
macro_rules! dialog {
    ($name:literal, $field:expr, $screen:expr) => {
        KeyContext {
            name: $name,
            links: &[$field, $screen],
        }
    };
}

/// Every context a binding can be active in, from `MainScreen::binding_chain` and each dialog's `field_chain`.
pub const KEY_CONTEXTS: &[KeyContext] = &[
    main!(
        "Repositories table",
        [
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            SectionId::Repos.bindings(),
            SELECTIONS_BINDINGS
        ]
    ),
    main!(
        "Pull requests table",
        [
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            SectionId::PullRequests.bindings(),
            SELECTIONS_BINDINGS
        ]
    ),
    main!(
        "Issues table",
        [
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            SectionId::Issues.bindings(),
            SELECTIONS_BINDINGS
        ]
    ),
    main!(
        "Workflows table",
        [
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            SectionId::Workflows.bindings(),
            SELECTIONS_BINDINGS
        ]
    ),
    main!("Details tabs", [TABS_BINDINGS]),
    main!("Command log", [LOG_BINDINGS]),
    // The introduction, a run's Overview and Logs: only the content's scroll keys.
    details!("Details", []),
    details!("Pull request overview", [PR_OVERVIEW_BINDINGS]),
    details!("Pull request overview", [TITLE_BINDINGS, PR_OVERVIEW_BINDINGS]),
    details!("Pull request overview", [LIST_BINDINGS, PR_OVERVIEW_BINDINGS]),
    details!("Pull request conversation", [CONVERSATION_BINDINGS]),
    details!(
        "Pull request conversation, on a review",
        [TITLE_BINDINGS, REVIEW_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a comment",
        [COMMENT_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a comment",
        [TITLE_BINDINGS, COMMENT_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a comment",
        [LIST_BINDINGS, COMMENT_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a review comment",
        [COMMENT_BINDINGS, REVIEW_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a review comment",
        [TITLE_BINDINGS, COMMENT_BINDINGS, REVIEW_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!(
        "Pull request conversation, on a review comment",
        [LIST_BINDINGS, COMMENT_BINDINGS, REVIEW_BINDINGS, CONVERSATION_BINDINGS]
    ),
    details!("Diff viewer", [DIFF_BODY_BINDINGS]),
    details!("Diff viewer review", [BUTTON_BINDINGS, DIFF_VIEWER_BINDINGS]),
    details!("Diff viewer review", [SELECT_BINDINGS, DIFF_VIEWER_BINDINGS]),
    details!("Issue overview", [ISSUE_OVERVIEW_BINDINGS]),
    details!("Issue conversation", [ISSUE_COMMENTS_BINDINGS]),
    details!("Issue conversation", [COMMENT_BINDINGS, ISSUE_COMMENTS_BINDINGS]),
    details!(
        "Issue conversation",
        [TITLE_BINDINGS, COMMENT_BINDINGS, ISSUE_COMMENTS_BINDINGS]
    ),
    details!(
        "Issue conversation",
        [LIST_BINDINGS, COMMENT_BINDINGS, ISSUE_COMMENTS_BINDINGS]
    ),
    details!("Run overview", [BUTTON_BINDINGS]),
    details!("Run jobs", [JOBS_BINDINGS]),
    details!("Run jobs", [TREE_BINDINGS, JOBS_BINDINGS]),
    dialog!("Confirm dialog", BUTTON_BINDINGS, CONFIRM_BINDINGS),
    dialog!("Lookup dialog", SWITCH_BINDINGS, LOOKUP_BINDINGS),
    dialog!("Lookup dialog", BUTTON_BINDINGS, LOOKUP_BINDINGS),
    dialog!("New issue dialog", INFO_BINDINGS, NEW_ISSUE_BINDINGS),
    dialog!("New issue dialog", SELECT_BINDINGS, NEW_ISSUE_BINDINGS),
    dialog!("New issue dialog", BUTTON_BINDINGS, NEW_ISSUE_BINDINGS),
    dialog!("Edit issue dialog", INFO_BINDINGS, EDIT_ISSUE_BINDINGS),
    dialog!("Edit issue dialog", SELECT_BINDINGS, EDIT_ISSUE_BINDINGS),
    dialog!("Edit issue dialog", BUTTON_BINDINGS, EDIT_ISSUE_BINDINGS),
    dialog!("Pull request dialog", INFO_BINDINGS, PR_BINDINGS),
    dialog!("Pull request dialog", SWITCH_BINDINGS, PR_BINDINGS),
    dialog!("Pull request dialog", SELECTION_LIST_BINDINGS, PR_BINDINGS),
    dialog!("Pull request dialog", BUTTON_BINDINGS, PR_BINDINGS),
    dialog!("New comment dialog", BUTTON_BINDINGS, NEW_COMMENT_BINDINGS),
    dialog!("Add comment dialog", BUTTON_BINDINGS, ADD_COMMENT_BINDINGS),
    dialog!("Add reaction dialog", SELECTION_LIST_BINDINGS, ADD_REACTION_BINDINGS),
    dialog!("Add reaction dialog", BUTTON_BINDINGS, ADD_REACTION_BINDINGS),
    dialog!("Trigger workflow dialog", SELECT_BINDINGS, TRIGGER_BINDINGS),
    dialog!("Trigger workflow dialog", BUTTON_BINDINGS, TRIGGER_BINDINGS),
    dialog!("Paste link dialog", BUTTON_BINDINGS, PASTE_LINK_BINDINGS),
    dialog!("Debug info dialog", INFO_BINDINGS, DEBUG_BINDINGS),
    dialog!("Debug info dialog", BUTTON_BINDINGS, DEBUG_BINDINGS),
    dialog!("User profile", INFO_BINDINGS, PROFILE_BINDINGS),
    dialog!("User profile", TABLE_BINDINGS, PROFILE_BINDINGS),
    KeyContext {
        name: "Notifications",
        links: &[INFO_BINDINGS, CONTAINER_BINDINGS, NOTIFICATIONS_BINDINGS],
    },
    KeyContext {
        name: "Notifications",
        links: &[TABS_BINDINGS, CONTAINER_BINDINGS, NOTIFICATIONS_BINDINGS],
    },
    KeyContext {
        name: "Notifications",
        links: &[
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            UNREAD_BINDINGS,
            CONTAINER_BINDINGS,
            NOTIFICATIONS_BINDINGS,
        ],
    },
    KeyContext {
        name: "Notifications",
        links: &[
            TABLE_BINDINGS,
            SEARCHABLE_BINDINGS,
            CONTAINER_BINDINGS,
            NOTIFICATIONS_BINDINGS,
        ],
    },
    dialog!("Settings", TABS_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", INFO_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", BUTTON_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", SWITCH_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", SELECT_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", LIST_EDITOR_BINDINGS, SETTINGS_BINDINGS),
    dialog!("Settings", KEY_LIST_BINDINGS, SETTINGS_BINDINGS),
];

/// How the two bindings of a [`KeyConflict`] meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    /// In one link: the clash rule gives the key to the changed binding, or list order picks one.
    Shares,
    /// `id` is nearer the focus, so `other` never sees the key.
    Shadows,
    /// `other` is nearer the focus, so `id` never sees the key.
    ShadowedBy,
    /// `other` is a priority binding (`ctrl+q`), which takes the key before anything else.
    Priority,
}

/// Two bindings that want one key in at least one context, from `id`'s side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConflict {
    /// The binding the warning is about.
    pub id: BindingId,
    /// The binding it meets.
    pub other: BindingId,
    /// The key, as [`Keymap::key_display`] shows it for `id`.
    pub key: String,
    /// How they meet.
    pub kind: ConflictKind,
    /// The [`KeyContext::name`]s where they meet, each once, in [`KEY_CONTEXTS`] order.
    pub contexts: Vec<&'static str>,
}

/// Every conflict in `keymap` from the side of each rebindable binding in it: two different bindings with the same
/// key in one context, where at least one of the two is changed ([`Keymap::is_overridden`]). The same pair in many
/// contexts is one conflict naming them all. A built-in or internal binding on the other side has no conflict of its
/// own, since nothing can rebind it.
pub fn conflicts(keymap: &Keymap) -> Vec<KeyConflict> {
    overlaps(keymap)
        .into_iter()
        .filter(|conflict| {
            conflict.id.config_id().is_some()
                && (keymap.is_overridden(conflict.id) || keymap.is_overridden(conflict.other))
        })
        .collect()
}

/// Every pair of bindings that want one key in one context, from both sides, changed or not.
pub(crate) fn overlaps(keymap: &Keymap) -> Vec<KeyConflict> {
    let mut found = Found::default();
    let priority: Vec<(BindingId, KeyChord)> = keymap
        .effective_keys(APP_BINDINGS)
        .into_iter()
        .filter(|(id, _)| id.is_priority())
        .collect();
    for context in KEY_CONTEXTS {
        // Within each link, before the clash rule: an override that takes a default's key still leaves the default
        // without it.
        for link in context.links {
            for (index, &a) in link.iter().enumerate() {
                for &b in &link[index + 1..] {
                    if a == b {
                        continue;
                    }
                    for (token_a, spec_a) in keymap.keys(a) {
                        let KeySpec::Chord(chord) = spec_a else { continue };
                        let Some(token_b) = token_for(keymap, b, chord) else {
                            continue;
                        };
                        found.add(keymap, a, b, token_a, ConflictKind::Shares, context.name);
                        found.add(keymap, b, a, token_b, ConflictKind::Shares, context.name);
                    }
                }
            }
        }
        // Across links, after the clash rule: the inner link's key hides the outer one's. Priority bindings come
        // below, since they go first wherever they are.
        let effective: Vec<Vec<(BindingId, &str, KeyChord)>> = context
            .links
            .iter()
            .map(|link| {
                keymap
                    .effective_specs(link)
                    .into_iter()
                    .filter_map(|(id, token, spec)| match spec {
                        KeySpec::Chord(chord) => Some((id, token, *chord)),
                        KeySpec::Unknown(_) => None,
                    })
                    .collect()
            })
            .collect();
        for (inner, keys) in effective.iter().enumerate() {
            for &(a, token_a, chord_a) in keys {
                if a.is_priority() {
                    continue;
                }
                for outer in &effective[inner + 1..] {
                    for &(b, token_b, chord_b) in outer {
                        if a != b && chord_a == chord_b && !b.is_priority() {
                            found.add(keymap, a, b, token_a, ConflictKind::Shadows, context.name);
                            found.add(keymap, b, a, token_b, ConflictKind::ShadowedBy, context.name);
                        }
                    }
                }
                for &(p, chord_p) in &priority {
                    if p != a && chord_p == chord_a {
                        found.add(keymap, a, p, token_a, ConflictKind::Priority, context.name);
                    }
                }
            }
        }
    }
    found.0
}

/// The token of `id`'s keys that is `chord`, if it has it.
fn token_for<'k>(keymap: &'k Keymap, id: BindingId, chord: &KeyChord) -> Option<&'k str> {
    keymap
        .keys(id)
        .iter()
        .find(|(_, spec)| matches!(spec, KeySpec::Chord(key) if key == chord))
        .map(|(token, _)| token.as_str())
}

/// The conflicts found so far, each pair once per kind and key.
#[derive(Default)]
struct Found(Vec<KeyConflict>);

impl Found {
    fn add(
        &mut self,
        keymap: &Keymap,
        id: BindingId,
        other: BindingId,
        token: &str,
        kind: ConflictKind,
        at: &'static str,
    ) {
        let key = keymap.key_display(id, token);
        let existing = self
            .0
            .iter_mut()
            .find(|c| c.id == id && c.other == other && c.kind == kind && c.key == key);
        match existing {
            Some(conflict) if conflict.contexts.contains(&at) => {}
            Some(conflict) => conflict.contexts.push(at),
            None => self.0.push(KeyConflict {
                id,
                other,
                key,
                kind,
                contexts: vec![at],
            }),
        }
    }
}

#[cfg(test)]
mod tests;
