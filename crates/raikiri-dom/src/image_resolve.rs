//! Pre-layout `<img>` intrinsic-size resolution.
//!
//! Mirrors [`crate::layout`]'s `preshape_text` role: runs once before taffy
//! layout, writes results onto [`crate::node::Node`], so the taffy
//! leaf-measure closures (`crate::taffy_impl`) stay synchronous flat reads
//! with no new integration logic into taffy's own trait surface. Each pass
//! reconciles the current result, including clearing stale sizes for removed
//! or invalid `src` values. A changed intrinsic size marks layout caches dirty
//! even when the caller reuses the same cascade generation.
//!
//! Two kinds of node are *not* visited, and so keep whatever value they
//! already held: nodes outside the rendered flat tree (`<template>`
//! descendants and other inert subtrees), which are skipped before the
//! reset; and — once a resolve fails — every node after the failing one,
//! since a resolver `Err` is terminal and returns immediately.

use raikiri_traits::{ReplacedResolver, ResolverError, ResolverRequest};
use url::Url;

use crate::document::Document;
use crate::node::NodeData;

/// Walks every element in `document`'s arena, resolves `<img src="...">`
/// intrinsic size via `resolver`, and stores the result on each node (see
/// [`crate::node::Node::image_intrinsic_size`]).
///
/// Only nodes that are part of the rendered flat tree are visited —
/// [`crate::node::Node::is_in_document`] gates the walk, so an `<img>`
/// inside a `<template>` (or any other inert subtree) never reaches
/// `resolver.resolve()`, and therefore never triggers the fetch behind it,
/// for an element that is never laid out or painted. Membership is decided
/// by that flag rather than by a tag-name test, per this crate's module
/// doc "Flat tree membership". The caller owns flag freshness
/// ([`Document::mark_in_document_flags`]);
/// [`crate::layout::layout_single_page_with_resolver`] refreshes them
/// immediately before calling here.
///
/// Only absolute `src` URLs are resolved (`Url::parse` must succeed
/// directly). Relative-URL resolution against a document base URL is out
/// of scope, because no base URL is persisted on [`Document`] past parse
/// time. An `<img>` with an unparsable/relative/missing `src` is left with
/// `image_intrinsic_size = None` (renders as a 0×0 replaced box — no
/// fallback size in this scope).
///
/// # Errors
/// [`ResolverError`] — propagated verbatim from the first
/// `resolver.resolve()` that fails, which also ends the walk. A resolver
/// `Err` is terminal by contract (see
/// [`raikiri_traits::ReplacedResolver`]): graceful degradation is the
/// Consumer's to express, as `Ok(ResolvedIntrinsic { disposition:
/// ResolveDisposition::Fallback { .. }, .. })`. This pass therefore treats
/// a `Fallback` disposition exactly like `Ok` — it still uses the returned
/// `intrinsic` — and never substitutes a fallback of its own by swallowing
/// an `Err`.
pub(crate) fn resolve_images(
    document: &mut Document,
    resolver: &dyn ReplacedResolver,
) -> Result<(), ResolverError> {
    resolve_images_with_base(document, resolver, None)
}

