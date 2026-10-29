use crate::{CacheAccountingCase, CacheAccountingProbe};

#[test]
fn incremental_cache_accounting_avoids_full_entry_scans() {
    for case in CacheAccountingCase::ALL {
        for count in [16, 32, 64] {
            let mut probe = CacheAccountingProbe::prepare(case, count);
            probe.run(true);
            let result = probe.result();
            assert!(result.work > 0);
            assert!(
                result.work <= 32 * count as u64,
                "case {} N={count}: {} accounting operations",
                case.name(),
                result.work
            );
        }
    }
}

#[test]
fn accounting_probe_reports_real_cache_outcomes_with_optional_work_counts() {
    for case in CacheAccountingCase::ALL {
        for count_work in [false, true] {
            let mut probe = CacheAccountingProbe::prepare(case, 4);
            let before = probe.result();
            probe.run(count_work);
            let result = probe.result();
            assert!(!case.name().is_empty());
            assert_eq!(
                result.attempts,
                if case == CacheAccountingCase::Sparse {
                    3
                } else {
                    4
                }
            );
            assert_eq!(result.accounted_bytes, result.process_bytes);
            if matches!(
                case,
                CacheAccountingCase::CacheDenied | CacheAccountingCase::ProcessDenied
            ) {
                assert_eq!(result.rejected, 4);
                assert_eq!(result.accounted_bytes, before.accounted_bytes);
                assert_eq!(result.entries, before.entries);
            } else {
                assert_eq!(result.rejected, 0);
                assert!(result.accounted_bytes > 0);
            }
            if !count_work {
                assert_eq!(result.work, 0);
            }
        }
    }
}

#[test]
#[should_panic(expected = "probe count must be positive")]
fn accounting_probe_rejects_an_empty_workload() {
    let _ = CacheAccountingProbe::prepare(CacheAccountingCase::NewSources, 0);
}
