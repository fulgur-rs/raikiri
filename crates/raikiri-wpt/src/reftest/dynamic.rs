use super::*;
use raikiri_style::{StyleDom, StyleElement, StyleNode};

/// A live reftest document after script execution: serialized HTML plus
/// canvas bitmaps in tree order.
///
/// HTML serialization drops canvas bitmaps (a bitmap is not part of
/// `innerHTML`), so the live document's
/// [`raikiri_dom::Document::take_canvases_in_tree_order`] sidecar travels
/// alongside the markup and the paint path restores it with
/// [`raikiri_dom::Document::set_canvases_in_tree_order_owned`] before paint.
#[derive(Debug)]
pub(super) struct PreparedDynamic {
    /// Serialized document after scripts ran (or the original source when no
    /// scripts were present).
    pub html: String,
    /// Canvas bitmaps in tree order, empty when the document has no canvas.
    pub canvases: Vec<raikiri_dom::CanvasBitmap>,
    /// Animation-origin declarations that are not represented in serialized HTML.
    pub animation_styles: Vec<AnimationStyleSidecar>,
    /// Script-created highlight ranges expressed against the serialized tree.
    pub custom_highlight_ranges: Vec<CustomHighlightPathRange>,
}

fn animation_style_sidecars(document: &raikiri_dom::Document) -> Vec<AnimationStyleSidecar> {
    let mut styles = Vec::new();
    let mut stack = vec![(document.root_id(), Vec::new())];
    while let Some((node_id, path)) = stack.pop() {
        let animation_style = document.node(node_id).and_then(|node| {
            node.as_element()
                .and_then(|element| element.animation_style_source().map(str::to_owned))
        });
        if let Some(declarations) = animation_style {
            styles.push(AnimationStyleSidecar {
                node_path: path.clone(),
                declarations,
            });
        }

        let children = document
            .child_ids(node_id)
            .filter(|child_id| {
                document
                    .node(*child_id)
                    .is_some_and(|child| child.as_element().is_some())
            })
            .collect::<Vec<_>>();
        for (index, child_id) in children.into_iter().enumerate().rev() {
            let mut child_path = path.clone();
            child_path.push(index);
            stack.push((child_id, child_path));
        }
    }
    styles
}

fn node_path(document: &raikiri_dom::Document, node: usize) -> Option<Vec<usize>> {
    let root = document.root_index();
    let mut path = Vec::new();
    let mut current = node;
    while current != root {
        let parent = document.parent_of(current)?;
        let child_index = document
            .get_node(parent)?
            .children
            .iter()
            .position(|child| *child == current)?;
        path.push(child_index);
        current = parent;
    }
    path.reverse();
    Some(path)
}

fn utf16_offset_to_byte(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += character.len_utf16();
    }
    (units == offset).then_some(text.len())
}

fn custom_highlight_paths(
    document: &raikiri_dom::Document,
    ranges: &[raikiri_js::runtime::CustomHighlightRange],
) -> Vec<CustomHighlightPathRange> {
    ranges
        .iter()
        .filter_map(|range| {
            if range.start_container != range.end_container {
                return None;
            }
            let text = document.get_node(range.start_container)?.text_content()?;
            let start_byte = utf16_offset_to_byte(text, range.start_offset)?;
            let end_byte = utf16_offset_to_byte(text, range.end_offset)?;
            if start_byte >= end_byte {
                return None;
            }
            Some(CustomHighlightPathRange {
                name: range.name.clone(),
                node_path: node_path(document, range.start_container)?,
                start_byte,
                end_byte,
            })
        })
        .collect()
}

fn waiting(document: &raikiri_dom::Document) -> bool {
    let mut stack = vec![document.root_index()];
    while let Some(id) = stack.pop() {
        let Some(node) = document.get_node(id) else {
            continue; // cov:ignore: Document arena only appends and children always hold valid indices, so traversal from root never misses.
        };
        if node.tag_name() == Some("html") {
            return node.attribute("class").is_some_and(|classes| {
                classes
                    .split_ascii_whitespace()
                    .any(|class| class == "reftest-wait")
            });
        }
        stack.extend(node.children.iter().rev().copied());
    }
    false
}

