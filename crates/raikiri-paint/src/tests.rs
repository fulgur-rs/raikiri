use super::*;
use anyrender::Scene;
use anyrender::recording::RenderCommand;
use parley::FontContext;
use peniko::Color;
use raikiri_dom::{Document, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::{Dimension, Display, Size, Style};

/// Return a Document + CascadeResult for hello-world
/// (`<html><head></head><body><p style="color:red">Hi</p></body></html>`)
/// after `layout_single_page`; shared setup for all tests in this module.
fn hello_world_paint_setup() -> (Document, raikiri_style::CascadeResult) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
    let _t = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    (doc, cr)
}

#[test]
fn paint_single_page_empty_list_item_emits_marker() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _item = doc.append_element(
        Some(body),
        "li",
        Style::default(),
        Some("display: list-item; list-style-type: disc"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

fn decorated_text_scene(style: &str) -> Scene {
    decorated_text_scene_with_text(style, "Decoration")
}

fn decorated_text_scene_with_text(style: &str, text: &str) -> Scene {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some(style));
    let _text = doc.append_text(p, text);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    scene
}

#[test]
fn named_page_paint_wrapper_uses_shared_context_entry_point() {
    let (document, cascade) = hello_world_paint_setup();
    let mut scene = Scene::new();
    paint_single_page_with_origin_and_page_context_named(
        &mut scene,
        &document,
        &cascade,
        PageBox::A4,
        0.0,
        0,
        1,
        false,
        None,
        None,
    );
    assert!(!scene.commands.is_empty());
}

#[test]
fn paint_single_page_box_shadow_emits_basic_outer_shadow_command() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _box = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 100px; height: 100px; box-shadow: rgba(0,255,0,1) 10px 10px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let shadow = scene.commands.iter().find_map(|command| match command {
        RenderCommand::BoxShadow(shadow) => Some(shadow),
        _ => None,
    });
    let shadow = shadow.expect("basic outer box-shadow should reach the scene");
    assert_eq!(shadow.rect.x0, 10.0);
    assert_eq!(shadow.rect.y0, 10.0);
    assert_eq!(shadow.rect.x1, 110.0);
    assert_eq!(shadow.rect.y1, 110.0);
    assert_eq!(shadow.radius, 0.0);
    assert_eq!(shadow.std_dev, 0.0);
    assert_eq!(shadow.brush, Color::from_rgba8(0, 255, 0, 255));
}

#[test]
fn paint_single_page_box_shadow_resolves_current_color_and_skips_inset_or_transparent() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _box = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(
                "width: 100px; height: 100px; color: #0a141e; box-shadow: currentcolor 4px 3px, transparent 8px 3px, inset black 12px 3px",
            ),
        );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let shadows: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::BoxShadow(shadow) => Some(shadow),
            _ => None,
        })
        .collect();
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].rect, kurbo::Rect::new(4.0, 3.0, 104.0, 103.0));
    assert_eq!(shadows[0].brush, Color::from_rgba8(10, 20, 30, 255));
}

#[test]
fn paint_single_page_outline_emits_basic_solid_outline() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _box = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 60px; height: 60px; color: green; outline: 20px solid currentcolor"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let green_fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => match fill.brush {
                anyrender::Paint::Solid(color) if color == Color::from_rgba8(0, 128, 0, 255) => {
                    Some(fill)
                }
                _ => None,
            },
            _ => None, // cov:ignore: this collector intentionally ignores non-Fill scene commands
        })
        .collect();
    assert_eq!(green_fills.len(), 4);
}

#[test]
fn paint_single_page_opacity_wraps_element_subtree_in_one_layer() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _box = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 60px; height: 60px; background: red; opacity: 0.5"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let layers: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::PushLayer(layer) => Some(layer),
            _ => None,
        })
        .collect();
    assert_eq!(layers.len(), 1);
    assert!((layers[0].alpha - 0.5).abs() < f32::EPSILON);
    assert!(
        scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::PopLayer))
            .count()
            >= 1
    );
}

#[test]
fn paint_single_page_blurred_text_shadow_uses_a_filtered_layer() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("color: black; text-shadow: 3px 4px 5px red"),
    );
    doc.append_text(p, "blur");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let filtered_layers = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::PushLayer(layer) if layer.filter.is_some() => Some(layer),
            _ => None,
        })
        .count();
    assert_eq!(
        filtered_layers, 1,
        "blurred text-shadow should paint its glyph mask through one filtered layer"
    );
}

#[test]
fn paint_single_page_compiles_and_returns_unit() {
    let doc = Document::new();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    // canvas white fill is now emitted even for empty Document
    assert_eq!(
        scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::Fill(_)))
            .count(),
        1,
        "empty Document should emit 1 canvas Fill (white)"
    );
}

#[test]
fn paint_single_page_canvas_background_site_is_noop_at_m1_4() {
    // canvas fill site is now active (paints white page background).
    // This test previously pinned the no-op state; now it pins that 1 canvas Fill is emitted.
    let (doc, cr) = hello_world_paint_setup();
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let fill_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::Fill(_)))
        .collect();
    assert_eq!(
        fill_commands.len(),
        1,
        "canvas site should emit 1 Fill (white page background), got {:?}",
        fill_commands.len()
    );
}

#[test]
fn paint_single_page_uses_page_background_color_before_document_canvas() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _text = doc.append_text(body, "Hi");
    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet(
        "@page { color: green; background-color: yellow; background-image: none }",
        raikiri_style::Origin::Author,
    );
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    // Force the matcher below through its non-fill arm as well as the
    // page-background fill arm.
    scene.commands.push(RenderCommand::PopLayer);

    let fill = scene
        .commands
        .iter()
        .rev()
        .find_map(|command| match command {
            RenderCommand::Fill(command) => Some(&command.brush),
            _ => None,
        })
        .expect("canvas fill command");
    assert_eq!(
        fill,
        &anyrender::Paint::Solid(peniko::Color::from_rgba8(255, 255, 0, 255))
    );
}

#[test]
fn paint_body_origin_fallbacks_cover_auto_and_computed_margins() {
    fn paint_body(style: Option<&str>, force_auto_computed_margin: bool) -> Scene {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), style);
        doc.append_text(body, "direct body text");
        let rules = build_rule_tree(&doc);
        let mut cascade = cascade(&doc, &rules).expect("cascade Ok");
        if force_auto_computed_margin {
            cascade.computed[body].margin.left =
                raikiri_style::resolve::ComputedLengthPercentageOrAuto::Auto;
        }
        layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
        scene
    }

    // An authored `auto` margin takes the non-UA branch but has no finite
    // used value, exercising its zero fallback.
    let auto_scene = paint_body(Some("margin-left:auto;background-color:red"), false);
    assert!(
        auto_scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );

    // With no authored margin, direct text plus a body canvas background
    // takes the computed-value fallback branch.
    let computed_scene = paint_body(Some("background-color:red"), true);
    assert!(
        computed_scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
    let computed_px_scene = paint_body(Some("background-color:red"), false);
    assert!(
        computed_px_scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_img_filename_color_uses_padding_aware_background_path() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let img = doc.append_element(
        Some(body),
        "img",
        Style::default(),
        Some("width:20px;height:10px;padding:2px"),
    );
    doc.set_element_attributes(img, vec![("src".into(), "red.png".into())]);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
    assert!(scene.commands.iter().any(|command| {
        matches!(
            command,
            RenderCommand::Fill(fill)
                if fill.brush == anyrender::Paint::Solid(peniko::Color::from_rgba8(255, 0, 0, 255))
        )
    }));
}

#[test]
fn paint_single_page_without_body_returns_early() {
    // Fragment (Document → direct `<p>` child, no `<body>`): `paint_document`
    // returns early, but still emits the white canvas background.
    let mut doc = Document::new();
    let p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
    let _t = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    // Do not call `layout_single_page` here (it returns Err); test canvas painting alone.
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    assert_eq!(
        scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::Fill(_)))
            .count(),
        1,
        "fragment (no body) should emit 1 canvas Fill (white) even without body"
    );
    assert_eq!(
        scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .count(),
        0,
        "fragment should emit no GlyphRun"
    );
}

#[test]
fn paint_single_page_skips_zero_size_subtree() {
    // Pin that a zero-size element modeling display:none (Display::None,
    // explicit size) is not walked. This checks `paint_element`'s early return
    // (the layout-side `empty_display_none_leaf...` test covers that side).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // Put the hidden element directly under body (Display::None, size 0).
    let hidden_style = Style {
        display: Display::None,
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::length(50.0),
        },
        ..Default::default()
    };
    let hidden = doc.append_element(Some(body), "hidden", hidden_style, Some("display:none"));
    let _child_of_hidden = doc.append_text(hidden, "should not be painted");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    // The hidden element should have zero layout size (taffy LayoutOutput::HIDDEN).
    let hidden_layout = doc.get_node(hidden).unwrap().unrounded_layout;
    assert_eq!(
        hidden_layout.size.width, 0.0,
        "display:none should have zero size after taffy layout"
    );
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    // Its text must emit no glyph runs, whether or not text painting is implemented.
    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .collect();
    assert!(
        glyph_commands.is_empty(),
        "text inside display:none subtree should not be painted, got {}",
        glyph_commands.len()
    );
}

#[test]
fn paint_single_page_clips_overflow_hidden_descendants() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width:70px;height:70px;overflow:hidden"),
    );
    let _child = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("width:200px;height:20px;background:blue"),
    );
    let rules = raikiri_style::build_rule_tree(&doc);
    let cr = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let clips = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::PushClipLayer(_)))
        .count();
    let pops = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::PopLayer))
        .count();
    assert_eq!(clips, 1, "overflow:hidden should push one descendant clip");
    assert_eq!(pops, clips, "every overflow clip must be popped");
}

