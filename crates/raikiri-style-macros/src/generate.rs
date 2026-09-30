//! Code generation from validated [`Entry`] values.
//!
//! [`items`] is an explicit sequence of projections: each is a function
//! over the validated entries, named after the items it produces
//! (`key_method`, `table_structs`, `parse_dispatch_fn`, ..), and has its own
//! snapshot test on a fixed fixture. The per-entry pieces (`keyword_enum`,
//! `type_module`, `longhand_impl`, and the enum variants `expand` appends)
//! take one entry. A new generated item is a new projection added to that
//! sequence; validation stays in [`crate::model`].
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

use crate::model::{
    Compute, Entry, FIXED_DERIVES, Lift, Residue, Serialize, Value, is_reserved_type,
};

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
        let default = if entry.default_keyword.as_ref() == Some(kw) {
            quote!(#[default])
        } else {
            TokenStream::new()
        };
        quote!(#kw_doc #default #kw,)
    });
    let fixed = FIXED_DERIVES
        .iter()
        .map(|name| Ident::new(name, Span::call_site()));
    let derives = &entry.derives;
    let kws: Vec<_> = list.iter().map(|k| &k.ident).collect();
    let css: Vec<_> = list.iter().map(|k| &k.css).collect();
    let this = local("this");
    let name = local("ident");
    quote! {
        #doc
        #[non_exhaustive]
        #[derive(#(#fixed),* #(, #derives)*)]
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
    // A reserved name has been reported; `super::` keeps the alias from
    // naming itself, so the report is the only error.
    let in_module = |ty: TokenStream| match syn::parse2::<syn::Type>(ty.clone()) {
        Ok(parsed) if is_reserved_type(&parsed) => quote!(super::#ty),
        _ => ty,
    };
    let specified = in_module(specified_ty(entry));
    let computed = match &entry.computed_ty {
        Some(ty) => in_module(ty.to_token_stream()),
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
        Compute::Path(hook) => {
            let f = local("compute");
            quote_spanned! {hook.span()=>
                let #f: fn(Self::Specified, &#cx_ty) -> Self::Computed = #hook;
                #f(#specified, #cx)
            }
        }
        Compute::Identity => quote! {
            let _ = #cx;
            #specified
        },
        Compute::Broken => unreachable_body(&[&specified, &cx]),
    };

    let lift = match (&entry.computed_ty, &entry.lift) {
        (None, _) => quote!(#computed),
        (Some(_), Lift::Broken) => unreachable_body(&[&computed]),
        (Some(ty), lift) => {
            let f = local("lift");
            let (path, span) = match lift {
                Lift::Path(path) => (path.to_token_stream(), path.span()),
                _ => (quote!(::core::convert::Into::into), ty.span()),
            };
            quote_spanned! {span=>
                let #f: fn(Self::Computed) -> Self::Specified = #path;
                #f(#computed)
            }
        }
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

/// The `longhand_specified_residue` arm of one entry.
fn residue_arm(entry: &Entry, value: &Ident) -> TokenStream {
    let variant = &entry.variant;
    let field = &entry.field;
    let body = match &entry.residue {
        Residue::None => quote! {
            let _ = #value;
            ::core::option::Option::None
        },
        Residue::Path(path) => {
            let f = local("residue");
            quote_spanned! {path.span()=>
                let #f: fn(&#field::Specified) -> ::core::option::Option<&'static str> = #path;
                #f(#value)
            }
        }
        Residue::Broken => unreachable_body(&[value]),
    };
    quote!(PropertyValue::#variant(#value) => { #body })
}

/// Every generated item of the module, after the (extended) hand-written
/// enums: the projections below, in this order. `key_arms` are the `key()`
/// arms of the hand-written variants.
pub(crate) fn items(entries: &[Entry], key_arms: &[TokenStream]) -> TokenStream {
    let items = [
        per_entry_items(entries),
        key_method(entries, key_arms),
        table_structs(entries),
        specified_table_impl(entries),
        computed_table_impl(entries),
        page_absolutize_fn(entries),
        deref_impls(),
        longhand_names_const(entries),
        value_pat_macro(entries),
        sample_fns(entries),
        specified_residue_fn(entries),
        registry_macros(entries),
        name_lookup_fn(entries),
        parse_dispatch_fn(entries),
        serialize_fn(entries),
        serialize_computed_fn(entries),
        serializes_fn(entries),
    ];
    quote!(#(#items)*)
}

/// The variant names of `entries`.
fn variants(entries: &[Entry]) -> Vec<&Ident> {
    entries.iter().map(|e| &e.variant).collect()
}

/// The table field names of `entries`.
fn fields(entries: &[Entry]) -> Vec<&Ident> {
    entries.iter().map(|e| &e.field).collect()
}

/// The `Longhand` projections of `entries`.
fn projections(entries: &[Entry]) -> Vec<TokenStream> {
    entries.iter().map(projection).collect()
}

/// The entries whose name takes part in name lookup and parse dispatch.
fn listed(entries: &[Entry]) -> Vec<&Entry> {
    entries.iter().filter(|e| e.name_listed).collect()
}

/// Per entry, in entry order: its keyword enum (for `keywords`), its type
/// module and its `Longhand` impl.
fn per_entry_items(entries: &[Entry]) -> TokenStream {
    let items = entries.iter().map(|entry| {
        let keyword_enum = keyword_enum(entry);
        let module = type_module(entry);
        let longhand = longhand_impl(entry);
        quote!(#keyword_enum #module #longhand)
    });
    quote!(#(#items)*)
}

/// `PropertyValue::key()`: the hand-written arms, then one arm per entry.
fn key_method(entries: &[Entry], key_arms: &[TokenStream]) -> TokenStream {
    let variants = variants(entries);
    quote! {
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
    }
}

/// `SpecifiedTable` and `ComputedTable`: one field per entry.
fn table_structs(entries: &[Entry]) -> TokenStream {
    let fields = fields(entries);
    let specified_docs = entries
        .iter()
        .map(|e| docs(&format!("Specified {}.", code_name(e)), &e.docs));
    let computed_docs = entries
        .iter()
        .map(|e| docs(&format!("Computed {}.", code_name(e)), &e.docs));
    quote! {
        /// Specified values of the longhands declared in `properties!`,
        /// embedded as `SpecifiedValues::longhands`; its fields are readable
        /// directly on `SpecifiedValues` through `Deref` and written through
        /// `longhands`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct SpecifiedTable {
            #( #specified_docs pub #fields: #fields::Specified, )*
        }

        /// Computed values of the longhands declared in `properties!`,
        /// embedded as `ComputedValues::longhands`; its fields are readable
        /// directly on `ComputedValues` through `Deref` and written through
        /// `longhands`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct ComputedTable {
            #( #computed_docs pub #fields: #fields::Computed, )*
        }
    }
}

/// The test-only `equal_fields` method shared by both tables.
fn equal_fields_method(entries: &[Entry]) -> TokenStream {
    let fields = fields(entries);
    let all_names: Vec<_> = entries.iter().map(|e| &e.name).collect();
    let other = local("other");
    let pairs = local("pairs");
    let name = local("name");
    let equal = local("equal");
    quote! {
        /// The CSS names of the entries whose fields are equal in `self`
        /// and `other`, in declaration order (test only; for checking that
        /// a fixture differs from the initial values in every entry).
        #[cfg(test)]
        pub(crate) fn equal_fields(&self, #other: &Self) -> ::std::vec::Vec<&'static str> {
            let #pairs: &[(&'static str, bool)] = &[
                #( (#all_names, self.#fields == #other.#fields), )*
            ];
            #pairs
                .iter()
                .filter(|(_, #equal)| *#equal)
                .map(|(#name, _)| *#name)
                .collect()
        }
    }
}

/// `impl SpecifiedTable`: `initial`, `inherit_from`, `absolutize`, `apply`
/// and the test-only `sample` and `equal_fields`.
fn specified_table_impl(entries: &[Entry]) -> TokenStream {
    let variants = variants(entries);
    let fields = fields(entries);
    let projections = projections(entries);
    let parent = local("parent");
    let inherit = entries.iter().map(|e| {
        let p = projection(e);
        let field = &e.field;
        if e.inherited {
            quote!(#p::lift(::core::clone::Clone::clone(&#parent.#field)))
        } else {
            quote!(#p::initial())
        }
    });
    let value = local("value");
    let cx = local("cx");
    let v = local("v");
    let other = local("other");
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);
    let equal_fields = equal_fields_method(entries);
    quote! {
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

            /// Every field at its entry's `sample` value: a non-initial
            /// value for test fixtures.
            #[cfg(test)]
            pub(crate) fn sample() -> Self {
                Self { #( #fields: #projections::sample(), )* }
            }

            #equal_fields
        }
    }
}

/// `impl ComputedTable`: `initial` and the test-only `sample` and
/// `equal_fields`.
fn computed_table_impl(entries: &[Entry]) -> TokenStream {
    let fields = fields(entries);
    let projections = projections(entries);
    let cx = local("cx");
    let ctx = local("ctx");
    let equal_fields = equal_fields_method(entries);
    quote! {
        #[allow(unused_variables)]
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

            /// Every field at its entry's `sample` value, computed in the
            /// initial context: a non-initial value for test fixtures, as
            /// long as no entry's `compute` maps its sample to the computed
            /// initial value (check with [`Self::equal_fields`]).
            #[cfg(test)]
            pub(crate) fn sample() -> Self {
                let #ctx = crate::resolve::ResolveContext::initial();
                let #cx = &crate::property::AbsolutizeCx::initial(&#ctx);
                Self {
                    #( #fields: #projections::compute(#projections::sample(), #cx), )*
                }
            }

            #equal_fields
        }
    }
}

/// `longhand_page_absolutize`: `lift(compute(value))` per entry.
fn page_absolutize_fn(entries: &[Entry]) -> TokenStream {
    let variants = variants(entries);
    let projections = projections(entries);
    let value = local("value");
    let cx = local("cx");
    let v = local("v");
    let other = local("other");
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);
    quote! {
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
    }
}

/// The read-only `Deref` from the host's value structs to the tables.
fn deref_impls() -> TokenStream {
    quote! {
        impl ::core::ops::Deref for crate::specified::SpecifiedValues {
            type Target = SpecifiedTable;
            fn deref(&self) -> &SpecifiedTable {
                &self.longhands
            }
        }
        impl ::core::ops::Deref for crate::computed::ComputedValues {
            type Target = ComputedTable;
            fn deref(&self) -> &ComputedTable {
                &self.longhands
            }
        }
    }
}

/// `LONGHAND_NAMES`.
fn longhand_names_const(entries: &[Entry]) -> TokenStream {
    let listed_names = listed(entries).into_iter().map(|e| &e.name);
    quote! {
        /// Every longhand name declared in `properties!`, lowercase, in
        /// declaration order.
        pub(crate) const LONGHAND_NAMES: &[&str] = &[#(#listed_names),*];
    }
}

/// `longhand_value_pat!()`, or nothing without entries.
fn value_pat_macro(entries: &[Entry]) -> TokenStream {
    let variants = variants(entries);
    let value_pat_name = format_ident!("longhand_value_pat");
    // An or-pattern needs at least one alternative, and no pattern matches
    // nothing, so without entries (every entry failed; the errors are
    // reported) the macro is omitted and each use of it fails as well.
    if variants.is_empty() {
        return TokenStream::new();
    }
    quote! {
        /// Pattern matching every `PropertyValue` variant declared in
        /// `properties!`, for the exhaustive matches whose declared
        /// longhands all take the same pass-through arm.
        macro_rules! #value_pat_name {
            () => { #( crate::property::PropertyValue::#variants(..) )|* };
        }
        pub(crate) use #value_pat_name;
    }
}

/// The test-only `longhand_samples` and `longhand_sample`.
fn sample_fns(entries: &[Entry]) -> TokenStream {
    let variants = variants(entries);
    let projections = projections(entries);
    let key = local("key");
    let other = local("other");
    quote! {
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
    }
}

/// The test-only `longhand_specified_residue`: one arm per entry.
fn specified_residue_fn(entries: &[Entry]) -> TokenStream {
    let value = local("value");
    let v = local("v");
    let other = local("other");
    let residue_arms = entries.iter().map(|e| residue_arm(e, &v));
    quote! {
        /// The length a declared longhand's specified value still carries
        /// (a unit or form that computing resolves), named for a test
        /// failure message; `None` when it carries none. Each entry answers
        /// through its `residue:` (a function, or `none` for a value that
        /// never carries a length). `value` must be a variant declared in
        /// `properties!`.
        #[cfg(test)]
        pub(crate) fn longhand_specified_residue(
            #value: &PropertyValue,
        ) -> ::core::option::Option<&'static str> {
            match #value {
                #( #residue_arms )*
                #other => ::core::unreachable!(
                    "not a longhand declared in `properties!`: {:?}", #other
                ),
            }
        }
    }
}

/// The test-only registry callbacks `with_longhand_samples!` and
/// `with_longhand_variants!`.
fn registry_macros(entries: &[Entry]) -> TokenStream {
    let variants = variants(entries);
    quote! {
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
    }
}

/// `longhand_key_for_name`: name lookup over the listed names.
fn name_lookup_fn(entries: &[Entry]) -> TokenStream {
    let listed = listed(entries);
    let listed_names = listed.iter().map(|e| &e.name);
    let listed_variants = listed.iter().map(|e| &e.variant);
    let normalized_name = local("normalized_name");
    quote! {
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
    }
}

/// `parse_longhand_value`: parse dispatch over the listed names.
fn parse_dispatch_fn(entries: &[Entry]) -> TokenStream {
    let listed = listed(entries);
    let listed_names = listed.iter().map(|e| &e.name);
    let listed_variants = listed.iter().map(|e| &e.variant);
    let listed_projections = listed.iter().map(|e| projection(e));
    let normalized_name = local("normalized_name");
    let input = local("input");
    quote! {
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

/// The `longhand_serialize` arm of one entry.
fn serialize_arm(entry: &Entry, value: &Ident) -> TokenStream {
    let variant = &entry.variant;
    let field = &entry.field;
    let body = match &entry.serialize {
        Serialize::None => quote! {
            let _ = #value;
            ::core::option::Option::None
        },
        Serialize::Keyword => quote! {
            ::core::option::Option::Some(::std::string::String::from(#value.as_css_str()))
        },
        Serialize::Path(path) => {
            let f = local("serialize");
            quote_spanned! {path.span()=>
                let #f: fn(&#field::Specified) -> ::core::option::Option<::std::string::String> = #path;
                #f(#value)
            }
        }
        Serialize::Broken => unreachable_body(&[value]),
    };
    quote!(PropertyValue::#variant(#value) => { #body })
}

/// `longhand_serialize`: one arm per entry.
fn serialize_fn(entries: &[Entry]) -> TokenStream {
    let value = local("value");
    let v = local("v");
    let other = local("other");
    let arms = entries.iter().map(|e| serialize_arm(e, &v));
    quote! {
        /// Serializes a declared longhand's specified value to CSS text
        /// through its entry's `serialize:` (the keyword spelling, or the
        /// named function); `None` for an entry without `serialize:`, whose
        /// callers echo the input instead. `value` must be a variant
        /// declared in `properties!`.
        pub(crate) fn longhand_serialize(
            #value: &PropertyValue,
        ) -> ::core::option::Option<::std::string::String> {
            match #value {
                #( #arms )*
                #other => ::core::unreachable!(
                    "not a longhand declared in `properties!`: {:?}", #other
                ),
            }
        }
    }
}

/// `longhand_serialize_computed`: per entry with `serialize:`, the computed
/// field lifted back to specified form and serialized by
/// `longhand_serialize`, so a serializer's signature is checked in one
/// place.
fn serialize_computed_fn(entries: &[Entry]) -> TokenStream {
    let key = local("key");
    let computed = local("computed");
    // A `Serialize::Broken` entry counts as serialized: its mistake has
    // been reported, so the expansion never compiles and the arm is moot.
    let serialized: Vec<&Entry> = entries
        .iter()
        .filter(|e| !matches!(e.serialize, Serialize::None))
        .collect();
    // Without a serialized entry no arm reads the table.
    let touch = if serialized.is_empty() {
        quote!(let _ = #computed;)
    } else {
        TokenStream::new()
    };
    let arms = serialized.iter().map(|e| {
        let variant = &e.variant;
        let field = &e.field;
        let p = projection(e);
        quote! {
            PropertyKey::#variant => longhand_serialize(&PropertyValue::#variant(
                #p::lift(::core::clone::Clone::clone(&#computed.#field)),
            )),
        }
    });
    quote! {
        /// Serializes the computed value of the declared longhand `key`
        /// from `computed`, for CSSOM computed-style reads: the field is
        /// lifted back to its specified type (the identity when the entry
        /// has no computed type of its own) and serialized as
        /// `longhand_serialize` does. `None` for an entry without
        /// `serialize:` and for a key not declared in `properties!`.
        pub(crate) fn longhand_serialize_computed(
            #key: PropertyKey,
            #computed: &ComputedTable,
        ) -> ::core::option::Option<::std::string::String> {
            #touch
            match #key {
                #( #arms )*
                _ => ::core::option::Option::None,
            }
        }
    }
}

/// `longhand_serializes`: whether an entry has `serialize:`.
fn serializes_fn(entries: &[Entry]) -> TokenStream {
    let key = local("key");
    // A `Serialize::Broken` entry counts as serialized, as in
    // `serialize_computed_fn`; the expansion never compiles then.
    let serialized: Vec<_> = entries
        .iter()
        .filter(|e| !matches!(e.serialize, Serialize::None))
        .map(|e| &e.variant)
        .collect();
    let body = if serialized.is_empty() {
        quote! {
            let _ = #key;
            false
        }
    } else {
        quote!(::core::matches!(#key, #( PropertyKey::#serialized )|*))
    };
    quote! {
        /// Whether the declared longhand `key` has a serialization
        /// (`serialize:`); `false` for a key not declared in `properties!`.
        pub(crate) fn longhand_serializes(#key: PropertyKey) -> bool {
            #body
        }
    }
}

#[cfg(test)]
mod tests;
