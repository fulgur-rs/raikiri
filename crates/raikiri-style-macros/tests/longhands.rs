//! Expands `#[longhands]` inside stand-ins for the raikiri-style items the
//! expansion names (`crate::property::{Longhand, AbsolutizeCx}`,
//! `crate::resolve::ResolveContext`, `crate::specified::SpecifiedValues`,
//! `crate::computed::ComputedValues`) and checks the generated items'
//! behaviour.

#![allow(missing_docs)]

mod resolve {
    /// Tree-global inputs of absolutization.
    #[derive(Debug)]
    pub struct ResolveContext {
        pub root_font_size: f32,
    }

    impl ResolveContext {
        pub fn initial() -> Self {
            Self {
                root_font_size: 16.0,
            }
        }
    }
}

mod property {
    use cssparser::{Parser, Token};

    use crate::resolve::ResolveContext;

    /// Inputs of a specified-to-computed hook.
    pub struct AbsolutizeCx<'a> {
        pub font_size: f32,
        pub ctx: &'a ResolveContext,
    }

    impl<'a> AbsolutizeCx<'a> {
        pub fn initial(ctx: &'a ResolveContext) -> Self {
            Self {
                font_size: ctx.root_font_size,
                ctx,
            }
        }
    }

    mod longhand_trait {
        use cssparser::Parser;

        use super::AbsolutizeCx;

        /// The shape the expansion implements.
        pub trait Longhand {
            const NAME: &'static str;
            const INHERITED: bool;
            type Specified: Clone + core::fmt::Debug + PartialEq;
            type Computed: Clone + core::fmt::Debug + PartialEq;
            fn initial() -> Self::Specified;
            fn parse(input: &mut Parser<'_, '_>) -> Option<Self::Specified>;
            fn compute(specified: Self::Specified, cx: &AbsolutizeCx<'_>) -> Self::Computed;
            fn lift(computed: Self::Computed) -> Self::Specified;
            #[cfg(test)]
            fn sample() -> Self::Specified;
        }
    }
    pub use longhand_trait::Longhand;

    /// A pre-existing value type, parsed by a hand-written function.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ObjectFit {
        Fill,
        Contain,
        Cover,
    }

    pub fn parse_object_fit(input: &mut Parser<'_, '_>) -> Option<ObjectFit> {
        let ident = input.expect_ident().ok()?;
        match &**ident {
            "fill" => Some(ObjectFit::Fill),
            "contain" => Some(ObjectFit::Contain),
            "cover" => Some(ObjectFit::Cover),
            _ => None,
        }
    }

    pub fn parse_number(input: &mut Parser<'_, '_>) -> Option<f32> {
        input.expect_number().ok()
    }

    pub fn clamp_opacity(value: f32, _cx: &AbsolutizeCx<'_>) -> f32 {
        value.clamp(0.0, 1.0)
    }

    /// A specified length.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Length {
        Px(f32),
        Em(f32),
        Rem(f32),
    }

    /// A computed length.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Px(pub f32);

    impl From<Px> for Length {
        fn from(px: Px) -> Self {
            Length::Px(px.0)
        }
    }

    pub fn parse_length(input: &mut Parser<'_, '_>) -> Option<Length> {
        match input.next().ok()? {
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => {
                Some(Length::Px(*value))
            }
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => {
                Some(Length::Em(*value))
            }
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => {
                Some(Length::Rem(*value))
            }
            _ => None,
        }
    }

    pub fn absolutize_length(length: Length, cx: &AbsolutizeCx<'_>) -> Px {
        match length {
            Length::Px(px) => Px(px),
            Length::Em(em) => Px(em * cx.font_size),
            Length::Rem(rem) => Px(rem * cx.ctx.root_font_size),
        }
    }

    /// The residue check of the length entries: font- and root-relative
    /// lengths are resolved by computing, `px` is not.
    pub fn length_residue(length: &Length) -> Option<&'static str> {
        match length {
            Length::Px(_) => None,
            Length::Em(_) => Some("Length::Em"),
            Length::Rem(_) => Some("Length::Rem"),
        }
    }

    /// An explicit lift, for the entry that does not use `Into::into`.
    pub fn px_to_length(px: Px) -> Length {
        Length::Px(px.0)
    }

    /// Payload of the hand-written `Deferred` variant: its key is data.
    #[derive(Clone, Debug, PartialEq)]
    pub struct DeferredValue {
        pub key: PropertyKey,
    }

    /// Payload whose key is computed by a named function.
    #[derive(Clone, Debug, PartialEq)]
    pub struct PendingValue(pub PropertyKey);

    impl PendingValue {
        pub fn property_key(&self) -> PropertyKey {
            self.0
        }
    }

    #[raikiri_style_macros::longhands]
    mod decl {
        //! Inner attributes stay inside the expanded module.
        #![allow(clippy::large_enum_variant)]

        use super::*;

        /// Every property value.
        #[non_exhaustive]
        #[derive(Clone, Debug, PartialEq)]
        pub enum PropertyValue {
            /// Hand-written, key of the same name.
            Color(u32),
            /// Hand-written, mapped to another key.
            #[key(Custom)]
            CustomProperty(String),
            /// Hand-written, key computed from the payload.
            #[key(with = |value| value.key)]
            Deferred(DeferredValue),
            /// Hand-written, key from a named function.
            #[key(with = PendingValue::property_key)]
            Pending(PendingValue),
        }

        /// Every property key.
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum PropertyKey {
            /// `color`.
            Color,
            /// A custom property.
            Custom,
        }

        properties! {
            /// CSS Compositing 1 §3.4.2
            "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },
            /// CSS Images 3 §5.1
            "object-fit" => ObjectFit {
                initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain, residue: none,
            },
        }

        properties! {
            /// CSS Color 4 §3.3
            "opacity" => Opacity: f32 {
                initial: 1.0, inherited: no, parse: parse_number,
                // Deliberately clamps to the initial value, against the crate
                // docs' advice, so that `equal_fields` has a clamp to catch.
                compute: clamp_opacity, sample: 2.0, residue: none,
            },
            /// An inherited keyword longhand with explicit spellings.
            "text-case" => TextCase {
                inherited: yes,
                derive: [Hash, Default],
                keywords: [
                    /// No transformation.
                    None = "none",
                    Upper = "uppercase",
                    SmallCaps,
                ],
                initial: None,
            },
            /// An inherited longhand whose computed type differs, lifted
            /// back through `Into::into`.
            "word-spacing" => WordSpacing: Length {
                initial: Length::Px(0.0), inherited: yes, parse: parse_length,
                computed: Px via absolutize_length, sample: Length::Em(2.0),
                residue: length_residue
            },
            /// A non-inherited longhand with an explicit lift and field.
            "tab-width" => TabWidth: Length {
                initial: Length::Px(8.0), inherited: no, parse: parse_length,
                computed: Px via absolutize_length, lift: px_to_length,
                field: tab, sample: Length::Em(1.0), residue: crate::property::length_residue,
            },
            /// A keywords longhand whose initial value is written as a path.
            "break-mode" => BreakMode {
                keywords: [Auto, Always, Avoid], initial: BreakMode::Always, inherited: no,
                derive: [core::default::Default, core::hash::Hash, PartialOrd, Ord],
            },
        }
    }
    pub use decl::*;
}

