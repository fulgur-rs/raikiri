use super::*;
use std::cell::Cell;
thread_local! { static WORK: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }
pub(in crate::page_scene) fn count_work(nodes: usize, lines: usize) {
    WORK.with(|work| {
        let (n, l) = work.get();
        work.set((n + nodes, l + lines));
    });
}
thread_local! { static OLD_WORK: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }
pub(super) fn count_old_work(nodes: usize, lines: usize) {
    OLD_WORK.with(|work| {
        let (n, l) = work.get();
        work.set((n + nodes, l + lines));
    });
}
fn pages(p: usize) -> (Document, CascadeResult, PageBox) {
    let html = format!(
        "<style>@page{{margin:20px}}html,body{{margin:0;padding:0}}section{{height:440px}}p{{margin:0}}</style>{}",
        "<section><p>x</p></section>".repeat(p)
    );
    let uncascaded = parse(html.as_bytes(), &ParseOptions::new()).unwrap();
    let cascade = build_cascaded(&uncascaded).unwrap();
    let mut dom = uncascaded.dom;
    let mut page = PageBox::new();
    page.width = 640.0;
    page.height = 480.0;
    raikiri_dom::layout_single_page(&mut dom, &cascade, page).unwrap();
    (dom, cascade, page)
}
#[test]
fn all_page_node_and_line_work_grows_with_page_output() {
    for p in [100, 200, 400, 800] {
        let (dom, cascade, page) = pages(p);
        WORK.with(|w| w.set((0, 0)));
        OLD_WORK.with(|w| w.set((0, 0)));
        for i in 0..p {
            let scene = build_page_scene_for_page(&dom, &cascade, page, i as u32, i as f32 * 440.0);
            assert_eq!(scene.drawables.paragraphs.len(), 1);
            let old = super::old_walk_tests::build_page_scene_for_page_named(
                &dom,
                &cascade,
                page,
                i as u32,
                i as f32 * 440.0,
                None,
            );
            assert_eq!(format!("{scene:?}"), format!("{old:?}"));
        }
        let (nodes, lines) = WORK.with(Cell::get);
        let (old_nodes, old_lines) = OLD_WORK.with(Cell::get);
        eprintln!("old-scene-work pages={p} nodes={old_nodes} lines={old_lines}");
        assert_eq!(old_lines, p * p);
        assert!(old_nodes >= 3 * p * p);
        eprintln!(
            "scene-work pages={p} dom={} nodes={nodes} lines={lines}",
            dom.node_count()
        );
        assert!(nodes <= 32 * p, "node work {nodes} for {p} pages");
        assert!(lines <= 4 * p, "line work {lines} for {p} pages");
    }
}

fn assert_oracle(dom: &Document, cascade: &CascadeResult, page: PageBox, origin: f32) {
    let actual =
        build_page_scene_for_page_named(dom, cascade, page, 7, origin, Some("chapter".into()));
    let expected = super::old_walk_tests::build_page_scene_for_page_named(
        dom,
        cascade,
        page,
        7,
        origin,
        Some("chapter".into()),
    );
    // All scene fields, drawable maps and their insertion histories are ordered.
    assert_eq!(
        format!("{actual:?}"),
        format!("{expected:?}"),
        "origin {origin:?}"
    );
}

#[test]
fn indexed_scenes_match_the_old_walk_for_layout_and_page_variants() {
    for (extra, body) in [
        (
            "",
            "<div id=span style='height:350px;background:red'>a</div><p>b</p>",
        ),
        (
            "",
            "<div style='height:1px;overflow:visible'><p style='position:relative;top:210px'>overflow</p><div style='position:absolute;top:310px'>absolute</div></div><p style='position:fixed;top:10px'>fixed</p>",
        ),
        (
            "",
            "<div style='transform:translate(0px,120px)'><p style='position:fixed'>contained</p></div><div style='display:contents'><p style='position:fixed'>viewport</p></div>",
        ),
        (
            "",
            "<p style='width:80px'><span id=wrap>one two three four five six seven eight nine ten</span></p>",
        ),
        (
            "",
            "<div style='columns:2;width:180px;height:100px'><p>one two three four five six seven eight nine ten</p></div>",
        ),
        (
            "html{writing-mode:vertical-lr}",
            "<div style='width:250px'>a<span>b</span></div><p>c</p>",
        ),
        (
            "html{writing-mode:vertical-rl}",
            "<div style='width:250px'>a<span>b</span></div><p>c</p>",
        ),
        (
            "html{margin:7px;padding:5px;background:red}body{margin:9px;padding:3px;background:blue}",
            "<div><p>roots</p></div>",
        ),
        (
            "",
            "<table><caption>caption</caption><tr><td>cell</td></tr></table><script>inert</script>",
        ),
    ] {
        let html = format!(
            "<style>@page{{margin:20px;@top-left{{content:'header'}}}}html,body{{margin:0;padding:0}}body{{font:10px Ahem}}p{{margin:0}}{extra}</style>{body}"
        );
        let uncascaded = parse(html.as_bytes(), &ParseOptions::new()).unwrap();
        let cascade = build_cascaded(&uncascaded).unwrap();
        let mut dom = uncascaded.dom;
        dom.set_font_collection(
            raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR)).unwrap(),
        );
        let mut page = PageBox::new();
        page.width = 240.0;
        page.height = 140.0;
        raikiri_dom::layout_single_page(&mut dom, &cascade, page).unwrap();
        for origin in [
            200.0,
            0.0,
            100.0,
            200.0,
            -0.0,
            -100.0,
            100.0_f32.next_up(),
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
        ] {
            assert_oracle(&dom, &cascade, page, origin);
        }
        page.width = 180.0;
        page.height = 90.0;
        assert_oracle(&dom, &cascade, page, 40.0);
    }
}

