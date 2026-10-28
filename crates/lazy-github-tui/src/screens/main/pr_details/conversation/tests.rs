//! The Conversation tab and the Overview's reviews summary, against the `pr-details` fixtures.

use lazy_github_core::models::{CommentRef, ReviewState};

use super::super::tests::{
    NAME, app, app_on, cancels, comment_reactions, comments, fixture_pr, gh, loading_sections, main, pr_requests,
    reviews, row_of, server_error, settle, shown,
};
use super::{ConversationStop, PrConversation};
use crate::action::Action;
use crate::component::Owner;
use crate::components::comment::CommentStop;
use crate::effect::{Effect, GithubRequest, GithubResponse, RequestId};
use crate::event::Event;
use crate::keymap::BindingId;
use crate::screens::main::FocusTarget;
use crate::test_util::{TestApp, logs};

fn conversation(app: &TestApp) -> &PrConversation {
    main(app).details.pull_request().unwrap().conversation()
}

fn stop(app: &TestApp) -> ConversationStop {
    conversation(app).stop()
}

fn focus(app: &TestApp) -> FocusTarget {
    main(app).focus()
}

fn block(app: &TestApp, comment: CommentRef) -> usize {
    conversation(app).block_index(comment).unwrap()
}

/// The footer, the screen's last row.
fn footer(app: &mut TestApp) -> String {
    app.screen(160, 48).lines().last().unwrap().to_owned()
}

/// PR 5's Conversation tab, every request answered, the tab strip focused.
fn shown_conversation() -> TestApp {
    let mut app = shown(&["enter"]);
    app.press(&["5", "right", "right"]);
    app
}

/// The rows of the details pane: the text right of the sections column.
fn details_rows(screen: &str) -> Vec<String> {
    screen
        .lines()
        .map(|line| line.chars().skip(64).collect::<String>())
        .collect()
}

/// The conversation requests among the app's effects.
fn conversation_requests(app: &TestApp) -> Vec<(RequestId, GithubRequest)> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .filter(|(_, request)| {
            matches!(
                request,
                GithubRequest::PullRequestReviews { .. }
                    | GithubRequest::IssueComments { .. }
                    | GithubRequest::CommentReactions { .. }
            )
        })
        .collect()
}

#[test]
fn the_reviews_and_comments_are_requested_with_the_pull_request() {
    let app = loading_sections();
    let requests: Vec<GithubRequest> = conversation_requests(&app).into_iter().map(|(_, r)| r).collect();
    let pr = fixture_pr(5);
    assert_eq!(
        requests,
        [
            GithubRequest::PullRequestReviews {
                pr: Box::new(pr.clone())
            },
            GithubRequest::IssueComments {
                issue: Box::new(pr.pr.issue.clone())
            },
        ]
    );
}

#[test]
fn the_spinner_shows_until_both_requests_answer() {
    let mut app = loading_sections();
    app.press(&["5", "right", "right"]);
    let requests = conversation_requests(&app);
    let (reviews_id, comments_id) = (requests[0].0, requests[1].0);
    assert!(app.screen(160, 48).contains("Loading..."));

    app.send(Event::Github {
        id: reviews_id,
        response: GithubResponse::PullRequestReviews(Ok(reviews(5))),
    });
    let screen = app.screen(160, 48);
    assert!(screen.contains("Loading..."), "{screen}");
    assert!(!screen.contains("Review from"), "{screen}");

    app.send(Event::Github {
        id: comments_id,
        response: GithubResponse::IssueComments(Ok(comments(5))),
    });
    let screen = app.screen(160, 48);
    assert!(!screen.contains("Loading..."), "{screen}");
    assert!(screen.contains("▼ Review from parity-bot (Commented)"), "{screen}");
}

