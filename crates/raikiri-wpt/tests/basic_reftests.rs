//! Basic CSS reftest 10 cases (the current reference set).
//!
//! Each case is a WPT-style reftest pair (test.html with `<link rel=match|mismatch href=ref.html>`).
//! We create temp files, discover pairs via `discover_pairs_for_file`, and assert `run_pair` passes.
//! The 10 cases cover box model, background-color, margin/padding, flex, font-size, color, display.

use raikiri_wpt::reftest::{
    ReftestConfig, ReftestKind, discover_pairs_for_file, parse_reftest_links, run_pair,
};
use raikiri_wpt::runner::Tolerance;

fn config() -> ReftestConfig {
    let mut c = ReftestConfig::default();
    c.width = 400;
    c.height = 300;
    c.tolerance = Tolerance::EXACT;
    c
}

fn run_match_pair(test_html: &str, ref_html: &str) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let test_path = dir.path().join("test.html");
    let ref_path = dir.path().join("ref.html");
    // Ensure link is present in test_html; if caller already embedded, use as-is, else prepend.
    let test_with_link = if test_html.contains("rel=") {
        test_html.to_owned()
    } else {
        format!(
            r#"<html><head><link rel="match" href="ref.html"></head><body>{}</body></html>"#,
            test_html
        )
    };
    // For our helper, we need test file to reference ref.html; write both.
    // If test_html is full document, ensure it contains the link; otherwise wrap.
    let (final_test, final_ref) = if test_html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<html")
    {
        // test_html is full doc; ensure link present
        let has_link = test_html.to_ascii_lowercase().contains("rel=\"match\"")
            || test_html.to_ascii_lowercase().contains("rel='match'")
            || test_html.to_ascii_lowercase().contains("rel=match");
        let t = if has_link {
            test_html.to_owned()
        } else {
            // inject link into head if exists, else prepend
            if let Some(pos) = test_html.to_ascii_lowercase().find("<head>") {
                let insert = pos + "<head>".len();
                let mut s = test_html.to_owned();
                s.insert_str(insert, r#"<link rel="match" href="ref.html">"#);
                s
            } else {
                format!(
                    r#"<html><head><link rel="match" href="ref.html"></head><body>{}</body></html>"#,
                    test_html
                )
            }
        };
        let r = if ref_html
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("<html")
        {
            ref_html.to_owned()
        } else {
            format!("<html><body>{}</body></html>", ref_html)
        };
        (t, r)
    } else {
        (
            test_with_link,
            format!("<html><body>{}</body></html>", ref_html),
        )
    };
    std::fs::write(&test_path, &final_test).unwrap();
    std::fs::write(&ref_path, &final_ref).unwrap();
    let pairs = discover_pairs_for_file(&test_path).expect("discover");
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].kind, ReftestKind::Match);
    let res = run_pair(&pairs[0], config()).unwrap();
    matches!(res.outcome, raikiri_wpt::runner::TestOutcome::Pass)
}

fn run_mismatch_pair(test_html: &str, ref_html: &str) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let test_path = dir.path().join("test.html");
    let ref_path = dir.path().join("ref.html");
    let test_doc = if test_html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<html")
    {
        test_html.to_owned()
    } else {
        format!("<html><body>{}</body></html>", test_html)
    };
    let test_with_link = if test_doc.to_ascii_lowercase().contains("rel=") {
        test_doc
    } else {
        // inject mismatch link
        if let Some(pos) = test_doc.to_ascii_lowercase().find("<head>") {
            let insert = pos + "<head>".len();
            let mut s = test_doc.clone();
            s.insert_str(insert, r#"<link rel="mismatch" href="ref.html">"#);
            s
        } else {
            test_doc.replace(
                "<html>",
                "<html><head><link rel=\"mismatch\" href=\"ref.html\"></head>",
            )
        }
    };
    // Ensure mismatch link specifically
    let final_test = test_with_link
        .replace("rel=\"match\"", "rel=\"mismatch\"")
        .replace("rel='match'", "rel='mismatch'")
        .replace("rel=match", "rel=mismatch");
    let final_ref = if ref_html
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("<html")
    {
        ref_html.to_owned()
    } else {
        format!("<html><body>{}</body></html>", ref_html)
    };
    std::fs::write(&test_path, &final_test).unwrap();
    std::fs::write(&ref_path, &final_ref).unwrap();
    let pairs = discover_pairs_for_file(&test_path).expect("discover mismatch");
    assert_eq!(pairs[0].kind, ReftestKind::Mismatch);
    let res = run_pair(&pairs[0], config()).unwrap();
    matches!(res.outcome, raikiri_wpt::runner::TestOutcome::Pass)
}

