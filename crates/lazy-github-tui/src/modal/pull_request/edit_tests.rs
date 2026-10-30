//! Edit PR: prefills, the disabled branch row, focus, closing, the Update chain and the drawing, opened with `E` on
//! the PR table against the `pr-details` and `pr-edit` fixtures.

use lazy_github_core::config::Config;
use lazy_github_core::github::{FakeGithub, GithubCall, PullRequestUpdate};

use super::tests::{
    dialog_view, fixture_body, fixture_repo, gh, is_open, messages, plain, refused, selected, settle, titled,
    type_text, walk,
};
use super::*;
use crate::test_util::TestApp;

const REPO: &str = "parity-bot/pr-details";

/// Pull request `number` of `pr-details`, as `GET .../pulls/<n>` answers.
fn pull_request(number: u64) -> FullPullRequest {
    let body = fixture_body("pr-details", &format!("repos/{REPO}/pulls/{number}/GET.json"));
    FullPullRequest::from_api(body, &fixture_repo()).unwrap()
}

/// PR 5's body, which the description holds.
const BODY_5: &str = "Adds a cursor to the viewer, as discussed in #3.\n\n- `j` and `k` move it\n- the current line is marked \
                      with `>`\n\nThanks @parity-bot for the first review!\n";

/// The fake with PRs 4 and 5, parity-reviewer requested on PR 5, and the updates and review requests accepted.
fn edit_gh() -> FakeGithub {
    let friend = serde_json::from_value(fixture_body("pr-edit", "users/parity-friend/GET.json")).unwrap();
    gh().with_pull_request(REPO, 4, Ok(pull_request(4)))
        .with_pull_request(REPO, 5, Ok(pull_request(5)))
        .with_requested_reviewers(REPO, 5, Ok(vec!["parity-reviewer".into()]))
        .with_pull_request_update(4, Ok(()))
        .with_pull_request_update(5, Ok(()))
        .with_review_request(4, Ok(()))
        .with_review_request(5, Ok(()))
        .with_profile("parity-friend", Ok(friend))
}

/// `E` on the PR table's first row (PR 5) and every request answered, drawn once.
fn editing_with(gh: FakeGithub) -> TestApp {
    editing_row(gh, 0)
}

/// `E` on the PR table's row `row` (0 is PR 5, 1 is PR 4), every request answered, drawn once.
fn editing_row(gh: FakeGithub, row: usize) -> TestApp {
    let mut app = selected(TestApp::with_github(Config::default(), gh));
    for _ in 0..row {
        app.press(&["j"]);
    }
    app.press(&["E"]);
    settle(&mut app);
    assert!(is_open(&app), "E opens Edit PR");
    let _ = app.render(160, 48);
    app
}

fn editing() -> TestApp {
    editing_with(edit_gh())
}

/// Edit PR built directly for `pr`.
fn direct(pr: FullPullRequest) -> PullRequestDialog {
    let mut app = TestApp::new(Config::default());
    app.with_cx(|cx| PullRequestDialog::edit(Box::new(pr), cx))
}

/// The updates sent, oldest first.
fn updates(app: &TestApp) -> Vec<(u64, PullRequestUpdate)> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::UpdatePullRequest { number, update, .. } => Some((number, update)),
            _ => None,
        })
        .collect()
}

/// The review requests sent, oldest first.
fn review_requests(app: &TestApp) -> Vec<Vec<String>> {
    app.gh()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            GithubCall::RequestReviewers { reviewers, .. } => Some(reviewers),
            _ => None,
        })
        .collect()
}

/// How often the requested reviewers were fetched.
fn listings(app: &TestApp) -> usize {
    app.gh()
        .calls()
        .iter()
        .filter(|call| matches!(call, GithubCall::RequestedReviewers { .. }))
        .count()
}

fn update_of(title: &str, body: &str) -> PullRequestUpdate {
    PullRequestUpdate {
        title: title.into(),
        body: body.into(),
        base: "main".into(),
    }
}

// Opening.

#[test]
fn the_fields_are_prefilled_from_the_pull_request() {
    let dialog = direct(pull_request(5));
    assert_eq!(dialog.heading(), "Editing PR 5");
    assert_eq!(
        dialog.form(),
        PullRequestForm {
            title: "Add a cursor to the viewer".into(),
            description: BODY_5.into(),
            base_ref: "main".into(),
            head_ref: "cursor".into(),
            is_draft: false,
            reviewers: Vec::new(),
        }
    );
    assert_eq!(dialog.ring.focused(), Some(PrField::Title));
    // The title's text is selected, so typing replaces it.
    assert_eq!(dialog.title.selection(), Some((0, "Add a cursor to the viewer".len())));
}

