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
    use raikiri_traits::{Dom, Element, Node, NodeKind};
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
}
