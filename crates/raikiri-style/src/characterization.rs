//! Characterization corpus: a recording of how the current code parses,
//! serializes and computes a set of longhands, one `insta` snapshot per
//! property domain (the domain names follow `property/parse/`).
//!
//! The snapshots pin today's behavior, whatever it is: a line that looks
//! wrong is recorded as-is, not fixed here. Changing how a property is
//! declared or computed (for example moving it into the `properties!`
//! table) must leave every line unchanged, or show the difference as a
//! snapshot diff to be reviewed.
//!
//! For every [`Entry`] the harness expands a list of sample inputs: the
//! entry's own samples, the shared length samples substituted into the
//! entry's length template, the CSS-wide keywords, and two `var()` forms.
//! Each sample is recorded at these stages, one line each, in the form
//! `<property>: <sample> | <stage> | <value>`:
//!
//! - `parse`: [`parse_value`] followed by an exhaustion check, as the
//!   declaration parser does (`Some(Debug)` or `None`);
//! - `serialize`: [`serialize_value`] of the parsed value (`-` when parsing
//!   failed);
//! - `computed`: the computed field of an element that declares the sample,
//!   after the real cascade over a small document (see [`element_fixture`]);
//! - `cssom`: [`ComputedProperty::serialize`] of that element, when
//!   [`ComputedProperty::from_name`] supports the property;
//! - `page`: the value `cascade_page` stores for the property when an
//!   `@page` rule declares the sample.
//!
//! Each property also records the parent's computed value and, for an
//! element and for a page context that do not declare the property, the
//! computed value, CSSOM string and page value they end up with (inheritance
//! or initial value). Stages that do not apply to a property (no computed
//! field, no CSSOM support, no `PropertyKey`) are recorded once as a marker
//! line instead of being omitted.
//!
//! Known limits of what the lines prove:
//!
//! - Many keyword and length properties have no [`serialize_value`] arm
//!   today (their `serialize` lines read `None`), and some have no
//!   [`ComputedProperty`] either. For them those stages are vacuous: adding a
//!   serializer or CSSOM support later is expected to show up as a snapshot
//!   diff, which is then the intended change, not a regression.
//! - `!important` is not sampled: the parse stage stops at the value and does
//!   not run `parse_important`.

use std::fmt::Write as _;

use cssparser::{Parser, ParserInput};

use crate::cascade::cascade;
use crate::computed::{ChFontKey, ComputedProperty, ComputedValues};
use crate::page::{PageContextQuery, PageInheritance, cascade_page};
use crate::property::{
    PropertyKey, PropertyValue, parse_value, property_key_for_name, serialize_value,
};
use crate::ruletree::{Origin, RuleTree, build_rule_tree};
use crate::test_dom::TestDoc;

mod box_model;
mod color;
mod content;
mod excluded;
mod layout;
mod meta;
mod text;
mod visual;

/// Reads one computed field of an element and formats it on one line.
type ComputedField = fn(&ComputedValues) -> String;

/// One property of the corpus.
struct Entry {
    /// The CSS property name.
    name: &'static str,
    /// A valid, non-initial value declared on the parent element and on the
    /// page's inheritance root, and bound to `--v`; `inherit` and
    /// `var(--v)` resolve against it.
    parent: &'static str,
    /// Where the shared [`LENGTHS`] go: every `$` in the template is replaced
    /// by one length sample. `None` for a property without lengths.
    lengths: Option<&'static str>,
    /// The property's own samples: typical valid values and invalid ones.
    samples: &'static [&'static str],
    /// The element's computed field for this property, or `None` when the
    /// property has no computed field.
    computed: Option<ComputedField>,
}

/// Declares an [`Entry`] whose computed field is `ComputedValues::$field`
/// (fields of the `properties!` table are reached through `Deref`), or which
/// has no computed field when `$field` is `_`.
macro_rules! entry {
    (
        $name:literal, _,
        parent: $parent:literal,
        $(lengths: $lengths:literal,)?
        samples: [$($sample:literal),* $(,)?] $(,)?
    ) => {
        $crate::characterization::Entry {
            name: $name,
            parent: $parent,
            lengths: entry!(@lengths $($lengths)?),
            samples: &[$($sample),*],
            computed: None,
        }
    };
    (
        $name:literal, $field:ident,
        parent: $parent:literal,
        $(lengths: $lengths:literal,)?
        samples: [$($sample:literal),* $(,)?] $(,)?
    ) => {
        $crate::characterization::Entry {
            name: $name,
            parent: $parent,
            lengths: entry!(@lengths $($lengths)?),
            samples: &[$($sample),*],
            computed: Some(
                (|computed: &$crate::computed::ComputedValues| {
                    format!("{:?}", computed.$field)
                }) as $crate::characterization::ComputedField,
            ),
        }
    };
    (@lengths) => { None };
    (@lengths $lengths:literal) => { Some($lengths) };
}
use entry;

