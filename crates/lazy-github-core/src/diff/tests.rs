use std::fmt::Write as _;

use super::*;

/// The diff `parity/fixtures/pr-details` serves for `pull_number`.
fn fixture_diff(pull_number: u32) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../parity/fixtures/pr-details/repos/parity-bot/pr-details/pulls/{pull_number}/GET.diff.json"
    ));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
    fixture["response"]["body_text"].as_str().unwrap().to_owned()
}

fn pr_5() -> Diff {
    parse(&fixture_diff(5)).unwrap()
}

fn paths(diff: &Diff) -> Vec<&str> {
    diff.files.iter().map(|file| file.path.as_str()).collect()
}

/// One line as `(kind, text, old, new, position)`.
type Row<'a> = (LineKind, &'a str, Option<u32>, Option<u32>, u32);

/// Each line as a [`Row`], for compact assertions.
fn rows(hunk: &Hunk) -> Vec<Row<'_>> {
    hunk.lines
        .iter()
        .map(|line| (line.kind, line.text.as_str(), line.old, line.new, line.position))
        .collect()
}

#[test]
fn pr_5_files_in_diff_order() {
    let diff = pr_5();
    assert_eq!(
        paths(&diff),
        [
            "README.md",
            "assets/app.css",
            "build.rs",
            "docs/old.md",
            "src/viewer.rs"
        ]
    );
    let deleted: Vec<bool> = diff.files.iter().map(|file| file.deleted).collect();
    assert_eq!(deleted, [false, false, false, true, false]);
    // The added file is an ordinary file whose old side is /dev/null.
    assert_eq!(diff.files[1].old_path, DEV_NULL);
    assert_eq!(diff.files[3].old_path, "docs/old.md");
    assert_eq!(diff.files[4].old_path, "src/viewer.rs");
    let hunk_counts: Vec<usize> = diff.files.iter().map(|file| file.hunks.len()).collect();
    assert_eq!(hunk_counts, [1, 1, 1, 1, 2]);
}

/// Py bug B55: Python strips the leading `b` along with the `+++ b/` prefix.
#[test]
fn build_rs_keeps_its_name() {
    let diff = pr_5();
    let build = &diff.files[2];
    assert_eq!(build.path, "build.rs");
    assert_eq!(build.old_path, "build.rs");
    assert_eq!(build.label().to_string(), "📄 build.rs (+1)");
}

#[test]
fn pr_5_viewer_rs_numbers_and_positions() {
    use LineKind::{Added, Context, Removed};
    let diff = pr_5();
    let viewer = &diff.files[4];
    let [first, second] = viewer.hunks.as_slice() else {
        panic!("expected two hunks: {:?}", viewer.hunks);
    };

    assert_eq!(first.header, "@@ -1,6 +1,8 @@");
    assert_eq!(
        first.range,
        Some(HunkRange {
            old_start: 1,
            old_count: 6,
            new_start: 1,
            new_count: 8,
        })
    );
    assert_eq!(first.context, "");
    assert_eq!(
        rows(first),
        [
            (Context, "use std::io;", Some(1), Some(1), 1),
            (Added, "use std::fmt;", None, Some(2), 2),
            (Context, "", Some(2), Some(3), 3),
            (Context, "pub struct Viewer {", Some(3), Some(4), 4),
            (Context, "    lines: Vec<String>,", Some(4), Some(5), 5),
            (Added, "    cursor: usize,", None, Some(6), 6),
            (Context, "}", Some(5), Some(7), 7),
            (Context, "", Some(6), Some(8), 8),
        ]
    );

    // The second header takes position 9, so its first line is 10. Its numbers start from its own header, not
    // from the old side's start (Py bug B22).
    assert_eq!(second.context, "impl Viewer {");
    assert_eq!(
        rows(second),
        [
            (
                Context,
                "    pub fn render(&self) -> io::Result<()> {",
                Some(20),
                Some(22),
                10
            ),
            (Removed, "        for line in &self.lines {", Some(21), None, 11),
            (Removed, "            println!(\"{line}\");", Some(22), None, 12),
            (
                Added,
                "        for (i, line) in self.lines.iter().enumerate() {",
                None,
                Some(23),
                13
            ),
            (
                Added,
                "            let marker = if i == self.cursor { \">\" } else { \" \" };",
                None,
                Some(24),
                14
            ),
            (Added, "            println!(\"{marker} {line}\");", None, Some(25), 15),
            (Context, "        }", Some(23), Some(26), 16),
            (Context, "        Ok(())", Some(24), Some(27), 17),
            (Context, "    }", Some(25), Some(28), 18),
        ]
    );
    assert_eq!(
        first.counts(),
        Counts {
            additions: 2,
            deletions: 0
        }
    );
    assert_eq!(
        second.counts(),
        Counts {
            additions: 3,
            deletions: 2
        }
    );
    assert_eq!(
        viewer.counts(),
        Counts {
            additions: 5,
            deletions: 2
        }
    );
}

