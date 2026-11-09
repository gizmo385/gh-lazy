//! The rebindable bindings grouped by area, in the order the Settings Key Bindings list shows them (#381). The help
//! panel can use them later.

use super::BindingId;

/// A titled group of rebindable bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingGroup {
    /// The group's header, such as "General".
    pub title: &'static str,
    /// Its bindings, in the order the list shows them.
    pub ids: &'static [BindingId],
}

/// Every binding `bindings.overrides` can name, each in exactly one group.
pub const BINDING_GROUPS: &[BindingGroup] = {
    use BindingId as B;
    &[
        BindingGroup {
            title: "General",
            ids: &[
                B::AppQuit,
                B::AppCommandPalette,
                B::AppMaximizeWidget,
                B::AppOpenHelp,
                B::NotificationsOpen,
                B::LinkPaste,
                B::MainRefresh,
                B::LinkFollow,
                B::LinkFollowExternal,
                B::UserViewProfile,
            ],
        },
        BindingGroup {
            title: "Focus",
            ids: &[
                B::MainReposFocus,
                B::MainPullRequestsFocus,
                B::MainIssuesFocus,
                B::MainWorkflowsFocus,
                B::MainDetailsFocus,
                B::MainCommandLogFocus,
            ],
        },
        BindingGroup {
            title: "Tables",
            ids: &[
                B::TableSelect,
                B::TableSearch,
                B::TableCursorDown,
                B::TablePageDown,
                B::TableCursorUp,
                B::TablePageUp,
                B::TableScrollRight,
                B::TablePageRight,
                B::TableScrollLeft,
                B::TablePageLeft,
                B::TableScrollTop,
                B::TableScrollBottom,
            ],
        },
        BindingGroup {
            title: "Repositories",
            ids: &[B::RepositoriesFavorite, B::RepositoriesLookup],
        },
        BindingGroup {
            title: "Pull requests",
            ids: &[
                B::PullRequestNew,
                B::PullRequestMerge,
                B::PullRequestEdit,
                B::PullRequestLookup,
            ],
        },
        BindingGroup {
            title: "Issues",
            ids: &[B::IssueNew, B::IssueEdit, B::IssueLookup],
        },
        BindingGroup {
            title: "Search",
            ids: &[B::SearchOpen],
        },
        BindingGroup {
            title: "Conversations",
            ids: &[
                B::ConversationCommentNew,
                B::ConversationReviewReply,
                B::ConversationCommentReply,
                B::ReactAdd,
            ],
        },
        BindingGroup {
            title: "Diff viewer",
            ids: &[
                B::DiffAddComment,
                B::DiffCursorDown,
                B::DiffCursorUp,
                B::DiffClearSelection,
                B::DiffNextHunk,
                B::DiffPreviousHunk,
                B::DiffFilterFiles,
                B::DiffNextFile,
                B::DiffPreviousFile,
                B::DiffToggle,
                B::DiffFoldAll,
                B::DiffFirstLine,
                B::DiffLastLine,
                B::DiffNextComment,
                B::DiffPreviousComment,
            ],
        },
        BindingGroup {
            title: "Workflows",
            ids: &[B::WorkflowsTrigger, B::WorkflowsViewRunDetails, B::WorkflowsViewJobLogs],
        },
        BindingGroup {
            title: "Notifications",
            ids: &[
                B::NotificationsMarkAllAsRead,
                B::NotificationsMarkRead,
                B::NotificationsViewRead,
                B::NotificationsViewUnread,
            ],
        },
        BindingGroup {
            title: "Dialogs",
            ids: &[
                B::ModalSubmit,
                B::ModalClose,
                B::ModalSearch,
                B::ConfirmYes,
                B::ConfirmNo,
            ],
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rebindable_id_is_in_exactly_one_group_and_nothing_else_is() {
        let grouped: Vec<BindingId> = BINDING_GROUPS
            .iter()
            .flat_map(|group| group.ids.iter().copied())
            .collect();
        for &id in BindingId::ALL {
            let count = grouped.iter().filter(|&&grouped| grouped == id).count();
            let expected = usize::from(id.config_id().is_some());
            assert_eq!(count, expected, "{} is in {count} groups", id.def().name);
        }
        // Python's 59, the eight Rust-only diff ids (decisions diff-file-navigation and diff-inline-comments), `main.refresh` (decision
        // cache-ttl-defaults-and-refresh) and `search.open` (decision search-issues-and-prs).
        assert_eq!(grouped.len(), 69);
        assert!(grouped.iter().all(|id| !id.def().name.starts_with("textual.")));
    }
}
