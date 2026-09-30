//! Supported property names the corpus deliberately does not record.
//!
//! The corpus covers the longhands that can move into the `properties!`
//! table one field per entry. The names below are shorthands, longhands
//! whose value is stored in or computed from more than their own field, or
//! aliases; they stay hand-written and are characterized by their own tests.
//! `meta.rs` checks that every name `parse_value` dispatches on is in the
//! corpus or in exactly one of these lists.

/// `(property name, reason)` for names left out of the corpus.
pub(super) const EXCLUDED: &[(&str, &str)] = &[
    // Shorthands: expanded into longhands before the cascade, or applied as a
    // group.
    ("background", "shorthand"),
    ("border", "shorthand"),
    ("border-color", "shorthand"),
    ("border-right", "shorthand"),
    ("border-style", "shorthand"),
    ("border-width", "shorthand"),
    ("columns", "shorthand"),
    ("flex", "shorthand"),
    ("flex-flow", "shorthand"),
    ("font", "shorthand"),
    ("gap", "shorthand"),
    ("grid-column", "shorthand"),
    ("grid-row", "shorthand"),
    ("margin", "shorthand"),
    ("margin-block", "shorthand"),
    ("margin-inline", "shorthand"),
    ("outline", "shorthand"),
    ("overflow", "shorthand"),
    ("padding", "shorthand"),
    ("padding-block", "shorthand"),
    ("padding-inline", "shorthand"),
    ("place-content", "shorthand"),
    ("place-items", "shorthand"),
    ("place-self", "shorthand"),
    ("text-decoration", "shorthand"),
    ("text-emphasis", "shorthand"),
    ("text-spacing", "shorthand"),
    (
        "text-wrap",
        "shorthand of text-wrap-mode and text-wrap-style; shares text-wrap-mode's PropertyKey",
    ),
    // Longhands stored in a shared aggregate field (`padding`, `margin`,
    // `border`, `outline`, `overflow`), several of them also coupled to a
    // sibling longhand or carrying `ch` provenance.
    (
        "border-top-width",
        "aggregate field, coupled to border-top-style",
    ),
    (
        "border-right-width",
        "aggregate field, coupled to border-right-style",
    ),
    (
        "border-bottom-width",
        "aggregate field, coupled to border-bottom-style",
    ),
    (
        "border-left-width",
        "aggregate field, coupled to border-left-style",
    ),
    ("border-top-style", "aggregate field"),
    ("border-right-style", "aggregate field"),
    ("border-bottom-style", "aggregate field"),
    ("border-left-style", "aggregate field"),
    ("border-top-color", "aggregate field"),
    ("border-right-color", "aggregate field"),
    ("border-bottom-color", "aggregate field"),
    ("border-left-color", "aggregate field"),
    ("margin-top", "aggregate field with ch provenance"),
    ("margin-right", "aggregate field with ch provenance"),
    ("margin-bottom", "aggregate field with ch provenance"),
    ("margin-left", "aggregate field with ch provenance"),
    ("padding-top", "aggregate field with ch provenance"),
    ("padding-right", "aggregate field with ch provenance"),
    ("padding-bottom", "aggregate field with ch provenance"),
    ("padding-left", "aggregate field with ch provenance"),
    // Flow-relative names of the aggregate longhands: separate parse arms
    // that call the physical side's parser and store into its field.
    ("margin-block-start", "logical alias of margin-top"),
    ("margin-block-end", "logical alias of margin-bottom"),
    ("margin-inline-start", "logical alias of margin-left"),
    ("margin-inline-end", "logical alias of margin-right"),
    ("padding-block-start", "logical alias of padding-top"),
    ("padding-block-end", "logical alias of padding-bottom"),
    ("padding-inline-start", "logical alias of padding-left"),
    ("padding-inline-end", "logical alias of padding-right"),
    ("outline-width", "aggregate field, coupled to outline-style"),
    ("outline-style", "aggregate field"),
    ("outline-color", "aggregate field"),
    ("overflow-x", "aggregate field, computed with overflow-y"),
    ("overflow-y", "aggregate field, computed with overflow-x"),
    // Longhands whose computation needs more than their own value.
    (
        "counter-reset",
        "same name also parses to a CSS-wide marker variant",
    ),
    ("display", "computed together with float"),
    (
        "font-size",
        "resolved against the parent; defines the em basis",
    ),
    ("font-weight", "bolder/lighter resolve against the parent"),
    ("line-height", "defines the lh basis"),
    ("text-align", "resolved against the parent and direction"),
    (
        "text-align-all",
        "no computed field until text-align is split",
    ),
    ("width", "writes ch provenance fields"),
    ("height", "writes ch provenance fields"),
    ("min-width", "swapped with min-block-size by writing-mode"),
    ("min-height", "swapped with min-block-size by writing-mode"),
    (
        "min-block-size",
        "writes min-width or min-height by writing-mode",
    ),
    ("text-indent", "writes several computed fields"),
    ("letter-spacing", "writes several computed fields"),
    ("word-spacing", "writes several computed fields"),
    ("text-decoration-inset", "writes ch provenance fields"),
    ("writing-mode", "writes a second CSSOM field"),
    (
        "text-emphasis-style",
        "default shape depends on writing-mode",
    ),
    ("quotes", "writes quotes_auto as well"),
    ("position", "running() appends to the running templates"),
    ("transform-origin", "two-field value"),
];

/// `(alias, target)`: legacy or logical names that share the target's parse
/// arm (`"target" | "alias" => ..` in `parse_value`), so they parse, cascade
/// and serialize exactly like the target, which is itself in the corpus or in
/// [`EXCLUDED`]. An alias with a parse arm of its own belongs in the corpus
/// or in [`EXCLUDED`] instead.
pub(super) const ALIASES_COVERED_BY_TARGET: &[(&str, &str)] = &[
    ("word-wrap", "overflow-wrap"),
    ("inline-size", "width"),
    ("block-size", "height"),
];

/// `(property name, reason)` for the names `parse_value` accepts that have
/// no `PropertyKey`. `meta.rs` checks that this is exactly the current set of
/// keyless names, so a new one has to be classified here.
pub(super) const KEYLESS: &[(&str, &str)] = &[
    (
        "border-radius",
        "shorthand; expands into the four corner longhands while parsing",
    ),
    (
        "border-top-left-radius",
        "longhand stored in the shared border_radius aggregate; keyed only through its variant",
    ),
    (
        "border-top-right-radius",
        "longhand stored in the shared border_radius aggregate; keyed only through its variant",
    ),
    (
        "border-bottom-right-radius",
        "longhand stored in the shared border_radius aggregate; keyed only through its variant",
    ),
    (
        "border-bottom-left-radius",
        "longhand stored in the shared border_radius aggregate; keyed only through its variant",
    ),
    ("grid", "shorthand applied as a group in apply_value"),
    ("grid-area", "shorthand applied as a group in apply_value"),
    ("grid-gap", "legacy name of the gap shorthand"),
    (
        "grid-row-gap",
        "legacy name of row-gap with its own arm calling row-gap's parser",
    ),
    (
        "grid-column-gap",
        "legacy name of column-gap with its own arm calling column-gap's parser",
    ),
];
