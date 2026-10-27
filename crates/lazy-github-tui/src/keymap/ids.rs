//! Binding ids and the registry of their defaults.
//!
//! The registry is `LazyGithubBindings.all()` from `lazy_github/lib/bindings.py`, in that order, followed by the
//! Textual built-ins the app relies on. `tests/registry.rs` checks it against a dump of the Python registry.

/// What a binding is: its default keys and how the footer and help panel show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingDef {
    /// The binding.
    pub id: BindingId,
    /// The full id: the `bindings.overrides` key (`app.quit`), or an internal `textual.*` id for a built-in.
    pub name: &'static str,
    /// The default keys, in Textual's syntax as `bindings.py` writes them (`"q, escape"`, `"ctrl+p"`).
    pub default_keys: &'static str,
    /// The Textual action name. Bindings with the same action in one component are one footer and help entry.
    pub action: &'static str,
    /// The footer and help panel label. Empty for bindings that are never shown.
    pub description: &'static str,
    /// The footer tooltip. Empty when there is none.
    pub tooltip: &'static str,
    /// Whether the footer and help panel list the binding. Already false when `description` is empty, as Textual
    /// computes it.
    pub show: bool,
    /// How the default key is shown, when it isn't derived from the key. Dropped once the binding is overridden.
    pub key_display: Option<&'static str>,
    /// Checked before the focused component sees the key (Textual's `priority=True`).
    pub priority: bool,
    /// Left out of the help panel (Textual's `system=True`).
    pub system: bool,
}

impl BindingDef {
    const fn new(
        id: BindingId,
        name: &'static str,
        default_keys: &'static str,
        action: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            name,
            default_keys,
            action,
            description,
            tooltip: "",
            show: !description.is_empty(),
            key_display: None,
            priority: false,
            system: false,
        }
    }

    const fn tooltip(mut self, tooltip: &'static str) -> Self {
        self.tooltip = tooltip;
        self
    }

    const fn hidden(mut self) -> Self {
        self.show = false;
        self
    }

    const fn key_display(mut self, key_display: &'static str) -> Self {
        self.key_display = Some(key_display);
        self
    }

    const fn priority(mut self) -> Self {
        self.priority = true;
        self
    }

    const fn system(mut self) -> Self {
        self.system = true;
        self
    }
}

/// Declares `BindingId`, `BindingId::ALL` and the registry from one list, so the three can't drift apart.
macro_rules! registry {
    ($($variant:ident($name:literal, $keys:literal, $action:literal, $description:literal) $(.$method:ident($($arg:expr)?))*;)*) => {
        /// A binding, by the id Python uses for it (`app.quit`) or, for a Textual built-in, an internal `textual.*`
        /// id.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum BindingId {
            $(
                #[doc = concat!("`", $name, "`: `", $keys, "`.")]
                $variant,
            )*
        }

        impl BindingId {
            /// Every id, in registry order.
            pub const ALL: &'static [BindingId] = &[$(BindingId::$variant,)*];
        }

        /// Indexed by `BindingId as usize`, which is the variant's position in `ALL`.
        static REGISTRY: &[BindingDef] = &[
            $(BindingDef::new(BindingId::$variant, $name, $keys, $action, $description) $(.$method($($arg)?))*,)*
        ];
    };
}

