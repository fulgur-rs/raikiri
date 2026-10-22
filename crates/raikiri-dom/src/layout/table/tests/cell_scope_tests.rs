use super::*;
use std::cell::Cell;
type Inspector = fn(&mut LayoutSession<'_>, usize);
thread_local! { static INSPECTOR: Cell<Option<Inspector>> = const { Cell::new(None) }; }
pub(in crate::layout::table) fn before_measure(tree: &mut LayoutSession<'_>, idx: usize) {
    if let Some(inspect) = INSPECTOR.get() {
        inspect(tree, idx);
    }
}
fn with_inspector<R>(inspect: Inspector, f: impl FnOnce() -> R) -> R {
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
        assert!(
            tree.layout_style_for_key(idx)
                .unwrap()
                .size
                .height
                .is_auto()
        );
        assert!(
            tree.override_checkpoint() > 0,
            "cell measurement must use the typed scope"
        );
        tree.layout_style_for_key_mut(idx).unwrap().size.width = Dimension::length(91.0);
    }
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture_with_styles(
        "display:table-row",
        "display:table-cell;height:80px;width:31px",
    );
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
    let mut cell = make_cell(child, 0, 0, 1, 1, Dimension::auto());
    let height = with_inspector(inspect, || {
        super::super::measure_cell(&mut tree, &mut cell, 50.0)
    });
    assert_eq!(height, 80.0);
    assert!(cell.natural_height <= height);
    assert!(cell.baseline.is_finite());
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.height,
        Dimension::length(80.0)
    );
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(91.0)
    );
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn cell_scope_panic_restores_height_and_preserves_payload() {
    fn fail(tree: &mut LayoutSession<'_>, idx: usize) {
        assert!(
            tree.layout_style_for_key(idx)
                .unwrap()
                .size
                .height
                .is_auto()
        );
        tree.layout_style_for_key_mut(idx).unwrap().size.width = Dimension::length(91.0);
        panic!("cell measurement failure");
    }
    let (mut doc, cascade, child, _) = crate::layout::style_views::tests::fixture_with_styles(
        "display:table-row",
        "display:table-cell;height:80px;width:31px",
    );
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    tree.prepare_style_views();
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
        tree.layout_style_for_key(child).unwrap().size.height,
        Dimension::length(80.0)
    );
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(91.0)
    );
    assert_eq!(tree.override_checkpoint(), 0);
}