#[test]
fn paint_single_page_paints_zero_size_display_block_subtree() {
    // The display:none test alone would pass even with the old size == 0
    // skip rule, so it cannot prevent regressions in the `is_display_none`
    // fix. This tests the converse: put a text child inside a display:block
    // container of size 0 and assert that a GlyphRun **is emitted**. Under
    // the old size-based skip, the container and its child would be skipped,
    // losing the GlyphRun. Under the current `is_display_none` check, the
    // Display::Block container is walked and the child's GlyphRun is emitted.
    // This distinguishes zero-size blocks from hidden elements.
    //
    // `apply_computed_to_style` now dispatches `bridge_size` for width.
    // The hand-built `taffy::Style { size.width = length(0) }` in this
    // fixture would be overwritten by the initial `cv.width` = Auto
    // (because `<container>` has no author width). An auto-width
    // Display::Block stretches to the containing width (A4), so the
    // `size.width == 0.0` sanity check would fail. Instead, specify
    // `"width: 0px; height: 0px"` inline so the bridge translates
    // cv.{width,height} = ComputedLengthPercentageOrAuto::Px(0.0).
    // Initially, height was not bridged, so the hand-built
    // `zero_block_style.size.height = length(0.0)` still preserved the
    // zero-height sanity check.
    //
    // Later, `bridge_size` began writing a struct literal for
    // `style.size = Size { width, height }`, bridging height as well.
    // The hand-built `size.height = length(0.0)` is now redundant because
    // the inline `height: 0px` overwrites it with the same value.
    // Behavior is unchanged.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let zero_block_style = Style {
        display: Display::Block,
        size: Size {
            width: Dimension::length(0.0),
            height: Dimension::length(0.0),
        },
        ..Default::default()
    };
    let container = doc.append_element(
        Some(body),
        "container",
        zero_block_style,
        // The bridge overwrites both width and height, so set them inline
        // (height is bridged too).
        Some("width: 0px; height: 0px"),
    );
    let _text = doc.append_text(container, "visible");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // Sanity check: honor the explicit zero width and height.
    let container_layout = doc.get_node(container).unwrap().unrounded_layout;
    assert_eq!(
        container_layout.size.width, 0.0,
        "zero-size Display::Block container should keep size.width = 0"
    );
    assert_eq!(
        container_layout.size.height, 0.0,
        "zero-size Display::Block container should keep size.height = 0"
    );

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .collect();
    assert_eq!(
        glyph_commands.len(),
        1,
        "text inside zero-size Display::Block container must still be painted (overflow: visible \
             default), got {}",
        glyph_commands.len()
    );
}

#[test]
fn paint_single_page_hello_world_emits_one_glyph_run() {
    // Pin that hello-world "Hi" emits one GlyphRun.
    let (doc, cr) = hello_world_paint_setup();
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .collect();
    assert_eq!(
        glyph_commands.len(),
        1,
        "hello world 'Hi' should emit exactly 1 GlyphRun, got {}",
        glyph_commands.len()
    );
}

#[test]
fn paint_single_page_simple_pre_tabs_normalizes_glyph_positions() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block; white-space:pre; tab-size:4; font-family:monospace; font-size:16px"),
    );
    let text = doc.append_text(block, "A\tB");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    assert!(doc.get_node(text).unwrap().snap_glyph_x_to_1_64());

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_single_page_line_break_anywhere_paints_text() {
    let scene = decorated_text_scene_with_text(
        "width:16px; line-height:1; font-family:monospace; line-break:anywhere",
        "aa-a.a)a,a）a",
    );
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_))),
        "line-break:anywhere should still emit a glyph run", // cov:ignore: assertion message runs only on failure
    );
}

#[test]
fn paint_single_page_uses_inherited_color_as_brush() {
    // `<p style="color:red">` passes red through the cascade to its text brush
    // (255, 0, 0, 255); guard the cascade-to-shaping path against regressions.
    use anyrender::types::Paint;
    use peniko::Color;

    let (doc, cr) = hello_world_paint_setup();
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let RenderCommand::GlyphRun(glyph_cmd) = scene
        .commands
        .iter()
        .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .expect("must have 1 GlyphRun")
    else {
        unreachable!()
    };
    match &glyph_cmd.brush {
        Paint::Solid(color) => {
            assert_eq!(
                *color,
                Color::from_rgba8(255, 0, 0, 255),
                "inline color:red must produce Color::from_rgba8(255,0,0,255) brush"
            );
        }
        other => panic!(
            "expected Paint::Solid, got {:?}",
            std::mem::discriminant(other)
        ),
    }
}

#[test]
fn paint_single_page_underline_uses_decoration_color() {
    use anyrender::types::Paint;
    use peniko::Color;

    let scene = decorated_text_scene(
        "color:red; text-decoration-line:underline; text-decoration-color:blue",
    );
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect();
    assert_eq!(fills.len(), 2, "canvas + one underline fill expected");
    match &fills[1].brush {
        Paint::Solid(color) => assert_eq!(*color, Color::from_rgba8(0, 0, 255, 255)),
        // cov:ignore: the recording scene stores this brush as a solid
        // color for every supported decoration style.
        other => panic!("expected a solid decoration brush, got {other:?}"),
    }
}

#[test]
fn paint_single_page_text_decoration_line_keywords_paint_all_three_lines() {
    let scene = decorated_text_scene(
        "text-decoration-line: underline overline line-through; text-decoration-color:green",
    );
    let fills = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    // cov:ignore: the assertion message is evaluated only when this
    // regression assertion fails.
    assert_eq!(
        fills, 4,
        "canvas plus underline, overline, and line-through fills expected"
    );
}

#[test]
fn paint_single_page_text_decoration_paints_around_glyphs_in_css_order() {
    let scene = decorated_text_scene(
        "text-decoration-line: underline overline line-through; text-decoration-style:solid",
    );
    let glyph_index = scene
        .commands
        .iter()
        .position(|command| matches!(command, RenderCommand::GlyphRun(_)))
        .expect("decorated text must emit a GlyphRun");
    let before_glyph_fills = scene.commands[..glyph_index]
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    let after_glyph_fills = scene.commands[glyph_index + 1..]
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count();
    // The canvas plus underline/overline precede text; line-through is
    // painted after the glyph run.
    assert_eq!(before_glyph_fills, 3);
    assert_eq!(after_glyph_fills, 1);
}

#[test]
fn paint_single_page_nested_decorations_follow_line_order() {
    use anyrender::types::Paint;
    use peniko::Color;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("text-decoration-line:overline; text-decoration-color:red"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("text-decoration-line:underline; text-decoration-color:blue"),
    );
    let _text = doc.append_text(child, "nested");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect();
    assert_eq!(fills.len(), 3);
    // Underline is the bottommost decoration; overline is above it.
    assert!(matches!(
        &fills[1].brush,
        Paint::Solid(color) if *color == Color::from_rgba8(0, 0, 255, 255)
    ));
    assert!(matches!(
        &fills[2].brush,
        Paint::Solid(color) if *color == Color::from_rgba8(255, 0, 0, 255)
    ));
}

#[test]
fn paint_single_page_decoration_trims_ltr_and_rtl_line_edge_whitespace() {
    for (style, text) in [
        (
            "white-space:pre; text-decoration-line:underline",
            "  Decoration  ",
        ),
        (
            "direction:rtl; white-space:pre; text-decoration-line:underline",
            "  שלום  ",
        ),
    ] {
        let scene = decorated_text_scene_with_text(style, text);
        let fills = scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Fill(_)))
            .count();
        assert_eq!(fills, 2, "canvas plus one trimmed decoration expected");
    }
}

#[test]
fn paint_single_page_text_decoration_styles_reach_scene() {
    let cases = [
        ("solid", Some(2), 0),
        ("double", Some(3), 0),
        ("dotted", None, 0),
        ("dashed", Some(1), 1),
        ("wavy", Some(1), 1),
    ];
    for (style, expected_fills, expected_strokes) in cases {
        let scene = decorated_text_scene(&format!(
            "text-decoration-line:underline; text-decoration-style:{style}"
        ));
        let fills = scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Fill(_)))
            .count();
        let strokes = scene
            .commands
            .iter()
            .filter(|command| matches!(command, RenderCommand::Stroke(_)))
            .count();
        if let Some(expected_fills) = expected_fills {
            // cov:ignore: the assertion message is evaluated only when
            // this per-style regression assertion fails.
            assert_eq!(
                fills, expected_fills,
                "unexpected Fill count for text-decoration-style:{style}"
            );
        } else {
            // cov:ignore: the assertion message is evaluated only when
            // this dotted-style regression assertion fails.
            assert!(
                fills > 1,
                "dotted decoration must emit at least one dot in addition to the canvas fill"
            );
        }
        // cov:ignore: the assertion message is evaluated only when this
        // per-style regression assertion fails.
        assert_eq!(
            strokes, expected_strokes,
            "unexpected Stroke count for text-decoration-style:{style}"
        );
    }
}

