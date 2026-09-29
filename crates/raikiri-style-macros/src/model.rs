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
use syn::{Attribute, Expr, ExprPath, Ident, LitStr, Type};

use crate::case::split_camel;
use crate::diag::Errors;
use crate::parse::{ComputedSpec, RawEntry, Slot};

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
    /// The computed-to-specified function; `None` is `Into::into`. Only
    /// used when `computed_ty` is set.
    pub(crate) lift: Option<ExprPath>,
    /// The test-only sample value expression; `None` after an error.
    pub(crate) sample: Option<TokenStream>,
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
        let Some(mut entry) = build_entry(raw, errors) else {
            continue;
        };
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

/// Whether `s` is a valid CSS name: lowercase ASCII letters, digits and
/// `-`, not starting with a digit or `--` (custom properties are not
/// longhands).
fn is_css_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with("--")
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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

fn build_entry(raw: RawEntry, errors: &mut Errors) -> Option<Entry> {
    let RawEntry {
        attrs,
        name,
        variant,
        value_ty,
        keywords,
        initial,
        inherited,
        parse,
        compute,
        computed,
        lift,
        field,
        sample,
        had_errors: _,
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
            Value::Parsed {
                ty: Box::new(ty),
                parse: None,
            }
        }
        (Slot::Absent, _) => Value::Parsed {
            ty: Box::new(value_ty.clone().unwrap_or_else(|| ident_type(&variant))),
            parse: parse.value().cloned(),
        },
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

    let initial_expr = match initial {
        Slot::Present(_, expr) => resolve_value_expr(&expr, &value, &variant, "initial", errors),
        Slot::Invalid(_) => None,
        Slot::Absent => {
            missing("initial", "<value>", errors);
            None
        }
    };
    let inherited = match inherited {
        Slot::Present(_, value) => value,
        Slot::Invalid(_) => false,
        Slot::Absent => {
            missing("inherited", "yes | no", errors);
            false
        }
    };

    let (computed_ty, via_hook) = match computed {
        Slot::Present(_, ComputedSpec::Via { ty, hook }) => (Some(ty), Some(hook)),
        _ => (None, None),
    };
    let compute = match (compute, via_hook) {
        (Slot::Present(key, _) | Slot::Invalid(key), Some(hook)) => {
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
            if computed_ty.is_none() {
                errors.push(syn::Error::new(
                    key.span(),
                    "`lift:` is only used with `computed: Type via hook`; a computed value of the specified type needs no lift",
                ));
                None
            } else {
                Some(path)
            }
        }
        _ => None,
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

    Some(Entry {
        docs,
        name,
        variant,
        field,
        value,
        value_span,
        initial: initial_expr,
        inherited,
        compute,
        computed_ty,
        lift,
        sample: sample_expr,
        name_listed: true,
    })
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
                let value = css.value();
                let valid = !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if !valid {
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
