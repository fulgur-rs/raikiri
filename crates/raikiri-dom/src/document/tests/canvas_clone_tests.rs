use super::canvas_tests::canvas_in_body;
use super::*;
use std::sync::Arc;

const ORIGINAL: [u8; 4] = [10, 20, 30, 255];

fn filled_canvas() -> (Document, usize) {
    let (mut doc, id) = canvas_in_body();
    doc.set_element_attribute(id, "width", "2").unwrap();
    doc.set_element_attribute(id, "height", "2").unwrap();
    assert!(doc.canvas_fill_rect(id, 0, 0, 2, 2, ORIGINAL));
    (doc, id)
}

#[test]
fn readonly_document_clones_and_extract_share_nonempty_pixels() {
    let (source, id) = filled_canvas();
    let pointer = source.canvas_bitmap_ref(id).unwrap().rgba.as_ptr();
    let copies: Vec<_> = (0..4).map(|_| source.clone()).collect();
    for copy in &copies {
        assert_eq!(copy.canvas_bitmap_ref(id).unwrap().rgba.as_ptr(), pointer);
    }
    let subset = source.extract_nodes(&source.preorder(source.root_index()).collect::<Vec<_>>());
    let subset_id = subset.canvas_ids_in_tree_order()[0];
    assert_eq!(
        subset.canvas_bitmap_ref(subset_id).unwrap().rgba.as_ptr(),
        pointer
    );
}

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Fill,
    Clear,
    Composite,
    Reset,
    Resize,
    Replace,
}

fn mutate(doc: &mut Document, id: usize, operation: Mutation) -> [u8; 4] {
    match operation {
        Mutation::Fill => {
            let pixel = [90, 80, 70, 255];
            assert!(doc.canvas_fill_rect(id, 0, 0, 1, 1, pixel));
            pixel
        }
        Mutation::Clear => {
            assert!(doc.canvas_clear_rect(id, 0, 0, 1, 1));
            [0; 4]
        }
        Mutation::Composite => {
            let pixel = [50, 40, 30, 255];
            assert!(doc.canvas_composite_rect(id, 0, 0, 1, 1, |_, _| Some(pixel)));
            pixel
        }
        Mutation::Reset => {
            doc.set_element_attribute(id, "width", "2").unwrap();
            [0; 4]
        }
        Mutation::Resize => {
            doc.set_element_attribute(id, "width", "3").unwrap();
            [0; 4]
        }
        Mutation::Replace => {
            let pixel = [3, 6, 9, 255];
            let mut rgba = vec![0; 16];
            rgba[..4].copy_from_slice(&pixel);
            doc.try_set_canvas_bitmap(id, CanvasBitmap::new(2, 2, rgba))
                .unwrap();
            pixel
        }
    }
}

#[test]
fn document_and_extract_writes_are_isolated_in_both_directions() {
    for extracted in [false, true] {
        for write_source in [false, true] {
            for operation in [
                Mutation::Fill,
                Mutation::Clear,
                Mutation::Composite,
                Mutation::Reset,
                Mutation::Resize,
                Mutation::Replace,
            ] {
                let (mut source, source_id) = filled_canvas();
                let mut copy = if extracted {
                    source.extract_nodes(&source.preorder(source.root_index()).collect::<Vec<_>>())
                } else {
                    source.clone()
                };
                let copy_id = copy.canvas_ids_in_tree_order()[0];
                assert_eq!(
                    source.canvas_bitmap_ref(source_id).unwrap().rgba.as_ptr(),
                    copy.canvas_bitmap_ref(copy_id).unwrap().rgba.as_ptr()
                );
                let (changed, changed_id, unchanged, unchanged_id) = if write_source {
                    (&mut source, source_id, &copy, copy_id)
                } else {
                    (&mut copy, copy_id, &source, source_id)
                };
                let expected = mutate(changed, changed_id, operation);
                let changed_bitmap = changed.canvas_bitmap_ref(changed_id).unwrap();
                assert_eq!(&changed_bitmap.rgba[..4], &expected, "{operation:?}");
                let untouched = unchanged.canvas_bitmap_ref(unchanged_id).unwrap();
                assert_eq!((untouched.width, untouched.height), (2, 2));
                assert_eq!(untouched.rgba, ORIGINAL.repeat(4), "{operation:?}");
                assert_ne!(
                    changed_bitmap.rgba.as_ptr(),
                    untouched.rgba.as_ptr(),
                    "{operation:?}"
                );
            }
        }
    }
}

