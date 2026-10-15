//! `HTMLImageElement`, the `Image` constructor, and the `load`/`error`
//! events an `<img>` fires once its `src` has been fetched.
//!
//! "Update the image data" (HTML §4.8.4.3.5) runs whenever an `<img>`
//! gets a new `src`, and once for every `<img>` already in the parsed
//! document: it queues a task that asks the host to load the resolved URL
//! and fires `load` or `error` at the element. A later `src` change makes an
//! earlier queued request stale, so only the latest one fires. `srcset`,
//! `<picture>`, lazy loading, and `decode()` are not modelled.
//!
//! Spec: <https://html.spec.whatwg.org/multipage/images.html#update-the-image-data>

use boa_engine::job::NativeJob;
use boa_engine::{Context, JsNativeError, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::dispatch;
use super::event_loop;
use super::interfaces::{HTML_NS, Members, wrap};
use super::node::{js_str, mark_dirty, write_attribute};
use super::webidl::{dom_string, guard_node_budget, this_element, with_state};

/// Whether `index` is an HTML `<img>` element.
pub(crate) fn is_html_image(document: &raikiri_dom::Document, index: usize) -> bool {
    document.get_node(index).is_some_and(|node| {
        node.kind() == NodeKind::Element
            && node.tag_name() == Some("img")
            && document.element_namespace_uri(index) == Some(HTML_NS)
    })
}

/// Parse `src` relative to the document base URL (HTML "encoding-parse a
/// URL"), the one `<base href>` sets and layout resolves images against,
/// else the document URL; `None` when it does not parse, which HTML treats
/// as a failed request.
fn resolve_source(context: &mut Context, src: &str) -> JsResult<Option<String>> {
    let base = with_state(context, |s| {
        s.host.document().base_url().cloned().or_else(|| {
            s.host
                .document_url()
                .and_then(|url| url::Url::parse(&url).ok())
        })
    })?;
    Ok(url::Url::options()
        .base_url(base.as_ref())
        .parse(src)
        .ok()
        .map(String::from))
}

/// Whether `index` sits inside some `<template>`'s contents, whose inert
/// document never loads images.
fn in_template_contents(document: &raikiri_dom::Document, index: usize) -> bool {
    let mut root = index;
    while let Some(parent) = document.parent_of(root) {
        root = parent;
    }
    document
        .get_node(root)
        .is_some_and(|node| node.kind() == NodeKind::DocumentFragment)
        && (0..document.node_count()).any(|owner| {
            document
                .get_node(owner)
                .is_some_and(|node| node.template_contents() == Some(root))
        })
}

/// HTML "update the image data" for `index`, after its `src` was set or
/// removed, or for an `<img>` the parser created. Starts a new request,
/// which makes any earlier pending one stale; with no `src` there is no
/// request and nothing fires.
pub(crate) fn update_image_data(context: &mut Context, index: usize) -> JsResult<()> {
    let src = with_state(context, |s| {
        let document = s.host.document();
        if in_template_contents(document, index) {
            return None;
        }
        document.element_attribute(index, "src").map(str::to_owned)
    })?;
    let request = with_state(context, |s| {
        s.image_requests.remove(&index);
        src.as_ref().map(|_| {
            s.next_image_request = s.next_image_request.wrapping_add(1);
            s.image_requests.insert(index, s.next_image_request);
            s.next_image_request
        })
    })?;
    let (Some(src), Some(request)) = (src, request) else {
        return Ok(());
    };
    let url = if src.is_empty() {
        None
    } else {
        resolve_source(context, &src)?
    };
    let job = NativeJob::new(move |context| {
        finish_request(context, index, request, url.as_deref())?;
        Ok(JsValue::undefined())
    });
    event_loop::queue_native_task(context, job)
}

/// The queued half of [`update_image_data`]: skip a stale request, else ask
/// the host to load `url` and fire `load` or `error` at the element.
fn finish_request(
    context: &mut Context,
    index: usize,
    request: u64,
    url: Option<&str>,
) -> JsResult<()> {
    let current = with_state(context, |s| s.image_requests.get(&index) == Some(&request))?;
    if !current {
        return Ok(());
    }
    let loaded = match url {
        Some(url) => with_state(context, |s| s.host.load_image(url).is_ok())?,
        None => false,
    };
    // Layout resolves the image on its own, so nothing here dirties it.
    with_state(context, |s| s.image_requests.remove(&index))?;
    let kind = if loaded { "load" } else { "error" };
    dispatch::fire_event(context, Some(index), kind, false, false)?;
    Ok(())
}

/// Start a request for every `<img>` with a `src` already in the document,
/// as the parser does when it inserts them.
pub(crate) fn update_parsed_images(context: &mut Context) -> JsResult<()> {
    let root = with_state(context, |s| s.host.document().root_index())?;
    update_images_below(context, root)
}

/// Start a request for every `<img>` below `root` (not `root` itself), in
/// tree order: the elements a fragment parse just created there. Template
/// contents are not children, so their inert images never load.
pub(crate) fn update_images_below(context: &mut Context, root: usize) -> JsResult<()> {
    let images = with_state(context, |s| {
        let document = s.host.document();
        let mut images = Vec::new();
        let mut stack = vec![root];
        while let Some(index) = stack.pop() {
            if index != root && is_html_image(document, index) {
                images.push(index);
            }
            if let Some(node) = document.get_node(index) {
                stack.extend(node.children.iter().rev());
            }
        }
        images
    })?;
    for index in images {
        update_image_data(context, index)?;
    }
    Ok(())
}

fn this_image(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_element(this, context)?;
    if with_state(context, |s| is_html_image(s.host.document(), index))? {
        Ok(index)
    } else {
        Err(JsNativeError::typ()
            .with_message("'this' is not an HTMLImageElement")
            .into())
    }
}

/// `src` getter: the attribute resolved against the document URL (a
/// reflected `USVString` URL attribute); the raw value when it does not
/// resolve, and the empty string when it is absent.
fn src(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_image(this, context)?;
    let raw = with_state(context, |s| {
        s.host
            .document()
            .element_attribute(index, "src")
            .map(str::to_owned)
    })?;
    let Some(raw) = raw else {
        return Ok(js_str(""));
    };
    let resolved = resolve_source(context, &raw)?;
    Ok(js_str(resolved.as_deref().unwrap_or(&raw)))
}

fn set_src(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_image(this, context)?;
    let value = dom_string(args, 0, context)?;
    write_attribute(context, index, "src", &value)?;
    Ok(JsValue::undefined())
}

/// `complete`: no request is pending, or the pending one has nothing to
/// fetch because `src` is empty (HTML counts both as complete).
fn complete(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_image(this, context)?;
    let fetching = with_state(context, |s| {
        s.image_requests.contains_key(&index)
            && s.host
                .document()
                .element_attribute(index, "src")
                .is_some_and(|src| !src.is_empty())
    })?;
    Ok(JsValue::from(!fetching))
}

pub(crate) const HTML_IMAGE_ELEMENT_MEMBERS: Members = Members {
    getters: &[("complete", complete)],
    accessors: &[("src", src, set_src)],
    methods: &[],
};

/// `new Image(width, height)` (HTML §4.8.4.1 legacy factory function): a
/// new `<img>`, with its `width` and `height` attributes set from the
/// arguments that were given.
pub(crate) fn image_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if this.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("Constructor Image requires 'new'")
            .into());
    }
    let mut sizes = Vec::new();
    for (name, value) in ["width", "height"].into_iter().zip(args) {
        if !value.is_undefined() {
            // WebIDL `unsigned long`, then the attribute's string form.
            sizes.push((name, value.to_u32(context)?));
        }
    }
    guard_node_budget(context)?;
    let index = with_state(context, |s| {
        s.host.document_mut().create_detached_element("img")
    })?
    // cov:ignore: "img" is always a valid element name.
    .map_err(|_| super::webidl::unreachable_mutation_error())?;
    for (name, value) in sizes {
        write_attribute(context, index, name, &value.to_string())?;
    }
    mark_dirty(context)?;
    Ok(wrap(context, index)?.into())
}

#[cfg(test)]
mod tests;
