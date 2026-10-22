use super::*;
use crate::layout::test_support::cascaded;

/// The properties a `NonFiniteClamped` warning names while the bridge walks
/// one `<div>` styled `css`, in the order they are reported.
fn clamped_sites(css: &str) -> Vec<&'static str> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    doc.append_element(Some(body), "div", Some(css));
    let cascade = cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &cascade);
    std::mem::take(&mut session.context.layout_warnings)
        .into_iter()
        .filter_map(|warning| match warning {
            LayoutWarn::NonFiniteClamped { site, .. } => Some(site),
            _ => None,
        })
        .collect()
}

#[test]
fn an_out_of_range_length_is_reported_under_the_property_that_carries_it() {
    for (css, site) in [
        ("width:1e40px", "width"),
        ("width:1e40%", "width"),
        ("width:fit-content(1e40px)", "width"),
        ("width:fit-content(1e40%)", "width"),
        ("height:1e40px", "height"),
        ("position:relative;top:1e40px", "top"),
        ("position:relative;right:1e40%", "right"),
        ("position:relative;bottom:1e40px", "bottom"),
        ("position:relative;left:1e40px", "left"),
        ("margin-left:1e40px", "margin"),
        ("margin-top:1e40%", "margin"),
        ("padding-top:1e40px", "padding"),
        ("padding-left:1e40%", "padding"),
        ("min-width:1e40px", "min-width"),
        ("min-height:1e40px", "min-height"),
        ("max-width:1e40%", "max-width"),
        ("max-height:1e40px", "max-height"),
        ("border-top:1e40px solid", "border-width"),
        ("display:flex;flex-basis:1e40px", "flex-basis"),
        ("display:flex;flex-basis:1e40%", "flex-basis"),
        ("display:grid;column-gap:1e40px", "column-gap"),
        ("display:grid;row-gap:1e40%", "row-gap"),
    ] {
        assert_eq!(clamped_sites(css), [site], "{css}");
    }
}

#[test]
fn a_finite_length_is_not_reported() {
    for css in [
        "width:1e6px;height:50%;padding:1px;margin:-2px;min-width:0",
        "display:flex;flex-basis:30px;column-gap:4px;row-gap:5%",
        "display:grid;grid-template-columns:10px 1fr;grid-auto-rows:minmax(0,1fr)",
    ] {
        assert!(clamped_sites(css).is_empty(), "{css}");
    }
}

#[test]
fn every_grid_track_function_reports_its_out_of_range_lengths() {
    for (css, sites) in [
        (
            "display:grid;grid-template-columns:1e40px",
            vec!["grid-template-columns"; 2],
        ),
        (
            "display:grid;grid-template-rows:1e40%",
            vec!["grid-template-rows"; 2],
        ),
        (
            "display:grid;grid-template-columns:minmax(1e40px,2px)",
            vec!["grid-template-columns"],
        ),
        (
            "display:grid;grid-template-columns:minmax(2px,1e40%)",
            vec!["grid-template-columns"],
        ),
        (
            "display:grid;grid-template-rows:minmax(1px,1e30fr)",
            vec!["grid-template-rows"],
        ),
        (
            "display:grid;grid-template-columns:fit-content(1e40px)",
            vec!["grid-template-columns"],
        ),
        (
            "display:grid;grid-template-columns:fit-content(1e40%)",
            vec!["grid-template-columns"],
        ),
        (
            "display:grid;grid-template-columns:repeat(2,1e40px)",
            vec!["grid-template-columns"; 2],
        ),
        (
            "display:grid;grid-auto-columns:1e40px",
            vec!["grid-auto-columns"; 2],
        ),
        (
            "display:grid;grid-auto-rows:minmax(1e40%,3px)",
            vec!["grid-auto-rows"],
        ),
    ] {
        assert_eq!(clamped_sites(css), sites, "{css}");
    }
}

#[test]
fn a_vertical_box_reports_the_physical_minimum_height() {
    assert_eq!(
        clamped_sites("writing-mode:vertical-rl;min-inline-size:1e40px"),
        ["min-height"]
    );
    assert_eq!(
        clamped_sites("writing-mode:vertical-rl;min-block-size:1e40px"),
        ["min-width"]
    );
}
