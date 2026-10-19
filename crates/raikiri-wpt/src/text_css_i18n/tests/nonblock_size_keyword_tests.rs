use std::path::Path;

fn assert_wpt_cases_pass(test_id: &str, case_numbers: &[usize]) {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let result = super::run_testharness_file(&wpt_root.join(test_id), &wpt_root);

    assert!(result.error.is_none(), "{}: {:?}", test_id, result.error);
    let failures = case_numbers
        .iter()
        .filter_map(|&case_number| {
            let name = format!(".test {case_number}");
            let outcome = result
                .outcomes
                .iter()
                .find(|outcome| outcome.name == name)
                .unwrap_or_else(|| panic!("{test_id}: missing {name}"));
            (!outcome.passed).then(|| format!("{name}: {}", outcome.message))
        })
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{test_id}: {failures:?}");
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn absolute_boxes_honor_intrinsic_min_max_keyword_wpt_assertions() {
    assert_wpt_cases_pass(
        "css/css-sizing/keyword-sizes-on-abspos.html",
        &[
            7, 8, 9, 12, 13, 14, 22, 23, 24, 27, 28, 29, 37, 38, 39, 42, 43, 44, 52, 53, 54, 57,
            58, 59,
        ],
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn floats_honor_intrinsic_min_max_keyword_wpt_assertions() {
    assert_wpt_cases_pass(
        "css/css-sizing/keyword-sizes-on-floated-element.html",
        &[
            10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 31, 32, 33, 34,
            35, 36, 69, 70, 71, 72, 73, 74,
        ],
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn inline_blocks_honor_intrinsic_min_max_keyword_wpt_assertions() {
    assert_wpt_cases_pass(
        "css/css-sizing/keyword-sizes-on-inline-block.html",
        &[
            10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 31, 32, 33, 34,
            35, 36, 69, 70, 71, 72, 73, 74,
        ],
    );
}
