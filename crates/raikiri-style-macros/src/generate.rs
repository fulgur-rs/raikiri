//! Code generation from validated [`Entry`] values.
//!
//! Paths into the host crate are spelled `crate::..` (see the crate docs for
//! the full list). Locals introduced by the expansion use
//! [`Span::mixed_site`], so user expressions (`initial:`, `sample:`, hook
//! paths) can neither see nor shadow them. User tokens keep their own spans,
//! and the fn-pointer coercions that check hook signatures are spanned on
//! the hook path, so a signature mismatch is reported at the declaration.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens as _, format_ident, quote, quote_spanned};
use syn::spanned::Spanned as _;
use syn::{Ident, Variant};

use crate::model::{Entry, Value};

/// A local identifier invisible to user tokens.
fn local(name: &str) -> Ident {
    Ident::new(name, Span::mixed_site())
}

/// `#[doc = ".."]` attributes: one generated summary line, then the
/// entry's own doc comments as a separate paragraph.
fn docs(summary: &str, user: &[syn::Attribute]) -> TokenStream {
    let summary = format!(" {summary}");
    if user.is_empty() {
        quote!(#[doc = #summary])
    } else {
        quote!(#[doc = #summary] #[doc = ""] #(#user)*)
    }
}

/// The CSS name as inline code: "`object-fit`".
fn code_name(entry: &Entry) -> String {
    format!("`{}`", entry.name.value())
}

/// The specified value type, as spelled in the declaring module.
fn specified_ty(entry: &Entry) -> TokenStream {
    match &entry.value {
        Value::Keywords(_) => entry.variant.to_token_stream(),
        Value::Parsed { ty, .. } => ty.to_token_stream(),
    }
}

/// `<field::Property as crate::property::Longhand>`.
fn projection(entry: &Entry) -> TokenStream {
    let field = &entry.field;
    quote!(<#field::Property as crate::property::Longhand>)
}

/// The variant appended to the hand-written `PropertyValue`.
pub(crate) fn value_variant(entry: &Entry) -> Variant {
    let doc = docs(&format!("The {} longhand.", code_name(entry)), &entry.docs);
    let variant = &entry.variant;
    let field = &entry.field;
    syn::parse_quote!(#doc #variant(#field::Specified))
}

/// The variant appended to the hand-written `PropertyKey`.
pub(crate) fn key_variant(entry: &Entry) -> Variant {
    let doc = docs(&format!("Key of the {} longhand.", code_name(entry)), &[]);
    let variant = &entry.variant;
    syn::parse_quote!(#doc #variant)
}

/// The keyword enum of a `keywords` entry.
fn keyword_enum(entry: &Entry) -> TokenStream {
    let Value::Keywords(list) = &entry.value else {
        return TokenStream::new();
    };
    let ident = &entry.variant;
    let doc = docs(
        &format!(
            "Specified value of {}; see [`PropertyValue::{ident}`].",
            code_name(entry)
        ),
        &entry.docs,
    );
    let variants = list.iter().map(|k| {
        let kw = &k.ident;
        let kw_doc = if k.docs.is_empty() {
            let text = format!(" `{}`", k.css.value());
            quote!(#[doc = #text])
        } else {
            let docs = &k.docs;
            quote!(#(#docs)*)
        };
        quote!(#kw_doc #kw,)
    });
    let kws: Vec<_> = list.iter().map(|k| &k.ident).collect();
    let css: Vec<_> = list.iter().map(|k| &k.css).collect();
    let this = local("this");
    let name = local("ident");
    quote! {
        #doc
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum #ident {
            #(#variants)*
        }

        impl #ident {
            /// Every keyword, in declaration order.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[#(Self::#kws),*];

            /// This keyword's CSS spelling.
            pub const fn as_css_str(self) -> &'static str {
                let #this = self;
                match #this {
                    #(Self::#kws => #css,)*
                }
            }

            /// Parses one keyword, ASCII case-insensitively.
            pub(crate) fn from_css_ident(#name: &str) -> ::core::option::Option<Self> {
                ::cssparser::match_ignore_ascii_case! { #name,
                    #(#css => ::core::option::Option::Some(Self::#kws),)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    }
}

/// The per-entry module holding the value type aliases and the marker type
/// that implements `Longhand`.
fn type_module(entry: &Entry) -> TokenStream {
    let field = &entry.field;
    let name = code_name(entry);
    let doc = docs(&format!("Value types of the {name} longhand."), &entry.docs);
    let specified = specified_ty(entry);
    let computed = match &entry.computed_ty {
        Some(ty) => ty.to_token_stream(),
        None => specified.clone(),
    };
    let specified_doc = format!(" Specified value of {name}.");
    let computed_doc = format!(" Computed value of {name}.");
    let marker_doc =
        format!(" The {name} longhand, as a type implementing the crate's `Longhand` trait.");
    quote! {
        #doc
        pub mod #field {
            // The aliases spell the written types, which are resolved in the
            // declaring module.
            #[allow(unused_imports)]
            use super::*;

            #[doc = #specified_doc]
            pub type Specified = #specified;

            #[doc = #computed_doc]
            pub type Computed = #computed;

            #[doc = #marker_doc]
            pub(crate) enum Property {}
        }
    }
}

/// `::core::unreachable!()` after touching `used`, for the body of an item
/// whose declaration had an error (already reported).
fn unreachable_body(used: &[&Ident]) -> TokenStream {
    quote!(#(let _ = &#used;)* ::core::unreachable!())
}

/// The `Longhand` impl of one entry.
fn longhand_impl(entry: &Entry) -> TokenStream {
    let field = &entry.field;
    let name = &entry.name;
    let inherited = entry.inherited;
    let span = entry.value_span;
    let input = local("input");
    let specified = local("specified");
    let cx = local("cx");
    let computed = local("computed");
    let parser_ty = quote!(::cssparser::Parser<'_, '_>);
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);
    let specified_ret = quote_spanned!(span=> Self::Specified);

    let initial = entry
        .initial
        .clone()
        .unwrap_or_else(|| unreachable_body(&[]));
    let sample = entry
        .sample
        .clone()
        .unwrap_or_else(|| unreachable_body(&[]));

    let parse = match &entry.value {
        Value::Keywords(_) => {
            let ident = local("ident");
            let variant = &entry.variant;
            quote! {
                #input
                    .expect_ident()
                    .ok()
                    .and_then(|#ident| #variant::from_css_ident(#ident))
            }
        }
        Value::Parsed {
            parse: Some(path), ..
        } => {
            let f = local("parse");
            quote_spanned! {path.span()=>
                let #f: fn(&mut #parser_ty) -> ::core::option::Option<Self::Specified> = #path;
                #f(#input)
            }
        }
        Value::Parsed { parse: None, .. } => unreachable_body(&[&input]),
    };

    let compute = match &entry.compute {
        Some(hook) => {
            let f = local("compute");
            quote_spanned! {hook.span()=>
                let #f: fn(Self::Specified, &#cx_ty) -> Self::Computed = #hook;
                #f(#specified, #cx)
            }
        }
        None => quote! {
            let _ = #cx;
            #specified
        },
    };

    let lift = match &entry.computed_ty {
        Some(ty) => {
            let f = local("lift");
            let (path, span) = match &entry.lift {
                Some(path) => (path.to_token_stream(), path.span()),
                None => (quote!(::core::convert::Into::into), ty.span()),
            };
            quote_spanned! {span=>
                let #f: fn(Self::Computed) -> Self::Specified = #path;
                #f(#computed)
            }
        }
        None => quote!(#computed),
    };

    let specified_item = quote_spanned!(span=> type Specified = #field::Specified;);
    quote! {
        impl crate::property::Longhand for #field::Property {
            const NAME: &'static str = #name;
            const INHERITED: bool = #inherited;
            #specified_item
            type Computed = #field::Computed;

            fn initial() -> #specified_ret {
                #initial
            }

            fn parse(#input: &mut #parser_ty) -> ::core::option::Option<Self::Specified> {
                #parse
            }

            fn compute(#specified: Self::Specified, #cx: &#cx_ty) -> Self::Computed {
                #compute
            }

            fn lift(#computed: Self::Computed) -> Self::Specified {
                #lift
            }

            #[cfg(test)]
            fn sample() -> #specified_ret {
                #sample
            }
        }
    }
}

/// Every generated item of the module, after the (extended) hand-written
/// enums. `key_arms` are the `key()` arms of the hand-written variants.
pub(crate) fn items(entries: &[Entry], key_arms: &[TokenStream]) -> TokenStream {
    let per_entry = entries.iter().map(|entry| {
        let keyword_enum = keyword_enum(entry);
        let module = type_module(entry);
        let longhand = longhand_impl(entry);
        quote!(#keyword_enum #module #longhand)
    });

    let variants: Vec<_> = entries.iter().map(|e| &e.variant).collect();
    let fields: Vec<_> = entries.iter().map(|e| &e.field).collect();
    let projections: Vec<_> = entries.iter().map(projection).collect();
    let specified_docs = entries
        .iter()
        .map(|e| docs(&format!("Specified {}.", code_name(e)), &e.docs));
    let computed_docs = entries
        .iter()
        .map(|e| docs(&format!("Computed {}.", code_name(e)), &e.docs));
    let listed: Vec<&Entry> = entries.iter().filter(|e| e.name_listed).collect();
    let listed_names: Vec<_> = listed.iter().map(|e| &e.name).collect();
    let listed_variants: Vec<_> = listed.iter().map(|e| &e.variant).collect();
    let listed_projections: Vec<_> = listed.iter().map(|e| projection(e)).collect();

    let parent = local("parent");
    let inherit: Vec<_> = entries
        .iter()
        .map(|e| {
            let p = projection(e);
            let field = &e.field;
            if e.inherited {
                quote!(#p::lift(::core::clone::Clone::clone(&#parent.#field)))
            } else {
                quote!(#p::initial())
            }
        })
        .collect();
    let value = local("value");
    let cx = local("cx");
    let ctx = local("ctx");
    let v = local("v");
    let other = local("other");
    let key = local("key");
    let normalized_name = local("normalized_name");
    let input = local("input");
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);
    let value_pat = format_ident!("longhand_value_pat");

    quote! {
        #(#per_entry)*

        impl PropertyValue {
            /// Returns the property key for this value.
            ///
            /// Used as the discriminant for selecting one winner per property in the
            /// cascade, and as the key in the `@page` cascade result map.
            pub fn key(&self) -> PropertyKey {
                match self {
                    #(#key_arms)*
                    #( PropertyValue::#variants(_) => PropertyKey::#variants, )*
                }
            }
        }

        /// Specified values of the longhands declared in `properties!`,
        /// embedded as `SpecifiedValues::longhands`; its fields are reachable
        /// directly on `SpecifiedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct SpecifiedTable {
            #( #specified_docs pub #fields: #fields::Specified, )*
        }

        /// Computed values of the longhands declared in `properties!`,
        /// embedded as `ComputedValues::longhands`; its fields are reachable
        /// directly on `ComputedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct ComputedTable {
            #( #computed_docs pub #fields: #fields::Computed, )*
        }

        #[allow(clippy::clone_on_copy, unused_variables)]
        impl SpecifiedTable {
            /// Every field at its initial value.
            pub(crate) fn initial() -> Self {
                Self { #( #fields: #projections::initial(), )* }
            }

            /// The specified state of a child: inherited fields copy the
            /// parent's computed value (lifted back to specified form), the
            /// others start at their initial value.
            pub(crate) fn inherit_from(#parent: &ComputedTable) -> Self {
                Self {
                    #( #fields: #inherit, )*
                }
            }

            /// Specified to computed, through each longhand's `compute`.
            pub(crate) fn absolutize(self, #cx: &#cx_ty) -> ComputedTable {
                ComputedTable {
                    #( #fields: #projections::compute(self.#fields, #cx), )*
                }
            }

            /// Stores a cascade winner in its field. `value` must be a
            /// variant declared in `properties!`.
            pub(crate) fn apply(&mut self, #value: PropertyValue) {
                match #value {
                    #( PropertyValue::#variants(#v) => self.#fields = #v, )*
                    #other => ::core::unreachable!(
                        "not a longhand declared in `properties!`: {:?}", #other
                    ),
                }
            }
        }

        impl ComputedTable {
            /// Every field at its initial value, computed in the initial
            /// context.
            pub(crate) fn initial() -> Self {
                let #ctx = crate::resolve::ResolveContext::initial();
                let #cx = &crate::property::AbsolutizeCx::initial(&#ctx);
                Self {
                    #( #fields: #projections::compute(#projections::initial(), #cx), )*
                }
            }
        }

        /// Page-context absolutization of one declared longhand value.
        /// `PropertyValue` keeps its specified payload type, so the value is
        /// computed and lifted back. `value` must be a variant declared in
        /// `properties!`.
        pub(crate) fn longhand_page_absolutize(
            #value: PropertyValue,
            #cx: &#cx_ty,
        ) -> PropertyValue {
            match #value {
                #(
                    PropertyValue::#variants(#v) => PropertyValue::#variants(
                        #projections::lift(#projections::compute(#v, #cx)),
                    ),
                )*
                #other => ::core::unreachable!(
                    "not a longhand declared in `properties!`: {:?}", #other
                ),
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

        /// Every longhand name declared in `properties!`, lowercase, in
        /// declaration order.
        pub(crate) const LONGHAND_NAMES: &[&str] = &[#(#listed_names),*];

        /// Pattern matching every `PropertyValue` variant declared in
        /// `properties!`, for the exhaustive matches whose declared
        /// longhands all take the same pass-through arm.
        macro_rules! #value_pat {
            () => { #( crate::property::PropertyValue::#variants(..) )|* };
        }
        pub(crate) use #value_pat;

        /// Every declared longhand's CSS name paired with its sample value.
        #[cfg(test)]
        pub(crate) fn longhand_samples() -> ::std::vec::Vec<(&'static str, PropertyValue)> {
            ::std::vec![ #( (#projections::NAME, longhand_sample(PropertyKey::#variants)), )* ]
        }

        /// The sample value of the declared longhand `key`.
        #[cfg(test)]
        pub(crate) fn longhand_sample(#key: PropertyKey) -> PropertyValue {
            match #key {
                #( PropertyKey::#variants => PropertyValue::#variants(#projections::sample()), )*
                #other => ::core::unreachable!(
                    "not a longhand declared in `properties!`: {:?}", #other
                ),
            }
        }

        /// Invokes `$cb! { <hand entries> Variant => sample, ... }` with every
        /// declared longhand's sample value appended after the hand entries.
        // The expansion names only items reachable from `crate::property`,
        // so it resolves wherever it is invoked.
        #[cfg(test)]
        macro_rules! with_longhand_samples {
            ($cb:ident { $($hand:tt)* }) => {
                $cb! { $($hand)* #( #variants => crate::property::longhand_sample(crate::property::PropertyKey::#variants), )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_samples;

        /// Invokes `$cb! { <hand names> Variant, ... }` with every declared
        /// variant name appended after the hand names.
        #[cfg(test)]
        macro_rules! with_longhand_variants {
            ($cb:ident { $($hand:tt)* }) => {
                $cb! { $($hand)* #( #variants, )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_variants;

        /// Name lookup for the longhands declared in `properties!`;
        /// `normalized_name` must already be ASCII-lowercase.
        pub(crate) fn longhand_key_for_name(
            #normalized_name: &str,
        ) -> ::core::option::Option<PropertyKey> {
            match #normalized_name {
                #( #listed_names => ::core::option::Option::Some(PropertyKey::#listed_variants), )*
                _ => ::core::option::Option::None,
            }
        }

        /// Value parsing for the longhands declared in `properties!`;
        /// `normalized_name` must already be ASCII-lowercase. `None` covers
        /// both an unknown name and an invalid value.
        pub(crate) fn parse_longhand_value(
            #normalized_name: &str,
            #input: &mut ::cssparser::Parser<'_, '_>,
        ) -> ::core::option::Option<PropertyValue> {
            match #normalized_name {
                #(
                    #listed_names => #listed_projections::parse(#input)
                        .map(PropertyValue::#listed_variants),
                )*
                _ => {
                    let _ = #input;
                    ::core::option::Option::None
                }
            }
        }
    }
}
