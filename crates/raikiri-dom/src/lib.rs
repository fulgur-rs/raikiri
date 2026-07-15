//! raikiri-dom — DOM data model + layout engine (taffy + parley) + GCPM runtime side.
//!
//! M1.5 で node arena + taffy 6 trait impl + raikiri_traits::Dom co-design を
//! 実装。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 raikiri-dom 参照。
//!
//! ## Module tour
//!
//! - [`document`] — `Document` arena + `append_element` / `append_text`
//! - `node`     — `Node` struct (crate-private, no rustdoc entry)
//! - [`taffy_impl`] — taffy 6 layout trait impls + `unsafe impl Send for Document`
//! - [`dom_impl`] — `raikiri_traits::Dom / Node / Element` impls + `NodeRef` /
//!   `ElementRef` types

mod node;

pub mod document;
pub mod dom_impl;
pub mod taffy_impl;

pub use document::Document;
pub use dom_impl::{ChildIter, ElementRef, NodeRef};

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{Dom, Element, Node, NodeId, NodeKind};
    use taffy::prelude::*;
    use taffy::{AvailableSpace, Dimension, Display, Size, Style, compute_root_layout};

    fn build_document(display: Display) -> (Document, usize) {
        let mut doc = Document::new();
        let leaf_style = Style {
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(50.0),
            },
            ..Default::default()
        };
        let mut root_style = Style {
            display,
            size: Size {
                width: Dimension::length(400.0),
                height: Dimension::auto(),
            },
            ..Default::default()
        };
        if matches!(display, Display::Grid) {
            root_style.grid_template_columns = vec![length(200.0), length(200.0)];
            root_style.grid_template_rows = vec![length(50.0)];
        }
        // Document node (idx=0) の子として layout root (idx=1) を作る
        let layout_root = doc.append_element(Some(0), "root", root_style);
        doc.append_element(Some(layout_root), "a", leaf_style.clone());
        doc.append_element(Some(layout_root), "b", leaf_style);
        (doc, layout_root)
    }

    #[test]
    fn all_three_display_modes_layout_non_degenerate() {
        for display in [Display::Block, Display::Flex, Display::Grid] {
            let (mut doc, layout_root) = build_document(display);
            compute_root_layout(
                &mut doc,
                taffy::NodeId::from(layout_root),
                Size {
                    width: AvailableSpace::Definite(800.0),
                    height: AvailableSpace::Definite(600.0),
                },
            );
            let layout = doc.nodes[layout_root].unrounded_layout;
            assert!(
                layout.size.width > 0.0 && layout.size.height > 0.0,
                "display={display:?} produced degenerate size ({}x{})",
                layout.size.width,
                layout.size.height,
            );
            assert!(
                (layout.size.width - 400.0).abs() < 0.5,
                "display={display:?}: root width should be ~400, got {}",
                layout.size.width,
            );
        }
    }

    #[test]
    fn independent_documents_lay_out_in_parallel() {
        use std::thread;
        const N: usize = 4;

        let docs: Vec<(Document, usize)> = (0..N)
            .map(|i| {
                let display = match i % 3 {
                    0 => Display::Block,
                    1 => Display::Flex,
                    _ => Display::Grid,
                };
                build_document(display)
            })
            .collect();

        let sizes: Vec<(f32, f32)> = thread::scope(|s| {
            let handles: Vec<_> = docs
                .into_iter()
                .map(|(mut doc, layout_root)| {
                    s.spawn(move || {
                        compute_root_layout(
                            &mut doc,
                            taffy::NodeId::from(layout_root),
                            Size {
                                width: AvailableSpace::Definite(800.0),
                                height: AvailableSpace::Definite(600.0),
                            },
                        );
                        let l = doc.nodes[layout_root].unrounded_layout;
                        (l.size.width, l.size.height)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        assert_eq!(sizes.len(), N);
        for (i, (w, h)) in sizes.iter().enumerate() {
            assert!(*w > 0.0 && *h > 0.0, "shard {i}: degenerate size ({w}x{h})");
        }
    }

    #[test]
    fn dom_trait_navigation() {
        let mut doc = Document::new();
        let p = doc.append_element(Some(0), "p", Style::default());
        let _t = doc.append_text(p, "hello");

        // root_id は Document kind の virtual root
        let root_id = doc.root_id();
        let root_node = doc.node(root_id).expect("root node exists");
        assert_eq!(root_node.kind(), NodeKind::Document);

        // Document の child = <p> element
        // NB: taffy::prelude::TraversePartialTree にも child_ids があるため UFCS で
        // raikiri_traits::Dom::child_ids を明示する。
        let root_children: Vec<_> = Dom::child_ids(&doc, root_id).collect();
        assert_eq!(root_children.len(), 1);

        // <p> は Element、tag_name = "p"
        let elem_id = root_children[0];
        let elem_node = doc.node(elem_id).expect("element exists");
        assert_eq!(elem_node.kind(), NodeKind::Element);
        let elem = elem_node.as_element().expect("kind == Element");
        assert_eq!(elem.tag_name(), "p");

        // <p> の child = "hello" text node
        let elem_children: Vec<_> = Dom::child_ids(&doc, elem_id).collect();
        assert_eq!(elem_children.len(), 1);
        let text_node = doc.node(elem_children[0]).expect("text exists");
        assert_eq!(text_node.kind(), NodeKind::Text);
        assert_eq!(text_node.text_content(), Some("hello"));
        assert!(text_node.as_element().is_none());
    }

    #[test]
    fn document_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Document>();
    }

    #[test]
    fn attach_child_appends_to_parent_children() {
        let mut doc = Document::new();
        let a = doc.append_element(None, "a", Style::default()); // detached
        doc.attach_child(0, a);
        let children: Vec<_> = Dom::child_ids(&doc, doc.root_id()).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0 as usize, a);
    }

    #[test]
    fn insert_child_before_places_at_correct_index() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default());
        let c = doc.append_element(Some(0), "c", Style::default());
        let b = doc.append_element(None, "b", Style::default()); // detached
        doc.insert_child_before(0, c, b);
        let kids: Vec<_> = Dom::child_ids(&doc, doc.root_id())
            .map(|n| n.0 as usize)
            .collect();
        assert_eq!(kids, vec![a, b, c]);
    }

    #[test]
    fn parent_of_returns_containing_parent() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default());
        let child = doc.append_element(Some(a), "child", Style::default());
        assert_eq!(doc.parent_of(child), Some(a));
        assert_eq!(doc.parent_of(0), None); // root has no parent
    }

    #[test]
    fn detach_from_parent_removes_child_and_returns_parent() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default());
        let b = doc.append_element(Some(0), "b", Style::default());
        assert_eq!(doc.detach_from_parent(a), Some(0));
        let kids: Vec<_> = Dom::child_ids(&doc, doc.root_id())
            .map(|n| n.0 as usize)
            .collect();
        assert_eq!(kids, vec![b]);
        // second detach is a no-op
        assert_eq!(doc.detach_from_parent(a), None);
    }

    #[test]
    fn empty_display_none_leaf_produces_hidden_layout() {
        // Case 1: empty (leaf) element with display:none — this is the case
        // Finding #1 caught (is_leaf was checked before display, so an empty
        // display:none leaf took the leaf-layout path instead of
        // LayoutOutput::HIDDEN).
        // A fixed size is set deliberately: if the buggy `is_leaf`-before-`display`
        // check regresses, the leaf-layout path would honor this explicit size
        // and produce a non-zero layout instead of LayoutOutput::HIDDEN's zero size.
        let mut doc = Document::new();
        let hidden_style = Style {
            display: Display::None,
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(50.0),
            },
            ..Default::default()
        };
        let hidden = doc.append_element(Some(0), "hidden", hidden_style);
        compute_root_layout(
            &mut doc,
            taffy::NodeId::from(hidden),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        );
        let layout = doc.nodes[hidden].unrounded_layout;
        assert_eq!(
            layout.size.width, 0.0,
            "display:none leaf should produce zero-size layout"
        );
        assert_eq!(layout.size.height, 0.0);

        // Case 2: non-empty (container) element with display:none — should
        // also produce HIDDEN, confirming the container path is unaffected.
        let mut doc2 = Document::new();
        let hidden_parent_style = Style {
            display: Display::None,
            ..Default::default()
        };
        let hidden_parent = doc2.append_element(Some(0), "hp", hidden_parent_style);
        doc2.append_element(Some(hidden_parent), "child", Style::default());
        compute_root_layout(
            &mut doc2,
            taffy::NodeId::from(hidden_parent),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        );
        let layout2 = doc2.nodes[hidden_parent].unrounded_layout;
        assert_eq!(
            layout2.size.width, 0.0,
            "display:none container should produce zero-size layout"
        );
        assert_eq!(layout2.size.height, 0.0);
    }

    #[test]
    fn reparent_children_moves_all_children_to_new_parent() {
        let mut doc = Document::new();
        let src = doc.append_element(Some(0), "src", Style::default());
        let dst = doc.append_element(Some(0), "dst", Style::default());
        let c1 = doc.append_element(Some(src), "c1", Style::default());
        let c2 = doc.append_element(Some(src), "c2", Style::default());
        doc.reparent_children(src, dst);
        let src_kids: Vec<_> = Dom::child_ids(&doc, NodeId::new(src as u64)).collect();
        let dst_kids: Vec<_> = Dom::child_ids(&doc, NodeId::new(dst as u64))
            .map(|n| n.0 as usize)
            .collect();
        assert!(src_kids.is_empty());
        assert_eq!(dst_kids, vec![c1, c2]);
    }
}