// ── 10 cases ───────────────────────────────────────────────────────────────

#[test]
fn reftest_01_background_hex_vs_name() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="background-color:#ff0000;width:80px;height:80px"></div></body></html>"#;
    let reference = r#"<html><body><div style="background-color:red;width:80px;height:80px"></div></body></html>"#;
    assert!(
        run_match_pair(test, reference),
        "background hex vs name should match"
    );
}

#[test]
fn reftest_02_margin_shorthand_vs_longhand() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="margin:10px;background-color:blue;width:50px;height:50px">hi</div></body></html>"#;
    let reference = r#"<html><body><div style="margin-top:10px;margin-right:10px;margin-bottom:10px;margin-left:10px;background-color:blue;width:50px;height:50px">hi</div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_03_padding_shorthand_vs_longhand() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="padding:12px;background-color:green;width:40px;height:40px">x</div></body></html>"#;
    let reference = r#"<html><body><div style="padding-top:12px;padding-right:12px;padding-bottom:12px;padding-left:12px;background-color:green;width:40px;height:40px">x</div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_04_flex_row_basic() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="display:flex;width:100px"><div style="width:50px;height:30px;background-color:red"></div><div style="width:50px;height:30px;background-color:blue"></div></div></body></html>"#;
    let reference = r#"<html><body><div style="display:flex;width:100px"><div style="width:50px;height:30px;background-color:red"></div><div style="width:50px;height:30px;background-color:blue"></div></div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_05_block_box_size() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="width:120px;height:60px;background-color:orange"></div></body></html>"#;
    let reference = r#"<html><body><div style="width:120px;height:60px;background-color:orange"></div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_06_font_size() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><p style="font-size:24px;color:black">Hello</p></body></html>"#;
    let reference = r#"<html><body><p style="font-size:24px;color:black">Hello</p></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_07_color_inherit() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="color:red"><p>inherit red</p></div></body></html>"#;
    let reference = r#"<html><body><div style="color:red"><p>inherit red</p></div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_08_display_none_equivalence() {
    // display:none element should not affect layout; compare against omitted element (margin 0 to avoid body margin diff)
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body style="margin:0"><div style="display:none">hidden</div><p style="margin:0">visible</p></body></html>"#;
    let reference = r#"<html><body style="margin:0"><p style="margin:0">visible</p></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_09_nested_backgrounds() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="width:100px;height:100px;background-color:red"><div style="width:50px;height:50px;background-color:blue"></div></div></body></html>"#;
    let reference = r#"<html><body><div style="width:100px;height:100px;background-color:red"><div style="width:50px;height:50px;background-color:blue"></div></div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn reftest_10_mismatch_background_colors() {
    // Mismatch: red vs blue must differ, so Mismatch should Pass
    let test = r#"<html><head><link rel="mismatch" href="ref.html"></head><body><div style="width:60px;height:60px;background-color:red"></div></body></html>"#;
    let reference = r#"<html><body><div style="width:60px;height:60px;background-color:blue"></div></body></html>"#;
    assert!(
        run_mismatch_pair(test, reference),
        "mismatch red vs blue should pass"
    );
}

#[test]
fn reftest_11_text_align_center_same() {
    let test = r#"<html><head><link rel="match" href="ref.html"></head><body><div style="width:300px;text-align:center">hello world</div></body></html>"#;
    let reference =
        r#"<html><body><div style="width:300px;text-align:center">hello world</div></body></html>"#;
    assert!(run_match_pair(test, reference));
}

#[test]
fn parse_reftest_links_api_smoke() {
    // Ensure link parser still works for the 10 fixtures
    let html = r#"<link rel="match" href="ref.html">"#;
    let links = parse_reftest_links(html);
    assert_eq!(links.len(), 1);
}
