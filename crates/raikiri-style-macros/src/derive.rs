//! `#[derive(Longhand)]`.

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Fields, Ident, LitBool, LitStr, Path, Type};

use crate::case::split_camel;

const KEYS: &str = "`name`, `initial`, `inherited`, `sample`, `value`, `parse`, `compute`, `computed`, `lift`, `listed`";

/// The parsed `#[longhand(..)]` attribute.
#[derive(Default)]
struct Args {
    name: Option<LitStr>,
    initial: Option<Expr>,
    inherited: Option<LitBool>,
    sample: Option<Expr>,
    value: Option<Type>,
    parse: Option<Path>,
    compute: Option<Path>,
    computed: Option<Type>,
    lift: Option<Path>,
    listed: Option<LitBool>,
}

/// Every error found in one declaration, reported together.
#[derive(Default)]
struct Errors(Option<syn::Error>);

impl Errors {
    fn push(&mut self, error: syn::Error) {
        match &mut self.0 {
            Some(first) => first.combine(error),
            None => self.0 = Some(error),
        }
    }
}

/// Stores `value` in `slot`, reporting a second occurrence of the key.
fn set_once<T>(slot: &mut Option<T>, value: T, key: &syn::Path, errors: &mut Errors) {
    if slot.is_some() {
        errors.push(syn::Error::new_spanned(key, "duplicate `#[longhand]` key"));
    } else {
        *slot = Some(value);
    }
}

/// Parses `#[longhand(..)]`. Mistakes are collected in `errors` rather than
/// returned, so the caller can still emit a best-effort impl and the
/// declaration's own error is not buried under follow-on errors from every
/// use of the missing impl.
fn parse_args(input: &DeriveInput, errors: &mut Errors) -> (Args, Span) {
    let mut args = Args::default();
    let mut attr_span = None;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("longhand")) {
        if attr_span.is_some() {
            errors.push(syn::Error::new_spanned(
                attr,
                "a type takes exactly one `#[longhand(..)]` attribute",
            ));
            continue;
        }
        attr_span = Some(attr.path().span());
        let parsed = attr.parse_nested_meta(|meta| {
            let key = &meta.path;
            if key.is_ident("name") {
                set_once(&mut args.name, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("initial") {
                set_once(&mut args.initial, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("inherited") {
                set_once(&mut args.inherited, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("sample") {
                set_once(&mut args.sample, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("value") {
                set_once(&mut args.value, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("parse") {
                set_once(&mut args.parse, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("compute") {
                set_once(&mut args.compute, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("computed") {
                set_once(&mut args.computed, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("lift") {
                set_once(&mut args.lift, meta.value()?.parse()?, key, errors);
            } else if key.is_ident("listed") {
                set_once(&mut args.listed, meta.value()?.parse()?, key, errors);
            } else {
                errors
                    .push(meta.error(format!("unknown `#[longhand]` key; expected one of {KEYS}")));
                // Skip the value so the remaining keys are still read.
                if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<Expr>()?;
                }
            }
            Ok(())
        });
        if let Err(error) = parsed {
            errors.push(error);
        }
    }
    let attr_span = attr_span.unwrap_or_else(|| {
        errors.push(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Longhand)]` needs a `#[longhand(name = .., initial = .., inherited = .., sample = ..)]` attribute",
        ));
        input.ident.span()
    });
    (args, attr_span)
}

/// A required key's value, or `fallback` after reporting it missing.
fn required<T>(slot: Option<T>, key: &str, span: Span, errors: &mut Errors, fallback: T) -> T {
    slot.unwrap_or_else(|| {
        errors.push(syn::Error::new(
            span,
            format!("`#[longhand]` is missing `{key} = ..`"),
        ));
        fallback
    })
}

/// A CSS property name: ASCII lowercase letters, digits and `-`.
fn check_name(name: &LitStr) -> syn::Result<()> {
    let value = name.value();
    let valid = !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            name,
            "a longhand name is lowercase ASCII letters, digits and `-`, e.g. \"object-fit\"",
        ))
    }
}

/// One keyword of a keyword enum: the variant and its CSS spelling.
struct Keyword {
    variant: Ident,
    css: LitStr,
}

/// The keywords of a unit-only enum: each variant's `#[css("..")]` spelling,
/// or its name in kebab case.
fn keywords(input: &DeriveInput) -> syn::Result<Vec<Keyword>> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "a `#[longhand]` without `parse = ..` must be a unit-only enum (its variants are the keywords); \
             for other value types supply `parse = some_fn`",
        ));
    };
    let mut out = Vec::with_capacity(data.variants.len());
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                &variant.fields,
                "a keyword variant takes no fields; supply `parse = some_fn` for other value types",
            ));
        }
        let mut css = None;
        for attr in variant.attrs.iter().filter(|a| a.path().is_ident("css")) {
            if css.is_some() {
                return Err(syn::Error::new_spanned(attr, "duplicate `#[css(..)]`"));
            }
            css = Some(attr.parse_args::<LitStr>()?);
        }
        let css = css.unwrap_or_else(|| {
            LitStr::new(
                &split_camel(&variant.ident.to_string(), '-'),
                variant.ident.span(),
            )
        });
        out.push(Keyword {
            variant: variant.ident.clone(),
            css,
        });
    }
    if out.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "a keyword longhand needs at least one variant",
        ));
    }
    Ok(out)
}