#[test]
fn pr_conversation_loading() {
    let mut app = loading_sections();
    app.press(&["5", "right", "right"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn the_content_is_the_reviews_then_the_comments() {
    let mut app = shown_conversation();
    // Taller, so everything shows.
    let screen = app.screen(160, 90);
    let order = [
        "▼ Review from parity-bot (Commented)",
        "Looks close. One question inline.",
        "Should the cursor wrap at the end?",
        "parity-bot • Mon Feb  2 10:00:00 2026",
        "No, it stops at the last line.",
        "parity-friend • Mon Feb  2 11:00:00 2026",
        "▶ Review from parity-reviewer (Approved)",
        "▶ Review from parity-carol (Dismissed)",
        "▼ Review from parity-bot (Changes Requested)",
        "Please add a test for the cursor.",
        "This needs a test.",
        "Rebased on main. @parity-bot ready for another look.",
        "Is this related to",
        "Unknown • Tue Feb  3 13:30:00 2026",
    ];
    let mut at = 0;
    for text in order {
        let found = screen[at..]
            .find(text)
            .unwrap_or_else(|| panic!("{text} after {at}: {screen}"));
        at += found + text.len();
    }
    // The body-less comment review 502 only answers 501's thread: hidden (Py bug B10). 7003's body is empty.
    assert!(!screen.contains("Review from parity-friend"), "{screen}");
    assert_eq!(conversation(&app).block_index(CommentRef::Issue(7003)), None);
}

#[test]
fn pr_conversation_160x48() {
    let mut app = shown_conversation();
    let screen = app.screen(160, 48);
    let rows = details_rows(&screen);
    // The review body in the section, then a thread's border with its comment blocks one cell in.
    assert!(
        rows.iter().any(|row| row.starts_with("│      Looks close.")),
        "{screen}"
    );
    assert!(
        rows.iter()
            .any(|row| row.starts_with("│    │ │ Should the cursor wrap")),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn dismissed_and_empty_reviews_start_closed() {
    let app = shown_conversation();
    let conversation = conversation(&app);
    // 501, 503 (approved, empty), 504 (dismissed), 505.
    assert!(conversation.is_open(0));
    assert!(!conversation.is_open(1));
    assert!(!conversation.is_open(2));
    assert!(conversation.is_open(3));
}

#[test]
fn a_review_with_nothing_holds_a_line_saying_so() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab", "tab", "tab", "tab", "tab"]);
    assert_eq!(stop(&app), ConversationStop::Review(1));
    app.press(&["enter"]);
    assert!(conversation(&app).is_open(1));
    let screen = app.screen(160, 48);
    assert!(screen.contains("▼ Review from parity-reviewer (Approved)"), "{screen}");
    assert!(screen.contains("    No additional review content"), "{screen}");
    app.press(&["enter"]);
    assert!(!conversation(&app).is_open(1));
}

#[test]
fn a_click_on_a_review_title_focuses_and_toggles_it() {
    let mut app = shown_conversation();
    let row = row_of(&mut app, "▶ Review from parity-carol (Dismissed)");
    let screen = app.screen(160, 48);
    let line = screen.lines().nth(usize::from(row)).unwrap();
    let column = u16::try_from(line[..line.find('▶').unwrap()].chars().count()).unwrap();
    app.click(column, row);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert_eq!(stop(&app), ConversationStop::Review(2));
    assert!(conversation(&app).is_open(2));
    let screen = app.screen(160, 48);
    assert!(screen.contains("Blocking until the tests pass."), "{screen}");
    // A click on a comment focuses its block.
    let row = row_of(&mut app, "Should the cursor wrap");
    app.click(column + 8, row);
    assert_eq!(
        stop(&app),
        ConversationStop::Comment(block(&app, CommentRef::Review(9001)), CommentStop::Block)
    );
}

#[test]
fn pr_conversation_empty() {
    let mut app = shown(&["j", "enter"]);
    app.press(&["5", "right", "right"]);
    let screen = app.screen(160, 48);
    let rows = details_rows(&screen);
    assert!(rows[6].starts_with("│No reviews or comments available "), "{screen}");
    insta::assert_snapshot!(screen);
    // The scroll area is the only stop.
    app.press(&["tab"]);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    assert!(footer(&mut app).starts_with(" n New Comment"));
    app.press(&["tab"]);
    assert_ne!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn a_failure_says_so_here_and_in_the_overview() {
    for gh in [
        gh().with_pull_request_reviews(NAME, 5, Err(server_error())),
        gh().with_issue_comments(NAME, 5, Err(server_error())),
    ] {
        let mut app = app_on(gh);
        app.press(&["enter"]);
        let out = settle(&mut app);
        assert_eq!(
            out.matches("ERROR Error retrieving PR reviews or comments").count(),
            1,
            "{out}"
        );
        let screen = app.screen(160, 48);
        assert!(screen.contains("▶ PR Reviews: failed to load "), "{screen}");
        assert!(app.app().toasts().is_empty());
        app.press(&["5", "right", "right"]);
        let screen = app.screen(160, 48);
        assert!(
            details_rows(&screen)[6].starts_with("│Failed to load reviews and comments "),
            "{screen}"
        );
        assert!(!screen.contains("Loading..."), "{screen}");
    }
}

#[test]
fn pr_conversation_failed() {
    let mut app = app_on(gh().with_pull_request_reviews(NAME, 5, Err(server_error())));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["5", "right", "right"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn a_failure_cancels_the_other_request() {
    let mut app = loading_sections();
    let requests = conversation_requests(&app);
    let out = logs(|| {
        app.send(Event::Github {
            id: requests[0].0,
            response: GithubResponse::PullRequestReviews(Err(server_error())),
        });
    });
    assert!(out.contains("ERROR Error retrieving PR reviews or comments"), "{out}");
    assert!(cancels(&app).contains(&requests[1].0));
    // Its late answer is dropped.
    let out = logs(|| {
        app.send(Event::Github {
            id: requests[1].0,
            response: GithubResponse::IssueComments(Ok(comments(5))),
        });
    });
    assert!(out.contains("Dropping the comments response"), "{out}");
}

#[test]
fn each_shown_comment_asks_for_its_reactions_on_its_own_endpoint() {
    let app = shown_conversation();
    let asked: Vec<CommentRef> = conversation_requests(&app)
        .into_iter()
        .filter_map(|(_, request)| match request {
            GithubRequest::CommentReactions { comment, .. } => Some(comment),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked,
        [
            CommentRef::Review(9001),
            CommentRef::Review(9002),
            CommentRef::Review(9003),
            CommentRef::Issue(7001),
            CommentRef::Issue(7002),
        ]
    );
    // A review comment's reactions come from the review comment endpoint (Py bug B54).
    let conversation = conversation(&app);
    let has = |comment| conversation.block(comment).unwrap().has_reactions();
    assert!(has(CommentRef::Review(9001)));
    assert!(!has(CommentRef::Review(9002)));
    assert!(has(CommentRef::Issue(7001)));
    assert!(!has(CommentRef::Issue(7002)));
}

#[test]
fn reactions_attach_to_their_block_as_they_arrive() {
    let mut app = app();
    app.press(&["enter"]);
    let (id, _) = pr_requests(&app)[0];
    app.send(Event::Github {
        id,
        response: GithubResponse::PullRequest(Ok(Box::new(fixture_pr(5)))),
    });
    let requests = conversation_requests(&app);
    app.send(Event::Github {
        id: requests[0].0,
        response: GithubResponse::PullRequestReviews(Ok(reviews(5))),
    });
    app.send(Event::Github {
        id: requests[1].0,
        response: GithubResponse::IssueComments(Ok(comments(5))),
    });
    let reactions: Vec<(RequestId, CommentRef)> = conversation_requests(&app)
        .into_iter()
        .filter_map(|(id, request)| match request {
            GithubRequest::CommentReactions { comment, .. } => Some((id, comment)),
            _ => None,
        })
        .collect();
    assert_eq!(reactions.len(), 5);
    assert_eq!(reactions[0].1, CommentRef::Review(9001));
    assert_eq!(reactions[3].1, CommentRef::Issue(7001));
    let reacted = comment_reactions(CommentRef::Issue(7001));
    // An answer lands on the block its request was for: 9001's request, then 7001's.
    app.send(Event::Github {
        id: reactions[0].0,
        response: GithubResponse::CommentReactions(Ok(reacted.clone())),
    });
    let shown = conversation(&app);
    assert!(shown.block(CommentRef::Review(9001)).unwrap().has_reactions());
    assert!(!shown.block(CommentRef::Issue(7001)).unwrap().has_reactions());
    app.send(Event::Github {
        id: reactions[3].0,
        response: GithubResponse::CommentReactions(Ok(reacted)),
    });
    assert!(
        conversation(&app)
            .block(CommentRef::Issue(7001))
            .unwrap()
            .has_reactions()
    );
    app.press(&["5", "right", "right"]);
    let screen = app.screen(160, 90);
    assert_eq!(screen.matches("▶ 👍  1 | 🚀  1").count(), 2, "{screen}");
}

#[test]
fn the_focus_ring_walks_reviews_open_threads_and_comments() {
    let mut app = shown_conversation();
    app.press(&["tab"]);
    assert_eq!(
        (focus(&app), stop(&app)),
        (FocusTarget::DetailsContent, ConversationStop::Scroll)
    );
    let mut walked = Vec::new();
    loop {
        app.press(&["tab"]);
        if focus(&app) != FocusTarget::DetailsContent {
            break;
        }
        walked.push(stop(&app));
    }
    let comment = |id| ConversationStop::Comment(block(&app, id), CommentStop::Block);
    let reactions = |id| ConversationStop::Comment(block(&app, id), CommentStop::ReactionsTitle);
    assert_eq!(
        walked,
        [
            ConversationStop::Review(0),
            comment(CommentRef::Review(9001)),
            reactions(CommentRef::Review(9001)),
            comment(CommentRef::Review(9002)),
            ConversationStop::Review(1),
            ConversationStop::Review(2),
            ConversationStop::Review(3),
            comment(CommentRef::Review(9003)),
            comment(CommentRef::Issue(7001)),
            reactions(CommentRef::Issue(7001)),
            comment(CommentRef::Issue(7002)),
        ]
    );
    // And back in from after: the last stop.
    let last = comment(CommentRef::Issue(7002));
    app.press(&["shift+tab"]);
    assert_eq!((focus(&app), stop(&app)), (FocusTarget::DetailsContent, last));
}

#[test]
fn a_closed_review_hides_its_threads_from_the_ring() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab", "enter"]);
    assert!(!conversation(&app).is_open(0));
    app.press(&["tab"]);
    assert_eq!(stop(&app), ConversationStop::Review(1));
}

#[test]
fn the_review_box_shows_while_focus_is_inside_it() {
    let mut app = shown_conversation();
    let boxed = |app: &mut TestApp| {
        let screen = app.screen(160, 48);
        details_rows(&screen).iter().filter(|row| row.starts_with("│┌")).count()
    };
    app.press(&["tab"]);
    assert_eq!(boxed(&mut app), 0);
    let line = footer(&mut app);
    assert!(line.starts_with(" n New Comment"), "{line}");
    app.press(&["tab"]);
    assert_eq!(boxed(&mut app), 1);
    let line = footer(&mut app);
    assert!(line.starts_with(" r Reply to Review  n New Comment"), "{line}");
    app.press(&["tab"]);
    assert_eq!(boxed(&mut app), 1);
    let line = footer(&mut app);
    assert!(line.starts_with(" r Reply to Comment  n New Comment"), "{line}");
    // Out of the reviews: no box. On a comment, the comment's reply and the tab's new comment.
    app.press(&["tab", "tab", "tab", "tab", "tab", "tab", "tab"]);
    assert_eq!(
        stop(&app),
        ConversationStop::Comment(block(&app, CommentRef::Issue(7001)), CommentStop::Block)
    );
    assert_eq!(boxed(&mut app), 0);
    let line = footer(&mut app);
    assert!(line.starts_with(" r Reply to Comment  n New Comment"), "{line}");
    // Back on a review's comment: the box.
    app.press(&["shift+tab"]);
    assert_eq!(boxed(&mut app), 1);
    // Focus elsewhere: no box either.
    app.press(&["2"]);
    assert_eq!(boxed(&mut app), 0);
}

#[test]
fn pr_conversation_review_focused() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn pr_conversation_comment_focused() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab", "tab"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

/// PR 5's Conversation with `gh` answering, the tab strip focused.
fn conversation_on(gh: lazy_github_core::github::FakeGithub) -> TestApp {
    let mut app = app_on(gh);
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["5", "right", "right"]);
    app
}

/// The fake, also answering a reply "Yes" to a review comment and a comment "Yes".
fn writing_gh() -> lazy_github_core::github::FakeGithub {
    let reply = reviews(5)[0].comments[0].clone();
    let comment = comments(5)[0].clone();
    gh().with_review_reply("Yes", Ok(reply))
        .with_posted_comment("Yes", Ok(comment))
}

/// Tabs from the tab strip until focus is on `wanted`.
fn tab_to(app: &mut TestApp, wanted: ConversationStop) {
    for _ in 0..60 {
        app.press(&["tab"]);
        if stop(app) == wanted {
            return;
        }
    }
    panic!("never reached {wanted:?}");
}

/// The comment-writing requests the app sent.
fn writes(app: &TestApp) -> Vec<GithubRequest> {
    app.effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Github { request, .. }
                if matches!(
                    request,
                    GithubRequest::CreateIssueComment { .. } | GithubRequest::ReplyToReviewComment { .. }
                ) =>
            {
                Some(request.clone())
            }
            _ => None,
        })
        .collect()
}

fn review_requests(app: &TestApp) -> usize {
    app.effects()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Github {
                    request: GithubRequest::PullRequestReviews { .. },
                    ..
                }
            )
        })
        .count()
}

fn dialog_open(app: &TestApp) -> bool {
    app.app().modals() == [crate::modal::ModalKind::NewComment]
}

/// Types "Yes" and posts it.
fn post_yes(app: &mut TestApp) {
    app.press(&["Y", "e", "s", "shift+enter"]);
}

#[test]
fn the_bindings_ask_for_the_dialog() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab", "tab"]);
    let main = app.app_mut().main_screen_mut().unwrap();
    for (owner, id) in [
        (Owner::CommentBlock, BindingId::ConversationCommentReply),
        (Owner::ConversationReview, BindingId::ConversationReviewReply),
        (Owner::PrConversation, BindingId::ConversationCommentNew),
    ] {
        assert!(matches!(
            main.details.on_binding(owner, id),
            Some(Action::Main(crate::screens::main::MainAction::DetailsCommand(o, got))) if o == owner && got == id
        ));
    }
}

