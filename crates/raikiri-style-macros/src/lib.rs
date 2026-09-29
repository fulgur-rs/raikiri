//! Procedural macros that declare raikiri-style's CSS longhands.
//!
//! The expansions name raikiri-style's own items (`crate::property::Longhand`,
//! `crate::specified::SpecifiedValues`, ...), so these macros are only usable
//! from inside that crate. See `raikiri-style/src/property/longhands.rs` for
//! the recipe.
//!
//! - [`macro@Longhand`] declares one property on its value type (or on a
//!   marker type) by implementing `Longhand`.
//! - [`macro@longhands`] aggregates the declared properties into
//!   `PropertyValue`, `PropertyKey` and the generated tables.
//!
//! Neither macro panics on malformed input: every mistake is reported as a
//! compile error spanned on the offending tokens of the declaration.

mod case;
mod derive;
mod table;

use proc_macro::TokenStream;

/// Implements `Longhand` for a property declared with `#[longhand(..)]`.
///
/// Keys (in any order): `name = "css-name"`, `initial = <expr>`,
/// `inherited = <bool>` and `sample = <expr>` are required; `value = <Type>`,
/// `parse = <path>`, `compute = <path>`, `computed = <Type>` and
/// `lift = <path>` are optional. See `property/longhands.rs` for their
/// meaning.
#[proc_macro_derive(Longhand, attributes(longhand, css))]
pub fn derive_longhand(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Appends the listed longhands to the `PropertyValue` and `PropertyKey`
/// enums of the annotated inline module and generates the table items.
///
/// `#[longhands(Isolation, ObjectFit, ..)] mod decl { .. }`: each identifier
/// names a type implementing `Longhand` and becomes the `PropertyValue` /
/// `PropertyKey` variant name; its table field is the identifier in snake
/// case. A hand-written `PropertyValue` variant maps to the `PropertyKey`
/// variant of the same name unless it carries `#[key(OtherKey)]` or
/// `#[key(|payload| expr)]`.
#[proc_macro_attribute]
pub fn longhands(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = proc_macro2::TokenStream::from(args);
    let item = syn::parse_macro_input!(item as syn::ItemMod);
    table::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
