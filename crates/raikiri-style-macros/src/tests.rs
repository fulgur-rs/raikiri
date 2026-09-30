//! Expansion tests through the internal entry point: diagnostics rendered
//! the way the compiler prints them, and the generated code, both pinned
//! with snapshots.

use std::fmt::Write as _;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

use crate::diag::Errors;
use crate::expand::expand_collecting;

/// Expands `#[longhands]` (with `args`) over `src`.
fn expand_with_args(args: &str, src: &str) -> (TokenStream, Vec<syn::Error>) {
    let mut errors = Errors::default();
    let args: TokenStream = args.parse().expect("test args tokenize");
    let item: TokenStream = src.parse().expect("test input tokenizes");
    let expanded = expand_collecting(args, item, &mut errors);
    (expanded, errors.into_vec())
}

fn expand(src: &str) -> (TokenStream, Vec<syn::Error>) {
    expand_with_args("", src)
}

/// A `#[longhands]` module with two hand-written variants around `table`
/// (the body of one `properties!` block).
fn module(table: &str) -> String {
    format!(
        r#"mod decl {{
    pub enum PropertyValue {{
        Color(u32),
        #[key(Custom)]
        CustomProperty(String),
    }}
    pub enum PropertyKey {{
        Color,
        Custom,
    }}
    properties! {{
{table}
    }}
}}
"#
    )
}

/// Renders `errors` like rustc does: message, location and a caret line
/// under the spanned source.
fn render(src: &str, errors: &[syn::Error]) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    for error in errors {
        let span = error.span();
        let (start, end) = (span.start(), span.end());
        let line = lines
            .get(start.line.saturating_sub(1))
            .copied()
            .unwrap_or("");
        let width = if end.line == start.line {
            end.column.saturating_sub(start.column).max(1)
        } else {
            line.chars().count().saturating_sub(start.column).max(1)
        };
        let _ = writeln!(out, "error: {error}");
        let _ = writeln!(out, "  --> {}:{}", start.line, start.column + 1);
        let _ = writeln!(out, "   |");
        let _ = writeln!(out, "{:>2} | {line}", start.line);
        let _ = writeln!(
            out,
            "   | {}{}",
            " ".repeat(start.column),
            "^".repeat(width)
        );
        let _ = writeln!(out);
    }
    out
}

/// Expands `src` and renders its diagnostics.
fn diagnostics(src: &str) -> (String, usize) {
    let (_, errors) = expand(src);
    (render(src, &errors), errors.len())
}

/// A readable layout of generated tokens for snapshots: one statement,
/// field, match arm or attribute per line, braces indented.
pub(crate) fn pretty(tokens: TokenStream) -> String {
    struct Printer {
        out: String,
        indent: usize,
        at_line_start: bool,
        glue: bool,
    }
    impl Printer {
        fn word(&mut self, text: &str) {
            if self.at_line_start {
                self.out.push_str(&"    ".repeat(self.indent));
                self.at_line_start = false;
            } else if !self.glue {
                self.out.push(' ');
            }
            self.out.push_str(text);
            self.glue = false;
        }
        fn newline(&mut self) {
            if !self.at_line_start {
                self.out.push('\n');
                self.at_line_start = true;
            }
        }
        fn stream(&mut self, tokens: TokenStream) {
            let mut after_hash = false;
            for tt in tokens {
                match tt {
                    TokenTree::Group(group) => match group.delimiter() {
                        Delimiter::Brace => {
                            self.word("{");
                            self.newline();
                            self.indent += 1;
                            self.stream(group.stream());
                            self.indent -= 1;
                            self.newline();
                            self.word("}");
                            self.newline();
                        }
                        _ => {
                            let text = TokenTree::Group(group).to_string();
                            self.word(&text);
                            if after_hash {
                                self.newline();
                            }
                        }
                    },
                    TokenTree::Punct(punct) => {
                        let ch = punct.as_char();
                        self.word(&ch.to_string());
                        if punct.spacing() == Spacing::Joint {
                            self.glue = true;
                        }
                        if ch == ';' || ch == ',' {
                            self.newline();
                        }
                        after_hash = ch == '#';
                        continue;
                    }
                    other => self.word(&other.to_string()),
                }
                after_hash = false;
            }
        }
    }
    let mut printer = Printer {
        out: String::new(),
        indent: 0,
        at_line_start: true,
        glue: false,
    };
    printer.stream(tokens);
    printer.out
}

