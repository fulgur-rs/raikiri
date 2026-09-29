//! Procedural macros that declare raikiri-style's CSS longhands from a
//! table.

// The parser and model have no caller until the expansion lands.
#![allow(dead_code)]

mod case;
mod diag;
mod model;
mod parse;

use proc_macro::TokenStream;

/// Declares the longhand table of the annotated inline module.
#[proc_macro_attribute]
pub fn longhands(args: TokenStream, item: TokenStream) -> TokenStream {
    let _ = args;
    item
}
