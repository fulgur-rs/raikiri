//! raikiri-dom — DOM data model + layout engine (taffy + parley) + GCPM runtime side.
//!
//! node arena + taffy 6 trait impl + raikiri_traits::Dom co-design を実装。
//! 詳細は design 由来の §4 raikiri-dom scope 参照。

// Public module rustdoc cross-links some crate-private helpers (e.g.
// `crate::layout::preshape_text`, `Document::flags_dirty`) which resolve fine
// under `--document-private-items` but trip the strict public build.  Preserve
// the cross-links; the linker's audience is intra-crate readers.
#![allow(rustdoc::private_intra_doc_links)]
//!
//! ## Module tour
//!
//! - [`document`] — `Document` arena + `append_element` / `append_text`
//! - `node`     — `Node` struct (crate-private, no rustdoc entry)
//! - [`taffy_impl`] — taffy 6 layout trait impls + `unsafe impl Send for Document`
//! - [`dom_impl`] — `raikiri_traits::{Dom, Node, Element}` +
//!   `raikiri_style::{StyleDom, StyleNode, StyleElement}` impls + `NodeRef` /
//!   `ElementRef` types (both trait families over one arena, see file header)
//! - [`fonts`] — WPT bundled font dir から cross-machine 決定性 `FontContext`
//!   を構築する (`build_wpt_font_ctx`)
//!
//! # Flat tree membership
//!
//! [`Node`] は [`NodeFlags::IS_IN_DOCUMENT`] bit で「Document root から
//! flat-tree-parent 経由で到達可能」を表す。以下の subtree は clear される:
//!
//! - `<template>` element の子孫 (element 自身は in_document=true)
//! - 将来: shadow root 外の light-DOM 子孫、slotted-only 子孫、mutator の
//!   transient な detached node
//!
//! 維持: raikiri-html sink `finish()` が
//! [`Document::mark_in_document_flags`] を single pass で呼ぶ。現状の spike
//! 実装は parse-only なので finish 後は固定。将来 runtime mutation を導入する
//! 時に blitz `process_added_subtree` / `process_removed_subtree` 相当を
//! 追加する予定。
//!
//! Traversal が inert subtree を skip したい場合、
//! [`Node::is_in_document`] を各 iteration で呼ぶ。string 比較 (tag_name ==
//! "template" 等) で個別判定するのは禁止 — 概念が implicit になり、shadow DOM
//! 追加時に漏れる。

mod diag;
mod node;
mod phase_b;
mod running;
mod target;

pub mod document;
pub mod dom_impl;
pub mod fonts;
pub mod layout;
pub mod taffy_impl;

