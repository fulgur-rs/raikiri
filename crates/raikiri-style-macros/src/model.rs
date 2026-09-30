//! Validation and defaulting: turns the [`RawEntry`] values of every
//! `properties!` block into the [`Entry`] values the generator consumes.
//!
//! Every rule violation is recorded as one error on the user's own tokens.
//! An entry with errors is still returned whenever its variant and field
//! can be declared without clashing; its unusable parts become `None`, which
//! the generator expands to `unreachable!()` bodies. That keeps the rest of
//! the crate compiling against the entry's variant, field and types, so the
//! mistake is reported once instead of at every use.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};
use quote::quote_spanned;
use syn::spanned::Spanned as _;
use syn::{Attribute, Expr, ExprPath, Ident, LitStr, Path, Type};

use crate::case::split_camel;
use crate::diag::Errors;
use crate::parse::{ComputedSpec, RawEntry, ResidueSpec, Slot};

/// One keyword of a `keywords` entry, with its spelling resolved.
pub(crate) struct Keyword {
    /// Doc comments written before the keyword.
    pub(crate) docs: Vec<Attribute>,
    /// The enum variant.
    pub(crate) ident: Ident,
    /// The CSS spelling (explicit, or the kebab case of `ident`).
    pub(crate) css: LitStr,
}

/// The specified value type of an entry.
pub(crate) enum Value {
    /// `keywords: [..]`: an enum named after the variant is generated.
    Keywords(Vec<Keyword>),
    /// `parse: <fn>` with the value type (written after `Variant:`, or the
    /// variant name itself).
    Parsed {
        /// The specified value type.
        ty: Box<Type>,
        /// The parse function; `None` after an error.
        parse: Option<ExprPath>,
    },
}

/// How a computed value is turned back into a specified one.
pub(crate) enum Lift {
    /// `Into::into` (no `lift:` written).
    Into,
    /// `lift: <path>`.
    Path(ExprPath),
    /// `lift:` was malformed (already reported).
    Broken,
}

/// How the test-only residue check inspects an entry's specified value.
pub(crate) enum Residue {
    /// `residue: none`, written or defaulted: never a length residue.
    None,
    /// `residue: <path>`.
    Path(ExprPath),
    /// `residue:` was malformed, or is missing where it is required (both
    /// already reported).
    Broken,
}

/// The derives every generated keyword enum has; `derive:` appends to them.
pub(crate) const FIXED_DERIVES: [&str; 5] = ["Clone", "Copy", "Debug", "PartialEq", "Eq"];

/// The names the per-entry type module defines itself; a written value type
/// with one of these names would resolve to the alias instead.
pub(crate) const RESERVED_TYPE_NAMES: [&str; 3] = ["Specified", "Computed", "Property"];

/// Whether `ty` is a bare single-segment path named like one of
/// [`RESERVED_TYPE_NAMES`].
pub(crate) fn is_reserved_type(ty: &Type) -> bool {
    match ty {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .get_ident()
            .is_some_and(|ident| RESERVED_TYPE_NAMES.iter().any(|name| ident == name)),
        _ => false,
    }
}

/// Reports a value type that is a bare reserved name.
fn check_reserved_type(ty: &Type, errors: &mut Errors) {
    if is_reserved_type(ty) {
        errors.push(syn::Error::new_spanned(
            ty,
            format!(
                "a value type named `{}` would resolve to the entry's own type alias; write a path such as `crate::..::{}`",
                quote::ToTokens::to_token_stream(ty),
                quote::ToTokens::to_token_stream(ty)
            ),
        ));
    }
}

