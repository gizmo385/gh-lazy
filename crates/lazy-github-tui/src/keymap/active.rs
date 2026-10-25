//! The bindings active for the current focus, as Textual's `Screen.active_bindings` finds them, grouped the way its
//! `Footer` and `KeyPanel` show them. The footer (#107) and the help panel (#108) both draw from [`active_bindings`].

use super::{BindingId, KeySpec, Keymap};
use crate::component::{BindingState, ChainLink, Owner};

/// The bindings of one link that share an action, shown as one footer or help entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGroup {
    /// The chain link that declares them, which handles a click on the entry.
    pub owner: Owner,
    /// The first binding of the group: what a click on the entry runs.
    pub id: BindingId,
    /// The Textual action they share.
    pub action: &'static str,
    /// The first binding's label.
    pub description: &'static str,
    /// The first binding's tooltip, or empty.
    pub tooltip: &'static str,
    /// Every key still claimed by the group, as displayed (`^p`, `?`, `bogus`), in chain order.
    pub keys: Vec<String>,
    /// Whether the footer lists the group.
    pub show: bool,
    /// False for a group shown dimmed ([`BindingState::Disabled`]).
    pub enabled: bool,
}

impl ActiveGroup {
    /// The key the footer shows: the first one claimed.
    pub fn first_key(&self) -> &str {
        self.keys.first().map_or("", String::as_str)
    }
}

