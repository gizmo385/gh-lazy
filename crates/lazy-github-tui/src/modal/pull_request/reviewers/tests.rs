//! The Reviewers field: suggestions, the lookup, the list, removal, focus stops, the mouse and its drawing.

use std::path::Path;

use crossterm::event::KeyModifiers;
use lazy_github_core::config::Config;
use lazy_github_core::models::{FullUser, User};

use super::*;
use crate::effect::Effect;
use crate::keymap::KeySpec;
use crate::test_util::{TestApp, buffer_to_styled_string};
use crate::toast::Severity;

fn chord(key: &str) -> KeyChord {
    match KeySpec::parse(key) {
        KeySpec::Chord(chord) => chord,
        KeySpec::Unknown(token) => panic!("{token:?} isn't a key"),
    }
}

fn user(login: &str) -> FullUser {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/pr-edit/users/parity-friend/GET.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut user: FullUser = serde_json::from_value(fixture["response"]["body"].clone()).unwrap();
    login.clone_into(&mut user.user.login);
    user
}

fn not_found() -> GhError {
    GhError::Http {
        status: 404,
        message: "Not Found".to_owned(),
        body: Vec::new(),
    }
}

fn repo() -> Repository {
    Repository {
        name: "pr-details".into(),
        full_name: "parity-bot/pr-details".into(),
        default_branch: Some("main".into()),
        private: false,
        archived: Some(false),
        owner: User {
            login: "parity-bot".into(),
            id: 1,
            name: None,
            html_url: "https://github.com/parity-bot".into(),
        },
        description: None,
        permissions: None,
    }
}

/// A field with `configured` suggestions, and an app to give it a context.
struct Harness {
    app: TestApp,
    field: ReviewersField,
}

impl Harness {
    fn new(configured: &[&str]) -> Self {
        let configured: Vec<String> = configured.iter().map(|&login| login.to_owned()).collect();
        Self {
            app: TestApp::new(Config::default()),
            field: ReviewersField::new(&configured),
        }
    }

    fn with(&mut self, f: impl FnOnce(&mut ReviewersField, &mut Cx<'_>)) {
        let field = &mut self.field;
        self.app.with_cx(|cx| f(field, cx));
    }

    fn press(&mut self, focus: ReviewersFocus, key: &str) -> ReviewersOutcome {
        let keymap = Keymap::defaults();
        self.field.handle_key(focus, &chord(key), &keymap, "")
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' { "space".to_owned() } else { c.to_string() };
            assert_eq!(self.press(ReviewersFocus::Input, &key), ReviewersOutcome::Handled);
        }
    }

    /// `enter` in the input, and the submit the dialog's update runs for it.
    fn enter(&mut self) {
        assert_eq!(self.press(ReviewersFocus::Input, "enter"), ReviewersOutcome::Submitted);
        self.with(ReviewersField::submit);
    }

    /// The user lookups sent so far, with their ids.
    fn lookups(&self) -> Vec<(RequestId, String)> {
        self.app
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Github {
                    id,
                    request: GithubRequest::User(login),
                } => Some((*id, login.clone())),
                _ => None,
            })
            .collect()
    }

    fn collaborators_request(&self) -> Option<RequestId> {
        self.app.effects().iter().find_map(|effect| match effect {
            Effect::Github {
                id,
                request: GithubRequest::Collaborators(_),
            } => Some(*id),
            _ => None,
        })
    }

    /// Answers the last lookup with `result`.
    fn answer_lookup(&mut self, result: Result<FullUser, GhError>) {
        let (id, _) = self.lookups().pop().expect("a lookup was sent");
        self.answer(id, GithubResponse::User(result.map(Box::new)));
    }

    fn answer(&mut self, id: RequestId, response: GithubResponse) {
        let mut unclaimed = None;
        self.with(|field, cx| unclaimed = field.on_response(id, response, cx));
        assert!(unclaimed.is_none(), "the field takes the answer to its request");
    }

    fn toasts(&mut self) -> Vec<(Severity, String)> {
        // Toasts are dispatched as actions, which the app applies right after `with_cx`.
        self.app
            .app()
            .toasts()
            .iter()
            .map(|toast| (toast.severity, toast.message.to_string()))
            .collect()
    }

    /// Lists `logins` the way a found lookup does.
    fn add(&mut self, logins: &[&str]) {
        for &login in logins {
            self.type_text(login);
            self.enter();
            self.answer_lookup(Ok(user(login)));
        }
    }
}