registry! {
    // `LazyGithubBindings.all()`. `common.table.page_left`/`page_right` are only the later `^`/`$` definitions,
    // which replace `H`/`L` in Python (Py bug B9).
    AppQuit("app.quit", "q", "quit", "Quit").tooltip("Exit LazyGithub");
    // Python says "Command Pallete" (Py bug B14).
    AppCommandPalette("app.command_palette", "ctrl+p", "command_palette", "Command Palette");
    AppMaximizeWidget("app.maximize_widget", "M", "maximize", "Maximize")
        .tooltip("Fullscreen the currently focused element");
    AppOpenHelp("app.open_help", "?", "open_help", "Keybindings").key_display("?").tooltip("Toggle this window");
    ConversationCommentNew("conversation.comment.new", "n", "new_comment", "New Comment")
        .tooltip("Create a new comment in the thread");
    ConversationReviewReply("conversation.review.reply", "r", "reply_to_review", "Reply to Review")
        .tooltip("Reply to the selected Github review");
    ConversationCommentReply("conversation.comment.reply", "r", "reply_to_individual_comment", "Reply to Comment")
        .tooltip("Reply to the selected Github comment");
    IssueNew("issue.new", "I", "open_issue", "New Issue").tooltip("Open a new issue in the current repository");
    IssueEdit("issue.edit", "E", "edit_issue", "Edit Issue").tooltip("Edit the selected issue");
    IssueLookup("issue.lookup", "O", "lookup_issue", "Lookup Issue")
        .tooltip("Lookup an issue in the current repository");
    PullRequestNew("pull_request.new", "P", "open_pull_request", "New PR")
        .tooltip("Open a new pull request in the current repository");
    // Fires only on a real ctrl+m (kitty keyboard protocol), never on Enter (Py bug B30).
    PullRequestMerge("pull_request.merge", "ctrl+m", "merge_pull_request", "Merge PR")
        .tooltip("Merge the selected pull request");
    PullRequestEdit("pull_request.edit", "E", "edit_pull_request", "Edit PR").tooltip("Edit the selected pull request");
    PullRequestLookup("pull_request.lookup", "O", "lookup_pull_request", "Lookup Pull Request")
        .tooltip("Lookup a pull request in the current repository");
    RepositoriesFavorite("repositories.favorite", "ctrl+f", "toggle_favorite_repo", "Toggle Favorite")
        .tooltip("Mark/unmark a repository as favorited, which are displayed at the top of the repos table");
    RepositoriesLookup("repositories.lookup", "O", "lookup_repository", "Lookup Repository");
    UserViewProfile("user.view_profile", "u", "view_user_profile", "View User")
        .tooltip("Open the GitHub profile for the author/owner of the highlighted row");
    TableSelect("common.table.select", "enter,space", "select_cursor", "Select table entry").hidden();
    TableSearch("common.table.search", "/", "focus_search", "Search").tooltip("Search in the table");
    TableCursorDown("common.table.cursor_down", "j", "cursor_down", "Table cursor down").hidden();
    TablePageDown("common.table.page_down", "J", "page_down", "Table page down").hidden();
    TableCursorUp("common.table.cursor_up", "k", "cursor_up", "Table cursor up").hidden();
    TablePageUp("common.table.page_up", "K", "page_up", "Table page up").hidden();
    TableScrollRight("common.table.scroll_right", "l", "scroll_right", "Table scroll right").hidden();
    TablePageRight("common.table.page_right", "$", "page_right", "Table page right").key_display("$").hidden();
    TableScrollLeft("common.table.scroll_left", "h", "scroll_left", "Table scroll left").hidden();
    TablePageLeft("common.table.page_left", "^", "page_left", "Table page left").key_display("^").hidden();
    TableScrollTop("common.table.scroll_top", "g", "scroll_top", "Table scroll to top").hidden();
    TableScrollBottom("common.table.scroll_bottom", "G", "scroll_bottom", "Table scroll to bottom").hidden();
    WorkflowsTrigger("workflows.trigger", "T", "trigger_workflow", "Trigger Workflow");
    WorkflowsViewRunDetails("workflows.view_run_details", "enter", "select_cursor", "View Run Details");
    WorkflowsViewJobLogs("workflows.view_job_logs", "l", "view_logs", "View Job Logs");
    DiffAddComment("diff.add_comment", "c", "add_comment", "Add Comment");
    DiffCursorDown("diff.cursor_down", "j", "cursor_down", "");
    DiffCursorUp("diff.cursor_up", "k", "cursor_up", "");
    DiffClearSelection("diff.clear_selection", "escape", "clear_selection", "Clear Selection");
    DiffNextHunk("diff.next_hunk", "J", "next_hunk", "Next Hunk");
    DiffPreviousHunk("diff.previous_hunk", "K", "previous_hunk", "Previous Hunk");
    DiffFilterFiles("diff.filter_files", "/", "focus_file_filter", "Filter Files");
    // Shown, and active only while `notifications.enabled` (Py bug B7: Python decides `show` once at import). The
    // component that declares it hides it through `Component::binding_state`.
    NotificationsOpen("notifications.open", "ctrl+n", "view_notifications", "Notifications")
        .tooltip("Open the notifications modal");
    NotificationsMarkAllAsRead("notifications.mark_all_as_read", "ctrl+r", "mark_all_read", "Mark all as read")
        .tooltip("Mark all unread notifications as read");
    NotificationsMarkRead("notifications.mark_read", "R", "mark_read", "Mark as Read")
        .tooltip("Mark a single notification as read");
    NotificationsViewRead("notifications.view_read", "r", "view_read", "View Read Notifications")
        .tooltip("Swap to the viewed notifications tab");
    NotificationsViewUnread("notifications.view_unread", "u", "view_unread", "View Unread Notifications")
        .tooltip("Swap to the viewed notifications tab");
    // Needs the kitty keyboard protocol: elsewhere shift+enter arrives as plain Enter. The Submit button is the
    // fallback.
    ModalSubmit("modal.submit", "shift+enter", "submit", "Submit")
        .tooltip("Submit whatever is being created in the current dialog");
    ModalClose("modal.close", "q, escape", "close", "Close").tooltip("Close the current dialog");
    ModalSearch("modal.search", "/", "search", "Search").tooltip("Search within a dialog window");
    ConfirmYes("confirm.yes", "y", "confirm_yes", "Yes");
    ConfirmNo("confirm.no", "n", "confirm_no", "No");
    ReactAdd("react.add", "R", "add_reaction", "Add reaction");
    LinkPaste("link.paste", "ctrl+v", "open_link_paste_modal", "Paste link");
    LinkFollow("link.follow", "f", "follow_link", "Follow link").tooltip("Show hints on visible links and pick one");
    LinkFollowExternal("link.follow_external", "F", "follow_link_external", "Follow link (browser)")
        .tooltip("Show hints on visible links and open the pick in the system browser");
    MainReposFocus("main.repos.focus", "1", "focus_section('#repos_table')", "Focus repos table")
        .hidden()
        .tooltip("Focus the table of accessible repositories");
    MainPullRequestsFocus("main.pull_requests.focus", "2", "focus_section('#pull_requests_table')", "Focus PRs table")
        .hidden()
        .tooltip("Focus the table of pull requests for the repo");
    MainIssuesFocus("main.issues.focus", "3", "focus_section('#issues_table')", "Focus issues table")
        .hidden()
        .tooltip("Focus the table of issues for the repo");
    MainWorkflowsFocus("main.workflows.focus", "4", "focus_workflow_tabs", "Focus workflows table")
        .hidden()
        .tooltip("Focus and swap between the workflow tabs");
    MainDetailsFocus("main.details.focus", "5", "focus_tabs", "Focus details tabs")
        .hidden()
        .tooltip("Focus and swap between the selection detail tabs on the right");
    MainCommandLogFocus("main.command_log.focus", "6", "focus_section('LazyGithubCommandLog')", "Focus command log")
        .hidden()
        .tooltip("Focus the command log, where actions taken by LazyGithub are shown");

    // Textual built-ins. Python has no ids for them, so overrides never reach them. All are hidden.
    TextualQuit("textual.app.quit", "ctrl+q", "quit", "Quit")
        .tooltip("Quit the app and return to the command prompt.")
        .hidden()
        .priority();
    TextualHelpQuit("textual.app.help_quit", "ctrl+c", "help_quit", "").system();
    TextualFocusNext("textual.screen.focus_next", "tab", "app.focus_next", "Focus Next").hidden();
    TextualFocusPrevious("textual.screen.focus_previous", "shift+tab", "app.focus_previous", "Focus Previous").hidden();
    // Falls through to `help_quit` while there's no text selection.
    TextualCopyText("textual.screen.copy_text", "ctrl+c,super+c", "screen.copy_text", "Copy selected text").hidden();
    // `DataTable`, then what it inherits from `ScrollView`.
    TextualTableSelect("textual.table.select_cursor", "enter", "select_cursor", "Select").hidden();
    TextualTableCursorUp("textual.table.cursor_up", "up", "cursor_up", "Cursor up").hidden();
    TextualTableCursorDown("textual.table.cursor_down", "down", "cursor_down", "Cursor down").hidden();
    TextualTableCursorRight("textual.table.cursor_right", "right", "cursor_right", "Cursor right").hidden();
    TextualTableCursorLeft("textual.table.cursor_left", "left", "cursor_left", "Cursor left").hidden();
    TextualTablePageUp("textual.table.page_up", "pageup", "page_up", "Page up").hidden();
    TextualTablePageDown("textual.table.page_down", "pagedown", "page_down", "Page down").hidden();
    TextualTableScrollTop("textual.table.scroll_top", "ctrl+home", "scroll_top", "Top").hidden();
    TextualTableScrollBottom("textual.table.scroll_bottom", "ctrl+end", "scroll_bottom", "Bottom").hidden();
    TextualTableScrollHome("textual.table.scroll_home", "home", "scroll_home", "Home").hidden();
    TextualTableScrollEnd("textual.table.scroll_end", "end", "scroll_end", "End").hidden();
    TextualTablePageLeft("textual.table.page_left", "ctrl+pageup", "page_left", "Page Left").hidden();
    TextualTablePageRight("textual.table.page_right", "ctrl+pagedown", "page_right", "Page Right").hidden();
    // `ScrollableContainer`: the details content (#74) and the command log (#80).
    TextualScrollUp("textual.scroll.scroll_up", "up", "scroll_up", "Scroll Up").hidden();
    TextualScrollDown("textual.scroll.scroll_down", "down", "scroll_down", "Scroll Down").hidden();
    TextualScrollLeft("textual.scroll.scroll_left", "left", "scroll_left", "Scroll Left").hidden();
    TextualScrollRight("textual.scroll.scroll_right", "right", "scroll_right", "Scroll Right").hidden();
    TextualScrollHome("textual.scroll.scroll_home", "home", "scroll_home", "Scroll Home").hidden();
    TextualScrollEnd("textual.scroll.scroll_end", "end", "scroll_end", "Scroll End").hidden();
    TextualScrollPageUp("textual.scroll.page_up", "pageup", "page_up", "Page Up").hidden();
    TextualScrollPageDown("textual.scroll.page_down", "pagedown", "page_down", "Page Down").hidden();
    TextualScrollPageLeft("textual.scroll.page_left", "ctrl+pageup", "page_left", "Page Left").hidden();
    TextualScrollPageRight("textual.scroll.page_right", "ctrl+pagedown", "page_right", "Page Right").hidden();
    // `Tabs`: the details tab strip (#74).
    TextualTabsPrevious("textual.tabs.previous_tab", "left", "previous_tab", "Previous tab").hidden();
    TextualTabsNext("textual.tabs.next_tab", "right", "next_tab", "Next tab").hidden();
    // `Input`: the table's search box (#101). Selection, copy and cut come with the M3 shared input (#145).
    TextualInputCursorLeft("textual.input.cursor_left", "left", "cursor_left", "Move cursor left").hidden();
    TextualInputCursorLeftWord("textual.input.cursor_left_word", "ctrl+left", "cursor_left_word", "Move cursor left a word")
        .hidden();
    TextualInputCursorRight("textual.input.cursor_right", "right", "cursor_right", "Move cursor right or accept the completion suggestion")
        .hidden();
    TextualInputCursorRightWord("textual.input.cursor_right_word", "ctrl+right", "cursor_right_word", "Move cursor right a word")
        .hidden();
    TextualInputDeleteLeft("textual.input.delete_left", "backspace", "delete_left", "Delete character left").hidden();
    TextualInputHome("textual.input.home", "home,ctrl+a", "home", "Go to start").hidden();
    TextualInputEnd("textual.input.end", "end,ctrl+e", "end", "Go to end").hidden();
    TextualInputDeleteRight("textual.input.delete_right", "delete,ctrl+d", "delete_right", "Delete character right")
        .hidden();
    TextualInputSubmit("textual.input.submit", "enter", "submit", "Submit").hidden();
    TextualInputDeleteLeftWord("textual.input.delete_left_word", "ctrl+w", "delete_left_word", "Delete left to start of word")
        .hidden();
    TextualInputDeleteLeftAll("textual.input.delete_left_all", "ctrl+u", "delete_left_all", "Delete all to the left")
        .hidden();
    TextualInputDeleteRightWord("textual.input.delete_right_word", "ctrl+f", "delete_right_word", "Delete right to start of word")
        .hidden();
    TextualInputDeleteRightAll("textual.input.delete_right_all", "ctrl+k", "delete_right_all", "Delete all to the right")
        .hidden();
    TextualInputPaste("textual.input.paste", "ctrl+v", "paste", "Paste text from the clipboard").hidden();
    // LazyGithub's `FileFilterInput`, the diff viewer's file filter (#236), takes `escape` in `on_key` before any
    // binding: it clears the filter and leaves it. Python has no id for it.
    TextualFileFilterClear("textual.file_filter.clear", "escape", "clear", "Clear filter").hidden();
    // `Button`: the dialogs' buttons (#213).
    TextualButtonPress("textual.button.press", "enter", "press", "Press button").hidden();
    // `Switch`: the dialogs' switches (#223).
    TextualSwitchToggle("textual.switch.toggle_switch", "enter,space", "toggle_switch", "Toggle").hidden();
    // `Tree`: the run Jobs tab's tree (#242), in `Tree.BINDINGS` order. It inherits the `textual.scroll.*` keys too.
    TextualTreeCursorParent("textual.tree.cursor_parent", "shift+left", "cursor_parent", "Cursor to parent").hidden();
    TextualTreeCursorParentNextSibling("textual.tree.cursor_parent_next_sibling", "shift+right", "cursor_parent_next_sibling", "Cursor to next ancestor")
        .hidden();
    TextualTreeCursorPreviousSibling("textual.tree.cursor_previous_sibling", "shift+up", "cursor_previous_sibling", "Cursor to previous sibling")
        .hidden();
    TextualTreeCursorNextSibling("textual.tree.cursor_next_sibling", "shift+down", "cursor_next_sibling", "Cursor to next sibling")
        .hidden();
    TextualTreeSelect("textual.tree.select_cursor", "enter", "select_cursor", "Select").hidden();
    TextualTreeToggle("textual.tree.toggle_node", "space", "toggle_node", "Toggle").hidden();
    TextualTreeToggleExpandAll("textual.tree.toggle_expand_all", "shift+space", "toggle_expand_all", "Expand or collapse all")
        .hidden();
    TextualTreeCursorUp("textual.tree.cursor_up", "up", "cursor_up", "Cursor Up").hidden();
    TextualTreeCursorDown("textual.tree.cursor_down", "down", "cursor_down", "Cursor Down").hidden();
    // `CollapsibleTitle`: the PR Overview's sections (#232) and every collapsible after them.
    TextualCollapsibleToggle("textual.collapsible.toggle_collapsible", "enter", "toggle_collapsible", "Toggle collapsible")
        .hidden();
    // `ListView`: the lists inside the PR Overview's sections (#232).
    TextualListSelect("textual.list_view.select_cursor", "enter", "select_cursor", "Select").hidden();
    TextualListCursorUp("textual.list_view.cursor_up", "up", "cursor_up", "Cursor up").hidden();
    TextualListCursorDown("textual.list_view.cursor_down", "down", "cursor_down", "Cursor down").hidden();
}