/// The active bindings of `chain` (innermost first), grouped by owner and action in first-seen order.
///
/// - Links are walked innermost first, and each link's bindings in [`Keymap::effective_specs`] order.
/// - Each key goes to the first binding that has it. A binding whose keys are all taken by then disappears.
/// - A binding whose `state` is [`BindingState::Hidden`] is skipped and claims nothing, like Textual's
///   `check_action` returning `False`. A disabled one is kept, with `enabled` false.
/// - Shown and hidden bindings of the same action never share a group: Textual drops `show=False` bindings before it
///   groups by action, so the hidden `ctrl+q` quit doesn't take `q Quit` with it.
pub fn active_bindings(
    chain: &[ChainLink],
    keymap: &Keymap,
    state: impl Fn(Owner, BindingId) -> BindingState,
) -> Vec<ActiveGroup> {
    let mut claimed: Vec<&KeySpec> = Vec::new();
    let mut groups: Vec<ActiveGroup> = Vec::new();
    for link in chain {
        for (id, token, spec) in keymap.effective_specs(link.bindings) {
            let binding_state = state(link.owner, id);
            if binding_state == BindingState::Hidden || claimed.contains(&spec) {
                continue;
            }
            claimed.push(spec);
            let def = id.def();
            let key = keymap.key_display(id, token);
            let same =
                |group: &ActiveGroup| group.owner == link.owner && group.action == def.action && group.show == def.show;
            match groups.iter_mut().find(|group| same(group)) {
                Some(group) => group.keys.push(key),
                None => groups.push(ActiveGroup {
                    owner: link.owner,
                    id,
                    action: def.action,
                    description: def.description,
                    tooltip: def.tooltip,
                    keys: vec![key],
                    show: def.show,
                    enabled: binding_state == BindingState::Enabled,
                }),
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::component::ScreenKind;

    fn keymap(overrides: &[(&str, &str)]) -> Keymap {
        let overrides: IndexMap<String, String> = overrides
            .iter()
            .map(|&(id, keys)| (id.to_owned(), keys.to_owned()))
            .collect();
        Keymap::new(&overrides)
    }

    fn link(owner: Owner, bindings: &'static [BindingId]) -> ChainLink {
        ChainLink { owner, bindings }
    }

    fn enabled(_: Owner, _: BindingId) -> BindingState {
        BindingState::Enabled
    }

    /// `"<key> <description>"` for each group, all keys joined with a space.
    fn entries(groups: &[ActiveGroup]) -> Vec<String> {
        groups
            .iter()
            .map(|group| format!("{} {}", group.keys.join(" "), group.description))
            .collect()
    }

    #[test]
    fn the_first_key_wins_across_the_chain() {
        // Both `O` bindings: the inner one (the PR section) wins.
        let chain = [
            link(Owner::Screen(ScreenKind::Main), &[BindingId::PullRequestLookup]),
            link(Owner::App, &[BindingId::IssueLookup]),
        ];
        let groups = active_bindings(&chain, &Keymap::defaults(), enabled);
        assert_eq!(entries(&groups), ["O Lookup Pull Request"]);
        assert_eq!(groups[0].owner, Owner::Screen(ScreenKind::Main));
        assert_eq!(groups[0].id, BindingId::PullRequestLookup);
    }

    #[test]
    fn a_binding_whose_keys_are_all_taken_disappears() {
        // `modal.close` is `q, escape`: with both taken further in, it's gone. With only `q` taken, it keeps `esc`.
        let both = [
            link(Owner::MainView, &[BindingId::AppQuit, BindingId::DiffClearSelection]),
            link(Owner::App, &[BindingId::ModalClose]),
        ];
        assert_eq!(
            entries(&active_bindings(&both, &Keymap::defaults(), enabled)),
            ["q Quit", "esc Clear Selection"]
        );
        let one = [
            link(Owner::MainView, &[BindingId::AppQuit]),
            link(Owner::App, &[BindingId::ModalClose]),
        ];
        assert_eq!(
            entries(&active_bindings(&one, &Keymap::defaults(), enabled)),
            ["q Quit", "esc Close"]
        );
    }

    #[test]
    fn same_action_bindings_merge_within_an_owner() {
        // `textual.app.quit` and `app.quit` are both `quit`, and the hidden one comes first. An overridden binding's
        // keys go last, and all of them stay in its group.
        let chain = [link(
            Owner::App,
            &[
                BindingId::TextualQuit,
                BindingId::AppQuit,
                BindingId::LinkFollow,
                BindingId::NotificationsMarkRead,
            ],
        )];
        let keymap = keymap(&[("link.follow", "f, x")]);
        let groups = active_bindings(&chain, &keymap, enabled);
        let shown: Vec<&ActiveGroup> = groups.iter().filter(|group| group.show).collect();
        assert_eq!(
            shown
                .iter()
                .map(|g| (g.description, g.keys.clone()))
                .collect::<Vec<_>>(),
            [
                ("Quit", vec!["q".to_owned()]),
                ("Mark as Read", vec!["R".to_owned()]),
                ("Follow link", vec!["f".to_owned(), "x".to_owned()]),
            ]
        );
        // A shown and a hidden binding with one action stay apart: the hidden one keeps only the key left to it.
        let chain = [link(
            Owner::Table(crate::screens::main::SectionId::Workflows),
            &[BindingId::WorkflowsViewRunDetails, BindingId::TableSelect],
        )];
        let groups = active_bindings(&chain, &Keymap::defaults(), enabled);
        assert_eq!(entries(&groups), ["⏎ View Run Details", "space Select table entry"]);
        assert!(groups[0].show && !groups[1].show);
    }

    #[test]
    fn hidden_is_skipped_and_claims_nothing() {
        let chain = [
            link(
                Owner::Screen(ScreenKind::Main),
                &[BindingId::NotificationsOpen, BindingId::LinkPaste],
            ),
            link(Owner::App, &[BindingId::NotificationsOpen]),
        ];
        let state = |owner: Owner, id: BindingId| match (owner, id) {
            (Owner::Screen(_), BindingId::NotificationsOpen) => BindingState::Hidden,
            (_, BindingId::LinkPaste) => BindingState::Disabled,
            _ => BindingState::Enabled,
        };
        let groups = active_bindings(&chain, &Keymap::defaults(), state);
        // The screen's hidden ^n leaves the key to the app's.
        assert_eq!(entries(&groups), ["^v Paste link", "^n Notifications"]);
        assert_eq!(groups[1].owner, Owner::App);
        assert!(!groups[0].enabled);
        assert!(groups[1].enabled);
    }

    #[test]
    fn the_override_example_drops_h_keybindings() {
        // #73's example: the focused table's hidden `h` takes the key `app.open_help` was moved to.
        let keymap = keymap(&[
            ("common.table.search", "question_mark"),
            ("app.open_help", "h"),
            ("app.maximize_widget", "bogus"),
        ]);
        let chain = [
            link(
                Owner::Table(crate::screens::main::SectionId::Repos),
                &[BindingId::TableScrollLeft],
            ),
            link(
                Owner::SearchableTable(crate::screens::main::SectionId::Repos),
                &[BindingId::TableSearch],
            ),
            link(
                Owner::App,
                &[BindingId::AppQuit, BindingId::AppMaximizeWidget, BindingId::AppOpenHelp],
            ),
        ];
        let groups = active_bindings(&chain, &keymap, enabled);
        let shown: Vec<String> = entries(&groups).into_iter().skip(1).collect();
        assert_eq!(shown, ["? Search", "q Quit", "bogus Maximize"]);
        assert!(!groups[0].show);
    }
}