#[test]
fn a_pull_request_without_a_body_gives_an_empty_editor() {
    let mut pr = pull_request(5);
    pr.pr.issue.body = None;
    assert_eq!(direct(pr).form().description, "");
}

#[test]
fn the_draft_flag_shows_in_the_switch() {
    let mut pr = pull_request(5);
    pr.pr.draft = true;
    assert!(direct(pr).form().is_draft);
}

#[test]
fn e_asks_for_the_collaborators_and_the_requested_reviewers_but_not_the_branches() {
    let app = editing();
    let calls = app.gh().calls();
    assert!(calls.contains(&GithubCall::PullRequest {
        full_name: REPO.into(),
        number: 5
    }));
    assert!(calls.contains(&GithubCall::Collaborators(REPO.into())), "{calls:?}");
    assert!(calls.contains(&GithubCall::RequestedReviewers {
        full_name: REPO.into(),
        number: 5
    }));
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, GithubCall::ListBranches(_) | GithubCall::ListBranchesUncached(_)))
    );
}

#[test]
fn the_requested_reviewers_are_listed_when_they_arrive_and_a_failure_lists_none() {
    // The Reviewers input scrolls the list into view.
    let mut app = editing();
    app.press(&["tab", "tab"]);
    let screen = dialog_view(&mut app).screen;
    assert!(
        screen.contains("Current Reviewers - Click/Select them to remove"),
        "{screen}"
    );
    assert!(screen.contains("parity-reviewer"), "{screen}");

    let gh = edit_gh().with_requested_reviewers(REPO, 5, Err(refused()));
    let mut app = editing_with(gh);
    app.press(&["tab", "tab"]);
    let screen = dialog_view(&mut app).screen;
    assert!(!screen.contains("Current Reviewers"), "{screen}");
}

#[test]
fn the_branch_row_is_disabled_and_out_of_the_focus_order() {
    use PrField::*;
    let mut dialog = direct(pull_request(5));
    assert!(dialog.base.is_disabled() && dialog.compare.is_disabled() && dialog.draft.is_disabled());
    let reviewer = Reviewers(ReviewersFocus::Input);
    assert_eq!(
        walk(&mut dialog, FocusMove::Next, 6),
        [Description, reviewer, Submit, Cancel, Scroll, Title]
    );
    assert_eq!(
        walk(&mut dialog, FocusMove::Previous, 6),
        [Scroll, Cancel, Submit, reviewer, Description, Title]
    );
}

#[test]
fn the_reviewers_list_joins_the_focus_order_while_anyone_is_listed() {
    use PrField::*;
    let mut dialog = direct(pull_request(5));
    dialog.reviewers.add_existing(["parity-reviewer".to_owned()]);
    assert_eq!(
        walk(&mut dialog, FocusMove::Next, 7),
        [
            Description,
            Reviewers(ReviewersFocus::Input),
            Reviewers(ReviewersFocus::List),
            Submit,
            Cancel,
            Scroll,
            Title
        ]
    );
}

#[test]
fn a_click_on_the_branch_row_changes_nothing() {
    let mut app = editing();
    let before = dialog_view(&mut app).screen;
    // Base's input, Compare's and the switch, on the branch row's middle line.
    for column in [40, 80, 110] {
        app.click(column, 14);
    }
    assert!(is_open(&app));
    let dialog = dialog_view(&mut app).screen;
    assert_eq!(
        dialog.lines().nth(14),
        before.lines().nth(14),
        "the switch didn't flip and no cursor shows"
    );
    // The title lost focus to the scroll area: typing does nothing.
    type_text(&mut app, "x");
    assert!(!dialog_view(&mut app).screen.contains("xAdd"), "nothing typed");
}

// Closing.

#[test]
fn cancel_escape_and_q_outside_the_text_fields_close_without_a_request() {
    // escape from the title; q from the reviewers list, Update and Cancel; Cancel pressed. parity-reviewer is
    // listed, so the list is a stop.
    for keys in [
        &["escape"][..],
        &["tab", "tab", "tab", "q"],
        &["tab", "tab", "tab", "tab", "q"],
        &["tab", "tab", "tab", "tab", "tab", "q"],
        &["tab", "tab", "tab", "tab", "tab", "enter"],
    ] {
        let mut app = editing();
        app.press(keys);
        settle(&mut app);
        assert!(!is_open(&app), "{keys:?}");
        assert!(updates(&app).is_empty() && review_requests(&app).is_empty(), "{keys:?}");
        assert_eq!(listings(&app), 1, "only the one on open: {keys:?}");
        assert!(messages(&app).is_empty(), "{keys:?}");
    }
}