/// The variant names of the enum `name` in an expanded module.
fn enum_variants(expanded: &TokenStream, name: &str) -> Vec<String> {
    let module: syn::ItemMod = syn::parse2(expanded.clone()).expect("expansion is a module");
    let (_, items) = module.content.expect("inline module");
    items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == name => {
                Some(e.variants.iter().map(|v| v.ident.to_string()).collect())
            }
            _ => None,
        })
        .unwrap_or_default()
}

// ---- generated code --------------------------------------------------------

#[test]
fn generates_a_keyword_entry() {
    let src = module(
        r#"        /// CSS Compositing 1 §3.4.2
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },"#,
    );
    let (expanded, errors) = expand(&src);
    assert!(errors.is_empty(), "{}", render(&src, &errors));
    insta::assert_snapshot!("generated_keyword_entry", pretty(expanded));
}

#[test]
fn generates_parsed_and_hook_entries() {
    let src = r#"mod decl {
    pub enum PropertyValue {
        #[key(with = |value| value.key)]
        Deferred(DeferredValue),
    }
    pub enum PropertyKey {}
    properties! {
        /// CSS Images 3 §5.1
        "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain, residue: none },
        /// CSS Text 3 §7.2
        "word-spacing" => WordSpacing: Length {
            initial: Length::Px(0.0), inherited: yes, parse: parse_length,
            computed: ComputedLength, compute: absolutize_length, lift: lift_length,
            field: spacing, sample: Length::Em(2.0), residue: length_residue,
        },
    }
}
"#;
    let (expanded, errors) = expand(src);
    assert!(errors.is_empty(), "{}", render(src, &errors));
    insta::assert_snapshot!("generated_parsed_and_hook_entries", pretty(expanded));
}

/// The item named `name` in an expanded module, as tokens.
fn item_tokens(expanded: &TokenStream, name: &str) -> TokenStream {
    let module: syn::ItemMod = syn::parse2(expanded.clone()).expect("expansion is a module");
    let (_, items) = module.content.expect("inline module");
    items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == name => Some(quote::ToTokens::into_token_stream(e)),
            _ => None,
        })
        .expect("item present")
}

#[test]
fn generates_extra_derives_and_the_default_keyword() {
    let src = module(
        r#"        /// A keyword longhand whose initial keyword is not the first.
        "a-mode" => AMode {
            keywords: [NoAutospace, Auto, Normal],
            derive: [Hash, core::default::Default],
            initial: Normal,
            inherited: yes,
        },"#,
    );
    let (expanded, errors) = expand(&src);
    assert!(errors.is_empty(), "{}", render(&src, &errors));
    insta::assert_snapshot!(
        "generated_keyword_enum_derives",
        pretty(item_tokens(&expanded, "AMode"))
    );
}

#[test]
fn several_blocks_append_in_order_after_hand_written_variants() {
    let src = r#"mod decl {
    pub enum PropertyValue { Color(u32) }
    pub enum PropertyKey { Color }
    properties! {
        /// A.
        "b-prop" => BProp { keywords: [X, Y], initial: X, inherited: no },
    }
    fn between() {}
    properties! {
        /// B.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
    }
}
"#;
    let (expanded, errors) = expand(src);
    assert!(errors.is_empty(), "{}", render(src, &errors));
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        ["Color", "BProp", "AProp"]
    );
    assert_eq!(
        enum_variants(&expanded, "PropertyKey"),
        ["Color", "BProp", "AProp"]
    );
}

// ---- diagnostics -----------------------------------------------------------

/// One typo is one error, and the entry is still declared, so the code that
/// uses its variant, field and types keeps compiling.
#[test]
fn unknown_key() {
    let src = module(
        r#"        /// CSS Compositing 1 §3.4.2
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no, animatable: no },
        /// CSS Images 3 §5.1
        "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain, residue: none },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 1);
    insta::assert_snapshot!("unknown_key", render(&src, &errors));
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        ["Color", "CustomProperty", "Isolation", "ObjectFit"]
    );
    assert_eq!(enum_variants(&expanded, "Isolation"), ["Auto", "Isolate"]);
}