/// A bare identifier naming one of `input`'s own variants resolves to
/// `Self::Variant`, so `initial = Auto` reads like the property table.
fn resolve_variant(expr: Expr, input: &DeriveInput) -> TokenStream {
    if let (Expr::Path(path), Data::Enum(data)) = (&expr, &input.data)
        && path.qself.is_none()
        && let Some(ident) = path.path.get_ident()
        && data.variants.iter().any(|v| v.ident == *ident)
    {
        return quote_spanned!(ident.span()=> Self::#ident);
    }
    quote!(#expr)
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "a longhand type cannot be generic",
        ));
    }
    let mut errors = Errors::default();
    let (args, attr_span) = parse_args(&input, &mut errors);
    let unreachable: Expr = syn::parse_quote!(::core::unreachable!());
    let name = required(
        args.name,
        "name",
        attr_span,
        &mut errors,
        LitStr::new("", attr_span),
    );
    if let Err(error) = check_name(&name) {
        errors.push(error);
    }
    let initial = required(
        args.initial,
        "initial",
        attr_span,
        &mut errors,
        unreachable.clone(),
    );
    let inherited = required(
        args.inherited,
        "inherited",
        attr_span,
        &mut errors,
        LitBool::new(false, attr_span),
    );
    let sample = required(args.sample, "sample", attr_span, &mut errors, unreachable);
    if let (Some(computed), None) = (&args.computed, &args.lift) {
        errors.push(syn::Error::new_spanned(
            computed,
            "a `computed = ..` type needs `lift = some_fn` to turn a computed value back into a specified one",
        ));
    }
    if let (None, Some(lift)) = (&args.computed, &args.lift) {
        errors.push(syn::Error::new_spanned(
            lift,
            "`lift = ..` is only used with `computed = ..`; without it the computed type is the specified type",
        ));
    }

    let ident = &input.ident;
    let longhand = quote!(crate::property::Longhand);
    let cx_ty = quote!(crate::property::AbsolutizeCx<'_>);
    let parser_ty = quote!(cssparser::Parser<'_, '_>);

    let specified = match &args.value {
        Some(ty) => quote!(#ty),
        None => quote!(Self),
    };
    let computed = match &args.computed {
        Some(ty) => quote!(#ty),
        None => quote!(Self::Specified),
    };
    // The bound on `Specified` is checked at the declaration.
    let specified_item = quote_spanned!(ident.span()=> type Specified = #specified;);

    let (keyword_impl, parse_body) = match &args.parse {
        Some(parse) => {
            let body = quote_spanned! {parse.span()=>
                let parse: fn(&mut #parser_ty) -> ::core::option::Option<Self::Specified> = #parse;
                parse(input)
            };
            (TokenStream::new(), body)
        }
        None => {
            if let Some(value) = &args.value {
                errors.push(syn::Error::new_spanned(
                    value,
                    "a keyword longhand's value is the enum itself; drop `value = ..` or supply `parse = some_fn`",
                ));
            }
            let kws = match keywords(&input) {
                Ok(kws) if args.value.is_none() => kws,
                Ok(_) => Vec::new(),
                Err(error) => {
                    errors.push(error);
                    Vec::new()
                }
            };
            let variants: Vec<_> = kws.iter().map(|k| &k.variant).collect();
            let css: Vec<_> = kws.iter().map(|k| &k.css).collect();
            let keyword_impl = quote! {
                impl #ident {
                    /// Every variant, in declaration order.
                    #[cfg(test)]
                    pub(crate) const ALL: &'static [Self] = &[#(Self::#variants),*];

                    /// This variant's own keyword spelling.
                    pub const fn as_css_str(self) -> &'static str {
                        match self {
                            #(Self::#variants => #css,)*
                        }
                    }

                    /// Parses one keyword, ASCII case-insensitively.
                    pub(crate) fn from_css_ident(ident: &str) -> ::core::option::Option<Self> {
                        cssparser::match_ignore_ascii_case! { ident,
                            #(#css => ::core::option::Option::Some(Self::#variants),)*
                            _ => ::core::option::Option::None,
                        }
                    }
                }
            };
            if kws.is_empty() {
                (TokenStream::new(), quote!(::core::unreachable!()))
            } else {
                let body = quote! {
                    input.expect_ident().ok().and_then(|ident| Self::from_css_ident(ident))
                };
                (keyword_impl, body)
            }
        }
    };

    let compute_body = match &args.compute {
        Some(hook) => quote_spanned! {hook.span()=>
            let hook: fn(Self::Specified, &#cx_ty) -> Self::Computed = #hook;
            hook(specified, cx)
        },
        None => quote! {
            let _ = cx;
            specified
        },
    };
    let lift_body = match (&args.computed, &args.lift) {
        (Some(_), Some(lift)) => quote_spanned! {lift.span()=>
            let lift: fn(Self::Computed) -> Self::Specified = #lift;
            lift(computed)
        },
        (Some(_), None) => quote!(::core::unreachable!()),
        (None, _) => quote!(computed),
    };

    // A longhand must also be listed in `#[longhands(..)]`, which adds the
    // `PropertyKey` variant of the same name; without it the property would
    // compile but never be parsed. `listed = false` opts out (test fixtures).
    let listed_check = if args.listed.as_ref().is_none_or(|b| b.value) {
        quote_spanned! {ident.span()=>
            const _: crate::property::PropertyKey = crate::property::PropertyKey::#ident;
        }
    } else {
        TokenStream::new()
    };

    let initial = resolve_variant(initial, &input);
    let sample = resolve_variant(sample, &input);

    let errors = errors.0.map(syn::Error::into_compile_error);
    Ok(quote! {
        #errors
        #keyword_impl
        #listed_check

        impl #longhand for #ident {
            const NAME: &'static str = #name;
            const INHERITED: bool = #inherited;
            #specified_item
            type Computed = #computed;

            fn initial() -> Self::Specified {
                #initial
            }

            fn parse(input: &mut #parser_ty) -> ::core::option::Option<Self::Specified> {
                #parse_body
            }

            fn compute(specified: Self::Specified, cx: &#cx_ty) -> Self::Computed {
                #compute_body
            }

            fn lift(computed: Self::Computed) -> Self::Specified {
                #lift_body
            }

            #[cfg(test)]
            fn sample() -> Self::Specified {
                #sample
            }
        }
    })
}
