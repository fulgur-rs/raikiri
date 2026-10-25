# raikiri-html

The HTML and XHTML entry points: parsing, cascade assembly, resource handoff, and the layout entry point that returns an owned `DocumentLayout` of pages to draw.

raikiri is a Rust HTML/CSS paged layout and rendering foundation, developed to
replace the layout engine of [Fulgur](https://github.com/fulgur-rs/fulgur).
`raikiri-html` is one of the crates that Fulgur builds on. The 0.0.x releases make no
promise of API stability: any release may change the public API.

- Repository: <https://github.com/fulgur-rs/raikiri>
- License: MIT OR Apache-2.0
