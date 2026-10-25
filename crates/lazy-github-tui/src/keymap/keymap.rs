//! The keymap: which keys fire which binding, after `bindings.overrides`.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::{BindingId, KeyChord, KeySpec};

/// The keys of every binding, with `bindings.overrides` applied as Textual's `App.set_keymap` applies them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    keys: HashMap<BindingId, Vec<(String, KeySpec)>>,
    overridden: HashSet<BindingId>,
    /// Override strings as written, including empty ones, for pane titles.
    raw: HashMap<BindingId, String>,
}

impl Keymap {
    /// Every binding on its default keys, with `overrides` (`bindings.overrides`: id to key list) on top.
    ///
    /// - An overridden binding answers only to its override keys, and its `key_display` is dropped.
    /// - An empty or blank override is ignored, so the binding keeps its defaults. One made only of commas (`" , "`)
    ///   unbinds it, as in Textual, where it isn't blank.
    /// - An override for an id that doesn't exist is ignored. It stays in the config file.
    /// - A token that isn't a key never matches, but it's displayed as written.
    pub fn new(overrides: &IndexMap<String, String>) -> Self {
        let mut keys: HashMap<_, _> = BindingId::ALL
            .iter()
            .map(|&id| (id, KeySpec::parse_list(id.def().default_keys)))
            .collect();
        let mut overridden = HashSet::new();
        let mut raw = HashMap::new();
        for (config_id, value) in overrides {
            let Some(id) = BindingId::from_config_id(config_id) else {
                tracing::debug!("Ignoring binding override for unknown id {config_id}");
                continue;
            };
            raw.insert(id, value.clone());
            // Textual's `apply_keymap` skips only a falsy (blank, after normalization) override.
            if value.trim().is_empty() {
                continue;
            }
            let parsed = KeySpec::parse_list(value);
            for (token, spec) in &parsed {
                if matches!(spec, KeySpec::Unknown(_)) {
                    tracing::warn!("Unknown key {token:?} in bindings.overrides for {config_id}");
                }
            }
            keys.insert(id, parsed);
            overridden.insert(id);
        }
        Self { keys, overridden, raw }
    }

    /// Every binding on its default keys.
    pub fn defaults() -> Self {
        Self::new(&IndexMap::new())
    }