pub use document::Document;
pub use dom_impl::{ChildIter, ElementRef, NodeRef, StyleChildIter};
pub use fonts::{FontError, FontWarn, build_wpt_font_ctx, build_wpt_font_ctx_with_observer};
pub use layout::layout_single_page;
pub use node::{ElementData, Node, NodeData, NodeFlags, TextData};

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
        let layout_root = doc.append_element(Some(0), "root", root_style, None::<&str>);
        doc.append_element(Some(layout_root), "a", leaf_style.clone(), None::<&str>);
        doc.append_element(Some(layout_root), "b", leaf_style, None::<&str>);
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
        let p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
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
    fn child_ids_returns_empty_on_invalid_nodeid() {
        // child_ids は node() と対称に、範囲外 NodeId で
        // panic せず empty iter を返す。将来の blitz-compat integration で
        // Consumer が document rebuild を挟んで NodeId を stash する pattern に
        // 備える。
        let mut doc = Document::new();
        doc.append_element(Some(0), "p", Style::default(), None::<&str>);

        let far = NodeId::new(doc.nodes.len() as u64 + 100);
        let kids: Vec<_> = Dom::child_ids(&doc, far).collect();
        assert!(
            kids.is_empty(),
            "out-of-range NodeId should yield empty iter"
        );
        // node() と contract 一致確認 (対称性のリファレンス)
        assert!(doc.node(far).is_none());
    }

    #[test]
    fn document_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Document>();
    }

    #[test]
    fn element_ref_reflects_inline_style_source() {
        use raikiri_traits::{Dom, Element as _, Node as _, NodeId};

        let mut doc = Document::new();
        let p = doc.append_element(Some(0), "p", Style::default(), Some("color:red"));
        let noattr = doc.append_element(Some(0), "div", Style::default(), None::<&str>);

        let p_node = doc.node(NodeId::new(p as u64)).expect("p exists");
        let p_elem = p_node.as_element().expect("p is element");
        assert_eq!(p_elem.inline_style_source(), Some("color:red"));

        let d_node = doc.node(NodeId::new(noattr as u64)).expect("div exists");
        let d_elem = d_node.as_element().expect("div is element");
        assert_eq!(d_elem.inline_style_source(), None);
    }

    // ── Element trait extension ─────────────

    #[test]
    fn element_ref_reflects_namespace_uri_when_set() {
        use raikiri_traits::{Dom, Element as _, Node as _, NodeId};
        use smol_str::SmolStr;

        let mut doc = Document::new();
        let html_p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
        let svg_g = doc.append_element(Some(0), "g", Style::default(), None::<&str>);
        doc.set_element_namespace(svg_g, Some(SmolStr::new("http://www.w3.org/2000/svg")));

        let p_node = doc.node(NodeId::new(html_p as u64)).expect("p exists");
        let p_elem = p_node.as_element().expect("p is element");
        // HTML default は None を fast path として返す (setter を呼ばなくてよい契約)。
        assert_eq!(p_elem.namespace_uri(), None);

        let g_node = doc.node(NodeId::new(svg_g as u64)).expect("g exists");
        let g_elem = g_node.as_element().expect("g is element");
        assert_eq!(g_elem.namespace_uri(), Some("http://www.w3.org/2000/svg"));
    }

    #[test]
    fn element_ref_reflects_id_and_class_and_attr() {
        use raikiri_traits::{Dom, Element as _, Node as _, NodeId};
        use smol_str::SmolStr;

        let mut doc = Document::new();
        let el = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.set_element_attributes(
            el,
            vec![
                (SmolStr::new("id"), SmolStr::new("main")),
                (SmolStr::new("class"), SmolStr::new("foo  bar\tbaz")),
                (SmolStr::new("data-x"), SmolStr::new("42")),
                (SmolStr::new("empty"), SmolStr::new("")),
            ],
        );

        let el_node = doc.node(NodeId::new(el as u64)).expect("div exists");
        let elem = el_node.as_element().expect("div is element");

        // id lookup
        assert_eq!(elem.id(), Some("main"));

        // has_class: ASCII whitespace で split — space / tab 混在も token 化
        assert!(elem.has_class("foo"));
        assert!(elem.has_class("bar"));
        assert!(elem.has_class("baz"));
        assert!(!elem.has_class("qux"));
        // 空 token を渡すと false (spec: empty class token は match しない)
        assert!(!elem.has_class(""));

        // attr generic lookup
        assert_eq!(elem.attr("data-x"), Some("42"));
        // 空文字列 attribute は Some("") (attribute の有無と値は独立に追跡する
        // — CSS Selectors L4 の attribute-presence selector `[foo]` は値と
        // 無関係に存在だけで match するため、空文字列を absent と同一視しては
        // ならない contract)。id() 固有の empty-is-absent 正規化は id() 自身に
        // 局所化されており、この generic attr() には適用されない。
        assert_eq!(elem.attr("empty"), Some(""));
        // 未設定 attribute は None
        assert_eq!(elem.attr("missing"), None);
    }

    #[test]
    fn element_attr_style_reads_through_inline_style() {
        // `attr("style")` は Node.inline_style へ redirect され、
        // inline_style_source と同じ値を返す (trait doc の一致性契約)。
        use raikiri_traits::{Dom, Element as _, Node as _, NodeId};

        let mut doc = Document::new();
        let el = doc.append_element(Some(0), "p", Style::default(), Some("color:red"));
        // attributes には style を含めない (sink 側で分離済想定)。

        let el_node = doc.node(NodeId::new(el as u64)).expect("p exists");
        let elem = el_node.as_element().expect("p is element");
        assert_eq!(elem.attr("style"), Some("color:red"));
        assert_eq!(elem.attr("style"), elem.inline_style_source());
    }

    #[test]
    fn element_id_treats_empty_value_as_none_while_attr_preserves_presence() {
        // id="" is narrowly normalized to "no id" by `id()` (CSS Selectors L4
        // ID-selector semantics: an empty ID token cannot match `#foo`), but
        // the underlying `attr("id")` lookup must still report presence —
        // that empty-is-absent normalization is scoped to `id()` alone, not
        // to the general attribute-lookup path (see `ElementRef::attr`'s doc
        // in dom_impl.rs for why `[foo]` / boolean-attribute matching depends
        // on that distinction being preserved).
        use raikiri_traits::{Dom, Element as _, Node as _, NodeId};
        use smol_str::SmolStr;

        let mut doc = Document::new();
        let el = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.set_element_attributes(
            el,
            vec![
                (SmolStr::new("id"), SmolStr::new("")),
                (SmolStr::new("class"), SmolStr::new("")),
            ],
        );
        let el_node = doc.node(NodeId::new(el as u64)).expect("div exists");
        let elem = el_node.as_element().expect("div is element");
        assert_eq!(elem.id(), None);
        assert_eq!(elem.attr("id"), Some(""));
        assert!(!elem.has_class("foo"));
    }

    #[test]
    #[should_panic(expected = "set_element_attributes called on non-Element")]
    fn set_element_attributes_panics_on_non_element_in_debug() {
        // Document root (index 0) は Document kind、Text node は Text kind。
        // どちらも attribute-family setter の対象外なので debug_assert が
        // 発火することを regression pin する。
        use smol_str::SmolStr;
        let mut doc = Document::new();
        // arena index 0 = Document root
        doc.set_element_attributes(0, vec![(SmolStr::new("id"), SmolStr::new("bad"))]);
    }

    // ── TreeSink support APIs ────────────────────────────
    // NB: append_element takes a 4th `inline_style_source: Option<impl Into<SmolStr>>`
    // argument. These tests don't exercise inline style, so pass `None::<&str>`.

    #[test]
    fn attach_child_appends_to_parent_children() {
        let mut doc = Document::new();
        let a = doc.append_element(None, "a", Style::default(), None::<&str>); // detached
        doc.attach_child(0, a);
        let children: Vec<_> = Dom::child_ids(&doc, doc.root_id()).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0 as usize, a);
    }

    #[test]
    fn insert_child_before_places_at_correct_index() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default(), None::<&str>);
        let c = doc.append_element(Some(0), "c", Style::default(), None::<&str>);
        let b = doc.append_element(None, "b", Style::default(), None::<&str>); // detached
        doc.insert_child_before(0, c, b);
        let kids: Vec<_> = Dom::child_ids(&doc, doc.root_id())
            .map(|n| n.0 as usize)
            .collect();
        assert_eq!(kids, vec![a, b, c]);
    }

    #[test]
    fn parent_of_returns_containing_parent() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default(), None::<&str>);
        let child = doc.append_element(Some(a), "child", Style::default(), None::<&str>);
        assert_eq!(doc.parent_of(child), Some(a));
        assert_eq!(doc.parent_of(0), None); // root has no parent
    }

    #[test]
    fn detach_from_parent_removes_child_and_returns_parent() {
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default(), None::<&str>);
        let b = doc.append_element(Some(0), "b", Style::default(), None::<&str>);
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
        let hidden = doc.append_element(Some(0), "hidden", hidden_style, None::<&str>);
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
        let hidden_parent = doc2.append_element(Some(0), "hp", hidden_parent_style, None::<&str>);
        doc2.append_element(Some(hidden_parent), "child", Style::default(), None::<&str>);
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
        let src = doc.append_element(Some(0), "src", Style::default(), None::<&str>);
        let dst = doc.append_element(Some(0), "dst", Style::default(), None::<&str>);
        let c1 = doc.append_element(Some(src), "c1", Style::default(), None::<&str>);
        let c2 = doc.append_element(Some(src), "c2", Style::default(), None::<&str>);
        doc.reparent_children(src, dst);
        let src_kids: Vec<_> = Dom::child_ids(&doc, NodeId::new(src as u64)).collect();
        let dst_kids: Vec<_> = Dom::child_ids(&doc, NodeId::new(dst as u64))
            .map(|n| n.0 as usize)
            .collect();
        assert!(src_kids.is_empty());
        assert_eq!(dst_kids, vec![c1, c2]);
    }

    #[test]
    fn layout_cache_invalidated_after_mutation() {
        // Build a Document, run layout, mutate, run layout again — verify
        // the new layout reflects the mutation (not the stale cache).
        let leaf_style = Style {
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(50.0),
            },
            ..Default::default()
        };
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "root",
            Style {
                display: Display::Block,
                size: Size {
                    width: Dimension::length(400.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            },
            None::<&str>,
        );
        doc.append_element(Some(root), "a", leaf_style.clone(), None::<&str>);

        // First layout
        compute_root_layout(
            &mut doc,
            taffy::NodeId::from(root),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        );
        let first_height = doc.nodes[root].unrounded_layout.size.height;

        // Add a second child — root height should change (2 leaves = ~100)
        doc.append_element(Some(root), "b", leaf_style, None::<&str>);

        compute_root_layout(
            &mut doc,
            taffy::NodeId::from(root),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        );
        let second_height = doc.nodes[root].unrounded_layout.size.height;

        // If cache wasn't invalidated, second_height would equal first_height (stale)
        assert_ne!(
            first_height, second_height,
            "root height should change after adding a second child; cache invalidation missing"
        );
        assert!(
            second_height > first_height,
            "root should be taller after adding a child (got {first_height} -> {second_height})"
        );
    }

    #[test]
    fn many_mutations_still_yield_correct_layout() {
        // Verify that repeated mutations don't accumulate stale state.
        // Mutations only set a dirty flag (O(1)); the lazy clear on the
        // first compute_child_layout ensures correctness without O(N) per-mutation cost.
        let leaf_style = Style {
            size: Size {
                width: Dimension::length(10.0),
                height: Dimension::length(10.0),
            },
            ..Default::default()
        };
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "root",
            Style {
                display: Display::Block,
                size: Size {
                    width: Dimension::length(400.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            },
            None::<&str>,
        );
        // Add 200 children — each mutation flips layout_dirty (O(1)).
        for _ in 0..200 {
            doc.append_element(Some(root), "child", leaf_style.clone(), None::<&str>);
        }
        compute_root_layout(
            &mut doc,
            taffy::NodeId::from(root),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(9999.0),
            },
        );
        let h = doc.nodes[root].unrounded_layout.size.height;
        // 200 children * 10px stacked = 2000px (block layout).
        assert!(
            (h - 2000.0).abs() < 0.5,
            "expected root height ~2000, got {h} (stale cache would give a much smaller value)"
        );
    }

    #[test]
    fn taffy_leaf_measure_reads_pre_populated_text_layout() {
        // Node.text_layout に手動で parley Layout をセットして、taffy leaf closure が
        // その intrinsic size を返すことを直接検証する (layout_single_page 経由
        // ではなく leaf closure の pin として)。
        use parley::{Alignment, AlignmentOptions, FontContext, LayoutContext};
        use taffy::{AvailableSpace, NodeId as TaffyNodeId, Size};

        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "root",
            Style {
                display: Display::Block,
                size: Size {
                    width: Dimension::length(400.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            },
            None::<&str>,
        );
        let text = doc.append_text(root, "Hi");

        // 手動 pre-shape
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        let builder = layout_cx.ranged_builder(&mut fonts, "Hi", 1.0, true);
        let mut layout = builder.build("Hi");
        layout.break_all_lines(Some(400.0));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let expected_h = layout.height();
        doc.nodes[text]
            .data
            .as_text_mut()
            .expect("text node")
            .text_layout = Some(layout);

        compute_root_layout(
            &mut doc,
            TaffyNodeId::from(root),
            Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::Definite(600.0),
            },
        );

        let text_size = doc.nodes[text].unrounded_layout.size;
        assert!(
            text_size.width > 0.0,
            "text leaf must have non-zero width from parley layout (got {})",
            text_size.width
        );
        // root の block layout 経由で text leaf の高さが flow するはず
        let root_size = doc.nodes[root].unrounded_layout.size;
        assert!(
            (root_size.height - expected_h).abs() < 0.5,
            "root block should stack single text child at parley height {expected_h} (got {})",
            root_size.height
        );
    }

    // ── pub 化 smoke test ─────────────

    #[test]
    fn document_get_node_returns_some_for_valid_id_and_none_for_out_of_range() {
        let mut doc = Document::new();
        let p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
        // valid: root + p の 2 個存在
        assert!(doc.get_node(0).is_some());
        assert!(doc.get_node(p).is_some());
        // out of range: node_count 以上
        assert!(doc.get_node(doc.node_count()).is_none());
        assert!(doc.get_node(doc.node_count() + 100).is_none());
    }

    #[test]
    fn document_node_count_grows_with_appends() {
        let mut doc = Document::new();
        let n0 = doc.node_count(); // root only = 1
        assert_eq!(n0, 1);
        let _e = doc.append_element(Some(0), "e", Style::default(), None::<&str>);
        let _t = doc.append_text(0, "hi");
        assert_eq!(doc.node_count(), n0 + 2);
    }

    #[test]
    fn document_root_index_is_zero_and_matches_get_node_kind() {
        let doc = Document::new();
        assert_eq!(doc.root_index(), 0);
        let root = doc.get_node(doc.root_index()).expect("root exists");
        assert_eq!(root.kind(), NodeKind::Document);
    }

    #[test]
    fn node_accessors_are_callable_from_external_call_site() {
        // Node が NodeData tagged union に refactor された
        // 後の pub_surface pin。旧 pub field (kind / tag_name / text_layout)
        // が accessor method 化されたことを super::* から見えることで regression
        // pin する。external consumer 契約は無影響
        // (crates/raikiri/tests/external_consumer.rs は Node/Element field
        // access 0 件、こちらは raikiri-dom 内部 pub_surface)。
        let mut doc = Document::new();
        let e = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let t = doc.append_text(e, "hi");
        let node = doc.get_node(e).unwrap();
        let _ = &node.children;
        let _ = &node.unrounded_layout;
        let _ = node.kind();
        let _ = node.tag_name();
        let _ = node.text_layout();
        let _ = node.is_in_document();
        let tn = doc.get_node(t).unwrap();
        assert_eq!(tn.kind(), NodeKind::Text);
    }

    /// Does taffy's own block layout algorithm hang (infinite loop) when the
    /// `taffy::Style` geometry it's driven with is non-finite?
    ///
    /// This bypasses cascade / `raikiri_style::resolve` / `raikiri_dom::layout`'s
    /// bridge helpers **on the input side** — including the `sanitize_taffy` /
    /// `sanitize_finite` guards installed in those bridge functions — by
    /// constructing the `taffy::Style` directly and handing it to
    /// `Document::append_element` (the same raw-`Style` escape hatch
    /// `build_document` above already uses for non-cascade layout tests),
    /// then driving `compute_root_layout` straight from this test. That
    /// characterizes **taffy's own** block layout algorithm — the sink under
    /// test here — independent of whether raikiri's input guard currently
    /// prevents the input from reaching it — the same "characterize the
    /// sink, not just the guard" approach used for the parley probe in
    /// `crates/raikiri-dom/src/layout.rs`
    /// (`parley_break_all_lines_hangs_on_raw_infinite_font_size_bypassing_the_guard`)
    /// and the rasterizer probe in `crates/raikiri-paint/src/lib.rs`
    /// (`nonfinite_rasterizer_probe`).
    ///
    /// **What this does *not* bypass**: the output-side guard,
    /// `sanitize_taffy_layout`, called unconditionally from
    /// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
    /// (`crates/raikiri-dom/src/taffy_impl.rs`). `compute_root_layout` invokes
    /// that trait method itself as part of committing every node's computed
    /// layout, regardless of how the `Style` it started from was constructed
    /// — there is no code path through `compute_root_layout` that skips it.
    /// So this test's `Document.nodes[..].unrounded_layout` values are
    /// guaranteed finite by the output-side guard even though this test's
    /// *input* bypasses the input-side guard; the assertion below on the
    /// child's layout confirms exactly that (finite, non-default output),
    /// rather than silently relying on it.
    ///
    /// # Finding
    ///
    /// No hang: `compute_root_layout` returns well within the 10s bound for
    /// every non-finite field this test sets (width/height `+Inf`/`NaN`,
    /// padding/margin/border mixing `NaN`, `+Inf`, `-Inf`, and a `+Inf`
    /// percentage) — and it isn't a silently-skipped no-op either, since the
    /// child's `unrounded_layout` is asserted below to be both non-default
    /// and (per the output-side guard) finite. This is consistent with —
    /// and now formalizes as an automated regression pin, rather than leaving
    /// it as prose — the manual observation already recorded on
    /// `MAX_FONT_SIZE_PX` in `crates/raikiri-dom/src/layout.rs`: "site 1-4 の
    /// taffy 側 test は即座に assert 失敗する (値が壊れるだけ)". This test
    /// does not assert anything about *which specific* values taffy produces
    /// (a separate, still-open concern about the "finite garbage"
    /// semantic-validity of taffy's output is tracked elsewhere); it pins the
    /// **timing** (no livelock) and the **finiteness invariant** (the
    /// output-side guard holds even under directly-adversarial input, not
    /// just cascade-derived input) — taffy's block layout is not a
    /// livelock/hang surface for non-finite `Style` geometry, whether or not
    /// raikiri's own input guard is in the picture.
    #[test]
    fn taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry() {
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::Duration;
        use taffy::{LengthPercentage, LengthPercentageAuto, Rect};

        /// Returns `(doc, root, first_pathological_child)` — the child id is
        /// needed after layout to check its `unrounded_layout` (non-vacuity:
        /// distinguishes "actually laid out" from "silently skipped
        /// no-op", which a bare timing pass/fail cannot).
        fn build_nonfinite_document() -> (Document, usize, usize) {
            // Every geometry field taffy's block layout consumes gets a
            // different non-finite flavor, so a single run exercises +Inf,
            // -Inf, NaN, and +Inf-as-percentage together rather than needing
            // one test per flavor (this test only cares about *timing*, so
            // mixing them is fine — see doc comment above).
            let pathological_style = Style {
                size: Size {
                    width: Dimension::length(f32::INFINITY),
                    height: Dimension::length(f32::NAN),
                },
                padding: Rect {
                    left: LengthPercentage::length(f32::NAN),
                    right: LengthPercentage::percent(f32::INFINITY),
                    top: LengthPercentage::length(f32::NEG_INFINITY),
                    bottom: LengthPercentage::length(0.0),
                },
                margin: Rect {
                    left: LengthPercentageAuto::length(f32::INFINITY),
                    right: LengthPercentageAuto::length(f32::NEG_INFINITY),
                    top: LengthPercentageAuto::length(f32::NAN),
                    bottom: LengthPercentageAuto::length(0.0),
                },
                border: Rect {
                    left: LengthPercentage::length(f32::NAN),
                    right: LengthPercentage::length(0.0),
                    top: LengthPercentage::length(0.0),
                    bottom: LengthPercentage::length(0.0),
                },
                ..Default::default()
            };
            let root_style = Style {
                display: Display::Block,
                size: Size {
                    width: Dimension::length(400.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            };
            let mut doc = Document::new();
            let root = doc.append_element(Some(0), "root", root_style, None::<&str>);
            let a = doc.append_element(Some(root), "a", pathological_style.clone(), None::<&str>);
            doc.append_element(Some(root), "b", pathological_style, None::<&str>);
            (doc, root, a)
        }

        /// Layout::default()'s `size` is `(0.0, 0.0)` — a node taffy never
        /// visited (e.g. a bug that silently no-ops instead of laying out)
        /// would look identical to that, which is what this checks against.
        // cov:ignore: the `||` on the next line short-circuits — whichever
        // side isn't needed to determine `non_default` for the fixed test
        // fixture below never executes, but that's still finite coverage of
        // the boolean's meaning, not a gap in what's tested.
        fn layout_is_non_default_and_finite(layout: &taffy::Layout) -> bool {
            let default_size = taffy::Layout::new().size;
            let non_default = layout.size.width != default_size.width
                || layout.size.height != default_size.height;
            let finite = layout.size.width.is_finite()
                && layout.size.height.is_finite()
                && layout.location.x.is_finite()
                && layout.location.y.is_finite();
            non_default && finite
        }

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut doc, root, a) = build_nonfinite_document();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compute_root_layout(
                    &mut doc,
                    taffy::NodeId::from(root),
                    Size {
                        width: AvailableSpace::Definite(800.0),
                        height: AvailableSpace::Definite(600.0),
                    },
                );
                layout_is_non_default_and_finite(&doc.nodes[a].unrounded_layout)
            }));
            let _ = tx.send(result);
        });
        // cov:ignore: the happy path (Ok(Ok(true))) is the only arm this
        // fixed, non-hanging, non-panicking test fixture ever reaches; the
        // other 3 arms are diagnostic panics for failure modes this specific
        // characterization test isn't exercising (vacuous-pass, panic,
        // timeout, worker-disconnect) — see the doc comment above for why
        // each exists.
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(non_vacuous)) => assert!(
                non_vacuous,
                "compute_root_layout completed but child 'a' (pathological, non-finite Style) has a default-sized or non-finite unrounded_layout — either taffy silently skipped it (this test would be vacuous re: 'no hang', since a skipped node also 'completes' instantly) or the sanitize_taffy_layout output guard (set_unrounded_layout, crates/raikiri-dom/src/taffy_impl.rs) did not fire as expected — re-characterize this test rather than deleting this assertion"
            ),
            Ok(Err(_panic_payload)) => panic!(
                "taffy::compute_root_layout (or the post-layout assertion) panicked on non-finite Style geometry — re-characterize this test (it previously pinned 'completes without panic or hang')"
            ),
            Err(RecvTimeoutError::Timeout) => panic!(
                "taffy::compute_root_layout did not return within 10s on non-finite Style geometry — possible taffy block layout hang/livelock found; this would be a new finding, not a regression of a previously-passing guarantee, since this test is the first automated characterization of this input"
            ),
            Err(RecvTimeoutError::Disconnected) => panic!(
                "worker thread panicked before catch_unwind could report it cleanly — see stderr above"
            ),
        }
    }
}