mod specified {
    use crate::property::SpecifiedTable;

    pub struct SpecifiedValues {
        pub color: u32,
        pub longhands: SpecifiedTable,
    }
}

mod computed {
    use crate::property::ComputedTable;

    pub struct ComputedValues {
        pub color: u32,
        pub longhands: ComputedTable,
    }
}

use cssparser::{Parser, ParserInput};
use property::{
    BreakMode, ComputedTable, DeferredValue, Isolation, Length, Longhand, ObjectFit, PendingValue,
    PropertyKey, PropertyValue, Px, SpecifiedTable, TextCase,
};

fn parse(name: &str, css: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    property::parse_longhand_value(name, &mut parser)
}

fn cx_with_font_size(ctx: &resolve::ResolveContext, font_size: f32) -> property::AbsolutizeCx<'_> {
    property::AbsolutizeCx { font_size, ctx }
}

#[test]
fn keyword_enum_has_spellings_and_all() {
    assert_eq!(Isolation::ALL, &[Isolation::Auto, Isolation::Isolate]);
    assert_eq!(Isolation::Isolate.as_css_str(), "isolate");
    assert_eq!(
        Isolation::from_css_ident("ISOLATE"),
        Some(Isolation::Isolate)
    );
    assert_eq!(Isolation::from_css_ident("isolated"), None);
    assert_eq!(
        TextCase::ALL
            .iter()
            .map(|k| k.as_css_str())
            .collect::<Vec<_>>(),
        ["none", "uppercase", "small-caps"]
    );
    assert_eq!(
        TextCase::from_css_ident("Small-Caps"),
        Some(TextCase::SmallCaps)
    );
}

