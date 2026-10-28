//! Opening Create PR (`pull_request.new`): Python's `SelectionsPane.action_open_pull_request` and
//! `trigger_pr_creation_flow`.

use std::path::Path;

use super::MainAction;
use super::pr_lookup::select_repository_first;
use crate::action::Action;
use crate::component::Cx;
use crate::modal::{CreatePrefill, PullRequestDialog};

/// `pull_request.new`: without a selected repo, the toast; otherwise Create PR for it.
pub(super) fn new_pull_request(cx: &mut Cx<'_>) {
    let Some(repo) = cx.current_repo().cloned() else {
        cx.notify(select_repository_first().with_title("Cannot open new pull request"));
        return;
    };
    let template = cx.config().pull_requests.pull_request_template.clone();
    let prefill = CreatePrefill {
        base: repo.default_branch.clone().unwrap_or_default(),
        compare: cx
            .local_repo()
            .matches(&repo.full_name)
            .then(|| cx.current_branch().map(str::to_owned))
            .flatten(),
        description: template.as_deref().map(read_template).unwrap_or_default(),
    };
    let dialog = PullRequestDialog::create(repo, prefill, cx);
    cx.open_dialog(dialog, |pr| {
        pr.map_or(Action::Consumed, |pr| Action::Main(MainAction::PullRequestCreated(pr)))
    });
}

/// The template's text: `pull_requests.pull_request_template` as Python reads it, not `~`-expanded, a relative path
/// against the process's working directory. A missing file gives none; one that can't be read (a directory, bad
/// UTF-8) is logged and gives none. A small local read, done once as the dialog opens (the spec allows it there).
fn read_template(path: &Path) -> String {
    if !path.exists() {
        return String::new();
    }
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        tracing::error!("Could not read the pull request template {}: {error}", path.display());
        String::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_read_from_an_existing_file_and_a_relative_path() {
        let dir = std::env::temp_dir().join(format!("lazy-github-pr-template-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("template.md");
        std::fs::write(&file, "## Summary\n").unwrap();
        assert_eq!(read_template(&file), "## Summary\n");
        // A relative path resolves against the working directory, as the crate's tests run in its directory.
        assert!(read_template(Path::new("Cargo.toml")).contains("[package]"));
        assert_eq!(read_template(&dir.join("missing.md")), "");
        let logs = crate::test_util::logs(|| assert_eq!(read_template(&dir), ""));
        assert!(logs.contains("Could not read the pull request template"), "{logs}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