fn suggestion_for(field: &mut ReviewersField, text: &str) -> Option<String> {
    let keymap = Keymap::defaults();
    field.input.set_value("");
    for c in text.chars() {
        let _ = field.handle_key(ReviewersFocus::Input, &chord(&c.to_string()), &keymap, "");
    }
    field.suggestion().map(str::to_owned)
}

// Suggestions.

#[test]
fn suggestions_are_the_configured_logins_then_the_collaborators_followed_by_them() {
    let mut h = Harness::new(&["parity-zed"]);
    assert_eq!(suggestion_for(&mut h.field, "parity-z").as_deref(), Some("parity-zed"));
    assert_eq!(suggestion_for(&mut h.field, "parity-f"), None);
    h.with(|field, cx| field.load(&repo(), cx));
    let id = h.collaborators_request().expect("load asks for the collaborators");
    h.answer(
        id,
        GithubResponse::Collaborators(Ok(vec!["parity-friend".into(), "parity-zoe".into()])),
    );
    assert_eq!(
        suggestion_for(&mut h.field, "parity-f").as_deref(),
        Some("parity-friend")
    );
    // The collaborators come first.
    assert_eq!(suggestion_for(&mut h.field, "parity-z").as_deref(), Some("parity-zoe"));
    assert_eq!(suggestion_for(&mut h.field, "parity-ze").as_deref(), Some("parity-zed"));
}

#[test]
fn a_failed_collaborators_answer_keeps_the_configured_logins() {
    let mut h = Harness::new(&["parity-zed"]);
    h.with(|field, cx| field.load(&repo(), cx));
    let id = h.collaborators_request().unwrap();
    h.answer(id, GithubResponse::Collaborators(Err(not_found())));
    assert_eq!(suggestion_for(&mut h.field, "parity").as_deref(), Some("parity-zed"));
    assert!(h.toasts().is_empty(), "silently");
}

#[test]
fn other_responses_are_handed_back() {
    let mut h = Harness::new(&[]);
    let mut unclaimed = None;
    h.with(|field, cx| {
        unclaimed = field.on_response(RequestId::new(99), GithubResponse::Collaborators(Ok(vec![])), cx);
    });
    assert!(unclaimed.is_some());
}

// Enter.

#[test]
fn enter_looks_up_the_trimmed_lower_cased_login() {
    let mut h = Harness::new(&[]);
    h.type_text(" Parity-Friend ");
    h.enter();
    assert_eq!(
        h.lookups().into_iter().map(|(_, login)| login).collect::<Vec<_>>(),
        ["parity-friend"]
    );
}

#[test]
fn a_blank_enter_sends_nothing() {
    let mut h = Harness::new(&[]);
    h.enter();
    h.type_text("   ");
    h.enter();
    assert!(h.lookups().is_empty());
    assert!(h.toasts().is_empty());
}

#[test]
fn a_listed_login_is_already_a_reviewer() {
    let mut h = Harness::new(&[]);
    h.add(&["parity-friend"]);
    h.type_text("parity-friend");
    h.enter();
    assert_eq!(h.lookups().len(), 1, "no second lookup");
    assert_eq!(h.toasts(), [(Severity::Error, "Already a reviewer!".to_owned())]);
    assert_eq!(h.field.input_value(), "parity-friend", "the text stays");
}

