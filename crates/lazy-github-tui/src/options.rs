//! Process-wide switches and facts read once at startup from the environment. They're not settings.

use lazy_github_core::debug::DebugEnv;

/// The environment variable Textual reads its animation level from.
pub const TEXTUAL_ANIMATIONS_ENV: &str = "TEXTUAL_ANIMATIONS";

/// The environment variable that makes a debug build panic while a dialog is open: `update` on the next key, `render`
/// on the next draw. For the tests of how a panic leaves the terminal; release builds ignore it.
pub const TEST_PANIC_ENV: &str = "LAZY_GITHUB_TEST_PANIC";

/// Where [`TEST_PANIC_ENV`] makes the app panic, once a dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPanic {
    /// While handling a key.
    Update,
    /// While drawing.
    Render,
}

impl TestPanic {
    /// The trigger a [`TEST_PANIC_ENV`] value asks for, if any.
    pub fn from_var(value: Option<&str>) -> Option<Self> {
        match value? {
            "update" => Some(Self::Update),
            "render" => Some(Self::Render),
            _ => None,
        }
    }
}

/// Options the binary reads once, before the terminal is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppOptions {
    /// Whether things animate. Off with `TEXTUAL_ANIMATIONS=none`, as the parity harness sets it for both apps:
    /// loading overlays then show static text.
    pub animations: bool,
    /// The terminal as the debug info names it ([`DebugEnv::terminal`]), from `TERM_PROGRAM`,
    /// `TERM_PROGRAM_VERSION` and `TERM`.
    pub terminal: &'static str,
    /// A deliberate panic for tests, from [`TEST_PANIC_ENV`]. Always `None` in release builds.
    pub test_panic: Option<TestPanic>,
}

impl AppOptions {
    /// The options from the process environment.
    pub fn from_env() -> Self {
        let env = |name: &str| std::env::var(name).ok();
        let terminal = DebugEnv::detect(env, None).terminal;
        Self {
            // Read once per process, and the options are `Copy`, so the text is kept for the rest of it.
            terminal: Box::leak(terminal.into_boxed_str()),
            test_panic: if cfg!(debug_assertions) {
                TestPanic::from_var(env(TEST_PANIC_ENV).as_deref())
            } else {
                None
            },
            ..Self::from_animations_var(env(TEXTUAL_ANIMATIONS_ENV).as_deref())
        }
    }

    /// The options for a `TEXTUAL_ANIMATIONS` value. Textual lowercases it and treats anything other than `none`,
    /// `basic` or `full` as `full`; only `none` turns animations off.
    pub fn from_animations_var(value: Option<&str>) -> Self {
        Self {
            animations: !value.is_some_and(|v| v.eq_ignore_ascii_case("none")),
            ..Self::default()
        }
    }
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            animations: true,
            terminal: "unknown",
            test_panic: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_none_turns_animations_off() {
        for (value, animations) in [
            (None, true),
            (Some("none"), false),
            (Some("NONE"), false),
            (Some("basic"), true),
            (Some("full"), true),
            (Some("bogus"), true),
            (Some(""), true),
        ] {
            assert_eq!(
                AppOptions::from_animations_var(value).animations,
                animations,
                "{value:?}"
            );
        }
        assert_eq!(AppOptions::default(), AppOptions::from_animations_var(None));
    }

    #[test]
    fn the_test_panic_trigger_names_a_place() {
        assert_eq!(TestPanic::from_var(Some("update")), Some(TestPanic::Update));
        assert_eq!(TestPanic::from_var(Some("render")), Some(TestPanic::Render));
        assert_eq!(TestPanic::from_var(Some("")), None);
        assert_eq!(TestPanic::from_var(None), None);
    }
}