/// Every domain of the corpus with its entries, in snapshot order.
const DOMAINS: &[(&str, &[Entry])] = &[
    ("box_model", box_model::ENTRIES),
    ("color", color::ENTRIES),
    ("content", content::ENTRIES),
    ("layout", layout::ENTRIES),
    ("text", text::ENTRIES),
    ("visual", visual::ENTRIES),
];

/// Length samples substituted into an entry's length template. They cover
/// the font-relative units against distinct bases (see [`element_fixture`]),
/// a percentage, and `calc()` with and without a percentage.
const LENGTHS: &[&str] = &[
    "1em",
    "1rem",
    "10%",
    "2ch",
    "1lh",
    "1rlh",
    "calc(1em + 2px)",
    "calc(10% + 1em)",
];

/// CSS-wide keywords every entry is sampled with.
const CSS_WIDE: &[&str] = &["inherit", "initial", "unset", "revert", "revert-layer"];

/// `var()` samples: `--v` is bound to the entry's parent value on the parent
/// element and in the `@page` rule; `--undefined` is never bound.
const VARS: &[&str] = &["var(--v)", "var(--undefined)"];

/// The advance of `0` reported to [`ComputedProperty::serialize`] for lengths
/// authored in `ch`; the style layer has no font data of its own.
const CH_ADVANCE_PX: f32 = 8.0;

/// The page context's own font metrics, distinct from every element basis.
const PAGE_FONT: &str = "font-size: 12px; line-height: 18px";

impl Entry {
    /// The full, ordered sample list of this entry.
    fn all_samples(&self) -> Vec<String> {
        let mut samples: Vec<String> = self.samples.iter().map(|s| (*s).to_owned()).collect();
        if let Some(template) = self.lengths {
            samples.extend(LENGTHS.iter().map(|length| template.replace('$', length)));
        }
        samples.extend(CSS_WIDE.iter().map(|s| (*s).to_owned()));
        samples.extend(VARS.iter().map(|s| (*s).to_owned()));
        samples
    }
}

/// The computed values of one run of the element fixture.
struct ElementRun {
    /// The parent element, which declares [`Entry::parent`].
    parent: ComputedValues,
    /// The child that declares the sample (or nothing, for the baseline run).
    declared: ComputedValues,
    /// The child's sibling, which declares nothing for the property.
    undeclared: ComputedValues,
}

/// Runs the real cascade over
///
/// ```text
/// html                                  (no style: rem = 16px, rlh = normal)
/// └─ div  font-size: 20px; line-height: 30px; --v: P; <name>: P
///    ├─ p font-size: 10px; line-height: 15px; <name>: <sample>
///    └─ p font-size: 10px; line-height: 15px
/// ```
///
/// where `P` is the entry's parent value, so `em`, `rem`, `lh` and the
/// inherited parent value each resolve against a different basis.
fn element_fixture(entry: &Entry, sample: Option<&str>) -> ElementRun {
    let name = entry.name;
    let parent_value = entry.parent;
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let parent_style =
        format!("font-size: 20px; line-height: 30px; --v: {parent_value}; {name}: {parent_value}");
    let parent = doc.push_element(html, "div", Some(&parent_style));
    let own_font = "font-size: 10px; line-height: 15px";
    let declared_style = match sample {
        Some(sample) => format!("{own_font}; {name}: {sample}"),
        None => own_font.to_owned(),
    };
    let declared = doc.push_element(parent, "p", Some(&declared_style));
    let undeclared = doc.push_element(parent, "p", Some(own_font));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    ElementRun {
        parent: result.computed[parent].clone(),
        declared: result.computed[declared].clone(),
        undeclared: result.computed[undeclared].clone(),
    }
}

/// Parses `text` as the value of `name` the way the declaration parser does:
/// the value must consume the whole input.
fn parse(name: &str, text: &str) -> Result<PropertyValue, String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let Some(value) = parse_value(name, &mut parser) else {
        return Err("None".to_owned());
    };
    match parser.expect_exhausted() {
        Ok(()) => Ok(value),
        Err(_) => Err(format!("None (input left after {value:?})")),
    }
}

