use super::*;

fn document() -> CachedBackgroundSource {
    CachedBackgroundSource::Svg(
        Arc::new(
            raikiri_svg::SvgDocument::parse(
                b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
            )
            .unwrap(),
        )
        .into_source_only(),
    )
}

#[test]
fn checked_delta_rejects_underflow_and_overflow() {
    let change = SourceChange {
        old: None,
        new: None,
        removed: 4,
        added: 8,
    };
    assert_eq!(change.apply_to(3), None);
    assert_eq!(change.apply_to(u64::MAX), None);
    assert_eq!(change.apply_to(4), Some(8));
}

#[test]
fn invalid_or_overflowed_ledger_plan_leaves_records_unchanged() {
    let mut ledger = SourceLedger::default();
    let old = document();
    let new = document();
    let old_id = identity(&old).unwrap();
    let new_id = identity(&new).unwrap();
    let url = Url::parse("https://account.test/a.svg").unwrap();
    let work = AccountingWork::default();
    assert!(ledger.plan(Some(&old), &new, &url, &work).is_none());
    ledger.records.insert(
        old_id,
        SourceAllocation {
            references: 0,
            bytes: 1,
        },
    );
    assert!(ledger.plan(Some(&old), &new, &url, &work).is_none());
    ledger.records.insert(
        old_id,
        SourceAllocation {
            references: 1,
            bytes: u64::MAX,
        },
    );
    assert!(ledger.plan(Some(&old), &new, &url, &work).is_none());
    ledger.records.insert(
        new_id,
        SourceAllocation {
            references: usize::MAX,
            bytes: 1,
        },
    );
    let before = ledger.records.clone();
    assert!(ledger.plan(None, &new, &url, &work).is_none());
    assert_eq!(ledger.records, before);
}

#[test]
fn shared_reference_counts_follow_every_addition_and_removal() {
    let mut ledger = SourceLedger::default();
    let source = document();
    let id = identity(&source).unwrap();
    let url = Url::parse("https://account.test/a.svg").unwrap();
    let work = AccountingWork::default();
    let raster = CachedBackgroundSource::Raster(Arc::new(raikiri_traits::DecodedImage::new(
        1,
        1,
        vec![0; 4],
    )));
    for references in 1..=8 {
        let change = ledger.plan(None, &source, &url, &work).unwrap();
        ledger.apply(change, &work);
        assert_eq!(ledger.records[&id].references, references);
    }
    for references in (0..8).rev() {
        let change = ledger.plan(Some(&source), &raster, &url, &work).unwrap();
        ledger.apply(change, &work);
        if references == 0 {
            assert!(ledger.records.is_empty());
        } else {
            assert_eq!(ledger.records[&id].references, references);
        }
    }
}
