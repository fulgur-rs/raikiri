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
///     field: isolation,
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
///     field: object_fit,
///     sample: ObjectFit::Contain,
/// }
/// ```
///
/// The field order is fixed: `value`, `initial`, `inherited`, `parse` (when
/// present), `computed`, `field`, then `sample`. `value:` accepts a single
/// identifier type only.
///
/// - `value: keywords { .. }` defines the value enum named after the block and
///   parses through its `from_css_ident`. `parse:` must then be omitted.
/// - `value: SomeType` reuses an existing type and requires `parse: some_fn`,
///   a `fn(&mut Parser) -> Option<SomeType>` path.
/// - `initial` supplies the field's initial value in `SpecifiedTable` and
///   `ComputedTable`, and `inherited` decides whether `inherit_from` copies the
///   parent's value or resets to the initial one.
/// - `computed:` is either `as_specified` (the computed value is the
///   specified value, same type) or `via Marker -> ComputedType`, where
///   `Marker` is a hand-written unit type implementing
///   [`Longhand`](crate::property::Longhand) with
///   `Specified` = the `value:` type and `Computed` = `ComputedType`. The
///   computed type is spelled out because the `ComputedTable` field is public
///   and `Longhand` is not; a mismatch with the impl is a type error. A bare
///   hook function name is a `compile_error!`: call it from the marker's
///   `compute` through [`run_hook`](crate::property::run_hook) instead.
/// - `sample:` is the test-only worst-case value used by the page-cascade
///   corpus: pick a non-initial value, so a regression that silently resets
///   or transforms the property is visible. It is an expression of the value
///   type and is compiled only under `cfg(test)`.
/// - `field:` names the property's field in the generated `SpecifiedTable`
///   and `ComputedTable` structs, which are built from `initial`, `inherited`
///   and `computed:`. They are embedded as
///   `SpecifiedValues::longhands` and `ComputedValues::longhands`, and the
///   generated `Deref`/`DerefMut` impls keep `values.<field>` working.
///
/// Also generated: `longhand_key_for_name`, `parse_longhand_value` and
/// `LONGHAND_NAMES`, which the hand-written name lookup and `parse_value`
/// dispatch fall through to, and `longhand_value_pat!()`, a pattern matching
/// every table variant, which the pass-through arms of the exhaustive matches
/// in `rule.rs`, `cascade/inherit.rs` and `serialize.rs` use.
/// `page/absolutize.rs` routes the same pattern to the generated
/// `longhand_page_absolutize`, which applies `lift(compute(..))` to `via`
/// entries and passes `as_specified` entries through.
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
///    `computed: as_specified`, `field:` (the table field name), and a
///    non-initial `sample:` (for test coverage), in the field order above.
///    Write `sample:` as a path-qualified expression (e.g. `crate::property::Foo::Bar`)
///    so it resolves at both the table's expansion site and in `page/cascade/tests.rs`,
///    or import the value type there. Follow the format of existing entries like
///    `"isolation" => Isolation { .. }`, using `value: keywords { .. }` for keyword
///    enums or `value: SomeType` with `parse: some_fn` for other types.
/// 2. Add the property's CSS name to `supported_property_names()` in `names.rs`,
///    maintaining alphabetical order (pinned by the
///    `longhand_names_are_supported_property_names` test).
///
/// That is all a simple keyword property needs; everything else below is
/// generated from the block. A property whose computed value differs from
/// the specified one also needs its `Longhand` marker and hook function,
/// written next to the table:
///
/// ```ignore
/// "opacity" => Opacity {
///     value: f32,
///     initial: 1.0,
///     inherited: no,
///     parse: parse_opacity_value,
///     computed: via OpacityLonghand -> f32,
///     field: opacity,
///     sample: 2.0,
/// }
///
/// pub(crate) struct OpacityLonghand;
/// impl Longhand for OpacityLonghand {
///     type Specified = f32;
///     type Computed = f32;
///     fn compute(specified: f32, cx: &AbsolutizeCx<'_>) -> f32 {
///         run_hook(clamp_opacity, specified, cx)
///     }
///     fn lift(computed: f32) -> f32 { computed }
/// }
/// fn clamp_opacity(value: f32) -> f32 { value.clamp(0.0, 1.0) }
/// ```
///
/// A hook's parameters after the specified value are extracted from the
/// [`AbsolutizeCx`](crate::property::AbsolutizeCx) by type: `FontSize`,
/// `OwnLineHeight` and `&ResolveContext`.
///
/// `field:` must not equal the name of a hand-written field of `SpecifiedValues`
/// or `ComputedValues`: the inherent field would shadow the `Deref` target, so
/// writes through `apply` and reads through the field would diverge. The
/// block's attributes are copied onto the generated table fields, so use doc
/// comments only.
///
/// One follow-up remains in tests: every full
/// `ComputedValues { longhands: ComputedTable { .. }, .. }` literal, such as
/// the non-initial parent fixtures in `computed/tests.rs` and
/// `specified/tests.rs`, must list the new field (the compiler points at
/// them). Give it a non-initial value there so inheritance tests can tell it
/// from the default.
///
/// Limits of the generated table:
///
/// - The test-only length-residue detector in `page/cascade/tests.rs` treats
///   every table payload as length-free; a `via` entry whose specified type
///   carries lengths needs its own arm there.
/// - The initial computed value of a `via` entry is `compute(initial)` in
///   the initial context (`ResolveContext::initial()`, no own line height).
/// - Table fields are readable and writable as `values.field` through
///   `Deref`/`DerefMut`, but struct patterns cannot destructure them, and an
///   exhaustive struct literal of `SpecifiedValues` or `ComputedValues` must
///   set them through its `longhands` field.
/// - Intra-doc links to a table field must target `ComputedTable::field` or
///   `SpecifiedTable::field`; rustdoc cannot resolve fields through `Deref`,
///   so `ComputedValues::field` would be a broken link.
///
/// Properties outside the table stay hand-written: their storage in
/// `SpecifiedValues` / `ComputedValues`, their initial / `inherit_from` /
/// `absolutize_with` handling, the `apply_value` arm in `cascade/inherit.rs`,
/// the name lookup and `parse_value` arm, the `serialize_value` arm, and their
/// own `PropertyKey` / `PropertyValue` variants in the `manual` sections.
///
/// A table block generates: `PropertyKey` and `PropertyValue` enum variants;
/// the `key()` projection; the name-to-key lookup (`longhand_key_for_name`);
/// the value parsing dispatch (`parse_longhand_value`); the
/// `longhand_value_pat!()` pass-through pattern macro (used by `rule.rs`,
/// `cascade/inherit.rs`, `page/absolutize.rs`, `serialize_value`, and the
/// test-only residue detector in `page/cascade/tests.rs`); the page-context
/// absolutization (`longhand_page_absolutize`); the
/// `LONGHAND_NAMES` constant; the test registries' macros
/// (`with_longhand_samples` and `with_longhand_variants`); a field of
/// `SpecifiedTable` and `ComputedTable` (with their `initial`, `inherit_from`,
/// `absolutize` and `apply`, and the `Deref`/`DerefMut` impls that expose them
/// on `SpecifiedValues` and `ComputedValues`); and, for keyword value blocks,
/// the value enum with its `INITIAL` constant.
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

    (@initial_value $V:ident keywords { $($kw:tt)* } $init:expr) => { $V::INITIAL };
    (@initial_value $V:ident $ty:ident $init:expr) => { $init };

    (@inherited yes) => {};
    (@inherited no) => {};

    // Inherited properties take the parent's computed value; the rest reset
    // to their initial value.
    // A `via` entry lifts the parent's computed value back to its specified
    // form.
    (@inherit yes $comp:tt $parent:ident $field:ident $V:ident $vt:ident $({ $($kw:tt)* })? ; $init:expr) => {
        longhands!(@lift $comp ::core::clone::Clone::clone(&$parent.$field))
    };
    (@inherit no $comp:tt $parent:ident $field:ident $V:ident $vt:ident $({ $($kw:tt)* })? ; $init:expr) => {
        longhands!(@initial_value $V $vt $({ $($kw)* })? $init)
    };

    (@computed as_specified) => {};
    (@computed via $T:ident -> $C:ty) => {};
    (@computed $f:ident $($T:ident -> $C:ty)?) => {
        compile_error!(
            "`computed:` takes `as_specified` or `via Marker -> ComputedType`, where \
             `Marker` implements `Longhand`; call a hook function from that type's `compute`"
        );
    };

    // Per-entry computed-value behavior. `$comp` is the parenthesized
    // `computed:` spec. The catch-all arms only keep a rejected spec from
    // adding match errors on top of the `@computed` `compile_error!`.
    //
    // The computed field type is the one spelled in the table, not
    // `<Marker as Longhand>::Computed`: `Longhand` is crate-private and the
    // field is public. The two must agree, or `compute`'s result does not
    // type-check against the field.
    (@computed_ty (via $T:ident -> $C:ty) $st:ty) => { $C };
    (@computed_ty $comp:tt $st:ty) => { $st };

    (@compute (via $T:ident -> $C:ty) $e:expr, $cx:ident) => {
        <$T as crate::property::Longhand>::compute($e, $cx)
    };
    (@compute $comp:tt $e:expr, $cx:ident) => { $e };

    (@lift (via $T:ident -> $C:ty) $e:expr) => { <$T as crate::property::Longhand>::lift($e) };
    (@lift $comp:tt $e:expr) => { $e };

    // Page-context absolutization keeps the value in `PropertyValue`, so a
    // `via` entry is computed and lifted back to its specified form.
    (@page (via $T:ident -> $C:ty) $e:expr, $cx:ident) => {
        <$T as crate::property::Longhand>::lift(
            <$T as crate::property::Longhand>::compute($e, $cx),
        )
    };
    (@page $comp:tt $e:expr, $cx:ident) => { $e };

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
                computed: $comp:ident $($hook:ident -> $cty:ty)?,
                field: $field:ident,
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
            longhands!(@computed $comp $($hook -> $cty)?);
        )*

        /// Table-declared specified values, embedded as
        /// `SpecifiedValues::longhands`; its fields are reachable directly on
        /// `SpecifiedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct SpecifiedTable {
            $( $(#[$m])* pub $field: longhands!(@ty $V $vt $({ $($kw)* })?), )*
        }

        /// Table-declared computed values, embedded as
        /// `ComputedValues::longhands`; its fields are reachable directly on
        /// `ComputedValues` through `Deref`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct ComputedTable {
            $(
                $(#[$m])*
                pub $field: longhands!(@computed_ty ($comp $($hook -> $cty)?) longhands!(@ty $V $vt $({ $($kw)* })?)),
            )*
        }

        #[allow(clippy::clone_on_copy)]
        impl SpecifiedTable {
            /// Every field at its initial value.
            pub(crate) fn initial() -> Self {
                Self { $( $field: longhands!(@initial_value $V $vt $({ $($kw)* })? $init), )* }
            }

            /// The specified state of a child: inherited fields copy the parent's
            /// computed value, the others start at their initial value.
            // `parent` is unused when every table entry is non-inherited.
            #[allow(unused_variables)]
            pub(crate) fn inherit_from(parent: &ComputedTable) -> Self {
                Self {
                    $( $field: longhands!(@inherit $inh ($comp $($hook -> $cty)?) parent $field $V $vt $({ $($kw)* })? ; $init), )*
                }
            }

            /// Specified to computed: `as_specified` fields move over
            /// unchanged, `via` fields go through their `Longhand::compute`.
            // `cx` is unused when every table entry is `as_specified`.
            #[allow(unused_variables)]
            pub(crate) fn absolutize(self, cx: &crate::property::AbsolutizeCx<'_>) -> ComputedTable {
                ComputedTable {
                    $( $field: longhands!(@compute ($comp $($hook -> $cty)?) self.$field, cx), )*
                }
            }

            /// Stores a cascade winner in its field. `value` must be a table variant.
            pub(crate) fn apply(&mut self, value: PropertyValue) {
                match value {
                    $( PropertyValue::$V(v) => self.$field = v, )*
                    other => unreachable!("not a table-declared property value: {other:?}"),
                }
            }
        }

        impl ComputedTable {
            /// Every field at its initial value; a `via` field holds its
            /// initial value computed in the initial context.
            // `cx` is unused when every table entry is `as_specified`.
            #[allow(unused_variables)]
            pub(crate) fn initial() -> Self {
                let ctx = crate::resolve::ResolveContext::initial();
                let cx = &crate::property::AbsolutizeCx::initial(&ctx);
                Self {
                    $(
                        $field: longhands!(
                            @compute ($comp $($hook -> $cty)?)
                            longhands!(@initial_value $V $vt $({ $($kw)* })? $init),
                            cx
                        ),
                    )*
                }
            }
        }

        /// Page-context absolutization of one table value. `PropertyValue`
        /// keeps its specified payload type, so a `via` value is computed and
        /// lifted back; `as_specified` values pass through. `value` must be a
        /// table variant.
        // `cx` is unused when every table entry is `as_specified`.
        #[allow(unused_variables)]
        pub(crate) fn longhand_page_absolutize(
            value: PropertyValue,
            cx: &crate::property::AbsolutizeCx<'_>,
        ) -> PropertyValue {
            match value {
                $( PropertyValue::$V(v) => PropertyValue::$V(longhands!(@page ($comp $($hook -> $cty)?) v, cx)), )*
                other => unreachable!("not a table-declared property value: {other:?}"),
            }
        }

        impl ::core::ops::Deref for crate::specified::SpecifiedValues {
            type Target = SpecifiedTable;
            fn deref(&self) -> &SpecifiedTable {
                &self.longhands
            }
        }
        impl ::core::ops::DerefMut for crate::specified::SpecifiedValues {
            fn deref_mut(&mut self) -> &mut SpecifiedTable {
                &mut self.longhands
            }
        }
        impl ::core::ops::Deref for crate::computed::ComputedValues {
            type Target = ComputedTable;
            fn deref(&self) -> &ComputedTable {
                &self.longhands
            }
        }
        impl ::core::ops::DerefMut for crate::computed::ComputedValues {
            fn deref_mut(&mut self) -> &mut ComputedTable {
                &mut self.longhands
            }
        }

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
