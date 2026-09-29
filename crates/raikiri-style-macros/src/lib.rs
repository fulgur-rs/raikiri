//! Procedural macros that declare raikiri-style's CSS longhands.
//!
//! The expansions name raikiri-style's own items (`crate::property::Longhand`,
//! `crate::specified::SpecifiedValues`, ...), so these macros are only usable
//! from inside that crate. See `raikiri-style/src/property/longhands.rs` for
//! the recipe.
//!
//! - [`macro@Longhand`] declares one property on its value type (or on a
//!   marker type) by implementing `Longhand`.
//!
//! Neither macro panics on malformed input: every mistake is reported as a
//! compile error spanned on the offending tokens of the declaration.

mod case;
mod derive;

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