#[test]
fn paint_single_page_html_decoration_propagates_into_body() {
    use anyrender::types::Paint;
    use peniko::Color;

    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("text-decoration-line:underline; text-decoration-color:red"),
    );
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("color:blue; text-decoration-line:none"),
    );
    let _text = doc.append_text(p, "root");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect();
    // The canvas and html-originated underline are both fills.
    // cov:ignore: the assertion message is evaluated only when this
    // root-decoration regression assertion fails.
    assert_eq!(fills.len(), 2, "html decoration must reach body text");
    match &fills[1].brush {
        Paint::Solid(color) => assert_eq!(*color, Color::from_rgba8(255, 0, 0, 255)),
        // cov:ignore: the recording scene stores this brush as a solid
        // color for every supported decoration style.
        other => panic!("expected a solid root decoration brush, got {other:?}"),
    }
}

#[test]
fn paint_single_page_ancestor_decoration_propagates_and_keeps_origin_color() {
    use anyrender::types::Paint;
    use peniko::Color;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("color:red; text-decoration-line:underline"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("color:blue; text-decoration-line:none"),
    );
    let _text = doc.append_text(child, "child");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let glyph = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::GlyphRun(glyph) => Some(glyph),
            _ => None,
        })
        .expect("child text must paint");
    match &glyph.brush {
        Paint::Solid(color) => assert_eq!(*color, Color::from_rgba8(0, 0, 255, 255)),
        // cov:ignore: the recording scene stores the text brush as a
        // solid color on this path.
        other => panic!("expected a solid text brush, got {other:?}"),
    }
    let decoration = scene
        .commands
        .iter()
        .rev()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => Some(fill),
            // cov:ignore: this search intentionally skips glyph/stroke
            // commands until it reaches the final decoration Fill.
            _ => None,
        })
        .expect("propagated underline must paint");
    match &decoration.brush {
        Paint::Solid(color) => assert_eq!(*color, Color::from_rgba8(255, 0, 0, 255)),
        // cov:ignore: the recording scene stores this brush as a solid
        // color for every supported decoration style.
        other => panic!("expected a solid decoration brush, got {other:?}"),
    }
}

#[test]
fn paint_single_page_positions_glyphs_via_absolute_offset() {
    // Match exactly how the body/p/text accumulated locations appear in the
    // `draw_glyphs` transform.
    //
    // A weak `translation >= 0` check also passes for an identity transform
    // (which omits accumulation entirely). Add a nonzero taffy margin to `<p>`
    // so p.location.x/y is nonzero and the accumulation logic is exercised.
    // Expected: body.location + p.location + text.location (block-flow
    // accumulation).
    //
    // `apply_computed_to_style` converts cascade values to taffy through
    // `bridge_margin`, so the cascade's initial zero overwrites a hand-built
    // taffy `Style { margin: ... }`. Specify the margin through inline CSS
    // (`style="margin: ..."`) instead; this also matches the production
    // code path.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // `margin: 20px 0px 0px 20px` (top=20, right=0, bottom=0, left=20)
    // recreates the old hand-built shape in CSS. Specify `0px` explicitly:
    // raikiri-style `parse_length_value` does not accept bare unitless `0`
    // in this spec subset, so all four shorthand sides need units.
    // Preserve `color:red` by concatenating it: an existing assertion uses
    // it to catch brush-path regressions.
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("margin: 20px 0px 0px 20px; color: red"),
    );
    let text = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // Check that margin=20 makes the accumulated block-flow position
    // nonzero: its top margin can collapse from p onto body.
    let body_loc = doc.get_node(body).unwrap().unrounded_layout.location;
    let p_loc = doc.get_node(p).unwrap().unrounded_layout.location;
    assert!(
        p_loc.x >= 20.0,
        "p.location.x should reflect margin=20, got {}",
        p_loc.x
    );
    assert!(
        body_loc.y + p_loc.y >= 20.0,
        "body+p accumulated y should reflect margin=20, got body {} + p {}",
        body_loc.y,
        p_loc.y
    );

    // Expected accumulated location = body.location + p.location + text.location.
    let (expected_x, expected_y) =
        [body, p, text]
            .iter()
            .fold((0.0f32, 0.0f32), |(ax, ay), &id| {
                let loc = doc.get_node(id).unwrap().unrounded_layout.location;
                (ax + loc.x, ay + loc.y)
            });

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let RenderCommand::GlyphRun(glyph_cmd) = scene
        .commands
        .iter()
        .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .expect("must have 1 GlyphRun")
    else {
        unreachable!()
    };
    // Affine translation is in `[4, 5]` of `as_coeffs()` (2D identity plus
    // translation). kurbo::Affine has no translation() getter, so decode
    // those coefficients directly.
    let coeffs = glyph_cmd.transform.as_coeffs();
    let epsilon = 1e-5f64;
    assert!(
        (coeffs[4] - expected_x as f64).abs() < epsilon,
        "translation.x = {}, expected accumulated = {} (body {} + p {} + text {})",
        coeffs[4],
        expected_x,
        doc.get_node(body).unwrap().unrounded_layout.location.x,
        doc.get_node(p).unwrap().unrounded_layout.location.x,
        doc.get_node(text).unwrap().unrounded_layout.location.x,
    );
    assert!(
        (coeffs[5] - expected_y as f64).abs() < epsilon,
        "translation.y = {}, expected accumulated = {} (body {} + p {} + text {})",
        coeffs[5],
        expected_y,
        doc.get_node(body).unwrap().unrounded_layout.location.y,
        doc.get_node(p).unwrap().unrounded_layout.location.y,
        doc.get_node(text).unwrap().unrounded_layout.location.y,
    );
}

/// `<html><head></head><body><p><span style="{span_style}">Sub</span></p></body></html>`
/// After processing it, return the Y transform component (`as_coeffs()[5]`)
/// of the only GlyphRun for the Text inside `<span>`. If `span_style` is `None`,
/// omit the `style` attribute (no author `vertical-align`, so the cascade
/// uses the initial `VerticalAlign::Baseline`).
///
/// Neither `<p>` nor `<span>` specifies an author font-size, so the CSS
/// initial value (16px) is the used font-size. The parent (`<p>`) used
/// font-size underlying the `vertical-align: sub` / `super` shift in
/// [`crate::walk::vertical_align_shift_px`] is thus always 16px here.
/// Shared helper for the `paint_single_page_vertical_align_*` tests.
///
fn span_text_glyph_y(span_style: Option<&str>) -> f32 {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let span = doc.append_element(Some(p), "span", Style::default(), span_style);
    let _text = doc.append_text(span, "Sub");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let RenderCommand::GlyphRun(glyph_cmd) = scene
        .commands
        .iter()
        .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .expect("must have 1 GlyphRun")
    else {
        unreachable!()
    };
    glyph_cmd.transform.as_coeffs()[5] as f32
}

#[test]
fn paint_single_page_vertical_align_sub_shifts_glyph_down_by_one_fifth_parent_font_size() {
    // Differential assertion (not an absolute `coeffs[5]` match) — the
    // delta between the styled and unstyled fixture isolates exactly
    // the vertical-align contribution, independent of whatever the
    // baseline absolute position happens to be. CSS Inline Layout
    // Module Level 3 §4.2.3 <https://www.w3.org/TR/css-inline-3/#baseline-shift-property>
    // sub fallback: "dropping by one fifth of the parent's used
    // font-size" — parent here is `<p>`, used font-size 16px (helper
    // doc).
    let baseline_y = span_text_glyph_y(None);
    let sub_y = span_text_glyph_y(Some("vertical-align: sub"));
    let expected_shift = 16.0 / 5.0;
    let epsilon = 1e-4;
    assert!(
        (sub_y - baseline_y - expected_shift).abs() < epsilon,
        "vertical-align: sub delta = {} (baseline {}, sub {}), expected {}",
        sub_y - baseline_y,
        baseline_y,
        sub_y,
        expected_shift
    );
}

#[test]
fn paint_single_page_vertical_align_super_shifts_glyph_up_by_one_third_parent_font_size() {
    // Same differential shape as the `sub` test above. CSS Inline 3
    // §4.2.3 super fallback: "raising by one third of the parent's
    // used font-size" — raikiri-paint's Y axis grows downward
    // (`draw_text_node`'s `abs_y` convention), so "raise" is a
    // negative delta.
    let baseline_y = span_text_glyph_y(None);
    let super_y = span_text_glyph_y(Some("vertical-align: super"));
    let expected_shift = -(16.0 / 3.0);
    let epsilon = 1e-4;
    assert!(
        (super_y - baseline_y - expected_shift).abs() < epsilon,
        "vertical-align: super delta = {} (baseline {}, super {}), expected {}",
        super_y - baseline_y,
        baseline_y,
        super_y,
        expected_shift
    );
}

#[test]
fn paint_single_page_vertical_align_length_raises_or_lowers_glyph_by_px_value() {
    let baseline_y = span_text_glyph_y(None);
    let raised_y = span_text_glyph_y(Some("vertical-align: 96px"));
    let lowered_y = span_text_glyph_y(Some("vertical-align: -12px"));
    let epsilon = 1e-4;
    assert!((raised_y - baseline_y + 96.0).abs() < epsilon);
    assert!((lowered_y - baseline_y - 12.0).abs() < epsilon);
}

#[test]
fn paint_single_page_vertical_align_explicit_baseline_matches_implicit_default() {
    // `vertical-align: baseline` (explicit author value, spec initial)
    // must contribute the same zero shift as no author declaration at
    // all — pins that `VerticalAlign::Baseline` isn't accidentally
    // routed into a nonzero arm.
    let implicit_y = span_text_glyph_y(None);
    let explicit_y = span_text_glyph_y(Some("vertical-align: baseline"));
    assert_eq!(
        implicit_y, explicit_y,
        "explicit `vertical-align: baseline` must not shift relative to the implicit default"
    );
}

