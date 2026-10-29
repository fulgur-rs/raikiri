use super::*;
use raikiri_layout_geometry::Edges;

use raikiri_style::{build_rule_tree, cascade};

fn cascaded(doc: &Document) -> CascadeResult {
    let rules = build_rule_tree(doc);
    cascade(doc, &rules).unwrap()
}

#[test]
fn empty_table_cell_classification_walks_text_children() {
    let mut doc = Document::new();
    let table = doc.append_element(
        Some(doc.root_index()),
        "table",
        Some("display:table;border-collapse:separate;empty-cells:hide"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "td", Some("display:table-cell"));
    doc.append_text(cell, "visible content");
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);

    assert!(!hides_empty_table_cell(&doc, &cascade, cell));
}

#[test]
fn overflow_clip_rejects_an_unknown_document_node() {
    let doc = Document::new();
    let cascade = cascaded(&doc);
    assert!(
        overflow_clip(
            &doc,
            &cascade,
            usize::MAX,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintInsets::new(0.0, 0.0, 0.0, 0.0),
        )
        .is_none()
    );
}

#[test]
fn overflow_clip_rejects_a_node_without_cascaded_values() {
    let mut doc = Document::new();
    let node = doc.append_element(
        Some(doc.root_index()),
        "div",
        Some("display:block;overflow:hidden"),
    );
    doc.mark_in_document_flags();
    let mut cascade = cascaded(&doc);
    cascade.computed.truncate(node);
    assert!(
        overflow_clip(
            &doc,
            &cascade,
            node,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintInsets::new(0.0, 0.0, 0.0, 0.0),
        )
        .is_none()
    );
}

#[test]
fn overflow_clip_does_not_apply_to_an_ordinary_inline_box() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(doc.root_index()), "div", Some("display:block"));
    let inline = doc.append_element(Some(parent), "span", Some("display:inline;overflow:hidden"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    assert!(
        overflow_clip(
            &doc,
            &cascade,
            inline,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintInsets::new(0.0, 0.0, 0.0, 0.0),
        )
        .is_none()
    );
}

#[test]
fn overflow_clip_applies_to_an_inline_flex_item() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(doc.root_index()), "div", Some("display:flex"));
    let item = doc.append_element(Some(parent), "fieldset", Some("overflow:hidden"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    // The cascade keeps `inline`; the flex container blockifies the item.
    assert_eq!(cascade.computed[item].display, DisplayValue::Inline);
    assert!(
        overflow_clip(
            &doc,
            &cascade,
            item,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintInsets::new(0.0, 0.0, 0.0, 0.0),
        )
        .is_some()
    );
}

#[test]
fn overflow_clip_propagates_a_block_root_to_the_viewport() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(doc.root_index()),
        "html",
        Some("display:block;overflow:hidden"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    assert_eq!(cascade.computed[root].display, DisplayValue::Block);
    assert!(clips_overflow(&cascade.computed[root]));
    assert!(
        overflow_clip(
            &doc,
            &cascade,
            root,
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintInsets::new(0.0, 0.0, 0.0, 0.0),
        )
        .is_none()
    );
}

#[test]
fn a_cell_spanning_a_collapsed_row_clips_to_its_border_box() {
    let border_box = PaintRect::new(10.0, 20.0, 100.0, 50.0);
    let no_border = PaintInsets::new(0.0, 0.0, 0.0, 0.0);
    let mut rounded = PaintClip::new(border_box);
    rounded.corner_radii = Some([[8.0; 2]; 4]);
    for (css, expected, same_before) in [
        // Visible overflow clips at the border box on both axes.
        ("display:block", PaintClip::new(border_box), false),
        // A visible block axis joins a clipped inline axis.
        (
            "display:block;overflow-x:clip;overflow-y:visible",
            PaintClip::new(border_box),
            false,
        ),
        // A visible inline axis joins a clipped block axis.
        (
            "display:block;overflow-x:visible;overflow-y:clip",
            PaintClip::new(border_box),
            false,
        ),
        // An overflow-clip-margin past the border box is cut back to it.
        (
            "display:block;overflow:clip;overflow-clip-margin:5px",
            PaintClip::new(border_box),
            false,
        ),
        (
            "display:block;overflow-x:clip;overflow-y:visible;overflow-clip-margin:5px",
            PaintClip::new(border_box),
            false,
        ),
        // A clip already inside the border box keeps its edges and curves.
        (
            "display:block;overflow:clip;border-radius:8px",
            rounded,
            true,
        ),
    ] {
        let mut doc = Document::new();
        let node = doc.append_element(Some(doc.root_index()), "div", Some(css));
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        let before = overflow_clip(&doc, &cascade, node, border_box, no_border);
        assert_eq!(
            before == Some(expected),
            same_before,
            "{css}: only a clip inside the border box on both axes stays the same"
        );
        doc.nodes[node].clips_collapsed_table_span = true;
        assert_eq!(
            overflow_clip(&doc, &cascade, node, border_box, no_border),
            Some(expected),
            "{css}"
        );
    }
}

#[test]
fn negative_z_index_sorts_before_static_and_positive_after() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let a = doc.append_element(Some(body), "div", Some("position:relative;z-index:2"));
    let b = doc.append_element(Some(body), "div", None::<&str>);
    let c = doc.append_element(Some(body), "div", Some("position:relative;z-index:-1"));
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert_eq!(find_paint_root(&doc), Some(body));
    let mut children = doc.get_node(body).unwrap().children.clone();
    sort_paint_children(&mut children, cr.computed[body].display, &cr);
    assert_eq!(children, [c, b, a]);
}

