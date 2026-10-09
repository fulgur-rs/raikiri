//! Seeded generator of documents and stylesheets.
//!
//! A case is a pure function of its seed: the same seed always produces the
//! same document and the same stylesheets, so a case that differs between two
//! builds can be reproduced from its id alone. The generator aims for breadth
//! rather than validity — declarations the parser rejects are simply dropped —
//! and it deliberately repeats sibling structures and inline styles so the
//! cascade's sharing and caching paths run alongside the ordinary ones.

use raikiri_style::{Origin, StyleQuirksMode};

use crate::doc::{GenDoc, GenNode, SVG_NAMESPACE};

/// xorshift64* pseudo-random generator.
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator for `seed`. Distinct seeds give distinct streams.
    pub(crate) fn new(seed: u64) -> Self {
        Self((seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..n`. `n` must be non-zero.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// True with probability `percent / 100`.
    pub(crate) fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    /// A uniformly chosen element of `items`, which must be non-empty.
    pub(crate) fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// One generated case: a document and its stylesheets in cascade order.
pub(crate) struct Case {
    pub(crate) doc: GenDoc,
    pub(crate) sheets: Vec<(String, Origin)>,
}

/// Generates the case for `seed`.
pub(crate) fn generate(seed: u64) -> Case {
    let mut rng = Rng::new(seed);
    let quirks = match rng.below(10) {
        0 => StyleQuirksMode::Quirks,
        1 => StyleQuirksMode::LimitedQuirks,
        _ => StyleQuirksMode::NoQuirks,
    };
    let mut doc = GenDoc::new(quirks);
    let mut budget = 12 + rng.below(60);
    if rng.chance(85) {
        let html = doc.append(0, element(&mut rng, "html"));
        let body = doc.append(html, element(&mut rng, "body"));
        grow(&mut rng, &mut doc, body, 0, &mut budget);
    } else {
        // Several elements directly under the Document node: each one acts as
        // its own root element.
        for _ in 0..1 + rng.below(3) {
            let tag = rng.pick(FLOW_TAGS);
            let top = doc.append(0, element(&mut rng, tag));
            grow(&mut rng, &mut doc, top, 1, &mut budget);
        }
    }
    let mut sheets = vec![(stylesheet(&mut rng), Origin::Author)];
    if rng.chance(30) {
        sheets.insert(0, (stylesheet(&mut rng), Origin::UserAgent));
    }
    if rng.chance(60) {
        // Block-level display for the structural elements, as a user agent
        // stylesheet gives them, so most cases have block formatting contexts
        // (and the `::first-line` entry point has a block to start from).
        sheets.insert(0, (BASIC_DISPLAY.to_owned(), Origin::UserAgent));
    }
    if rng.chance(20) {
        sheets.push((stylesheet(&mut rng), Origin::User));
    }
    if rng.chance(30) {
        sheets.push((stylesheet(&mut rng), Origin::Author));
    }
    Case { doc, sheets }
}

/// A readable listing of a generated case: its stylesheets, then its tree in
/// an HTML-like notation with one node per line.
pub(crate) fn describe(case: &Case) -> String {
    let mut out = String::new();
    for (source, origin) in &case.sheets {
        out.push_str(&format!("/* {origin:?} */\n{source}\n"));
    }
    out.push_str(&format!("<!-- {:?} -->\n", case.doc.quirks));
    let mut stack = vec![(0usize, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let node = &case.doc.nodes[id];
        let indent = "  ".repeat(depth);
        let detached = if node.in_document { "" } else { " (detached)" };
        match node.kind {
            raikiri_style::StyleNodeKind::Element => {
                let mut attrs = String::new();
                for (name, value) in &node.attrs {
                    attrs.push_str(&format!(" {name}={value:?}"));
                }
                if let Some(style) = &node.style {
                    attrs.push_str(&format!(" style={style:?}"));
                }
                if let Some(animation) = &node.animation {
                    attrs.push_str(&format!(" animation-style={animation:?}"));
                }
                out.push_str(&format!("{indent}[{id}] <{}{attrs}>{detached}\n", node.tag));
            }
            kind => {
                let text = node.text.as_deref().unwrap_or("");
                out.push_str(&format!("{indent}[{id}] {kind:?} {text:?}{detached}\n"));
            }
        }
        for &child in node.children.iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    out
}

const BASIC_DISPLAY: &str =
    "html, body, div, p, section, ul, ol, blockquote, pre, h1 { display: block }
li { display: list-item }
table { display: table }
tbody { display: table-row-group }
tr { display: table-row }
td, th { display: table-cell }
";

const FLOW_TAGS: &[&str] = &[
    "div",
    "p",
    "span",
    "section",
    "ul",
    "ol",
    "li",
    "table",
    "a",
    "img",
    "em",
    "strong",
    "blockquote",
    "pre",
    "h1",
    "b",
    "svg",
    "template",
];
const CLASSES: &[&str] = &["c0", "c1", "c2", "c3", "c4", "C1"];
const IDS: &[&str] = &["i0", "i1", "i2", "I0"];
const LANGS: &[&str] = &["en", "en-US", "ja", "fr", ""];
const DIRS: &[&str] = &["ltr", "rtl", "auto", ""];
const DIMENSIONS: &[&str] = &["10", "25%", "0", "abc", "7.5", "-3"];

fn element(rng: &mut Rng, tag: &str) -> GenNode {
    let mut node = GenNode::element(tag);
    if tag == "svg" {
        node.namespace = Some(SVG_NAMESPACE);
        if rng.chance(50) {
            node.attrs
                .push(("opacity".into(), rng.pick(&["0.5", "1", "x"]).into()));
        }
        if rng.chance(50) {
            node.attrs.push((
                "background-color".into(),
                rng.pick(&["red", "#0f0", "bogus"]).into(),
            ));
        }
    }
    if rng.chance(45) {
        let classes: Vec<&str> = (0..1 + rng.below(3)).map(|_| rng.pick(CLASSES)).collect();
        node.attrs.push(("class".into(), classes.join(" ")));
    }
    if rng.chance(12) {
        node.attrs.push(("id".into(), rng.pick(IDS).into()));
    }
    if rng.chance(10) {
        node.attrs.push(("lang".into(), rng.pick(LANGS).into()));
    }
    if rng.chance(8) {
        node.attrs.push(("dir".into(), rng.pick(DIRS).into()));
    }
    match tag {
        "a" if rng.chance(60) => node.attrs.push(("href".into(), "#x".into())),
        "img" | "td" | "th" | "table" => {
            for attr in ["width", "height"] {
                if rng.chance(40) {
                    node.attrs.push((attr.into(), rng.pick(DIMENSIONS).into()));
                }
            }
        }
        _ => {}
    }
    if tag == "table" {
        for (attr, values) in [
            ("cellpadding", &["2", "0", "5%"][..]),
            ("cellspacing", &["3", "0"][..]),
            ("border", &["1", "0"][..]),
            ("rules", &["all", "rows", "cols", "none", "groups"][..]),
        ] {
            if rng.chance(35) {
                node.attrs.push((attr.into(), rng.pick(values).into()));
            }
        }
    }
    if rng.chance(25) {
        node.style = Some(inline_style(rng));
    }
    if rng.chance(5) {
        let count = 1 + rng.below(2);
        node.animation = Some(declarations(rng, count, false));
    }
    node
}

/// Inline styles come from a small pool part of the time, so identical
/// sources repeat and the parsed-block cache is exercised.
fn inline_style(rng: &mut Rng) -> String {
    const POOL: &[&str] = &[
        "color: red",
        "margin: 1px 2px; padding: 3px",
        "font-size: 2em; --v0: 7px",
        "display: none",
        "border: 1px solid blue !important",
    ];
    if rng.chance(40) {
        rng.pick(POOL).to_owned()
    } else {
        let count = 1 + rng.below(4);
        declarations(rng, count, true)
    }
}

fn grow(rng: &mut Rng, doc: &mut GenDoc, parent: usize, depth: usize, budget: &mut usize) {
    if depth > 7 {
        return;
    }
    let fanout = if depth < 2 {
        1 + rng.below(6)
    } else {
        rng.below(4)
    };
    for _ in 0..fanout {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        match rng.below(20) {
            0..=2 => {
                doc.append(
                    parent,
                    GenNode::text(rng.pick(&["hello world", " ", "x y z"])),
                );
            }
            3 => {
                doc.append(parent, GenNode::comment("c"));
            }
            4 if rng.chance(50) => {
                // A run of identical siblings, each with an identical child, so
                // sibling and cousin sharing both trigger.
                let tag = rng.pick(&["li", "p", "td", "span"]);
                let template = element(rng, tag);
                let child_tag = rng.pick(&["span", "b", "em"]);
                let child = element(rng, child_tag);
                for _ in 0..3 + rng.below(4) {
                    let id = doc.append(parent, clone_element(&template));
                    doc.append(id, clone_element(&child));
                    doc.append(id, GenNode::text("t"));
                }
            }
            _ => {
                let tag = rng.pick(FLOW_TAGS);
                let node = element(rng, tag);
                let id = doc.append(parent, node);
                if tag == "table" {
                    grow_table(rng, doc, id, budget);
                } else if tag == "svg" {
                    let mut g = GenNode::element("g");
                    g.namespace = Some(SVG_NAMESPACE);
                    doc.append(id, g);
                } else if tag != "img" {
                    grow(rng, doc, id, depth + 1, budget);
                }
                // Template contents are inert: the `<template>` element is in the
                // document, its descendants are not.
                if tag == "template" && rng.chance(90) {
                    detach_descendants(doc, id);
                }
            }
        }
    }
}

fn grow_table(rng: &mut Rng, doc: &mut GenDoc, table: usize, budget: &mut usize) {
    let tbody = doc.append(table, GenNode::element("tbody"));
    for _ in 0..1 + rng.below(3) {
        let tr = doc.append(tbody, element(rng, "tr"));
        for _ in 0..1 + rng.below(3) {
            let tag = rng.pick(&["td", "td", "th"]);
            let cell = doc.append(tr, element(rng, tag));
            doc.append(cell, GenNode::text("cell"));
            *budget = budget.saturating_sub(1);
        }
    }
}

fn detach_descendants(doc: &mut GenDoc, id: usize) {
    let mut stack = doc.nodes[id].children.clone();
    while let Some(child) = stack.pop() {
        doc.nodes[child].in_document = false;
        stack.extend_from_slice(&doc.nodes[child].children);
    }
}

fn clone_element(node: &GenNode) -> GenNode {
    let mut copy = GenNode::element(&node.tag);
    copy.namespace = node.namespace;
    copy.attrs.clone_from(&node.attrs);
    copy.style.clone_from(&node.style);
    copy.animation.clone_from(&node.animation);
    copy
}

fn stylesheet(rng: &mut Rng) -> String {
    let mut css = String::new();
    if rng.chance(20) {
        css.push_str("@layer base, theme;\n");
    }
    for _ in 0..rng.below(26) {
        let rule = match rng.below(40) {
            0..=3 => format!(
                "@layer {} {{ {} }}",
                rng.pick(&["base", "theme", "base.inner"]),
                style_rule(rng)
            ),
            4 => format!("@layer {{ {} }}", style_rule(rng)),
            5 => format!(
                "@media {} {{ {} }}",
                rng.pick(&["print", "screen", "(min-width: 100px)"]),
                style_rule(rng)
            ),
            6 => format!("@supports (display: grid) {{ {} }}", style_rule(rng)),
            7 => format!(
                "{} {{ {}; & {} {{ {} }} }}",
                selector(rng),
                declarations(rng, 2, true),
                compound(rng),
                declarations(rng, 2, true)
            ),
            8 => format!(
                "@page {} {{ {} }}",
                rng.pick(&["", ":first", ":left", "named"]),
                page_declarations(rng)
            ),
            9 if rng.chance(30) => {
                "@counter-style cs1 { system: cyclic; symbols: \"*\" \"+\"; }".to_owned()
            }
            10 if rng.chance(30) => {
                "::highlight(hl) { background-color: yellow; color: red }".to_owned()
            }
            _ => style_rule(rng),
        };
        css.push_str(&rule);
        css.push('\n');
    }
    css
}

fn style_rule(rng: &mut Rng) -> String {
    let count = 1 + rng.below(3);
    let selectors: Vec<String> = (0..count).map(|_| selector(rng)).collect();
    let count = 1 + rng.below(8);
    format!(
        "{} {{ {} }}",
        selectors.join(", "),
        declarations(rng, count, true)
    )
}

fn selector(rng: &mut Rng) -> String {
    let mut out = compound(rng);
    for _ in 0..rng.below(3) {
        let combinator = rng.pick(&[" ", " > ", " + ", " ~ "]);
        out = format!("{out}{combinator}{}", compound(rng));
    }
    if rng.chance(12) {
        out.push_str(rng.pick(&["::before", "::after", "::marker", "::first-line"]));
    }
    out
}

fn compound(rng: &mut Rng) -> String {
    let mut out = String::new();
    if rng.chance(55) {
        out.push_str(rng.pick(&[
            "div", "p", "span", "li", "td", "th", "table", "a", "img", "svg", "em", "body", "html",
            "*",
        ]));
    }
    for _ in 0..rng.below(3) {
        let simple = match rng.below(16) {
            0..=3 => format!(".{}", rng.pick(CLASSES)),
            4 => format!("#{}", rng.pick(IDS)),
            5 => format!("[{}]", rng.pick(&["class", "id", "href", "lang", "width"])),
            6 => format!("[class~={}]", rng.pick(CLASSES)),
            7 => format!(
                ":nth-child({})",
                rng.pick(&["odd", "even", "2n+1", "3", "-n+2"])
            ),
            8 => rng
                .pick(&[
                    ":first-child",
                    ":last-child",
                    ":only-child",
                    ":empty",
                    ":root",
                ])
                .to_owned(),
            9 => format!(":not({})", rng.pick(&[".c0", "p", ":first-child"])),
            10 => format!(
                ":is({}, {})",
                rng.pick(&["p", ".c1"]),
                rng.pick(&["li", "#i0"])
            ),
            11 => format!(":where({})", rng.pick(&["div", ".c2"])),
            12 => format!(":has({})", rng.pick(&["> span", "+ p", ".c3"])),
            13 => format!(":lang({})", rng.pick(&["en", "ja"])),
            14 => format!(":dir({})", rng.pick(&["ltr", "rtl"])),
            _ => rng
                .pick(&[":link", ":nth-of-type(2)", ":first-of-type"])
                .to_owned(),
        };
        out.push_str(&simple);
    }
    if out.is_empty() {
        out.push('*');
    }
    out
}

const LENGTHS: &[&str] = &[
    "0",
    "1px",
    "2.5px",
    "10px",
    "1em",
    "0.5em",
    "2rem",
    "1ch",
    "3ex",
    "50%",
    "-4px",
    "calc(1px + 2em)",
    "calc(50% - 3px)",
    "min(10px, 2em)",
    "max(1em, 5px)",
    "clamp(1px, 2em, 30px)",
    "auto",
    "1lh",
    "1rlh",
    "1vw",
    "1in",
    "2pt",
];
const COLORS: &[&str] = &[
    "red",
    "#123",
    "#abcdef",
    "#0f08",
    "rgb(1, 2, 3)",
    "rgba(10, 20, 30, 0.5)",
    "hsl(120 50% 50%)",
    "transparent",
    "currentcolor",
    "color-mix(in srgb, red, blue)",
];
const WIDE_KEYWORDS: &[&str] = &["inherit", "initial", "unset", "revert", "revert-layer"];
const GENERIC_VALUES: &[&str] = &[
    "none", "auto", "normal", "0", "1", "10px", "red", "bold", "50%", "2",
];

fn length(rng: &mut Rng) -> String {
    if rng.chance(15) {
        var_ref(rng)
    } else {
        rng.pick(LENGTHS).to_owned()
    }
}

fn color(rng: &mut Rng) -> String {
    if rng.chance(15) {
        var_ref(rng)
    } else {
        rng.pick(COLORS).to_owned()
    }
}

fn var_ref(rng: &mut Rng) -> String {
    let name = rng.pick(&["--v0", "--v1", "--v2", "--v3"]);
    if rng.chance(40) {
        format!("var({name}, {})", rng.pick(&["3px", "blue", "1em 2em"]))
    } else {
        format!("var({name})")
    }
}

fn sides(rng: &mut Rng) -> String {
    let count = 1 + rng.below(4);
    (0..count)
        .map(|_| length(rng))
        .collect::<Vec<_>>()
        .join(" ")
}

fn declarations(rng: &mut Rng, count: usize, allow_important: bool) -> String {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut declaration = declaration(rng);
        if allow_important && rng.chance(12) {
            declaration.push_str(" !important");
        }
        out.push(declaration);
    }
    out.join("; ")
}

fn page_declarations(rng: &mut Rng) -> String {
    let property = rng.pick(&["margin", "margin-top", "size", "color", "font-size"]);
    let value = match property {
        "size" => rng
            .pick(&["A4", "letter landscape", "100px 200px"])
            .to_owned(),
        "color" => color(rng),
        _ => length(rng),
    };
    format!("{property}: {value}")
}

fn declaration(rng: &mut Rng) -> String {
    if rng.chance(8) {
        let property = rng.pick(&[
            "color",
            "margin-left",
            "display",
            "font-size",
            "border-top-color",
            "background-color",
            "padding",
        ]);
        return format!("{property}: {}", rng.pick(WIDE_KEYWORDS));
    }
    if rng.chance(1) {
        return format!("all: {}", rng.pick(&["revert-layer", "inherit", "initial"]));
    }
    if rng.chance(15) {
        let names = raikiri_style::property::supported_property_names();
        let property = names[rng.below(names.len())];
        return format!("{property}: {}", rng.pick(GENERIC_VALUES));
    }
    if rng.chance(10) {
        let name = rng.pick(&["--v0", "--v1", "--v2", "--v3"]);
        let value = match rng.below(5) {
            0 => length(rng),
            1 => color(rng),
            2 => format!("var({})", rng.pick(&["--v0", "--v1", "--v2"])),
            3 => "foo bar".to_owned(),
            _ => String::new(),
        };
        return format!("{name}: {value}");
    }
    match rng.below(34) {
        0 => format!(
            "display: {}",
            rng.pick(&[
                "block",
                "inline",
                "inline-block",
                "flex",
                "grid",
                "none",
                "list-item",
                "table",
                "table-cell",
                "contents",
                "flow-root",
                "inline-flex"
            ])
        ),
        1 => format!(
            "position: {}",
            rng.pick(&["static", "relative", "absolute", "fixed", "sticky"])
        ),
        2 => format!(
            "{}: {}",
            rng.pick(&[
                "color",
                "background-color",
                "border-color",
                "outline-color",
                "text-decoration-color"
            ]),
            color(rng)
        ),
        3 => format!(
            "{}: {}",
            rng.pick(&[
                "margin",
                "padding",
                "inset",
                "margin-inline",
                "padding-block"
            ]),
            sides(rng)
        ),
        4 => format!(
            "{}: {}",
            rng.pick(&[
                "margin-top",
                "padding-left",
                "top",
                "left",
                "width",
                "height",
                "min-width",
                "max-height",
                "text-indent",
                "letter-spacing",
                "word-spacing"
            ]),
            length(rng)
        ),
        5 => format!(
            "border: {} {} {}",
            rng.pick(&["1px", "thin", "medium", "3px"]),
            rng.pick(&["solid", "dashed", "none", "double"]),
            color(rng)
        ),
        6 => format!(
            "{}: {} {}",
            rng.pick(&["border-top", "border-left", "outline"]),
            rng.pick(&["2px", "thick"]),
            rng.pick(&["dotted", "solid"])
        ),
        7 => format!(
            "border-radius: {}",
            rng.pick(&["4px", "50%", "1px 2px 3px 4px", "10px / 5px"])
        ),
        8 => format!(
            "font-size: {}",
            rng.pick(&[
                "12px",
                "1.5em",
                "larger",
                "smaller",
                "medium",
                "xx-large",
                "150%",
                "calc(1em + 2px)",
                "2rem"
            ])
        ),
        9 => format!(
            "font-weight: {}",
            rng.pick(&["100", "400", "700", "bold", "bolder", "lighter", "normal"])
        ),
        10 => format!(
            "font-family: {}",
            rng.pick(&["serif", "\"Foo Bar\", sans-serif", "monospace", "system-ui"])
        ),
        11 => format!(
            "font: {}",
            rng.pick(&[
                "italic bold 12px/1.5 serif",
                "16px sans-serif",
                "small-caps 1em monospace",
                "caption"
            ])
        ),
        12 => format!(
            "line-height: {}",
            rng.pick(&["normal", "1.5", "20px", "150%", "2em"])
        ),
        13 => format!(
            "white-space: {}",
            rng.pick(&[
                "normal",
                "pre",
                "pre-wrap",
                "nowrap",
                "pre-line",
                "break-spaces"
            ])
        ),
        14 => format!(
            "text-wrap: {}",
            rng.pick(&["wrap", "nowrap", "balance", "pretty"])
        ),
        15 => format!("text-spacing: {}", rng.pick(&["none", "auto", "normal"])),
        16 => format!(
            "text-align: {}",
            rng.pick(&[
                "left",
                "right",
                "center",
                "justify",
                "start",
                "end",
                "match-parent"
            ])
        ),
        17 => format!(
            "text-decoration: {}",
            rng.pick(&[
                "underline",
                "line-through red",
                "underline wavy blue",
                "none"
            ])
        ),
        18 => format!(
            "text-transform: {}",
            rng.pick(&["uppercase", "lowercase", "capitalize", "none", "full-width"])
        ),
        19 => format!(
            "list-style: {}",
            rng.pick(&[
                "square inside",
                "decimal",
                "none",
                "upper-roman outside",
                "\"-\""
            ])
        ),
        20 => format!(
            "{}: {}",
            rng.pick(&["counter-reset", "counter-increment", "counter-set"]),
            rng.pick(&["c1", "c1 2", "c1 c2 3", "none"])
        ),
        21 => format!(
            "content: {}",
            rng.pick(&[
                "\"x\"",
                "counter(c1)",
                "counters(c1, \".\")",
                "attr(id)",
                "open-quote",
                "none",
                "normal",
                "\"a\" counter(c1, cs1)"
            ])
        ),
        22 => format!("quotes: {}", rng.pick(&["auto", "none", "\"<\" \">\""])),
        23 => format!(
            "background: {}",
            rng.pick(&[
                "red",
                "url(x.png) no-repeat 10px 20px",
                "linear-gradient(red, blue)",
                "none",
                "blue border-box"
            ])
        ),
        24 => format!(
            "{}: {}",
            rng.pick(&[
                "background-repeat",
                "background-size",
                "background-position",
                "background-clip"
            ]),
            rng.pick(&["no-repeat", "cover", "10px 20px", "content-box", "repeat-x"])
        ),
        25 => format!("opacity: {}", rng.pick(&["0", "0.5", "1", "50%"])),
        26 => format!(
            "{}: {}",
            rng.pick(&[
                "visibility",
                "overflow",
                "box-sizing",
                "float",
                "clear",
                "vertical-align"
            ]),
            rng.pick(&[
                "hidden",
                "visible",
                "border-box",
                "left",
                "both",
                "middle",
                "scroll"
            ])
        ),
        27 => format!("flex: {}", rng.pick(&["1", "1 1 auto", "none", "2 0 10px"])),
        28 => format!(
            "{}: {}",
            rng.pick(&[
                "grid-template-columns",
                "grid-auto-rows",
                "grid-auto-columns"
            ]),
            rng.pick(&[
                "1fr 2fr",
                "repeat(3, 10px)",
                "10px",
                "auto",
                "minmax(10px, 1fr)",
                "1em"
            ])
        ),
        29 => format!(
            "{}: {}",
            rng.pick(&["grid-area", "grid-row", "grid-column"]),
            rng.pick(&["1 / 2 / 3 / 4", "1 / 3", "span 2", "auto"])
        ),
        30 => format!(
            "transform: {}",
            rng.pick(&["translate(1px, 2px)", "rotate(10deg)", "scale(2)", "none"])
        ),
        31 => format!(
            "{}: {}",
            rng.pick(&["writing-mode", "direction", "unicode-bidi"]),
            rng.pick(&[
                "vertical-rl",
                "rtl",
                "ltr",
                "isolate",
                "horizontal-tb",
                "bidi-override"
            ])
        ),
        32 => format!(
            "{}: {}",
            rng.pick(&["break-before", "break-after", "break-inside", "page"]),
            rng.pick(&["page", "avoid", "auto", "left", "named"])
        ),
        _ => format!(
            "{}: {}",
            rng.pick(&[
                "border-collapse",
                "border-spacing",
                "caption-side",
                "empty-cells",
                "table-layout"
            ]),
            rng.pick(&["collapse", "2px 3px", "bottom", "hide", "fixed"])
        ),
    }
}

#[cfg(test)]
mod tests;