#[test]
fn pr_5_labels() {
    let diff = pr_5();
    let file_labels: Vec<String> = diff.files.iter().map(|file| file.label().to_string()).collect();
    assert_eq!(
        file_labels,
        [
            "📄 README.md (+3 -1)",
            "📄 app.css (+3)",
            "📄 build.rs (+1)",
            "🗑️  old.md (deleted)",
            "📄 viewer.rs (+5 -2)",
        ]
    );
    let hunk_label = |file: usize, hunk: usize| diff.files[file].hunks[hunk].label().to_string();
    assert_eq!(hunk_label(0, 0), "Lines 3-7 (+3 -1) • A tiny pager.");
    assert_eq!(hunk_label(1, 0), "Lines 1-3 (+3)");
    assert_eq!(hunk_label(2, 0), "Lines 1-4 (+1)");
    assert_eq!(hunk_label(4, 0), "Lines 1-8 (+2)");
    assert_eq!(hunk_label(4, 1), "Lines 22-28 (+3 -2) • impl Viewer {");
    // A deleted file's `+0,0` range is titled from its old side (Py bug B65; Python shows `Lines 0--1`).
    assert_eq!(hunk_label(3, 0), "Lines 1-3 (-3)");
}

#[test]
fn labels_come_in_parts() {
    let diff = pr_5();
    let label = diff.files[0].hunks[0].label();
    assert_eq!(
        label,
        Label {
            text: "Lines 3-7".into(),
            counts: Counts {
                additions: 3,
                deletions: 1
            },
            suffix: " • A tiny pager.".into(),
        }
    );
    let deleted = diff.files[3].label();
    assert_eq!(deleted.text, "🗑️  old.md (deleted)");
    assert!(deleted.counts.is_empty());
    assert_eq!(
        diff.files[3].counts(),
        Counts {
            additions: 0,
            deletions: 3
        }
    );
}

#[test]
fn hunk_labels() {
    let label = |diff: &str| parse(diff).unwrap().files[0].hunks[0].label().to_string();
    let section = "diff --git a/f b/f\n--- a/f\n+++ b/f\n";
    assert_eq!(label(&format!("{section}@@ -1 +1 @@\n-a\n+b\n")), "Line 1 (+1 -1)");
    assert_eq!(
        label(&format!("{section}@@ -4,2 +4,2 @@  fn x()  \n a\n b\n")),
        "Lines 4-5 • fn x()"
    );
    // An empty new side is titled from the old side (Py bug B65; Python shows `Lines 9-8`).
    assert_eq!(label(&format!("{section}@@ -9 +9,0 @@\n-a\n")), "Line 9 (-1)");
    assert_eq!(label(&format!("{section}@@ -12,2 +11,0 @@\n-a\n-b\n")), "Lines 12-13 (-2)");
    // Not a header the label can read: shown as is, without counts.
    assert_eq!(label(&format!("{section}@@@ -1 -1 +1 @@@\n+x\n")), "@@@ -1 -1 +1 @@@");
    assert_eq!(label(&format!("{section}@@ -1 +x @@\n+x\n")), "@@ -1 +x @@");
}

#[test]
fn pr_3_and_9_parse() {
    let pr_3 = parse(&fixture_diff(3)).unwrap();
    assert_eq!(paths(&pr_3), ["Cargo.toml", "CHANGELOG.md"]);
    assert_eq!(pr_3.files[0].label().to_string(), "📄 Cargo.toml (+1 -1)");
    assert_eq!(pr_3.files[1].hunks[0].label().to_string(), "Lines 1-5 (+2)");
    let pr_9 = parse(&fixture_diff(9)).unwrap();
    assert_eq!(paths(&pr_9), ["src/viewer.rs"]);
    assert_eq!(
        pr_9.files[0].hunks[0].label().to_string(),
        "Lines 30-34 (+3 -1) • impl Viewer {"
    );
}

#[test]
fn an_empty_diff_has_no_files() {
    assert_eq!(parse("").unwrap(), Diff::default());
    assert_eq!(parse("\n").unwrap(), Diff::default());
}

