//! Parsing of `properties! { .. }` bodies.
//!
//! The grammar of one entry is
//!
//! ```text
//! <outer doc comments>
//! "css-name" => Variant [: ValueType] { key: value, .. } [,]
//! ```
//!
//! Parsing is resilient: a mistake inside an entry's braces is recorded and
//! parsing resumes at the next `key:`; a mistake in an entry's head skips to
//! the next `"css-name" =>` (or the doc comment in front of it). Each
//! mistake therefore produces exactly one error, and every entry that has a
//! readable head still reaches the model, so the expansion can declare its
//! variant and field and the rest of the crate keeps compiling.
//!
//! This module only checks the shape of each value. Rules that relate keys
//! or entries to each other live in [`crate::model`].

use proc_macro2::{Spacing, TokenStream};
use syn::buffer::Cursor;
use syn::ext::IdentExt as _;
use syn::parse::{ParseStream, Parser as _};
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, ExprPath, Ident, LitBool, LitStr, Path, Token, Type, bracketed, token};

use crate::diag::{Errors, display};

/// Every key an entry accepts, in the order the documentation lists them.
pub(crate) const KEYS: [&str; 12] = [
    "keywords",
    "derive",
    "initial",
    "inherited",
    "parse",
    "compute",
    "computed",
    "lift",
    "serialize",
    "field",
    "sample",
    "residue",
];

/// The list of valid keys as a diagnostic fragment: "`keywords`, `initial`, ..".
pub(crate) fn key_list() -> String {
    KEYS.iter()
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One key of an entry as written.
#[derive(Debug)]
pub(crate) enum Slot<T> {
    /// The key was not written.
    Absent,
    /// The key was written but its value was malformed; the error has
    /// already been reported, so the key must not also be reported missing.
    Invalid(Ident),
    /// The key and its parsed value.
    Present(Ident, T),
}

impl<T> Slot<T> {
    /// The key as written, when it was written at all.
    pub(crate) fn key(&self) -> Option<&Ident> {
        match self {
            Self::Absent => None,
            Self::Invalid(key) | Self::Present(key, _) => Some(key),
        }
    }

    /// The parsed value, when the key was written and well-formed.
    pub(crate) fn value(&self) -> Option<&T> {
        match self {
            Self::Present(_, value) => Some(value),
            _ => None,
        }
    }

    /// Whether the key was not written.
    pub(crate) fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

/// One keyword of `keywords: [..]`.
#[derive(Debug)]
pub(crate) struct Keyword {
    /// Outer attributes (doc comments) written before the keyword.
    pub(crate) attrs: Vec<Attribute>,
    /// The enum variant.
    pub(crate) ident: Ident,
    /// The explicit spelling from `Variant = "spelling"`.
    pub(crate) css: Option<LitStr>,
}

/// The value of `residue:`.
#[derive(Debug)]
pub(crate) enum ResidueSpec {
    /// `residue: none`: the specified value never carries a length.
    None,
    /// `residue: path`: a `fn(&Specified) -> Option<&'static str>` that
    /// names the length the specified value still carries, if any.
    Path(ExprPath),
}

/// The value of `serialize:`.
#[derive(Debug)]
pub(crate) enum SerializeSpec {
    /// `serialize: keyword`: the generated keyword enum's `as_css_str`. The
    /// word is kept for the error when the entry has no keyword enum.
    Keyword(Ident),
    /// `serialize: path`: a `fn(&Specified) -> Option<String>`.
    Path(ExprPath),
}

/// One entry as written, before validation.
#[derive(Debug)]
pub(crate) struct RawEntry {
    /// Outer attributes (doc comments) written before the CSS name.
    pub(crate) attrs: Vec<Attribute>,
    /// The CSS property name.
    pub(crate) name: LitStr,
    /// The `PropertyValue` / `PropertyKey` variant.
    pub(crate) variant: Ident,
    /// The value type after `Variant:`.
    pub(crate) value_ty: Option<Type>,
    /// `keywords: [..]`.
    pub(crate) keywords: Slot<Vec<Keyword>>,
    /// `derive: [Path, ..]`: extra derives of the generated keyword enum.
    pub(crate) derive: Slot<Vec<Path>>,
    /// `initial: <expr>`.
    pub(crate) initial: Slot<Expr>,
    /// `inherited: yes | no`.
    pub(crate) inherited: Slot<bool>,
    /// `parse: <path>`.
    pub(crate) parse: Slot<ExprPath>,
    /// `compute: <path>`.
    pub(crate) compute: Slot<ExprPath>,
    /// `computed: Type`: the computed value type, produced by the entry's
    /// `compute:` hook.
    pub(crate) computed: Slot<Box<Type>>,
    /// `lift: <path>`.
    pub(crate) lift: Slot<ExprPath>,
    /// `serialize: keyword | <path>`.
    pub(crate) serialize: Slot<SerializeSpec>,
    /// `field: <ident>`.
    pub(crate) field: Slot<Ident>,
    /// `sample: <expr>`.
    pub(crate) sample: Slot<Expr>,
    /// `residue: none | <path>`.
    pub(crate) residue: Slot<ResidueSpec>,
}

/// Parses the body of one `properties! { .. }` invocation. Every mistake is
/// recorded in `errors`; the entries whose head could be read are returned.
pub(crate) fn parse_block(tokens: TokenStream, errors: &mut Errors) -> Vec<RawEntry> {
    let parser = |input: ParseStream| -> syn::Result<Vec<RawEntry>> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            match parse_entry(input, errors) {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    errors.push(error);
                    skip_to_entry_start(input);
                }
            }
        }
        Ok(entries)
    };
    match parser.parse2(tokens) {
        Ok(entries) => entries,
        Err(error) => {
            // Unreachable in practice: the loop consumes the whole input.
            errors.push(error);
            Vec::new()
        }
    }
}

