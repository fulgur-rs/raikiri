use super::*;

struct Fixture {
    doc: Document,
    body: usize,
    chain: Vec<usize>,
    displays: Vec<String>,
}

impl Fixture {
    fn new(displays: &[&str]) -> Self {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let mut parent = body;
        let mut chain = Vec::new();
        for display in displays {
            parent = doc.append_element(Some(parent), "div", Some(format!("display:{display}")));
            chain.push(parent);
        }
        doc.append_text(parent, "z");
        Self {
            doc,
            body,
            chain,
            displays: displays.iter().map(|text| (*text).to_owned()).collect(),
        }
    }

    fn style(&mut self, index: usize, css: &str) {
        self.doc
            .set_element_attribute(
                self.chain[index],
                "style",
                format!("display:{};{css}", self.displays[index]),
            )
            .unwrap();
    }

    fn qualified(mut self) -> bool {
        self.doc.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&self.doc, &raikiri_style::build_rule_tree(&self.doc)).unwrap();
        super::super::layout_single_page(
            crate::layout::test_support::with_ahem(&mut self.doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let session = crate::layout::context::LayoutSession::bridged(&mut self.doc, &cascade);
        holder(&session, &cascade, *self.chain.last().unwrap(), self.body).is_some()
    }
}

#[test]
fn sole_table_groups_and_inline_flex_qualify_without_enabling_block_fragmentation() {
    for chain in [
        vec!["table", "table-row", "table-cell", "block"],
        vec![
            "table",
            "table-row-group",
            "table-row",
            "table-cell",
            "block",
        ],
        vec![
            "table",
            "table-header-group",
            "table-row",
            "table-cell",
            "block",
        ],
        vec![
            "table",
            "table-footer-group",
            "table-row",
            "table-cell",
            "block",
        ],
        vec!["inline-flex", "block"],
    ] {
        assert!(Fixture::new(&chain).qualified(), "{chain:?}");
    }
}

#[test]
fn effective_constraints_clips_and_separate_positioning_do_not_qualify() {
    let mut unexpected = Vec::new();
    for chain in [
        &["table", "table-row", "table-cell", "block"][..],
        &["inline-flex", "block"][..],
    ] {
        for index in 0..chain.len() {
            for css in [
                "height:30px",
                "min-height:30px",
                "max-height:30px",
                "position:relative",
                "position:absolute",
                "float:left",
                "contain:size",
                "overflow:hidden",
                "writing-mode:vertical-rl",
                "column-count:2",
            ] {
                // These declarations do not establish the tested constraint:
                // table containment, table/flex multicol, row clipping, cell
                // min-height, and floats on a flex item are ineffective here.
                let display = chain[index];
                if matches!(
                    (display, css),
                    ("table" | "table-row" | "table-cell", "contain:size")
                        | ("table" | "inline-flex", "column-count:2")
                        | ("table-row", "overflow:hidden")
                        | ("table-cell", "min-height:30px")
                ) || (chain[0] == "inline-flex" && index == 1 && css == "float:left")
                {
                    continue;
                }
                let mut fixture = Fixture::new(chain);
                fixture.style(index, css);
                if fixture.qualified() {
                    unexpected.push((chain.to_vec(), index, css));
                }
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:?}");
}

#[test]
fn other_boxes_and_non_collapsible_text_reject_the_sole_item_path() {
    for chain in [
        &["table", "table-row", "table-cell", "block"][..],
        &[
            "table",
            "table-row-group",
            "table-row",
            "table-cell",
            "block",
        ][..],
        &["inline-flex", "block"][..],
    ] {
        for index in 0..chain.len() - 1 {
            let mut fixture = Fixture::new(chain);
            fixture.doc.append_element(
                Some(fixture.chain[index]),
                "div",
                Some("display:block;height:10px"),
            );
            assert!(!fixture.qualified(), "{chain:?} {index}");
        }
    }
    for text in ["other", "\u{a0}"] {
        let mut fixture = Fixture::new(&["inline-flex", "block"]);
        fixture.doc.append_text(fixture.body, text);
        assert!(!fixture.qualified(), "{text:?}");
    }
    for span in ["rowspan", "colspan"] {
        let mut fixture = Fixture::new(&["table", "table-row", "table-cell", "block"]);
        fixture
            .doc
            .set_element_attribute(fixture.chain[2], span, "2")
            .unwrap();
        assert!(!fixture.qualified(), "{span}");
    }
}

#[test]
fn nested_formatting_contexts_and_contents_text_do_not_form_an_isolated_atomic_line() {
    for parent in ["flex", "inline-block", "inline-flex"] {
        assert!(
            !Fixture::new(&[parent, "inline-flex", "block"]).qualified(),
            "{parent}"
        );
    }
    let mut fixture = Fixture::new(&["inline-flex", "block"]);
    let wrapper = fixture
        .doc
        .append_element(Some(fixture.body), "span", Some("display:contents"));
    fixture.doc.append_text(wrapper, "other");
    assert!(!fixture.qualified());
}