/// `derive:` adds derives to the generated enum; with `Default`, the
/// `initial:` keyword (bare or written as a path) is the default.
#[test]
fn keyword_enums_take_extra_derives_and_default_to_their_initial() {
    use std::collections::HashSet;

    assert_eq!(TextCase::default(), TextCase::None);
    assert_eq!(BreakMode::default(), BreakMode::Always);
    let set: HashSet<TextCase> = TextCase::ALL.iter().copied().collect();
    assert_eq!(set.len(), 3);
    let set: HashSet<BreakMode> = [BreakMode::Avoid, BreakMode::Avoid].into_iter().collect();
    assert_eq!(set.len(), 1);
    // `PartialOrd` / `Ord` follow the declaration order.
    assert!(BreakMode::Auto < BreakMode::Avoid);
    assert_eq!(BreakMode::ALL.iter().max(), Some(&BreakMode::Avoid));
}

#[test]
fn type_modules_name_the_written_types() {
    fn same<T>(_: T, _: T) {}
    let specified: property::object_fit::Specified = ObjectFit::Fill;
    same(specified, ObjectFit::Fill);
    let opacity: property::opacity::Computed = 0.5_f32;
    same(opacity, 0.5_f32);
    let computed: property::word_spacing::Computed = Px(1.0);
    same(computed, Px(1.0));
    let tab: property::tab::Specified = Length::Em(1.0);
    same(tab, Length::Em(1.0));
    let kw: property::isolation::Specified = Isolation::Auto;
    same(kw, Isolation::Auto);
}

#[test]
fn longhand_impls_carry_name_and_inheritance() {
    assert_eq!(<property::opacity::Property as Longhand>::NAME, "opacity");
    let inherited = [
        <property::opacity::Property as Longhand>::INHERITED,
        <property::text_case::Property as Longhand>::INHERITED,
    ];
    assert_eq!(inherited, [false, true]);
    assert_eq!(<property::tab::Property as Longhand>::NAME, "tab-width");
}

#[test]
fn specified_initial_values() {
    let table = SpecifiedTable::initial();
    assert_eq!(table.isolation, Isolation::Auto);
    assert_eq!(table.object_fit, ObjectFit::Fill);
    assert_eq!(table.opacity, 1.0);
    assert_eq!(table.text_case, TextCase::None);
    assert_eq!(table.word_spacing, Length::Px(0.0));
    assert_eq!(table.tab, Length::Px(8.0));
    assert_eq!(table.break_mode, BreakMode::Always);
}

#[test]
fn computed_initial_values_go_through_compute() {
    let table = ComputedTable::initial();
    assert_eq!(table.opacity, 1.0);
    assert_eq!(table.word_spacing, Px(0.0));
    assert_eq!(table.tab, Px(8.0));
}

