use lazy_github_core::models::{Reaction, ReactionKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};

use super::*;
use crate::test_util::{buffer_to_string, buffer_to_styled_string, logs};

const REPO: &str = "parity-bot/pr-details";

fn user(login: &str, id: u64) -> User {
    User {
        login: login.into(),
        id,
        name: None,
        html_url: format!("https://github.com/{login}"),
    }
}

fn comment(body: &str, user: Option<User>) -> IssueComment {
    let created_at: Timestamp = "2026-02-03T12:00:00Z".parse().unwrap();
    IssueComment {
        id: 7001,
        body: body.into(),
        user,
        created_at,
        updated_at: created_at,
        author_association: "CONTRIBUTOR".into(),
    }
}

/// 7001 of the `pr-details` fixtures.
fn rebased() -> CommentBlock {
    CommentBlock::new(
        &comment(
            "Rebased on main. @parity-bot ready for another look.",
            Some(user("parity-friend", 2)),
        ),
        REPO,
    )
}

/// 7001's reactions: 👍 and 🚀.
fn reactions() -> Reactions {
    [
        Reaction {
            kind: ReactionKind::ThumbsUp,
            user: user("parity-bot", 1),
        },
        Reaction {
            kind: ReactionKind::Rocket,
            user: user("parity-reviewer", 3),
        },
    ]
    .into_iter()
    .collect()
}

fn draw(layout: &CommentLayout, width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, u16::try_from(layout.rendered.lines.len()).unwrap());
    let mut buf = Buffer::empty(area);
    Paragraph::new(layout.rendered.lines.clone()).render(area, &mut buf);
    buf
}

fn screen(block: &mut CommentBlock, width: u16, focus: Option<CommentStop>) -> String {
    let theme = Theme::default();
    let layout = block.layout(width, focus, theme.background(), &theme);
    buffer_to_string(&draw(&layout, width))
}

#[test]
fn the_author_line() {
    let at: Timestamp = "2026-02-03T12:00:00Z".parse().unwrap();
    assert_eq!(
        author_line(Some(&user("parity-friend", 2)), at),
        "parity-friend • Tue Feb  3 12:00:00 2026"
    );
    let later: Timestamp = "2026-11-23T01:02:03Z".parse().unwrap();
    assert_eq!(author_line(None, later), "Unknown • Mon Nov 23 01:02:03 2026");
}

#[test]
fn comment_plain() {
    insta::assert_snapshot!(screen(&mut rebased(), 60, None));
}

#[test]
fn comment_focused() {
    let theme = Theme::default();
    let layout = rebased().layout(60, Some(CommentStop::Block), theme.background(), &theme);
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&layout, 60)));
}

#[test]
fn comment_null_user() {
    let mut block = CommentBlock::new(&comment("Is this related to #4?", None), REPO);
    insta::assert_snapshot!(screen(&mut block, 60, None));
}

#[test]
fn comment_reactions_collapsed() {
    let mut block = rebased();
    block.set_reactions(Ok(reactions()));
    insta::assert_snapshot!(screen(&mut block, 60, None));
}

#[test]
fn comment_reactions_open() {
    let mut block = rebased();
    block.set_reactions(Ok(reactions()));
    block.toggle_reactions();
    let theme = Theme::default();
    let layout = block.layout(60, Some(CommentStop::ReactionsTitle), theme.background(), &theme);
    insta::assert_snapshot!(buffer_to_styled_string(&draw(&layout, 60)));
}

#[test]
fn comment_references_wide_and_narrow() {
    let body = "Rebased on main. @parity-bot ready for another look; see #4 and octo/other#12 too.";
    let mut block = CommentBlock::new(&comment(body, Some(user("parity-friend", 2))), REPO);
    let wide = screen(&mut block, 100, None);
    let narrow = screen(&mut block, 36, Some(CommentStop::Block));
    insta::assert_snapshot!(format!("{wide}\n{narrow}"));
}

#[test]
fn references_are_links_but_not_in_code() {
    let body = "See #4 and @parity-bot, not `#5`.\n\n```\n#6 @nobody\n```\n";
    let mut block = CommentBlock::new(&comment(body, None), REPO);
    let theme = Theme::default();
    let layout = block.layout(60, None, theme.background(), &theme);
    let targets: Vec<&str> = layout.rendered.links.iter().map(|link| &*link.target).collect();
    assert_eq!(
        targets,
        [
            "https://github.com/parity-bot/pr-details/issues/4",
            "https://github.com/parity-bot"
        ]
    );
    let text = buffer_to_string(&draw(&layout, 60));
    assert!(text.contains("#6 @nobody"), "{text}");
    assert!(text.contains("not #5"), "{text}");
    // The links sit where their text is, from the block's left edge.
    let first = &layout.rendered.links[0];
    let row: String = layout.rendered.lines[first.row].to_string();
    let cells: String = row
        .chars()
        .skip(usize::from(first.columns.start))
        .take(first.columns.len())
        .collect();
    assert_eq!(cells, "#4");
}