/// Whether `cursor` is at `"name" =>` or at the `#` of an attribute (the doc
/// comment in front of an entry).
fn at_entry_start(cursor: Cursor<'_>) -> bool {
    if let Some((punct, _)) = cursor.punct() {
        return punct.as_char() == '#';
    }
    let Some((lit, rest)) = cursor.literal() else {
        return false;
    };
    if !lit.to_string().ends_with('"') {
        return false;
    }
    matches!(
        rest.punct(),
        Some((eq, rest)) if eq.as_char() == '=' && eq.spacing() == Spacing::Joint
            && matches!(rest.punct(), Some((gt, _)) if gt.as_char() == '>')
    )
}

/// Skips at least one token tree, then up to the start of the next entry.
fn skip_to_entry_start(input: ParseStream) {
    let _ = input.step(|cursor| {
        let mut rest = *cursor;
        if let Some((_, next)) = rest.token_tree() {
            rest = next;
        }
        while !rest.eof() && !at_entry_start(rest) {
            match rest.token_tree() {
                Some((_, next)) => rest = next,
                None => break,
            }
        }
        Ok(((), rest))
    });
}

fn parse_entry(input: ParseStream, errors: &mut Errors) -> syn::Result<RawEntry> {
    let attrs = input.call(Attribute::parse_outer)?;
    if !input.peek(LitStr) {
        return Err(
            input.error("expected a longhand entry: `\"css-name\" => Variant { key: value, .. }`")
        );
    }
    let name: LitStr = input.parse()?;
    input.parse::<Token![=>]>()?;
    let variant = plain_ident(
        input,
        "expected the `PropertyValue` variant name after `=>`, e.g. `ObjectFit`",
    )?;
    let value_ty = if input.peek(Token![:]) {
        input.parse::<Token![:]>()?;
        Some(input.parse::<Type>()?)
    } else {
        None
    };
    if !input.peek(token::Brace) {
        return Err(input.error("expected `{ key: value, .. }` after the variant name"));
    }
    let content;
    syn::braced!(content in input);
    let mut entry = RawEntry {
        attrs,
        name,
        variant,
        value_ty,
        keywords: Slot::Absent,
        derive: Slot::Absent,
        initial: Slot::Absent,
        inherited: Slot::Absent,
        parse: Slot::Absent,
        compute: Slot::Absent,
        computed: Slot::Absent,
        lift: Slot::Absent,
        serialize: Slot::Absent,
        field: Slot::Absent,
        sample: Slot::Absent,
        residue: Slot::Absent,
    };
    parse_keys(&content, &mut entry, errors);

    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    } else if !input.is_empty() {
        errors.push(input.error("expected `,` after this entry's `}`"));
        if !at_entry_start(input.cursor()) {
            skip_to_entry_start(input);
        }
    }
    Ok(entry)
}