    /// The keys of `id`, each with its token as written: the override's, or the defaults.
    pub fn keys(&self, id: BindingId) -> &[(String, KeySpec)] {
        self.keys.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Whether `bindings.overrides` rebinds `id`. An ignored (empty) override doesn't count.
    pub fn is_overridden(&self, id: BindingId) -> bool {
        self.overridden.contains(&id)
    }

    /// The key text for a pane title such as `[1] Repositories`: the override string exactly as written, even when
    /// it's empty, or the default keys. Python's `LazyGithubContext.get_key`.
    pub fn title_key(&self, id: BindingId) -> &str {
        self.raw.get(&id).map_or(id.def().default_keys, String::as_str)
    }

    /// How the footer shows `id`'s key: its first key.
    pub fn first_key_display(&self, id: BindingId) -> String {
        self.key_displays(id).into_iter().next().unwrap_or_default()
    }

    /// How the help panel shows `id`'s keys, all of them in order. A binding's `key_display` replaces each default
    /// key's display, until the binding is overridden.
    pub fn key_displays(&self, id: BindingId) -> Vec<String> {
        self.keys(id)
            .iter()
            .map(|(token, _)| self.key_display(id, token))
            .collect()
    }

    /// How one of `id`'s keys, `token` as written, is shown: the binding's `key_display` until it's overridden, or
    /// the key's own display (Textual's `App.get_key_display`).
    pub fn key_display(&self, id: BindingId, token: &str) -> String {
        match id.def().key_display.filter(|_| !self.is_overridden(id)) {
            Some(display) => display.to_owned(),
            None => KeySpec::display(token),
        }
    }

    /// The keys of each binding of one component's list `ids`, in Textual's order after a keymap: the keys left at
    /// their defaults in list order, then the overridden bindings' keys in list order. Each comes with its token as
    /// written.
    ///
    /// This is Textual's per-component clash rule: a default key that an overridden binding in the same list claims
    /// is gone. Unknown tokens are kept, as Textual keeps any key string: they never match a press, but the footer
    /// shows them.
    pub fn effective_specs(&self, ids: &[BindingId]) -> Vec<(BindingId, &str, &KeySpec)> {
        let specs = |id: BindingId| {
            self.keys(id)
                .iter()
                .map(move |(token, spec)| (id, token.as_str(), spec))
        };
        let (overridden, defaults): (Vec<BindingId>, Vec<BindingId>) =
            ids.iter().copied().partition(|&id| self.is_overridden(id));
        let claimed: Vec<_> = overridden.iter().flat_map(|&id| specs(id)).collect();
        defaults
            .iter()
            .flat_map(|&id| specs(id))
            .filter(|(_, _, spec)| !claimed.iter().any(|(_, _, claimed)| claimed == spec))
            .chain(claimed.iter().copied())
            .collect()
    }

    /// [`Self::effective_specs`] without the unknown tokens: the chords that fire each binding of `ids`.
    pub fn effective_keys(&self, ids: &[BindingId]) -> Vec<(BindingId, KeyChord)> {
        self.effective_specs(ids)
            .into_iter()
            .filter_map(|(id, _, spec)| match spec {
                KeySpec::Chord(chord) => Some((id, *chord)),
                KeySpec::Unknown(_) => None,
            })
            .collect()
    }

    /// Every binding in one component's list `ids` that `chord` fires, in the order Textual's `_check_bindings`
    /// tries them: [`Self::effective_keys`] order. Dispatch tries each until one applies, then stops; only when none
    /// does it moves on to the next chain link.
    pub fn matches_in(&self, ids: &[BindingId], chord: &KeyChord) -> impl Iterator<Item = BindingId> {
        let chord = *chord;
        self.effective_keys(ids)
            .into_iter()
            .filter(move |(_, key)| *key == chord)
            .map(|(id, _)| id)
    }

    /// The first of [`Self::matches_in`]: an overridden binding wins over a default one, and otherwise the first in
    /// list order wins. A component nearer the focus still wins over one further out: that's the caller walking the
    /// chain innermost first.
    pub fn match_in(&self, ids: &[BindingId], chord: &KeyChord) -> Option<BindingId> {
        self.matches_in(ids, chord).next()
    }

    /// The priority bindings in `ids` that `chord` fires, in [`Self::matches_in`] order. The clash rule applies to
    /// the whole list first, as Textual applies the keymap to a node before checking `priority`: an override onto
    /// `ctrl+q` in the same list takes the key from the built-in quit.
    pub fn priority_matches_in(&self, ids: &[BindingId], chord: &KeyChord) -> impl Iterator<Item = BindingId> {
        self.matches_in(ids, chord).filter(|id| id.is_priority())
    }

    /// The bindings checked before the focused component sees a key.
    pub fn priority_ids(&self) -> impl Iterator<Item = BindingId> {
        BindingId::ALL.iter().copied().filter(|id| id.is_priority())
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crossterm::event::{KeyCode, KeyModifiers};
    use tracing::Level;

    use super::*;

    fn key(token: &str) -> KeyChord {
        match KeySpec::parse(token) {
            KeySpec::Chord(chord) => chord,
            KeySpec::Unknown(token) => panic!("{token:?} didn't parse"),
        }
    }

    fn keymap(overrides: &[(&str, &str)]) -> Keymap {
        Keymap::new(
            &overrides
                .iter()
                .map(|&(id, keys)| (id.to_owned(), keys.to_owned()))
                .collect(),
        )
    }

    /// What `f` logs at `DEBUG` and above, as `LEVEL message` lines.
    fn logs<R>(f: impl FnOnce() -> R) -> (R, String) {
        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::DEBUG)
            .without_time()
            .with_target(false)
            .with_writer(move || writer.clone())
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let out = out
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ") + "\n")
            .collect();
        (result, out)
    }

    /// The overrides verified against the Python app in #73.
    fn python_example() -> Keymap {
        keymap(&[
            ("main.repos.focus", "9"),
            ("main.pull_requests.focus", "x, ctrl+t"),
            ("common.table.search", "question_mark"),
            ("app.open_help", "h"),
            ("app.maximize_widget", "bogus"),
        ])
    }

    const MAIN_FOCUS: &[BindingId] = &[
        BindingId::MainReposFocus,
        BindingId::MainPullRequestsFocus,
        BindingId::MainIssuesFocus,
        BindingId::MainWorkflowsFocus,
        BindingId::MainDetailsFocus,
        BindingId::MainCommandLogFocus,
    ];

    #[test]
    fn the_python_example_end_to_end() {
        let keymap = python_example();
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("9")), Some(BindingId::MainReposFocus));
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("1")), None);
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("2")), None);
        assert_eq!(
            keymap.match_in(MAIN_FOCUS, &key("x")),
            Some(BindingId::MainPullRequestsFocus)
        );
        assert_eq!(
            keymap.match_in(MAIN_FOCUS, &key("ctrl+t")),
            Some(BindingId::MainPullRequestsFocus)
        );
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("3")), Some(BindingId::MainIssuesFocus));

        assert_eq!(keymap.title_key(BindingId::MainReposFocus), "9");
        assert_eq!(keymap.title_key(BindingId::MainPullRequestsFocus), "x, ctrl+t");
        assert_eq!(keymap.title_key(BindingId::MainIssuesFocus), "3");

        assert_eq!(keymap.first_key_display(BindingId::TableSearch), "?");
        assert_eq!(keymap.first_key_display(BindingId::AppMaximizeWidget), "bogus");
        assert_eq!(keymap.first_key_display(BindingId::AppOpenHelp), "h");
        assert_eq!(
            keymap.match_in(&[BindingId::TableSearch], &key("?")),
            Some(BindingId::TableSearch)
        );
        assert_eq!(keymap.match_in(&[BindingId::TableSearch], &key("/")), None);
        assert_eq!(keymap.match_in(&[BindingId::AppMaximizeWidget], &key("M")), None);

        // The focused table's own `h` comes first in the chain, so the help binding never sees it: "h Keybindings"
        // is missing from Python's footer.
        let chain: [&[BindingId]; 2] = [
            &[BindingId::TableScrollLeft],
            &[BindingId::AppQuit, BindingId::AppOpenHelp],
        ];
        let winner = chain.iter().find_map(|ids| keymap.match_in(ids, &key("h")));
        assert_eq!(winner, Some(BindingId::TableScrollLeft));
    }

    #[test]
    fn an_override_clashes_with_defaults_in_the_same_component() {
        // `X` stands for any binding; `diff.add_comment` (default `c`).
        let keymap = keymap(&[("diff.add_comment", "h")]);
        let table = [BindingId::TableScrollLeft, BindingId::DiffAddComment];
        assert_eq!(keymap.match_in(&table, &key("h")), Some(BindingId::DiffAddComment));
        assert_eq!(keymap.match_in(&table, &key("c")), None);
        assert_eq!(keymap.effective_keys(&table), [(BindingId::DiffAddComment, key("h"))]);

        // A component nearer the focus that declares `h` still wins in chain order.
        let chain: [&[BindingId]; 2] = [&[BindingId::TableScrollLeft], &[BindingId::DiffAddComment]];
        let winner = chain.iter().find_map(|ids| keymap.match_in(ids, &key("h")));
        assert_eq!(winner, Some(BindingId::TableScrollLeft));
    }

    #[test]
    fn effective_keys_put_overrides_last() {
        let keymap = keymap(&[("common.table.cursor_down", "n")]);
        let ids = [
            BindingId::TableSelect,
            BindingId::TableCursorDown,
            BindingId::TableCursorUp,
        ];
        assert_eq!(
            keymap.effective_keys(&ids),
            [
                (BindingId::TableSelect, key("enter")),
                (BindingId::TableSelect, key("space")),
                (BindingId::TableCursorUp, key("k")),
                (BindingId::TableCursorDown, key("n")),
            ]
        );
        // Every chord `effective_keys` lists resolves to its binding, so dispatch and the footer agree.
        for (id, chord) in keymap.effective_keys(&ids) {
            assert_eq!(keymap.match_in(&ids, &chord), Some(id));
        }
    }

    #[test]
    fn same_default_keys_resolve_in_list_order() {
        let keymap = Keymap::defaults();
        let ids = [BindingId::ConversationReviewReply, BindingId::ConversationCommentReply];
        assert_eq!(
            keymap.match_in(&ids, &key("r")),
            Some(BindingId::ConversationReviewReply)
        );
        // Dispatch falls through to the second when the first doesn't apply.
        assert_eq!(
            keymap.matches_in(&ids, &key("r")).collect::<Vec<_>>(),
            [BindingId::ConversationReviewReply, BindingId::ConversationCommentReply]
        );
    }

    #[test]
    fn matches_in_puts_overrides_after_defaults() {
        let keymap = keymap(&[("common.table.select", "enter")]);
        let table = [
            BindingId::TableSelect,
            BindingId::TextualTableSelect,
            BindingId::WorkflowsViewRunDetails,
        ];
        // The override claims Enter, so the defaults that held it lose it.
        assert_eq!(
            keymap.matches_in(&table, &key("enter")).collect::<Vec<_>>(),
            [BindingId::TableSelect]
        );
        let keymap = Keymap::defaults();
        assert_eq!(
            keymap.matches_in(&table, &key("enter")).collect::<Vec<_>>(),
            [
                BindingId::TableSelect,
                BindingId::TextualTableSelect,
                BindingId::WorkflowsViewRunDetails
            ]
        );
    }

    #[test]
    fn an_override_onto_ctrl_q_takes_it_from_the_priority_quit() {
        let ids = [BindingId::TextualQuit, BindingId::AppOpenHelp];
        let ctrl_q = key("ctrl+q");
        let keymap = keymap(&[("app.open_help", "ctrl+q")]);
        assert_eq!(keymap.priority_matches_in(&ids, &ctrl_q).next(), None);
        assert_eq!(keymap.match_in(&ids, &ctrl_q), Some(BindingId::AppOpenHelp));
        assert_eq!(
            Keymap::defaults()
                .priority_matches_in(&ids, &ctrl_q)
                .collect::<Vec<_>>(),
            [BindingId::TextualQuit]
        );
    }
    #[test]
    fn a_comma_only_override_unbinds() {
        let keymap = keymap(&[("main.repos.focus", " , ")]);
        assert!(keymap.is_overridden(BindingId::MainReposFocus));
        assert_eq!(keymap.keys(BindingId::MainReposFocus), []);
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("1")), None);
        assert_eq!(keymap.title_key(BindingId::MainReposFocus), " , ");
    }

    #[test]
    fn an_empty_override_keeps_the_default() {
        for blank in ["", "  "] {
            let keymap = keymap(&[("main.repos.focus", blank)]);
            assert!(!keymap.is_overridden(BindingId::MainReposFocus));
            assert_eq!(keymap.match_in(MAIN_FOCUS, &key("1")), Some(BindingId::MainReposFocus));
            assert_eq!(keymap.title_key(BindingId::MainReposFocus), blank);
        }
    }

    #[test]
    fn an_unknown_id_is_ignored() {
        let (keymap, out) = logs(|| keymap(&[("app.nope", "z"), ("textual.app.quit", "z")]));
        assert_eq!(keymap, Keymap::defaults());
        assert_eq!(
            out,
            "DEBUG Ignoring binding override for unknown id app.nope\n\
             DEBUG Ignoring binding override for unknown id textual.app.quit\n"
        );
    }

    #[test]
    fn an_unknown_key_warns_once_and_never_matches() {
        let (keymap, out) = logs(|| keymap(&[("app.maximize_widget", "bogus, Z")]));
        assert_eq!(
            out,
            "WARN Unknown key \"bogus\" in bindings.overrides for app.maximize_widget\n"
        );
        let ids = [BindingId::AppMaximizeWidget];
        assert_eq!(keymap.match_in(&ids, &key("M")), None);
        assert_eq!(keymap.match_in(&ids, &key("Z")), Some(BindingId::AppMaximizeWidget));
        assert_eq!(keymap.key_displays(BindingId::AppMaximizeWidget), ["bogus", "Z"]);
        assert_eq!(keymap.effective_keys(&ids), [(BindingId::AppMaximizeWidget, key("Z"))]);
    }

    #[test]
    fn overriding_drops_key_display() {
        let keymap = keymap(&[("app.open_help", "h")]);
        assert_eq!(keymap.first_key_display(BindingId::AppOpenHelp), "h");
        assert_eq!(Keymap::defaults().first_key_display(BindingId::AppOpenHelp), "?");
        let keymap = self::keymap(&[("common.table.page_right", "ctrl+right")]);
        assert_eq!(keymap.first_key_display(BindingId::TablePageRight), "^→");
    }

    #[test]
    fn default_displays_match_the_python_footer_and_help() {
        let keymap = Keymap::defaults();
        for (id, expected) in [
            (BindingId::AppQuit, "q"),
            (BindingId::AppCommandPalette, "^p"),
            (BindingId::AppMaximizeWidget, "M"),
            (BindingId::AppOpenHelp, "?"),
            (BindingId::RepositoriesFavorite, "^f"),
            (BindingId::LinkPaste, "^v"),
            (BindingId::TablePageRight, "$"),
            (BindingId::TablePageLeft, "^"),
            (BindingId::TableSearch, "/"),
            (BindingId::TableSelect, "⏎ space"),
            (BindingId::ModalClose, "q esc"),
            (BindingId::ModalSubmit, "shift+⏎"),
            (BindingId::NotificationsOpen, "^n"),
            (BindingId::NotificationsMarkAllAsRead, "^r"),
            (BindingId::PullRequestMerge, "^m"),
            (BindingId::DiffClearSelection, "esc"),
            (BindingId::WorkflowsViewRunDetails, "⏎"),
            (BindingId::MainReposFocus, "1"),
            (BindingId::TextualQuit, "^q"),
            (BindingId::TextualCopyText, "^c super+c"),
            (BindingId::TextualTablePageLeft, "^pgup"),
            (BindingId::TextualScrollDown, "↓"),
        ] {
            assert_eq!(keymap.key_displays(id).join(" "), expected, "{id:?}");
        }
    }

    #[test]
    fn python_written_configs_load() {
        // What Python's Settings modal records: Textual key-event names.
        let keymap = keymap(&[
            ("app.open_help", "question_mark"),
            ("common.table.search", "slash"),
            ("common.table.page_right", "dollar_sign"),
            ("main.repos.focus", "ctrl+1"),
        ]);
        for (id, pressed) in [
            (
                BindingId::AppOpenHelp,
                KeyChord::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            ),
            (
                BindingId::TableSearch,
                KeyChord::new(KeyCode::Char('/'), KeyModifiers::NONE),
            ),
            (
                BindingId::TablePageRight,
                KeyChord::new(KeyCode::Char('$'), KeyModifiers::SHIFT),
            ),
            (
                BindingId::MainReposFocus,
                KeyChord::new(KeyCode::Char('1'), KeyModifiers::CONTROL),
            ),
        ] {
            assert!(keymap.is_overridden(id));
            assert_eq!(keymap.match_in(&[id], &pressed), Some(id), "{id:?}");
        }
        assert_eq!(keymap.first_key_display(BindingId::TablePageRight), "$");
        assert_eq!(keymap.first_key_display(BindingId::MainReposFocus), "^1");
        assert_eq!(keymap.match_in(MAIN_FOCUS, &key("1")), None);
    }

    #[test]
    fn priority_ids_are_ctrl_q() {
        assert_eq!(
            Keymap::defaults().priority_ids().collect::<Vec<_>>(),
            [BindingId::TextualQuit]
        );
    }
}
