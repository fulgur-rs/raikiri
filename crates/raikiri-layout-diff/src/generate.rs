//! Generated documents for the layout paths a conformance run reaches rarely.
//!
//! A seed picks, deterministically, a small tree of boxes that mixes the
//! things the layout code has separate paths for: block, flex, grid, table,
//! multi-column, float, inline-block and out-of-flow boxes, vertical writing
//! modes and right-to-left text, overflow, percentages and `auto` sizes, and
//! pages cut by an `@page` size and forced breaks. The documents do not aim
//! at being meaningful or free of conflicting declarations: the point is to
//! give two builds of the layout engine the same awkward input, so every
//! property is chosen independently of the others.
//!
//! The same seed gives the same document in every build of the tool, which
//! `scripts/layout-diff.sh` checks before comparing anything.

use std::fmt::Write as _;

/// Boxes a document holds at most, counting table cells, so that the cost of
/// a case stays small.
const BOX_BUDGET: usize = 18;

/// Levels of nesting below which a box holds only text.
const MAX_DEPTH: usize = 4;

/// Words the text of a box is made of. The font that lays them out has
/// square glyphs, so a word's width is its length.
const WORDS: [&str; 8] = [
    "a",
    "bb",
    "ccc ddd",
    "eeee",
    "ff gg hh",
    "iiiiiiiiii",
    "jj kk",
    "l m n o p",
];

/// A declaration the generator may add, and how often.
struct Property {
    name: &'static str,
    values: &'static [&'static str],
    /// The chance, in percent, that a box gets the property.
    percent: usize,
}