#[test]
fn binary_files_and_pure_renames_are_files_without_hunks() {
    let diff = "diff --git a/one.txt b/one.txt\n\
                index 1..2 100644\n\
                --- a/one.txt\n\
                +++ b/one.txt\n\
                @@ -1 +1 @@\n\
                -1\n\
                +one\n\
                diff --git a/logo.png b/logo.png\n\
                index 3..4 100644\n\
                Binary files a/logo.png and b/logo.png differ\n\
                diff --git a/old name.rs b/new name.rs\n\
                similarity index 100%\n\
                rename from old name.rs\n\
                rename to new name.rs\n\
                diff --git a/script.sh b/script.sh\n\
                old mode 100644\n\
                new mode 100755\n\
                diff --git a/two.txt b/two.txt\n\
                --- a/two.txt\n\
                +++ b/two.txt\n\
                @@ -1 +1 @@\n\
                -2\n\
                +two\n\
                diff --git a/gone.bin b/gone.bin\n\
                deleted file mode 100644\n\
                index 5..0\n\
                Binary files a/gone.bin and /dev/null differ\n";
    let diff = parse(diff).unwrap();
    assert_eq!(
        paths(&diff),
        ["one.txt", "logo.png", "new name.rs", "script.sh", "two.txt", "gone.bin"]
    );
    let hunks: Vec<usize> = diff.files.iter().map(|file| file.hunks.len()).collect();
    assert_eq!(hunks, [1, 0, 0, 0, 1, 0]);
    assert_eq!(diff.files[2].old_path, "old name.rs");
    assert_eq!(diff.files[1].old_path, "logo.png");
    assert!(diff.files[5].deleted);
    assert!(!diff.files[1].deleted);
    // Neither swallowed the next file's lines.
    assert_eq!(diff.files[0].hunks[0].lines.len(), 2);
    assert_eq!(diff.files[4].hunks[0].lines[1].text, "two");
    assert_eq!(diff.files[1].label().to_string(), "📄 logo.png");
    assert_eq!(diff.files[5].label().to_string(), "🗑️  gone.bin (deleted)");
}

#[test]
fn the_no_newline_marker() {
    use LineKind::{Added, NoNewline, Removed};
    let diff = "diff --git a/f b/f\n\
                --- a/f\n\
                +++ b/f\n\
                @@ -1 +1 @@\n\
                -old\n\
                \\ No newline at end of file\n\
                +new\n\
                \\ No newline at end of file\n";
    let diff = parse(diff).unwrap();
    let hunk = &diff.files[0].hunks[0];
    assert_eq!(
        rows(hunk),
        [
            (Removed, "old", Some(1), None, 1),
            (NoNewline, "No newline at end of file", None, None, 2),
            (Added, "new", None, Some(1), 3),
            (NoNewline, "No newline at end of file", None, None, 4),
        ]
    );
    assert_eq!(
        hunk.counts(),
        Counts {
            additions: 1,
            deletions: 1
        }
    );
}

#[test]
fn a_header_without_counts() {
    let diff = parse("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n").unwrap();
    assert_eq!(
        diff.files[0].hunks[0].range,
        Some(HunkRange {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
        })
    );
}

#[test]
fn an_unparsable_header_keeps_its_lines_without_numbers() {
    let diff = parse("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ weird @@\n x\n+y\n").unwrap();
    let hunk = &diff.files[0].hunks[0];
    assert_eq!(hunk.range, None);
    assert_eq!(hunk.context, "");
    assert!(hunk.lines.iter().all(|line| line.old.is_none() && line.new.is_none()));
    assert_eq!(hunk.lines[1].position, 2);
}

#[test]
fn truncated_file_headers_are_errors() {
    let section = "diff --git a/f b/f";
    let end = DiffError::UnexpectedEnd {
        section: section.into(),
    };
    for diff in [
        format!("{section}\n--- a/f\n"),
        format!("{section}\nindex 1..2\n--- a/f\n+++ b/f\n"),
        format!("{section}\n--- a/f\n+++ b/f\ndiff --git a/g b/g\n"),
    ] {
        assert_eq!(parse(&diff), Err(end.clone()), "{diff:?}");
    }
    assert_eq!(
        parse(&format!("{section}\n--- a/f\nindex 1..2\n")),
        Err(DiffError::UnexpectedLine {
            section: section.into(),
            line: 3,
            text: "index 1..2".into(),
        })
    );
    assert_eq!(
        parse(&format!("{section}\n@@ -1 +1 @@\n")),
        Err(DiffError::UnexpectedLine {
            section: section.into(),
            line: 2,
            text: "@@ -1 +1 @@".into(),
        })
    );
    assert_eq!(
        end.to_string(),
        "unexpected end of the file header of `diff --git a/f b/f`"
    );
}

