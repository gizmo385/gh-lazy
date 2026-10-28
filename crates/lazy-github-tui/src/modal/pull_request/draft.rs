//! Create PR's draft: the form saved for the repo 2 s after the last edit and when the dialog closes without a pull
//! request, restored when Create PR next opens, and deleted once a pull request is created. Python's
//! `_save_draft_state`, `_restore_draft_state` and `_schedule_draft_save`. Edit PR has none.

use std::path::PathBuf;
use std::time::Duration;

use lazy_github_core::drafts::{self, PrDraft};

use super::{PullRequestDialog, PullRequestForm};
use crate::component::Cx;
use crate::effect::{Effect, RequestId, TimerId};
use crate::toast::Toast;

/// How long after the last edit the draft is saved (Textual's `set_timer(2, ...)`).
pub(super) const SAVE_AFTER: Duration = Duration::from_secs(2);

const RESTORED: &str = "Restored saved PR draft";

/// Create PR's draft file and the read of it on open.
#[derive(Debug)]
pub(super) struct DraftFile {
    path: PathBuf,
    /// The read, until it answers.
    pub(super) read: Option<RequestId>,
}

impl DraftFile {
    /// The draft of `repo_full_name`, read at once.
    pub(super) fn open(repo_full_name: &str, cx: &mut Cx<'_>) -> Self {
        let path = drafts::draft_path(&cx.config().cache.cache_directory, repo_full_name);
        let read = Some(cx.read_cache(path.clone()));
        Self { path, read }
    }
}

impl PullRequestDialog {
    /// Title, Base, Compare or the description changed: the save is (re)scheduled 2 s on.
    pub(super) fn schedule_draft_save(&self, cx: &mut Cx<'_>) {
        if self.draft_file.is_some() {
            cx.effect(Effect::StartTimeout {
                id: TimerId::PrDraftSave,
                after: SAVE_AFTER,
            });
        }
    }

    /// Saves the form as it is now, title and description stripped, or deletes the draft when there's nothing in
    /// it. A pending scheduled save is dropped, and a read still out is cancelled.
    pub(super) fn save_draft(&mut self, cx: &mut Cx<'_>) {
        let Some(file) = &mut self.draft_file else { return };
        if let Some(id) = file.read.take() {
            cx.effect(Effect::Cancel(id));
        }
        cx.effect(Effect::CancelTimer(TimerId::PrDraftSave));
        let path = file.path.clone();
        let form = self.form();
        let draft = PrDraft {
            repo_full_name: self.repo.full_name.clone(),
            title: form.title.trim().to_owned(),
            description: form.description.trim().to_owned(),
            base_ref: form.base_ref,
            head_ref: form.head_ref,
            is_draft: form.is_draft,
            reviewers: form.reviewers,
        };
        cx.effect(if draft.is_empty() {
            Effect::RemoveCache { path }
        } else {
            Effect::WriteCache {
                path,
                text: drafts::to_json(&draft),
            }
        });
    }

    /// A pull request was created from the form: the draft is deleted and a scheduled save dropped.
    pub(super) fn clear_draft(&mut self, cx: &mut Cx<'_>) {
        let Some(file) = &self.draft_file else { return };
        cx.effect(Effect::CancelTimer(TimerId::PrDraftSave));
        cx.effect(Effect::RemoveCache {
            path: file.path.clone(),
        });
    }

    /// The scheduled save is due. Ticks once the dialog is done (a stale one still queued) do nothing.
    pub(super) fn on_draft_timer(&mut self, cx: &mut Cx<'_>) -> bool {
        if self.draft_file.is_none() {
            return false;
        }
        if self.output.is_none() {
            self.save_draft(cx);
        }
        true
    }

    /// The draft file was read. A draft that loads fills the whole form in over the prefills (the current-directory
    /// Compare included: Py bug B72's recommendation), schedules a save as Python's change events do, and toasts
    /// when it has something in it. No file, or one that doesn't load, leaves the form as it opened.
    pub(super) fn on_draft_read(
        &mut self,
        id: RequestId,
        result: Result<Option<String>, String>,
        cx: &mut Cx<'_>,
    ) -> Option<Result<Option<String>, String>> {
        let Some(file) = self.draft_file.as_mut().filter(|file| file.read == Some(id)) else {
            return Some(result);
        };
        file.read = None;
        // No draft, or one that doesn't load, leaves the form as it opened; the read was the dialog's either way.
        let draft = drafts::load(&file.path, result)?;
        let has_content = draft.has_content();
        self.restore(PullRequestForm {
            title: draft.title,
            description: draft.description,
            base_ref: draft.base_ref,
            head_ref: draft.head_ref,
            is_draft: draft.is_draft,
            reviewers: draft.reviewers,
        });
        self.schedule_draft_save(cx);
        if has_content {
            cx.notify(Toast::info(RESTORED));
        }
        None
    }
}
