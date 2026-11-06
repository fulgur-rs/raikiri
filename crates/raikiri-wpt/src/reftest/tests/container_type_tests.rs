use super::*;

#[test]
#[ignore = "requires scripts/wpt/fetch.sh; run with --ignored"]
fn container_type_size_matches_original_contain_size_014_pixels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt/css");
    let test = root.join("css-contain/contain-size-014.html");
    let reference = root.join("reference/ref-filled-green-100px-square.xht");
    let actual = render_raikiri_file_pages(&test, 800, 600).unwrap();
    let expected = render_raikiri_file_pages(&reference, 800, 600).unwrap();
    assert_eq!(actual.pages.len(), 1);
    assert_eq!(expected.pages.len(), 1);
    assert!(compare_images(&actual.pages[0], &expected.pages[0], Tolerance::EXACT).matched);
    let source = std::fs::read_to_string(&test).unwrap();
    let uncontained = source.replace("container-type: size", "container-type: normal");
    let control = render_raikiri_pages_inner(&uncontained, 800, 600, test.parent(), None).unwrap();
    assert!(!compare_images(&control.pages[0], &expected.pages[0], Tolerance::EXACT).matched);
}

fn render(body: &str) -> RenderedDocument {
    render_raikiri_pages(&format!("<!doctype html><style>@page{{size:200px 100px;margin:0}}html,body{{margin:0}}</style>{body}"), 800, 600).unwrap()
}

#[test]
fn container_type_keeps_visible_overflow_painted() {
    for kind in ["inline-size", "size"] {
        let actual = render(&format!(
            "<div style='container-type:{kind};width:20px;height:20px;background:red'><div style='width:40px;height:40px;background:green'></div></div>"
        ));
        let expected = render("<div style='width:40px;height:40px;background:green'></div>");
        assert_eq!(actual.pages.len(), 1);
        assert!(actual.pages[0].rgba == expected.pages[0].rgba, "{kind}");
        let pixel = (30 * 200 + 30) * 4;
        assert_eq!(&actual.pages[0].rgba[pixel..pixel + 4], &[0, 128, 0, 255]);
    }
}

#[test]
fn container_type_does_not_add_a_stacking_context() {
    let body = "<div style='position:relative;width:40px;height:40px'><div style='CONTAINER;width:40px;height:40px'><div style='position:relative;z-index:1;width:40px;height:40px;background:green'></div></div><div style='position:absolute;left:0;top:0;width:40px;height:40px;background:red'></div></div>";
    let expected = render(&body.replace("CONTAINER", "container-type:normal"));
    for kind in ["inline-size", "size"] {
        let actual = render(&body.replace("CONTAINER", &format!("container-type:{kind}")));
        assert!(actual.pages[0].rgba == expected.pages[0].rgba, "{kind}");
        assert_eq!(&actual.pages[0].rgba[..4], &[0, 128, 0, 255]);
    }
}

#[test]
fn container_type_inline_size_fragments_like_an_independent_flow_root() {
    let body = "<div style='CONTAINER;width:100px'><div style='height:60px;background:green'></div><div style='break-before:page;height:60px;background:blue'></div></div>";
    let actual = render(&body.replace("CONTAINER", "container-type:inline-size"));
    let expected = render(&body.replace("CONTAINER", "display:flow-root"));
    assert_eq!(actual.pages.len(), 2);
    assert_eq!(expected.pages.len(), 2);
    for index in 0..2 {
        assert!(
            actual.pages[index].rgba == expected.pages[index].rgba,
            "page {index}"
        );
    }
}