#[test]
fn overflow_and_opacity_predicates() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let a = doc.append_element(Some(body), "div", Some("overflow:hidden;opacity:0.5"));
    let b = doc.append_element(Some(body), "div", None::<&str>);
    let contents = doc.append_element(Some(body), "div", Some("display:contents;opacity:0.5"));
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert!(clips_overflow(&cr.computed[a]));
    assert_eq!(opacity_layer(&cr.computed[a]), Some(0.5));
    assert!(!clips_overflow(&cr.computed[b]));
    assert_eq!(opacity_layer(&cr.computed[b]), None);
    // No box, so nothing for the opacity to apply to.
    assert_eq!(opacity_layer(&cr.computed[contents]), None);
    assert!(!is_visibility_hidden_table(&cr.computed[a]));
}

#[test]
fn a_contents_document_element_paints_the_box_of_a_block() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(doc.root_index()),
        "html",
        Some("display:contents;opacity:0.5"),
    );
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let contents = doc.append_element(Some(body), "div", Some("display:contents;opacity:0.5"));
    let hidden = doc.append_element(Some(body), "div", Some("display:none;opacity:0.5"));
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    let cv = |id: usize| &cr.computed[id];
    // The document element has no parent to take its contents.
    assert_eq!(painted_display(&doc, html, cv(html)), DisplayValue::Block);
    assert!(element_generates_box(&doc, html, cv(html)));
    assert_eq!(element_opacity_layer(&doc, html, cv(html)), Some(0.5));
    // Any other contents element keeps its display and has no box.
    assert_eq!(
        painted_display(&doc, contents, cv(contents)),
        DisplayValue::Contents
    );
    assert!(!element_generates_box(&doc, contents, cv(contents)));
    assert_eq!(element_opacity_layer(&doc, contents, cv(contents)), None);
    assert_eq!(
        painted_display(&doc, hidden, cv(hidden)),
        DisplayValue::None
    );
    assert!(!element_generates_box(&doc, hidden, cv(hidden)));
}

#[test]
fn internal_table_tracks_do_not_clip_but_containers_do() {
    for (display, clips) in [
        ("table-row", false),
        ("table-row-group", false),
        ("table-header-group", false),
        ("table-footer-group", false),
        ("table-column", false),
        ("table-column-group", false),
        ("block", true),
        ("table-cell", true),
        ("table-caption", true),
        ("flex", true),
        ("grid", true),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let node = doc.append_element(
            Some(body),
            "div",
            Some(&format!("display:{display};overflow:hidden")),
        );
        doc.mark_in_document_flags();
        let cr = cascaded(&doc);
        assert_eq!(
            clips_overflow(&cr.computed[node]),
            clips,
            "display={display}"
        );
    }
}

