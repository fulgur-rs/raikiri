use super::*;

#[test]
fn only_running_elements_are_laid_out() {
    let cache = LayoutCache::new(&RunningIndex::default(), Arc::default(), 128);
    let none = cache.get_or_try_make(NodeId(2), 10.0, || -> Result<_, RenderError> {
        unreachable!("only running elements are laid out")
    });
    assert!(matches!(none, Ok(None)));
}

fn fixture(limit: Option<usize>) -> (crate::DocumentLayout, Vec<NodeId>) {
    let resources = crate::RenderResources::new();
    let parsed = crate::parse_html_with_resources(
        "<style>h1{position:running(a)}h2{position:running(b)}</style><h1 id=a>alpha</h1><h2 id=b>beta</h2><p>body</p>".as_bytes(),
        &resources,
    ).unwrap();
    let nodes = parsed
        .dom()
        .preorder(parsed.dom().root_index())
        .filter(|&id| matches!(parsed.dom().element_attribute(id, "id"), Some("a" | "b")))
        .map(|id| NodeId(id as u64))
        .collect();
    let options = crate::LayoutOptions::new().resources(&resources);
    let options = match limit {
        Some(limit) => options.max_running_layout_entries(limit),
        None => options,
    };
    let crate::LayoutStatus::Completed(out) = crate::layout(
        &parsed,
        crate::PageDefaults::default(),
        crate::LayoutConfig::default(),
        options,
    )
    .unwrap() else {
        panic!("aborted")
    };
    (out, nodes)
}

#[test]
fn default_cache_budget_preserves_existing_references_and_refuses_new_widths() {
    let (out, nodes) = fixture(None);
    let node = nodes[0];
    let first = out.layout_running_element(node, 200.0).unwrap().unwrap();
    for width in 201..328 {
        out.layout_running_element(node, width as f32)
            .unwrap()
            .unwrap();
    }
    assert!(matches!(
        out.layout_running_element(node, 328.0),
        Err(RenderError::LimitExceeded { .. })
    ));
    assert!(std::ptr::eq(
        first,
        out.layout_running_element(node, 200.0).unwrap().unwrap()
    ));
    assert_eq!(
        first
            .page()
            .text_runs()
            .into_iter()
            .map(|run| run.text)
            .collect::<String>(),
        "alpha"
    );
}

fn make(
    out: &crate::DocumentLayout,
    node: NodeId,
    width: f32,
) -> Result<Option<RunningElementLayout>, RenderError> {
    let source = out.running_source();
    layout_running_element(
        source.document,
        source.cascade,
        node,
        width,
        source.layouts.faces().clone(),
        source.svg,
    )
}

fn denied(result: Result<Option<&RunningElementLayout>, RenderError>, limit: u64) {
    assert!(matches!(result, Err(RenderError::LimitExceeded {
        kind: LimitKind::RunningLayoutEntries, limit: value, actual,
    }) if value == limit && actual == limit + 1));
}

#[test]
fn zero_and_global_multi_node_budget_are_explicit() {
    let (out, nodes) = fixture(Some(0));
    denied(out.layout_running_element(nodes[0], 200.0), 0);
    assert!(
        out.layout_running_element(NodeId(u64::MAX), 200.0)
            .unwrap()
            .is_none()
    );
    let (out, nodes) = fixture(Some(2));
    let a = out
        .layout_running_element(nodes[0], 200.0)
        .unwrap()
        .unwrap();
    let b = out
        .layout_running_element(nodes[1], 200.0)
        .unwrap()
        .unwrap();
    denied(out.layout_running_element(nodes[0], 201.0), 2);
    denied(out.layout_running_element(nodes[1], 201.0), 2);
    assert!(std::ptr::eq(
        a,
        out.layout_running_element(nodes[0], 200.0)
            .unwrap()
            .unwrap()
    ));
    assert!(std::ptr::eq(
        b,
        out.layout_running_element(nodes[1], 200.0)
            .unwrap()
            .unwrap()
    ));
    assert_eq!(
        b.page()
            .text_runs()
            .into_iter()
            .map(|run| run.text)
            .collect::<String>(),
        "beta"
    );
}

