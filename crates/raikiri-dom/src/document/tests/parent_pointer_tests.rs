//! Parent-pointer correctness and appendChild loop smoke.
//!
//! Covers the O(1) parent_of backing store introduced for
//! raikiri-spike-4nhl.119: every mutation primitive keeps the per-node
//! parent in sync with the parent children list, so script loops over
//! appendChild stay linear instead of quadratic.

use super::*;
use taffy::Style;

#[test]
fn parent_of_tracks_append_primitives() {
    let mut doc = Document::new();
    assert_eq!(doc.parent_of(0), None);
    let elem = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    assert_eq!(doc.parent_of(elem), Some(0));
    let text = doc.append_text(elem, "hi");
    assert_eq!(doc.parent_of(text), Some(elem));
    let comment = doc.append_comment(Some(elem), "c");
    assert_eq!(doc.parent_of(comment), Some(elem));
    let pi = doc.append_processing_instruction(Some(elem), "target", "data");
    assert_eq!(doc.parent_of(pi), Some(elem));
    let detached = doc.append_element(None, "span", Style::default(), None::<&str>);
    assert_eq!(doc.parent_of(detached), None);
    assert_eq!(doc.parent_of(doc.node_count() + 100), None);
}

#[test]
fn parent_of_tracks_attach_and_insert_before() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let other = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
    let child = doc.append_element(None, "span", Style::default(), None::<&str>);
    doc.attach_child(parent, child);
    assert_eq!(doc.parent_of(child), Some(parent));
    doc.detach_from_parent(child);
    assert_eq!(doc.parent_of(child), None);
    assert!(!doc.nodes[parent].children.contains(&child));
    doc.attach_child(parent, child);
    let before = doc.append_text(parent, "tail");
    doc.detach_from_parent(child);
    doc.insert_child_before(parent, before, child);
    assert_eq!(doc.nodes[parent].children, vec![child, before]);
    assert_eq!(doc.parent_of(child), Some(parent));
    assert_eq!(doc.parent_of(before), Some(parent));
    let _ = other;
}

#[test]
fn parent_of_tracks_fragment_splice() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "main", Style::default(), None::<&str>);
    let frag = doc.create_detached_fragment();
    let a = doc.append_text(frag, "a");
    let b = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);
    assert_eq!(doc.parent_of(a), Some(frag));
    doc.attach_child(host, frag);
    assert_eq!(doc.nodes[frag].children.len(), 0);
    assert_eq!(doc.parent_of(a), Some(host));
    assert_eq!(doc.parent_of(b), Some(host));
    assert_eq!(doc.parent_of(frag), None);
    let frag2 = doc.create_detached_fragment();
    let c = doc.append_text(frag2, "c");
    let tail = doc.append_text(host, "tail");
    doc.insert_child_before(host, tail, frag2);
    assert_eq!(doc.parent_of(c), Some(host));
    assert!(doc.nodes[frag2].children.is_empty());
}

#[test]
fn parent_of_tracks_reparent_and_retain() {
    let mut doc = Document::new();
    let from = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let to = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
    let k1 = doc.append_text(from, "one");
    let k2 = doc.append_text(from, "two");
    doc.reparent_children(from, to);
    assert!(doc.nodes[from].children.is_empty());
    assert_eq!(doc.parent_of(k1), Some(to));
    assert_eq!(doc.parent_of(k2), Some(to));
    doc.retain_children(|id| id != k1);
    assert_eq!(doc.parent_of(k1), None);
    assert_eq!(doc.parent_of(k2), Some(to));
    assert!(!doc.nodes[to].children.contains(&k1));
}

#[test]
fn parent_of_tracks_text_content_replacement() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let nested = doc.append_element(Some(host), "span", Style::default(), None::<&str>);
    let nested_text = doc.append_text(nested, "old");
    doc.set_element_text_content(host, "new").unwrap();
    assert_eq!(doc.parent_of(nested), None);
    assert_eq!(doc.parent_of(nested_text), Some(nested));
    assert_eq!(doc.nodes[host].children.len(), 1);
    let fresh = doc.nodes[host].children[0];
    assert_eq!(doc.parent_of(fresh), Some(host));
    doc.set_element_text_content(host, "").unwrap();
    assert!(doc.nodes[host].children.is_empty());
    assert_eq!(doc.parent_of(fresh), None);
}

#[test]
fn parent_of_tracks_move_semantics_append_child() {
    let mut doc = Document::new();
    let first = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let second = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
    let child = doc.append_element(Some(first), "span", Style::default(), None::<&str>);
    doc.append_child(second, child).unwrap();
    assert!(doc.nodes[first].children.is_empty());
    assert_eq!(doc.parent_of(child), Some(second));
}

#[test]
#[cfg(feature = "logical-snapshot")]
fn parent_of_survives_snapshot_roundtrip() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_text(host, "hi");
    let snap = doc.logical_snapshot();
    let restored = Document::from_logical_snapshot(snap, 100).unwrap();
    assert_eq!(restored.parent_of(host), Some(0));
    assert_eq!(restored.parent_of(child), Some(host));
}

#[test]
fn append_child_loop_stays_linear() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let count = 3000usize;
    let start = std::time::Instant::now();
    for i in 0..count {
        let child = doc.create_detached_element("span").unwrap();
        doc.append_child(host, child).unwrap();
        assert_eq!(doc.parent_of(child), Some(host));
        let _ = i;
    }
    let elapsed = start.elapsed();
    assert_eq!(doc.nodes[host].children.len(), count);
    assert!(
        elapsed.as_secs() < 10,
        "3000 detached appendChild took {elapsed:?}, expected well under 10s with O(1) parent pointers"
    );
}
