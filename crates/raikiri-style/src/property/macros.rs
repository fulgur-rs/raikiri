//! Declarative macros shared by the property modules.

/// Implements keyword serialization, and optionally parsing, for a CSS
/// keyword enum from one variant-to-keyword table, so the two directions
/// cannot drift apart.
///
/// `css_keywords!(Type { Variant => "keyword", ... })` adds `as_css_str`
/// and an ASCII case-insensitive `from_css_ident`. The `@serialize` form adds
/// only `as_css_str`, for enums whose grammar spans several tokens.
macro_rules! css_keywords {
    (@serialize $ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        impl $ty {
            /// Every variant, in table order.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// This variant's own keyword spelling. Computed-value remaps, such as
            /// a legacy keyword computing to another one, are the caller's job.
            pub const fn as_css_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $css,)+
                }
            }
        }
    };
    ($ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        css_keywords!(@serialize $ty { $($variant => $css),+ });

        impl $ty {
            /// Parses one keyword, ASCII case-insensitively.
            pub(crate) fn from_css_ident(ident: &str) -> Option<Self> {
                cssparser::match_ignore_ascii_case! { ident,
                    $($css => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

/// Declares the property enums and their table-driven projections in one place.
///
/// The invocation is the definition site of [`PropertyValue`], [`PropertyKey`]
/// and [`PropertyValue::key`], because a macro cannot add variants to an enum
/// declared elsewhere. Properties not yet moved to the table pass through
/// verbatim in the `manual` sections; table blocks are appended after them.
///
/// Appending is safe for cascade ordering: table variants are 1:1 with
/// disjoint fields and are never shorthands, so the load-bearing
/// declaration-order rule documented on [`PropertyKey`] does not apply to them.
///
/// A block reads like the property's definition table:
///
/// ```ignore
/// /// CSS Compositing and Blending Level 1 §3.4.2
/// "isolation" => Isolation {
///     value: keywords {
///         /// `auto`: the initial value.
///         Auto => "auto",
///         Isolate => "isolate",
///     },
///     initial: Auto,
///     inherited: no,
///     computed: as_specified,
///     sample: Isolation::Isolate,
/// }
/// ```
///
/// A block that reuses an existing value type supplies `parse:` instead:
///
/// ```ignore
/// "object-fit" => ObjectFit {
///     value: ObjectFit,
///     initial: ObjectFit::Fill,
///     inherited: no,
///     parse: parse_object_fit,
///     computed: as_specified,
///     sample: ObjectFit::Contain,
/// }
/// ```
///
/// The field order is fixed: `value`, `initial`, `inherited`, `parse` (when
/// present), `computed`, then `sample`. `value:` accepts a single identifier
/// type only.
///
/// - `value: keywords { .. }` defines the value enum named after the block and
///   parses through its `from_css_ident`. `parse:` must then be omitted.
/// - `value: SomeType` reuses an existing type and requires `parse: some_fn`,
///   a `fn(&mut Parser) -> Option<SomeType>` path.
/// - `initial` and `inherited` are validated at compile time but not consumed
///   yet. `computed:` accepts only `as_specified`; any other hook is a
///   `compile_error!` until hooks are consumed.
/// - `sample:` is the test-only worst-case value used by the page-cascade
///   corpus: pick a non-initial value, so a regression that silently resets
///   or transforms the property is visible. It is an expression of the value
///   type and is compiled only under `cfg(test)`.
///
/// Also generated: `longhand_key_for_name`, `parse_longhand_value` and
/// `LONGHAND_NAMES`, which the hand-written name lookup and `parse_value`
/// dispatch fall through to, and `longhand_value_pat!()`, a pattern matching
/// every table variant, which the pass-through arms of the exhaustive matches
/// in `rule.rs`, `cascade/inherit.rs`, `page/absolutize.rs` and
/// `serialize.rs` use.
///
/// Test-only: `longhand_samples()` pairs each CSS name with its `sample:`
/// value, and `with_longhand_samples!` / `with_longhand_variants!` feed the
/// table entries into `property_key_samples!` and
/// `property_value_variant_registry!` in `page/cascade/tests.rs`.
///
/// # Adding a property
///
/// To add a new property to the table:
///
/// 1. Add a block to this invocation in `longhands.rs` with the property's CSS
///    name, variant name, value type, initial value, inherited status,
///    `computed: as_specified`, and a non-initial `sample:` (for test coverage).
///    Follow the format of existing entries like `"isolation" => Isolation { .. }`,
///    using `value: keywords { .. }` for keyword enums or `value: SomeType` with
///    `parse: some_fn` for other types.
/// 2. Add the property's CSS name to `supported_property_names()` in `names.rs`,
///    maintaining alphabetical order (pinned by the
///    `longhand_names_are_supported_property_names` test).
/// 3. The `apply_value` arm in `cascade/inherit.rs` and the `SpecifiedValues` and
///    `ComputedValues` fields in `specified.rs` and `computed.rs` remain hand-written
///    until those sites are generated. Pass-through matches in `rule.rs`, the resolve
///    step in `cascade/inherit.rs`, `page/absolutize.rs`, and `serialize_value`'s
///    `None` arm are covered by the table, as are the two page-cascade registries.
///    `computed:` hooks are rejected for now; table payloads must be free of
///    lengths needing absolutization.
///
/// A table block currently generates: `PropertyKey` and `PropertyValue` enum
/// variants; the `key()` projection; the name-to-key lookup
/// (`longhand_key_for_name`); the value parsing dispatch (`parse_longhand_value`);
/// the `longhand_value_pat!()` pass-through pattern macro; the `LONGHAND_NAMES`
/// constant; the test registries' macros (`with_longhand_samples` and
/// `with_longhand_variants`); and for keyword value blocks, the value enum with
/// its `INITIAL` constant.
macro_rules! longhands {
    // ---- helpers -------------------------------------------------------
    (@ty $V:ident keywords { $($kw:tt)* }) => { $V };
    (@ty $V:ident $ty:ident) => { $ty };

    (@value_enum $V:ident $css:literal keywords {
        $( $(#[$vm:meta])* $Var:ident => $kw:literal ),+ $(,)?
    }) => {
        #[doc = concat!("Specified value of `", $css, "`; see [`PropertyValue::", stringify!($V), "`].")]
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $V {
            $( $(#[$vm])* $Var, )+
        }
        css_keywords!($V { $( $Var => $kw ),+ });
    };
    (@value_enum $V:ident $css:literal $ty:ident) => {};

    (@parse $V:ident $input:ident keywords { $($kw:tt)* }) => {
        $input
            .expect_ident()
            .ok()
            .and_then(|ident| $V::from_css_ident(ident))
            .map(PropertyValue::$V)
    };
    (@parse $V:ident $input:ident $ty:ident $parse:path) => {
        $parse($input).map(PropertyValue::$V)
    };

    (@initial $V:ident keywords { $($kw:tt)* } $init:expr) => {
        impl $V {
            /// The property's initial value.
            #[allow(dead_code)]
            pub(crate) const INITIAL: $V = {
                #[allow(unused_imports)]
                use $V::*;
                $init
            };
        }
    };
    (@initial $V:ident $ty:ident $init:expr) => {
        const _: fn() -> $ty = || $init;
    };

    (@inherited yes) => {};
    (@inherited no) => {};

    (@computed as_specified) => {};
    (@computed $f:ident $(:: $rest:ident)*) => {
        compile_error!(
            "`computed:` hooks are not consumed yet; use `computed: as_specified` \
             and keep this property's computed-value handling in the hand-written sites"
        );
    };

    // Test-only callback macros feeding table entries into the page-cascade
    // registries. `$d` is a literal `$` token, so the nested macro can declare
    // its own metavariables without the outer expansion claiming them.
    // `$sample` arrives wrapped in parentheses, which double as the variant
    // constructor's argument list.
    (@test_registries [$d:tt] $( $V:ident => $sample:tt )*) => {
        /// Invokes `$cb! { <hand entries> Variant => sample, ... }` with every
        /// table property's `sample:` value appended after the hand entries.
        #[cfg(test)]
        macro_rules! with_longhand_samples {
            ($d cb:ident { $d ($d hand:tt)* }) => {
                $d cb! { $d ($d hand)* $( $V => PropertyValue::$V $sample, )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_samples;

        /// Invokes `$cb! { <hand names> Variant, ... }` with every table
        /// variant name appended after the hand names.
        #[cfg(test)]
        macro_rules! with_longhand_variants {
            ($d cb:ident { $d ($d hand:tt)* }) => {
                $d cb! { $d ($d hand)* $( $V, )* }
            };
        }
        #[cfg(test)]
        pub(crate) use with_longhand_variants;
    };

    // ---- entry ---------------------------------------------------------
    (
        property_value {
            $(#[$pvm:meta])*
            manual { $($manual_value:tt)* }
        }
        property_key {
            $(#[$pkm:meta])*
            manual { $($manual_key:tt)* }
        }
        key_arms { $($manual_key_arm:tt)* }
        $(
            $(#[$m:meta])*
            $css:literal => $V:ident {
                value: $vt:ident $({ $($kw:tt)* })?,
                initial: $init:expr,
                inherited: $inh:ident,
                $(parse: $parse:path,)?
                computed: $comp:ident $(:: $comp_rest:ident)*,
                sample: $sample:expr $(,)?
            }
        )*
    ) => {
        $(#[$pvm])*
        pub enum PropertyValue {
            $($manual_value)*
            $(
                $(#[$m])*
                $V(longhands!(@ty $V $vt $({ $($kw)* })?)),
            )*
        }

        $(#[$pkm])*
        pub enum PropertyKey {
            $($manual_key)*
            $( $V, )*
        }

        impl PropertyValue {
            /// Returns the property key for this value.
            ///
            /// Used as the discriminant for selecting one winner per property in the
            /// cascade, and as the key in the `@page` cascade result map.
            pub fn key(&self) -> PropertyKey {
                match self {
                    $($manual_key_arm)*
                    $( PropertyValue::$V(_) => PropertyKey::$V, )*
                }
            }
        }

        $(
            longhands!(@value_enum $V $css $vt $({ $($kw)* })?);
            longhands!(@initial $V $vt $({ $($kw)* })? $init);
            longhands!(@inherited $inh);
            longhands!(@computed $comp $(:: $comp_rest)*);
        )*

        /// Every property name declared in the table, lowercase.
        #[cfg_attr(not(test), allow(dead_code))]
        pub(crate) const LONGHAND_NAMES: &[&str] = &[$($css),*];

        /// Pattern matching every table-declared `PropertyValue` variant,
        /// for the exhaustive matches whose table properties all take the
        /// same pass-through arm.
        macro_rules! longhand_value_pat {
            () => { $( PropertyValue::$V(..) )|+ };
        }
        pub(crate) use longhand_value_pat;

        /// Every table property's CSS name paired with its `sample:` value.
        #[cfg(test)]
        pub(crate) fn longhand_samples() -> Vec<(&'static str, PropertyValue)> {
            vec![ $( ($css, PropertyValue::$V($sample)), )* ]
        }

        longhands!(@test_registries [$] $( $V => ($sample) )*);

        /// Name lookup for table-declared properties; `normalized_name` must
        /// already be ASCII-lowercase.
        #[allow(unused_variables)]
        pub(crate) fn longhand_key_for_name(normalized_name: &str) -> Option<PropertyKey> {
            match normalized_name {
                $( $css => Some(PropertyKey::$V), )*
                _ => None,
            }
        }

        /// Value parsing for table-declared properties; `normalized_name` must
        /// already be ASCII-lowercase. `None` covers both an unknown name and
        /// an invalid value, like `parse_value`.
        #[allow(unused_variables)]
        pub(crate) fn parse_longhand_value(
            normalized_name: &str,
            input: &mut Parser<'_, '_>,
        ) -> Option<PropertyValue> {
            match normalized_name {
                $( $css => longhands!(@parse $V input $vt $({ $($kw)* })? $($parse)?), )*
                _ => None,
            }
        }
    };
}
