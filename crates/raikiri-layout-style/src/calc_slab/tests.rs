use super::*;
use raikiri_style::property::CalcLengthPercentage;

fn value(px: f32) -> CalcValue {
    CalcValue::new(CalcLengthPercentage { px, percent: 0.0 }, true)
}

fn px_of(table: &Table, handle: usize) -> Option<f32> {
    table.get(handle).map(|value| value.resolve(0.0))
}

fn index_of(handle: usize) -> usize {
    (handle >> TAG_BITS) & INDEX_MASK
}

#[test]
fn a_handle_has_its_tag_bits_clear_and_is_never_zero() {
    let table = Table::new();
    let handles: Vec<_> = (0..64)
        .map(|i| table.insert(value(i as f32)).unwrap())
        .collect();
    for (i, handle) in handles.iter().enumerate() {
        assert_eq!(handle & TAG_MASK, 0);
        assert_ne!(*handle, 0);
        assert_eq!(px_of(&table, *handle), Some(i as f32));
    }
    let mut distinct = handles.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), handles.len());
}

#[test]
fn a_value_keeps_its_expression_and_range_policy() {
    let table = Table::new();
    let expression = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let signed = table.insert(CalcValue::new(expression, false)).unwrap();
    let unsigned = table.insert(CalcValue::new(expression, true)).unwrap();
    let signed = table.get(signed).unwrap();
    let unsigned = table.get(unsigned).unwrap();
    assert_eq!(signed.resolve(0.0), -10.0);
    assert_eq!(unsigned.resolve(0.0), 0.0);
    assert!(!signed.non_negative() && unsigned.non_negative());
    assert!(signed.has_percentage());
    assert_eq!(signed.to_bits(), unsigned.to_bits());
}

#[test]
fn a_removed_handle_is_stale_even_when_its_slot_is_reused() {
    let table = Table::new();
    let first = table.insert(value(1.0)).unwrap();
    table.remove(first);
    assert_eq!(px_of(&table, first), None);

    let second = table.insert(value(2.0)).unwrap();
    assert_eq!(
        index_of(first),
        index_of(second),
        "the freed slot is reused"
    );
    assert_ne!(first, second);
    assert_eq!(px_of(&table, first), None);
    assert_eq!(px_of(&table, second), Some(2.0));
}

#[test]
fn removing_one_payload_leaves_the_others() {
    let table = Table::new();
    let handles: Vec<_> = (0..4)
        .map(|i| table.insert(value(i as f32)).unwrap())
        .collect();
    table.remove(handles[1]);
    assert_eq!(px_of(&table, handles[0]), Some(0.0));
    assert_eq!(px_of(&table, handles[1]), None);
    assert_eq!(px_of(&table, handles[2]), Some(2.0));
    assert_eq!(px_of(&table, handles[3]), Some(3.0));
}

#[test]
fn freed_slots_are_reused_in_the_order_they_were_freed() {
    let table = Table::new();
    let handles: Vec<_> = (0..4)
        .map(|i| table.insert(value(i as f32)).unwrap())
        .collect();
    table.remove(handles[2]);
    table.remove(handles[0]);
    let reused: Vec<_> = (0..3).map(|_| table.insert(value(9.0)).unwrap()).collect();
    assert_eq!(index_of(reused[0]), index_of(handles[2]));
    assert_eq!(index_of(reused[1]), index_of(handles[0]));
    assert_eq!(index_of(reused[2]), 4, "then a slot never used");
}

#[test]
fn slots_span_chunks_and_keep_their_values() {
    assert_eq!(locate(0), (0, 0));
    assert_eq!(locate(63), (0, 63));
    assert_eq!(locate(64), (1, 0));
    assert_eq!(locate(64 + 127), (1, 127));
    assert_eq!(locate(64 + 128), (2, 0));
    assert_eq!(locate(INDEX_MASK).0, CHUNKS - 1);

    let table = Table::new();
    let handles: Vec<_> = (0..700)
        .map(|i| table.insert(value(i as f32)).unwrap())
        .collect();
    for (i, handle) in handles.iter().enumerate() {
        assert_eq!(px_of(&table, *handle), Some(i as f32));
    }
}

#[test]
fn a_generation_wraps_to_one_and_never_to_zero() {
    assert_eq!(next_generation(1), 2);
    assert_eq!(next_generation(GENERATION_MASK - 1), GENERATION_MASK);
    assert_eq!(next_generation(GENERATION_MASK), 1);

    let table = Table::new();
    let seed = table.insert(value(0.0)).unwrap();
    table.remove(seed);
    let entry = table.entry(0).unwrap();
    entry
        .state
        .store((GENERATION_MASK - 1) << GENERATION_SHIFT, Ordering::Relaxed);

    let before = table.insert(value(1.0)).unwrap();
    table.remove(before);
    let last = table.insert(value(2.0)).unwrap();
    assert_eq!(px_of(&table, last), Some(2.0));
    assert_eq!(px_of(&table, before), None);
    table.remove(last);
    let wrapped = table.insert(value(3.0)).unwrap();
    assert_eq!(entry.state.load(Ordering::Relaxed) >> GENERATION_SHIFT, 1);
    assert_eq!(px_of(&table, wrapped), Some(3.0));
    assert_eq!(px_of(&table, before), None);
    assert_eq!(px_of(&table, last), None);
}