/// One validated longhand.
pub(crate) struct Entry {
    /// Doc comments written before the CSS name.
    pub(crate) docs: Vec<Attribute>,
    /// The CSS property name.
    pub(crate) name: LitStr,
    /// The `PropertyValue` / `PropertyKey` variant.
    pub(crate) variant: Ident,
    /// The table field, and the name of the per-entry type module.
    pub(crate) field: Ident,
    /// The specified value.
    pub(crate) value: Value,
    /// Extra derives of the generated keyword enum (`derive:`), appended to
    /// [`FIXED_DERIVES`]; empty for a `parse:` entry.
    pub(crate) derives: Vec<Path>,
    /// The keyword marked `#[default]`: the `initial:` keyword when
    /// `derive:` lists `Default`.
    pub(crate) default_keyword: Option<Ident>,
    /// Span used for errors about the specified type (the written type, or
    /// the variant name when the type defaults to it).
    pub(crate) value_span: Span,
    /// The initial value expression; `None` after an error.
    pub(crate) initial: Option<TokenStream>,
    /// Whether the property is inherited.
    pub(crate) inherited: bool,
    /// The specified-to-computed hook (`compute:` or the hook of
    /// `computed: Type via hook`); `None` is the identity.
    pub(crate) compute: Option<ExprPath>,
    /// The computed value type when it differs from the specified one.
    pub(crate) computed_ty: Option<Box<Type>>,
    /// The computed-to-specified function. Only used when `computed_ty` is
    /// set.
    pub(crate) lift: Lift,
    /// The test-only sample value expression; `None` after an error.
    pub(crate) sample: Option<TokenStream>,
    /// The test-only length-residue check of the specified value.
    pub(crate) residue: Residue,
    /// Whether the name takes part in name lookup and parse dispatch
    /// (`false` when the name duplicates an earlier entry's).
    pub(crate) name_listed: bool,
}

/// Validates every raw entry, in order, against each other and against the
/// hand-written `PropertyValue` / `PropertyKey` variants.
pub(crate) fn build(raw: Vec<RawEntry>, hand_written: &[Ident], errors: &mut Errors) -> Vec<Entry> {
    let mut names: HashMap<String, Ident> = HashMap::new();
    let mut variants: HashMap<String, LitStr> = HashMap::new();
    let mut fields: HashMap<String, Ident> = HashMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for raw in raw {
        let mut entry = build_entry(raw, errors);
        let variant = entry.variant.to_string();
        if hand_written.contains(&entry.variant) {
            errors.push(syn::Error::new(
                entry.variant.span(),
                format!(
                    "variant `{variant}` is already a hand-written `PropertyValue` or `PropertyKey` variant"
                ),
            ));
            continue;
        }
        if let Some(earlier) = variants.get(&variant) {
            errors.push(syn::Error::new(
                entry.variant.span(),
                format!(
                    "variant `{variant}` is already declared by the {} entry",
                    quoted(earlier)
                ),
            ));
            continue;
        }
        let field = entry.field.to_string();
        if let Some(earlier) = fields.get(&field) {
            errors.push(syn::Error::new(
                entry.field.span(),
                format!(
                    "field `{field}` is already used by the `{earlier}` entry; set `field:` to another name"
                ),
            ));
            continue;
        }
        let name = entry.name.value();
        if let Some(earlier) = names.get(&name) {
            errors.push(syn::Error::new(
                entry.name.span(),
                format!("longhand \"{name}\" is already declared by the `{earlier}` entry"),
            ));
            entry.name_listed = false;
        } else {
            names.insert(name, entry.variant.clone());
        }
        variants.insert(variant, entry.name.clone());
        fields.insert(field, entry.variant.clone());
        out.push(entry);
    }
    out
}

/// `"name"` for diagnostics.
fn quoted(name: &LitStr) -> String {
    format!("\"{}\"", name.value())
}

