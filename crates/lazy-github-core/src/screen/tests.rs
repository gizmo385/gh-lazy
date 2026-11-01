use super::*;

fn options() -> SvgOptions {
    SvgOptions {
        title: "LazyGithub".to_owned(),
        default_fg: (0xe0, 0xe0, 0xe0),
        default_bg: (0x12, 0x12, 0x12),
    }
}

/// Writes `text` from column `x` of row `y`, one cell per `char`, in `style`.
fn put(screen: &mut Screen, x: u16, y: u16, text: &str, style: CellStyle) {
    for (col, c) in (x..).zip(text.chars()) {
        let cell = screen.cell_mut(col, y).unwrap();
        cell.symbol = c.to_string();
        cell.style = style;
    }
}

/// Puts a two-cell grapheme at `(x, y)`.
fn put_wide(screen: &mut Screen, x: u16, y: u16, symbol: &str, style: CellStyle) {
    let cell = screen.cell_mut(x, y).unwrap();
    cell.symbol = symbol.to_owned();
    cell.style = style;
    let next = screen.cell_mut(x + 1, y).unwrap();
    next.symbol = String::new();
    next.style = style;
}

fn fg(color: Color) -> CellStyle {
    CellStyle {
        fg: color,
        ..CellStyle::default()
    }
}

/// The `<text>` elements of `svg`, one per line.
fn texts(svg: &str) -> Vec<&str> {
    svg.lines().filter(|line| line.starts_with("<text")).collect()
}

/// The `<rect>`s of `svg` other than the whole-picture background.
fn rects(svg: &str) -> Vec<&str> {
    svg.lines().filter(|line| line.starts_with("<rect x=")).collect()
}

/// A small well-formedness check, enough for what `to_svg` writes: one root element, every tag closed in order,
/// attribute values quoted, `&` only in the five predefined entities, and no character XML 1.0 forbids.
fn assert_well_formed(xml: &str) {
    for c in xml.chars() {
        let allowed = matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}');
        assert!(allowed, "forbidden character {c:?}");
    }
    let mut stack: Vec<String> = Vec::new();
    let mut roots = 0;
    let mut rest = xml;
    while let Some(start) = rest.find(['<', '&']) {
        let text = &rest[..start];
        assert!(!text.contains('>'), "a bare `>` in {text:?}");
        if !text.trim().is_empty() {
            assert!(!stack.is_empty(), "text outside the root: {text:?}");
        }
        rest = &rest[start..];
        if rest.starts_with('&') {
            let end = rest.find(';').expect("an unterminated entity");
            let entity = &rest[1..end];
            assert!(
                ["amp", "lt", "gt", "quot", "apos"].contains(&entity),
                "unknown entity &{entity};"
            );
            rest = &rest[end + 1..];
            continue;
        }
        let end = rest.find('>').expect("an unterminated tag");
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(stack.pop().as_deref(), Some(name), "mismatched </{name}>");
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, mut attrs) = tag.split_once(' ').unwrap_or((tag, ""));
        assert!(
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()),
            "bad tag {name:?}"
        );
        while !attrs.trim().is_empty() {
            let (key, after) = attrs.trim_start().split_once("=\"").expect("an unquoted attribute");
            assert!(!key.contains([' ', '<', '"']), "bad attribute name {key:?}");
            let close = after.find('"').expect("an unclosed attribute value");
            assert!(!after[..close].contains('<'), "`<` in an attribute value");
            attrs = &after[close + 1..];
        }
        if stack.is_empty() {
            roots += 1;
        }
        if !self_closing {
            stack.push(name.to_owned());
        }
    }
    assert!(rest.trim().is_empty(), "text after the root: {rest:?}");
    assert!(stack.is_empty(), "unclosed {stack:?}");
    assert_eq!(roots, 1, "not exactly one root element");
}

