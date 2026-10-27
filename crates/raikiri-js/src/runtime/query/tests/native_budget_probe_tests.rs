use crate::runtime::webidl::with_state_unchecked;
use crate::runtime::{Limits, RunOptions};

#[test]
#[ignore = "manual configured cumulative production query budget probe"]
fn native_selector_budget_probe() {
    let limit = std::env::var("SELECTOR_NATIVE_LIMIT")
        .ok()
        .map(|s| s.parse().unwrap());
    let mut rt = super::native_work_probe_tests::run(
        RunOptions {
            limits: Limits {
                max_selector_work: limit,
                ..Limits::default()
            },
        },
        "configured",
    );
    let count =
        with_state_unchecked(rt.context_mut(), |s| s.event_loop.selector_work.count()).unwrap();
    eprintln!("SELECTOR_NATIVE_BUDGET limit={limit:?} count={count}");
}