/// Whether `s` is a well-formed lowercase CSS identifier, as used for
/// property names and keyword spellings: words of lowercase ASCII letters
/// and digits joined by single `-`, optionally after one leading `-` (a
/// vendor prefix), with the first word not starting with a digit. `-`,
/// `a-`, `a--b`, `--custom` and `2d` are rejected.
pub(crate) fn is_css_name(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    !body.is_empty()
        && !body.starts_with(|c: char| c.is_ascii_digit())
        && body.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Keeps the doc comments of `attrs` and reports any other attribute.
fn docs_only(attrs: Vec<Attribute>, what: &str, errors: &mut Errors) -> Vec<Attribute> {
    attrs
        .into_iter()
        .filter(|attr| {
            let doc = attr.path().is_ident("doc");
            if !doc {
                errors.push(syn::Error::new_spanned(
                    attr,
                    format!("only doc comments are allowed on {what}"),
                ));
            }
            doc
        })
        .collect()
}

/// A bare identifier (`Auto`), if `expr` is one.
fn bare_ident(expr: &Expr) -> Option<&Ident> {
    match expr {
        Expr::Path(path) if path.qself.is_none() && path.attrs.is_empty() => path.path.get_ident(),
        _ => None,
    }
}

fn build_entry(raw: RawEntry, errors: &mut Errors) -> Entry {
    let RawEntry {
        attrs,
        name,
        variant,
        value_ty,
        keywords,
        derive,
        initial,
        inherited,
        parse,
        compute,
        computed,
        lift,
        field,
        sample,
        residue,
    } = raw;
    let docs = docs_only(attrs, "a longhand entry", errors);
    if docs.is_empty() {
        errors.push(syn::Error::new(
            name.span(),
            format!(
                "the {} entry needs a `///` doc comment citing its specification, e.g. `/// CSS Images 3 §5.1`",
                quoted(&name)
            ),
        ));
    }
    if !is_css_name(&name.value()) {
        errors.push(syn::Error::new(
            name.span(),
            "a longhand name is lowercase ASCII letters, digits and `-`, e.g. \"object-fit\"",
        ));
    }
    let entry_name = quoted(&name);
    let missing = |key: &str, hint: &str, errors: &mut Errors| {
        errors.push(syn::Error::new(
            variant.span(),
            format!("the {entry_name} entry is missing `{key}: {hint}`"),
        ));
    };

    // Without either, the entry is reported below, and rules that depend on
    // the kind of value are not reported again.
    let value_kind_unknown = keywords.is_absent() && parse.is_absent();

    // The specified value: keywords or a parsed type.
    let value = match (&keywords, &parse) {
        (Slot::Absent, Slot::Absent) => {
            errors.push(syn::Error::new(
                variant.span(),
                format!(
                    "the {entry_name} entry needs `keywords: [..]` or `parse: <fn>` to know how its value is parsed"
                ),
            ));
            let ty = value_ty.clone().unwrap_or_else(|| ident_type(&variant));
            check_reserved_type(&ty, errors);
            Value::Parsed {
                ty: Box::new(ty),
                parse: None,
            }
        }
        (Slot::Absent, _) => {
            let ty = value_ty.clone().unwrap_or_else(|| ident_type(&variant));
            check_reserved_type(&ty, errors);
            Value::Parsed {
                ty: Box::new(ty),
                parse: parse.value().cloned(),
            }
        }
        (_, parse_slot) => {
            if let Some(key) = parse_slot.key() {
                errors.push(syn::Error::new(
                    key.span(),
                    "`parse:` cannot be combined with `keywords:`; the generated keyword enum parses itself",
                ));
            }
            if let Some(ty) = &value_ty {
                errors.push(syn::Error::new_spanned(
                    ty,
                    format!(
                        "a `keywords` entry's value type is the generated enum `{variant}`; remove `: {}`",
                        quote::ToTokens::to_token_stream(ty)
                    ),
                ));
            }
            check_reserved_type(&ident_type(&variant), errors);
            let list = match keywords {
                Slot::Present(key, list) => {
                    if list.is_empty() {
                        errors.push(syn::Error::new(
                            key.span(),
                            "`keywords:` needs at least one keyword",
                        ));
                    }
                    resolve_keywords(list, errors)
                }
                _ => Vec::new(),
            };
            Value::Keywords(list)
        }
    };
    let value_span = match &value_ty {
        Some(ty) if matches!(value, Value::Parsed { .. }) => ty.span(),
        _ => variant.span(),
    };

    let initial_written = matches!(initial, Slot::Present(..));
    let initial_expr = match initial {
        Slot::Present(_, expr) => resolve_value_expr(&expr, &value, &variant, "initial", errors),
        Slot::Invalid(_) => None,
        Slot::Absent => {
            missing("initial", "<value>", errors);
            None
        }
    };
    let (derives, default_keyword) = match derive {
        Slot::Present(key, paths) => match &value {
            Value::Keywords(list) => {
                let mut derives = check_derives(paths, errors);
                let default_keyword = derives
                    .iter()
                    .find(|path| last_segment_is(path, "Default"))
                    .and_then(|default| {
                        // A missing or unresolved `initial:` has been
                        // reported already; so has an empty keyword list.
                        if !initial_written || initial_expr.is_none() || list.is_empty() {
                            return None;
                        }
                        let initial = initial_keyword(initial_expr.as_ref());
                        let found = list
                            .iter()
                            .find(|k| initial.as_deref() == Some(&*k.ident.to_string()));
                        if found.is_none() {
                            errors.push(syn::Error::new_spanned(
                                default,
                                format!(
                                    "`Default` marks the `initial:` keyword with `#[default]`, but the {entry_name} entry's `initial:` is not one of its keywords"
                                ),
                            ));
                        }
                        found.map(|k| k.ident.clone())
                    });
                if default_keyword.is_none() {
                    // `derive(Default)` without a `#[default]` variant would
                    // only repeat the error reported above.
                    derives.retain(|path| !last_segment_is(path, "Default"));
                }
                (derives, default_keyword)
            }
            Value::Parsed { .. } => {
                // Without `keywords:` or `parse:` the entry has been reported
                // already.
                if !parse.is_absent() {
                    errors.push(syn::Error::new(
                        key.span(),
                        "`derive:` only applies to the enum a `keywords:` entry generates; derive on the value type where it is declared",
                    ));
                }
                (Vec::new(), None)
            }
        },
        Slot::Invalid(_) | Slot::Absent => (Vec::new(), None),
    };
    let inherited = match inherited {
        Slot::Present(_, value) => value,
        Slot::Invalid(_) => false,
        Slot::Absent => {
            missing("inherited", "yes | no", errors);
            false
        }
    };

    let computed_invalid = matches!(computed, Slot::Invalid(_));
    // Whether the entry has a specified-to-computed hook; `None` when a
    // malformed hook key leaves that unknown.
    let hooked = match (&compute, &computed) {
        (Slot::Present(..), _) | (_, Slot::Present(_, ComputedSpec::Via { .. })) => Some(true),
        (Slot::Invalid(_), _) | (_, Slot::Invalid(_)) => None,
        _ => Some(false),
    };
    let (computed_ty, via_hook) = match computed {
        Slot::Present(_, ComputedSpec::Via { ty, hook }) => {
            check_reserved_type(&ty, errors);
            (Some(ty), Some(hook))
        }
        _ => (None, None),
    };
    let compute = match (compute, via_hook) {
        // A malformed `compute:` has been reported already.
        (Slot::Invalid(_), via) => via,
        (Slot::Present(key, _), Some(hook)) => {
            errors.push(syn::Error::new(
                key.span(),
                "`compute:` and `computed: Type via hook` both name a hook; keep `computed: .. via ..` when the computed type differs, `compute:` otherwise",
            ));
            Some(hook)
        }
        (Slot::Present(_, hook), None) => Some(hook),
        (_, via) => via,
    };
    let lift = match lift {
        Slot::Present(key, path) => {
            if computed_ty.is_some() {
                Lift::Path(path)
            } else {
                // After a malformed `computed:`, whether a lift is needed is
                // unknown; that error is the one reported.
                if !computed_invalid {
                    errors.push(syn::Error::new(
                        key.span(),
                        "`lift:` is only used with `computed: Type via hook`; a computed value of the specified type needs no lift",
                    ));
                }
                Lift::Into
            }
        }
        Slot::Invalid(_) => Lift::Broken,
        Slot::Absent => Lift::Into,
    };

    let field = match field {
        Slot::Present(_, field) => field,
        _ => {
            let name = split_camel(&variant.to_string(), '_');
            if syn::parse_str::<Ident>(&name).is_ok() {
                Ident::new(&name, variant.span())
            } else {
                errors.push(syn::Error::new(
                    variant.span(),
                    format!(
                        "the default field name `{name}` is a Rust keyword; set `field:` to another name"
                    ),
                ));
                Ident::new(&format!("{name}_"), variant.span())
            }
        }
    };

    let sample_expr = match sample {
        Slot::Present(_, expr) => resolve_value_expr(&expr, &value, &variant, "sample", errors),
        Slot::Invalid(_) => None,
        Slot::Absent => match &value {
            Value::Keywords(list) => default_sample(
                list,
                &variant,
                &entry_name,
                errors,
                initial_keyword(initial_expr.as_ref()),
            ),
            Value::Parsed { .. } => {
                missing("sample", "<non-initial value>", errors);
                None
            }
        },
    };

    let residue = match residue {
        Slot::Present(_, ResidueSpec::None) => Residue::None,
        Slot::Present(_, ResidueSpec::Path(path)) => Residue::Path(path),
        Slot::Invalid(_) => Residue::Broken,
        // Only a keyword enum without a hook is known to carry no length:
        // the default.
        Slot::Absent if hooked == Some(false) && matches!(value, Value::Keywords(_)) => {
            Residue::None
        }
        Slot::Absent => {
            let reason = match hooked {
                // A malformed hook key or a missing `keywords:` / `parse:`
                // has been reported; whether `residue:` is required is then
                // unknown, so its absence is not reported as well.
                None if matches!(value, Value::Keywords(_)) => None,
                _ if value_kind_unknown => None,
                Some(true) => Some("it has a hook"),
                _ => Some("its value type is not a `keywords:` enum"),
            };
            if let Some(reason) = reason {
                errors.push(syn::Error::new(
                    variant.span(),
                    format!(
                        "the {entry_name} entry is missing `residue: none | <fn>`; {reason}, so say whether its specified value can carry a length that computing resolves: `none`, or a `fn(&Specified) -> Option<&'static str>` that names it"
                    ),
                ));
            }
            Residue::Broken
        }
    };

    Entry {
        docs,
        name,
        variant,
        field,
        value,
        derives,
        default_keyword,
        value_span,
        initial: initial_expr,
        inherited,
        compute,
        computed_ty,
        lift,
        sample: sample_expr,
        residue,
        name_listed: true,
    }
}

/// Whether the last segment of `path` is `name` (`Hash` and
/// `core::hash::Hash` both name `Hash`).
fn last_segment_is(path: &Path, name: &str) -> bool {
    path.segments.last().is_some_and(|s| s.ident == name)
}

/// Keeps the extra derives of `derive:`, reporting (and dropping) one the
/// generated enum already has and one listed twice. Derives are compared
/// by their last path segment.
fn check_derives(paths: Vec<Path>, errors: &mut Errors) -> Vec<Path> {
    let mut out: Vec<Path> = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(last) = path.segments.last() else {
            continue;
        };
        let name = last.ident.to_string();
        if FIXED_DERIVES.contains(&name.as_str()) {
            errors.push(syn::Error::new_spanned(
                &path,
                format!(
                    "`{name}` is always derived for a keyword enum (with {}); remove it from `derive:`",
                    FIXED_DERIVES
                        .iter()
                        .map(|d| format!("`{d}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
            continue;
        }
        if out.iter().any(|p| last_segment_is(p, &name)) {
            errors.push(syn::Error::new_spanned(
                &path,
                format!("`{name}` is listed twice in `derive:`"),
            ));
            continue;
        }
        out.push(path);
    }
    out
}

/// The keyword name of a resolved `Variant::Keyword` initial value (the
/// last path segment; `None` when the value is not a path).
fn initial_keyword(initial: Option<&TokenStream>) -> Option<String> {
    let path: syn::Path = syn::parse2(initial?.clone()).ok()?;
    path.segments.last().map(|s| s.ident.to_string())
}

/// The first keyword that is not the initial value.
fn default_sample(
    list: &[Keyword],
    variant: &Ident,
    entry_name: &str,
    errors: &mut Errors,
    initial: Option<String>,
) -> Option<TokenStream> {
    if list.is_empty() {
        return None;
    }
    match list
        .iter()
        .find(|k| initial.as_deref() != Some(&*k.ident.to_string()))
    {
        Some(k) => {
            let ident = &k.ident;
            Some(quote_spanned!(ident.span()=> #variant::#ident))
        }
        None => {
            errors.push(syn::Error::new(
                variant.span(),
                format!(
                    "the {entry_name} entry has no keyword other than its initial value; add `sample: <value>`"
                ),
            ));
            None
        }
    }
}

/// A type naming the identifier `ident`.
fn ident_type(ident: &Ident) -> Type {
    syn::parse_quote_spanned!(ident.span()=> #ident)
}

/// Resolves the spelling of every keyword and reports duplicates.
fn resolve_keywords(list: Vec<crate::parse::Keyword>, errors: &mut Errors) -> Vec<Keyword> {
    let mut out: Vec<Keyword> = Vec::with_capacity(list.len());
    for kw in list {
        let docs = docs_only(kw.attrs, "a keyword", errors);
        let css = match kw.css {
            Some(css) => {
                if !is_css_name(&css.value()) {
                    errors.push(syn::Error::new(
                        css.span(),
                        "a keyword spelling is lowercase ASCII letters, digits and `-`, e.g. \"scale-down\"",
                    ));
                }
                css
            }
            None => LitStr::new(&split_camel(&kw.ident.to_string(), '-'), kw.ident.span()),
        };
        if out.iter().any(|k| k.ident == kw.ident) {
            errors.push(syn::Error::new(
                kw.ident.span(),
                format!("keyword `{}` is listed twice", kw.ident),
            ));
            continue;
        }
        if let Some(other) = out.iter().find(|k| k.css.value() == css.value()) {
            errors.push(syn::Error::new(
                css.span(),
                format!(
                    "keyword `{}` is spelled \"{}\" like `{}`; give it another spelling with `{} = \"..\"`",
                    kw.ident,
                    css.value(),
                    other.ident,
                    kw.ident
                ),
            ));
            continue;
        }
        out.push(Keyword {
            docs,
            ident: kw.ident,
            css,
        });
    }
    out
}

/// Resolves an `initial` / `sample` value. A bare identifier names a
/// keyword of a `keywords` entry (`Auto` means `Isolation::Auto`) or an
/// associated item of the value type (`Fill` means `<ObjectFit>::Fill`, which
/// covers enum variants and associated constants); anything else is used as
/// written.
fn resolve_value_expr(
    expr: &Expr,
    value: &Value,
    variant: &Ident,
    key: &str,
    errors: &mut Errors,
) -> Option<TokenStream> {
    let Some(ident) = bare_ident(expr) else {
        return Some(quote::ToTokens::to_token_stream(expr));
    };
    match value {
        Value::Keywords(list) => {
            if list.iter().any(|k| k.ident == *ident) {
                Some(quote_spanned!(ident.span()=> #variant::#ident))
            } else {
                if !list.is_empty() {
                    let names: Vec<String> =
                        list.iter().map(|k| format!("`{}`", k.ident)).collect();
                    errors.push(syn::Error::new(
                        ident.span(),
                        format!(
                            "`{key}: {ident}` is not one of this entry's keywords: {}",
                            names.join(", ")
                        ),
                    ));
                }
                None
            }
        }
        Value::Parsed { ty, .. } => Some(quote_spanned!(ident.span()=> <#ty>::#ident)),
    }
}

#[cfg(test)]
mod tests;