#[test]
fn paint_single_page_vertical_align_sub_on_block_level_element_does_not_shift() {
    // CSS 2.1 §10.8.1 "Applies to: inline-level and 'table-cell'
    // elements" <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align> —
    // a block-level box's `vertical-align: sub` must not shift its
    // text, pinning `vertical_align_shift_px`'s `DisplayValue` gate.
    let baseline_y = span_text_glyph_y(None); // <span> is inline by default (CSS initial)
    let block_y = span_text_glyph_y(Some("display: block; vertical-align: sub"));
    assert_eq!(
        baseline_y, block_y,
        "block-level vertical-align: sub must not contribute a shift"
    );
}

#[test]
fn paint_single_page_vertical_align_nested_sub_composes_by_addition() {
    // Two nested `<span vertical-align: sub>` levels use the same 16px
    // parent-font-size basis. This crate's `shift_y` stack-frame accumulator
    // (documented in `crate::walk::paint_document` and the nested-shift
    // note in `vertical_align_shift_px`) sums each ancestor's shift. The
    // total should be twice the single-level shift. No spec cites this
    // composition rule; it is documented as an approximation. This test
    // pins the *mechanical* accumulation, not a spec requirement.
    // Both spans use the same inherited font-size for their parent basis.
    // The comparison below verifies the accumulated offset.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let outer = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("vertical-align: sub"),
    );
    let inner = doc.append_element(
        Some(outer),
        "span",
        Style::default(),
        Some("vertical-align: sub"),
    );
    let _text = doc.append_text(inner, "Sub");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let RenderCommand::GlyphRun(glyph_cmd) = scene
        .commands
        .iter()
        .find(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .expect("must have 1 GlyphRun")
    else {
        unreachable!()
    };
    let nested_y = glyph_cmd.transform.as_coeffs()[5] as f32;

    let baseline_y = span_text_glyph_y(None);
    let single_shift = 16.0 / 5.0;
    let expected = baseline_y + 2.0 * single_shift;
    let epsilon = 1e-4;
    assert!(
        (nested_y - expected).abs() < epsilon,
        "nested double-sub y = {nested_y}, expected {expected} (baseline {baseline_y} + 2 * {single_shift})"
    );
}

/// Process `<p>H<sub>2</sub>O</p>` with `raikiri_html::parse` and the UA
/// stylesheet (`sub { vertical-align: sub }` plus `sub, sup { font-size:
/// smaller; ... }` from `minimal.css`), then return the Y transform component
/// of the GlyphRun for the "2" text inside `<sub>`.
///
/// `build_rule_tree` collects only author `<style>` elements and does not
/// include UA CSS automatically (the `raikiri_style::ruletree::walk_style_elements`
/// docs say that raikiri-html::parse injects UA CSS via
/// Document::add_stylesheet, while the raikiri umbrella separately consumes it
/// via Document::stylesheets()). Thus the caller must explicitly call
/// `rules.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent)`.
/// This adds the UA declarations omitted by `build_rule_tree`.
///
/// `extra_head_style` adds author declarations through a `<style>` element
/// in `<head>`. Origin::Author outranks UA declarations of equal specificity:
/// passing `sub { vertical-align: baseline; }` overrides the UA rule
/// `sub { vertical-align: sub }`, but keeps the UA's
/// `font-size: smaller` declaration.
fn ua_sub_text_glyph_y(extra_head_style: &str) -> f32 {
    use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, parse};
    use raikiri_style::Origin;

    let html = format!(
        "<html><head><style>{extra_head_style}</style></head>\
             <body><p>H<sub>2</sub>O</p></body></html>"
    );
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
    let mut doc = uncascaded.dom;
    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::GlyphRun(cmd) => Some(cmd),
            _ => None,
        })
        .collect();
    // "H", "2" (inside <sub>), "O" are 3 separate Text nodes and —
    // absent an inline formatting context — each lays out as its own
    // block row (`crate::walk` module doc), so each shapes to its own
    // GlyphRun in document order: index 1 is <sub>'s "2".
    assert_eq!(
        glyph_commands.len(),
        3,
        "expected 3 GlyphRuns (H, <sub>'s 2, O), got {}",
        glyph_commands.len()
    );
    glyph_commands[1].transform.as_coeffs()[5] as f32
}

#[test]
fn paint_single_page_ua_sub_shift_uses_parent_font_size_not_subs_own_shrunk_size() {
    // Exercises the real path this task exists for — the UA rules
    // `sub { vertical-align: sub }` + `sub, sup { font-size: smaller;
    // ... }` (`minimal.css`) both apply to the same `<sub>` element,
    // reached through `raikiri_html::parse`, not the inline-style
    // shortcut the tests above use.
    //
    // This is the one case where the shift's font-size basis actually
    // matters: `<sub>`'s own used font-size is shrunk by `font-size:
    // smaller` (CSS Fonts 4 §2.2's `<relative-size>` table,
    // `resolve_relative_font_size`'s `RATIO = 1.2` — ~13.33px here),
    // but `vertical_align_shift_px`'s basis is `<sub>`'s *parent*'s
    // (`<p>`'s) used font-size, 16px, untouched by that shrink. Both
    // fixtures below share the same shrunk `<sub>` font-size (the
    // override only touches `vertical-align`), isolating the shift
    // contribution: if the implementation used `<sub>`'s own shrunk
    // font-size as the basis instead, this delta would be
    // (16.0/1.2)/5.0 ≈ 2.667 rather than 16.0/5.0 = 3.2 — a
    // difference this exact-match assertion would catch.
    let shifted_y = ua_sub_text_glyph_y(""); // UA `vertical-align: sub` applies
    let unshifted_y = ua_sub_text_glyph_y("sub { vertical-align: baseline; }");
    let expected_shift = 16.0 / 5.0;
    let epsilon = 1e-3;
    assert!(
        (shifted_y - unshifted_y - expected_shift).abs() < epsilon,
        "UA sub shift delta = {} (shifted {}, unshifted {}), expected {}",
        shifted_y - unshifted_y,
        shifted_y,
        unshifted_y,
        expected_shift
    );
}

#[test]
fn paint_single_page_skips_empty_text() {
    // A Text node whose text_layout is None (empty text) must silently skip
    // `draw_glyphs`. This mirrors `preshape_text`, which leaves text_layout
    // as None for empty text.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _t = doc.append_text(p, ""); // ← empty text
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .collect();
    assert!(
        glyph_commands.is_empty(),
        "empty text should not emit any GlyphRun, got {}",
        glyph_commands.len()
    );
}

#[test]
fn paint_single_page_can_be_called_multiple_times() {
    // Paint the same Document twice and require the same command sequence
    // both times (no state mutation, reentrancy). Catch silent regressions
    // if paint-side caching is added later.
    let (doc, cr) = hello_world_paint_setup();

    let mut scene1 = Scene::new();
    paint_single_page(&mut scene1, &doc, &cr, PageBox::A4);
    let count1 = scene1.commands.len();
    let glyph_count1 = scene1
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count();

    let mut scene2 = Scene::new();
    paint_single_page(&mut scene2, &doc, &cr, PageBox::A4);
    let count2 = scene2.commands.len();
    let glyph_count2 = scene2
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count();

    assert_eq!(
        count1, count2,
        "total command count must be identical across calls"
    );
    assert_eq!(
        glyph_count1, glyph_count2,
        "GlyphRun count must be identical across calls"
    );
    assert_eq!(glyph_count1, 1, "hello world must emit exactly 1 GlyphRun");
}

/// Check that text inside HTML hidden elements (`<style>` / `<script>` /
/// `<noscript>` / `<datalist>` / `<noembed>` / `<noframes>` / `<rp>` and others)
/// does not leak into the rendered output.
/// Surrounding text must remain visible as well.
///
/// In each fixture:
/// - Put "before" text before the inert element and "after" text after it;
///   both must be painted (two GlyphRuns).
/// - Raw text inside the inert element must not be painted (no leak).
///
/// Merely testing for zero painted glyphs would also pass if the inert filter
/// wrongly removed the **entire** subtree. The three-way check (keep before
/// and after; drop inert content) verifies that the filter is precise.
///
/// HTML LS §15.3.1 "Hidden elements"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>)
/// is the primary source.
fn assert_inert_html_content_not_painted(html: &[u8], fixture_label: &str) {
    use raikiri_html::{ParseOptions, parse};

    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(html, &opts).expect("parse ok");
    let mut doc = uncascaded.dom;
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::GlyphRun(cmd) => Some(cmd),
            _ => None,
        })
        .collect();
    // Exactly two GlyphRuns (before + after): a leak yields three, while a
    // lost boundary yields one or zero. Also detect over-filtering.
    assert_eq!(
        glyph_commands.len(),
        2,
        "[{fixture_label}] expected 2 GlyphRuns (before + after), got {}. \
             Any extra run indicates inert-element text leaked into paint.",
        glyph_commands.len()
    );
    // 2-of-3 selection (before + inert
    // A count of two could still pass if one boundary vanished while inert
    // content survived. Pin the total glyph count to "before" (6 chars) plus
    // "after" (5 chars) = 11. Leaked inert content increases the count;
    // losing boundary text decreases it. Each caller of
    // `assert_inert_html_content_not_painted` must use the same
    // "before…after" fixture text.
    let total_glyphs: usize = glyph_commands.iter().map(|cmd| cmd.glyphs.len()).sum();
    assert_eq!(
        total_glyphs,
        "before".len() + "after".len(),
        "[{fixture_label}] total glyph count must equal len('before') + len('after') = 11, \
             got {total_glyphs}. Any deviation indicates either inert-element leak (too many) \
             or boundary text drop (too few) — 2-of-3 selection also fails this exact match."
    );
}