#[test]
fn the_reactions_title_is_a_separate_stop() {
    let mut block = rebased();
    assert_eq!(block.stops(), [CommentStop::Block]);
    block.set_reactions(Ok(reactions()));
    assert!(block.has_reactions());
    assert_eq!(block.stops(), [CommentStop::Block, CommentStop::ReactionsTitle]);
    assert!(block.on_binding(CommentStop::ReactionsTitle, BindingId::TextualCollapsibleToggle));
    assert!(block.reactions_open());
    assert_eq!(
        block.stops(),
        [
            CommentStop::Block,
            CommentStop::ReactionsTitle,
            CommentStop::ReactionsList
        ]
    );
    assert!(block.on_binding(CommentStop::ReactionsList, BindingId::TextualListCursorDown));
    assert!(block.on_binding(CommentStop::ReactionsList, BindingId::TextualListCursorDown));
    assert_eq!(block.reaction_highlight(), Some(1));
    assert!(block.on_binding(CommentStop::ReactionsList, BindingId::TextualListCursorUp));
    assert_eq!(block.reaction_highlight(), Some(0));
    assert!(block.on_binding(CommentStop::ReactionsList, BindingId::TextualListSelect));
    // Reply isn't built yet (#182).
    assert!(!block.on_binding(CommentStop::Block, BindingId::ConversationCommentReply));
    assert!(!block.on_binding(CommentStop::Block, BindingId::TextualCollapsibleToggle));

    // No reactions, or a failed request: no section, and the failure is logged at debug.
    block.set_reactions(Ok(Reactions::default()));
    assert_eq!(block.stops(), [CommentStop::Block]);
    let log = logs(|| {
        block.set_reactions(Err(GhError::Http {
            status: 404,
            message: "Not Found".into(),
            body: Vec::new(),
        }));
    });
    assert!(log.starts_with("DEBUG Could not find comment with ID 7001"), "{log}");
    assert!(!block.has_reactions());
    assert!(!block.on_binding(CommentStop::ReactionsTitle, BindingId::TextualCollapsibleToggle));
}

#[test]
fn chains_put_the_block_after_the_title_or_list() {
    let owners = |stop| -> Vec<Owner> { CommentBlock::chain(stop).into_iter().map(|link| link.owner).collect() };
    assert_eq!(owners(CommentStop::Block), [Owner::CommentBlock]);
    assert_eq!(
        owners(CommentStop::ReactionsTitle),
        [Owner::CollapsibleTitle, Owner::CommentBlock]
    );
    assert_eq!(
        owners(CommentStop::ReactionsList),
        [Owner::ListView, Owner::CommentBlock]
    );
    assert_eq!(CommentBlock::chain(CommentStop::Block)[0].bindings, COMMENT_BINDINGS);
}

#[test]
fn clicks_hit_the_title_the_items_or_the_block() {
    let theme = Theme::default();
    let mut block = rebased();
    block.set_reactions(Ok(reactions()));
    block.toggle_reactions();
    let layout = block.layout(60, None, theme.background(), &theme);
    let placement = layout.reactions.clone().unwrap();
    assert_eq!(
        layout.hit(placement.title_row, placement.title_columns.start),
        Some(CommentHit::ReactionsTitle)
    );
    assert_eq!(layout.hit(placement.title_row, 59), Some(CommentHit::Block));
    let second = layout.reaction_items[1].start;
    assert_eq!(layout.hit(second, 8), Some(CommentHit::ReactionsItem(1)));
    assert_eq!(layout.hit(second, 2), Some(CommentHit::Block));
    assert_eq!(layout.hit(0, 0), Some(CommentHit::Block));
    let rows = layout.rendered.lines.len();
    assert_eq!(layout.hit(rows - 1, 5), None);
    assert_eq!(layout.hit(rows, 5), None);
}

#[test]
fn rows_fill_the_width_and_the_focused_box_adds_two() {
    let theme = Theme::default();
    let mut block = rebased();
    let plain = block.layout(40, None, theme.background(), &theme);
    let focused = block.layout(40, Some(CommentStop::Block), theme.background(), &theme);
    assert_eq!(focused.rendered.lines.len(), plain.rendered.lines.len() + 2);
    assert!(plain.rendered.lines.iter().all(|line| line.width() == 40));
    assert!(focused.rendered.lines.iter().all(|line| line.width() == 40));
    // Too narrow for anything.
    assert_eq!(
        block.layout(3, None, theme.background(), &theme),
        CommentLayout::default()
    );
}
