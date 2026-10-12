use super::*;

fn keys(html: &str) -> Vec<String> {
    author_declarations(html)
        .into_iter()
        .map(|decl| decl.key)
        .collect()
}

#[test]
fn collects_style_element_and_attribute_declarations() {
    let html = r#"<!DOCTYPE html>
<style>
  div { Color: Green;  width : 10px }
  @media print { p { margin: 0 } }
  a:hover { text-decoration: none; }
</style>
<div style="box-shadow: 0 0 2px red; float:left">x</div>"#;
    assert_eq!(
        keys(html),
        [
            "color:Green",
            "width:10px",
            "margin:0",
            "text-decoration:none",
            "box-shadow:0 0 2px red",
            "float:left",
        ]
    );
}

#[test]
fn skips_comments_scripts_and_strings() {
    let html = r#"<!-- <div style="color: red"> -->
<script>var s = '<div style="width: 1px">';</script>
<style>/* a { color: red } */ b { content: "x;y{z}"; background: url(a;b.png) }</style>"#;
    assert_eq!(
        keys(html),
        [r#"content:"x;y{z}""#, "background:url(a;b.png)"]
    );
}

#[test]
fn ignores_selectors_and_top_level_at_rules() {
    let html = "<style>@import url(x.css); a:hover, b:focus { color: blue }</style>";
    assert_eq!(keys(html), ["color:blue"]);
}

#[test]
fn removes_only_matching_declarations() {
    let html = r#"<style>div { color: green; box-shadow: 0 0 2px red; }</style><p style="float: left; width: 5px">"#;
    let (out, removed) = remove_declarations(html, |key| {
        key.starts_with("box-shadow") || key.starts_with("float")
    });
    assert_eq!(removed, 2);
    assert_eq!(
        out,
        r#"<style>div { color: green; }</style><p style=" width: 5px">"#
    );
}

#[test]
fn removes_last_declaration_without_semicolon() {
    let html = "<style>div { color: green; width: 1px }</style>";
    let (out, removed) = remove_declarations(html, |key| key == "width:1px");
    assert_eq!(removed, 1);
    assert_eq!(out, "<style>div { color: green;}</style>");
}

#[test]
fn blank_document_keeps_metadata_only() {
    let html = r#"<!DOCTYPE html><title>t</title>
<link rel="match" href="ref.html"><link rel="stylesheet" href="a.css">
<meta name="fuzzy" content="0-1;0-10"><style>div{color:red}</style><div>x</div>"#;
    let blank = blank_document(html);
    assert!(blank.contains(r#"<meta name="fuzzy" content="0-1;0-10">"#));
    assert!(blank.contains(r#"<link rel="match" href="ref.html">"#));
    assert!(!blank.contains("stylesheet"));
    assert!(!blank.contains("<div"));
    assert!(!blank.contains("color"));
}

#[test]
fn feature_properties_come_from_file_name_and_help_links() {
    let html = r#"<link rel="help" href="https://drafts.csswg.org/css-backgrounds/#the-border-radius">
<style>div { box-shadow: 1px 1px red; border-radius: 3px; color: green; background-color: lime; float: left }</style>"#;
    let features = feature_properties(
        html,
        "css/css-backgrounds/box-shadow-background-color-001.html",
    );
    assert_eq!(
        features.into_iter().collect::<Vec<_>>(),
        ["background-color", "border-radius", "box-shadow"]
    );
}

#[test]
fn blank_document_keeps_only_reference_links_by_rel_token() {
    let html = r#"<link rel="stylesheet" href="matching.css"><link rel=mismatch href="b.html">
<link rel="help" href="https://example.test/#match">"#;
    let blank = blank_document(html);
    assert!(!blank.contains("matching.css"));
    assert!(!blank.contains("help"));
    assert!(blank.contains("<link rel=mismatch href=\"b.html\">"));
}

#[test]
fn declaration_keys_keep_case_sensitive_parts() {
    let html = r#"<div style="--Foo: Bar; COLOR: Green; content: 'A'">"#;
    assert_eq!(keys(html), ["--Foo:Bar", "color:Green", "content:'A'"]);
}
