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

fn wpt_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    for (path, html) in files {
        let path = directory.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, html).unwrap();
    }
    directory
}

fn small() -> ReftestConfig {
    ReftestConfig {
        width: 16,
        height: 16,
        ..ReftestConfig::default()
    }
}

const GREEN_REF: &str = r#"<!DOCTYPE html><body style="margin:0"><div style="width:8px;height:8px;background-color:green"></div>"#;
const RED_REF: &str = r#"<!DOCTYPE html><body style="margin:0"><div style="width:8px;height:8px;background-color:red"></div>"#;

fn audit(files: &[(&str, &str)], id: &str) -> Verdict {
    let tree = wpt_tree(files);
    audit_id_with_config(tree.path(), id, small())
}

#[test]
fn a_test_whose_ablated_declarations_matter_is_kept() {
    let test = r#"<!DOCTYPE html><link rel="match" href="ref.html"><body style="margin:0"><div style="width:8px;height:8px;background:green"></div>"#;
    let verdict = audit(
        &[("css/a/keep.html", test), ("css/a/ref.html", GREEN_REF)],
        "css/a/keep.html",
    );
    assert_eq!(verdict, Verdict::Kept { ablated: 1 });
    assert_eq!(verdict.label(), "KEEP");
    assert_eq!(verdict.detail(), "1 test-only declaration(s) ablated");
}

#[test]
fn a_test_sharing_every_declaration_is_kept_with_weaker_evidence() {
    let test = r#"<!DOCTYPE html><link rel="match" href="ref.html"><body style="margin:0"><p style="margin:0"><span style="display:block;width:8px;height:8px;background-color:green"></span>"#;
    let reference = r#"<!DOCTYPE html><body style="margin:0"><div style="display:block;width:8px;height:8px;background-color:green"></div>"#;
    let verdict = audit(
        &[("css/a/markup.html", test), ("css/a/ref.html", reference)],
        "css/a/markup.html",
    );
    assert!(matches!(verdict, Verdict::Kept { .. }), "{verdict:?}");
}

#[test]
fn a_blank_pair_is_vacuous() {
    let test =
        r#"<!DOCTYPE html><link rel="match" href="ref.html"><div style="color:green"></div>"#;
    let verdict = audit(
        &[
            ("css/a/blank.html", test),
            ("css/a/ref.html", "<!DOCTYPE html><p></p>"),
        ],
        "css/a/blank.html",
    );
    assert_eq!(verdict, Verdict::Blank);
    assert_eq!(verdict.label(), "VACUOUS-BLANK");
    assert_eq!(verdict.detail(), "reference matches an empty document");
}

#[test]
fn a_subject_property_without_visible_effect_is_vacuous() {
    let test = r#"<!DOCTYPE html><link rel="match" href="ref.html"><body style="margin:0"><div style="clear:both;width:8px;height:8px;background-color:green"></div>"#;
    let verdict = audit(
        &[
            ("css/a/clear-001.html", test),
            ("css/a/ref.html", GREEN_REF),
        ],
        "css/a/clear-001.html",
    );
    assert_eq!(
        verdict,
        Verdict::Unstyled {
            probe: "feature",
            ablated: 1
        }
    );
    assert_eq!(verdict.label(), "VACUOUS-UNSTYLED");
    assert_eq!(
        verdict.detail(),
        "still matches after the feature ablation of 1 declaration(s)"
    );
}

#[test]
fn mismatch_only_tests_are_vacuous_without_probing() {
    let test = r#"<!DOCTYPE html><link rel="mismatch" href="ref.html"><body style="margin:0"><div style="width:8px;height:8px;background:green"></div>"#;
    let verdict = audit(
        &[("css/a/mismatch.html", test), ("css/a/ref.html", RED_REF)],
        "css/a/mismatch.html",
    );
    assert_eq!(verdict, Verdict::MismatchOnly);
    assert_eq!(verdict.label(), "VACUOUS-MISMATCH-ONLY");
    assert_eq!(verdict.detail(), "only rel=mismatch references");
}

#[test]
fn failing_and_missing_tests_are_not_pass() {
    let test = r#"<!DOCTYPE html><link rel="match" href="ref.html"><body style="margin:0"><div style="width:8px;height:8px;background:green"></div>"#;
    let verdict = audit(
        &[("css/a/fail.html", test), ("css/a/ref.html", RED_REF)],
        "css/a/fail.html",
    );
    assert!(
        matches!(
            verdict,
            Verdict::NotPass {
                status: Status::Fail,
                ..
            }
        ),
        "{verdict:?}"
    );
    assert_eq!(verdict.label(), "NOT-PASS");
    assert!(verdict.detail().starts_with("FAIL: "));
    let missing = audit(&[], "css/a/missing.html");
    assert!(
        matches!(
            missing,
            Verdict::NotPass {
                status: Status::Error,
                ..
            }
        ),
        "{missing:?}"
    );
}

#[test]
fn a_self_reference_keeps_its_original_document() {
    // The reference is the same file under a query that the page ignores, so
    // probing the test must not rewrite the reference too.
    let test = r#"<!DOCTYPE html><link rel="match" href="self.html?ref"><body style="margin:0"><div style="width:8px;height:8px;background:green"></div>"#;
    let tree = wpt_tree(&[("css/a/self.html", test)]);
    let root = std::fs::canonicalize(tree.path()).unwrap();
    let pair = discover_pairs_for_file_with_wpt_root(&root.join("css/a/self.html"), Some(&root))
        .unwrap()
        .remove(0);
    assert_eq!(passes(&pair, small(), false, "", test), Some(true));
    assert_eq!(
        passes(&pair, small(), false, "", &blank_document(test)),
        Some(false)
    );
}