#[test]
fn q_types_in_the_title_the_description_and_the_reviewers_input() {
    let mut app = editing();
    app.press(&["q", "tab", "q", "tab", "q"]);
    assert!(is_open(&app));
}

// Updating.

#[test]
fn an_empty_title_is_refused_and_marked() {
    let mut app = editing();
    app.press(&["backspace", "shift+enter"]);
    settle(&mut app);
    assert!(is_open(&app));
    assert!(updates(&app).is_empty());
    assert_eq!(listings(&app), 1);
    assert_eq!(messages(&app), [titled("Invalid PR!", "Missing required fields!")]);
    // A space is a title.
    let mut app = editing();
    app.press(&["space", "shift+enter"]);
    settle(&mut app);
    assert_eq!(updates(&app), [(5, update_of(" ", BODY_5))]);
}

#[test]
fn nothing_changed_sends_no_update_and_requests_the_listed_reviewers_not_yet_requested() {
    let mut app = editing();
    // parity-friend joins parity-reviewer, who's requested already.
    app.press(&["tab", "tab"]);
    type_text(&mut app, "parity-friend\n");
    settle(&mut app);
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(updates(&app).is_empty());
    assert_eq!(listings(&app), 2, "on open and on Update");
    assert_eq!(review_requests(&app), [vec!["parity-friend".to_owned()]]);
    assert!(!is_open(&app));
    assert_eq!(
        messages(&app),
        [plain("Updating PR..."), plain("Successfully updated PR!")]
    );
}

#[test]
fn all_listed_reviewers_requested_already_sends_an_empty_request() {
    let mut app = editing();
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(review_requests(&app), [Vec::<String>::new()]);
    assert!(!is_open(&app));
}

#[test]
fn an_empty_reviewers_field_requests_nobody() {
    let gh = edit_gh().with_requested_reviewers(REPO, 5, Ok(Vec::new()));
    let mut app = editing_with(gh);
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(review_requests(&app).is_empty());
    assert!(!is_open(&app));
    assert_eq!(
        messages(&app),
        [plain("Updating PR..."), plain("Successfully updated PR!")]
    );
}

#[test]
fn a_changed_title_sends_the_update_with_the_base() {
    let mut app = editing();
    type_text(&mut app, "New title");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(updates(&app), [(5, update_of("New title", BODY_5))]);
    assert_eq!(listings(&app), 2);
    assert!(!is_open(&app));
}

#[test]
fn a_pull_request_without_a_body_is_always_updated() {
    let mut pr = pull_request(5);
    pr.pr.issue.body = None;
    let mut app = editing_with(edit_gh().with_pull_request(REPO, 5, Ok(pr)));
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(updates(&app), [(5, update_of("Add a cursor to the viewer", ""))]);
}

#[test]
fn a_refused_update_keeps_the_dialog_and_sends_nothing_more() {
    let gh = edit_gh().with_pull_request_update(5, Err(refused()));
    let mut app = editing_with(gh);
    type_text(&mut app, "Kept title");
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(is_open(&app));
    assert_eq!(
        messages(&app),
        [
            plain("Updating PR..."),
            titled(
                "Error updating pull request",
                "Unexpected error while attempting to update PR"
            )
        ]
    );
    assert_eq!(listings(&app), 1);
    assert!(review_requests(&app).is_empty());
    assert!(dialog_view(&mut app).screen.contains("Kept title"));
    // The submit is over: another one goes out.
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert_eq!(updates(&app).len(), 2);
}