#[test]
fn duplicate_name() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// CSS Compositing 1 §3.4.2
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },
        /// CSS Images 3 §5.1
        "isolation" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain, residue: none },"#,
    ));
    assert_eq!(count, 1);
    insta::assert_snapshot!("duplicate_name", rendered);
}

#[test]
fn duplicate_variant_field_and_hand_written_variant() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// A.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
        /// Same variant.
        "b-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
        /// Same field.
        "c-prop" => CProp { keywords: [X, Y], initial: X, inherited: no, field: a_prop },
        /// Hand-written variant.
        "color" => Color { keywords: [X, Y], initial: X, inherited: yes },"#,
    ));
    assert_eq!(count, 3);
    insta::assert_snapshot!("duplicate_variant_field_and_hand_written_variant", rendered);
}

#[test]
fn missing_required_keys() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// CSS Color 4 §3.3
        "opacity" => Opacity: f32 { parse: parse_opacity_value, inherited: no, residue: none },
        /// CSS Images 3 §5.1
        "object-fit" => ObjectFit { initial: Fill, parse: parse_object_fit, residue: none },
        /// No parser.
        "x-prop" => XProp { initial: A, inherited: no, sample: B },
        "undocumented" => Undocumented { keywords: [A, B], initial: A, inherited: no },"#,
    ));
    assert_eq!(count, 6);
    insta::assert_snapshot!("missing_required_keys", rendered);
}

#[test]
fn malformed_values() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// A.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: true },
        /// B.
        "b-prop" => BProp: Length { initial: L, inherited: no, parse: |i| None, sample: L, residue: none },
        /// C.
        "c-prop" => CProp: Length { initial: L, inherited: no, parse: p, computed: 3, sample: L, residue: none },
        /// D.
        "d-prop" => DProp { keywords: Auto, initial: Auto, inherited: no },
        /// E.
        "e-prop" => EProp { keywords: [X, Y], initial: X, inherited: no, initial: Y },
        /// F.
        "f-prop" => FProp { keywords: [X, Y], initial: X inherited: no },"#,
    ));
    assert_eq!(count, 6);
    insta::assert_snapshot!("malformed_values", rendered);
}

#[test]
fn conflicting_keys() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// A.
        "a-prop" => AProp { keywords: [X, Y], parse: parse_a, initial: X, inherited: no },
        /// B.
        "b-prop" => BProp: u8 { keywords: [X, Y], initial: X, inherited: no },
        /// C.
        "c-prop" => CProp: f32 { initial: 1.0, inherited: no, parse: p,
                                 computed: Px, sample: 2.0, residue: none },
        /// D.
        "d-prop" => DProp: f32 { initial: 1.0, inherited: no, parse: p, lift: l, sample: 2.0, residue: none },"#,
    ));
    assert_eq!(count, 4);
    insta::assert_snapshot!("conflicting_keys", rendered);
}

/// The computed type, its hook and the lift back are three keys: the old
/// `computed: Type via hook` form is one error on `via` naming the two keys
/// with the written tokens, and each cross-key rule is one error on the key
/// that breaks it.
#[test]
fn computed_key_mistakes() {
    let src = module(
        r#"        /// A.
        "a-prop" => AProp: L { initial: L::Z, inherited: no, parse: p, computed: Px via to_px, lift: l, sample: L::O, residue: none },
        /// B.
        "b-prop" => BProp: L { initial: L::Z, inherited: no, parse: p, computed: Px, lift: l, sample: L::O, residue: none },
        /// C.
        "c-prop" => CProp: f32 { initial: 1.0, inherited: no, parse: p, compute: c, lift: l, sample: 2.0, residue: none },
        /// D.
        "d-prop" => DProp: L { initial: L::Z, inherited: no, parse: p, via: to_px, sample: L::O, residue: none },
        /// E.
        "e-prop" => EProp: L { initial: L::Z, inherited: no, parse: p, computed: Px, compute: to_px, lift: l, sample: L::O, residue: none },
        /// F: `via:` in place of `compute:` is the one error.
        "f-prop" => FProp: L { initial: L::Z, inherited: no, parse: p, computed: Px, via: to_px, sample: L::O, residue: none },
        /// G: a `compute:` after `via:` is not a duplicate.
        "g-prop" => GProp: L { initial: L::Z, inherited: no, parse: p, via: to_px, compute: to_px, computed: Px, sample: L::O, residue: none },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 6, "{}", render(&src, &errors));
    insta::assert_snapshot!("computed_key_mistakes", render(&src, &errors));
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        [
            "Color",
            "CustomProperty",
            "AProp",
            "BProp",
            "CProp",
            "DProp",
            "EProp",
            "FProp",
            "GProp"
        ]
    );
    // A computed type without its hook keeps the type and expands the hook
    // to an unreachable body rather than a mistyped identity.
    let text = expanded.to_string();
    assert!(text.contains("pub type Computed = Px ;"), "{text}");
}