#[test]
fn r_on_a_review_comment_replies_in_its_thread() {
    let mut app = conversation_on(writing_gh());
    let c9001 = block(&app, CommentRef::Review(9001));
    tab_to(&mut app, ConversationStop::Comment(c9001, CommentStop::Block));
    // The comment's `r` is innermost: it wins over the review's.
    app.press(&["r"]);
    assert!(dialog_open(&app));
    assert!(
        app.screen(160, 48)
            .contains("Replying to comment from parity-bot at Mon Feb  2 10:00:00 2026")
    );
    let before = review_requests(&app);
    post_yes(&mut app);
    assert_eq!(
        writes(&app),
        [GithubRequest::ReplyToReviewComment {
            repo: Box::new(fixture_pr(5).pr.issue.repo),
            pr_number: 5,
            comment_id: 9001,
            body: "Yes".into(),
        }]
    );
    settle(&mut app);
    assert!(!dialog_open(&app));
    // A posted reply reloads the conversation (Py bug B18's recommendation), and focus stays on the comment.
    assert_eq!(review_requests(&app), before + 1);
    assert_eq!(stop(&app), ConversationStop::Comment(c9001, CommentStop::Block));
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
}

#[test]
fn r_on_an_issue_comment_posts_an_issue_comment() {
    let mut app = conversation_on(writing_gh());
    let first = comments(5)[0].id;
    let c = block(&app, CommentRef::Issue(first));
    tab_to(&mut app, ConversationStop::Comment(c, CommentStop::Block));
    app.press(&["r"]);
    assert!(app.screen(160, 48).contains("Replying to comment from"));
    post_yes(&mut app);
    assert!(matches!(
        writes(&app).as_slice(),
        [GithubRequest::CreateIssueComment { number: 5, body, .. }] if body == "Yes"
    ));
}