#[test]
fn a_refused_review_request_still_closes_with_both_toasts() {
    let gh = edit_gh().with_review_request(5, Err(refused()));
    let mut app = editing_with(gh);
    app.press(&["tab", "tab"]);
    type_text(&mut app, "parity-friend\n");
    settle(&mut app);
    app.press(&["shift+enter"]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(
        messages(&app),
        [
            plain("Updating PR..."),
            plain("Successfully updated PR!"),
            titled(
                "Error requesting reviews",
                "Could not request reviews from parity-friend"
            ),
        ]
    );
}

#[test]
fn a_failed_listing_counts_as_nobody_requested() {
    let mut app = TestApp::new(Config::default());
    let mut dialog = app.with_cx(|cx| PullRequestDialog::edit(Box::new(pull_request(5)), cx));
    dialog.reviewers.add_existing(["parity-reviewer".to_owned()]);
    // Nothing changed: Update goes straight to the listing, which fails.
    app.with_cx(|cx| dialog.submit(cx));
    let Some(Submit::Listing { id, .. }) = dialog.submit else {
        panic!("listing: {:?}", dialog.submit);
    };
    let unclaimed = app.with_cx(|cx| dialog.on_response(id, GithubResponse::RequestedReviewers(Err(refused())), cx));
    assert!(unclaimed.is_none());
    let Some(Submit::RequestingReviews { reviewers, .. }) = &dialog.submit else {
        panic!("requesting: {:?}", dialog.submit);
    };
    assert_eq!(reviewers, &["parity-reviewer".to_owned()]);
}

#[test]
fn a_second_submit_mid_chain_is_ignored() {
    let mut app = editing();
    type_text(&mut app, "T");
    app.press(&["shift+enter", "shift+enter"]);
    settle(&mut app);
    assert_eq!(updates(&app).len(), 1);
    assert_eq!(review_requests(&app).len(), 1);
    assert_eq!(
        messages(&app),
        [plain("Updating PR..."), plain("Successfully updated PR!")]
    );
}

#[test]
fn escape_mid_chain_drops_the_later_answers() {
    let mut app = editing();
    type_text(&mut app, "T");
    app.press(&["shift+enter", "escape"]);
    settle(&mut app);
    assert!(!is_open(&app));
    assert_eq!(listings(&app), 1, "no listing after the update");
    assert!(review_requests(&app).is_empty());
    assert_eq!(messages(&app), [plain("Updating PR...")]);
    let main = app.app().main_screen().unwrap();
    assert_ne!(main.details_title(), "[5] PR #5 Details");
}

#[test]
fn a_closed_unmerged_pull_request_opens() {
    let mut app = editing_row(edit_gh(), 1);
    assert!(dialog_view(&mut app).screen.contains("Editing PR 4"));
}

// Drawing.

#[test]
fn edit_pr_opened() {
    let mut app = editing();
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn edit_pr_scrolled_to_the_reviewers() {
    let mut app = editing();
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn edit_pr_disabled_branch_row() {
    let mut app = editing();
    let screen = app.render(160, 48);
    // The branch row with its labels, faded inputs and switch.
    let row = Rect::new(30, 13, 100, 3);
    let mut branches = ratatui::buffer::Buffer::empty(row);
    for (x, y) in row.positions().map(|p| (p.x, p.y)) {
        branches[(x, y)] = screen[(x, y)].clone();
    }
    insta::assert_snapshot!(crate::test_util::buffer_to_styled_string(&branches));
}

// `gh` failing mid-session (#323): the requested reviewers on open, the update and the listing on Update.

mod gh_failures {
    use super::*;
    use crate::test_util::{assert_gh_failures_surface, gh_failures, logs};

    #[test]
    fn the_requested_reviewers_on_open() {
        for (name, error) in gh_failures() {
            let gh = edit_gh().with_requested_reviewers(REPO, 5, Err(error));
            let mut app = selected(TestApp::with_github(Config::default(), gh));
            app.press(&["E"]);
            let out = logs(|| {
                futures_util::FutureExt::now_or_never(app.settle()).unwrap();
            });
            assert!(is_open(&app), "{name}");
            // Python's `list_requested_reviewers` message.
            assert!(out.contains("ERROR Error creating review requests"), "{name}: {out}");
        }
    }

    #[test]
    fn updating() {
        assert_gh_failures_surface(
            || {
                let mut app = editing();
                type_text(&mut app, "Kept title");
                app
            },
            |app| {
                app.press(&["shift+enter"]);
            },
            |app| {
                if !is_open(app) {
                    return Err("the dialog closed".into());
                }
                let before = updates(app).len();
                app.press(&["shift+enter"]);
                futures_util::FutureExt::now_or_never(app.settle()).unwrap();
                (updates(app).len() > before)
                    .then_some(())
                    .ok_or_else(|| "Update is stuck".into())
            },
        );
    }

    #[test]
    fn the_listing_and_the_review_request_on_update() {
        // Nothing changed: Update lists the requested reviewers, then requests parity-reviewer, both failing.
        assert_gh_failures_surface(
            editing,
            |app| {
                app.press(&["shift+enter"]);
            },
            |app| {
                let failed = messages(app)
                    .iter()
                    .any(|(title, _)| title.as_deref() == Some("Error requesting reviews"));
                if !failed {
                    return Err(format!("{:?}", messages(app)));
                }
                // The chain is over: the dialog closed, as it does after any review request.
                (!is_open(app)).then_some(()).ok_or_else(|| "still open".into())
            },
        );
    }
}
