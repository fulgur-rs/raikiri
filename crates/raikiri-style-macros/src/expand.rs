//! `#[longhands] mod decl { .. }`: finds the hand-written enums and the
//! `properties!` blocks, and assembles the expanded module.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens as _, quote, quote_spanned};
use syn::spanned::Spanned as _;
use syn::{Expr, Fields, Ident, Item, ItemEnum, ItemMod};

use crate::diag::Errors;
use crate::{generate, model, parse};

/// How a hand-written `PropertyValue` variant maps to its `PropertyKey`.
enum KeyRule {
    /// No `#[key]`: the key variant of the same name.
    Same,
    /// `#[key(Other)]`: another key variant.
    Other(Ident),
    /// `#[key(with = f)]`: `f(&payload)` for a variant with one field.
    With(Box<Expr>),
    /// A malformed `#[key]` (already reported).
    Invalid,
}

const KEY_ATTR_HELP: &str = "expected `#[key(OtherKey)]` or `#[key(with = <fn(&Payload) -> PropertyKey>)]`";

fn parse_key_attr(attr: &syn::Attribute) -> syn::Result<KeyRule> {
    let syn::Meta::List(list) = &attr.meta else {
        return Err(syn::Error::new_spanned(attr, KEY_ATTR_HELP));
    };
    list.parse_args_with(|input: syn::parse::ParseStream| {
        if input.peek(Ident) && input.peek2(syn::Token![=]) {
            let name: Ident = input.parse()?;
            if name != "with" {
                return Err(syn::Error::new(name.span(), KEY_ATTR_HELP));
            }
            input.parse::<syn::Token![=]>()?;
            let expr: Expr = input.parse()?;
            return Ok(KeyRule::With(Box::new(expr)));
        }
        if input.peek(syn::Token![|]) {
            return Err(input.error(
                "write a closure as `#[key(with = |payload| ..)]`; it receives `&Payload`",
            ));
        }
        let key: Ident = input
            .parse()
            .map_err(|_| input.error(KEY_ATTR_HELP))?;
        if !input.is_empty() {
            return Err(input.error(KEY_ATTR_HELP));
        }
        Ok(KeyRule::Other(key))
    })
}

