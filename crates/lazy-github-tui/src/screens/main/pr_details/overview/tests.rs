use lazy_github_core::models::{CheckConclusion, CheckRunStatus, CommitStatus, Reaction, ReactionKind, User};

use super::*;
use crate::screens::main::pr_details::tests::fixture_pr;

fn text(rendered: &Rendered) -> Vec<String> {
    rendered
        .lines
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

fn description(pr: &FullPullRequest) -> Option<Markdown> {
    let issue = &pr.pr.issue;
    issue
        .body
        .as_deref()
        .filter(|body| !body.is_empty())
        .map(|body| Markdown::parse(&references::expand(body, &issue.repo.full_name)))
}

fn hidden() -> [Section; 3] {
    std::array::from_fn(|_| Section {
        state: SectionState::Hidden,
        ..Section::default()
    })
}

const NO_FOCUS: [(SectionFocus, bool); 3] = [(
    SectionFocus {
        title: false,
        within: false,
    },
    false,
); 3];

/// The Overview of `pr` with `sections`, laid out `width` wide.
fn layout(pr: &FullPullRequest, sections: &[Section; 3], width: u16) -> (Rendered, Placements) {
    overview(
        pr,
        description(pr).as_ref(),
        sections,
        &NO_FOCUS,
        width,
        &Theme::default(),
    )
}

fn lines(pr: &FullPullRequest, width: u16) -> Vec<String> {
    text(&layout(pr, &hidden(), width).0)
}

fn user(login: &str, id: u64) -> User {
    User {
        login: login.into(),
        id,
        name: None,
        html_url: format!("https://github.com/{login}"),
    }
}

/// PR 5's reactions, as the fixtures list them.
fn pr5_reactions() -> Reactions {
    use ReactionKind::{Eyes, Hooray, ThumbsUp};
    [
        (ThumbsUp, "parity-bot", 1),
        (ThumbsUp, "parity-friend", 2),
        (Hooray, "parity-bot", 1),
        (Eyes, "parity-bot", 1),
        (Eyes, "parity-reviewer", 3),
        (Eyes, "parity-carol", 4),
        (Eyes, "parity-dave", 5),
    ]
    .into_iter()
    .map(|(kind, login, id)| Reaction {
        kind,
        user: user(login, id),
    })
    .collect()
}

/// PR 5's statuses and runs, as the fixtures list them.
fn pr5_checks() -> (CombinedStatus, Vec<CheckRun>) {
    let status = |context: &str, state, description: &str, url: Option<&str>| CommitStatus {
        context: context.into(),
        description: Some(description.into()),
        state,
        target_url: url.map(Into::into),
    };
    let run = |id, name: &str, status, conclusion| CheckRun {
        id,
        name: name.into(),
        status,
        conclusion,
        html_url: Some(format!("https://github.com/parity-bot/pr-details/runs/{id}")),
    };
    (
        CombinedStatus {
            state: CheckState::Pending,
            statuses: vec![
                status(
                    "ci/lint",
                    CheckState::Success,
                    "Lint passed",
                    Some("https://ci.example.com/lint/5"),
                ),
                status("ci/docs", CheckState::Pending, "Building docs", None),
            ],
        },
        vec![
            run(1, "build", CheckRunStatus::Completed, Some(CheckConclusion::Success)),
            run(2, "test", CheckRunStatus::Completed, Some(CheckConclusion::Failure)),
            run(3, "deploy", CheckRunStatus::InProgress, None),
        ],
    )
}

fn loaded_sections(open: [bool; 3]) -> [Section; 3] {
    let (status, runs) = pr5_checks();
    let mut sections: [Section; 3] = std::array::from_fn(|_| Section::loading());
    sections[0].settle(SectionState::Loaded(Loaded::Reactions(
        ReactionsSummary::new(&pr5_reactions()).unwrap(),
    )));
    sections[1].settle(SectionState::Loaded(Loaded::Checks {
        overall: overall_state(&status, &runs),
        rows: check_rows(&status, &runs),
    }));
    for (section, open) in sections.iter_mut().zip(open) {
        section.collapsible.set_open(open);
    }
    sections
}

#[test]
fn an_open_pull_request() {
    let pr = fixture_pr(5);
    let theme = Theme::default();
    let (rendered, _) = layout(&pr, &hidden(), 94);
    assert_eq!(
        text(&rendered)[..8],
        [
            "Open Add a cursor to the viewer (#5) by parity-friend",
            "3 commits • 5 files changed • +12 • -6 • parity-friend:cursor → parity-bot:main",
            "",
            "Opened on Sun Feb  1 08:30:00 2026",
            "",
            &"─".repeat(94),
            "",
            "  Adds a cursor to the viewer, as discussed in #3.",
        ]
    );
    let header = &rendered.lines[0].spans;
    assert_eq!(header[0].style.fg, Some(theme.accent(Accent::GreenYellow)));
    assert!(header[2].style.add_modifier.contains(Modifier::BOLD));
    assert!(!header[4].style.add_modifier.contains(Modifier::BOLD));
    let summary = &rendered.lines[1].spans;
    let plus = summary.iter().find(|span| span.content == "+12").unwrap();
    let minus = summary.iter().find(|span| span.content == "-6").unwrap();
    assert_eq!(plus.style.fg, Some(theme.accent(Accent::GreenYellow)));
    assert_eq!(minus.style.fg, Some(theme.accent(Accent::Red)));
    let head = summary
        .iter()
        .find(|span| span.content == "parity-friend:cursor")
        .unwrap();
    assert!(head.style.add_modifier.contains(Modifier::BOLD));

    let links: Vec<(usize, &str)> = rendered.links.iter().map(|link| (link.row, &*link.target)).collect();
    assert_eq!(
        links[..2],
        [
            (0, "https://github.com/parity-bot/pr-details/pull/5"),
            (0, "https://github.com/parity-friend")
        ]
    );
    // The description's references are links.
    assert!(
        links.contains(&(7, "https://github.com/parity-bot/pr-details/issues/3")),
        "{links:?}"
    );
    assert!(links.contains(&(12, "https://github.com/parity-bot")), "{links:?}");
}

#[test]
fn closed_and_merged_states() {
    let theme = Theme::default();
    let closed = fixture_pr(4);
    let (rendered, _) = layout(&closed, &hidden(), 94);
    assert_eq!(text(&rendered)[0], "closed Drop the legacy renderer (#4) by parity-bot");
    assert_eq!(rendered.lines[0].spans[0].style.fg, Some(theme.accent(Accent::Red)));
    assert_eq!(
        lines(&closed, 94)[..8],
        [
            "closed Drop the legacy renderer (#4) by parity-bot",
            "1 commit • 1 file changed • +0 • -12 • parity-bot:drop-legacy → parity-bot:main",
            "",
            "Opened on Tue Jan 20 09:00:00 2026",
            "",
            &"─".repeat(94),
            "",
            "  Superseded by the new renderer.",
        ]
    );

    let merged = fixture_pr(3);
    let (rendered, _) = layout(&merged, &hidden(), 94);
    assert_eq!(rendered.lines[0].spans[0].style.fg, Some(theme.accent(Accent::Orchid)));
    // A merged pull request is closed too: Merged wins.
    assert!(merged.pr.issue.closed_at.is_some());
    assert_eq!(
        lines(&merged, 94),
        [
            "Merged Release 0.2.0 (#3) by parity-bot",
            "2 commits • 2 files changed • +3 • -2 • parity-bot:release-0.2.0 → parity-bot:main",
            "",
            "Opened on Sat Jan 10 14:00:00 2026 • Merged on Mon Jan 12 16:20:00 2026",
            "",
            &"─".repeat(94),
            "",
        ]
    );
}

#[test]
fn a_title_with_brackets_is_literal() {
    let mut pr = fixture_pr(5);
    pr.pr.issue.title = "[docs] Fix [b]bold[/b] and [/]".into();
    assert_eq!(
        lines(&pr, 94)[0],
        "Open [docs] Fix [b]bold[/b] and [/] (#5) by parity-friend"
    );
}

#[test]
fn an_empty_description_shows_nothing_below_the_rule() {
    let mut pr = fixture_pr(5);
    pr.pr.issue.body = Some(String::new());
    assert_eq!(lines(&pr, 40).last().unwrap(), "");
    assert_eq!(lines(&pr, 40).len(), 9);
}

#[test]
fn the_header_wraps_at_the_width() {
    let pr = fixture_pr(5);
    assert_eq!(
        lines(&pr, 40)[..5],
        [
            "Open Add a cursor to the viewer (#5) by",
            "parity-friend",
            "3 commits • 5 files changed • +12 • -6 •",
            "parity-friend:cursor → parity-bot:main",
            "",
        ]
    );
    assert!(layout(&pr, &hidden(), 0).0.lines.is_empty());
}

#[test]
fn plurals() {
    assert_eq!(plural(1, "commit", "commits"), "1 commit");
    assert_eq!(plural(0, "commit", "commits"), "0 commits");
    assert_eq!(plural(2, "file changed", "files changed"), "2 files changed");
}

#[test]
fn the_sections_sit_between_the_dates_and_the_rule() {
    let pr = fixture_pr(5);
    let (rendered, placements) = layout(&pr, &loaded_sections([false; 3]), 30);
    let rule = "▔".repeat(30);
    let text = text(&rendered);
    let dates = text.iter().position(|line| line.starts_with("Opened on")).unwrap();
    assert_eq!(
        text[dates..dates + 12],
        [
            "Opened on Sun Feb  1 08:30:00",
            "2026",
            &rule,
            "  ▶ 👍 2 | 🎉 1 | 👀 4",
            "",
            &rule,
            "  ▶ Status checks: ✘ Failed",
            "",
            &rule,
            "  ▶ Reviews: ...",
            "",
            "",
        ]
    );
    let reactions = placements[0].as_ref().unwrap();
    assert_eq!(reactions.section.rows, dates + 2..dates + 5);
    assert_eq!(reactions.section.title_row, dates + 3);
    assert!(reactions.items.is_empty());
}

#[test]
fn open_sections_list_their_rows() {
    let pr = fixture_pr(5);
    let (rendered, placements) = layout(&pr, &loaded_sections([true, true, false]), 94);
    let text = text(&rendered);
    let start = text.iter().position(|line| line.contains("▼ 👍 2")).unwrap();
    assert_eq!(
        text[start..start + 16],
        [
            "  ▼ 👍 2 | 🎉 1 | 👀 4",
            "",
            "    👍: parity-bot, parity-friend",
            "    🎉: parity-bot",
            "    👀: parity-bot, parity-reviewer, parity-carol, and 1 more",
            "",
            &"▔".repeat(94),
            "  ▼ Status checks: ✘ Failed",
            "",
            "    ✘ Failed test",
            "    ... Pending ci/docs - Building docs",
            "    ... Pending deploy",
            "    ✓ Passed build",
            "    ✓ Passed ci/lint - Lint passed",
            "",
            &"▔".repeat(94),
        ]
    );
    let checks = placements[1].as_ref().unwrap();
    assert_eq!(checks.items.len(), 5);
    assert_eq!(checks.items[0], start + 9..start + 10);
    // Each linked row links its whole text, moved in with the indent.
    let row = start + 13;
    let links: Vec<(&str, std::ops::Range<u16>)> = rendered
        .links
        .iter()
        .filter(|link| link.row == row)
        .map(|link| (&*link.target, link.columns.clone()))
        .collect();
    assert_eq!(
        links,
        [
            ("https://ci.example.com/lint/5", 4..12),
            ("https://ci.example.com/lint/5", 12..34),
        ]
    );
    // `ci/docs` has no link.
    assert!(!rendered.links.iter().any(|link| link.row == start + 10));
}

#[test]
fn failed_sections_say_so() {
    let pr = fixture_pr(5);
    let mut sections = hidden();
    sections[0].settle(SectionState::Failed);
    sections[1].settle(SectionState::Failed);
    let text = text(&layout(&pr, &sections, 94).0);
    assert!(text.contains(&"  ▶ Reactions: failed to load".to_owned()), "{text:#?}");
    assert!(
        text.contains(&"  ▶ Status checks: failed to load".to_owned()),
        "{text:#?}"
    );
}

#[test]
fn a_status_without_a_description_has_no_dash() {
    let row = CheckRow {
        state: CheckState::Error,
        name: "ci/x".into(),
        description: None,
        url: None,
    };
    let runs = check_row(&row, &Theme::default());
    let text: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(text, "✘ Errored ci/x");
}

#[test]
fn check_states_read_as_python_draws_them() {
    let theme = Theme::default();
    for (state, text, color) in [
        (CheckState::Success, "✓ Passed", Accent::GreenYellow),
        (CheckState::Pending, "... Pending", Accent::Yellow),
        (CheckState::Failure, "✘ Failed", Accent::Red),
        (CheckState::Error, "✘ Errored", Accent::Red),
    ] {
        let run = check_state(state, &theme);
        assert_eq!(run.text, text);
        assert_eq!(run.style.fg, Some(theme.accent(color)));
    }
}
