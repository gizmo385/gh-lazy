//! The ratatui user interface for LazyGithub: app state, actions, components, theme and keymap.
//!
//! One [`App`](app::App) owns all UI state. Terminal input, finished requests, timers and log lines arrive as
//! [`Event`](event::Event)s; `App` maps them to [`Action`](action::Action)s and applies them synchronously, returning
//! [`Effect`](effect::Effect)s that the [`Runtime`](runtime::Runtime) performs in the background. [`tui::run`] is the
//! loop that ties them together.

pub mod action;
pub mod app;
pub mod backend;
pub mod bug_report;
pub mod clock;
pub mod component;
pub mod components;
pub mod display;
pub mod effect;
pub mod event;
pub mod fuzzy;
pub mod gh_health;
pub mod keymap;
pub mod modal;
pub mod notifications;
pub mod options;
pub mod rate_limit;
pub mod runtime;
pub mod screens;
pub mod theme;
pub mod toast;
pub mod tui;

#[cfg(any(test, feature = "test-util"))]
pub mod test_util;

#[cfg(test)]
mod perf_tests;
