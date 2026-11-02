use super::*;

#[test]
fn one_block_walk_reuses_every_origin_prefix_exactly_once() {
    for depth in [16, 32, 64, 128] {
        let mut dom = TestDoc::new();
        let sheet = dom.push_element(0, "style", None);
        dom.push_text(
            sheet,
            "div::first-line{color:red;text-decoration:underline}",
        );
        let mut parent = 0;
        let mut origins = Vec::new();
        for _ in 0..depth {
            parent = dom.push_element(parent, "div", Some("display:block;font-size:10px"));
            origins.push(StyleNodeId::new(parent as u64));
        }
        let paragraph = *origins.last().unwrap();
        let cascade = crate::cascade(&dom, &crate::build_rule_tree(&dom)).unwrap();
        let ((expected_root, expected_origins), old_work) = count_recomputations(|| {
            let root = cascade
                .first_line_style(&dom, &origins, paragraph, paragraph, None)
                .unwrap();
            let values = origins
                .iter()
                .map(|&origin| {
                    (
                        origin,
                        cascade
                            .first_line_style(&dom, &origins, origin, origin, None)
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            (root, values)
        });
        let mut found = Vec::new();
        let (root, new_work) = count_recomputations(|| {
            cascade
                .first_line_block_style(&dom, &origins, paragraph, |id, cv| {
                    found.push((id, cv.clone()))
                })
                .unwrap()
        });
        assert_eq!(root, expected_root);
        assert_eq!(found, expected_origins);
        assert_eq!(old_work, (depth - 1) + depth * (depth - 1) / 2);
        assert_eq!(new_work, depth - 1);
        eprintln!(
            "first-line origin recomputations: depth={depth}, old={old_work}, new={new_work}"
        );
    }
}

#[test]
fn block_origin_visits_preserve_relative_values_rollback_and_gaps() {
    for gap in [false, true] {
        for paragraph_below_origin in [false, true] {
            let mut dom = TestDoc::new();
            let sheet = dom.push_element(0, "style", None);
            dom.push_text(sheet,"@layer low, high; body::first-line{font-size:12px;color:red;--ink:blue;text-decoration:underline} @layer low {div::first-line{font-size:150%;color:var(--ink);text-decoration:overline}} @layer high {div::first-line{color:purple;all:revert-layer}} ");
            let outer = dom.push_element(0, "body", Some("font-size:10px;--ink:green"));
            let parent = if gap {
                dom.push_element(
                    outer,
                    "section",
                    Some("display:block;font-size:99px;color:yellow;--ink:blue"),
                )
            } else {
                outer
            };
            let inner = dom.push_element(
                parent,
                "div",
                Some("display:block;font-size:40px;--ink:purple"),
            );
            let paragraph = if paragraph_below_origin {
                dom.push_element(
                    inner,
                    "section",
                    Some("display:block;font-size:77px;color:pink;--ink:black"),
                )
            } else {
                inner
            };
            let origins = [
                StyleNodeId::new(outer as u64),
                StyleNodeId::new(inner as u64),
            ];
            let paragraph = StyleNodeId::new(paragraph as u64);
            let cascade = crate::cascade(&dom, &crate::build_rule_tree(&dom)).unwrap();
            let expected = cascade
                .first_line_style(&dom, &origins, paragraph, paragraph, None)
                .unwrap();
            let mut found = Vec::new();
            let root = cascade
                .first_line_block_style(&dom, &origins, paragraph, |id, cv| {
                    found.push((id, cv.clone()))
                })
                .unwrap();
            assert_eq!(root, expected);
            assert_eq!(found.len(), origins.len());
            for ((id, cv), origin) in found.iter().zip(origins) {
                assert_eq!(*id, origin);
                assert_eq!(
                    *cv,
                    cascade
                        .first_line_style(&dom, &origins, origin, origin, None)
                        .unwrap()
                );
            }
            assert_ne!(found[0].1.font_size, found[1].1.font_size);
            assert_eq!(found[0].1.font_size.px(), 12.);
            assert_eq!(found[1].1.font_size.px(), 18.);
        }
    }
}

#[test]
fn one_origin_at_paragraph_and_invalid_outer_contexts_are_handled() {
    let mut dom = TestDoc::new();
    let sheet = dom.push_element(0, "style", None);
    dom.push_text(sheet, "div::first-line{font-size:20px;--ink:blue}");
    let origin = dom.push_element(0, "div", Some("--ink:red"));
    let unrelated = dom.push_element(0, "span", None);
    let cascade = crate::cascade(&dom, &crate::build_rule_tree(&dom)).unwrap();
    let origin = StyleNodeId::new(origin as u64);
    let unrelated = StyleNodeId::new(unrelated as u64);
    let mut visits = Vec::new();
    let root = cascade
        .first_line_block_style(&dom, &[origin], origin, |id, cv| {
            visits.push((id, cv.clone()))
        })
        .unwrap();
    assert_eq!(visits, [(origin, root.clone())]);
    assert_eq!(
        root.resolved_custom_property("--ink").as_deref(),
        Some("red")
    );
    for (origins, paragraph) in [
        (&[][..], origin),
        (&[unrelated][..], unrelated),
        (&[origin][..], unrelated),
    ] {
        assert!(
            cascade
                .first_line_block_style(&dom, origins, paragraph, |_, _| panic!(
                    "invalid context must not publish values"
                ))
                .is_none()
        );
    }
}