/// The value `cascade_page` stores for `key` when an `@page` rule declares
/// `sample` (or does not declare the property, for `None`), inheriting from
/// `root`.
fn page_value(
    entry: &Entry,
    key: PropertyKey,
    sample: Option<&str>,
    root: &ComputedValues,
) -> String {
    let mut tree = RuleTree::empty();
    let declaration = match sample {
        Some(sample) => format!("; {}: {sample}", entry.name),
        None => String::new(),
    };
    let css = format!(
        "@page {{ {PAGE_FONT}; --v: {}{declaration} }}",
        entry.parent
    );
    tree.add_stylesheet(&css, Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(root),
    );
    match result.declarations().get(&key) {
        Some(value) => format!("{value:?}"),
        None => "<absent>".to_owned(),
    }
}

fn cssom(property: ComputedProperty, computed: &ComputedValues) -> String {
    let mut ch_advance = |_: &ChFontKey| CH_ADVANCE_PX;
    match property.serialize(computed, &mut ch_advance) {
        Some(text) => format!("Some({text:?})"),
        None => "None".to_owned(),
    }
}

/// Records every stage of every sample of `entry` into `out`.
fn record_entry(entry: &Entry, out: &mut String) {
    let name = entry.name;
    let key = property_key_for_name(name);
    let cssom_property = ComputedProperty::from_name(name);
    let mut line = |sample: &str, stage: &str, value: &str| {
        writeln!(out, "{name}: {sample} | {stage} | {value}").expect("write to String");
    };

    // Per-property lines: the stages that do not apply, and the values of the
    // parent and of a child that does not declare the property.
    let baseline = element_fixture(entry, None);
    match key {
        Some(key) => line(
            "<undeclared>",
            "page",
            &page_value(entry, key, None, &baseline.parent),
        ),
        None => line("*", "page", "<no PropertyKey>"),
    }
    match entry.computed {
        Some(field) => {
            let parent_label = format!("<parent {}>", entry.parent);
            line(&parent_label, "computed", &field(&baseline.parent));
            line("<undeclared>", "computed", &field(&baseline.undeclared));
        }
        None => line("*", "computed", "<no computed field>"),
    }
    match cssom_property {
        Some(property) => line(
            "<undeclared>",
            "cssom",
            &cssom(property, &baseline.undeclared),
        ),
        None => line("*", "cssom", "<not a ComputedProperty>"),
    }

    for sample in entry.all_samples() {
        match parse(name, &sample) {
            Ok(value) => {
                line(&sample, "parse", &format!("Some({value:?})"));
                let serialized = serialize_value(&value);
                line(&sample, "serialize", &format!("{serialized:?}"));
            }
            Err(rejected) => {
                line(&sample, "parse", &rejected);
                line(&sample, "serialize", "-");
            }
        }
        let run = element_fixture(entry, Some(&sample));
        if let Some(field) = entry.computed {
            line(&sample, "computed", &field(&run.declared));
        }
        if let Some(property) = cssom_property {
            line(&sample, "cssom", &cssom(property, &run.declared));
        }
        if let Some(key) = key {
            line(
                &sample,
                "page",
                &page_value(entry, key, Some(&sample), &baseline.parent),
            );
        }
    }
}

/// The snapshot text of one domain.
fn record_domain(entries: &[Entry]) -> String {
    let mut out = String::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        record_entry(entry, &mut out);
    }
    out
}

fn assert_domain_snapshot(domain: &str) {
    let entries = DOMAINS
        .iter()
        .find(|(name, _)| *name == domain)
        .map(|(_, entries)| *entries)
        .expect("domain is listed in DOMAINS");
    let text = record_domain(entries);
    insta::with_settings!({
        snapshot_path => "characterization/snapshots",
        prepend_module_to_snapshot => false,
        omit_expression => true,
    }, {
        insta::assert_snapshot!(domain, text);
    });
}

#[test]
fn box_model() {
    assert_domain_snapshot("box_model");
}

#[test]
fn color() {
    assert_domain_snapshot("color");
}

#[test]
fn content() {
    assert_domain_snapshot("content");
}

#[test]
fn layout() {
    assert_domain_snapshot("layout");
}

#[test]
fn text() {
    assert_domain_snapshot("text");
}

#[test]
fn visual() {
    assert_domain_snapshot("visual");
}