#[test]
fn an_unknown_user_toasts_with_the_login_in_yellow_and_keeps_the_text() {
    for error in [
        not_found(),
        GhError::Decode(serde_json::from_str::<u8>("x").unwrap_err()),
    ] {
        let mut h = Harness::new(&[]);
        h.type_text("nobody-here");
        h.enter();
        h.answer_lookup(Err(error));
        assert_eq!(
            h.toasts(),
            [(
                Severity::Error,
                "Could not find user nobody-here - are you sure they exist?".to_owned()
            )]
        );
        let toast = h.app.app().toasts().iter().next().unwrap();
        assert_eq!(
            toast.message.spans[1].style.fg,
            Some(Theme::default().accent(Accent::Yellow))
        );
        assert_eq!(h.field.input_value(), "nobody-here");
        assert!(h.field.reviewers().is_empty());
    }
}

#[test]
fn a_found_user_is_listed_checked_and_the_input_cleared() {
    let mut h = Harness::new(&[]);
    assert_eq!(h.field.focus_stops(), [ReviewersFocus::Input]);
    let empty = h.field.height();
    h.add(&["parity-friend"]);
    assert_eq!(h.field.reviewers(), ["parity-friend"]);
    assert!(h.field.list.is_selected(0));
    assert_eq!(h.field.input_value(), "");
    assert_eq!(h.field.focus_stops(), [ReviewersFocus::Input, ReviewersFocus::List]);
    // The list's label and the list: one row and a bordered row.
    assert_eq!(h.field.height(), empty + 1 + 3);
}

#[test]
fn a_second_answer_for_a_listed_login_isnt_added_twice() {
    let mut h = Harness::new(&[]);
    h.type_text("parity-friend");
    h.enter();
    h.enter();
    let lookups = h.lookups();
    assert_eq!(lookups.len(), 2, "both Enters went out before either answered");
    for (id, _) in lookups {
        h.answer(id, GithubResponse::User(Ok(Box::new(user("parity-friend")))));
    }
    assert_eq!(h.field.reviewers(), ["parity-friend"]);
}

#[test]
fn existing_reviewers_keep_their_order_and_case_and_skip_duplicates() {
    let mut h = Harness::new(&[]);
    h.add(&["parity-friend"]);
    h.field.add_existing([
        "Parity-Reviewer".to_owned(),
        "parity-friend".to_owned(),
        "other".to_owned(),
    ]);
    assert_eq!(h.field.reviewers(), ["parity-friend", "Parity-Reviewer", "other"]);
    // Typed logins are lower-cased, so only an exact match is a duplicate.
    h.type_text("parity-reviewer");
    h.enter();
    assert_eq!(h.lookups().len(), 2);
}

// The list.

#[test]
fn nothing_is_highlighted_when_the_list_first_gets_focus() {
    let mut h = Harness::new(&[]);
    h.add(&["alice", "bob"]);
    h.field.focus_gained(ReviewersFocus::List);
    assert_eq!(h.field.highlighted(), None);
    assert_eq!(h.press(ReviewersFocus::List, "down"), ReviewersOutcome::Handled);
    assert_eq!(h.field.highlighted(), Some(0));
}

#[test]
fn toggling_removes_the_reviewer_and_the_highlight_keeps_its_index() {
    let mut h = Harness::new(&[]);
    h.add(&["alice", "bob", "carol"]);
    let _ = h.field.on_binding(BindingId::TextualOptionListCursorDown);
    let _ = h.field.on_binding(BindingId::TextualOptionListCursorDown);
    assert_eq!(
        h.field.on_binding(BindingId::TextualSelectionListSelect),
        ReviewersOutcome::Removed {
            login: "bob".into(),
            emptied: false
        }
    );
    assert_eq!(h.field.reviewers(), ["alice", "carol"]);
    assert_eq!(h.field.highlighted(), Some(1));
    // `enter` toggles too; removing the last row moves the highlight to the new last.
    assert_eq!(
        h.field.on_binding(BindingId::TextualOptionListSelect),
        ReviewersOutcome::Removed {
            login: "carol".into(),
            emptied: false
        }
    );
    assert_eq!(h.field.highlighted(), Some(0));
    assert_eq!(removing_toast("carol").message.to_string(), "Removing carol");
    assert_eq!(removing_toast("carol").severity, Severity::Information);
}

