//! Corpus entries of the layout domain (`property/parse/layout.rs`), plus
//! `empty-cells`, which the `properties!` table declares.

use super::{Entry, entry};

/// Samples shared by the four grid line placement properties.
macro_rules! grid_line_entry {
    ($name:literal, $field:ident) => {
        entry! {
            $name, $field,
            parent: "2",
            samples: [
                "auto", "3", "-1", "span 2", "a", "a 2", "span a", "0", "span 0",
            ],
        }
    };
}

pub(super) const ENTRIES: &[Entry] = &[
    entry! {
        "list-style-type", list_style_type,
        parent: "square",
        samples: [
            "disc",
            "none",
            "decimal",
            "lower-roman",
            "\"-\"",
            "symbols(cyclic \"*\")",
            "my-style",
            "123",
        ],
    },
    entry! {
        "list-style-position", list_style_position,
        parent: "inside",
        samples: ["outside", "inside", "middle"],
    },
    entry! {
        "list-style-image", list_style_image,
        parent: "url(p.png)",
        samples: [
            "none",
            "url(a.png)",
            "linear-gradient(red, blue)",
            "linear-gradient(red 1em, blue)",
            "bogus",
        ],
    },
    entry! {
        "flex-direction", flex_direction,
        parent: "column",
        samples: ["row", "row-reverse", "column-reverse", "up"],
    },
    entry! {
        "flex-wrap", flex_wrap,
        parent: "wrap",
        samples: ["nowrap", "wrap-reverse", "both"],
    },
    entry! {
        "flex-grow", flex_grow,
        parent: "2",
        samples: ["0", "1.5", "-1", "calc(1 + 1)", "1px"],
    },
    entry! {
        "flex-shrink", flex_shrink,
        parent: "2",
        samples: ["0", "1.5", "-1", "calc(1 + 1)", "1px"],
    },
    entry! {
        "flex-basis", flex_basis,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "content", "100px", "0", "-1px", "min-content"],
    },
    entry! {
        "order", order,
        parent: "2",
        samples: ["0", "-3", "1.5", "calc(2 * 2)"],
    },
    entry! {
        "justify-content", justify_content,
        parent: "center",
        samples: [
            "normal",
            "flex-start",
            "space-between",
            "safe center",
            "stretch",
            "left",
            "baseline",
        ],
    },
    entry! {
        "align-content", align_content,
        parent: "center",
        samples: [
            "normal",
            "flex-end",
            "space-around",
            "baseline",
            "unsafe end",
            "left",
        ],
    },
    entry! {
        "align-items", align_items,
        parent: "center",
        samples: [
            "normal",
            "stretch",
            "flex-start",
            "baseline",
            "last baseline",
            "safe end",
            "legacy",
            "auto",
        ],
    },
    entry! {
        "justify-items", justify_items,
        parent: "center",
        samples: [
            "normal",
            "stretch",
            "start",
            "left",
            "legacy",
            "legacy left",
            "auto",
        ],
    },
    entry! {
        "align-self", align_self,
        parent: "center",
        samples: [
            "auto",
            "normal",
            "stretch",
            "flex-end",
            "baseline",
            "unsafe center",
            "left",
        ],
    },
    entry! {
        "justify-self", justify_self,
        parent: "center",
        samples: [
            "auto",
            "normal",
            "start",
            "left",
            "right",
            "baseline",
            "space-between",
        ],
    },
    entry! {
        "row-gap", row_gap,
        parent: "2em",
        lengths: "$",
        samples: ["normal", "0", "10px", "-1px", "auto"],
    },
    entry! {
        "column-gap", column_gap,
        parent: "2em",
        lengths: "$",
        samples: ["normal", "0", "10px", "-1px", "auto"],
    },
    entry! {
        "grid-template-columns", grid_template_columns,
        parent: "2em 1fr",
        lengths: "$ 1fr",
        samples: [
            "none",
            "100px 1fr",
            "repeat(2, 10px)",
            "minmax(10px, 1fr) auto",
            "[a] 10px [b]",
            "repeat(auto-fill, 10px)",
            "subgrid",
            "fit-content(10px)",
            "-1px",
        ],
    },
    entry! {
        "grid-template-rows", grid_template_rows,
        parent: "2em",
        lengths: "$",
        samples: ["none", "100px auto", "repeat(3, 1fr)", "bogus"],
    },
    entry! {
        "grid-template-areas", grid_template_areas,
        parent: "\"a b\"",
        samples: ["none", "\"a a\" \"b c\"", "\"a .\"", "\"a\" \"b c\"", "a"],
    },
    entry! {
        "grid-auto-columns", grid_auto_columns,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "1fr", "10px 20px", "minmax(10px, auto)", "none"],
    },
    entry! {
        "grid-auto-rows", grid_auto_rows,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "1fr", "10px 20px", "minmax(10px, auto)", "none"],
    },
    entry! {
        "grid-auto-flow", grid_auto_flow,
        parent: "column",
        samples: [
            "row",
            "dense",
            "row dense",
            "column dense",
            "dense column",
            "row column",
        ],
    },
    grid_line_entry!("grid-row-start", grid_row_start),
    grid_line_entry!("grid-row-end", grid_row_end),
    grid_line_entry!("grid-column-start", grid_column_start),
    grid_line_entry!("grid-column-end", grid_column_end),
    entry! {
        "orphans", orphans,
        parent: "3",
        samples: ["1", "2", "0", "-1", "2.5"],
    },
    entry! {
        "widows", widows,
        parent: "3",
        samples: ["1", "2", "0", "-1", "2.5"],
    },
    entry! {
        "break-before", break_before,
        parent: "page",
        samples: [
            "auto",
            "avoid",
            "always",
            "left",
            "right",
            "recto",
            "verso",
            "avoid-page",
            "column",
            "avoid-column",
            "region",
            "bogus",
        ],
    },
    entry! {
        "break-after", break_after,
        parent: "page",
        samples: [
            "auto",
            "avoid",
            "always",
            "left",
            "right",
            "recto",
            "verso",
            "avoid-page",
            "column",
            "avoid-column",
            "region",
            "bogus",
        ],
    },
    entry! {
        "break-inside", break_inside,
        parent: "avoid",
        samples: ["auto", "avoid-page", "avoid-column", "avoid-region", "page"],
    },
    entry! {
        "float", float,
        parent: "left",
        samples: ["none", "right", "inline-start", "inline-end", "center"],
    },
    entry! {
        "clear", clear,
        parent: "both",
        samples: ["none", "left", "right", "inline-start", "both", "all"],
    },
    entry! {
        "table-layout", table_layout,
        parent: "fixed",
        samples: ["auto", "fixed", "none"],
    },
    entry! {
        "border-collapse", border_collapse,
        parent: "collapse",
        samples: ["separate", "collapse", "none"],
    },
    entry! {
        "border-spacing", border_spacing,
        parent: "2em 1em",
        lengths: "$",
        samples: ["0", "1px 2px", "1px 2px 3px", "-1px", "1px 10%"],
    },
    entry! {
        "caption-side", caption_side,
        parent: "bottom",
        samples: ["top", "bottom", "left", "block-end"],
    },
    entry! {
        "column-count", column_count,
        parent: "3",
        samples: ["auto", "1", "0", "-2", "2.5"],
    },
    entry! {
        "column-width", column_width,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "100px", "0", "-1px"],
    },
    entry! {
        "empty-cells", empty_cells,
        parent: "hide",
        samples: ["show", "hide", "collapse"],
    },
];
