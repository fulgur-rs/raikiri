//! Intrinsic inline sizes in vertical block formatting contexts.
//!
//! Border-box sizes recorded in Chromium with an 800x600 viewport, Ahem at
//! 10px, and zero body margins. An intrinsic used height remains indefinite
//! for percentage heights of replaced descendants.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

fn check(body: &str, expected: &[(&str, (f32, f32))]) {
    let html = format!(
        "<!doctype html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html,body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    for &(id, expected) in expected {
        let rect = page
            .fragments()
            .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
            .unwrap_or_else(|| panic!("no fragment for #{id}: {body}"))
            .rect();
        let actual = (rect.width, rect.height);
        assert!(
            (actual.0 - expected.0).abs() <= 0.02 && (actual.1 - expected.1).abs() <= 0.02,
            "#{id}: expected {expected:?}, got {actual:?}: {body}"
        );
    }
}

#[test]
fn vertical_lr_height_min_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:min-content;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_min_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:min-content;float:left;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_min_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:min-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_min_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;height:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_min_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;height:min-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_min_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;height:100px;"><div id="target" style="writing-mode:vertical-lr;height:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:max-content;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:max-content;float:left;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:max-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;height:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;height:max-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_max_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;height:100px;"><div id="target" style="writing-mode:vertical-lr;height:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_height_canvas() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:min-content"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (10.0, 0.0)), ("replaced", (0.0, 0.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:min-content;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:min-content;float:left;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:min-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;inline-size:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;inline-size:min-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_min_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;height:100px;"><div id="target" style="writing-mode:vertical-lr;inline-size:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:max-content;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:max-content;float:left;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:max-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;inline-size:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-lr;"><div id="target" style="writing-mode:vertical-lr;inline-size:max-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_max_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-lr;height:100px;"><div id="target" style="writing-mode:vertical-lr;inline-size:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_canvas() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:min-content"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (10.0, 0.0)), ("replaced", (0.0, 0.0))],
    );
}

#[test]
fn vertical_lr_canvas_definite() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:100px"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (100.0, 100.0)), ("replaced", (100.0, 50.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:min-content;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:min-content;float:left;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:min-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;height:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;height:min-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_min_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;height:100px;"><div id="target" style="writing-mode:vertical-rl;height:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:max-content;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:max-content;float:left;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:max-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;height:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;height:max-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_max_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;height:100px;"><div id="target" style="writing-mode:vertical-rl;height:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_height_canvas() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:min-content"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (10.0, 0.0)), ("replaced", (0.0, 0.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:min-content;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:min-content;float:left;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:min-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;inline-size:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;inline-size:min-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_min_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;height:100px;"><div id="target" style="writing-mode:vertical-rl;inline-size:min-content;">aa bbbb</div></div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:max-content;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_float() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:max-content;float:left;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_inline_block() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:max-content;display:inline-block;">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_auto_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;inline-size:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_auto_parent_float() {
    check(
        r#"<div style="writing-mode:vertical-rl;"><div id="target" style="writing-mode:vertical-rl;inline-size:max-content;float:left;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_max_content_definite_parent() {
    check(
        r#"<div style="writing-mode:vertical-rl;height:100px;"><div id="target" style="writing-mode:vertical-rl;inline-size:max-content;">aa bbbb</div></div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_canvas() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:min-content"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (10.0, 0.0)), ("replaced", (0.0, 0.0))],
    );
}

#[test]
fn vertical_rl_canvas_definite() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:100px"><canvas id="replaced" style="height:50%"></canvas></div>"#,
        &[("target", (100.0, 100.0)), ("replaced", (100.0, 50.0))],
    );
}

#[test]
fn vertical_lr_precedence_0() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:min-content;inline-size:60px">aa bbbb</div>"#,
        &[("target", (20.0, 60.0))],
    );
}

#[test]
fn vertical_lr_precedence_1() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:60px;height:min-content">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_precedence_2() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:60px;inline-size:min-content">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_lr_precedence_3() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:min-content;height:60px">aa bbbb</div>"#,
        &[("target", (20.0, 60.0))],
    );
}

#[test]
fn vertical_lr_height_fit_content() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;height:fit-content">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_lr_inline_size_fit_content() {
    check(
        r#"<div id="target" style="writing-mode:vertical-lr;inline-size:fit-content">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_precedence_0() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:min-content;inline-size:60px">aa bbbb</div>"#,
        &[("target", (20.0, 60.0))],
    );
}

#[test]
fn vertical_rl_precedence_1() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:60px;height:min-content">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_precedence_2() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:60px;inline-size:min-content">aa bbbb</div>"#,
        &[("target", (20.0, 40.0))],
    );
}

#[test]
fn vertical_rl_precedence_3() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:min-content;height:60px">aa bbbb</div>"#,
        &[("target", (20.0, 60.0))],
    );
}

#[test]
fn vertical_rl_height_fit_content() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;height:fit-content">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn vertical_rl_inline_size_fit_content() {
    check(
        r#"<div id="target" style="writing-mode:vertical-rl;inline-size:fit-content">aa bbbb</div>"#,
        &[("target", (10.0, 70.0))],
    );
}

#[test]
fn horizontal_min_content() {
    check(
        r#"<div id="target" style="width:min-content">aa bbbb</div>"#,
        &[("target", (40.0, 20.0))],
    );
}
