//! Procedural macros that declare raikiri-style's CSS longhands from a
//! table.

use proc_macro::TokenStream;

/// Declares the longhand table of the annotated inline module.
#[proc_macro_attribute]
pub fn longhands(args: TokenStream, item: TokenStream) -> TokenStream {
    let _ = args;
    item
}