/// Whether `cursor` is at `ident :` (a single colon, not `::`).
fn at_key_start(cursor: Cursor<'_>) -> bool {
    let Some((_, rest)) = cursor.ident() else {
        return false;
    };
    matches!(rest.punct(), Some((colon, _)) if colon.as_char() == ':' && colon.spacing() == Spacing::Alone)
}

/// Skips to the next `key:` of the body, consuming a `,` in front of it.
fn skip_to_next_key(content: ParseStream) {
    let _ = content.step(|cursor| {
        let mut rest = *cursor;
        loop {
            if rest.eof() || at_key_start(rest) {
                break;
            }
            if let Some((comma, after)) = rest.punct()
                && comma.as_char() == ','
                && (after.eof() || at_key_start(after))
            {
                rest = after;
                break;
            }
            match rest.token_tree() {
                Some((_, next)) => rest = next,
                None => break,
            }
        }
        Ok(((), rest))
    });
}

fn parse_keys(content: ParseStream, entry: &mut RawEntry, errors: &mut Errors) {
    while !content.is_empty() {
        let before = content.cursor();
        if let Err(error) = parse_key(content, entry, errors) {
            errors.push(error);
            skip_to_next_key(content);
            if content.cursor() == before {
                // Guarantee progress on input the recovery cannot classify.
                let _ = content.step(|cursor| match cursor.token_tree() {
                    Some((_, next)) => Ok(((), next)),
                    None => Ok(((), *cursor)),
                });
            }
        }
    }
}

/// Stores a parsed key value, reporting a repeated key. Returns whether
/// the value was stored; otherwise the error is recorded and the rest of
/// the value has been skipped.
fn store<T>(
    slot: &mut Slot<T>,
    key: Ident,
    value: syn::Result<T>,
    content: ParseStream,
    errors: &mut Errors,
) -> bool {
    if !slot.is_absent() {
        errors.push(syn::Error::new(
            key.span(),
            format!("duplicate key `{key}` in this entry"),
        ));
        skip_to_next_key(content);
        return false;
    }
    match value {
        Ok(value) => {
            *slot = Slot::Present(key, value);
            true
        }
        Err(error) => {
            *slot = Slot::Invalid(key);
            errors.push(error);
            skip_to_next_key(content);
            false
        }
    }
}

fn parse_key(content: ParseStream, entry: &mut RawEntry, errors: &mut Errors) -> syn::Result<()> {
    let key = content.call(Ident::parse_any).map_err(|_| {
        content.error(format!(
            "expected `key: value`; the keys are {}",
            key_list()
        ))
    })?;
    if !content.peek(Token![:]) || content.peek(Token![::]) {
        return Err(content.error(format!("expected `:` after `{key}`")));
    }
    content.parse::<Token![:]>()?;
    let k = key.clone();
    let stored = match key.to_string().as_str() {
        "keywords" => store(
            &mut entry.keywords,
            k,
            parse_keywords(content),
            content,
            errors,
        ),
        "derive" => store(
            &mut entry.derive,
            k,
            parse_derives(content),
            content,
            errors,
        ),
        "initial" => store(
            &mut entry.initial,
            k,
            content.parse::<Expr>(),
            content,
            errors,
        ),
        "inherited" => store(
            &mut entry.inherited,
            k,
            parse_yes_no(content),
            content,
            errors,
        ),
        "parse" => store(&mut entry.parse, k, parse_fn_path(content), content, errors),
        "compute" => {
            // A `via:` key stands in for a missing `compute:` (see below);
            // a real `compute:` after it replaces the stand-in.
            if matches!(&entry.compute, Slot::Invalid(stand_in) if stand_in == "via") {
                entry.compute = Slot::Absent;
            }
            store(
                &mut entry.compute,
                k,
                parse_fn_path(content),
                content,
                errors,
            )
        }
        "computed" => store(
            &mut entry.computed,
            k,
            parse_computed(content),
            content,
            errors,
        ),
        "lift" => store(&mut entry.lift, k, parse_fn_path(content), content, errors),
        "serialize" => store(
            &mut entry.serialize,
            k,
            parse_serialize(content),
            content,
            errors,
        ),
        "field" => {
            let value = plain_ident(content, "expected a field name, e.g. `field: object_fit`");
            store(&mut entry.field, k, value, content, errors)
        }
        "sample" => store(
            &mut entry.sample,
            k,
            content.parse::<Expr>(),
            content,
            errors,
        ),
        "residue" => store(
            &mut entry.residue,
            k,
            parse_residue(content),
            content,
            errors,
        ),
        "via" => {
            errors.push(syn::Error::new(
                key.span(),
                "`via` is not a key; name the hook with `compute: <fn>`, and a computed type that differs from the specified one with `computed: Type`",
            ));
            // The entry did name a hook, just under the wrong key: mark
            // `compute:` as malformed so that `computed: Type` is not also
            // reported as missing its hook.
            if entry.compute.is_absent() {
                entry.compute = Slot::Invalid(key.clone());
            }
            skip_to_next_key(content);
            false
        }
        _ => {
            errors.push(syn::Error::new(
                key.span(),
                format!("unknown key `{key}`; expected one of {}", key_list()),
            ));
            skip_to_next_key(content);
            false
        }
    };
    if !stored || content.is_empty() {
        return Ok(());
    }
    if content.peek(Token![,]) {
        content.parse::<Token![,]>()?;
        return Ok(());
    }
    Err(content.error(format!(
        "unexpected token after the value of `{key}`; separate keys with `,`"
    )))
}

