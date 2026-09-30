//! Corpus entries of the content domain (`property/parse/content.rs`).

use super::{Entry, entry};

pub(super) const ENTRIES: &[Entry] = &[
    entry! {
        "counter-increment", counter_increment,
        parent: "a 2",
        samples: ["none", "a", "a 3 b", "a -1", "3", "a calc(1 + 1)"],
    },
    entry! {
        "counter-set", counter_set,
        parent: "a 2",
        samples: ["none", "a", "a 3 b", "a b 2", "3"],
    },
    entry! {
        "content", content,
        parent: "\"p\"",
        samples: [
            "normal",
            "none",
            "\"x\"",
            "counter(a)",
            "counters(a, \".\", upper-roman)",
            "attr(title)",
            "open-quote close-quote",
            "\"a\" counter(b, upper-roman)",
            "url(x.png)",
            "bogus",
        ],
    },
    entry! {
        "string-set", string_set,
        parent: "title \"p\"",
        samples: [
            "none",
            "title content(text)",
            "a \"x\", b attr(title)",
            "title content(before) \"-\" counter(page)",
            "title",
        ],
    },
    // `page` has no computed field: the cascade keeps its value in a side
    // slot for page-name propagation.
    entry! {
        "page", _,
        parent: "wide",
        samples: ["auto", "wide", "narrow", "1"],
    },
];
