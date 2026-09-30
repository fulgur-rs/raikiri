//! Corpus entries of the box_model domain (`property/parse/box_model.rs`).

use super::{Entry, entry};

pub(super) const ENTRIES: &[Entry] = &[
    entry! {
        "box-sizing", box_sizing,
        parent: "border-box",
        samples: ["content-box", "border-box", "padding-box"],
    },
    entry! {
        "z-index", z_index,
        parent: "3",
        samples: ["auto", "0", "-5", "2.5", "calc(1 + 2)"],
    },
    entry! {
        "max-width", max_width,
        parent: "2em",
        lengths: "$",
        samples: ["none", "100px", "min-content", "fit-content", "auto", "-1px"],
    },
    entry! {
        "max-height", max_height,
        parent: "2em",
        lengths: "$",
        samples: ["none", "100px", "max-content", "auto", "-1px"],
    },
    entry! {
        "box-shadow", box_shadow,
        parent: "1px 2px 3px red",
        lengths: "$ 2px 3px red",
        samples: [
            "none",
            "inset 1px 2px red",
            "1px 2px 3px 4px blue, 5px 6px green",
            "1px",
        ],
    },
    entry! {
        "top", top,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "-3px", "0", "none"],
    },
    entry! {
        "right", right,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "-3px", "0", "none"],
    },
    entry! {
        "bottom", bottom,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "-3px", "0", "none"],
    },
    entry! {
        "left", left,
        parent: "2em",
        lengths: "$",
        samples: ["auto", "-3px", "0", "none"],
    },
    entry! {
        "outline-offset", outline_offset,
        parent: "2em",
        lengths: "$",
        samples: ["0", "-2px", "auto"],
    },
];