#[test]
fn inherit_from_copies_inherited_and_resets_the_rest() {
    let mut parent = ComputedTable::initial();
    parent.isolation = Isolation::Isolate;
    parent.object_fit = ObjectFit::Cover;
    parent.opacity = 0.25;
    parent.text_case = TextCase::Upper;
    parent.word_spacing = Px(3.0);
    parent.tab = Px(40.0);

    let child = SpecifiedTable::inherit_from(&parent);
    // Inherited: the parent's computed value, lifted to specified form.
    assert_eq!(child.text_case, TextCase::Upper);
    assert_eq!(child.word_spacing, Length::Px(3.0));
    // Not inherited: the initial value.
    assert_eq!(child.isolation, Isolation::Auto);
    assert_eq!(child.object_fit, ObjectFit::Fill);
    assert_eq!(child.opacity, 1.0);
    assert_eq!(child.tab, Length::Px(8.0));
}

#[test]
fn apply_stores_the_winner_and_absolutize_runs_the_hooks() {
    let mut table = SpecifiedTable::initial();
    table.apply(PropertyValue::Opacity(2.0));
    table.apply(PropertyValue::WordSpacing(Length::Em(2.0)));
    table.apply(PropertyValue::TabWidth(Length::Em(0.5)));
    table.apply(PropertyValue::Isolation(Isolation::Isolate));
    assert_eq!(table.opacity, 2.0);
    assert_eq!(table.isolation, Isolation::Isolate);

    let ctx = resolve::ResolveContext::initial();
    let computed = table.absolutize(&cx_with_font_size(&ctx, 10.0));
    assert_eq!(computed.opacity, 1.0);
    assert_eq!(computed.word_spacing, Px(20.0));
    assert_eq!(computed.tab, Px(5.0));
    assert_eq!(computed.isolation, Isolation::Isolate);
}

#[test]
#[should_panic(expected = "not a longhand declared in `properties!`")]
fn apply_rejects_hand_written_variants() {
    SpecifiedTable::initial().apply(PropertyValue::Color(0));
}

#[test]
fn page_absolutize_computes_and_lifts_back() {
    let ctx = resolve::ResolveContext::initial();
    let cx = cx_with_font_size(&ctx, 10.0);
    assert_eq!(
        property::longhand_page_absolutize(PropertyValue::Opacity(2.0), &cx),
        PropertyValue::Opacity(1.0)
    );
    assert_eq!(
        property::longhand_page_absolutize(PropertyValue::WordSpacing(Length::Em(1.5)), &cx),
        PropertyValue::WordSpacing(Length::Px(15.0))
    );
    assert_eq!(
        property::longhand_page_absolutize(PropertyValue::TabWidth(Length::Em(1.0)), &cx),
        PropertyValue::TabWidth(Length::Px(10.0))
    );
    assert_eq!(
        property::longhand_page_absolutize(PropertyValue::TabWidth(Length::Rem(2.0)), &cx),
        PropertyValue::TabWidth(Length::Px(32.0))
    );
    assert_eq!(
        property::longhand_page_absolutize(PropertyValue::ObjectFit(ObjectFit::Cover), &cx),
        PropertyValue::ObjectFit(ObjectFit::Cover)
    );
}

#[test]
fn names_and_lookup() {
    assert_eq!(
        property::LONGHAND_NAMES,
        [
            "isolation",
            "object-fit",
            "opacity",
            "text-case",
            "word-spacing",
            "tab-width",
            "break-mode"
        ]
    );
    assert_eq!(
        property::longhand_key_for_name("object-fit"),
        Some(PropertyKey::ObjectFit)
    );
    assert_eq!(
        property::longhand_key_for_name("tab-width"),
        Some(PropertyKey::TabWidth)
    );
    assert_eq!(property::longhand_key_for_name("color"), None);
}

