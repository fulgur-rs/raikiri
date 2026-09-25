//! Inline style, computed style, `CSS.supports`, and geometry members.

use boa_engine::{Context, JsResult};

use super::interfaces::Members;

pub(crate) const HTML_ELEMENT_MEMBERS: Members = super::interfaces::NO_MEMBERS;

/// Install style-related globals.
pub(crate) fn install_globals(_context: &mut Context) -> JsResult<()> {
    Ok(())
}