#[test]
fn removing_the_last_reviewer_hides_the_list() {
    let mut h = Harness::new(&[]);
    h.add(&["alice"]);
    let _ = h.field.on_binding(BindingId::TextualOptionListCursorDown);
    assert_eq!(
        h.press(ReviewersFocus::List, "space"),
        ReviewersOutcome::Removed {
            login: "alice".into(),
            emptied: true
        }
    );
    assert_eq!(h.field.focus_stops(), [ReviewersFocus::Input]);
    assert_eq!(h.field.height(), 4);
}

#[test]
fn a_toggle_key_without_a_highlight_does_nothing() {
    let mut h = Harness::new(&[]);
    h.add(&["alice"]);
    assert_eq!(
        h.field.on_binding(BindingId::TextualSelectionListSelect),
        ReviewersOutcome::Handled
    );
    assert_eq!(h.field.reviewers(), ["alice"]);
}

#[test]
fn the_input_selects_its_text_on_focus() {
    let mut h = Harness::new(&[]);
    h.type_text("abc");
    h.field.focus_gained(ReviewersFocus::Input);
    assert_eq!(h.field.input.selected_text(), Some("abc"));
}

#[test]
fn closing_cancels_the_requests_in_flight() {
    let mut h = Harness::new(&[]);
    h.with(|field, cx| field.load(&repo(), cx));
    h.type_text("alice");
    h.enter();
    h.with(ReviewersField::cancel);
    let cancelled = h
        .app
        .effects()
        .iter()
        .filter(|effect| matches!(effect, Effect::Cancel(_)))
        .count();
    assert_eq!(cancelled, 2);
}

// The mouse.

fn press_at(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn a_click_on_a_reviewer_removes_it_and_one_on_the_input_focuses_it() {
    let mut h = Harness::new(&[]);
    h.add(&["alice", "bob"]);
    let area = Rect::new(0, 0, 40, h.field.height());
    let theme = Theme::default();
    let mut buf = Buffer::empty(area);
    h.field.render(&mut buf, area, &theme, theme.background(), None);
    // Rows: label, input (1-3), list label (4), list border (5), alice (6), bob (7).
    assert_eq!(
        h.field.handle_mouse(&press_at(8, 7), area),
        ReviewersOutcome::Removed {
            login: "bob".into(),
            emptied: false
        }
    );
    assert_eq!(
        h.field.handle_mouse(&press_at(8, 2), area),
        ReviewersOutcome::Focus(ReviewersFocus::Input)
    );
    assert!(h.field.is_selecting());
    assert_eq!(h.field.handle_mouse(&press_at(8, 0), area), ReviewersOutcome::Ignored);
}

// Drawing.

fn draw(field: &mut ReviewersField, focus: Option<ReviewersFocus>) -> String {
    let theme = Theme::default();
    let area = Rect::new(0, 0, 50, field.height());
    let mut buf = Buffer::empty(area);
    let under = theme.palette.surface_lighten_3.over(theme.background());
    field.render(&mut buf, area, &theme, under, focus);
    buffer_to_styled_string(&buf)
}

#[test]
fn reviewers_empty() {
    let mut h = Harness::new(&[]);
    insta::assert_snapshot!(draw(&mut h.field, None));
}

#[test]
fn reviewers_two_listed() {
    let mut h = Harness::new(&[]);
    h.add(&["parity-reviewer", "parity-friend"]);
    insta::assert_snapshot!(draw(&mut h.field, None));
}

#[test]
fn reviewers_list_focused_second_highlighted() {
    let mut h = Harness::new(&[]);
    h.add(&["parity-reviewer", "parity-friend"]);
    let _ = h.field.on_binding(BindingId::TextualOptionListCursorDown);
    let _ = h.field.on_binding(BindingId::TextualOptionListCursorDown);
    insta::assert_snapshot!(draw(&mut h.field, Some(ReviewersFocus::List)));
}

#[test]
fn reviewers_input_suggestion() {
    let mut h = Harness::new(&["parity-friend"]);
    h.type_text("parity-fr");
    insta::assert_snapshot!(draw(&mut h.field, Some(ReviewersFocus::Input)));
}