/// Whether the parsed document contains any HTML-namespace `<script>`
/// element. Used as the fast-path gate: static documents without scripts
/// never enter the JS runtime, preserving the historical static behavior
/// exactly (no serialization round-trip, no URL requirement).
fn has_script_elements(document: &raikiri_dom::Document) -> bool {
    let mut stack = vec![document.root_index()];
    while let Some(id) = stack.pop() {
        let Some(node) = document.get_node(id) else {
            continue; // cov:ignore: Document arena only appends and children always hold valid indices, so traversal from root never misses.
        };
        if node.tag_name() == Some("script")
            && document.element_namespace_uri(id) == Some("http://www.w3.org/1999/xhtml")
        {
            return true;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    false
}

fn serialize(
    document: &raikiri_dom::Document,
    report: &raikiri_js::runtime::RunReport,
) -> Result<String, ReftestError> {
    if report.aborted.is_some()
        || !report.host_failures.is_empty()
        || !report.fetch_errors.is_empty()
        || !report.uncaught_errors.is_empty()
    {
        return Err(ReftestError::RaikiriRender(format!(
            "dynamic reftest script failed: {report:?}"
        )));
    }
    if waiting(document) {
        return Err(ReftestError::RaikiriRender(
            "dynamic reftest still has reftest-wait after event loop drained".into(),
        ));
    }
    let markup = document
        .serialize_inner_html(document.root_index())
        .map_err(ReftestError::RaikiriRender)?;
    let doctype = match document.quirks_mode() {
        raikiri_dom::QuirksMode::NoQuirks => "<!DOCTYPE html>",
        raikiri_dom::QuirksMode::LimitedQuirks => {
            "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\" \"http://www.w3.org/TR/html4/loose.dtd\">"
        }
        raikiri_dom::QuirksMode::Quirks => "",
        // cov:ignore: QuirksMode is non_exhaustive with only NoQuirks, LimitedQuirks, and Quirks handled above; this arm is for future variants.
        _ => {
            return Err(ReftestError::RaikiriRender(
                "unsupported dynamic document mode".into(),
            ));
        }
    };
    Ok(format!("{doctype}{markup}"))
}

pub(super) fn prepare(
    html: &str,
    path: &Path,
    suffix: &str,
    config: ReftestConfig,
) -> Result<PreparedDynamic, ReftestError> {
    let parsed = raikiri_html::parse(
        html.as_bytes(),
        &raikiri::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .map_err(|e| ReftestError::RaikiriRender(format!("parse readiness: {e:?}")))?;
    // Static fast path: no scripts and no reftest-wait means no JS, no
    // canvas, no URL needed. Returns the original source verbatim so static
    // reftests never see a serialization round-trip. A reftest-wait document
    // without scripts still enters the live path so the unreleased wait
    // fails closed instead of silently passing.
    if !has_script_elements(&parsed.dom) && !waiting(&parsed.dom) {
        return Ok(PreparedDynamic {
            html: html.to_owned(),
            canvases: Vec::new(),
            animation_styles: Vec::new(),
            custom_highlight_ranges: Vec::new(),
        });
    }
    let parent = path
        .parent()
        .ok_or_else(|| ReftestError::RaikiriRender("dynamic reftest has no directory".into()))?;
    let wpt_root = parent
        .ancestors()
        .find(|p| p.join("resources/testharness.js").is_file())
        .unwrap_or(parent);
    let setup = prepare_wpt_live_document(html, config.width, config.height, parent, wpt_root)
        .map_err(ReftestError::RaikiriRender)?;
    let url = raikiri::Url::from_file_path(path).map_err(|_| {
        ReftestError::RaikiriRender("dynamic reftest needs an absolute path".into())
    })?;
    let url = url
        .join(suffix)
        .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
    let host = crate::wpt_host::WptDocumentHost::new(setup, wpt_root).with_page_url(url);
    // spec: https://web-platform-tests.org/writing-tests/reftests.html#controlling-when-the-screenshot-is-taken
    let preamble = "window.addEventListener('load', function() { requestAnimationFrame(function() { requestAnimationFrame(function() { if (document.documentElement.classList.contains('reftest-wait')) document.documentElement.dispatchEvent(new Event('TestRendered', {bubbles:true})); }); }); });";
    #[cfg(feature = "js-native")]
    {
        let mut runtime = raikiri_js::runtime::DomRuntime::new(host)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        raikiri_js_wasmtime_harness::install_page_support(&mut runtime)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        runtime
            .evaluate(preamble)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        let report = runtime.run_document_with_callback(
            raikiri_js_wasmtime_harness::SINK_SYMBOL_DESCRIPTION,
            raikiri_js_wasmtime_harness::deliver,
        );
        let highlights = runtime
            .custom_highlight_ranges()
            .map_err(|error| ReftestError::RaikiriRender(error.to_string()))?;
        let mut host = runtime.into_host();
        let custom_highlight_ranges = custom_highlight_paths(host.document(), &highlights);
        let animation_styles = animation_style_sidecars(host.document());
        let html = serialize(host.document(), &report)?;
        let canvases = host.document_mut().take_canvases_in_tree_order();
        Ok(PreparedDynamic {
            html,
            canvases,
            animation_styles,
            custom_highlight_ranges,
        })
    }
    #[cfg(feature = "js-wasmtime")]
    {
        let mut runtime = raikiri_js_wasmtime_host::WasmtimePage::new(
            Box::new(host),
            Default::default(),
            Default::default(),
        )
        .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        runtime
            .evaluate_preamble(preamble)
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        let report = runtime
            .run_document()
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?
            .into_native();
        runtime
            .synchronize_final_document()
            .map_err(|e| ReftestError::RaikiriRender(e.to_string()))?;
        let animation_styles = animation_style_sidecars(runtime.document());
        let html = serialize(runtime.document(), &report)?;
        let canvases = runtime.take_canvases_in_tree_order();
        return Ok(PreparedDynamic {
            html,
            canvases,
            animation_styles,
            custom_highlight_ranges: Vec::new(),
        });
    }
    #[cfg(not(any(feature = "js-native", feature = "js-wasmtime")))]
    {
        return Err(ReftestError::RaikiriRender(
            "dynamic reftest needs a JS backend".into(),
        ));
    }
}

#[cfg(test)]
mod tests;
