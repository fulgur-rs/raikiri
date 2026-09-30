//! Procedural macros that declare raikiri-style's CSS longhands from a
//! table.
//!
//! The only macro is [`macro@longhands`]. It is applied to the inline module
//! that declares raikiri-style's `PropertyValue` and `PropertyKey` enums,
//! reads the `properties! { .. }` tables inside it, and generates every item
//! a table-declared longhand needs. The expansion names raikiri-style's own
//! items through `crate::` paths (see [Host crate](#host-crate)), so the
//! macro is only usable from inside that crate.
//!
//! ```ignore
//! #[longhands]
//! mod decl {
//!     use super::*;
//!
//!     pub enum PropertyValue {
//!         Color(CssColor),
//!         #[key(Custom)]
//!         CustomProperty(CustomProperty),
//!         #[key(with = |value| value.key)]
//!         Deferred(DeferredValue),
//!     }
//!
//!     pub enum PropertyKey { Color, Custom }
//!
//!     properties! {
//!         /// CSS Compositing 1 §3.4.2
//!         "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },
//!         /// CSS Images 3 §5.1
//!         "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit,
//!                                     sample: Contain, residue: none },
//!         /// CSS Color 4 §3.3
//!         "opacity" => Opacity: f32 { initial: 1.0, inherited: no, parse: parse_opacity_value,
//!                                     compute: clamp_opacity, sample: -0.5, residue: none },
//!     }
//! }
//! ```
//!
//! # The module
//!
//! `#[longhands]` must be the outermost attribute of an inline module (an
//! attribute macro cannot read an out-of-line `mod decl;`). The module
//! declares `enum PropertyValue` and `enum PropertyKey` as ordinary Rust, so
//! rustfmt formats them. The macro appends one variant per table entry to
//! each enum, after the hand-written variants, so the discriminants of the
//! hand-written `PropertyKey` variants do not change.
//!
//! `PropertyValue::key()` is generated. A hand-written `PropertyValue`
//! variant maps to the `PropertyKey` variant of the same name, unless it
//! carries one of these helper attributes (removed during expansion):
//!
//! - `#[key(Other)]`: maps to `PropertyKey::Other`.
//! - `#[key(with = f)]`: for a variant with one unnamed field of type `T`,
//!   the key is `f(&payload)`. `f` is any expression that coerces to
//!   `fn(&T) -> PropertyKey`, such as a path or a non-capturing closure
//!   (`#[key(with = |value| value.key)]`).
//!
//! The module may contain any number of `properties! { .. }` blocks (one per
//! domain, say) among its other items. They are removed during expansion,
//! and their entries are declared in source order. A block must be a direct
//! item of the annotated module: one inside a nested inline module is
//! reported, and one outside any `#[longhands]` module is an unknown macro.
//! Inner attributes of the module (`//!`, `#![allow(..)]`) are kept.
//!
//! # Entries
//!
//! ```text
//! /// <doc comments citing the specification>
//! "css-name" => Variant[: ValueType] { key: value, .. },
//! ```
//!
//! Keys may appear in any order; trailing commas are optional after the last
//! key and the last entry.
//!
//! | Key | Value | Default |
//! |---|---|---|
//! | `keywords` | `[Auto, ScaleDown, Pre = "pre-line", ..]` | none |
//! | `derive` | `[Hash, Default, ..]` (paths of derive macros; `keywords` only) | none |
//! | `parse` | path to `fn(&mut cssparser::Parser) -> Option<Specified>` | none |
//! | `initial` | expression of the specified type | required |
//! | `inherited` | `yes` or `no` | required |
//! | `compute` | path to `fn(Specified, &AbsolutizeCx) -> Specified` | identity |
//! | `computed` | `as_specified`, or `Type via path` with `fn(Specified, &AbsolutizeCx) -> Type` | `as_specified` |
//! | `lift` | path to `fn(Computed) -> Specified` | `Into::into` |
//! | `field` | identifier | snake case of `Variant` |
//! | `sample` | expression of the specified type (test only) | first non-initial keyword |
//! | `residue` | `none`, or path to `fn(&Specified) -> Option<&'static str>` (test only) | `none` for a `keywords` entry without a hook; required otherwise |
//!
//! - Exactly one of `keywords` and `parse` is required. `keywords` generates
//!   a `Copy` enum named `Variant` with one variant per keyword; each
//!   keyword's CSS spelling is its name in kebab case (`ScaleDown` is
//!   `scale-down`) unless written as `Name = "spelling"`. Doc comments may
//!   precede a keyword. A `keywords` entry takes no `: ValueType`.
//! - The keyword enum always derives `Clone, Copy, Debug, PartialEq, Eq`.
//!   `derive: [..]` appends more derives (`derive: [Hash, Default]`); each
//!   is a path without generic arguments (`Hash` or `core::hash::Hash`).
//!   Derives are compared by their last path segment: one of the five
//!   above, or one listed twice, is an error. `derive` on an entry without
//!   `keywords` is an error too (derive on the value type where it is
//!   declared).
//! - With `Default` in `derive`, the keyword `initial` names (written bare
//!   or as a path, `Auto` or `Isolation::Auto`) is marked `#[default]`, so
//!   `Default::default()` is the initial value. An `initial` that is not
//!   one of the entry's keywords is then an error.
//! - Without `keywords`, the specified type is `ValueType`, or the type
//!   named `Variant` when `: ValueType` is omitted.
//! - In `initial` and `sample`, a bare identifier names a keyword of a
//!   `keywords` entry (`Auto` is `Isolation::Auto`) or an associated item of
//!   the value type (`Fill` is `<ObjectFit>::Fill`). Write a path or any
//!   other expression for anything else.
//! - The computed type equals the specified type, and `compute` (when
//!   given) maps one to the other. When they differ, `computed: Type via
//!   hook` names both the computed type and the hook; `compute` must then be
//!   omitted. `lift` turns a computed value back into a specified one, for
//!   inheritance and page-context absolutization; it only applies with
//!   `computed: Type via hook`.
//! - `sample` is the non-initial worst-case value of the page-cascade test
//!   corpus, compiled only under `cfg(test)`. Every entry without
//!   `keywords` must supply one. It also fills the test fixtures
//!   `SpecifiedTable::sample()` and `ComputedTable::sample()` (the latter
//!   through `compute` in the initial context), so choose one whose
//!   computed value differs from the computed initial value as well: a
//!   hook that maps the sample back to the initial value (a clamp, say)
//!   would leave the computed fixture at the initial value for that entry.
//! - `residue` tells the host's tests whether a specified value still
//!   carries a length that computing resolves (a font-relative unit, say):
//!   `none` when the value never carries one, or a function that returns
//!   the residue it finds (a name for the failure message) and `None`
//!   otherwise. It is compiled only under `cfg(test)`, so the function is
//!   defined under `#[cfg(test)]` too. A `keywords` entry without a hook
//!   defaults to `none`, since a generated keyword enum carries no length.
//!   Every other entry must write it, as a missing `residue` is an error on
//!   the entry: an entry with a hook (`compute` or `computed: Type via
//!   hook`) has a computed value that differs from the specified one, and
//!   the macro cannot tell whether a written value type carries lengths.
//!   Defaulting either case to `none` would hide a length that computing
//!   should have resolved.
//! - Each entry needs at least one doc comment (the specification
//!   reference). Other attributes are rejected.
//! - Names are plain identifiers: Rust keywords and raw identifiers are
//!   rejected. The default field splits the variant at word boundaries and
//!   keeps acronyms together (`HTMLMode` becomes `html_mode`); a default
//!   that is a Rust keyword must be replaced with `field:`.
//! - CSS names and keyword spellings are lowercase words of ASCII letters
//!   and digits joined by single `-`, optionally after one leading `-`
//!   (vendor prefixes).
//!
//! Hook, parser and residue paths are coerced to the fn-pointer types
//! above, so a wrong signature is reported at the path in the table (for
//! `residue`, only in builds with `cfg(test)`, such as `cargo test` or
//! `clippy --all-targets`).
//!
//! # Generated items
//!
//! In the annotated module, for each entry (`field` is the entry's field
//! name):
//!
//! - the `PropertyValue::Variant(field::Specified)` and `PropertyKey::Variant`
//!   variants, documented from the entry's doc comments;
//! - for `keywords`, `#[non_exhaustive] enum Variant` with the derives
//!   described above, `as_css_str`, `from_css_ident` (ASCII
//!   case-insensitive) and a `cfg(test)` `ALL`;
//! - `pub mod field` with `type Specified` and `type Computed` (aliases of
//!   the written types, so rustdoc shows the real payload types) and
//!   `Property`, an uninhabited marker implementing `crate::property::Longhand`.
//!
//!   The aliases repeat the written types inside `field`, which imports the
//!   annotated module with `use super::*`. Plain and crate-rooted paths
//!   therefore resolve as written, but a `self::` or `super::` path in a
//!   value type (`: ValueType` or `computed: Type via ..`) would be resolved
//!   one module too deep; write those types without the relative prefix.
//!   A value type named bare `Specified`, `Computed` or `Property` is
//!   rejected, since it would name the alias itself.
//!
//! and once for the module:
//!
//! - `PropertyValue::key()`;
//! - `SpecifiedTable` and `ComputedTable` (`#[non_exhaustive]`, one field per
//!   entry) with `SpecifiedTable::{initial, inherit_from, absolutize, apply}`
//!   and `ComputedTable::initial`, and a read-only `Deref` from
//!   `SpecifiedValues` / `ComputedValues` to them. No `DerefMut` is
//!   generated: a write names the table (`values.longhands.field = ..` or
//!   `values.longhands.apply(..)`), so a write can neither hide that it
//!   goes to the table nor land in a same-named host field that reads
//!   would not see;
//! - `longhand_page_absolutize(value, cx)`: `lift(compute(value))` for one
//!   table value;
//! - `longhand_key_for_name(name)` and `parse_longhand_value(name, input)`,
//!   the fall-through targets of the hand-written name lookup and parse
//!   dispatch, and `LONGHAND_NAMES`;
//! - `longhand_value_pat!()`, a pattern matching every table variant
//!   (omitted when no entry could be declared, since a pattern must match
//!   at least one variant);
//! - under `cfg(test)`: `longhand_samples()`, `longhand_sample(key)`,
//!   `with_longhand_samples!` and `with_longhand_variants!`;
//!   `SpecifiedTable::sample()` (every field at its `sample`) and
//!   `ComputedTable::sample()` (every `sample` computed in the initial
//!   context), for fixtures that must hold non-initial values; and
//!   `equal_fields(&self, other)` on both tables, the CSS names of the
//!   entries whose fields are equal, for checking such a fixture against
//!   `initial()`; and `longhand_specified_residue(value)`, the length a
//!   declared value's specified form still carries, if any: a match with
//!   one arm per entry, each `None` for `residue: none` or a call of the
//!   entry's residue function (every entry has an answer, written or
//!   defaulted, so a variant cannot be left out), for a test that checks
//!   values after computing.
//!
//! A host's tests need not use every one of these `cfg(test)` items: rustc
//! does not report `dead_code` inside another crate's macro expansion, so
//! an unused one passes `clippy --all-targets -D warnings` without an
//! `allow`. A host function reached only from an unused generated item
//! (a residue function, say) is reported as dead code, though.
//!
//! # Host crate
//!
//! The expansion assumes these items of the crate it is used in:
//!
//! - `crate::property::Longhand`, a trait with `const NAME: &'static str`,
//!   `const INHERITED: bool`, `type Specified`, `type Computed`,
//!   `fn initial() -> Specified`,
//!   `fn parse(&mut cssparser::Parser<'_, '_>) -> Option<Specified>`,
//!   `fn compute(Specified, &AbsolutizeCx<'_>) -> Computed`,
//!   `fn lift(Computed) -> Specified` and `#[cfg(test)] fn sample() -> Specified`;
//! - `crate::property::AbsolutizeCx<'a>` with
//!   `AbsolutizeCx::initial(&ResolveContext) -> AbsolutizeCx<'_>`, and
//!   `crate::resolve::ResolveContext::initial()`;
//! - `crate::property::{PropertyValue, PropertyKey, longhand_sample}`
//!   reachable (a re-export of the annotated module), for the generated
//!   `macro_rules!` that expand elsewhere;
//! - `crate::specified::SpecifiedValues` and
//!   `crate::computed::ComputedValues`, each with a `longhands` field of the
//!   generated table type;
//! - the `cssparser` crate as a dependency, and `Debug` on `PropertyValue`
//!   and `PropertyKey` (the generated `unreachable!` messages of `apply`,
//!   `longhand_page_absolutize`, `longhand_sample` and
//!   `longhand_specified_residue` format them with `{:?}`);
//! - `Clone`, `Debug` and `PartialEq` on every specified and computed value
//!   type (the `Longhand` bounds, and the derives of the generated tables).
//!
//! ## Wiring the module
//!
//! These rules come from name resolution rather than from the macro, so the
//! compiler reports a violation, not the macro:
//!
//! - **No glob import of the parent in the enclosing file module.** When
//!   the annotated inline module sits in a file module of the same name
//!   (`property/decl.rs` holding `#[longhands] mod decl { .. }`), that file
//!   module must not `use super::*`. The glob would import the file
//!   module's own name `decl` from its parent, and the inline `decl` is a
//!   macro-expanded item, which cannot shadow a glob import: every use of
//!   `decl` (including a `pub use decl::*` re-export) becomes ambiguous
//!   (E0659). Import what the table and the hand-written enums need by
//!   name or from sibling modules instead (`use super::types::*;`,
//!   `use super::parse::*;`). Inside the annotated module itself,
//!   `use super::*` is fine.
//! - **Parser and hook visibility.** `parse:`, `compute:`, `computed: ..
//!   via`, `lift:` and `residue:` paths are resolved in the annotated
//!   module, so each function must be visible there. A parser defined in a
//!   sibling module
//!   (such as a `parse/` submodule, where parsers are typically
//!   `pub(super)`) must be widened to the common ancestor, e.g.
//!   `pub(in crate::property)`; a missing widening is a privacy error at the
//!   path in the table.
//! - **Generated names.** Each entry adds `pub mod <field>` (the value type
//!   aliases and the `Property` marker) and, for `keywords`, `enum
//!   <Variant>` to the annotated module. A module, type or trait of the
//!   same name already in that module is a duplicate definition. Where the
//!   annotated module is re-exported with a glob (`pub use decl::*`), an
//!   item of the same name in the re-exporting module is not an error: it
//!   silently shadows the generated one there, and a glob-imported item of
//!   that name from elsewhere makes the name ambiguous at its uses. Give
//!   the entry another `field:` or rename the existing item.
//!
//! # Diagnostics
//!
//! The macro never panics. Every mistake it detects is a compile error on
//! the tokens that caused it, and it aims for one error per mistake:
//! parsing resumes at the next key or entry; an entry with an error is
//! still declared, with its broken parts expanded to `unreachable!()`;
//! rules that relate keys are not reported again for a key that is already
//! malformed; and the module-level items are generated even when no entry
//! survives. A duplicate CSS name, variant or field is reported at the
//! later entry. Mistakes the compiler finds in user expressions (a wrong
//! `initial` type, a hook with the wrong signature) are reported by rustc at
//! those expressions.
//!
//! Follow-on errors remain in these cases: an entry that cannot be declared
//! at all (a broken `"name" => Variant` head, or a variant or field that
//! duplicates another) leaves the uses of its variant and field
//! unresolved; and when no entry survives, each use of the omitted
//! `longhand_value_pat!` fails too.

mod case;
mod diag;
mod expand;
mod generate;
mod model;
mod parse;
#[cfg(test)]
mod tests;

use proc_macro::TokenStream;

/// Declares the longhands of the annotated inline module from its
/// `properties! { .. }` tables; see the [crate documentation](crate) for
/// the table syntax, the generated items and the host-crate requirements.
#[proc_macro_attribute]
pub fn longhands(args: TokenStream, item: TokenStream) -> TokenStream {
    expand::expand(args.into(), item.into()).into()
}