#[test]
fn keyword_mistakes() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// A.
        "a-prop" => AProp { keywords: [], initial: X, inherited: no },
        /// B.
        "b-prop" => BProp { keywords: [X, X, Y = "x", Z = "Zed"], initial: X, inherited: no },
        /// C.
        "c-prop" => CProp { keywords: [X, Y], initial: Q, inherited: no, sample: Y },
        /// D.
        "d-prop" => DProp { keywords: [X], initial: X, inherited: no },
        /// E.
        "Bad_Name" => EProp { keywords: [X, #[cfg(test)] Y], initial: X, inherited: no },"#,
    ));
    assert_eq!(count, 8);
    insta::assert_snapshot!("keyword_mistakes", rendered);
}

#[test]
fn derive_mistakes() {
    let (rendered, count) = diagnostics(&module(
        r#"        /// A.
        "a-prop" => AProp { keywords: [X, Y], derive: [Clone, Hash, Hash], initial: X, inherited: no },
        /// B.
        "b-prop" => BProp: f32 { derive: [Default], initial: 1.0, inherited: no, parse: p, sample: 2.0, residue: none },
        /// C.
        "c-prop" => CProp { keywords: [X, Y], derive: [Default], initial: CProp::Q, inherited: no },
        /// D.
        "d-prop" => DProp { keywords: [X, Y], derive: Hash, initial: X, inherited: no },"#,
    ));
    assert_eq!(count, 5);
    insta::assert_snapshot!("derive_mistakes", rendered);
}

/// A hooked or parsed entry without `residue:` is one error on its head; a
/// malformed or repeated `residue:` is one error on the written tokens.
#[test]
fn residue_mistakes() {
    let src = module(
        r#"        /// A.
        "a-prop" => AProp: f32 { initial: 1.0, inherited: no, parse: p, compute: c, sample: 2.0 },
        /// B.
        "b-prop" => BProp: L { initial: L::Z, inherited: no, parse: p, sample: L::O },
        /// C.
        "c-prop" => CProp { keywords: [X, Y], initial: X, inherited: no, computed: T, compute: t },
        /// D.
        "d-prop" => DProp { keywords: [X, Y], initial: X, inherited: no, residue: None },
        /// E.
        "e-prop" => EProp: L { initial: L::Z, inherited: no, parse: p, sample: L::O, residue: |v| None },
        /// F.
        "f-prop" => FProp: L { initial: L::Z, inherited: no, parse: p, sample: L::O, residue: none, residue: r },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 6);
    insta::assert_snapshot!("residue_mistakes", render(&src, &errors));
    // Every entry is still declared, so its uses keep compiling.
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        [
            "Color",
            "CustomProperty",
            "AProp",
            "BProp",
            "CProp",
            "DProp",
            "EProp",
            "FProp"
        ]
    );
}