/// Removes the `#[key(..)]` helper attributes from `PropertyValue`'s
/// variants and returns one `key()` match arm per variant.
fn take_key_arms(value_enum: &mut ItemEnum, errors: &mut Errors) -> Vec<TokenStream> {
    let mut arms = Vec::with_capacity(value_enum.variants.len());
    for variant in &mut value_enum.variants {
        let mut rule = KeyRule::Same;
        let mut seen = false;
        variant.attrs.retain(|attr| {
            if !attr.path().is_ident("key") {
                return true;
            }
            if seen {
                errors.push(syn::Error::new_spanned(
                    attr,
                    "a variant takes at most one `#[key(..)]`",
                ));
                rule = KeyRule::Invalid;
            } else {
                rule = parse_key_attr(attr).unwrap_or_else(|error| {
                    errors.push(error);
                    KeyRule::Invalid
                });
            }
            seen = true;
            false
        });
        let v = &variant.ident;
        let arm = match rule {
            KeyRule::Same => {
                quote_spanned!(v.span()=> PropertyValue::#v { .. } => PropertyKey::#v,)
            }
            KeyRule::Other(key) => {
                quote_spanned!(key.span()=> PropertyValue::#v { .. } => PropertyKey::#key,)
            }
            KeyRule::With(expr) => match single_field_type(&variant.fields) {
                Some(ty) => {
                    let payload = Ident::new("payload", Span::mixed_site());
                    let f = Ident::new("key_of", Span::mixed_site());
                    let call = quote_spanned! {expr.span()=>
                        let #f: fn(&#ty) -> PropertyKey = #expr;
                        #f(#payload)
                    };
                    quote!(PropertyValue::#v(#payload) => { #call })
                }
                None => {
                    errors.push(syn::Error::new_spanned(
                        &expr,
                        "`#[key(with = ..)]` needs a variant with exactly one unnamed field",
                    ));
                    quote!(PropertyValue::#v { .. } => ::core::unreachable!(),)
                }
            },
            KeyRule::Invalid => quote!(PropertyValue::#v { .. } => ::core::unreachable!(),),
        };
        arms.push(arm);
    }
    arms
}

/// The type of the one unnamed field of a tuple variant.
fn single_field_type(fields: &Fields) -> Option<&syn::Type> {
    match fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            fields.unnamed.first().map(|f| &f.ty)
        }
        _ => None,
    }
}

/// The index of the enum named `name` among `items`.
fn find_enum(items: &[Item], name: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| matches!(item, Item::Enum(e) if e.ident == name))
}

/// Mutable access to the enum at `index` (found by [`find_enum`]).
fn enum_at(items: &mut [Item], index: usize) -> Option<&mut ItemEnum> {
    match items.get_mut(index) {
        Some(Item::Enum(e)) => Some(e),
        _ => None,
    }
}

/// Removes every `properties! { .. }` item and parses its body.
fn take_properties(items: &mut Vec<Item>, errors: &mut Errors) -> Vec<parse::RawEntry> {
    let mut raw = Vec::new();
    let mut kept = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        match item {
            Item::Macro(mac) if mac.mac.path.is_ident("properties") && mac.ident.is_none() => {
                if let Some(attr) = mac.attrs.first() {
                    errors.push(syn::Error::new_spanned(
                        attr,
                        "`properties!` takes no attributes; put doc comments on its entries",
                    ));
                }
                raw.extend(parse::parse_block(mac.mac.tokens, errors));
            }
            other => kept.push(other),
        }
    }
    *items = kept;
    raw
}

/// Expands `#[longhands]` applied to `item`.
pub(crate) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = Errors::default();
    let expanded = expand_collecting(args, item, &mut errors);
    let errors = errors.to_compile_errors();
    quote!(#errors #expanded)
}

/// [`expand`] without the `compile_error!`s: the errors are left in
/// `errors`, so tests can inspect them.
pub(crate) fn expand_collecting(
    args: TokenStream,
    item: TokenStream,
    errors: &mut Errors,
) -> TokenStream {
    if !args.is_empty() {
        errors.push(syn::Error::new_spanned(
            &args,
            "`#[longhands]` takes no arguments; declare longhands in `properties! { .. }` inside the module",
        ));
    }
    let module = match syn::parse2::<Item>(item.clone()) {
        Ok(Item::Mod(module)) => module,
        Ok(other) => {
            errors.push(syn::Error::new_spanned(
                other,
                "`#[longhands]` applies to an inline module: `#[longhands] mod decl { .. }`",
            ));
            return item;
        }
        Err(error) => {
            errors.push(error);
            return item;
        }
    };
    expand_module(module, errors)
}

fn expand_module(mut module: ItemMod, errors: &mut Errors) -> TokenStream {
    let Some((_, mut items)) = module.content.take() else {
        errors.push(syn::Error::new_spanned(
            &module,
            "`#[longhands]` needs an inline module: `mod decl { .. }`",
        ));
        return module.into_token_stream();
    };
    let raw = take_properties(&mut items, errors);

    let value_index = find_enum(&items, "PropertyValue");
    let key_index = find_enum(&items, "PropertyKey");
    let generated = match (value_index, key_index) {
        (Some(value_index), Some(key_index)) => {
            let mut hand_written: Vec<Ident> = Vec::new();
            let key_arms = match enum_at(&mut items, value_index) {
                Some(value_enum) => {
                    hand_written.extend(value_enum.variants.iter().map(|v| v.ident.clone()));
                    take_key_arms(value_enum, errors)
                }
                None => Vec::new(),
            };
            if let Some(key_enum) = enum_at(&mut items, key_index) {
                hand_written.extend(key_enum.variants.iter().map(|v| v.ident.clone()));
            }
            let entries = model::build(raw, &hand_written, errors);
            if entries.is_empty() && errors.is_empty() {
                errors.push(syn::Error::new_spanned(
                    &module.ident,
                    "`#[longhands]` found no longhand; declare them in `properties! { .. }` inside the module",
                ));
            }
            if let Some(value_enum) = enum_at(&mut items, value_index) {
                value_enum
                    .variants
                    .extend(entries.iter().map(generate::value_variant));
            }
            if let Some(key_enum) = enum_at(&mut items, key_index) {
                key_enum
                    .variants
                    .extend(entries.iter().map(generate::key_variant));
            }
            if entries.is_empty() {
                TokenStream::new()
            } else {
                generate::items(&entries, &key_arms)
            }
        }
        (value_index, _) => {
            let missing = if value_index.is_none() {
                "PropertyValue"
            } else {
                "PropertyKey"
            };
            errors.push(syn::Error::new_spanned(
                &module.ident,
                format!(
                    "a `#[longhands]` module must declare `pub enum {missing} {{ .. }}` with its hand-written variants"
                ),
            ));
            TokenStream::new()
        }
    };

    let ItemMod {
        attrs,
        vis,
        unsafety,
        mod_token,
        ident,
        ..
    } = &module;
    let items = items.iter().map(|item| item.to_token_stream());
    quote! {
        #(#attrs)* #vis #unsafety #mod_token #ident {
            #(#items)*
            #generated
        }
    }
}
