//! raikiri-wpt — WPT test setup with blitz oracle.
//!
//! Provides expectations parsers, reftest discovery/render/diff, runner
//! dispatch, and the blitz oracle delta. See `reftest`, `runner`, and
//! `oracle` modules for the execution path (spec §12.4, §12.9, §12.10).

pub mod expectations;
mod http_resources;
pub mod lint;
pub mod oracle;
pub mod parsing_invalid;
pub mod print;
pub mod reftest;
pub mod runner;
pub mod screen;
#[cfg(test)]
mod test_http_server;
pub mod text_css_i18n;
pub(crate) mod wpt_host;
pub mod wpt_host_resolver;
pub mod wptrunner_browser;

pub use print::{PrintRenderError, render_print_url};
pub use screen::{ScreenRenderError, render_screen_url};
pub use wpt_host_resolver::{WptHostResolver, WptHostResolverError};