#[test]
fn generates_the_residue_check() {
    let src = module(
        r#"        /// A keyword entry: `none` by default.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
        /// A length entry with its residue function.
        "b-prop" => BProp: L { initial: L::Z, inherited: no, parse: p, computed: Px, compute: to_px, sample: L::O, residue: l_residue },
        /// A written `none`.
        "c-prop" => CProp: f32 { initial: 1.0, inherited: no, parse: p, compute: c, sample: 2.0, residue: none },
        /// A malformed `residue:`: the arm is unreachable.
        "d-prop" => DProp: L { initial: L::Z, inherited: no, parse: p, sample: L::O, residue: 3 },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 1, "{}", render(&src, &errors));
    let module: syn::ItemMod = syn::parse2(expanded).expect("expansion is a module");
    let (_, items) = module.content.expect("inline module");
    let function = items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Fn(f) if f.sig.ident == "longhand_specified_residue" => {
                Some(quote::ToTokens::into_token_stream(f))
            }
            _ => None,
        })
        .expect("longhand_specified_residue");
    insta::assert_snapshot!("generated_residue_check", pretty(function));
}

#[test]
fn broken_entry_heads_skip_to_the_next_entry() {
    let src = module(
        r#"        /// A.
        "a-prop" = AProp { keywords: [X, Y], initial: X, inherited: no },
        /// B.
        "b-prop" => BProp { keywords: [X, Y], initial: X, inherited: no }
        /// C.
        "c-prop" => { keywords: [X, Y], initial: X, inherited: no },
        #[cfg(test)]
        /// D.
        "d-prop" => DProp { keywords: [X, Y], initial: X, inherited: no },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 4);
    insta::assert_snapshot!("broken_entry_heads", render(&src, &errors));
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        ["Color", "CustomProperty", "BProp", "DProp"]
    );
}

#[test]
fn module_shape_mistakes() {
    let args = "Isolation, ObjectFit\n";
    let src = "mod decl { pub enum PropertyValue {} #[doc = \"x\"] properties! {} }\n";
    let (_, errors) = expand_with_args(args, src);
    assert_eq!(errors.len(), 3);
    // The first error is spanned on the attribute's arguments.
    insta::assert_snapshot!(
        "module_shape_mistakes",
        render(args, &errors[..1]) + &render(src, &errors[1..])
    );

    let src = "mod decl;\n";
    let (_, errors) = expand(src);
    assert_eq!(errors.len(), 1);
    let src2 = "struct Decl;\n";
    let (_, errors2) = expand(src2);
    assert_eq!(errors2.len(), 1);
    let src3 = "mod decl { pub enum PropertyValue {} pub enum PropertyKey {} }\n";
    let (_, errors3) = expand(src3);
    assert_eq!(errors3.len(), 1);
    insta::assert_snapshot!(
        "module_shape_mistakes_2",
        render(src, &errors) + &render(src2, &errors2) + &render(src3, &errors3)
    );
}

#[test]
fn key_attribute_mistakes() {
    let src = r#"mod decl {
    pub enum PropertyValue {
        #[key(|value| value.key)]
        Deferred(DeferredValue),
        #[key(A, B)]
        Two(u8),
        #[key(with = f)]
        Unit,
        #[key(Other)]
        #[key(Other)]
        Twice(u8),
    }
    pub enum PropertyKey { Other }
    properties! {
        /// A.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
    }
}
"#;
    let (rendered, count) = diagnostics(src);
    assert_eq!(count, 4);
    insta::assert_snapshot!("key_attribute_mistakes", rendered);
}

