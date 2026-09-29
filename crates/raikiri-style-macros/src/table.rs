//! `#[longhands(..)] mod decl { .. }`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::{Expr, ExprClosure, Ident, Item, ItemEnum, ItemMod, Token};

use crate::case::split_camel;

/// How a hand-written `PropertyValue` variant maps to its `PropertyKey`.
enum KeyRule {
    /// No `#[key]`: the key variant of the same name.
    Same,
    /// `#[key(Other)]`: another key variant.
    Other(Ident),
    /// `#[key(|payload| expr)]`: computed from the single payload field.
    Closure(Box<ExprClosure>),
}

fn parse_key_attr(attr: &syn::Attribute) -> syn::Result<KeyRule> {
    let expr: Expr = attr.parse_args()?;
    match expr {
        Expr::Path(path) if path.qself.is_none() && path.path.get_ident().is_some() => {
            Ok(KeyRule::Other(path.path.segments[0].ident.clone()))
        }
        Expr::Closure(closure) if closure.inputs.len() == 1 => {
            Ok(KeyRule::Closure(Box::new(closure)))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "`#[key(..)]` takes a `PropertyKey` variant name, or `|payload| expr` for a variant with one field",
        )),
    }
}

/// Removes the `#[key(..)]` helper attributes from `PropertyValue`'s
/// variants and returns one `key()` match arm per variant.
fn take_key_arms(value_enum: &mut ItemEnum) -> syn::Result<Vec<TokenStream>> {
    let mut arms = Vec::with_capacity(value_enum.variants.len());
    for variant in &mut value_enum.variants {
        let mut rule = KeyRule::Same;
        let mut seen = false;
        let mut error: Option<syn::Error> = None;
        variant.attrs.retain(|attr| {
            if !attr.path().is_ident("key") {
                return true;
            }
            let result = if seen {
                Err(syn::Error::new_spanned(attr, "duplicate `#[key(..)]`"))
            } else {
                parse_key_attr(attr)
            };
            seen = true;
            match result {
                Ok(r) => rule = r,
                Err(e) => match &mut error {
                    Some(prev) => prev.combine(e),
                    None => error = Some(e),
                },
            }
            false
        });
        if let Some(error) = error {
            return Err(error);
        }
        let v = &variant.ident;
        arms.push(match rule {
            KeyRule::Same => {
                quote_spanned!(v.span()=> PropertyValue::#v { .. } => PropertyKey::#v,)
            }
            KeyRule::Other(key) => {
                quote_spanned!(key.span()=> PropertyValue::#v { .. } => PropertyKey::#key,)
            }
            KeyRule::Closure(closure) => {
                let syn::Fields::Unnamed(fields) = &variant.fields else {
                    return Err(syn::Error::new_spanned(
                        &closure,
                        "`#[key(|payload| ..)]` needs a variant with exactly one unnamed field",
                    ));
                };
                if fields.unnamed.len() != 1 {
                    return Err(syn::Error::new_spanned(
                        &closure,
                        "`#[key(|payload| ..)]` needs a variant with exactly one unnamed field",
                    ));
                }
                let pat = &closure.inputs[0];
                let body = &closure.body;
                quote!(PropertyValue::#v(#pat) => #body,)
            }
        });
    }
    Ok(arms)
}

fn find_enum<'a>(items: &'a mut [Item], name: &str) -> Option<&'a mut ItemEnum> {
    items.iter_mut().find_map(|item| match item {
        Item::Enum(e) if e.ident == name => Some(e),
        _ => None,
    })
}

pub(crate) fn expand(args: TokenStream, mut module: ItemMod) -> syn::Result<TokenStream> {
    let props = Punctuated::<Ident, Token![,]>::parse_terminated.parse2(args)?;
    let props: Vec<Ident> = props.into_iter().collect();
    for (i, p) in props.iter().enumerate() {
        if props[..i].contains(p) {
            return Err(syn::Error::new_spanned(p, "longhand listed twice"));
        }
    }
    let fields: Vec<Ident> = props
        .iter()
        .map(|p| format_ident!("{}", split_camel(&p.to_string(), '_'), span = p.span()))
        .collect();

    let mod_ident = module.ident.clone();
    let Some((_, mut items)) = module.content.take() else {
        return Err(syn::Error::new_spanned(
            &mod_ident,
            "`#[longhands]` needs an inline module: `mod decl { .. }`",
        ));
    };

    let longhand = quote!(crate::property::Longhand);
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);

    let key_arms = {
        let Some(value_enum) = find_enum(&mut items, "PropertyValue") else {
            return Err(syn::Error::new_spanned(
                &mod_ident,
                "`#[longhands]` module must declare `enum PropertyValue`",
            ));
        };
        let arms = take_key_arms(value_enum)?;
        for p in &props {
            let doc = format!("Specified value of the property declared by [`{p}`].");
            value_enum
                .variants
                .push(syn::parse_quote_spanned!(p.span()=> #[doc = #doc] #p(<#p as #longhand>::Specified)));
        }
        arms
    };
    {
        let Some(key_enum) = find_enum(&mut items, "PropertyKey") else {
            return Err(syn::Error::new_spanned(
                &mod_ident,
                "`#[longhands]` module must declare `enum PropertyKey`",
            ));
        };
        for p in &props {
            let doc = format!("Key of the property declared by [`{p}`].");
            key_enum
                .variants
                .push(syn::parse_quote_spanned!(p.span()=> #[doc = #doc] #p));
        }
    }

    let specified_docs = props
        .iter()
        .map(|p| format!("Specified value of the property declared by [`{p}`]."));
    let computed_docs = props
        .iter()
        .map(|p| format!("Computed value of the property declared by [`{p}`]."));

    // Two longhands declaring the same CSS name fail to compile, at the
    // later entry in the attribute's list.
    let name_checks = props.iter().enumerate().map(|(i, p)| {
        quote_spanned! {p.span()=>
            const _: () = assert!(
                !__longhand_name_declared_before(LONGHAND_NAMES, #i),
                "this longhand's CSS name is already declared by an earlier entry",
            );
        }
    });
    let names_check = quote! {
        /// Whether `names[index]` also occurs in `names[..index]`.
        const fn __longhand_name_declared_before(names: &[&str], index: usize) -> bool {
            let wanted = names[index].as_bytes();
            let mut i = 0;
            while i < index {
                let other = names[i].as_bytes();
                if other.len() == wanted.len() {
                    let mut b = 0;
                    while b < other.len() && other[b] == wanted[b] {
                        b += 1;
                    }
                    if b == other.len() {
                        return true;
                    }
                }
                i += 1;
            }
            false
        }
        #(#name_checks)*
    };

    let generated = quote! {
        impl PropertyValue {
            /// Returns the property key for this value.
            ///
            /// Used as the discriminant for selecting one winner per property in the
            /// cascade, and as the key in the `@page` cascade result map.
            pub fn key(&self) -> PropertyKey {
                match self {
                    #(#key_arms)*
                    #( PropertyValue::#props(_) => PropertyKey::#props, )*
                }
            }
        }

        /// Table-declared specified values, embedded as
        /// `SpecifiedValues::longhands`; its fields are reachable directly on
        /// `SpecifiedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct SpecifiedTable {
            #( #[doc = #specified_docs] pub #fields: <#props as #longhand>::Specified, )*
        }

        /// Table-declared computed values, embedded as
        /// `ComputedValues::longhands`; its fields are reachable directly on
        /// `ComputedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct ComputedTable {
            #( #[doc = #computed_docs] pub #fields: <#props as #longhand>::Computed, )*
        }

        #[allow(clippy::clone_on_copy)]
        impl SpecifiedTable {
            /// Every field at its initial value.
            pub(crate) fn initial() -> Self {
                Self { #( #fields: <#props as #longhand>::initial(), )* }
            }

            /// The specified state of a child: inherited fields copy the
            /// parent's computed value (lifted back to specified form), the
            /// others start at their initial value.
            pub(crate) fn inherit_from(parent: &ComputedTable) -> Self {
                Self {
                    #(
                        #fields: if <#props as #longhand>::INHERITED {
                            <#props as #longhand>::lift(::core::clone::Clone::clone(&parent.#fields))
                        } else {
                            <#props as #longhand>::initial()
                        },
                    )*
                }
            }

            /// Specified to computed, through each property's `compute`.
            pub(crate) fn absolutize(self, cx: &#cx_ty) -> ComputedTable {
                ComputedTable {
                    #( #fields: <#props as #longhand>::compute(self.#fields, cx), )*
                }
            }

            /// Stores a cascade winner in its field. `value` must be a table variant.
            pub(crate) fn apply(&mut self, value: PropertyValue) {
                match value {
                    #( PropertyValue::#props(v) => self.#fields = v, )*
                    other => unreachable!("not a table-declared property value: {other:?}"),
                }
            }
        }

        impl ComputedTable {
            /// Every field at its initial value, computed in the initial
            /// context.
            pub(crate) fn initial() -> Self {
                let ctx = crate::resolve::ResolveContext::initial();
                let cx = &crate::property::AbsolutizeCx::initial(&ctx);
                Self {
                    #( #fields: <#props as #longhand>::compute(<#props as #longhand>::initial(), cx), )*
                }
            }
        }

        /// Page-context absolutization of one table value. `PropertyValue`
        /// keeps its specified payload type, so the value is computed and
        /// lifted back. `value` must be a table variant.
        pub(crate) fn longhand_page_absolutize(
            value: PropertyValue,
            cx: &#cx_ty,
        ) -> PropertyValue {
            match value {
                #(
                    PropertyValue::#props(v) => PropertyValue::#props(
                        <#props as #longhand>::lift(<#props as #longhand>::compute(v, cx)),
                    ),
                )*
                other => unreachable!("not a table-declared property value: {other:?}"),
            }
        }

        impl ::core::ops::Deref for crate::specified::SpecifiedValues {
            type Target = SpecifiedTable;
            fn deref(&self) -> &SpecifiedTable {
                &self.longhands
            }
        }
        impl ::core::ops::DerefMut for crate::specified::SpecifiedValues {
            fn deref_mut(&mut self) -> &mut SpecifiedTable {
                &mut self.longhands
            }
        }
        impl ::core::ops::Deref for crate::computed::ComputedValues {
            type Target = ComputedTable;
            fn deref(&self) -> &ComputedTable {
                &self.longhands
            }
        }
        impl ::core::ops::DerefMut for crate::computed::ComputedValues {
            fn deref_mut(&mut self) -> &mut ComputedTable {
                &mut self.longhands
            }
        }

        /// Every table-declared property name, lowercase.
        pub(crate) const LONGHAND_NAMES: &[&str] = &[#( <#props as #longhand>::NAME ),*];

        #names_check

        /// Pattern matching every table-declared `PropertyValue` variant,
        /// for the exhaustive matches whose table properties all take the
        /// same pass-through arm.
        macro_rules! longhand_value_pat {
            () => { #( PropertyValue::#props(..) )|* };
        }
        pub(crate) use longhand_value_pat;

        /// Every table property's CSS name paired with its sample value.
        #[cfg(test)]
        pub(crate) fn longhand_samples() -> Vec<(&'static str, PropertyValue)> {
            vec![ #( (<#props as #longhand>::NAME, longhand_sample(PropertyKey::#props)), )* ]
        }

        /// The sample value of the table property `key`.
        #[cfg(test)]
        pub(crate) fn longhand_sample(key: PropertyKey) -> PropertyValue {
            match key {
                #( PropertyKey::#props => PropertyValue::#props(<#props as #longhand>::sample()), )*
                other => unreachable!("not a table-declared property key: {other:?}"),
            }
        }

        /// Invokes `$cb! { <hand entries> Variant => sample, ... }` with every
        /// table property's sample value appended after the hand entries.
        // The expansion names only items reachable from `crate::property`,
        // so it resolves wherever it is invoked.
        #[cfg(test)]
        macro_rules! with_longhand_samples {
            ($cb:ident { $($hand:tt)* }) => {
                $cb! { $($hand)* #( #props => crate::property::longhand_sample(crate::property::PropertyKey::#props), )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_samples;

        /// Invokes `$cb! { <hand names> Variant, ... }` with every table
        /// variant name appended after the hand names.
        #[cfg(test)]
        macro_rules! with_longhand_variants {
            ($cb:ident { $($hand:tt)* }) => {
                $cb! { $($hand)* #( #props, )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_variants;

        /// Name lookup for table-declared properties; `normalized_name` must
        /// already be ASCII-lowercase.
        pub(crate) fn longhand_key_for_name(normalized_name: &str) -> ::core::option::Option<PropertyKey> {
            match normalized_name {
                #( <#props as #longhand>::NAME => ::core::option::Option::Some(PropertyKey::#props), )*
                _ => ::core::option::Option::None,
            }
        }

        /// Value parsing for table-declared properties; `normalized_name` must
        /// already be ASCII-lowercase. `None` covers both an unknown name and
        /// an invalid value, like `parse_value`.
        pub(crate) fn parse_longhand_value(
            normalized_name: &str,
            input: &mut cssparser::Parser<'_, '_>,
        ) -> ::core::option::Option<PropertyValue> {
            match normalized_name {
                #( <#props as #longhand>::NAME => <#props as #longhand>::parse(input).map(PropertyValue::#props), )*
                _ => ::core::option::Option::None,
            }
        }
    };

    let items = &items;
    let ItemMod {
        attrs,
        vis,
        unsafety,
        mod_token,
        ..
    } = &module;
    Ok(quote! {
        #(#attrs)* #vis #unsafety #mod_token #mod_ident {
            #(#items)*
            #generated
        }
    })
}