/// A small screen with what a frame has: box borders, bold, reversed, an emoji, dim text and colors of each kind.
fn sample() -> Screen {
    let mut screen = Screen::new(12, 4);
    let border = fg(Color::Rgb(0x00, 0x7a, 0xcc));
    put(&mut screen, 0, 0, "┌──────────┐", border);
    put(&mut screen, 0, 3, "└──────────┘", border);
    for y in 1..3 {
        put(&mut screen, 0, y, "│", border);
        put(&mut screen, 11, y, "│", border);
    }
    let bold = CellStyle {
        bold: true,
        ..fg(Color::Indexed(11))
    };
    put(&mut screen, 1, 1, "Repos", bold);
    put_wide(&mut screen, 7, 1, "🔎", CellStyle::default());
    let reversed = CellStyle {
        reversed: true,
        ..CellStyle::default()
    };
    put(&mut screen, 1, 2, "sel", reversed);
    let dim = CellStyle {
        dim: true,
        italic: true,
        ..CellStyle::default()
    };
    put(&mut screen, 5, 2, "a<b", dim);
    screen
}

#[test]
fn file_names_follow_python_isoformat() {
    let at = jiff::civil::date(2026, 11, 1).at(5, 20, 25, 627_683_000);
    assert_eq!(screenshot_file_name(&at), "LazyGithub_2026-11-01T05_20_25_627683.svg");
    // Python's `isoformat` leaves out the microseconds when they're 0.
    let on_the_second = jiff::civil::date(2026, 1, 2).at(3, 4, 5, 0);
    assert_eq!(
        screenshot_file_name(&on_the_second),
        "LazyGithub_2026-01-02T03_04_05.svg"
    );
    // Nanoseconds below a microsecond don't count, as Python has none.
    let sub_micro = jiff::civil::date(2026, 1, 2).at(3, 4, 5, 999);
    assert_eq!(screenshot_file_name(&sub_micro), "LazyGithub_2026-01-02T03_04_05.svg");
    let small = jiff::civil::date(2026, 1, 2).at(3, 4, 5, 42_000);
    assert_eq!(
        screenshot_file_name(&small),
        "LazyGithub_2026-01-02T03_04_05_000042.svg"
    );
}

#[cfg(not(any(target_os = "macos", windows)))]
mod downloads {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    /// The directory for a `user-dirs.dirs` holding `text` (`None`: no file), and the file that was read.
    fn resolve(xdg: Option<&Path>, text: Option<&str>) -> (PathBuf, PathBuf) {
        let mut read_path = PathBuf::new();
        let dir = downloads_dir(&home(), xdg, |path| {
            read_path = path.to_path_buf();
            text.map(str::to_owned)
        });
        (dir, read_path)
    }

    #[test]
    fn without_user_dirs_it_is_home_downloads() {
        let (dir, read) = resolve(None, None);
        assert_eq!(dir, home().join("Downloads"));
        assert_eq!(read, home().join(".config").join("user-dirs.dirs"));
    }

    #[test]
    fn xdg_config_home_moves_the_file() {
        let (_, read) = resolve(Some(Path::new("/cfg")), None);
        assert_eq!(read, Path::new("/cfg/user-dirs.dirs"));
    }

    #[test]
    fn a_file_without_the_key_falls_back() {
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\n";
        assert_eq!(resolve(None, Some(text)).0, home().join("Downloads"));
        assert_eq!(
            resolve(None, Some("XDG_DOWNLOAD_DIR=\"\"\n")).0,
            home().join("Downloads")
        );
    }

    #[test]
    fn home_is_expanded_and_quotes_stripped() {
        let text = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\n\
                    XDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n";
        assert_eq!(resolve(None, Some(text)).0, home().join("Téléchargements"));
    }

    #[test]
    fn an_absolute_path_is_kept() {
        let text = "xdg_download_dir = \"/data/dl\"\n";
        assert_eq!(resolve(None, Some(text)).0, Path::new("/data/dl"));
        assert_eq!(resolve(None, Some("XDG_DOWNLOAD_DIR: /mnt/x\n")).0, Path::new("/mnt/x"));
    }

