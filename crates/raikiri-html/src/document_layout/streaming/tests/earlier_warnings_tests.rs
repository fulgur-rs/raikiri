use super::*;

fn node_warning(node: u64, text: &str) -> RenderWarning {
    RenderWarning::new(WarningKind::PaintOrderApproximated, text.to_owned())
        .node_id(Some(NodeId(node)))
}

fn url(name: impl std::fmt::Display) -> url::Url {
    url::Url::parse(&format!("https://example.test/{name}")).expect("url")
}

/// What `sources` holds, in order, as text: `RenderWarning` has no
/// `PartialEq`.
fn held(sources: &[(WarningSource, Vec<RenderWarning>)]) -> Vec<(String, Vec<String>)> {
    sources
        .iter()
        .map(|(source, warnings)| {
            let label = match source {
                WarningSource::Node(node) => format!("node {}", node.0),
                WarningSource::Resource(url) => url.to_string(),
            };
            (label, warnings.iter().map(|w| format!("{w:?}")).collect())
        })
        .collect()
}

#[test]
fn warnings_are_held_per_element_in_the_order_first_reported() {
    let mut earlier = EarlierWarnings::default();
    let left_out = LeftOut(vec![true; 8]);
    let loaded = [(url("a.png"), vec![node_warning(0, "decode")])];
    earlier.record_warnings(
        &loaded,
        &[
            node_warning(5, "first of 5"),
            node_warning(2, "first of 2"),
            // A warning without an element is not kept.
            RenderWarning::new(WarningKind::PaintOrderApproximated, "none".to_owned()),
            node_warning(5, "second of 5"),
        ],
        &left_out,
    );
    let sources: Vec<_> = held(&earlier.sources)
        .into_iter()
        .map(|(source, warnings)| (source, warnings.len()))
        .collect();
    assert_eq!(
        sources,
        [
            ("node 5".to_owned(), 2),
            ("node 2".to_owned(), 1),
            (url("a.png").to_string(), 1),
        ]
    );
}

#[test]
fn a_resumed_layout_replaces_what_it_laid_out_again() {
    let mut earlier = EarlierWarnings::default();
    let all_left_out = LeftOut(vec![true; 8]);
    let resolved = [
        (url("kept.png"), vec![node_warning(0, "kept resource")]),
        (url("again.png"), vec![node_warning(0, "old resource")]),
        // A resource without warnings is not kept.
        (url("quiet.png"), Vec::new()),
    ];
    earlier.record_warnings(
        &resolved,
        &[
            node_warning(1, "left out"),
            node_warning(2, "laid out again"),
        ],
        &all_left_out,
    );
    // The next layout leaves out element 1 only, resolves `again.png` once
    // more and reports element 2 and 3 itself.
    let mut left_out = vec![false; 8];
    left_out[1] = true;
    earlier.record_warnings(
        &[(url("again.png"), Vec::new())],
        &[node_warning(1, "reported again"), node_warning(3, "new")],
        &LeftOut(left_out),
    );
    let held = held(&earlier.sources);
    let sources: Vec<&str> = held.iter().map(|(source, _)| source.as_str()).collect();
    assert_eq!(sources, ["node 1", url("kept.png").as_str(), "node 3"]);
    // Element 1 kept its earlier warning and took the new one after it.
    assert_eq!(held[0].1.len(), 2);
    assert!(held[0].1[0].contains("left out"), "{:?}", held[0].1);
    assert!(held[0].1[1].contains("reported again"), "{:?}", held[0].1);
}