#[test]
fn owned_reads_and_shared_take_copy_but_unique_take_moves() {
    let (mut source, id) = filled_canvas();
    let original = source.canvas_bitmap_ref(id).unwrap().rgba.as_ptr();
    let owned = source.canvas_bitmap(id).unwrap();
    assert_ne!(owned.rgba.as_ptr(), original);
    let sidecar = source.canvases_in_tree_order();
    assert_ne!(sidecar[0].rgba.as_ptr(), original);
    let mut copy = source.clone();
    let mut taken = copy.take_canvases_in_tree_order();
    assert_ne!(taken[0].rgba.as_ptr(), original);
    taken[0].rgba[0] = 99;
    assert_eq!(source.canvas_bitmap_ref(id).unwrap().rgba[0], 10);
    assert!(copy.canvas_bitmap_ref(id).is_none());
    let moved = source.take_canvases_in_tree_order();
    assert_eq!(moved[0].rgba.as_ptr(), original);
    assert!(source.canvas_bitmap_ref(id).is_none());
}

fn large_capacity_bitmap() -> CanvasBitmap {
    let mut rgba = Vec::with_capacity(super::super::MAX_DOCUMENT_CANVAS_BITMAP_BYTES / 2 + 1);
    rgba.resize(16, 0);
    CanvasBitmap::new(2, 2, rgba)
}

#[test]
fn shared_capacity_is_charged_per_document_and_rebuilt_after_cow() {
    let (mut source, first) = filled_canvas();
    let second = source.create_detached_element("canvas").unwrap();
    source
        .append_child(source.parent_of(first).unwrap(), second)
        .unwrap();
    source.set_element_attribute(second, "width", "2").unwrap();
    source.set_element_attribute(second, "height", "2").unwrap();
    source
        .try_set_canvas_bitmap(first, large_capacity_bitmap())
        .unwrap();
    let capacity = source.canvas_bitmap_ref(first).unwrap().rgba.capacity();
    let mut copy = source.clone();
    assert_eq!(
        copy.canvas_bitmap_ref(first).unwrap().rgba.capacity(),
        capacity
    );
    assert_eq!(
        copy.try_set_canvas_bitmap(second, large_capacity_bitmap()),
        Err(CanvasBitmapError::DocumentLimitExceeded)
    );
    assert!(copy.canvas_fill_rect(first, 0, 0, 1, 1, [90, 80, 70, 255]));
    assert_eq!(copy.canvas_bitmap_ref(first).unwrap().rgba.capacity(), 16);
    copy.try_set_canvas_bitmap(second, large_capacity_bitmap())
        .unwrap();
    assert_eq!(
        source.canvas_bitmap_ref(first).unwrap().rgba.capacity(),
        capacity
    );
    assert_eq!(
        source.try_set_canvas_bitmap(second, large_capacity_bitmap()),
        Err(CanvasBitmapError::DocumentLimitExceeded)
    );
}

#[test]
fn shared_pixels_live_until_the_last_document_or_snapshot_is_dropped() {
    let (source, id) = filled_canvas();
    let snapshot = source.canvas_bitmap_snapshot(id).unwrap();
    let weak = Arc::downgrade(&snapshot);
    let copy = source.clone();
    drop(source);
    assert!(weak.upgrade().is_some());
    drop(snapshot);
    assert!(weak.upgrade().is_some());
    drop(copy);
    assert!(weak.upgrade().is_none());
}
