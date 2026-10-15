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
use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};
use raikiri_dom::NodeKind;

use super::dispatch;
use super::event_loop;
use super::interfaces::{HTML_NS, Members, wrap};
use super::node::{js_str, mark_dirty, write_attribute};
use super::webidl::{guard_node_budget, this_element, usv_string, with_state};

/// Whether `index` is an HTML `<img>` element.
pub(crate) fn is_html_image(document: &raikiri_dom::Document, index: usize) -> bool {
    document.get_node(index).is_some_and(|node| {
        node.kind() == NodeKind::Element
            && node.tag_name() == Some("img")
            && document.element_namespace_uri(index) == Some(HTML_NS)
    })
}

/// Parse `src` relative to the document base URL (HTML "encoding-parse a
/// URL"); `None` when it does not parse, which HTML treats as a failed
/// request.
fn resolve_source(context: &mut Context, src: &str) -> JsResult<Option<String>> {
    // Computed from the live tree so a script-changed `<base href>` is seen;
    // a host with no document URL falls back to the base the parser recorded.
    let base = with_state(context, |s| {
        let document = s.host.document();
        match s
            .host
            .document_url()
            .and_then(|url| url::Url::parse(&url).ok())
        {
            Some(url) => document.live_base_url(Some(&url)),
            None => document.base_url().cloned(),
        }
    })?;
    Ok(url::Url::options()
        .base_url(base.as_ref())
        .parse(src)
        .ok()
        .map(String::from))
}

/// The root of the tree `index` is in.
fn tree_root(document: &raikiri_dom::Document, index: usize) -> usize {
    let mut root = index;
    while let Some(parent) = document.parent_of(root) {
        root = parent;
    }
    root
}

/// Whether `index` sits inside some `<template>`'s contents, whose inert
/// document never loads images.
fn in_template_contents(document: &raikiri_dom::Document, index: usize) -> bool {
    let root = tree_root(document, index);
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
/// request and nothing fires. An image in template contents is only
/// remembered: [`settle_moved_images`] starts its request once it is moved
/// out (HTML runs these steps again when an `<img>` is adopted).
pub(crate) fn update_image_data(context: &mut Context, index: usize) -> JsResult<()> {
    let src = with_state(context, |s| {
        let document = s.host.document();
        // A remembered image removed from template contents still belongs
        // to their inert document until it is inserted into this one.
        let adopted =
            !s.inert_images.contains(&index) || tree_root(document, index) == document.root_index();
        if !adopted || in_template_contents(document, index) {
            s.active_images.remove(&index);
            s.inert_images.insert(index);
            return None;
        }
        s.inert_images.remove(&index);
        s.active_images.insert(index);
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
    // Layout renders the resource selected here, even if `<base href>`
    // changes before it runs.
    if let Some(selected) = url.as_deref().and_then(|url| url::Url::parse(url).ok()) {
        with_state(context, |s| {
            s.host
                .document_mut()
                .select_image_url(index, &src, selected);
        })?;
    }
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
/// tree order: the elements a fragment parse just created there. Images in
/// template contents are only remembered, as [`update_image_data`] says.
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
                stack.extend(node.template_contents());
            }
        }
        images
    })?;
    for index in images {
        update_image_data(context, index)?;
    }
    Ok(())
}

/// Whether `index` belongs to an inert template document: it sits in
/// template contents, or is a remembered image not yet adopted out of them.
pub(crate) fn is_inert(context: &mut Context, index: usize) -> JsResult<bool> {
    with_state(context, |s| {
        s.inert_images.contains(&index) || in_template_contents(s.host.document(), index)
    })
}

/// Remember every `<img>` at or below `root` as inert, without a request:
/// the images of a detached copy that still belongs to an inert template
/// document. [`settle_moved_images`] starts them once they are inserted.
pub(crate) fn remember_inert_images(context: &mut Context, root: usize) -> JsResult<()> {
    with_state(context, |s| {
        let mut stack = vec![root];
        while let Some(index) = stack.pop() {
            let document = s.host.document();
            if is_html_image(document, index) {
                s.inert_images.insert(index);
            }
            if let Some(node) = s.host.document().get_node(index) {
                stack.extend(node.children.iter().copied());
                stack.extend(node.template_contents());
            }
        }
    })
}

/// After a tree mutation, make each image that was moved into template
/// contents (adopting it into their inert document) inert again, cancelling
/// any request it still had pending, then update the image data of each
/// remembered inert image that is now in this document's tree. Merely
/// removing one from template contents does not adopt it, so a detached one
/// keeps waiting.
pub(crate) fn settle_moved_images(context: &mut Context) -> JsResult<()> {
    let adopted = with_state(context, |s| {
        let document = s.host.document();
        let moved_in = s
            .active_images
            .iter()
            .copied()
            .filter(|&index| in_template_contents(document, index))
            .collect::<Vec<_>>();
        for index in moved_in {
            s.active_images.remove(&index);
            s.image_requests.remove(&index);
            s.inert_images.insert(index);
        }
        let root = document.root_index();
        s.inert_images
            .iter()
            .copied()
            .filter(|&index| tree_root(document, index) == root)
            .collect::<Vec<_>>()
    })?;
    for index in adopted {
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
    // `src` is a `USVString` attribute.
    let value = usv_string(args, 0, context)?;
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
    new_target: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let new_target = new_target
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Constructor Image requires 'new'"))?;
    // WebIDL legacy factory functions build the object from NewTarget's
    // `prototype`, falling back to the interface prototype when it is not
    // an object, so subclasses of `Image` keep their own prototype.
    let prototype = new_target
        .get(js_string!("prototype"), context)?
        .as_object();
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
    let wrapper = wrap(context, index)?;
    if let Some(prototype) = prototype {
        wrapper.set_prototype(Some(prototype));
    }
    Ok(wrapper.into())
}

#[cfg(test)]
mod tests;
