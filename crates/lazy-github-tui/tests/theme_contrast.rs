//! Body text stays readable in every built-in theme: WCAG 2 contrast for the text roles over the backgrounds they're
//! drawn on (#392).
//!
//! Body text (`text.normal`, `text.plain`) must meet AA, 4.5:1, over the screen, the panes and the dialog, toast and
//! card bodies. Over a table's header row and the diff viewer's tinted lines it must meet AA for large text and UI
//! components, 3:1: a header holds short labels, and a diff line's tint necessarily costs some contrast (Solarized
//! Dark's own body text is 4.75:1 on its bare background, so no visible green keeps it at 4.5). The app themes keep
//! the Python app's literal diff colors, so they're left out of the diff check. Syntax, muted and accent colors are
//! the upstream palettes' own and aren't held to a ratio here.

use lazy_github_tui::theme::{Paint, Rgb, Theme, ThemeColor};

/// WCAG AA for body text.
const BODY: f64 = 4.5;
/// WCAG AA for large text and UI components.
const LARGE: f64 = 3.0;

/// Themes that are the app's own rather than ports, whose diff lines keep the Python app's literal colors.
const PYTHON_DIFF_COLORS: [&str; 4] = ["dark", "light", "ansi-dark", "ansi-light"];

/// WCAG 2 relative luminance.
fn luminance(c: Rgb) -> f64 {
    let channel = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG 2 contrast ratio, from 1 to 21.
fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn rgb(paint: Paint) -> Option<Rgb> {
    match paint {
        Paint::Rgb(c) => Some(c),
        // The terminal's own colors, which the theme can't know.
        Paint::Ansi(_) => None,
    }
}

/// Every shortfall in `theme`, as `role over background: ratio < bar`.
fn shortfalls(theme: &Theme) -> Vec<String> {
    let r = theme.roles();
    let screen = theme.background();
    let pane = r.surface.pane.over(screen);
    let backgrounds: [(&str, Paint, f64); 10] = [
        ("surface.screen", screen, BODY),
        ("surface.pane", pane, BODY),
        ("surface.dialog", r.surface.dialog.over(screen), BODY),
        ("surface.raised", r.surface.raised.over(screen), BODY),
        ("surface.card", r.surface.card.over(screen), BODY),
        ("surface.header", r.surface.header.over(pane), LARGE),
        ("diff.added_bg", r.diff.added_bg.over(pane), LARGE),
        ("diff.removed_bg", r.diff.removed_bg.over(pane), LARGE),
        ("diff.added_current_bg", r.diff.added_current_bg.over(pane), LARGE),
        ("diff.removed_current_bg", r.diff.removed_current_bg.over(pane), LARGE),
    ];
    let texts: [(&str, ThemeColor); 2] = [("text.normal", r.text.normal), ("text.plain", r.text.plain)];
    let mut out = Vec::new();
    for (bg_role, bg, bar) in backgrounds {
        if bg_role.starts_with("diff.") && PYTHON_DIFF_COLORS.contains(&theme.name) {
            continue;
        }
        for (fg_role, fg) in texts {
            let (Some(fg), Some(under)) = (rgb(fg.over(bg)), rgb(bg)) else {
                continue;
            };
            let ratio = contrast(fg, under);
            if ratio < bar {
                out.push(format!("{fg_role} over {bg_role}: {ratio:.2} < {bar}"));
            }
        }
    }
    out
}

#[test]
fn body_text_meets_wcag_aa_in_every_builtin_theme() {
    let mut failures = Vec::new();
    for theme in Theme::all() {
        failures.extend(shortfalls(&theme).into_iter().map(|s| format!("{}: {s}", theme.name)));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn contrast_matches_wcag_reference_values() {
    let black = Rgb::new(0, 0, 0);
    let white = Rgb::new(255, 255, 255);
    assert!((contrast(black, white) - 21.0).abs() < 1e-9);
    assert!((contrast(white, white) - 1.0).abs() < 1e-9);
    // #767676 is the lightest gray that meets AA on white.
    assert!(contrast(Rgb::new(0x76, 0x76, 0x76), white) >= BODY);
    assert!(contrast(Rgb::new(0x77, 0x77, 0x77), white) < BODY);
}