/// An identifier that the expansion can use as a variant, field or module
/// name: not a Rust keyword and not a raw identifier.
fn plain_ident(input: ParseStream, expected: &str) -> syn::Result<Ident> {
    if input.peek(Ident) {
        let ident: Ident = input.parse()?;
        if ident.to_string().starts_with("r#") {
            return Err(syn::Error::new(
                ident.span(),
                "raw identifiers are not supported here; choose a name that is not a Rust keyword",
            ));
        }
        return Ok(ident);
    }
    if let Some((ident, _)) = input.cursor().ident() {
        return Err(syn::Error::new(
            ident.span(),
            format!("`{ident}` is a Rust keyword; choose another name"),
        ));
    }
    Err(input.error(expected))
}

fn parse_keywords(content: ParseStream) -> syn::Result<Vec<Keyword>> {
    if !content.peek(token::Bracket) {
        return Err(content.error("expected a keyword list, e.g. `keywords: [Auto, Isolate]`"));
    }
    let inner;
    bracketed!(inner in content);
    let list = Punctuated::<Keyword, Token![,]>::parse_terminated_with(&inner, |input| {
        let attrs = input.call(Attribute::parse_outer)?;
        let ident = plain_ident(
            input,
            "expected a keyword variant, e.g. `Auto` or `ScaleDown = \"scale-down\"`",
        )?;
        let css = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<LitStr>().map_err(|_| {
                input.error("expected the keyword spelling as a string, e.g. `= \"scale-down\"`")
            })?)
        } else {
            None
        };
        Ok(Keyword { attrs, ident, css })
    })?;
    Ok(list.into_iter().collect())
}

/// `[Path, ..]`: derive macro paths without generic arguments (`Hash`,
/// `core::hash::Hash`).
fn parse_derives(content: ParseStream) -> syn::Result<Vec<Path>> {
    if !content.peek(token::Bracket) {
        return Err(content.error("expected a list of derives, e.g. `derive: [Hash, Default]`"));
    }
    let inner;
    bracketed!(inner in content);
    let list = Punctuated::<Path, Token![,]>::parse_terminated_with(&inner, |input| {
        input.call(Path::parse_mod_style).map_err(|_| {
            input.error("expected the path of a derive macro, e.g. `Hash` or `core::hash::Hash`")
        })
    });
    if list.is_err() {
        // Consume the rest of the list, so that syn does not report its
        // unparsed tokens as a second error.
        let _ = inner.step(|cursor| {
            let mut rest = *cursor;
            while let Some((_, next)) = rest.token_tree() {
                rest = next;
            }
            Ok(((), rest))
        });
    }
    Ok(list?.into_iter().collect())
}

fn parse_yes_no(content: ParseStream) -> syn::Result<bool> {
    if content.peek(LitBool) {
        let lit: LitBool = content.parse()?;
        let word = if lit.value { "yes" } else { "no" };
        return Err(syn::Error::new(
            lit.span(),
            format!(
                "expected `yes` or `no`; write `{word}` instead of `{}`",
                lit.value
            ),
        ));
    }
    let ident = content
        .call(Ident::parse_any)
        .map_err(|_| content.error("expected `yes` or `no`"))?;
    match ident.to_string().as_str() {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(syn::Error::new(ident.span(), "expected `yes` or `no`")),
    }
}