#[test]
fn parse_dispatch() {
    assert_eq!(
        parse("isolation", "ISOLATE"),
        Some(PropertyValue::Isolation(Isolation::Isolate))
    );
    assert_eq!(
        parse("object-fit", "cover"),
        Some(PropertyValue::ObjectFit(ObjectFit::Cover))
    );
    assert_eq!(parse("opacity", "0.5"), Some(PropertyValue::Opacity(0.5)));
    assert_eq!(
        parse("text-case", "uppercase"),
        Some(PropertyValue::TextCase(TextCase::Upper))
    );
    assert_eq!(
        parse("word-spacing", "2em"),
        Some(PropertyValue::WordSpacing(Length::Em(2.0)))
    );
    assert_eq!(parse("isolation", "sideways"), None);
    assert_eq!(parse("opacity", "auto"), None);
    assert_eq!(parse("color", "red"), None);
}

#[test]
fn key_projection() {
    assert_eq!(PropertyValue::Color(0).key(), PropertyKey::Color);
    assert_eq!(
        PropertyValue::CustomProperty("--x".into()).key(),
        PropertyKey::Custom
    );
    assert_eq!(
        PropertyValue::Deferred(DeferredValue {
            key: PropertyKey::Opacity
        })
        .key(),
        PropertyKey::Opacity
    );
    assert_eq!(
        PropertyValue::Pending(PendingValue(PropertyKey::TabWidth)).key(),
        PropertyKey::TabWidth
    );
    assert_eq!(PropertyValue::Opacity(1.0).key(), PropertyKey::Opacity);
    assert_eq!(
        PropertyValue::TabWidth(Length::Px(1.0)).key(),
        PropertyKey::TabWidth
    );
}

#[test]
fn value_pattern_matches_exactly_the_declared_variants() {
    fn declared(value: &PropertyValue) -> bool {
        use property::longhand_value_pat;
        matches!(value, longhand_value_pat!())
    }
    assert!(declared(&PropertyValue::Isolation(Isolation::Auto)));
    assert!(declared(&PropertyValue::TabWidth(Length::Px(0.0))));
    assert!(!declared(&PropertyValue::Color(0)));
    assert!(!declared(&PropertyValue::CustomProperty(String::new())));
}

#[test]
fn samples_default_to_the_first_non_initial_keyword() {
    let samples = property::longhand_samples();
    assert_eq!(
        samples,
        [
            ("isolation", PropertyValue::Isolation(Isolation::Isolate)),
            ("object-fit", PropertyValue::ObjectFit(ObjectFit::Contain)),
            ("opacity", PropertyValue::Opacity(2.0)),
            ("text-case", PropertyValue::TextCase(TextCase::Upper)),
            ("word-spacing", PropertyValue::WordSpacing(Length::Em(2.0))),
            ("tab-width", PropertyValue::TabWidth(Length::Em(1.0))),
            // The first keyword that is not the (path-written) initial one.
            ("break-mode", PropertyValue::BreakMode(BreakMode::Auto)),
        ]
    );
}

/// The table samples hold every entry's `sample:`; the computed one runs
/// the hooks in the initial context. `equal_fields` names the entries that
/// did not move away from the initial value.
#[test]
fn table_samples_and_equal_fields() {
    let specified = SpecifiedTable::sample();
    assert_eq!(specified.isolation, Isolation::Isolate);
    assert_eq!(specified.object_fit, ObjectFit::Contain);
    assert_eq!(specified.opacity, 2.0);
    assert_eq!(specified.text_case, TextCase::Upper);
    assert_eq!(specified.word_spacing, Length::Em(2.0));
    assert_eq!(specified.tab, Length::Em(1.0));
    assert_eq!(specified.break_mode, BreakMode::Auto);
    assert_eq!(
        specified.equal_fields(&SpecifiedTable::initial()),
        Vec::<&str>::new()
    );
    assert_eq!(
        SpecifiedTable::initial().equal_fields(&SpecifiedTable::initial()),
        property::LONGHAND_NAMES
    );

    let computed = ComputedTable::sample();
    // Initial context: the root font size, 16px.
    assert_eq!(computed.word_spacing, Px(32.0));
    assert_eq!(computed.tab, Px(16.0));
    // `2.0` clamps to the initial `1.0`, which `equal_fields` reports.
    assert_eq!(computed.opacity, 1.0);
    assert_eq!(
        computed.equal_fields(&ComputedTable::initial()),
        ["opacity"]
    );
}

