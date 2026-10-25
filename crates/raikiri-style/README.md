# raikiri-style

The CSS engine: cssparser and selectors integration, one rule tree (`@page`, counters, running elements, `target-*`), the cascade that produces `ComputedValues`, and the static side of the paged-media (GCPM) directives.

raikiri is a Rust HTML/CSS paged layout and rendering foundation, developed to
replace the layout engine of [Fulgur](https://github.com/fulgur-rs/fulgur).
`raikiri-style` is one of the crates that Fulgur builds on. The 0.0.x releases make no
promise of API stability: any release may change the public API.

- Repository: <https://github.com/fulgur-rs/raikiri>
- License: MIT OR Apache-2.0