#[test]
fn named_page_matching_skips_check_when_no_active_page() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert!(named_page_matches(&doc, &cr, body, None));
    assert!(named_page_matches(&doc, &cr, body, Some(None)));
}

#[test]
fn flex_static_boxes_use_the_positioned_paint_bucket() {
    let mut document = Document::new();
    let flex = document.append_element(Some(document.root_index()), "div", Some("display:flex"));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    assert_eq!(paint_order_key(&cascade, flex), (2, 0));
}

#[test]
fn flex_grid_paint_order_uses_order_with_stable_source_order_ties() {
    fn parent_with_ordered_children(document: &mut Document, display: &str) -> (usize, [usize; 3]) {
        let parent = document.append_element(Some(document.root_index()), "div", Some(display));
        let a = document.append_element(Some(parent), "div", Some("display:block;order:2"));
        let b = document.append_element(Some(parent), "div", Some("display:block;order:-1"));
        let c = document.append_element(Some(parent), "div", Some("display:block;order:2"));
        (parent, [a, b, c])
    }

    let mut document = Document::new();
    let (flex, flex_items) = parent_with_ordered_children(&mut document, "display:flex");
    let absolute_first = document.append_element(
        Some(flex),
        "div",
        Some("display:block;order:10;position:absolute"),
    );
    let absolute_second = document.append_element(
        Some(flex),
        "div",
        Some("display:block;order:5;position:absolute"),
    );
    let (grid, grid_items) = parent_with_ordered_children(&mut document, "display:grid");
    let (block, block_items) = parent_with_ordered_children(&mut document, "display:block");

    // Flex/grid items with different inner display values still share one
    // order-modified paint sequence.
    let mixed_parent =
        document.append_element(Some(document.root_index()), "div", Some("display:grid"));
    let nested_flex_item =
        document.append_element(Some(mixed_parent), "div", Some("display:flex;order:-1"));
    let block_item =
        document.append_element(Some(mixed_parent), "div", Some("display:block;order:1"));

    let z_index_parent =
        document.append_element(Some(document.root_index()), "div", Some("display:flex"));
    let z_one_late = document.append_element(
        Some(z_index_parent),
        "div",
        Some("display:block;z-index:1;order:1"),
    );
    let z_two_early = document.append_element(
        Some(z_index_parent),
        "div",
        Some("display:block;z-index:2;order:-1"),
    );
    let z_one_early = document.append_element(
        Some(z_index_parent),
        "div",
        Some("display:block;z-index:1;order:0"),
    );
    let positioned_z_two = document.append_element(
        Some(z_index_parent),
        "div",
        Some("display:block;position:relative;z-index:2;order:-2"),
    );

    // Out-of-flow flex children are treated as order 0 for painting. Their
    // authored order is ignored, with DOM order breaking the 0-value tie.
    let positioned_parent =
        document.append_element(Some(document.root_index()), "div", Some("display:flex"));
    let absolute_slot = document.append_element(
        Some(positioned_parent),
        "div",
        Some("display:block;position:absolute;order:100"),
    );
    let relative_late = document.append_element(
        Some(positioned_parent),
        "div",
        Some("display:block;position:relative;order:1"),
    );
    let relative_early = document.append_element(
        Some(positioned_parent),
        "div",
        Some("display:block;position:relative;order:-1"),
    );

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");

    let mut flex_children = document.get_node(flex).unwrap().children.clone();
    sort_paint_children(&mut flex_children, cascade.computed[flex].display, &cascade);
    assert_eq!(
        flex_children,
        [
            flex_items[1],
            flex_items[0],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );
    assert_eq!(
        document.get_node(flex).unwrap().children.as_slice(),
        &[
            flex_items[0],
            flex_items[1],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );

    let mut grid_children = document.get_node(grid).unwrap().children.clone();
    sort_paint_children(&mut grid_children, cascade.computed[grid].display, &cascade);
    assert_eq!(grid_children, [grid_items[1], grid_items[0], grid_items[2]]);
    assert_eq!(
        document.get_node(grid).unwrap().children.as_slice(),
        &grid_items
    );

    let mut mixed_children = document.get_node(mixed_parent).unwrap().children.clone();
    sort_paint_children(
        &mut mixed_children,
        cascade.computed[mixed_parent].display,
        &cascade,
    );
    assert_eq!(mixed_children, [nested_flex_item, block_item]);

    let mut z_index_children = document.get_node(z_index_parent).unwrap().children.clone();
    sort_paint_children(
        &mut z_index_children,
        cascade.computed[z_index_parent].display,
        &cascade,
    );
    assert_eq!(
        z_index_children,
        [z_one_early, z_one_late, positioned_z_two, z_two_early]
    );

    let mut positioned_children = document
        .get_node(positioned_parent)
        .unwrap()
        .children
        .clone();
    sort_paint_children(
        &mut positioned_children,
        cascade.computed[positioned_parent].display,
        &cascade,
    );
    assert_eq!(
        positioned_children,
        [relative_early, absolute_slot, relative_late]
    );

    let mut block_children = document.get_node(block).unwrap().children.clone();
    sort_paint_children(
        &mut block_children,
        cascade.computed[block].display,
        &cascade,
    );
    assert_eq!(block_children, block_items);
}

#[test]
fn inline_deferral_rejects_a_node_without_cascaded_values() {
    let doc = Document::new();
    let cascade = cascaded(&doc);
    assert!(!defers_inline_content(&doc, &cascade, usize::MAX));
    assert!(!closes_deferred_inline_content(&doc, &cascade, usize::MAX));
    assert!(!merges_clipped_layer(&doc, &cascade, usize::MAX));
}

#[test]
fn only_in_flow_typographic_pseudo_elements_form_opacity_groups() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let style = doc.append_element(Some(html), "style", Some("display:none"));
    doc.append_text(
        style,
        "p::before{content:'a';opacity:.5} p::after{content:'b';opacity:.5;float:left} \
         p::marker{opacity:.5}",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let p = doc.append_element(Some(body), "p", Some("display:list-item"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let id = |pseudo| crate::generated_content::generated_node_id(p, pseudo);
    assert_eq!(
        pseudo_opacity_layer(&cascade, id(PseudoElem::Before)),
        Some(0.5)
    );
    assert_eq!(pseudo_opacity_layer(&cascade, id(PseudoElem::After)), None);
    assert_eq!(pseudo_opacity_layer(&cascade, id(PseudoElem::Marker)), None);
    assert_eq!(pseudo_opacity_layer(&cascade, p), None);
}

#[test]
fn generated_boxes_below_a_paragraph_hold_a_negative_stack_level() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let style = doc.append_element(Some(html), "style", Some("display:none"));
    doc.append_text(
        style,
        "p::before{content:'a';position:relative;z-index:-1} p+p::before{z-index:1}",
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let paragraph = |doc: &mut Document| {
        let p = doc.append_element(Some(body), "p", Some("display:block"));
        doc.append_text(p, "text");
        p
    };
    let low = paragraph(&mut doc);
    let high = paragraph(&mut doc);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    assert!(holds_negative_stack_level(&doc, &cascade, low));
    assert!(!holds_negative_stack_level(&doc, &cascade, high));
}

#[test]
fn atomic_inlines_hoist_positioned_content_unless_an_inline_forms_an_opacity_group() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let div = doc.append_element(Some(body), "div", Some("display:block"));
    let plain = doc.append_element(Some(div), "span", Some("display:inline"));
    let open = doc.append_element(Some(plain), "span", Some("display:inline-block"));
    let leveled = doc.append_element(
        Some(plain),
        "span",
        Some("display:inline;position:relative;z-index:1"),
    );
    let passed = doc.append_element(Some(leveled), "span", Some("display:inline-block"));
    let faded = doc.append_element(Some(plain), "span", Some("display:inline;opacity:0.5"));
    let kept = doc.append_element(Some(faded), "span", Some("display:inline-block"));
    let ruby = doc.append_element(Some(plain), "ruby", Some("display:ruby;opacity:0.5"));
    let in_ruby = doc.append_element(Some(ruby), "span", Some("display:inline-block"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    assert!(hoists_positioned_content(&doc, &cascade, open));
    // Ruby boxes are inline boxes too and form the same groups.
    assert!(!hoists_positioned_content(&doc, &cascade, in_ruby));
    // A positioned inline with a z-index forms no group in the painters, so
    // the boxes inside it still join the enclosing layer.
    assert!(hoists_positioned_content(&doc, &cascade, passed));
    assert!(!hoists_positioned_content(&doc, &cascade, kept));
}

fn clip_of(style: &str, border: f32, padding: f32) -> Option<PaintClip> {
    let mut doc = Document::new();
    let node = doc.append_element(Some(doc.root_index()), "div", Some(style));
    doc.mark_in_document_flags();
    doc.nodes[node].layout.padding = Edges {
        left: padding,
        right: padding,
        top: padding,
        bottom: padding,
    };
    let cascade = cascaded(&doc);
    overflow_clip(
        &doc,
        &cascade,
        node,
        PaintRect::new(10.0, 20.0, 100.0, 50.0),
        PaintInsets::new(border, border, border, border),
    )
}

#[test]
fn overflow_clip_margin_moves_the_clip_edge_of_a_clipping_axis() {
    // content-box 5px: the content edge (border 2 + padding 3) moved 5px out
    // is the border edge.
    let clip = clip_of(
        "display:block;overflow:clip;overflow-clip-margin:content-box 5px",
        2.0,
        3.0,
    )
    .unwrap();
    assert_eq!(clip.rect, PaintRect::new(10.0, 20.0, 100.0, 50.0));
    // border-box 4px on the block axis only; the open inline axis keeps its
    // padding edge.
    let clip = clip_of(
        "display:block;overflow-x:visible;overflow-y:clip;overflow-clip-margin:border-box 4px",
        2.0,
        3.0,
    )
    .unwrap();
    assert_eq!(clip.rect, PaintRect::new(12.0, 16.0, 96.0, 58.0));
    assert!(!clip.clip_x && clip.clip_y);
    // A scroll container keeps the padding edge.
    let clip = clip_of(
        "display:block;overflow:hidden;overflow-clip-margin:20px",
        2.0,
        3.0,
    )
    .unwrap();
    assert_eq!(clip.rect, PaintRect::new(12.0, 22.0, 96.0, 46.0));
}

#[test]
fn overflow_clip_margin_reshapes_the_corners_from_the_border_edge() {
    // Outside the border edge the radius grows by the outset, less for a
    // radius smaller than the outset.
    let clip = clip_of(
        "display:block;overflow:clip;border-radius:10px;overflow-clip-margin:border-box 20px",
        2.0,
        0.0,
    )
    .unwrap();
    let ratio: f32 = 10.0 / 20.0;
    let grown = 10.0 + 20.0 * (1.0 + (ratio - 1.0).powi(3));
    assert_eq!(clip.corner_radii, Some([[grown; 2]; 4]));
    let clip = clip_of(
        "display:block;overflow:clip;border-radius:10px;overflow-clip-margin:border-box 5px",
        2.0,
        0.0,
    )
    .unwrap();
    assert_eq!(clip.corner_radii, Some([[15.0; 2]; 4]));
    // Inside, it shrinks by the inset and vanishes past the radius.
    let clip = clip_of(
        "display:block;overflow:clip;border-radius:10px;overflow-clip-margin:content-box",
        2.0,
        3.0,
    )
    .unwrap();
    assert_eq!(clip.corner_radii, Some([[5.0; 2]; 4]));
    let clip = clip_of(
        "display:block;overflow:clip;border-radius:4px;overflow-clip-margin:content-box",
        2.0,
        3.0,
    )
    .unwrap();
    assert_eq!(clip.corner_radii, None);
}

#[test]
fn replaced_content_clip_follows_overflow_per_axis() {
    let mut doc = Document::new();
    let visible = doc.append_element(Some(doc.root_index()), "canvas", Some("overflow:visible"));
    // The UA style sheet's rule for replaced elements.
    let clipped = doc.append_element(
        Some(doc.root_index()),
        "canvas",
        Some("overflow:clip;overflow-clip-margin:content-box"),
    );
    let block_only = doc.append_element(
        Some(doc.root_index()),
        "canvas",
        Some("overflow:visible clip;overflow-clip-margin:content-box"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let border_box = PaintRect::new(0.5, 0.25, 40.0, 30.0);
    let border = PaintInsets::new(1.0, 1.0, 1.0, 1.0);
    let padding = PaintInsets::new(2.0, 2.0, 2.0, 2.0);
    assert_eq!(
        replaced_content_clip(&cascade.computed[visible], border_box, border, padding),
        None
    );
    let clip =
        replaced_content_clip(&cascade.computed[clipped], border_box, border, padding).unwrap();
    // The origin and the far edges of the clipping axes snap down.
    assert_eq!(clip.rect, PaintRect::new(3.0, 3.0, 34.0, 24.0));
    assert!(clip.clip_x && clip.clip_y);
    let clip =
        replaced_content_clip(&cascade.computed[block_only], border_box, border, padding).unwrap();
    assert!(!clip.clip_x && clip.clip_y);
}

#[test]
fn a_replaced_element_clips_every_hidden_axis_from_its_content_box() {
    let mut doc = Document::new();
    let hidden = doc.append_element(
        Some(doc.root_index()),
        "img",
        Some("display:block;overflow:hidden;overflow-clip-margin:content-box"),
    );
    let outset = doc.append_element(
        Some(doc.root_index()),
        "img",
        Some("display:block;overflow:clip visible;overflow-clip-margin:10px"),
    );
    doc.mark_in_document_flags();
    for node in [hidden, outset] {
        doc.nodes[node].layout.padding = Edges {
            left: 3.0,
            right: 3.0,
            top: 3.0,
            bottom: 3.0,
        };
    }
    let cascade = cascaded(&doc);
    let border_box = PaintRect::new(10.0, 20.0, 100.0, 50.0);
    let border = PaintInsets::new(2.0, 2.0, 2.0, 2.0);
    // `hidden` is used as `clip`, at the content box.
    let clip = overflow_clip(&doc, &cascade, hidden, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(15.0, 25.0, 90.0, 40.0));
    // An omitted box is the content box; only the clipping axis moves.
    let clip = overflow_clip(&doc, &cascade, outset, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(5.0, 22.0, 110.0, 46.0));
    assert!(clip.clip_x && !clip.clip_y);
}

#[test]
fn a_scroll_container_applies_only_an_inward_clip_margin() {
    let mut doc = Document::new();
    let inward = doc.append_element(
        Some(doc.root_index()),
        "div",
        Some("display:block;overflow:hidden;overflow-clip-margin:content-box"),
    );
    let outward = doc.append_element(
        Some(doc.root_index()),
        "div",
        Some("display:block;overflow:auto;overflow-clip-margin:10px"),
    );
    let border_box_value = doc.append_element(
        Some(doc.root_index()),
        "div",
        Some("display:block;overflow:hidden;overflow-clip-margin:border-box -10px"),
    );
    doc.mark_in_document_flags();
    for node in [inward, outward, border_box_value] {
        doc.nodes[node].layout.padding = Edges {
            left: 3.0,
            right: 3.0,
            top: 3.0,
            bottom: 3.0,
        };
    }
    let cascade = cascaded(&doc);
    let border_box = PaintRect::new(10.0, 20.0, 100.0, 50.0);
    let border = PaintInsets::new(2.0, 2.0, 2.0, 2.0);
    // The content box lies inside the padding box, so it clips there.
    let clip = overflow_clip(&doc, &cascade, inward, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(15.0, 25.0, 90.0, 40.0));
    // An outward margin stays at the padding box.
    let clip = overflow_clip(&doc, &cascade, outward, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(12.0, 22.0, 96.0, 46.0));
    // A `border-box` value is ignored, offset included.
    let clip = overflow_clip(&doc, &cascade, border_box_value, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(12.0, 22.0, 96.0, 46.0));
}

#[test]
fn an_audio_element_uses_its_hidden_overflow_as_clip() {
    let mut doc = Document::new();
    let audio = doc.append_element(
        Some(doc.root_index()),
        "audio",
        Some("display:block;overflow:hidden;overflow-clip-margin:10px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let border_box = PaintRect::new(10.0, 20.0, 100.0, 50.0);
    let border = PaintInsets::new(2.0, 2.0, 2.0, 2.0);
    // Used as `clip`, the outward margin applies.
    let clip = overflow_clip(&doc, &cascade, audio, border_box, border).unwrap();
    assert_eq!(clip.rect, PaintRect::new(2.0, 12.0, 116.0, 66.0));
}

#[test]
fn an_extreme_clip_margin_keeps_the_clip_finite() {
    let mut doc = Document::new();
    let node = doc.append_element(
        Some(doc.root_index()),
        "div",
        Some("display:block;overflow:clip;overflow-clip-margin:1e40px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let border_box = PaintRect::new(10.0, 20.0, 100.0, 50.0);
    let border = PaintInsets::new(2.0, 2.0, 2.0, 2.0);
    let clip = overflow_clip(&doc, &cascade, node, border_box, border).unwrap();
    for value in [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height] {
        assert!(value.is_finite(), "{:?}", clip.rect);
    }
}

#[test]
fn only_a_visible_box_paints_its_own_decorations() {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", None::<&str>);
    let boxes: Vec<_> = [
        ("table", "visible"),
        ("table", "hidden"),
        ("table", "collapse"),
        ("block", "hidden"),
    ]
    .into_iter()
    .map(|(display, keyword)| {
        doc.append_element(
            Some(body),
            "div",
            Some(format!("display:{display};visibility:{keyword}").as_str()),
        )
    })
    .collect();
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    let hides: Vec<_> = boxes
        .iter()
        .map(|&id| {
            (
                hides_own_box(&cr.computed[id]),
                is_visibility_hidden_table(&cr.computed[id]),
            )
        })
        .collect();
    assert_eq!(
        hides,
        [(false, false), (true, true), (true, true), (true, false)]
    );
}

#[test]
fn positioned_frames_go_back_to_their_place_in_tree_order() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let block =
        |doc: &mut Document, parent| doc.append_element(Some(parent), "div", Some("display:block"));
    let a = block(&mut doc, body);
    let b = block(&mut doc, body);
    let b1 = block(&mut doc, b);
    let b2 = block(&mut doc, b);
    let c = block(&mut doc, body);
    doc.mark_in_document_flags();

    let mut order = DocumentOrder::new(&doc);
    assert!(order.precedes(a, b1));
    assert!(order.precedes(b, b1));
    assert!(!order.precedes(b1, b));
    assert!(!order.precedes(c, a));
    assert!(!order.precedes(a, a));
    assert!(!order.precedes(a, usize::MAX));

    use PositionedFrame::{Box, Close, Open};
    let mut insert = |mut frames: Vec<PositionedFrame>, frame| {
        insert_positioned_frame(&mut order, &mut frames, frame, |frame| *frame);
        frames
    };
    // A box goes back past the boxes after it, and past a clip around
    // boxes after it as a whole.
    assert_eq!(
        insert(vec![Box(a), Box(c)], Box(b1)),
        [Box(a), Box(b1), Box(c)]
    );
    assert_eq!(
        insert(vec![Open(b), Box(b1), Close], Box(a)),
        [Box(a), Open(b), Box(b1), Close]
    );
    // It stays after a clip of its own ancestor, which it cannot reenter,
    // and after a clip still open.
    assert_eq!(
        insert(vec![Open(b), Box(b2), Close], Box(b1)),
        [Open(b), Box(b2), Close, Box(b1)]
    );
    assert_eq!(insert(vec![Open(b)], Box(a)), [Open(b), Box(a)]);
    // Clips keep the order they are added in.
    assert_eq!(insert(vec![Box(c)], Open(b)), [Box(c), Open(b)]);
}

mod first_line_membership_tests;
mod positioned_order_tests;

thread_local! {
    static FIRST_LINE_WORK: std::cell::Cell<[usize; 4]> = const { std::cell::Cell::new([0; 4]) };
}

pub(super) fn record_first_line_work(kind: usize) {
    FIRST_LINE_WORK.with(|work| {
        let mut counts = work.get();
        counts[kind] += 1;
        work.set(counts);
    });
}
