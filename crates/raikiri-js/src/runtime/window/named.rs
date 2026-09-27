use super::super::{
    collections::{CollectionSource, html_collection},
    interfaces::wrap,
    webidl::with_state,
};
use boa_engine::object::builtins::JsProxyBuilder;
use boa_engine::{Context, JsObject, JsResult, JsValue};
use raikiri_dom::Document;

pub(crate) fn nodes(document: &Document, name: &str) -> Vec<usize> {
    if name.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut stack = vec![document.root_index()];
    while let Some(id) = stack.pop() {
        let Some(node) = document.get_node(id) else {
            continue;
        };
        stack.extend(node.children.iter().rev().copied());
        if document.element_namespace_uri(id) != Some(super::super::interfaces::HTML_NS) {
            continue;
        }
        if node.attribute("id") == Some(name)
            || (matches!(node.tag_name(), Some("embed" | "form" | "img" | "object"))
                && node.attribute("name") == Some(name))
        {
            found.push(id);
        }
    }
    found
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
    let found = with_state(context, |s| nodes(s.host.document(), &name))?;
    match found.len() {
        0 => target.get(key, context),
        1 => Ok(wrap(context, found[0])?.into()),
        _ => Ok(html_collection(context, CollectionSource::WindowNamed(name))?.into()),
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
    Ok(with_state(context, |s| !nodes(s.host.document(), &name).is_empty())?.into())
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
