use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{AvailableSpace, LayoutInput, NodeId, RunMode};
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Size;

// Chromium 151.0.7922.34 DOM rectangles, before any page cuts. These catch
// cross-size clamping or loss of negative alignment in our layout boundary.
#[test]
fn row_items_keep_their_cross_size_when_aligned_above_the_container() {
    for writing_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for (alignment, tall_y, short_y) in [
            ("flex-start", 0.0, 0.0),
            ("flex-end", -90.0, 30.0),
            ("center", -45.0, 15.0),
            ("stretch", 0.0, 0.0),
        ] {
            for with_sibling in [false, true] {
                let mut doc = Document::new();
                let parent = doc.append_element(
                    Some(0),
                    "div",
                    Some(format!(
                        "display:flex;width:800px;height:80px;align-items:{alignment};font:10px/10px Ahem"
                    )),
                );
                let sibling = with_sibling.then(|| {
                    let node =
                        doc.append_element(Some(parent), "div", Some("width:10px;height:50px"));
                    doc.append_text(node, "x");
                    node
                });
                let item = doc.append_element(
                    Some(parent),
                    "div",
                    Some(format!(
                        "width:10px;height:170px;writing-mode:{writing_mode}"
                    )),
                );
                doc.append_text(item, "x");
                doc.mark_in_document_flags();
                doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());
                let cascade = cascaded(&doc);
                let mut tree = LayoutSession::bridged(&mut doc, &cascade);
                tree.compute_child_layout(
                    NodeId::from(parent),
                    LayoutInput {
                        run_mode: RunMode::PerformLayout,
                        known_dimensions: Size {
                            width: Some(800.0),
                            height: Some(80.0),
                        },
                        known_dimensions_are_definite: Size {
                            width: true,
                            height: true,
                        },
                        parent_size: Size {
                            width: Some(800.0),
                            height: Some(600.0),
                        },
                        available_space: Size {
                            width: AvailableSpace::Definite(800.0),
                            height: AvailableSpace::Definite(80.0),
                        },
                        ..LayoutInput::HIDDEN
                    },
                );
                let x = if with_sibling { 10.0 } else { 0.0 };
                let mut expected = vec![(item, (x, tall_y, 10.0, 170.0))];
                if let Some(sibling) = sibling {
                    expected.push((sibling, (0.0, short_y, 10.0, 50.0)));
                }
                for (node, expected) in expected {
                    let actual = tree.nodes[node].layout;
                    assert_eq!(
                        (
                            actual.location.x,
                            actual.location.y,
                            actual.size.width,
                            actual.size.height
                        ),
                        expected,
                        "{writing_mode}/{alignment}/sibling={with_sibling}"
                    );
                }
            }
        }
    }
}