#[test]
fn r_on_a_review_replies_with_an_issue_comment() {
    let mut app = conversation_on(writing_gh());
    tab_to(&mut app, ConversationStop::Review(0));
    app.press(&["r"]);
    assert!(dialog_open(&app));
    let review = &reviews(5)[0];
    let at = lazy_github_core::time::c_format(review.submitted_at.unwrap());
    assert!(
        app.screen(160, 48)
            .contains(&format!("Replying to comment from {} at {at}", review.user.login))
    );
    post_yes(&mut app);
    assert!(matches!(
        writes(&app).as_slice(),
        [GithubRequest::CreateIssueComment { number: 5, .. }]
    ));
}

#[test]
fn a_cancelled_reply_doesnt_reload_but_n_does() {
    let mut app = conversation_on(writing_gh());
    tab_to(&mut app, ConversationStop::Review(0));
    let before = review_requests(&app);
    app.press(&["r", "escape"]);
    settle(&mut app);
    assert_eq!(review_requests(&app), before);
    // `n`, cancelled: Python's `action_new_comment` reloads either way, and focus goes back to the scroll area.
    app.press(&["n", "escape"]);
    settle(&mut app);
    assert_eq!(review_requests(&app), before + 1);
    assert_eq!(stop(&app), ConversationStop::Scroll);
    assert_eq!(focus(&app), FocusTarget::DetailsContent);
    // `n`, posted: reloaded again.
    app.press(&["n"]);
    post_yes(&mut app);
    settle(&mut app);
    assert_eq!(review_requests(&app), before + 2);
    assert!(!dialog_open(&app));
}