    #[test]
    fn comment_lines_are_skipped() {
        let text = "#XDG_DOWNLOAD_DIR=\"/nope\"\n;XDG_DOWNLOAD_DIR=\"/nope\"\nXDG_DOWNLOAD_DIR=\"/yes\"\n";
        assert_eq!(resolve(None, Some(text)).0, Path::new("/yes"));
    }
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn macos_and_windows_use_home_downloads() {
    let home = Path::new("home");
    let dir = downloads_dir(home, None, |_| panic!("nothing is read"));
    assert_eq!(dir, home.join("Downloads"));
}

#[test]
fn xml_specials_are_escaped_and_forbidden_characters_replaced() {
    let mut screen = Screen::new(12, 1);
    put(&mut screen, 0, 0, "<&>\"'\u{1}\u{1b}\u{fffe}\t", CellStyle::default());
    let svg = screen.to_svg(&SvgOptions {
        title: "a & <b>".to_owned(),
        ..options()
    });
    assert_well_formed(&svg);
    assert!(svg.contains("<title>a &amp; &lt;b&gt;</title>"));
    assert!(
        svg.contains(">&lt;&amp;&gt;&quot;&apos;\u{fffd}\u{fffd}\u{fffd}\t   </text>"),
        "{svg}"
    );
}

#[test]
fn the_sample_is_well_formed() {
    assert_well_formed(&sample().to_svg(&options()));
    assert_well_formed(&Screen::new(0, 0).to_svg(&options()));
}

#[test]
fn a_wide_character_spans_two_cells() {
    let mut screen = Screen::new(5, 1);
    put(&mut screen, 0, 0, "a", CellStyle::default());
    put_wide(&mut screen, 1, 0, "🔎", CellStyle::default());
    put(&mut screen, 3, 0, "bc", CellStyle::default());
    let svg = screen.to_svg(&options());
    assert_eq!(
        texts(&svg),
        [
            r#"<text class="s0" x="0" y="15" textLength="10" lengthAdjust="spacingAndGlyphs">a</text>"#,
            r#"<text class="s0" x="10" y="15" textLength="20" lengthAdjust="spacingAndGlyphs">🔎</text>"#,
            r#"<text class="s0" x="30" y="15" textLength="20" lengthAdjust="spacingAndGlyphs">bc</text>"#,
        ]
    );
    assert_eq!(screen.to_text(), "a🔎bc");
}

#[test]
fn a_multi_character_grapheme_is_a_run_of_its_own() {
    let mut screen = Screen::new(6, 1);
    put(&mut screen, 0, 0, "ab", CellStyle::default());
    // A combining sequence is one grapheme of two characters, so it's a run of its own.
    screen.cell_mut(2, 0).unwrap().symbol = "e\u{301}".to_owned();
    put(&mut screen, 3, 0, "cd", CellStyle::default());
    let svg = screen.to_svg(&options());
    assert_eq!(
        texts(&svg),
        [
            r#"<text class="s0" x="0" y="15" textLength="20" lengthAdjust="spacingAndGlyphs">ab</text>"#,
            "<text class=\"s0\" x=\"20\" y=\"15\" textLength=\"10\" lengthAdjust=\"spacingAndGlyphs\">e\u{301}</text>",
            r#"<text class="s0" x="30" y="15" textLength="30" lengthAdjust="spacingAndGlyphs">cd </text>"#,
        ]
    );
}

#[test]
fn reversed_swaps_the_default_colors() {
    let mut screen = Screen::new(2, 1);
    let reversed = CellStyle {
        reversed: true,
        ..CellStyle::default()
    };
    put(&mut screen, 0, 0, "x", reversed);
    let svg = screen.to_svg(&options());
    assert_eq!(
        rects(&svg),
        [r##"<rect x="0" y="0" width="10" height="20" fill="#e0e0e0"/>"##]
    );
    assert!(svg.contains(".s0 { fill: #121212; }"), "{svg}");
}

#[test]
fn indexed_and_rgb_colors() {
    let mut screen = Screen::new(3, 1);
    put(&mut screen, 0, 0, "a", fg(Color::Indexed(1)));
    put(&mut screen, 1, 0, "b", fg(Color::Indexed(196)));
    let style = CellStyle {
        fg: Color::Rgb(1, 2, 3),
        bg: Color::Indexed(244),
        ..CellStyle::default()
    };
    put(&mut screen, 2, 0, "c", style);
    let svg = screen.to_svg(&options());
    assert!(svg.contains(".s0 { fill: #cd0000; }"), "{svg}");
    assert!(svg.contains(".s1 { fill: #ff0000; }"), "{svg}");
    assert!(svg.contains(".s2 { fill: #010203; }"), "{svg}");
    assert_eq!(
        rects(&svg),
        [r##"<rect x="20" y="0" width="10" height="20" fill="#808080"/>"##]
    );
}

#[test]
fn xterm_palette() {
    assert_eq!(xterm_rgb(0), (0, 0, 0));
    assert_eq!(xterm_rgb(12), (0x5c, 0x5c, 0xff));
    assert_eq!(xterm_rgb(16), (0, 0, 0));
    assert_eq!(xterm_rgb(196), (255, 0, 0));
    assert_eq!(xterm_rgb(231), (255, 255, 255));
    assert_eq!(xterm_rgb(232), (8, 8, 8));
    assert_eq!(xterm_rgb(255), (238, 238, 238));
}

#[test]
fn runs_merge_per_row() {
    let mut screen = Screen::new(6, 2);
    let red_bg = CellStyle {
        bg: Color::Rgb(200, 0, 0),
        ..CellStyle::default()
    };
    put(&mut screen, 0, 0, "abc", red_bg);
    put(&mut screen, 3, 0, "def", CellStyle::default());
    put(&mut screen, 0, 1, "ghijkl", red_bg);
    let svg = screen.to_svg(&options());
    // One background run and one text run per style change, and rows never merge.
    assert_eq!(
        rects(&svg),
        [
            r##"<rect x="0" y="0" width="30" height="20" fill="#c80000"/>"##,
            r##"<rect x="0" y="20" width="60" height="20" fill="#c80000"/>"##,
        ]
    );
    assert_eq!(
        texts(&svg),
        [
            r#"<text class="s0" x="0" y="15" textLength="60" lengthAdjust="spacingAndGlyphs">abcdef</text>"#,
            r#"<text class="s0" x="0" y="35" textLength="60" lengthAdjust="spacingAndGlyphs">ghijkl</text>"#,
        ]
    );
}

#[test]
fn blank_runs_draw_no_text_unless_decorated() {
    let mut screen = Screen::new(4, 1);
    let underline = CellStyle {
        underline: true,
        ..CellStyle::default()
    };
    put(&mut screen, 2, 0, "  ", underline);
    let svg = screen.to_svg(&options());
    assert_eq!(
        texts(&svg),
        [r#"<text class="s0" x="20" y="15" textLength="20" lengthAdjust="spacingAndGlyphs">  </text>"#]
    );
    assert!(svg.contains(".s0 { fill: #e0e0e0; text-decoration: underline; }"));
}

#[test]
fn text_trims_rows_and_skips_wide_continuations() {
    let mut screen = Screen::new(4, 3);
    put(&mut screen, 0, 0, "ab", CellStyle::default());
    put_wide(&mut screen, 0, 2, "🔎", CellStyle::default());
    assert_eq!(screen.to_text(), "ab\n\n🔎");
}

#[test]
fn out_of_range_cells_are_none() {
    let mut screen = Screen::new(2, 2);
    assert!(screen.cell(1, 1).is_some());
    assert!(screen.cell(2, 0).is_none());
    assert!(screen.cell_mut(0, 2).is_none());
}

#[test]
fn sample_svg() {
    insta::assert_snapshot!(sample().to_svg(&options()));
}

#[test]
fn sample_text() {
    insta::assert_snapshot!(sample().to_text());
}
