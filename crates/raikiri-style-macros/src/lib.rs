//! Procedural macros that declare raikiri-style's CSS longhands from a
//! table.

mod case;
mod diag;
mod expand;
mod generate;
mod model;
mod parse;
#[cfg(test)]
mod tests;

use proc_macro::TokenStream;

/// Declares the longhand table of the annotated inline module.
#[proc_macro_attribute]
pub fn longhands(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::expand(args.into(), item.into()).into()
}
