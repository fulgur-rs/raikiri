use super::*;
use raikiri_style::ComputedLengthPercentageOrAuto;
use std::cell::Cell;
type Inspector = fn(&mut LayoutSession<'_>, usize);
thread_local! { static INSPECTOR: Cell<Option<Inspector>> = const { Cell::new(None) }; }
pub(in crate::layout::table) fn before_measure(tree: &mut LayoutSession<'_>, idx: usize) {
    if let Some(inspect) = INSPECTOR.get() {
        inspect(tree, idx);
    }
}
pub(super) fn with_inspector<R>(inspect: Inspector, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<Inspector>);
    impl Drop for Reset {
        fn drop(&mut self) {
            INSPECTOR.set(self.0);
        }
    }
    let _reset = Reset(INSPECTOR.replace(Some(inspect)));
    f()
}

#[test]
fn cell_scope_height_auto_preserves_authored_floor() {
    fn inspect(tree: &mut LayoutSession<'_>, idx: usize) {
        assert!(tree.table_style(idx).size().height.is_auto());
        assert_eq!(
            tree.context.table_cell_replaced_height_bases.last(),
            Some(&(idx, 80.0))
        );
        assert!(
            tree.override_checkpoint() > 0,
            "cell measurement must use the typed scope"
        );
        tree.record_pass_override(
            idx,
            width_override(ComputedLengthPercentageOrAuto::Px(91.0)),
        );
    }
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture_with_styles(
        "display:table-row",
        "display:table-cell;height:80px;width:31px",
    );
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    tree.context
        .table_cell_replaced_height_bases
        .push((usize::MAX, 41.0));
    let mut cell = make_cell(child, 0, 0, 1, 1, Dimension::auto());
    let height = with_inspector(inspect, || {
        super::super::measure_cell(&mut tree, &mut cell, 50.0)
    });
    assert_eq!(height, 80.0);
    assert!(cell.natural_height <= height);
    assert!(cell.baseline.is_finite());
    assert_eq!(
        tree.table_style(child).size().height,
        Dimension::length(80.0)
    );
    assert_eq!(
        tree.table_style(child).size().width,
        Dimension::length(91.0)
    );
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(
        tree.context.table_cell_replaced_height_bases,
        vec![(usize::MAX, 41.0)]
    );
}

#[test]
fn cell_scope_panic_restores_height_and_preserves_payload() {
    fn fail(tree: &mut LayoutSession<'_>, idx: usize) {
        assert!(tree.table_style(idx).size().height.is_auto());
        assert_eq!(
            tree.context.table_cell_replaced_height_bases.last(),
            Some(&(idx, 80.0))
        );
        tree.record_pass_override(
            idx,
            width_override(ComputedLengthPercentageOrAuto::Px(91.0)),
        );
        panic!("cell measurement failure");
    }
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture_with_styles(
        "display:table-row",
        "display:table-cell;height:80px;width:31px",
    );
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    tree.context
        .table_cell_replaced_height_bases
        .push((usize::MAX, 41.0));
    let mut cell = make_cell(child, 0, 0, 1, 1, Dimension::auto());
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_inspector(fail, || {
            super::super::measure_cell(&mut tree, &mut cell, 50.0)
        })
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"cell measurement failure")
    );
    assert_eq!(
        tree.table_style(child).size().height,
        Dimension::length(80.0)
    );
    assert_eq!(
        tree.table_style(child).size().width,
        Dimension::length(91.0)
    );
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(
        tree.context.table_cell_replaced_height_bases,
        vec![(usize::MAX, 41.0)]
    );
}

#[test]
fn repeated_cell_measurements_use_the_current_authored_percentage_basis() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "html", Some("display:block"));
    let row = doc.append_element(Some(root), "div", Some("display:table-row"));
    let cell_id = doc.append_element(Some(row), "div", Some("display:table-cell;height:80px"));
    let image = doc.append_element(Some(cell_id), "img", Some("display:block;height:100%"));
    if let crate::node::NodeData::Element(element) = &mut doc.nodes[image].data {
        let mut intrinsic = raikiri_traits::IntrinsicBox::new(20.0, 50.0);
        intrinsic.aspect_ratio = Some(0.4);
        element.image_intrinsic_size = Some(intrinsic);
    }
    doc.append_element(Some(cell_id), "div", Some("display:block;height:220px"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let mut cell = make_cell(cell_id, 0, 0, 1, 1, Dimension::auto());
    for height in [80.0, 120.0, 80.0] {
        tree.record_pass_override(
            cell_id,
            length_override(
                raikiri_layout_style::LengthProperty::Height,
                ComputedLengthPercentageOrAuto::Px(height),
            ),
        );
        let measured = super::super::measure_cell(&mut tree, &mut cell, 100.0);
        assert_eq!(measured, height + 220.0);
        assert_eq!(tree.nodes[image].layout.size.height, height);
        assert!(tree.context.table_cell_replaced_height_bases.is_empty());
    }
}