impl BindingId {
    /// The id as `bindings.overrides` spells it, or `None` for a Textual built-in, which can't be overridden.
    pub fn config_id(self) -> Option<&'static str> {
        let name = self.def().name;
        (!name.starts_with("textual.")).then_some(name)
    }

    /// The binding a `bindings.overrides` key names. `textual.*` ids name nothing.
    pub fn from_config_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|binding| binding.config_id() == Some(id))
    }

    /// The binding's defaults and display.
    pub fn def(self) -> &'static BindingDef {
        &REGISTRY[self as usize]
    }

    /// Whether the binding is checked before the focused component sees the key (Textual's `priority=True`).
    pub fn is_priority(self) -> bool {
        self.def().priority
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_registry_is_indexed_by_variant() {
        assert_eq!(REGISTRY.len(), BindingId::ALL.len());
        for &id in BindingId::ALL {
            assert_eq!(id.def().id, id);
        }
    }

    #[test]
    fn names_are_unique_and_round_trip() {
        let mut names = HashSet::new();
        for &id in BindingId::ALL {
            assert!(names.insert(id.def().name), "{} is registered twice", id.def().name);
            if let Some(config_id) = id.config_id() {
                assert_eq!(BindingId::from_config_id(config_id), Some(id));
            }
        }
    }

    #[test]
    fn built_ins_cant_be_overridden() {
        assert_eq!(BindingId::TextualQuit.config_id(), None);
        assert_eq!(BindingId::from_config_id("textual.app.quit"), None);
        assert_eq!(BindingId::from_config_id("app.nope"), None);
        assert_eq!(BindingId::AppQuit.config_id(), Some("app.quit"));
    }

    #[test]
    fn only_help_quit_is_system() {
        let system: Vec<_> = BindingId::ALL.iter().copied().filter(|id| id.def().system).collect();
        assert_eq!(system, [BindingId::TextualHelpQuit]);
    }

    #[test]
    fn only_ctrl_q_is_priority() {
        let priority: Vec<_> = BindingId::ALL.iter().copied().filter(|id| id.is_priority()).collect();
        assert_eq!(priority, [BindingId::TextualQuit]);
    }
}