#[test]
fn registry_callbacks_append_declared_entries() {
    macro_rules! collect_samples {
        ($($variant:ident => $sample:expr,)*) => {
            vec![$((stringify!($variant), $sample)),*]
        };
    }
    macro_rules! collect_variants {
        ($($variant:ident,)*) => {
            vec![$(stringify!($variant)),*]
        };
    }
    let samples = property::with_longhand_samples!(collect_samples {
        Color => PropertyValue::Color(1),
    });
    assert_eq!(samples.len(), 8);
    assert_eq!(samples[0], ("Color", PropertyValue::Color(1)));
    assert_eq!(samples[3], ("Opacity", PropertyValue::Opacity(2.0)));

    // Brace invocation: rustfmt would drop the trailing comma the
    // callback needs.
    let variants = property::with_longhand_variants! { collect_variants { Color, } };
    assert_eq!(
        variants,
        [
            "Color",
            "Isolation",
            "ObjectFit",
            "Opacity",
            "TextCase",
            "WordSpacing",
            "TabWidth",
            "BreakMode"
        ]
    );
}

/// `residue:` answers per entry: `none` (written, or the default of a
/// `keywords:` entry without a hook) is never a residue, and a residue
/// function sees the specified payload.
#[test]
fn specified_residue_asks_each_entry() {
    use property::longhand_specified_residue as residue;

    assert_eq!(residue(&PropertyValue::Isolation(Isolation::Isolate)), None);
    assert_eq!(residue(&PropertyValue::TextCase(TextCase::Upper)), None);
    assert_eq!(residue(&PropertyValue::ObjectFit(ObjectFit::Cover)), None);
    assert_eq!(residue(&PropertyValue::Opacity(2.0)), None);
    assert_eq!(
        residue(&PropertyValue::WordSpacing(Length::Em(2.0))),
        Some("Length::Em")
    );
    assert_eq!(residue(&PropertyValue::WordSpacing(Length::Px(2.0))), None);
    assert_eq!(
        residue(&PropertyValue::TabWidth(Length::Rem(1.0))),
        Some("Length::Rem")
    );
    // Over every sample, only the length entries report a residue.
    let reported: Vec<_> = property::longhand_samples()
        .iter()
        .filter_map(|(name, value)| residue(value).map(|r| (*name, r)))
        .collect();
    assert_eq!(
        reported,
        [("word-spacing", "Length::Em"), ("tab-width", "Length::Em")]
    );
}

#[test]
#[should_panic(expected = "not a longhand declared in `properties!`")]
fn specified_residue_rejects_hand_written_variants() {
    property::longhand_specified_residue(&PropertyValue::Color(0));
}

/// Table fields are read through `Deref` and written through `longhands`:
/// no `DerefMut` is generated.
#[test]
fn deref_exposes_the_table_fields_for_reading() {
    let mut specified = specified::SpecifiedValues {
        color: 0,
        longhands: SpecifiedTable::initial(),
    };
    specified.longhands.opacity = 0.5;
    assert_eq!(specified.opacity, 0.5);
    assert_eq!(specified.color, 0);

    let computed = computed::ComputedValues {
        color: 0,
        longhands: ComputedTable::initial(),
    };
    assert_eq!(computed.word_spacing, Px(0.0));
    assert_eq!(computed.color, 0);
}