#[test]
fn scanner_handles_escapes_unquoted_and_valueless_attributes() {
    let html = r#"<input disabled style=color:green><p data-x = 'a' style = "content: '\'}'; /* x */ width: 1px; : bad; 9:9; --e:">"#;
    assert_eq!(
        keys(html),
        ["color:green", r"content:'\'}'", "width:1px", "9:9"]
    );
}

#[test]
fn scanner_stops_at_unterminated_constructs() {
    assert_eq!(keys("<style>a { color: red; /* open"), ["color:red"]);
    assert_eq!(keys("<div style='width: 1px"), Vec::<String>::new());
    assert_eq!(
        keys("<!-- never closed <div style='width: 1px'>"),
        Vec::<String>::new()
    );
    assert_eq!(keys("a < b <1 <style>b { top: 0 }"), ["top:0"]);
    assert_eq!(css_declarations("top: 1px; left: 2px", true, 10).len(), 2);
    assert_eq!(css_declarations("x { a: 1 } }", false, 0).len(), 1);
}

#[test]
fn every_passing_alternative_must_be_vacuous() {
    let test = r#"<!DOCTYPE html><link rel="match" href="red.html"><link rel="match" href="ref.html"><link rel="match" href="ref2.html"><body style="margin:0"><div style="clear:both;width:8px;height:8px;background-color:green"></div>"#;
    let verdict = audit(
        &[
            ("css/a/clear-002.html", test),
            ("css/a/red.html", RED_REF),
            ("css/a/ref.html", GREEN_REF),
            ("css/a/ref2.html", GREEN_REF),
        ],
        "css/a/clear-002.html",
    );
    assert!(matches!(verdict, Verdict::Unstyled { .. }), "{verdict:?}");

    let blank = r#"<!DOCTYPE html><link rel="match" href="ref.html"><link rel="match" href="ref2.html"><p></p>"#;
    let verdict = audit(
        &[
            ("css/a/blank-002.html", blank),
            ("css/a/ref.html", "<!DOCTYPE html>"),
            ("css/a/ref2.html", "<!DOCTYPE html>"),
        ],
        "css/a/blank-002.html",
    );
    assert_eq!(verdict, Verdict::Blank);
}

#[test]
fn a_missing_reference_is_a_probe_error() {
    let test = r#"<!DOCTYPE html><link rel="match" href="gone.html"><p></p>"#;
    let tree = wpt_tree(&[("css/a/gone-ref.html", test), ("css/a/gone.html", "")]);
    let root = std::fs::canonicalize(tree.path()).unwrap();
    let pairs =
        discover_pairs_for_file_with_wpt_root(&root.join("css/a/gone-ref.html"), Some(&root))
            .unwrap();
    std::fs::remove_file(root.join("css/a/gone.html")).unwrap();
    assert_eq!(passes(&pairs[0], small(), false, "", test), None);
}

#[test]
fn metadata_and_styles_inside_comments_or_scripts_are_ignored() {
    let html = r#"<!-- <meta name="fuzzy" content="0-255;0-480000"> -->
<script>document.write('<meta name="fuzzy" content="0-255;0-480000"><link rel="match" href="x.html">')</script>
<title><style>a { color: red }</style></title><meta name="assert" content="kept">"#;
    let blank = blank_document(html);
    assert!(!blank.contains("fuzzy"), "{blank}");
    assert!(!blank.contains("x.html"), "{blank}");
    assert!(blank.contains(r#"<meta name="assert" content="kept">"#));
    assert!(keys(html).is_empty());
}

#[test]
fn declaration_keys_keep_whitespace_in_strings_and_accept_escaped_names() {
    let html = r#"<div style="content: 'a  b'  'c';  --caf\e9:  x   y; --café: 1">"#;
    assert_eq!(
        keys(html),
        ["content:'a  b' 'c'", r"--caf\e9:x y", "--café:1"]
    );
    assert_eq!(collapse_whitespace(r#" "a\"  b"   c "#), r#""a\"  b" c"#);
}

#[test]
fn inconclusive_verdicts_name_the_failing_probe() {
    let verdict = Verdict::Inconclusive { probe: "blank" };
    assert_eq!(verdict.label(), "INCONCLUSIVE");
    assert_eq!(verdict.detail(), "the blank probe failed to render");
}

#[test]
fn a_probe_that_fails_to_render_is_inconclusive() {
    // The script throws once the subject declaration is ablated, so the
    // feature probe errors instead of rendering.
    let test = r#"<!DOCTYPE html><link rel="match" href="ref.html"><body style="margin:0"><div id="d" style="clear:both;width:8px;height:8px;background-color:green"></div>
<script>if (!document.getElementById("d").getAttribute("style").includes("clear")) throw new Error("ablated");</script>"#;
    let verdict = audit(
        &[
            ("css/a/clear-003.html", test),
            ("css/a/ref.html", GREEN_REF),
        ],
        "css/a/clear-003.html",
    );
    assert_eq!(verdict, Verdict::Inconclusive { probe: "feature" });
}
