use super::*;

fn block(document: &mut Document, parent: usize, style: &str) -> usize {
    document.append_element(Some(parent), "div", Some(style))
}

#[test]
fn height_dependent_positioned_subtrees_are_recomputed_on_repeated_layouts() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let containing = block(
        &mut document,
        body,
        "display:block;position:relative;height:300px",
    );
    let owner = block(
        &mut document,
        containing,
        "display:block;position:absolute;top:75px;left:0;width:80px;font-family:Ahem;font-size:20px;line-height:30px",
    );
    for text in ["A", "B"] {
        let paragraph = block(&mut document, owner, "display:block;margin:0");
        document.append_text(paragraph, text);
    }
    let child = block(
        &mut document,
        owner,
        "display:block;position:absolute;bottom:0;left:60px;width:10px;height:50%",
    );
    document.mark_in_document_flags();
    with_ahem(&mut document);
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    for pass in 0..3 {
        layout_pages(&mut document, &cascade, page).unwrap();
        assert_eq!(
            document.nodes[child].layout.size.height, 42.5,
            "pass {pass}"
        );
        assert_eq!(
            document.layout_origin_of(child).unwrap().y,
            117.5,
            "pass {pass}"
        );
    }
}

#[test]
fn only_generated_fixed_boxes_suppress_absolute_flow_pagination() {
    for (wrapper_style, height) in [
        ("display:contents;position:fixed", 85.0),
        ("display:contents;position:absolute", 85.0),
        ("display:block;position:fixed", 60.0),
    ] {
        let mut document = Document::new();
        let html = document.append_element(Some(0), "html", Some("display:block"));
        let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
        let containing = block(
            &mut document,
            body,
            "display:block;position:relative;height:300px",
        );
        let wrapper = block(&mut document, containing, wrapper_style);
        let owner = block(
            &mut document,
            wrapper,
            "display:block;position:absolute;top:75px;left:0;width:80px;font-family:Ahem;font-size:20px;line-height:30px",
        );
        for text in ["A", "B"] {
            let paragraph = block(&mut document, owner, "display:block;margin:0");
            document.append_text(paragraph, text);
        }
        document.mark_in_document_flags();
        with_ahem(&mut document);
        let rules = raikiri_style::build_rule_tree(&document);
        let cascade = raikiri_style::cascade(&document, &rules).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_pages(&mut document, &cascade, page).unwrap();
        assert_eq!(
            document.nodes[owner].layout.size.height, height,
            "{wrapper_style}"
        );
        assert_eq!(
            is_unfixed_absolute_box(&document, owner),
            height == 85.0,
            "{wrapper_style}"
        );
        assert!(
            !is_unfixed_absolute_box(&document, wrapper),
            "{wrapper_style}"
        );
    }
}

#[test]
fn replayed_clip_bounds_start_at_the_candidate_flow_root() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let clipping = block(
        &mut document,
        body,
        "display:block;position:relative;height:10px;overflow:hidden",
    );
    let flow = block(
        &mut document,
        clipping,
        "display:block;position:relative;height:300px",
    );
    let owner = block(
        &mut document,
        flow,
        "display:block;position:absolute;top:75px;height:60px;width:80px",
    );
    document.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    layout_single_page(&mut document, &cascade, page_box_800x600()).unwrap();
    let origins = PageOrigins::new(&[], 100.0);
    let control = PageLayoutControl::new(None);
    let lines = TextLineIndex::default();
    let avoided = AvoidedBreaks::default();
    let roots = [
        candidates::FlowRoots::new(&document, flow),
        candidates::FlowRoots::new(&document, body),
    ];
    let session = crate::layout::context::LayoutSession::bridged(&mut document, &cascade);
    for (roots, expected) in roots.into_iter().zip([f32::INFINITY, 76.0]) {
        let env = FlowEnv {
            cascade: &cascade,
            body_id: body,
            control: &control,
            page_origins: &origins,
            text_lines: &lines,
            roots,
            root_flow_offset: 0.0,
            content_width: 800.0,
            avoided_breaks: &avoided,
            insets: PageContentInsets::default(),
            page_box: page_box_800x600(),
            clip_edges: &std::collections::HashMap::new(),
        };
        assert_eq!(
            super::super::positioned_flow::visible_end(&session, &env, owner),
            expected
        );
    }
}
