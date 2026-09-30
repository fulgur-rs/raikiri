//! Corpus entries of the visual domain (`property/parse/visual.rs`), plus
//! `isolation`, `opacity`, `object-fit` and `object-position`, which the
//! `properties!` table declares.

use super::{Entry, entry};

pub(super) const ENTRIES: &[Entry] = &[
    entry! {
        "visibility", visibility,
        parent: "hidden",
        samples: ["visible", "hidden", "collapse", "none"],
    },
    entry! {
        "background-attachment", background_attachment,
        parent: "fixed",
        samples: ["scroll", "fixed", "local", "bogus"],
    },
    entry! {
        "mix-blend-mode", mix_blend_mode,
        parent: "multiply",
        samples: [
            "normal",
            "multiply",
            "screen",
            "plus-lighter",
            "color-dodge",
            "bogus",
        ],
    },
    entry! {
        "background-repeat", background_repeat,
        parent: "no-repeat",
        samples: [
            "repeat",
            "repeat-x",
            "repeat-y",
            "space round",
            "no-repeat repeat",
            "round",
            "repeat-x repeat",
            "bogus",
        ],
    },
    entry! {
        "background-clip", background_clip,
        parent: "content-box",
        samples: ["border-box", "padding-box", "content-box", "text", "margin-box"],
    },
    entry! {
        "background-origin", background_origin,
        parent: "content-box",
        samples: ["border-box", "padding-box", "content-box", "text"],
    },
    entry! {
        "clip-path", clip_path,
        parent: "circle(50%)",
        samples: [
            "none",
            "inset(10px)",
            "circle(10px at 1em 2em)",
            "ellipse(10px 20px)",
            "polygon(0 0, 10px 0, 0 10px)",
            "url(#c)",
            "border-box",
            "inset(1em round 2px) padding-box",
            "bogus",
        ],
    },
    entry! {
        "filter", filter,
        parent: "blur(2px)",
        samples: [
            "none",
            "blur(1em)",
            "brightness(50%)",
            "drop-shadow(1px 2px 3px red)",
            "grayscale(1) sepia(0.5)",
            "url(#f)",
            "hue-rotate(90deg)",
            "bogus",
        ],
    },
    entry! {
        "background-size", background_size,
        parent: "2em auto",
        lengths: "$",
        samples: [
            "auto",
            "cover",
            "contain",
            "10px 20px",
            "50% auto",
            "-1px",
            "auto auto",
        ],
    },
    entry! {
        "background-position", background_position,
        parent: "2em 1em",
        lengths: "$",
        samples: [
            "center",
            "left top",
            "right 10px bottom 20%",
            "10px 20px",
            "bottom",
            "1px 2px 3px",
        ],
    },
    entry! {
        "background-image", background_image,
        parent: "url(p.png)",
        samples: [
            "none",
            "url(a.png)",
            "linear-gradient(red, blue)",
            "linear-gradient(to right, red 1em, blue 10%)",
            "radial-gradient(circle 1em, red, blue)",
            "url(a.png), none",
            "bogus",
        ],
    },
    entry! {
        "mask-image", mask_image,
        parent: "url(p.png)",
        samples: [
            "none",
            "url(a.png)",
            "linear-gradient(red, blue)",
            "linear-gradient(to right, red 1em, blue 10%)",
            "bogus",
        ],
    },
    entry! {
        "transform", transform,
        parent: "translate(2em)",
        samples: [
            "none",
            "translate(1em, 10%)",
            "rotate(45deg)",
            "scale(2) translateX(1rem)",
            "matrix(1, 0, 0, 1, 10, 20)",
            "skew(10deg)",
            "translate(calc(1em + 2px))",
            "rotate(45)",
        ],
    },
    entry! {
        "isolation", isolation,
        parent: "isolate",
        samples: ["auto", "isolate", "none"],
    },
    entry! {
        "opacity", opacity,
        parent: "0.5",
        samples: ["0", "1", "0.25", "50%", "2", "-1", "calc(0.5 + 0.1)"],
    },
    entry! {
        "object-fit", object_fit,
        parent: "cover",
        samples: ["fill", "contain", "cover", "none", "scale-down", "bogus"],
    },
    entry! {
        "object-position", object_position,
        parent: "2em 1em",
        lengths: "$",
        samples: ["center", "left top", "right 10px bottom 20%", "10px 20px"],
    },
];
