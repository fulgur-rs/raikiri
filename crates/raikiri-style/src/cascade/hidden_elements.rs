//! The user agent `display: none` of the HTML elements that are never
//! rendered.
//!
//! HTML Living Standard §15.3.1 "Hidden elements"
//! (<https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>)
//! gives metadata and fallback elements `display: none` from the user agent
//! style sheet. A type selector there would also match the SVG `script`,
//! `style` and `title`, and a namespaced selector keeps the user agent sheet
//! from being shared between documents, so the declaration is added here for
//! HTML elements only. It sits in the user agent origin below every user
//! agent rule, so an author `display` (or a user agent rule) still wins, as in
//! browsers, and `revert` comes back to it.

use std::borrow::Cow;

use crate::error::CascadeError;
use crate::layer::LayerPosition;
use crate::property::{DisplayValue, PropertyValue};
use crate::rule::Declaration;
use crate::ruletree::Origin;
use crate::style_dom::StyleElement;

use super::candidate::{CandidateSink, Precedence};

/// Adds the user agent `display: none` of `elem` when it is the HTML
/// `script`, `style`, `title` or `noscript`. The user agent style sheet hides
/// the other elements of §15.3.1 with a type selector, as no other namespace
/// uses their names. Scripting counts as enabled, as in browsers, so
/// `noscript` is among them. The names match case-sensitively, as in
/// `Node::is_non_rendered_html_element` in raikiri-dom, which names the same
/// elements: the HTML parser lowercases them, and an XHTML
/// `SCRIPT` is not the script element.
pub(crate) fn push_hidden_element_display(
    elem: &impl StyleElement,
    sink: &mut CandidateSink<'_>,
) -> Result<(), CascadeError> {
    let html = matches!(
        elem.namespace_uri(),
        None | Some("http://www.w3.org/1999/xhtml")
    );
    if !html || !matches!(elem.tag_name(), "script" | "style" | "title" | "noscript") {
        return Ok(());
    }
    sink.push_local(
        Cow::Owned(Declaration::new(
            PropertyValue::Display(DisplayValue::None),
            false,
        )),
        Precedence::new(Origin::UserAgent, false, 0, 0, LayerPosition::default()),
    )
}