#[test]
fn a_handle_that_was_never_issued_names_nothing() {
    let table = Table::new();
    let handle = table.insert(value(1.0)).unwrap();
    assert_eq!(px_of(&table, 0), None);
    assert_eq!(px_of(&table, usize::MAX), None);
    assert_eq!(px_of(&table, usize::MAX & !TAG_MASK), None);
    // Another slot, another generation, or a tag bit that a real handle lacks.
    assert_eq!(px_of(&table, handle + (1 << (TAG_BITS + INDEX_BITS))), None);
    assert_eq!(px_of(&table, handle + (1 << TAG_BITS)), None);
    assert_eq!(px_of(&table, handle | 1), None);
    assert_eq!(px_of(&table, handle), Some(1.0));
}

#[test]
fn a_full_index_space_gives_no_handle() {
    let table = Table::new();
    table.writer().next = INDEX_MASK + 1;
    assert!(table.insert(value(1.0)).is_none());
}

#[test]
fn a_slot_frees_its_payload_when_dropped() {
    let slot = Slot::new(value(9.0));
    let handle = slot.handle();
    assert_eq!(lookup(handle).map(|value| value.resolve(0.0)), Some(9.0));
    drop(slot);
    assert!(lookup(handle).is_none());
}

#[test]
fn a_stale_handle_is_rejected_while_its_slot_is_reused_on_other_threads() {
    let table = Table::new();
    let stale = table.insert(value(7.0)).unwrap();
    table.remove(stale);
    let running = std::sync::atomic::AtomicBool::new(true);
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let mut reads = 0_u64;
                    while running.load(Ordering::Relaxed) {
                        assert!(table.get(stale).is_none());
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();
        let writers: Vec<_> = (0..2)
            .map(|thread| {
                let table = &table;
                scope.spawn(move || {
                    for round in 0..20_000 {
                        let px = (thread * 100_000 + round) as f32;
                        let handle = table.insert(value(px)).unwrap();
                        assert_eq!(px_of(table, handle), Some(px));
                        table.remove(handle);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        running.store(false, Ordering::Relaxed);
        for reader in readers {
            assert!(reader.join().unwrap() > 0);
        }
    });
}

#[test]
fn math_snapshots_survive_retirement_and_reuse_across_payload_kinds() {
    let table = Table::new();
    let expression = crate::tests::comparison("min(30px, calc(10px + 5%))");
    let weak = Arc::downgrade(&expression);
    let first = table
        .insert(CalcValue::new(expression.clone(), true))
        .unwrap();
    drop(expression);
    let snapshot = table.get(first).unwrap();
    assert_eq!(snapshot.resolve(10.0), 10.5);
    table.remove(first);
    assert!(table.get(first).is_none());
    let affine = table.insert(value(42.0)).unwrap();
    assert_eq!(index_of(first), index_of(affine));
    assert_eq!(px_of(&table, affine), Some(42.0));
    assert_eq!(snapshot.resolve(1000.0), 30.0);
    table.remove(affine);
    let last = table
        .insert(CalcValue::new(
            crate::tests::comparison("max(10px, 5%)"),
            false,
        ))
        .unwrap();
    assert_eq!(index_of(first), index_of(last));
    assert_eq!(table.get(last).unwrap().resolve(1000.0), 50.0);
    assert!(table.get(first).is_none());
    assert!(table.get(affine).is_none());
    assert!(table.get(last | 1).is_none());
    drop(snapshot);
    assert!(weak.upgrade().is_none());
    table.remove(last);
    assert!(
        table
            .entry(index_of(last))
            .unwrap()
            .math
            .get()
            .unwrap()
            .read()
            .unwrap()
            .is_none()
    );
}

#[test]
fn retiring_math_waits_for_a_reader_snapshot_to_release_the_slot_lock() {
    use std::sync::mpsc;
    let table = Arc::new(Table::new());
    let expression = crate::tests::comparison("min(30px, calc(10px + 5%))");
    let handle = table.insert(CalcValue::new(expression, true)).unwrap();
    let lock = table.entry(index_of(handle)).unwrap().math.get().unwrap();
    let reader = lock.read().unwrap();
    let snapshot = reader.as_ref().unwrap().clone();
    assert!(lock.try_write().is_err());
    let (started, receiver) = mpsc::channel();
    let (done, completion) = mpsc::channel();
    let writer = {
        let table = table.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            table.remove(handle);
            done.send(()).unwrap();
        })
    };
    receiver.recv().unwrap();
    assert!(matches!(
        completion.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(snapshot.resolve(10.0), 10.5);
    drop(reader);
    completion.recv().unwrap();
    writer.join().unwrap();
    assert!(table.get(handle).is_none());
    assert!(lock.read().unwrap().is_none());
    assert_eq!(snapshot.resolve(1000.0), 30.0);
}

#[test]
fn affine_slots_do_not_allocate_comparison_locks() {
    let table = Table::new();
    let handle = table.insert(value(42.0)).unwrap();
    assert!(table.entry(index_of(handle)).unwrap().math.get().is_none());
    assert_eq!(table.get(handle).unwrap().resolve(100.0), 42.0);
    assert!(table.entry(index_of(handle)).unwrap().math.get().is_none());
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::size_of::<Entry>(), 32);
    table.remove(handle);
}
