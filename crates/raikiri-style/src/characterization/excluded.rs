//! Properties with a `PropertyKey` that the corpus deliberately leaves out.
//!
//! The corpus covers the longhands that can move into the `properties!`
//! table one field per entry. The rows below are shorthands, or longhands
//! whose value is stored in or computed from more than their own field, so
//! they stay hand-written and are characterized by their own tests.

/// `(property name, reason)`, one row per excluded `PropertyKey`. Aliases of
/// an excluded key (`inline-size`, logical margins and paddings, ...) need no
/// row of their own.
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