#[test]
fn warm_geometry_observes_current_computed_values_and_page_cascade() {
    use raikiri_style::property::{DisplayValue, PositionValue};
    let html = "<style>@page{margin:10px;@top-left{content:'one'}}html,body{margin:0}#parent{height:600px}#child{position:fixed;top:15px}#donor{transform:translate(1px,2px)}</style><div id=parent><p id=child>child</p></div><div id=donor></div>";
    let uncascaded = parse(html.as_bytes(), &ParseOptions::new()).unwrap();
    let mut cascade = build_cascaded(&uncascaded).unwrap();
    let mut dom = uncascaded.dom;
    let mut page = PageBox::new();
    page.width = 240.0;
    page.height = 140.0;
    raikiri_dom::layout_single_page(&mut dom, &cascade, page).unwrap();
    let id = |name| {
        (0..dom.node_count())
            .find(|&i| dom.element_attribute(i, "id") == Some(name))
            .unwrap()
    };
    let parent = id("parent");
    let child = id("child");
    let donor = id("donor");
    let root = dom.document_element_index().unwrap();
    assert_oracle(&dom, &cascade, page, 200.0);
    let generation = cascade.element_generation();
    cascade.computed[child].position = PositionValue::Static;
    assert_oracle(&dom, &cascade, page, 200.0);
    cascade.computed[child].position = PositionValue::Fixed;
    cascade.computed[parent].transform = cascade.computed[donor].transform.clone();
    assert_oracle(&dom, &cascade, page, 200.0);
    cascade.computed[parent].display = DisplayValue::Contents;
    assert_oracle(&dom, &cascade, page, 200.0);
    cascade.computed[parent].display = DisplayValue::None;
    assert_oracle(&dom, &cascade, page, 200.0);
    cascade.computed[root].display = DisplayValue::None;
    assert_oracle(&dom, &cascade, page, 0.0);
    cascade.computed[root].display = DisplayValue::Block;
    assert_oracle(&dom, &cascade, page, 0.0);
    assert_eq!(cascade.element_generation(), generation);
    let other = parse(
        &b"<style>@page{margin:30px;@bottom-right{content:'two'}}</style><p>x</p>"[..],
        &ParseOptions::new(),
    )
    .unwrap();
    let other = build_cascaded(&other).unwrap();
    cascade.replace_page(other.page.clone());
    assert_oracle(&dom, &cascade, page, 100.0);
}

#[test]
fn mutation_and_relayout_rebuild_warm_geometry_without_changing_a_clone() {
    let (mut dom, cascade, mut page) = pages(3);
    assert_oracle(&dom, &cascade, page, 440.0);
    let clone = dom.clone();
    let body = raikiri_dom::paint_rules::find_paint_root(&dom).unwrap();
    dom.set_element_attribute(body, "id", "changed").unwrap();
    let section = (0..dom.node_count())
        .find(|&i| dom.get_node(i).and_then(|n| n.tag_name()) == Some("section"))
        .unwrap();
    dom.detach_from_parent(section);
    assert_oracle(&dom, &cascade, page, 440.0);
    // Relayout invalidates geometry even when source nodes are unchanged.
    let mut input = parse(&b"<p>replacement</p>"[..], &ParseOptions::new()).unwrap();
    let other = build_cascaded(&input).unwrap();
    page.height = 240.0;
    raikiri_dom::layout_single_page(&mut input.dom, &other, page).unwrap();
    assert_oracle(&input.dom, &other, page, 0.0);
    let mut original_page = page;
    original_page.height = 480.0;
    assert_oracle(&clone, &cascade, original_page, 440.0);
    raikiri_dom::layout_single_page(&mut input.dom, &other, original_page).unwrap();
    assert_oracle(&input.dom, &other, page, 0.0);
}

#[test]
fn warm_geometry_is_replaced_when_text_reflows_at_a_new_width() {
    let html = "<style>body{font:10px Ahem}p{margin:0}</style><p>one two three four five six seven eight nine ten</p>";
    let (mut dom, cascade, _) = scene_with_engine(html);
    let before = build_page_scene(&dom, &cascade, PageBox::A4);
    let before_lines: usize = before
        .drawables
        .paragraphs
        .values()
        .map(|p| p.line_count)
        .sum();
    raikiri_dom::relayout_text_for_width(&mut dom, &cascade, 40.0);
    assert_oracle(&dom, &cascade, PageBox::A4, 0.0);
    let after = build_page_scene(&dom, &cascade, PageBox::A4);
    let after_lines: usize = after
        .drawables
        .paragraphs
        .values()
        .map(|p| p.line_count)
        .sum();
    assert!(
        after_lines > before_lines,
        "reflow must change cached line counts"
    );
    let mut narrow = PageBox::A4;
    narrow.width = 100.0;
    raikiri_dom::layout_single_page(&mut dom, &cascade, narrow).unwrap();
    assert_oracle(&dom, &cascade, narrow, 0.0);
}