#[test]
fn the_content_stays_up_while_it_reloads() {
    let mut app = conversation_on(writing_gh());
    app.press(&["tab", "n", "escape"]);
    // The reload's requests are out, unanswered: the conversation still shows.
    let screen = app.screen(160, 48);
    assert!(screen.contains("Review from parity-bot"), "{screen}");
    settle(&mut app);
    assert!(app.screen(160, 48).contains("Review from parity-bot"));
}

#[test]
fn pr_conversation_reply_to_a_review() {
    let mut app = conversation_on(writing_gh());
    tab_to(&mut app, ConversationStop::Review(0));
    app.press(&["r"]);
    insta::assert_snapshot!(app.screen(160, 48));
}

#[test]
fn enter_on_a_reactions_title_opens_it_and_the_list_takes_keys() {
    let mut app = shown_conversation();
    app.press(&["tab", "tab", "tab", "tab", "enter"]);
    let c9001 = block(&app, CommentRef::Review(9001));
    assert_eq!(
        stop(&app),
        ConversationStop::Comment(c9001, CommentStop::ReactionsTitle)
    );
    assert!(
        conversation(&app)
            .block(CommentRef::Review(9001))
            .unwrap()
            .reactions_open()
    );
    let screen = app.screen(160, 48);
    assert!(screen.contains("👀 : parity-friend"), "{screen}");
    app.press(&["tab", "down"]);
    assert_eq!(stop(&app), ConversationStop::Comment(c9001, CommentStop::ReactionsList));
    assert_eq!(
        conversation(&app)
            .block(CommentRef::Review(9001))
            .unwrap()
            .reaction_highlight(),
        Some(0)
    );
}

