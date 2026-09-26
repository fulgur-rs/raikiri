//! raikiri-wpt — WPT test setup with blitz oracle.
//!
//! Provides expectations parsers, reftest discovery/render/diff, runner
//! dispatch, and the blitz oracle delta. See `reftest`, `runner`, and
//! `oracle` modules for the execution path (spec §12.4, §12.9, §12.10).

pub mod expectations;
pub mod lint;
pub mod oracle;
pub mod parsing_invalid;
pub mod reftest;
pub mod runner;
pub mod screen;
pub mod text_css_i18n;
pub(crate) mod wpt_host;
pub mod wpt_host_resolver;

pub use screen::{ScreenRenderError, render_screen_url};
pub use wpt_host_resolver::{WptHostResolver, WptHostResolverError};