const fn property(name: &'static str, values: &'static [&'static str], percent: usize) -> Property {
    Property {
        name,
        values,
        percent,
    }
}

/// Properties of any box.
const BOX: &[Property] = &[
    property(
        "width",
        &[
            "auto",
            "30px",
            "60px",
            "90px",
            "50%",
            "25%",
            "10em",
            "fit-content",
            "min-content",
            "max-content",
        ],
        40,
    ),
    property(
        "height",
        &["auto", "20px", "50px", "80px", "30%", "5em"],
        25,
    ),
    property("min-width", &["0", "40px", "20%", "min-content"], 10),
    property("max-width", &["none", "70px", "60%", "fit-content"], 10),
    property("min-height", &["0", "30px", "10%"], 10),
    property("max-height", &["none", "40px", "50%"], 10),
    property(
        "margin",
        &[
            "0",
            "4px",
            "8px 2px",
            "auto",
            "0 auto",
            "-4px",
            "2%",
            "3px 0 0 9px",
        ],
        30,
    ),
    property(
        "padding",
        &["0", "3px", "6px 2px", "5%", "1px 4px 7px 0"],
        30,
    ),
    property(
        "border",
        &[
            "0",
            "2px solid",
            "1px dashed",
            "3px double",
            "4px solid 0 2px",
        ],
        25,
    ),
    property("box-sizing", &["border-box", "content-box"], 15),
    property(
        "overflow",
        &["hidden", "scroll", "auto", "visible", "clip", "hidden auto"],
        12,
    ),
    property("float", &["left", "right", "inline-start", "none"], 8),
    property("clear", &["left", "right", "both", "none"], 6),
    property(
        "writing-mode",
        &["vertical-rl", "vertical-lr", "horizontal-tb", "sideways-rl"],
        14,
    ),
    property("direction", &["rtl", "ltr"], 8),
    property("text-orientation", &["mixed", "upright", "sideways"], 4),
    property(
        "text-align",
        &["left", "right", "center", "justify", "end"],
        12,
    ),
    property("line-height", &["10px", "16px", "1.5", "normal"], 10),
    property("font-size", &["8px", "10px", "14px", "2em"], 8),
    property("white-space", &["nowrap", "pre", "pre-wrap", "normal"], 8),
    property(
        "vertical-align",
        &["top", "middle", "bottom", "baseline", "-3px"],
        8,
    ),
    property("break-before", &["page", "avoid", "column", "auto"], 6),
    property("break-inside", &["avoid", "auto"], 6),
    property(
        "transform",
        &["translate(3px, 4px)", "rotate(5deg)", "none"],
        4,
    ),
];

/// Properties that place a box out of the normal flow.
const POSITION: &[Property] = &[
    property(
        "position",
        &["relative", "absolute", "fixed", "sticky", "static"],
        30,
    ),
    property("top", &["auto", "0", "5px", "10%", "-3px"], 40),
    property("left", &["auto", "0", "5px", "10%", "-3px"], 40),
    property("right", &["auto", "0", "7px", "10%"], 30),
    property("bottom", &["auto", "0", "7px", "10%"], 30),
];

/// Properties of a box that is an item of a flex or grid container.
const ITEM: &[Property] = &[
    property("flex", &["1", "0 0 40px", "2 1 auto", "none", "1 1 0"], 30),
    property("order", &["-1", "1", "2"], 15),
    property(
        "align-self",
        &["start", "end", "center", "stretch", "baseline"],
        20,
    ),
    property("justify-self", &["start", "end", "center", "stretch"], 10),
    property("grid-column", &["1 / 3", "2", "span 2", "1 / -1"], 25),
    property("grid-row", &["1 / 3", "2", "span 2", "auto"], 20),
    property("align-content", &["center", "space-between", "end"], 6),
    property("z-index", &["-1", "1", "2"], 8),
    property("visibility", &["hidden", "collapse", "visible"], 4),
];

const FLEX: &[Property] = &[
    property(
        "flex-direction",
        &["row", "column", "row-reverse", "column-reverse"],
        60,
    ),
    property("flex-wrap", &["wrap", "nowrap", "wrap-reverse"], 40),
    property(
        "justify-content",
        &[
            "center",
            "space-between",
            "space-around",
            "end",
            "flex-start",
        ],
        40,
    ),
    property(
        "align-items",
        &["center", "end", "stretch", "baseline", "flex-start"],
        40,
    ),
    property("align-content", &["center", "space-between", "stretch"], 20),
    property("gap", &["0", "4px", "2px 6px", "5%"], 40),
];

const GRID: &[Property] = &[
    property(
        "grid-template-columns",
        &[
            "1fr 1fr",
            "40px auto",
            "repeat(2, 1fr)",
            "50% 50%",
            "minmax(10px, 1fr) 30px",
        ],
        80,
    ),
    property(
        "grid-template-rows",
        &["auto auto", "20px 1fr", "repeat(2, minmax(10px, auto))"],
        40,
    ),
    property("grid-auto-flow", &["row", "column", "row dense"], 30),
    property("grid-auto-rows", &["10px", "auto", "minmax(8px, auto)"], 30),
    property("gap", &["0", "4px", "2px 6px"], 40),
    property("justify-items", &["start", "center", "stretch"], 25),
    property("align-items", &["start", "center", "end", "stretch"], 25),
];

const COLUMNS: &[Property] = &[
    property("column-count", &["2", "3", "auto"], 80),
    property("column-width", &["40px", "80px", "auto"], 30),
    property("column-gap", &["0", "6px", "normal"], 50),
    property("column-rule", &["1px solid", "none", "3px dotted"], 30),
    property("column-fill", &["balance", "auto"], 40),
    property("height", &["60px", "auto", "100px"], 50),
];

/// A small, fast, deterministic generator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Seeds are small consecutive numbers; one step spreads them.
        let mut rng = Self(seed ^ 0x5851_f42d_4c95_7f2d);
        rng.next_u64();
        rng
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `bound`.
    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    /// Whether an event of the given chance in percent happens.
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a>(&mut self, options: &[&'a str]) -> &'a str {
        options[self.below(options.len())]
    }
}

struct Generator {
    rng: Rng,
    budget: usize,
    out: String,
}

/// The HTML document seed `seed` stands for.
pub(crate) fn generate(seed: u64) -> String {
    let mut generator = Generator {
        rng: Rng::new(seed),
        budget: BOX_BUDGET,
        out: String::new(),
    };
    generator.document();
    generator.out
}

impl Generator {
    fn document(&mut self) {
        self.out
            .push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\"><style>\n");
        if self.rng.chance(50) {
            let width = 160 + self.rng.below(240);
            let height = 120 + self.rng.below(180);
            let margin = self.rng.pick(&["0", "8px", "16px 24px", "5%"]);
            let _ = writeln!(
                self.out,
                "@page {{ size: {width}px {height}px; margin: {margin} }}"
            );
        }
        let margin = self.rng.pick(&["0", "8px", "4px 16px"]);
        let _ = writeln!(
            self.out,
            "body {{ margin: {margin}; font: 10px/12px Ahem }}"
        );
        self.out.push_str("</style></head><body>\n");
        let roots = 1 + self.rng.below(3);
        for _ in 0..roots {
            self.element(0);
        }
        self.out.push_str("</body></html>\n");
    }

    /// The `name:value;` declarations of the properties of `pool` this box
    /// gets.
    fn declarations(&mut self, pool: &[Property]) -> String {
        let mut text = String::new();
        for property in pool {
            if self.rng.chance(property.percent) {
                let value = self.rng.pick(property.values);
                let _ = write!(text, "{}:{value};", property.name);
            }
        }
        text
    }

    /// The `style` attribute of a box with the properties of `pools`.
    fn style(&mut self, display: &str, pools: &[&[Property]]) -> String {
        let mut style = format!("display:{display};");
        for pool in pools {
            style.push_str(&self.declarations(pool));
        }
        style
    }

    /// Text of one to five words, followed by a space.
    fn text(&mut self) {
        let count = 1 + self.rng.below(5);
        for _ in 0..count {
            let word = self.rng.pick(&WORDS);
            self.out.push_str(word);
            self.out.push(' ');
        }
    }

    /// The children of a container: boxes, until the box budget runs out, or
    /// text.
    fn children(&mut self, depth: usize) {
        let count = 1 + self.rng.below(4);
        for _ in 0..count {
            self.element(depth + 1);
        }
    }

    fn element(&mut self, depth: usize) {
        if self.budget == 0 || depth >= MAX_DEPTH {
            self.text();
            return;
        }
        self.budget -= 1;
        let pools: [&[Property]; 3] = [BOX, POSITION, ITEM];
        match self.rng.below(12) {
            0..=3 => {
                let style = self.style("block", &pools);
                let _ = write!(self.out, "<div style=\"{style}\">");
                self.children(depth);
                self.out.push_str("</div>");
            }
            4 | 5 => {
                let style = self.style("flex", &[BOX, POSITION, ITEM, FLEX]);
                let _ = write!(self.out, "<div style=\"{style}\">");
                self.children(depth);
                self.out.push_str("</div>");
            }
            6 | 7 => {
                let style = self.style("grid", &[BOX, POSITION, ITEM, GRID]);
                let _ = write!(self.out, "<div style=\"{style}\">");
                self.children(depth);
                self.out.push_str("</div>");
            }
            8 => self.table(depth),
            9 => self.columns(depth),
            10 => {
                let display = self
                    .rng
                    .pick(&["inline-block", "inline-flex", "inline-grid"]);
                let style = self.style(display, &pools);
                let _ = write!(self.out, "<span style=\"{style}\">");
                self.children(depth);
                self.out.push_str("</span>");
            }
            _ => {
                let style = self.style("inline", &[BOX, POSITION]);
                self.text();
                let _ = write!(self.out, " <span style=\"{style}\">");
                self.text();
                self.out.push_str("</span> ");
                self.text();
            }
        }
    }

    fn table(&mut self, depth: usize) {
        let style = self.style("table", &[BOX, POSITION]);
        let spacing = self.rng.pick(&["0", "2px", "4px 1px"]);
        let collapse = self.rng.pick(&["collapse", "separate"]);
        let _ = write!(
            self.out,
            "<table style=\"{style}border-spacing:{spacing};border-collapse:{collapse}\">"
        );
        if self.rng.chance(25) {
            self.out.push_str("<caption>");
            self.text();
            self.out.push_str("</caption>");
        }
        let rows = 1 + self.rng.below(3);
        let columns = 1 + self.rng.below(3);
        for _ in 0..rows {
            self.out.push_str("<tr>");
            for _ in 0..columns {
                self.budget = self.budget.saturating_sub(1);
                let style = self.style("table-cell", &[BOX]);
                let span = self.rng.pick(&["", " colspan=2", " rowspan=2"]);
                let _ = write!(self.out, "<td{span} style=\"{style}\">");
                self.element(depth + 1);
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>");
        }
        self.out.push_str("</table>");
    }

    fn columns(&mut self, depth: usize) {
        let style = self.style("block", &[BOX, POSITION, COLUMNS]);
        let _ = write!(self.out, "<div style=\"{style}\">");
        self.children(depth);
        if self.rng.chance(30) {
            self.out
                .push_str("<div style=\"column-span:all;margin:3px 0\">");
            self.text();
            self.out.push_str("</div>");
        }
        self.children(depth);
        self.out.push_str("</div>");
    }
}

#[cfg(test)]
mod tests;