#[test]
fn crlf_line_endings() {
    let lf = fixture_diff(5);
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(parse(&crlf).unwrap(), parse(&lf).unwrap());
}

#[test]
fn text_before_the_first_section_is_ignored() {
    let diff = parse("From abc\nSubject: x\n\ndiff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n+a\n").unwrap();
    assert_eq!(paths(&diff), ["f"]);
}

#[test]
fn quoted_and_tab_terminated_paths() {
    let diff = "diff --git \"a/t\\303\\251st.md\" \"b/t\\303\\251st.md\"\n\
                --- \"a/t\\303\\251st.md\"\n\
                +++ \"b/t\\303\\251st.md\"\n\
                @@ -1 +1 @@\n\
                +x\n\
                diff --git a/my file b/my file\n\
                --- a/my file\t\n\
                +++ b/my file\t\n\
                @@ -1 +1 @@\n\
                +x\n\
                diff --git \"a/q\\\"uote\" \"b/q\\\"uote\"\n\
                new file mode 100644\n\
                diff --git a/b c b/b c\n\
                new file mode 100644\n";
    let diff = parse(diff).unwrap();
    assert_eq!(paths(&diff), ["tést.md", "my file", "q\"uote", "b c"]);
}

/// A small deterministic generator (xorshift), so the property test needs no extra crate.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        u32::try_from(self.0 % u64::from(n)).unwrap()
    }
}

/// Writes a random well-formed diff and returns it with the `+` / `-` lines of each file, in order.
fn random_diff(rng: &mut Rng) -> (String, Vec<Vec<String>>) {
    let mut text = String::new();
    let mut changes = Vec::new();
    for file in 0..rng.below(5) {
        let path = ["a", "b", "/x", "+p", "-m", "build.rs"][rng.below(6) as usize];
        let path = format!("{path}{file}");
        write!(
            text,
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n"
        )
        .unwrap();
        let mut file_changes = Vec::new();
        let (mut old, mut new) = (1, 1);
        for _ in 0..=rng.below(4) {
            let mut body = Vec::new();
            let (mut old_count, mut new_count) = (0, 0);
            for line in 0..=rng.below(8) {
                // Content that looks like headers, to check that nothing inside a hunk is mistaken for one.
                let content = ["", "--- a/x", "+++ b/x", "@ x", "diff", "plain"][rng.below(6) as usize];
                let content = format!("{content}{line}");
                match rng.below(3) {
                    0 => {
                        body.push(format!(" {content}"));
                        old_count += 1;
                        new_count += 1;
                    }
                    1 => {
                        body.push(format!("+{content}"));
                        file_changes.push(format!("+{content}"));
                        new_count += 1;
                    }
                    _ => {
                        body.push(format!("-{content}"));
                        file_changes.push(format!("-{content}"));
                        old_count += 1;
                    }
                }
            }
            writeln!(text, "@@ -{old},{old_count} +{new},{new_count} @@ ctx").unwrap();
            for line in body {
                text += &line;
                text.push('\n');
            }
            old += old_count + rng.below(5);
            new += new_count + rng.below(5);
        }
        changes.push(file_changes);
    }
    (text, changes)
}

#[test]
fn every_change_lands_in_one_hunk_and_positions_increase() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..500 {
        let (text, changes) = random_diff(&mut rng);
        let diff = parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(diff.files.len(), changes.len(), "{text}");
        for (file, expected) in diff.files.iter().zip(&changes) {
            let found: Vec<String> = file
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter_map(|line| match line.kind {
                    LineKind::Added => Some(format!("+{}", line.text)),
                    LineKind::Removed => Some(format!("-{}", line.text)),
                    LineKind::Context | LineKind::NoNewline => None,
                })
                .collect();
            assert_eq!(&found, expected, "{text}");
            let positions: Vec<u32> = file
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .map(|l| l.position)
                .collect();
            assert!(positions.is_sorted_by(|a, b| a < b), "{positions:?}\n{text}");
            assert_eq!(positions.first().copied(), Some(1));
            // Every hunk's counts agree with its header.
            for hunk in &file.hunks {
                let range = hunk.range.unwrap();
                let olds = hunk.lines.iter().filter(|line| line.old.is_some()).count();
                let news = hunk.lines.iter().filter(|line| line.new.is_some()).count();
                assert_eq!((olds, news), (range.old_count as usize, range.new_count as usize));
            }
        }
    }
}