#[test]
fn paint_single_page_skips_style_subtree_content() {
    // <body>before<style>#a{color:red}</style>after</body>
    // HTML LS §15.3.1 "Hidden elements" covers <style>; its raw text
    // (`#a{color:red}`) must not be painted. The before/after text
    // must be painted (two GlyphRuns).
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<style>#a{color:red}</style>after</body></html>",
        "style",
    );
}

#[test]
fn paint_single_page_skips_script_subtree_content() {
    // <body>before<script>alert(1)</script>after</body>
    // Do not paint <script> raw text (HTML LS §4.12.1).
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<script>alert(1)</script>after</body></html>",
        "script",
    );
}

#[test]
fn paint_single_page_skips_noscript_subtree_content() {
    // <body>before<noscript>fallback</noscript>after</body>
    // With scripting_enabled=true (html5ever default, inherited in parse.rs),
    // <noscript> content is tokenized as raw text and must not be painted.
    // If scripting_enabled becomes false, quarantine and redesign this test.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<noscript>fallback</noscript>after</body></html>",
        "noscript",
    );
}

// Four remaining §15.3.1 elements (datalist / noembed / noframes / rp).
// Like style / script / noscript / template fixtures, these use the
// same three-way check (`before` + `after` = 11 glyphs; leaked inert text
// makes total_glyphs > 11).
//
// HTML5 parsing behavior for each element:
// - `<datalist>` is a normal element; its character tokens remain Text
//   children (HTML LS §4.10.8). Even if author CSS overrides §15.3.1's
//   UA `display:none`, the paint-side predicate skips its subtree.
// - `<noembed>` / `<noframes>` switch to the RAWTEXT tokenizer in the
//   "in body" insertion mode (HTML LS §13.2.6.4.7); their text is one child.
// - `<rp>` is parsed as a normal element. Even outside `<ruby>`, "in body"
//   inserts it normally (when the current node is not ruby / rtc, only a
//   parse error is marked; structure remains, HTML LS §13.2.6.4.7).
//   The §15.3.1 hidden-elements rule sets `display: none` unconditionally;
//   §15.3.4 "Phrasing content" only styles ruby / rt, not rp. The paint-side
//   defense-in-depth gate prevents content leaks even without ruby support.
//

#[test]
fn paint_single_page_skips_datalist_subtree_content() {
    // <body>before<datalist>hidden</datalist>after</body>
    // §15.3.1 hides <datalist> with `display: none`. Prevent author CSS
    // overrides from leaking its content into painted glyphs.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<datalist>hidden</datalist>after</body></html>",
        "datalist",
    );
}

#[test]
fn paint_single_page_skips_noembed_subtree_content() {
    // <body>before<noembed>hidden</noembed>after</body>
    // <noembed> uses RAWTEXT parsing (§13.2.6.4.7): its internal "hidden"
    // raw text remains a Text child, then §15.3.1 applies display:none.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<noembed>hidden</noembed>after</body></html>",
        "noembed",
    );
}

#[test]
fn paint_single_page_skips_noframes_subtree_content() {
    // <body>before<noframes>hidden</noframes>after</body>
    // <noframes> also uses RAWTEXT parsing (§13.2.6.4.7) and §15.3.1 hiding.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<noframes>hidden</noframes>after</body></html>",
        "noframes",
    );
}

#[test]
fn paint_single_page_skips_rp_subtree_content() {
    // <body>before<rp>hidden</rp>after</body>
    // <rp> is fallback parentheses for ruby. "In body" inserts it normally
    // even outside `<ruby>` (§13.2.6.4.7: a current node other than ruby / rtc
    // marks a parse error but preserves the structure).
    // §15.3.1 hides it via `display: none`; §15.3.4 only styles ruby / rt
    // and does not make rp visible. This defense-in-depth gate prevents
    // content leaks even without ruby support.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<rp>hidden</rp>after</body></html>",
        "rp",
    );
}

#[test]
fn paint_single_page_skips_template_subtree_content_via_inert_predicate() {
    // <template> is already skipped by the is_in_document() gate. Also pin
    // that is_non_rendered_html_element() independently skips it.
    // "before<template>...</template>after" yields two GlyphRuns.
    assert_inert_html_content_not_painted(
        b"<html><head></head><body>before<template><p>secret</p></template>after</body></html>",
        "template",
    );
}

#[test]
fn paint_single_page_skips_template_subtree_without_display_none_ua_rule() {
    // Pin the contract: the is_in_document() predicate explicitly skips
    // template subtrees regardless of any UA CSS template { display: none }
    // rule. Prevent regression of this formerly silent bug.
    //
    // Setup: <body><template><p>should_not_paint</p></template></body>.
    // Current UA CSS has no template rule (checked in minimal.css), so its
    // default is Display::Block. Before the gate, painting descended into
    // the subtree and emitted a GlyphRun without reporting the bug.
    use raikiri_html::{ParseOptions, parse};

    let html = b"<html><head></head><body>\
                     <template><p>should_not_paint</p></template>\
                     </body></html>";
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    // parse returns an UncascadedDocument; extract its DOM, then cascade, layout, and paint.
    let mut doc = uncascaded.dom;
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

    let glyph_commands: Vec<_> = scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .collect();
    assert!(
        glyph_commands.is_empty(),
        "text inside <template> subtree should not paint (is_in_document gate), got {} glyph runs",
        glyph_commands.len()
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must equal document.node_count()")]
fn paint_single_page_debug_asserts_cascade_document_length_match() {
    // Intentionally violate the module doc's `## Contract` (`cascade.computed.len() ==
    // document.node_count()`) by adding a node to the arena after cascading,
    // leaving `cascade.computed` behind. Check that `paint_single_page`'s
    // initial debug_assert catches the violation and panics. In release
    // builds this assertion is omitted, so gate this test with
    // #[cfg(debug_assertions)]; otherwise `cargo test --release` fails.
    // Release builds deliberately do not enforce this invariant.
    let (mut doc, cr) = hello_world_paint_setup();
    doc.append_element(Some(0), "p", Style::default(), None::<&str>);
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cr, PageBox::A4);
}

#[test]
fn img_element_paints_its_decoded_pixels() {
    use raikiri_traits::{DecodedImage, ImagePixelSource, ImageRasterSize};
    use std::sync::Arc;

    struct OneImageSource(
        url::Url,
        Arc<DecodedImage>,
        Arc<std::sync::Mutex<Vec<ImageRasterSize>>>,
    );
    impl ImagePixelSource for OneImageSource {
        fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
            (*url == self.0).then(|| self.1.clone())
        }

        fn get_decoded_at_size(
            &self,
            url: &url::Url,
            size: ImageRasterSize,
            _max_output_bytes: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            if *url != self.0 {
                return None;
            }
            self.2
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(size);
            Some(self.1.clone())
        }
    }

    // A single `<img src="file:///x.png">` inside <body>, sized via
    // inline style (same idiom as `hello_world_paint_setup`'s
    // `Some("color:red")`) so the content box has a known, non-zero
    // size independent of any resolver-driven intrinsic sizing layout
    // may separately apply — this test only checks paint's own draw
    // call given a known box.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let img = doc.append_element(
        Some(body),
        "img",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let url = url::Url::parse("file:///x.png").unwrap();
    let decoded = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    });
    let requested_sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pixel_source = OneImageSource(url, decoded, Arc::clone(&requested_sizes));

    let mut scene = Scene::new();
    // Exercises the public entry point, not just the private
    // `walk::paint_document_with_images` it wraps, so this test also
    // covers `paint_single_page_with_images`'s own debug_assert and
    // canvas-background call.
    paint_single_page_with_images(&mut scene, &doc, &cr, PageBox::A4, &pixel_source);

    let fill = scene
        .commands
        .iter()
        .find_map(|cmd| match cmd {
            RenderCommand::Fill(f) if matches!(f.brush, anyrender::types::Paint::Image(_)) => {
                Some(f)
            }
            _ => None,
        })
        .expect("expected an image fill command in the recorded scene");

    assert_eq!(
        *requested_sizes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![ImageRasterSize {
            width: 10.0,
            height: 10.0,
        }],
        "paint must request source pixels at the concrete object-fit viewport"
    );

    // `object-fit: fill` (the only value this scope implements, CSS
    // Images 3 §4.3) must stretch the 1×1 source to exactly cover the
    // 10×10 content box (no border/padding here, so content box ==
    // border box) — not draw it at its native size with an identity
    // transform. `Affine::translate(t) * Affine::scale_non_uniform(sx,
    // sy)` composes to coeffs `[sx, 0, 0, sy, tx, ty]` (kurbo's `Mul for
    // Affine`), so `as_coeffs()[0]`/`[3]` are the scale factors and
    // `[4]`/`[5]` are the translation.
    let body_loc = doc.get_node(body).unwrap().unrounded_layout.location;
    let img_loc = doc.get_node(img).unwrap().unrounded_layout.location;
    let expected_x = (body_loc.x + img_loc.x) as f64;
    let expected_y = (body_loc.y + img_loc.y) as f64;
    let coeffs = fill.transform.as_coeffs();
    let epsilon = 1e-4;
    assert!(
        (coeffs[0] - 10.0).abs() < epsilon && (coeffs[3] - 10.0).abs() < epsilon,
        "expected the 1x1 source scaled by 10x/10y to fill the 10px content box, got scale ({}, {})",
        coeffs[0],
        coeffs[3]
    );
    assert!(
        (coeffs[4] - expected_x).abs() < epsilon && (coeffs[5] - expected_y).abs() < epsilon,
        "expected the image translated to the content-box origin ({}, {}), got ({}, {})",
        expected_x,
        expected_y,
        coeffs[4],
        coeffs[5]
    );
}

