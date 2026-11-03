# A notification whose subject is an issue opens the issue

- Decision: Enter on a notification whose subject URL is an issue (`.../repos/<owner>/<repo>/issues/<n>`) selects
  the notification's repo, as before, then fetches issue `<n>` from that repo and opens it in Details ("Issue #n",
  tab strip focused), with the information toast "Opening issue for notification". A number that turns out to be a
  pull request opens as the pull request, with "Opening pull request for notification". An issue that fails to load
  leaves only the repo open, with "Opening repository for notification" and the error at debug, as a pull request
  that fails to load does. Python opens only the repo for an issue subject. Pull request subjects, subjects without a
  URL and every other subject type (releases, check suites, discussions) are unchanged.
- Decided by: Chris, 2026-11-02 (#405: D1 (a), open the issue; D2 (a), a notification in the selected repo selects it again)
- Link: #405; #386 (D5 b, Chris 2026-11-01: opening a hit from another repo selects that repo, and notifications
  share the code path); #394 (the Search modal, whose hits open through the same path)
- Rows: `modals.notifications-open-issue`, `toasts.opening-issue-for-notification`

Chris's D5 answer on #386 asked that a search hit from another repo select its repo, and that notifications do the
same. Notifications already do: Python's `action_view_notifications` calls `set_repository(notification.repository)`
before anything else, and the Rust app dispatches `Action::SelectRepo` the same way (scenario `notifications-open`,
gated against Python, captures the switch from `parity-bot/pr-details` to `parity-bot/dotfiles`). What a followed
link does that a notification doesn't is open an issue: a link to `/issues/<n>` shows the issue, while a notification
about issue `<n>` shows only its repo. #405 moves notification opening onto the path #394's search hits use ("select
the repo, then open the pull request or issue"), and that path opens issues because search hits are issues as often
as pull requests. Letting notifications use the issue half too is what makes the two behave the same.

The selected repo and the three tables change exactly as before, including when the notification is in the repo
that's already selected (Python selects it again, and so does Rust). The repos table doesn't change: a notification
from a repo the table doesn't list selects it without adding it, as Python and followed links do.