#[test]
fn absent_error_and_unwind_release_reservations() {
    crate::running_cache_diagnostics::begin_running_cache_diagnostics();
    let (out, nodes) = fixture(Some(1));
    let cache = out.running_source().layouts;
    let node = nodes[0];
    assert!(
        cache
            .get_or_try_make(node, 200.0, || Ok(None))
            .unwrap()
            .is_none()
    );
    assert_eq!(cache.budget.reserved.get(), 0);
    assert!(
        cache
            .get_or_try_make(node, 200.0, || Err(RenderError::Configuration(
                "test".into()
            )))
            .is_err()
    );
    assert_eq!(cache.budget.reserved.get(), 0);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cache.get_or_try_make(node, 200.0, || panic!("builder panic"))
    }));
    assert!(panic.is_err());
    assert_eq!(cache.budget.reserved.get(), 0);
    out.layout_running_element(node, 200.0).unwrap().unwrap();
    assert_eq!(cache.budget.reserved.get(), 1);
    let work = crate::running_cache_diagnostics::finish_running_cache_diagnostics();
    assert_eq!(work.admitted_entries, 1);
    assert_eq!(work.max_reserved_entries, 1);
}

#[test]
fn reentry_rechecks_same_and_different_widths_after_construction() {
    for same in [false, true] {
        crate::running_cache_diagnostics::begin_running_cache_diagnostics();
        let (out, nodes) = fixture(Some(2));
        let cache = out.running_source().layouts;
        let node = nodes[0];
        let inner_width = if same { 200.0 } else { 201.0 };
        let mut inner_ptr = None;
        let result = cache
            .get_or_try_make(node, 200.0, || {
                let inner = cache
                    .get_or_try_make(node, inner_width, || {
                        denied(cache.get_or_try_make(node, 202.0, || unreachable!()), 2);
                        make(&out, node, inner_width)
                    })?
                    .unwrap();
                inner_ptr = Some(inner as *const _);
                make(&out, node, 200.0)
            })
            .unwrap()
            .unwrap();
        assert_eq!(result.width(), 200.0);
        assert_eq!(cache.budget.reserved.get(), if same { 1 } else { 2 });
        let inner = out
            .layout_running_element(node, inner_width)
            .unwrap()
            .unwrap();
        assert_eq!(inner as *const _, inner_ptr.unwrap());
        assert_eq!(inner.width(), inner_width);
        if same {
            assert!(std::ptr::eq(result, inner));
        }
        let work = crate::running_cache_diagnostics::finish_running_cache_diagnostics();
        assert_eq!(work.budget_denials, 1);
        assert_eq!(work.max_reserved_entries, 2);
    }
}

#[test]
fn normalized_invalid_widths_share_zero_but_adjacent_widths_remain_distinct() {
    let (out, nodes) = fixture(Some(3));
    let node = nodes[0];
    let zero = out.layout_running_element(node, 0.0).unwrap().unwrap();
    for width in [-0.0, -10.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(std::ptr::eq(
            zero,
            out.layout_running_element(node, width).unwrap().unwrap()
        ));
    }
    let a = out.layout_running_element(node, 200.0).unwrap().unwrap();
    let width = f32::from_bits(200.0_f32.to_bits() + 1);
    let b = out.layout_running_element(node, width).unwrap().unwrap();
    assert!(!std::ptr::eq(a, b));
    assert_eq!(a.width().to_bits(), 200.0_f32.to_bits());
    assert_eq!(b.width().to_bits(), width.to_bits());
    assert_eq!(out.running_source().layouts.budget.reserved.get(), 3);
}

#[test]
fn longest_digital_path_has_at_most_33_comparisons_per_hit() {
    let (out, nodes) = fixture(Some(64));
    let cache = out.running_source().layouts;
    let node = nodes[0];
    let keys = [0x40000000, 0x40000001]
        .into_iter()
        .chain((0..30).rev().map(|bit| 1u32 << bit))
        .chain([0])
        .collect::<Vec<_>>();
    for &bits in &keys {
        cache
            .get_or_try_make(node, f32::from_bits(bits), || make(&out, node, 200.0))
            .unwrap()
            .unwrap();
    }
    // The builder's geometry is deliberately constant: this test isolates
    // the private index; the public width/normalization test checks geometry.
    for (index, &bits) in keys.iter().enumerate() {
        crate::running_cache_diagnostics::begin_running_cache_diagnostics();
        cache
            .get_or_try_make(node, f32::from_bits(bits), || unreachable!())
            .unwrap()
            .unwrap();
        let work = crate::running_cache_diagnostics::finish_running_cache_diagnostics();
        assert_eq!(work.lookups, 1);
        assert_eq!(work.width_comparisons, (index + 1) as u64);
        assert_eq!(work.make_calls, 0);
    }
    assert_eq!(keys.len(), 33);
}