#[test]
fn inline_svg_is_atomic_and_groups_root_opacity_with_decorations() {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::sync::Arc;

    struct EmptyImageSource;
    impl ImagePixelSource for EmptyImageSource {
        fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let style = doc.append_element(Some(html), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        ".svg-root { opacity:0.75; background-color:blue; border:1px solid red }",
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("color:rgb(0,128,0)"),
    );
    let svg = doc.append_element(
        Some(body),
        "svg",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(
        svg,
        vec![
            ("class".into(), "svg-root".into()),
            ("opacity".into(), "0.25".into()),
            ("viewBox".into(), "0 0 1 1".into()),
            ("width".into(), "1".into()),
            ("height".into(), "1".into()),
        ],
    );
    let rect = doc.append_element(Some(svg), "rect", Style::default(), None::<&str>);
    doc.set_element_namespace(rect, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(
        rect,
        vec![
            ("width".into(), "1".into()),
            ("height".into(), "1".into()),
            ("fill".into(), "currentColor".into()),
        ],
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade succeeds");
    assert_eq!(cascade.computed[svg].opacity, 0.75);
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    assert!(doc.get_node(svg).unwrap().is_inline_svg_root());
    assert!(doc.get_node(rect).unwrap().is_inline_svg_content());
    assert!(doc.get_node(svg).unwrap().layout_children().is_empty());
    let mut scene = Scene::new();
    let warnings = paint_single_page_with_images_and_warnings(
        &mut scene,
        &doc,
        &cascade,
        PageBox::A4,
        &EmptyImageSource,
    );
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::types::Paint::Image(brush) => Some(&brush.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG produces an image fill");

    assert!(warnings.is_empty());
    assert_eq!((image.width, image.height), (10, 10));

    let layer_start = scene
        .commands
        .iter()
        .position(|command| matches!(command, RenderCommand::PushLayer(layer) if (layer.alpha - 0.75).abs() < f32::EPSILON))
        .expect("computed SVG opacity wraps the complete paint item");
    let layer_end = scene
        .commands
        .iter()
        .enumerate()
        .skip(layer_start + 1)
        .find_map(|(index, command)| matches!(command, RenderCommand::PopLayer).then_some(index))
        .expect("opacity layer closes after the SVG paint item");
    let grouped_commands = &scene.commands[layer_start + 1..layer_end];
    assert!(grouped_commands.iter().any(|command| matches!(
        command,
        RenderCommand::Fill(fill)
            if fill.brush == anyrender::Paint::Solid(Color::from_rgba8(0, 0, 255, 255))
    )));
    assert!(grouped_commands.iter().any(|command| matches!(
        command,
        RenderCommand::Fill(fill)
            if fill.brush == anyrender::Paint::Solid(Color::from_rgba8(255, 0, 0, 255))
    )));
    assert!(grouped_commands.iter().any(|command| matches!(
        command,
        RenderCommand::Fill(fill)
            if matches!(&fill.brush, anyrender::Paint::Image(_))
    )));
    assert_eq!(&image.data.as_ref()[..4], &[0, 128, 0, 255]);
}

#[test]
fn html_inline_svg_stylesheet_opacity_groups_the_complete_root() {
    use raikiri_html::{ParseOptions, parse};

    let html = br#"<html><body><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>svg { opacity:0.25 }</style><rect width="10" height="10" fill="red"/></svg></body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&html[..], &options).expect("HTML parse succeeds");
    assert_eq!(uncascaded.stylesheet_sources.len(), 1);
    let cascade = raikiri_html::build_cascaded(&uncascaded);
    let mut doc = uncascaded.dom;
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::types::Paint::Image(brush) => Some(&brush.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG produces an image fill");

    assert!(scene.commands.iter().any(|command| matches!(
        command,
        RenderCommand::PushLayer(layer) if (layer.alpha - 0.25).abs() < f32::EPSILON
    )));
    assert_eq!(image.data.as_ref()[3], 255);
}

#[test]
fn html_inline_svg_preserves_stylesheet_inherited_opacity() {
    use raikiri_html::{ParseOptions, parse};

    let html = br#"<html><body><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" style="opacity:.5"><style>rect { opacity:inherit }</style><rect width="10" height="10" fill="red"/></svg></body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&html[..], &options).expect("HTML parse succeeds");
    let cascade = raikiri_html::build_cascaded(&uncascaded);
    let mut doc = uncascaded.dom;
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::types::Paint::Image(brush) => Some(&brush.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG produces an image fill");

    assert!(scene.commands.iter().any(|command| matches!(
        command,
        RenderCommand::PushLayer(layer) if (layer.alpha - 0.5).abs() < f32::EPSILON
    )));
    assert_eq!(image.data.as_ref()[3], 128);
}

#[test]
fn html_inline_svg_root_background_stays_inside_the_opacity_group() {
    use raikiri_html::{ParseOptions, parse};

    let html = br#"<html><body><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" style="opacity:.5;background-color:rgba(0,0,255,.5)"><rect width="5" height="10" fill="red"/></svg></body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&html[..], &options).expect("HTML parse succeeds");
    let cascade = raikiri_html::build_cascaded(&uncascaded);
    let mut doc = uncascaded.dom;
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::types::Paint::Image(brush) => Some(&brush.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG produces an image fill");

    assert!(scene.commands.iter().any(|command| matches!(
        command,
        RenderCommand::PushLayer(layer) if (layer.alpha - 0.5).abs() < f32::EPSILON
    )));
    assert_eq!(&image.data.as_ref()[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.data.as_ref()[9 * 4..10 * 4], &[0, 0, 0, 0]);
    assert!(scene.commands.iter().any(|command| matches!(
        command,
        RenderCommand::Fill(fill)
            if fill.brush == anyrender::Paint::Solid(Color::from_rgba8(0, 0, 255, 128))
    )));
}

#[test]
fn html_inline_svg_host_transparent_background_suppresses_source_background() {
    use raikiri_html::{ParseOptions, parse};

    let html = br#"<html><head><style>svg { background-color: transparent !important }</style></head><body><svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><style>svg { background-color: blue }</style></svg></body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&html[..], &options).expect("HTML parse succeeds");
    let cascade = raikiri_html::build_cascaded(&uncascaded);
    let mut doc = uncascaded.dom;
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &doc, &cascade, PageBox::A4);
    let image = scene
        .commands
        .iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match &fill.brush {
                anyrender::types::Paint::Image(brush) => Some(&brush.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG produces an image fill");

    assert!(
        image
            .data
            .as_ref()
            .chunks_exact(4)
            .all(|rgba| rgba == [0, 0, 0, 0])
    );
}

fn paint_inline_svg_with_external_image(style: &str) -> Vec<raikiri_traits::RenderWarning> {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::sync::Arc;

    struct EmptyImageSource;
    impl ImagePixelSource for EmptyImageSource {
        fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
            None
        }
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let svg = doc.append_element(Some(body), "svg", Style::default(), Some(style));
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(
        svg,
        vec![
            ("viewBox".into(), "0 0 1 1".into()),
            ("width".into(), "1".into()),
            ("height".into(), "1".into()),
        ],
    );
    let image = doc.append_element(Some(svg), "image", Style::default(), None::<&str>);
    doc.set_element_namespace(image, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(
        image,
        vec![("href".into(), "https://images.test/external.png".into())],
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade succeeds");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");
    let mut scene = Scene::new();
    paint_single_page_with_images_and_warnings(
        &mut scene,
        &doc,
        &cascade,
        PageBox::A4,
        &EmptyImageSource,
    )
}

#[test]
fn inline_svg_raster_failure_warns_only_for_visible_positive_viewport() {
    let visible = paint_inline_svg_with_external_image("width:10px;height:10px");
    assert_eq!(visible.len(), 1);
    assert!(matches!(
        visible[0].kind,
        raikiri_traits::WarningKind::ResourceFallback {
            kind: raikiri_traits::ResourceKind::Image,
            ..
        }
    ));

    let hidden = paint_inline_svg_with_external_image("width:10px;height:10px;visibility:hidden");
    assert!(hidden.is_empty());

    let zero_width = paint_inline_svg_with_external_image("width:0px;height:10px");
    assert!(zero_width.is_empty());
}

#[test]
fn img_padding_percentage_resolves_against_width_not_height() {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::sync::Arc;

    struct OneImageSource(url::Url, Arc<DecodedImage>);
    impl ImagePixelSource for OneImageSource {
        fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
            (*url == self.0).then(|| self.1.clone())
        }
    }

    // CSS 2.1 §8.4 <https://www.w3.org/TR/CSS21/box.html#propdef-padding-top>:
    // every `padding-*` percentage — top/bottom included — resolves
    // against the containing block's *inline size* (width), never the
    // element's own height. width=100px, height=50px, padding-top:20%
    // deliberately makes "resolved against width" (20px) and "resolved
    // against height" (10px) disagree, so a wrong reference axis
    // produces a different, assertion-failing offset.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let img = doc.append_element(
        Some(body),
        "img",
        Style::default(),
        Some(
            "width:100px;height:50px;padding-top:20%;padding-right:0px;\
                 padding-bottom:0px;padding-left:0px",
        ),
    );
    doc.set_element_attributes(img, vec![("src".into(), "file:///pad.png".into())]);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let url = url::Url::parse("file:///pad.png").unwrap();
    let decoded = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![0, 255, 0, 255],
    });
    let pixel_source = OneImageSource(url, decoded);

    let mut scene = Scene::new();
    paint_single_page_with_images(&mut scene, &doc, &cr, PageBox::A4, &pixel_source);

    let fill = scene
        .commands
        .iter()
        .find_map(|cmd| match cmd {
            RenderCommand::Fill(f) if matches!(f.brush, anyrender::types::Paint::Image(_)) => {
                Some(f)
            }
            _ => None,
        })
        .expect("expected an image fill command in the recorded scene");

    let body_loc = doc.get_node(body).unwrap().unrounded_layout.location;
    let img_loc = doc.get_node(img).unwrap().unrounded_layout.location;
    // border-box top + padding-top resolved against width (100px * 20% = 20px).
    let expected_content_y = (body_loc.y + img_loc.y + 20.0) as f64;
    let coeffs = fill.transform.as_coeffs();
    let epsilon = 1e-4;
    assert!(
        (coeffs[5] - expected_content_y).abs() < epsilon,
        "content-box y translation = {}, expected {} (padding-top:20% of \
             width=100px is 20px, not 20% of height=50px which would be 10px)",
        coeffs[5],
        expected_content_y
    );
}

#[test]
fn img_ch_padding_uses_the_pre_taffy_used_value() {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::path::PathBuf;
    use std::sync::Arc;

    struct OneImageSource(url::Url, Arc<DecodedImage>);
    impl ImagePixelSource for OneImageSource {
        fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
            (*url == self.0).then(|| self.1.clone())
        }
    }

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    // cov:ignore: this integration fixture is intentionally skippable when shared WPT assets are absent.
    if !fonts_dir.join("Ahem.ttf").exists() {
        eprintln!(
            "skipping ch padding paint regression: Ahem.ttf is required under {}",
            fonts_dir.display()
        );
        return;
    }
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let img = doc.append_element(
            Some(body),
            "img",
            Style::default(),
            Some(
                "width:100px;height:50px;font-family:Ahem;font-size:16px;padding-top:1ch;                 padding-right:0px;padding-bottom:0px;padding-left:0px;                 background-color:rgb(255,0,0);background-clip:content-box",
            ),
        );
    doc.set_element_attributes(img, vec![("src".into(), "file:///ch-pad.png".into())]);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let fonts = raikiri_dom::fonts::build_wpt_font_ctx(&fonts_dir)
        .expect("bundled WPT fonts should register");
    layout_single_page(&mut doc, &cr, PageBox::A4, fonts).expect("layout Ok");

    let url = url::Url::parse("file:///ch-pad.png").unwrap();
    let decoded = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![0, 255, 0, 255],
    });
    let pixel_source = OneImageSource(url, decoded);
    let mut scene = Scene::new();
    paint_single_page_with_images(&mut scene, &doc, &cr, PageBox::A4, &pixel_source);
    let fill = scene
        .commands
        .iter()
        .find_map(|cmd| match cmd {
            RenderCommand::Fill(fill)
                if matches!(fill.brush, anyrender::types::Paint::Image(_)) =>
            {
                Some(fill)
            }
            _ => None,
        })
        .expect("expected an image fill command in the recorded scene");
    let body_loc = doc.get_node(body).unwrap().unrounded_layout.location;
    let img_loc = doc.get_node(img).unwrap().unrounded_layout.location;
    let expected_content_y = (body_loc.y + img_loc.y + 16.0) as f64;
    let coeffs = fill.transform.as_coeffs();
    assert!((coeffs[5] - expected_content_y).abs() < 1e-4);
}
#[test]
fn paint_single_page_border_radius_unifies_matching_border_and_background() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _box = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "width: 100px; height: 80px; background: red; border: 4px solid red; \
                 border-radius: 12px; background-clip: padding-box",
        ),
    );
    let _current_color = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "width: 100px; height: 80px; color: green; background: red; \
                 border: 4px solid currentcolor; border-radius: 12px; \
                 background-clip: padding-box",
        ),
    );
    let _dashed = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "width: 100px; height: 80px; background: red; border: 4px dashed blue; \
                 border-radius: 12px",
        ),
    );
    let rules = build_rule_tree(&document);
    let cascade_result = cascade(&document, &rules).expect("cascade Ok");
    let mut document = document;
    layout_single_page(
        &mut document,
        &cascade_result,
        PageBox::A4,
        FontContext::new(),
    )
    .expect("layout Ok");
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &document, &cascade_result, PageBox::A4);
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::Fill(_))),
        "rounded border/background fixture should emit fill commands", // cov:ignore: assertion message is evaluated only on failure
    );
}