/// `EarlierWarnings::record_warnings` as it was when each warning looked for
/// its element among the held sources, and each source looked for its
/// resource among the resolved ones.
fn record_by_scan(
    sources: &mut Vec<(WarningSource, Vec<RenderWarning>)>,
    resolved: &[(url::Url, Vec<RenderWarning>)],
    warnings: &[RenderWarning],
    left_out: &LeftOut,
) {
    sources.retain(|(source, _)| match source {
        WarningSource::Node(node) => left_out.holds(*node),
        WarningSource::Resource(url) => !resolved.iter().any(|(resolved, _)| resolved == url),
    });
    for warning in warnings {
        let Some(node) = warning.node_id else {
            continue;
        };
        let source = WarningSource::Node(node);
        match sources.iter_mut().find(|(held, _)| *held == source) {
            Some((_, warnings)) => warnings.push(warning.clone()),
            None => sources.push((source, vec![warning.clone()])),
        }
    }
    sources.extend(
        resolved
            .iter()
            .filter(|(_, warnings)| !warnings.is_empty())
            .map(|(url, warnings)| (WarningSource::Resource(url.clone()), warnings.clone())),
    );
}

#[test]
fn held_warnings_are_the_ones_a_scan_of_the_held_sources_keeps() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut dropped, mut kept) = (0, 0);
    for case in 0..1_500 {
        let mut earlier = EarlierWarnings::default();
        let mut by_scan = Vec::new();
        for round in 0..4 {
            let left_out = LeftOut((0..10).map(|_| next(3) != 0).collect());
            let warnings: Vec<RenderWarning> = (0..next(9))
                .map(|k| {
                    let text = format!("case {case} round {round} warning {k}");
                    match next(8) {
                        // Ids past the table are not left out by an element
                        // of it.
                        0 => RenderWarning::new(WarningKind::PaintOrderApproximated, text),
                        n => node_warning(n as u64 + 4 * (next(3) as u64), &text),
                    }
                })
                .collect();
            let resolved: Vec<(url::Url, Vec<RenderWarning>)> = (0..next(5))
                .map(|k| {
                    let own = (0..next(3))
                        .map(|w| node_warning(0, &format!("resource {k} warning {w}")))
                        .collect();
                    (url(next(4)), own)
                })
                .collect();
            let before = earlier.sources.len();
            earlier.record_warnings(&resolved, &warnings, &left_out);
            record_by_scan(&mut by_scan, &resolved, &warnings, &left_out);
            assert_eq!(
                held(&earlier.sources),
                held(&by_scan),
                "case {case} round {round}"
            );
            kept += earlier.sources.len();
            dropped += before.saturating_sub(earlier.sources.len());
        }
    }
    // The cases reach both what is kept and what is dropped.
    assert!(
        kept > 2_000 && dropped > 500,
        "kept {kept}, dropped {dropped}"
    );
}

#[test]
fn the_warnings_about_many_elements_are_held_in_linear_time() {
    let count = 100_000_u64;
    let warnings: Vec<RenderWarning> = (0..count).map(|node| node_warning(node, "x")).collect();
    let left_out = LeftOut(vec![true; count as usize]);
    let mut earlier = EarlierWarnings::default();
    let start = std::time::Instant::now();
    earlier.record_warnings(&[], &warnings, &left_out);
    let elapsed = start.elapsed();
    assert_eq!(earlier.sources.len(), count as usize);
    // Looking each element up takes milliseconds; scanning the held ones for
    // every warning takes seconds at this size.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "{elapsed:?} for {count} elements"
    );
}

#[test]
fn many_resolved_resources_are_checked_in_linear_time() {
    let count = 60_000;
    let resolved: Vec<(url::Url, Vec<RenderWarning>)> = (0..count)
        .map(|k| (url(k), vec![node_warning(0, "x")]))
        .collect();
    let left_out = LeftOut(vec![true; 1]);
    let mut earlier = EarlierWarnings::default();
    earlier.record_warnings(&resolved, &[], &left_out);
    assert_eq!(earlier.sources.len(), count);
    // The next layout resolves every one of them again.
    let start = std::time::Instant::now();
    earlier.record_warnings(&resolved, &[], &left_out);
    let elapsed = start.elapsed();
    assert_eq!(earlier.sources.len(), count);
    // Looking each held resource up takes milliseconds; scanning the
    // resolved ones for every held resource takes seconds at this size.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "{elapsed:?} for {count} resources"
    );
}