#[test]
fn the_focused_stop_scrolls_into_view() {
    let mut app = shown_conversation();
    app.press(&["tab"]);
    for _ in 0..9 {
        app.press(&["tab"]);
        app.render(160, 48);
    }
    assert_eq!(
        stop(&app),
        ConversationStop::Comment(block(&app, CommentRef::Issue(7001)), CommentStop::Block)
    );
    assert!(conversation(&app).offset() > 0);
    let screen = app.screen(160, 48);
    assert!(screen.contains("Rebased on main."), "{screen}");
    assert!(screen.contains("parity-friend • Tue Feb  3 12:00:00 2026"), "{screen}");
    // The scroll keys scroll from the scroll area.
    let mut app = shown_conversation();
    app.press(&["tab"]);
    app.render(160, 48);
    app.press(&["end"]);
    assert!(conversation(&app).offset() > 0);
}

#[test]
fn a_body_less_comment_review_shows_when_it_starts_a_thread() {
    // Py bug B10: Python hides every comment review without a body. Rust shows one with a thread of its own.
    let mut lone = reviews(5);
    lone.retain(|review| review.id == 502);
    lone[0].comments[0].in_reply_to_id = None;
    let mut app = app_on(gh().with_pull_request_reviews(NAME, 5, Ok(lone)));
    app.press(&["enter"]);
    settle(&mut app);
    app.press(&["5", "right", "right"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("▼ Review from parity-friend (Commented)"), "{screen}");
    assert!(screen.contains("No, it stops at the last line."), "{screen}");
}

#[test]
fn late_answers_for_a_replaced_pull_request_are_dropped() {
    let mut app = loading_sections();
    let requests = conversation_requests(&app);
    app.dispatch(Action::Main(crate::screens::main::MainAction::OpenPullRequest {
        target: super::super::PullRequestTarget::Full(Box::new(fixture_pr(4))),
        focus: true,
    }));
    for (id, _) in &requests {
        assert!(cancels(&app).contains(id), "{id}");
    }
    let out = logs(|| {
        app.send(Event::Github {
            id: requests[0].0,
            response: GithubResponse::PullRequestReviews(Ok(reviews(5))),
        });
    });
    assert!(out.contains("Dropping the reviews response"), "{out}");
    app.press(&["5", "right", "right"]);
    assert!(!app.screen(160, 48).contains("Review from"));
}

// The Overview's reviews summary.

#[test]
fn the_overview_summarizes_each_reviewers_latest_review() {
    let mut app = shown(&["enter"]);
    let screen = app.screen(160, 48);
    assert!(screen.contains("▶ PR Reviews: ✘ Changes Requested "), "{screen}");
    let mut merged = shown(&["j", "j", "enter"]);
    let screen = merged.screen(160, 48);
    assert!(screen.contains("▶ PR Reviews: ✓ Approved "), "{screen}");
    // PR 4 has no reviews: no section.
    let mut closed = shown(&["j", "enter"]);
    assert!(!closed.screen(160, 48).contains("PR Reviews"));
    let mut commented = reviews(5);
    commented.retain(|review| review.state == ReviewState::Commented);
    let mut app = app_on(gh().with_pull_request_reviews(NAME, 5, Ok(commented)));
    app.press(&["enter"]);
    settle(&mut app);
    assert!(app.screen(160, 48).contains("▶ PR Reviews Summary "));
}

#[test]
fn pr_overview_reviews_open() {
    let mut app = shown(&["enter"]);
    // The body, the reactions title, the checks title, then the reviews title.
    app.press(&["tab", "tab", "tab", "tab", "enter"]);
    let screen = app.screen(160, 48);
    for row in [
        "parity-bot: ✘ Changes Requested",
        "parity-friend: Commented",
        "parity-reviewer: ✓ Approved",
        "parity-carol: Dismissed",
    ] {
        assert!(screen.contains(row), "{row}: {screen}");
    }
    insta::assert_snapshot!(screen);
}