#[cfg(test)]
mod nonfinite_rasterizer_probe {
    use std::panic::AssertUnwindSafe;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Vec2};
    use parley::{FontContext, PositionedLayoutItem};
    use peniko::{Color, Fill, FontData};
    use raikiri_dom::{Document, layout_single_page};
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use taffy::Style;

    /// Deliberately tiny (`"Hi"` at the CSS-initial 16px comfortably fits and
    /// inks it — see
    /// [`draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`], which
    /// pins that this canvas size is not itself the reason later tests draw
    /// nothing) — every extreme-`font_size` case is expected to leave it
    /// blank (a glyph many orders of magnitude larger than 16px does not
    /// happen to place any of its ink inside a 16x16 window at this
    /// position), which this module treats as an informative finding, not a
    /// probe defect, precisely because the control above proves the test setup
    /// can draw when the input is ordinary.
    const CANVAS_W: u32 = 16;
    const CANVAS_H: u32 = 16;

    /// Shapes a real "Hi" paragraph through the normal (guarded)
    /// `layout_single_page` pipeline and extracts the resulting `GlyphRun`'s
    /// real `FontData` + `normalized_coords` + positioned glyphs (real glyph
    /// ids with non-degenerate bounding boxes — a synthetic all-zero glyph
    /// would turn `+Inf * 0` into `NaN`, which saturates to `0` on
    /// `as i32` and would *not* reproduce the overflow this probe measures)
    /// — plus the run's own natural `font_size` (the CSS initial `16px`,
    /// unclamped since nothing overrode it here), returned so the control
    /// test below can draw at the size the glyphs were actually shaped for.
    fn real_glyph_run_parts() -> (FontData, Vec<i16>, Vec<AnyrenderGlyph>, f32) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let text_id = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let layout = doc
            .get_node(text_id)
            .expect("text node must exist (just appended it)")
            .text_layout()
            .expect("preshape_text must populate text_layout for a non-empty Text node");

        // cov:ignore: this block's `return` on match means its closing
        // braces are never "reached" as a fallthrough line once the first
        // GlyphRun is found (which it always is, for "Hi") — and the
        // assert!'s message is only executed on failure, which doesn't
        // happen while this test passes.
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let font = run.font().clone();
                    let natural_font_size = run.font_size();
                    let coords = run.normalized_coords().to_vec();
                    let glyphs: Vec<AnyrenderGlyph> = glyph_run
                        .positioned_glyphs()
                        .map(|g| AnyrenderGlyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        })
                        .collect();
                    assert!(
                        !glyphs.is_empty(),
                        "sanity: hello-world 'Hi' must shape to >= 1 glyph"
                    );
                    return (font, coords, glyphs, natural_font_size);
                }
            }
        }
        // cov:ignore: this panic is a setup-sanity fallback for a code path
        // ("Hi" produces no GlyphRun) that doesn't occur while the test
        // suite's font/shaping setup is intact.
        panic!(
            "sanity: hello-world 'Hi' produced no GlyphRun — setup is broken, not a rasterizer finding"
        );
    }

    /// Runs `draw_glyphs` against a fresh `VelloCpuImageRenderer` canvas and
    /// reports both the raw buffer length and how many bytes in it are
    /// non-zero (i.e. whether anything was actually drawn). Shared by the
    /// control test and the bounded extreme-value probe below so both use
    /// the exact same canvas geometry / transform — the only variable
    /// between them is `font_size`.
    fn render_glyphs_unbounded(
        font: &FontData,
        coords: &[i16],
        glyphs: &[AnyrenderGlyph],
        font_size: f32,
    ) -> (usize, usize) {
        let buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                scene.draw_glyphs(
                    font,
                    font_size,
                    true, // hint
                    coords,
                    Vec2::ZERO, // embolden
                    Fill::NonZero,
                    Color::BLACK,
                    1.0,              // brush_alpha
                    Affine::IDENTITY, // == Affine::translate((0.0, 0.0)); parley's
                    // `positioned_glyphs()` already bakes line offset + baseline into
                    // `g.x`/`g.y` relative to a top-left (0,0) origin (see
                    // `crates/raikiri-paint/src/text.rs` module doc), same convention
                    // `draw_text_node` uses for its own `base_transform`.
                    None, // glyph_transform
                    glyphs.iter().copied(),
                );
            },
            CANVAS_W,
            CANVAS_H,
        );
        let nonzero = buf.iter().filter(|&&b| b != 0).count();
        (buf.len(), nonzero)
    }

    /// Non-vacuity control: **without any extreme input**, does this exact
    /// test setup (font, glyphs, `CANVAS_W`×`CANVAS_H` canvas, `Affine::IDENTITY`
    /// transform) actually put visible ink on the canvas? Drawn at the run's
    /// own natural (unmodified, un-overridden) font-size — i.e. this is the
    /// one call in this module that does not touch `font_size` at all.
    ///
    /// This exists because a probe that draws nothing at *every* input
    /// (including a normal one) cannot distinguish "the rasterizer is robust
    /// to extreme scale" from "this call configuration never reaches the
    /// rasterization code path in the first place" — the three extreme-value
    /// tests below all report zero non-zero bytes, and without this control
    /// that would be uninformative rather than a finding. With it: the exact
    /// same font/glyphs/canvas/transform genuinely draws (this test), and
    /// then genuinely stops drawing once `font_size` crosses into extreme
    /// territory (the other tests) — isolating `font_size` as the one
    /// variable that changed.
    #[test]
    fn draw_glyphs_at_natural_font_size_is_a_non_vacuous_control() {
        let (font, coords, glyphs, natural_font_size) = real_glyph_run_parts();
        let (buf_len, nonzero) =
            render_glyphs_unbounded(&font, &coords, &glyphs, natural_font_size);
        assert_eq!(buf_len, (CANVAS_W * CANVAS_H * 4) as usize);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            nonzero > 0,
            "control: draw_glyphs(font_size={natural_font_size}) — the run's own natural size, no override — drew zero non-zero bytes into a {CANVAS_W}x{CANVAS_H} canvas at Affine::IDENTITY. If this control ever fails, the extreme-value tests below are vacuous (they'd pass whether or not the rasterizer is actually robust) and this module's conclusion is unsupported until this is fixed — do not just delete the assertion"
        );
    }

    /// Outcome of a bounded `draw_glyphs` probe call.
    enum ProbeOutcome {
        /// Rasterization returned a buffer (whether or not the glyph was
        /// actually drawn into it — `buf_len` is the raw byte count, always
        /// `CANVAS_W * CANVAS_H * 4` regardless of whether the atlas rejected
        /// the glyph, since `render_to_buffer` always allocates the *canvas*
        /// up front; `nonzero_bytes` is how many of those bytes are non-zero,
        /// i.e. whether anything was actually drawn — see
        /// [`draw_glyphs_at_natural_font_size_is_a_non_vacuous_control`] for
        /// why this field matters, not just `buf_len`).
        Completed {
            buf_len: usize,
            nonzero_bytes: usize,
        },
        /// The worker thread panicked (`catch_unwind` caught it directly, or
        /// the channel disconnected because the panic unwound past the
        /// `tx.send` — both are reported the same way since either means
        /// "did not complete normally", the distinction the caller cares
        /// about is panic vs. hang, not which detection path fired).
        Panicked,
    }

    /// worker thread + `recv_timeout` — the same bounding pattern
    /// `crates/raikiri-dom/src/layout.rs`'s
    /// `nonfinite_font_size_is_clamped_before_parley` uses for parley.
    /// Necessary here for the same reason: if the
    /// rasterizer *did* hang, an un-bounded `cargo test` run would eat the
    /// CI job timeout and take the rest of the test binary's results with it
    /// (indistinguishable from infra flake). A caught panic is fine to run
    /// un-bounded (it returns immediately) but is routed through the same
    /// worker so one probe function handles both failure shapes.
    fn draw_glyphs_bounded(font_size: f32) -> ProbeOutcome {
        let (font, coords, glyphs, _natural_font_size) = real_glyph_run_parts();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                render_glyphs_unbounded(&font, &coords, &glyphs, font_size)
            }));
            // Send unconditionally when we get here at all — if `catch_unwind`
            // itself failed to catch (shouldn't happen; the closure has no
            // `extern "C"` boundary or `panic = "abort"` in this workspace's
            // profile) the thread would already be gone and `tx` dropped,
            // which the `Disconnected` arm below handles.
            let _ = tx.send(result);
        });
        // cov:ignore: every call site of this helper completes well within
        // the 30s bound (that's the finding this module characterizes) —
        // the Timeout panic arm is a diagnostic for a hang this module found
        // does not occur, and Disconnected mirrors the same never-taken
        // defensive shape as the layout.rs/lib.rs siblings of this pattern.
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok((buf_len, nonzero_bytes))) => ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            },
            Ok(Err(_panic_payload)) => ProbeOutcome::Panicked,
            Err(RecvTimeoutError::Timeout) => panic!(
                "draw_glyphs(font_size={font_size}) did not return within 30s — possible CPU rasterizer hang; note the worker thread is leaked (not joined) on this path, same caveat as the parley bound in layout.rs"
            ),
            Err(RecvTimeoutError::Disconnected) => ProbeOutcome::Panicked,
        }
    }

    /// The headline finding: literal `+Inf` font-size reaching `draw_glyphs`
    /// completes normally — no panic, no hang, the returned buffer is
    /// exactly the caller-requested canvas size (not something proportional
    /// to the infinite glyph scale), and — unlike the vacuous first draft of
    /// this test — it draws **zero** visible ink, in contrast to
    /// `draw_glyphs_at_natural_font_size_is_a_non_vacuous_control` drawing
    /// non-zero ink through the identical canvas/transform/font/glyphs. See
    /// this module's doc comment for the full mechanism (the
    /// `max_cached_font_size` gate) and source citations.
    ///
    /// This is the module's load-bearing regression check: if a future
    /// `glifo`/`vello_cpu` upgrade changes this dispatch (e.g. raises/removes
    /// the atlas-caching font-size gate, or the uncached fill path stops
    /// being canvas-bounded), this test flips to `Panicked` or a timeout and
    /// must be re-characterized rather than silently left describing stale
    /// behavior — do not weaken this to
    /// `assert!(matches!(outcome, Panicked | Completed { .. }))`.
    #[test]
    fn draw_glyphs_with_inf_font_size_completes_within_bounded_canvas_sized_buffer() {
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(f32::INFINITY) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "draw_glyphs(f32::INFINITY) must allocate exactly the caller-sized canvas buffer, not something proportional to glyph scale"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(f32::INFINITY) drew {nonzero_bytes} non-zero bytes — this module's characterization was 'completes but draws nothing'; if this now draws *something*, that's a different (not necessarily worse) finding and needs its own re-characterization, not silent acceptance"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(f32::INFINITY) panicked — this module's characterization (uncached direct-fill path is used above the 128px glifo atlas-cache threshold, and it's canvas-bounded, not glyph-scale-bounded) no longer holds; re-characterize this test rather than deleting it"
            ),
        }
    }

    /// Same probe, but with a merely-huge *finite* font-size
    /// (`i32::MAX as f32`, ~2.1e9) rather than literal `+Inf` — confirms the
    /// no-crash result isn't an IEEE-infinity special case somewhere in the
    /// stack; ordinary huge finite values take the same safe path. Still
    /// ~2100x above the production clamp (`MAX_FONT_SIZE_PX = 1e6`,
    /// `crates/raikiri-dom/src/layout.rs`), i.e. still outside what the
    /// guarded pipeline can ever produce.
    #[test]
    fn draw_glyphs_with_huge_finite_font_size_also_completes_normally() {
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(i32::MAX as f32) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "draw_glyphs(i32::MAX as f32) must allocate exactly the caller-sized canvas buffer, not something proportional to glyph scale"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(i32::MAX as f32) drew {nonzero_bytes} non-zero bytes — re-characterize this test, see module doc"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(i32::MAX as f32) panicked — re-characterize this test, see module doc"
            ),
        }
    }

    /// Sanity / headroom check at the production clamp boundary itself
    /// (`MAX_FONT_SIZE_PX = 1e6`, `crates/raikiri-dom/src/layout.rs`) — this
    /// value must also complete normally (no panic, no timeout), confirming
    /// the existing input guard's chosen bound lands
    /// comfortably inside the region this module found safe (in fact, `1e6`
    /// is already ~7800x past `glifo`'s own 128px atlas-cache-eligibility
    /// cutoff, so production text has *never* exercised that caching path
    /// for any remotely large heading/display font-size — not just for
    /// pathological input). Not a new guard — a check that the current guard's
    /// chosen bound is also safe for this previously-uncharacterized sink.
    /// Like the two tests above (and unlike the natural-size control), a
    /// production-clamp-sized glyph is *also* far too large to put visible
    /// ink on a `CANVAS_W`×`CANVAS_H` probe canvas, so this asserts
    /// `nonzero_bytes == 0` too — that's expected here, not a regression.
    #[test]
    fn draw_glyphs_at_production_font_size_clamp_completes_normally() {
        const MAX_FONT_SIZE_PX: f32 = 1e6; // mirrors raikiri-dom's private const; see doc above
        // cov:ignore: this test's whole point is that the Completed arm is
        // always taken (no panic) — the Panicked arm's message is a
        // diagnostic for the failure mode this module found does not occur.
        match draw_glyphs_bounded(MAX_FONT_SIZE_PX) {
            ProbeOutcome::Completed {
                buf_len,
                nonzero_bytes,
            } => {
                assert_eq!(
                    buf_len,
                    (CANVAS_W * CANVAS_H * 4) as usize,
                    "render_to_buffer must return exactly width*height*4 bytes"
                );
                assert_eq!(
                    nonzero_bytes, 0,
                    "draw_glyphs(MAX_FONT_SIZE_PX = 1e6) drew {nonzero_bytes} non-zero bytes into a {CANVAS_W}x{CANVAS_H} probe canvas — expected zero (the glyph is far larger than the probe canvas at this size); if this changed, it's not itself a problem but re-check the module doc's characterization still holds"
                );
            }
            ProbeOutcome::Panicked => panic!(
                "draw_glyphs(MAX_FONT_SIZE_PX = 1e6) panicked — this would mean the input guard has a residual gap against this specific sink, escalate rather than widen this test's expectation"
            ),
        }
    }
}