fn parse_fn_path(content: ParseStream) -> syn::Result<ExprPath> {
    let fork = content.fork();
    match fork.parse::<ExprPath>() {
        Ok(_) => content.parse::<ExprPath>(),
        Err(_) => Err(content.error("expected a path to a function, e.g. `parse_object_fit`")),
    }
}

fn parse_computed(content: ParseStream) -> syn::Result<Box<Type>> {
    if let Some((ident, rest)) = content.cursor().ident()
        && ident == "as_specified"
        && (rest.eof() || matches!(rest.punct(), Some((comma, _)) if comma.as_char() == ','))
    {
        return Err(syn::Error::new(
            ident.span(),
            "omit `computed:`; the computed type defaults to the specified type",
        ));
    }
    let ty: Type = content.parse().map_err(|_| {
        content.error("expected the computed value type, e.g. `computed: ComputedLength`")
    })?;
    if let Some((via, _)) = content.cursor().ident()
        && via == "via"
    {
        return Err(via_error(content, &ty, &via));
    }
    Ok(Box::new(ty))
}

/// The error for `computed: Type via hook`: the hook has its own key. The
/// suggestion repeats the written type and hook when the hook is a path.
fn via_error(content: ParseStream, ty: &Type, via: &Ident) -> syn::Error {
    let fork = content.fork();
    let hook = fork
        .call(Ident::parse_any)
        .and_then(|_| fork.parse::<ExprPath>())
        .ok()
        .filter(|_| fork.is_empty() || fork.peek(Token![,]));
    let ty = display(ty);
    let hook = hook.map_or_else(|| "<fn>".to_owned(), |hook| display(&hook));
    syn::Error::new(
        via.span(),
        format!(
            "`computed:` takes only the computed type; name the hook with its own key: `computed: {ty}, compute: {hook}`"
        ),
    )
}

/// `none`, or a path to a function (`object_position_residue`).
fn parse_residue(content: ParseStream) -> syn::Result<ResidueSpec> {
    const EXPECTED: &str = "expected `none` or a path to a `fn(&Specified) -> Option<&'static str>`, e.g. `residue: object_position_residue`";
    if let Some((ident, rest)) = content.cursor().ident() {
        let ends = rest.eof() || matches!(rest.punct(), Some((comma, _)) if comma.as_char() == ',');
        if ends && ident == "none" {
            content.call(Ident::parse_any)?;
            return Ok(ResidueSpec::None);
        }
        if ends && ident == "None" {
            return Err(syn::Error::new(
                ident.span(),
                "write `residue: none` (lowercase) for a value that never carries a length",
            ));
        }
    }
    let fork = content.fork();
    match fork.parse::<ExprPath>() {
        Ok(_) if fork.is_empty() || fork.peek(Token![,]) => {
            Ok(ResidueSpec::Path(content.parse::<ExprPath>()?))
        }
        _ => Err(content.error(EXPECTED)),
    }
}

/// `keyword`, or a path to a function (`serialize_object_position`).
fn parse_serialize(content: ParseStream) -> syn::Result<SerializeSpec> {
    const EXPECTED: &str = "expected `keyword` or a path to a `fn(&Specified) -> Option<String>`, e.g. `serialize: serialize_object_position`";
    if let Some((ident, rest)) = content.cursor().ident() {
        let ends = rest.eof() || matches!(rest.punct(), Some((comma, _)) if comma.as_char() == ',');
        if ends && ident == "keyword" {
            return Ok(SerializeSpec::Keyword(content.call(Ident::parse_any)?));
        }
        if ends && ident == "keywords" {
            return Err(syn::Error::new(
                ident.span(),
                "write `serialize: keyword` (singular) to serialize the keyword enum through `as_css_str`",
            ));
        }
        if ends && (ident == "none" || ident == "None") {
            return Err(syn::Error::new(
                ident.span(),
                "omit `serialize:` for a value that has no serialization (callers then echo the input)",
            ));
        }
    }
    let fork = content.fork();
    match fork.parse::<ExprPath>() {
        Ok(_) if fork.is_empty() || fork.peek(Token![,]) => {
            Ok(SerializeSpec::Path(content.parse::<ExprPath>()?))
        }
        _ => Err(content.error(EXPECTED)),
    }
}

#[cfg(test)]
mod tests;