/// Resolve replaced-element URLs against the document base when they are not
/// already absolute.
pub(crate) fn resolve_images_with_base(
    document: &mut Document,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&Url>,
) -> Result<(), ResolverError> {
    let mut intrinsic_size_changed = false;
    let result = (|| {
        for node in document.nodes.iter_mut() {
            if !node.is_in_document() {
                continue;
            }
            let NodeData::Element(element) = &mut node.data else {
                continue;
            };
            if element.tag_name.as_str() != "img" {
                continue;
            }
            let previous_size = element.image_intrinsic_size;
            let src = element
                .attributes
                .iter()
                .find(|attribute| {
                    attribute.namespace.is_none() && attribute.local.as_str() == "src"
                })
                .map(|attribute| attribute.value.as_str());
            let next_size = if let Some(src) = src {
                let url = Url::parse(src)
                    .ok()
                    .or_else(|| base_url.and_then(|base| base.join(src).ok()));
                if let Some(url) = url {
                    match resolver.resolve(ResolverRequest::new(&url)) {
                        Ok(resolved) => Some(resolved.intrinsic),
                        Err(error) => {
                            if previous_size.is_some() {
                                element.image_intrinsic_size = None;
                                intrinsic_size_changed = true;
                            }
                            return Err(error);
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if previous_size != next_size {
                element.image_intrinsic_size = next_size;
                intrinsic_size_changed = true;
            }
        }
        Ok(())
    })();
    if intrinsic_size_changed {
        document.invalidate_layout_cache();
    }
    result
}

/// Populate CSS-defaulted layout sizes for outermost inline SVG roots.
///
/// This reads only the root's namespace-qualified DOM metadata; SVG parsing and
/// rasterization remain in `raikiri-svg` at paint time. Missing or unsupported
/// natural dimensions use the CSS default object size so a parse failure can
/// still reach paint and be reported there.
pub(crate) fn resolve_inline_svg_intrinsic_sizes(document: &mut Document) {
    let mut changed = false;
    for node in &mut document.nodes {
        let is_inline_svg_root = node.is_inline_svg_root();
        let NodeData::Element(element) = &mut node.data else {
            continue;
        };
        let next_size = if is_inline_svg_root {
            let attr = |name: &str| {
                element
                    .attributes
                    .iter()
                    .find(|attribute| {
                        attribute.namespace.is_none() && attribute.local.as_str() == name
                    })
                    .map(|attribute| attribute.value.as_str())
            };
            let width = attr("width").and_then(parse_absolute_svg_length);
            let height = attr("height").and_then(parse_absolute_svg_length);
            let ratio = match (width, height) {
                (Some(width), Some(height)) => positive_svg_ratio(width, height),
                _ => attr("viewBox").and_then(parse_svg_view_box_ratio),
            };
            let (default_width, default_height) = default_svg_object_size(width, height, ratio);
            let mut intrinsic = raikiri_traits::IntrinsicBox::new(default_width, default_height);
            intrinsic.aspect_ratio = ratio;
            Some(intrinsic)
        } else if element.tag_name.as_str() != "img" {
            None
        } else {
            element.image_intrinsic_size
        };
        if element.image_intrinsic_size != next_size {
            element.image_intrinsic_size = next_size;
            changed = true;
        }
    }
    if changed {
        document.invalidate_layout_cache();
    }
}

fn parse_absolute_svg_length(value: &str) -> Option<f32> {
    let value = value.trim();
    let units = [
        ("px", 1.0_f64),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("q", 96.0 / 101.6),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
    ];
    let lower = value.to_ascii_lowercase();
    let (number, factor) = units
        .iter()
        .find_map(|(unit, factor)| lower.strip_suffix(unit).map(|number| (number, *factor)))
        .unwrap_or((value, 1.0));
    let value = number.trim().parse::<f64>().ok()? * factor;
    (value.is_finite() && value > 0.0 && value <= f32::MAX as f64).then_some(value as f32)
}

fn parse_svg_view_box_ratio(value: &str) -> Option<f32> {
    let mut values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>);
    let _x = values.next()?.ok()?;
    let _y = values.next()?.ok()?;
    let width = values.next()?.ok()?;
    let height = values.next()?.ok()?;
    if values.next().is_some() {
        return None;
    }
    positive_svg_ratio(width, height)
}

fn positive_svg_ratio(width: f32, height: f32) -> Option<f32> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let ratio = width / height;
    (ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

fn default_svg_object_size(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
) -> (f32, f32) {
    const DEFAULT_WIDTH: f32 = 300.0;
    const DEFAULT_HEIGHT: f32 = 150.0;
    match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (None, None, Some(ratio)) => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            (width, width / ratio)
        }
        (Some(width), None, _) => (width, DEFAULT_HEIGHT),
        (None, Some(height), _) => (DEFAULT_WIDTH, height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    }
}

#[cfg(test)]
mod tests;
