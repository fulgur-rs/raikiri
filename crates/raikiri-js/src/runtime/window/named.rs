use super::super::{
    collections::{CollectionSource, html_collection},
    interfaces::wrap,
    webidl::with_state,
};
use boa_engine::object::builtins::JsProxyBuilder;
use boa_engine::{Context, JsObject, JsResult, JsValue};
use raikiri_dom::Document;

const MAX_CACHE_ENTRIES: usize = 128;
const MAX_CACHE_KEY_BYTES: usize = 16 * 1024;
const MAX_SINGLE_KEY_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Matches {
    Missing,
    Single(usize),
    Multiple,
}

#[derive(Clone, Copy)]
enum Cached {
    Present,
    Classified(Matches),
}

/// Bounded summaries, never wrappers or complete lists of matching nodes.
#[derive(Default)]
pub(crate) struct NamedAccessCache {
    version: Option<raikiri_dom::document::NamedElementVersion>,
    entries: std::collections::HashMap<String, Cached>,
    key_bytes: usize,
}

impl NamedAccessCache {
    fn prepare(&mut self, document: &Document) {
        if self.version.as_ref() != Some(document.named_element_version()) {
            self.entries.clear();
            self.key_bytes = 0;
            self.version = Some(document.named_element_version().clone());
        }
    }

    fn remember(&mut self, name: &str, result: Cached) {
        if name.len() > MAX_SINGLE_KEY_BYTES {
            return;
        }
        if let Some(existing) = self.entries.get_mut(name) {
            *existing = result;
            return;
        }
        if self.entries.len() == MAX_CACHE_ENTRIES
            || self.key_bytes + name.len() > MAX_CACHE_KEY_BYTES
        {
            self.entries.clear();
            self.key_bytes = 0;
        }
        self.key_bytes += name.len();
        self.entries.insert(name.to_owned(), result);
    }

    fn get(&mut self, document: &Document, name: &str) -> Matches {
        if name.is_empty() {
            return Matches::Missing;
        }
        self.prepare(document);
        if let Some(Cached::Classified(result)) = self.entries.get(name) {
            return *result;
        }
        let mut matches = matching_nodes(document, name);
        let result = match matches.next() {
            None => Matches::Missing,
            Some(first) => {
                if matches.next().is_some() {
                    Matches::Multiple
                } else {
                    Matches::Single(first)
                }
            }
        };
        self.remember(name, Cached::Classified(result));
        result
    }

    fn has(&mut self, document: &Document, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        self.prepare(document);
        if let Some(result) = self.entries.get(name) {
            return !matches!(result, Cached::Classified(Matches::Missing));
        }
        let found = matching_nodes(document, name).next().is_some();
        self.remember(
            name,
            if found {
                Cached::Present
            } else {
                Cached::Classified(Matches::Missing)
            },
        );
        found
    }
}

fn matching_nodes<'a>(document: &'a Document, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let mut root = (!name.is_empty()).then_some(document.root_index());
    let mut stack: Vec<std::slice::Iter<'a, usize>> = Vec::new();
    std::iter::from_fn(move || {
        loop {
            let id = if let Some(root) = root.take() {
                root
            } else {
                loop {
                    let children = stack.last_mut()?;
                    if let Some(&child) = children.next() {
                        break child;
                    }
                    stack.pop();
                }
            };
            let Some(node) = document.get_node(id) else {
                continue; // cov:ignore: Source children always hold valid arena indices.
            };
            // Keep one iterator per ancestor rather than one ID per sibling.
            if !node.children.is_empty() {
                stack.push(node.children.iter());
            }
            #[cfg(test)]
            tests::record_visit(stack.len());
            if document.element_namespace_uri(id) == Some(super::super::interfaces::HTML_NS)
                && (node.attribute("id") == Some(name)
                    || (matches!(node.tag_name(), Some("embed" | "form" | "img" | "object"))
                        && node.attribute("name") == Some(name)))
            {
                return Some(id);
            }
        }
    })
}

pub(crate) fn nodes(document: &Document, name: &str) -> Vec<usize> {
    matching_nodes(document, name).collect()
}

fn target_key(
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<(JsObject, boa_engine::property::PropertyKey)> {
    let target = args[0].as_object().expect("proxy supplies target");
    let key = args[1].to_property_key(context)?;
    Ok((target, key))
}

fn name(key: &boa_engine::property::PropertyKey) -> Option<String> {
    match key {
        boa_engine::property::PropertyKey::String(s) => Some(s.to_std_string_escaped()),
        boa_engine::property::PropertyKey::Index(i) => Some(i.get().to_string()),
        _ => None,
    }
}

fn get(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (target, key) = target_key(args, context)?;
    let Some(name) = name(&key) else {
        return target.get(key, context);
    };
    // spec: https://html.spec.whatwg.org/multipage/nav-history-apis.html#named-access-on-the-window-object
    let found = with_state(context, |s| {
        s.window_named_cache.get(s.host.document(), &name)
    })?;
    match found {
        Matches::Missing => target.get(key, context),
        Matches::Single(node) => Ok(wrap(context, node)?.into()),
        Matches::Multiple => {
            Ok(html_collection(context, CollectionSource::WindowNamed(name))?.into())
        }
    }
}

fn has(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (target, key) = target_key(args, context)?;
    if target.has_property(key.clone(), context)? {
        return Ok(true.into());
    }
    let Some(name) = name(&key) else {
        return Ok(false.into());
    };
    Ok(with_state(context, |s| {
        s.window_named_cache.has(s.host.document(), &name)
    })?
    .into())
}

pub(super) fn install(context: &mut Context) -> JsResult<()> {
    let global = context.global_object();
    let target = JsObject::from_proto_and_data(global.prototype(), ());
    let proxy = JsProxyBuilder::new(target)
        .get(get)
        .has(has)
        .build(context)?;
    if !global.set_prototype(Some(proxy.into())) {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("cannot install Window named properties")
            .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
