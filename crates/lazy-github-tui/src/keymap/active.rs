//! The bindings active for the current focus, one per key, as Textual's `Screen.active_bindings` finds them. The
//! footer (#107) and the help panel (#108) group them each their own way, as Textual's `Footer` and `KeyPanel` do.

use super::{BindingDef, BindingId, KeySpec, Keymap};
use crate::component::{BindingState, ChainLink, Owner};

/// One active key: the binding that claimed it, and the chain link that declares the binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveBinding {
    /// The chain link that declares the binding, which handles it.
    pub owner: Owner,
    /// The binding.
    pub id: BindingId,
    /// The key, as displayed (`^p`, `?`, `bogus`).
    pub key: String,
    /// False for a binding shown dimmed ([`BindingState::Disabled`]).
    pub enabled: bool,
}

impl ActiveBinding {
    /// The binding's registry entry: its action, description, tooltip and flags.
    pub fn def(&self) -> &'static BindingDef {
        self.id.def()
    }
}

/// The active keys of `chain` (innermost first), in Textual's `Screen.active_bindings` order.
///
/// - Links are walked innermost first, and each link's bindings in [`Keymap::link_specs`] order, which drops the
///   keys a focused input types from the links behind it.
/// - Each key goes to the first binding that has it. A binding whose keys are all taken by then has no entry.
/// - A priority binding takes a key from a non-priority one further in, at that key's place, as Textual replaces
///   the entry in its key-ordered dict.
/// - A binding whose `state` is [`BindingState::Hidden`] is skipped and claims nothing, like Textual's
///   `check_action` returning `False`. A disabled one is kept, with `enabled` false.
pub fn active_bindings(
    chain: &[ChainLink],
    keymap: &Keymap,
    state: impl Fn(Owner, BindingId) -> BindingState,
) -> Vec<ActiveBinding> {
    // Parallel to `active`: the key each entry claimed.
    let mut claimed: Vec<&KeySpec> = Vec::new();
    let mut active: Vec<ActiveBinding> = Vec::new();
    for link in chain {
        for (id, token, spec) in keymap.link_specs(link) {
            let binding_state = state(link.owner, id);
            if binding_state == BindingState::Hidden {
                continue;
            }
            let binding = ActiveBinding {
                owner: link.owner,
                id,
                key: keymap.key_display(id, token),
                enabled: binding_state == BindingState::Enabled,
            };
            match claimed.iter().position(|&taken| taken == spec) {
                Some(i) if id.is_priority() && !active[i].id.is_priority() => active[i] = binding,
                Some(_) => {}
                None => {
                    claimed.push(spec);
                    active.push(binding);
                }
            }
        }
    }
    active
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::component::ScreenKind;
    use crate::screens::main::SectionId;

    fn keymap(overrides: &[(&str, &str)]) -> Keymap {
        let overrides: IndexMap<String, String> = overrides
            .iter()
            .map(|&(id, keys)| (id.to_owned(), keys.to_owned()))
            .collect();
        Keymap::new(&overrides)
    }

    fn link(owner: Owner, bindings: &'static [BindingId]) -> ChainLink {
        ChainLink::new(owner, bindings)
    }

    fn enabled(_: Owner, _: BindingId) -> BindingState {
        BindingState::Enabled
    }

    /// `"<key> <description>"` for each key.
    fn entries(active: &[ActiveBinding]) -> Vec<String> {
        active
            .iter()
            .map(|binding| format!("{} {}", binding.key, binding.def().description))
            .collect()
    }

    #[test]
    fn the_first_key_wins_across_the_chain() {
        // Both `O` bindings: the inner one (the PR section) wins.
        let chain = [
            link(Owner::Screen(ScreenKind::Main), &[BindingId::PullRequestLookup]),
            link(Owner::App, &[BindingId::IssueLookup]),
        ];
        let active = active_bindings(&chain, &Keymap::defaults(), enabled);
        assert_eq!(entries(&active), ["O Lookup Pull Request"]);
        assert_eq!(active[0].owner, Owner::Screen(ScreenKind::Main));
        assert_eq!(active[0].id, BindingId::PullRequestLookup);
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
    fn every_key_is_its_own_entry() {
        // An overridden binding's keys go after the defaults, one entry each.
        let chain = [link(
            Owner::App,
            &[
                BindingId::TextualQuit,
                BindingId::AppQuit,
                BindingId::LinkFollow,
                BindingId::TableSelect,
            ],
        )];
        let keymap = keymap(&[("link.follow", "f, x")]);
        assert_eq!(
            entries(&active_bindings(&chain, &keymap, enabled)),
            [
                "^q Quit",
                "q Quit",
                "⏎ Select table entry",
                "space Select table entry",
                "f Follow link",
                "x Follow link"
            ]
        );
    }

    #[test]
    fn a_priority_binding_takes_the_key_in_place() {
        // `ctrl+q` moved onto the PR lookup further in (overridden keys go last in their link): the app's priority
        // quit takes it back, at the inner place.
        let chain = [
            link(
                Owner::Section(SectionId::PullRequests),
                &[BindingId::PullRequestLookup, BindingId::PullRequestEdit],
            ),
            link(Owner::App, &[BindingId::TextualQuit, BindingId::AppQuit]),
        ];
        let keymap = keymap(&[("pull_request.lookup", "ctrl+q")]);
        let active = active_bindings(&chain, &keymap, enabled);
        assert_eq!(entries(&active), ["E Edit PR", "^q Quit", "q Quit"]);
        assert_eq!(active[1].owner, Owner::App);
        assert_eq!(active[1].id, BindingId::TextualQuit);
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
        let active = active_bindings(&chain, &Keymap::defaults(), state);
        // The screen's hidden ^n leaves the key to the app's.
        assert_eq!(entries(&active), ["^v Paste link", "^n Notifications"]);
        assert_eq!(active[1].owner, Owner::App);
        assert!(!active[0].enabled);
        assert!(active[1].enabled);
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
            link(Owner::Table(SectionId::Repos), &[BindingId::TableScrollLeft]),
            link(Owner::SearchableTable(SectionId::Repos), &[BindingId::TableSearch]),
            link(
                Owner::App,
                &[BindingId::AppQuit, BindingId::AppMaximizeWidget, BindingId::AppOpenHelp],
            ),
        ];
        let active = active_bindings(&chain, &keymap, enabled);
        assert_eq!(
            entries(&active),
            ["h Table scroll left", "? Search", "q Quit", "bogus Maximize"]
        );
    }
}
