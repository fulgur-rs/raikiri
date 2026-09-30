//! Corpus entries of the color domain (`property/parse/color.rs`).

use super::{Entry, entry};

pub(super) const ENTRIES: &[Entry] = &[
    entry! {
        "color", color,
        parent: "blue",
        samples: [
            "red",
            "#00ff0080",
            "rgb(1 2 3 / 50%)",
            "hsl(120 100% 50%)",
            "currentcolor",
            "transparent",
            "color-mix(in srgb, red, blue)",
            "rgb(from red r g b)",
            "notacolor",
        ],
    },
    entry! {
        "background-color", background_color,
        parent: "blue",
        samples: [
            "red",
            "#00ff0080",
            "rgb(1 2 3 / 50%)",
            "currentcolor",
            "transparent",
            "color-mix(in srgb, red, blue)",
            "notacolor",
        ],
    },
];
