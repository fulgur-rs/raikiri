use super::*;
use crate::node::NodeFlags;
use crate::{Document, LayoutStyle};

fn element(doc: &mut Document, parent: Option<usize>) -> usize {
    doc.append_element(parent, "div", LayoutStyle::default(), None::<&str>)
}

#[test]
fn cursor_visits_each_ancestor_and_stops_after_the_root() {
    let mut doc = Document::new();
    let parent = element(&mut doc, Some(0));
    let child = element(&mut doc, Some(parent));
    for relation in [ParentRelation::Source, ParentRelation::Layout] {
        let mut walk = AncestorCursor::new(Some(child), relation);
        assert_eq!(walk.current(), Some(child));
        walk.advance(&doc);
        assert_eq!(walk.current(), Some(parent));
        walk.advance(&doc);
        assert_eq!(walk.current(), Some(0));
        walk.advance(&doc);
        assert_eq!(walk.current(), None);
        walk.advance(&doc);
        assert_eq!(walk.current(), None);
    }
}

#[test]
fn cursor_prefetch_keeps_the_parent_from_before_a_tree_mutation() {
    let mut doc = Document::new();
    let parent = element(&mut doc, Some(0));
    let other = element(&mut doc, Some(0));
    let child = element(&mut doc, Some(parent));
    for relation in [ParentRelation::Source, ParentRelation::Layout] {
        doc.append_child(parent, child).unwrap();
        let mut walk = AncestorCursor::new(Some(child), relation);
        walk.advance(&doc);
        doc.append_child(other, child).unwrap();
        assert_eq!(walk.current(), Some(parent));
    }
}

#[test]
fn cursor_postfetch_reads_the_parent_after_a_tree_mutation() {
    let mut doc = Document::new();
    let parent = element(&mut doc, Some(0));
    let other = element(&mut doc, Some(0));
    let child = element(&mut doc, Some(parent));
    for relation in [ParentRelation::Source, ParentRelation::Layout] {
        doc.append_child(parent, child).unwrap();
        let mut walk = AncestorCursor::new(Some(child), relation);
        assert_eq!(walk.current(), Some(child));
        doc.append_child(other, child).unwrap();
        walk.advance(&doc);
        assert_eq!(walk.current(), Some(other));
    }
}

#[test]
fn an_empty_cursor_remains_finished() {
    let doc = Document::new();
    for relation in [ParentRelation::Source, ParentRelation::Layout] {
        let mut walk = AncestorCursor::new(None, relation);
        walk.advance(&doc);
        assert_eq!(walk.current(), None);
    }
    assert_eq!(doc.layout_ancestors(0).next(), None);
    assert_eq!(doc.inclusive_layout_ancestors(0).collect::<Vec<_>>(), [0]);
}

#[test]
fn detached_and_template_subtrees_stop_before_any_host() {
    let mut doc = Document::new();
    let detached = element(&mut doc, None);
    let detached_leaf = element(&mut doc, Some(detached));
    let template = doc.append_element(Some(0), "template", LayoutStyle::default(), None::<&str>);
    let fragment = doc.allocate_template_fragment_root(template);
    let template_leaf = element(&mut doc, Some(fragment));
    for (leaf, top) in [(detached_leaf, detached), (template_leaf, fragment)] {
        for relation in [ParentRelation::Source, ParentRelation::Layout] {
            let mut walk = AncestorCursor::new(Some(leaf), relation);
            walk.advance(&doc);
            assert_eq!(walk.current(), Some(top));
            walk.advance(&doc);
            assert_eq!(walk.current(), None);
        }
        assert_eq!(doc.layout_ancestors(leaf).collect::<Vec<_>>(), [top]);
        assert_eq!(
            doc.inclusive_layout_ancestors(leaf).collect::<Vec<_>>(),
            [leaf, top]
        );
    }
}

#[test]
fn an_invalid_inclusive_start_is_yielded_once() {
    let doc = Document::new();
    let missing = usize::MAX;
    for relation in [ParentRelation::Source, ParentRelation::Layout] {
        let mut walk = AncestorCursor::new(Some(missing), relation);
        assert_eq!(walk.current(), Some(missing));
        walk.advance(&doc);
        assert_eq!(walk.current(), None);
    }
    assert_eq!(doc.layout_ancestors(missing).next(), None);
    assert_eq!(
        doc.inclusive_layout_ancestors(missing).collect::<Vec<_>>(),
        [missing]
    );
}

#[test]
fn layout_walk_skips_inline_offsets_that_a_source_walk_visits() {
    let mut doc = Document::new();
    let root = element(&mut doc, Some(0));
    let outer = element(&mut doc, Some(root));
    let inner = element(&mut doc, Some(outer));
    let atomic = element(&mut doc, Some(inner));
    doc.nodes[root].flags.insert(NodeFlags::IS_IFC_ROOT);
    for id in [outer, inner] {
        doc.nodes[id].flags.insert(NodeFlags::IN_IFC_SUBTREE);
    }
    let mut source = AncestorCursor::new(Some(atomic), ParentRelation::Source);
    let mut layout = AncestorCursor::new(Some(atomic), ParentRelation::Layout);
    source.advance(&doc);
    layout.advance(&doc);
    assert_eq!(source.current(), Some(inner));
    assert_eq!(layout.current(), Some(root));
    assert_eq!(
        doc.ancestors(atomic).collect::<Vec<_>>(),
        [inner, outer, root, 0]
    );
    assert_eq!(doc.layout_ancestors(atomic).collect::<Vec<_>>(), [root, 0]);
    assert_eq!(
        doc.inclusive_layout_ancestors(atomic).collect::<Vec<_>>(),
        [atomic, root, 0]
    );
    assert_eq!(
        doc.layout_ancestors(inner).collect::<Vec<_>>(),
        [outer, root, 0]
    );
}