#[test]
fn rust_keywords_and_raw_identifiers_are_errors_not_panics() {
    let src = module(
        r#"        /// A.
        "a-prop" => type { keywords: [X, Y], initial: X, inherited: no },
        /// B.
        "b-prop" => r#type { keywords: [X, Y], initial: X, inherited: no },
        /// C.
        "c-prop" => CProp { keywords: [X, r#fn], initial: X, inherited: no },
        /// D.
        "d-prop" => DProp { keywords: [X, Y], initial: X, inherited: no, field: fn },
        /// E.
        "type" => Type { keywords: [X, Y], initial: X, inherited: no },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 5);
    insta::assert_snapshot!("rust_keywords", render(&src, &errors));
    assert_eq!(
        enum_variants(&expanded, "PropertyValue"),
        ["Color", "CustomProperty", "CProp", "DProp", "Type"]
    );
}

/// The names of the items directly inside an expanded module (functions,
/// structs, enums, modules, consts and macros).
fn item_names(expanded: &TokenStream) -> Vec<String> {
    let module: syn::ItemMod = syn::parse2(expanded.clone()).expect("expansion is a module");
    let (_, items) = module.content.expect("inline module");
    items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
            syn::Item::Struct(s) => Some(s.ident.to_string()),
            syn::Item::Enum(e) => Some(e.ident.to_string()),
            syn::Item::Mod(m) => Some(m.ident.to_string()),
            syn::Item::Const(c) => Some(c.ident.to_string()),
            syn::Item::Macro(m) => m.ident.as_ref().map(|i| i.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn inner_attributes_stay_inside_the_module() {
    let src = r#"/// Outer docs.
#[allow(dead_code)]
mod decl {
    //! Inner docs.
    #![allow(clippy::large_enum_variant)]
    pub enum PropertyValue { Color(u32) }
    pub enum PropertyKey { Color }
    properties! {
        /// A.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
    }
}
"#;
    let (expanded, errors) = expand(src);
    assert!(errors.is_empty(), "{}", render(src, &errors));
    let text = expanded.to_string();
    let open = text.find("mod decl {").expect("module header");
    let (before, inside) = text.split_at(open);
    assert!(before.contains("Outer docs") && before.contains("allow (dead_code)"));
    assert!(!before.contains("Inner docs") && !before.contains("large_enum_variant"));
    assert!(inside.starts_with(
        "mod decl { # ! [doc = \" Inner docs.\"] # ! [allow (clippy :: large_enum_variant)]"
    ));
    // The expansion must parse back as a module with its inner attributes.
    let module: syn::ItemMod = syn::parse2(expanded).expect("expansion is a module");
    assert_eq!(
        module
            .attrs
            .iter()
            .filter(|a| matches!(a.style, syn::AttrStyle::Inner(_)))
            .count(),
        2
    );
}

/// When the only entry fails, the module-level items are still generated,
/// so the rest of the crate compiles against them and the one mistake is
/// the one error. `longhand_value_pat!` is the exception: with no entry it
/// cannot be a pattern.
#[test]
fn module_items_survive_when_no_entry_does() {
    let src = module(
        r#"        /// A.
        "a-prop" = AProp { keywords: [X, Y], initial: X, inherited: no },"#,
    );
    let (expanded, errors) = expand(&src);
    assert_eq!(errors.len(), 1, "{}", render(&src, &errors));
    let names = item_names(&expanded);
    for expected in [
        "SpecifiedTable",
        "ComputedTable",
        "longhand_page_absolutize",
        "LONGHAND_NAMES",
        "longhand_samples",
        "longhand_sample",
        "longhand_key_for_name",
        "parse_longhand_value",
        "with_longhand_samples",
        "with_longhand_variants",
        "longhand_specified_residue",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} in {names:?}"
        );
    }
    assert!(!names.iter().any(|n| n == "longhand_value_pat"));
    assert!(
        expanded
            .to_string()
            .contains("pub fn key (& self) -> PropertyKey")
    );
}

#[test]
fn key_helpers_are_stripped_when_an_enum_is_missing() {
    let src = r#"mod decl {
    pub enum PropertyValue {
        #[key(Custom)]
        CustomProperty(String),
    }
}
"#;
    let (expanded, errors) = expand(src);
    assert_eq!(errors.len(), 1);
    assert!(!expanded.to_string().contains("key (Custom)"));
}

#[test]
fn nested_properties_blocks_are_reported_and_removed() {
    let src = r#"mod decl {
    pub enum PropertyValue { Color(u32) }
    pub enum PropertyKey { Color }
    properties! {
        /// A.
        "a-prop" => AProp { keywords: [X, Y], initial: X, inherited: no },
    }
    mod inner {
        properties! {
            /// B.
            "b-prop" => BProp { keywords: [X, Y], initial: X, inherited: no },
        }
    }
}
"#;
    let (expanded, errors) = expand(src);
    assert_eq!(errors.len(), 1);
    insta::assert_snapshot!("nested_properties", render(src, &errors));
    assert!(!expanded.to_string().contains("b-prop"));
}
