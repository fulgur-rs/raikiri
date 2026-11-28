use super::*;

#[test]
fn large_source_ids_and_repeated_capture_use_one_dense_slot_per_cell() {
    let mut store = GlyphStore::new(24).unwrap();
    let first = store.window(0, vec![usize::MAX, 12345]).unwrap();
    assert!(first.can_write(12345, 3));
    assert!(!first.can_write(9, 1));
    first.write(12345, b"xyz");
    first.write(usize::MAX, b"abc");
    assert!(!first.can_write(usize::MAX, 3));
    first.write(usize::MAX, b"new");
    store.append(&first);
    let mut bytes = [0; 3];
    assert!(store.read(0, &mut bytes));
    assert_eq!(&bytes, b"abc");
    assert!(store.read(1, &mut bytes));
    assert_eq!(&bytes, b"xyz");
    assert!(!store.read(usize::MAX, &mut bytes));
    assert!(!store.read(0, &mut [0; 2]));
    assert!(!store.can_record_from(2));
    assert_eq!(
        store.index.file.get_ref().metadata().unwrap().len()
            + store.data.file.get_ref().metadata().unwrap().len(),
        24
    );
}

#[test]
fn independent_windows_commit_in_order_with_a_shared_disk_limit() {
    let mut store = GlyphStore::new(49).unwrap();
    let first = store.window(0, vec![9, 19]).unwrap();
    let next = store.window(2, vec![29, 39, 49]).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| first.write(19, b"half"));
        scope.spawn(|| {
            next.write(29, b"second");
            next.write(39, b"12345678");
            next.write(49, b"");
        });
    });
    store.append(&first);
    store.append(&next);
    let mut bytes = [0; 6];
    assert!(store.read(2, &mut bytes));
    assert_eq!(&bytes, b"second");
    assert!(!store.read(0, &mut [0; 4]));
    assert!(!store.read(3, &mut [0; 8]));
    assert!(!store.read(4, &mut []));
    assert!(store.read(1, &mut [0; 4]));
}

#[test]
fn insufficient_space_for_one_record_still_allows_a_shorter_record() {
    let mut store = GlyphStore::new(19).unwrap();
    let window = store.window(0, vec![10, 20, 30]).unwrap();
    assert!(!window.can_write(10, 11));
    assert!(!window.can_write(30, 1));
    window.write(10, &[7; 11]);
    window.write(20, b"ok");
    store.append(&window);
    assert!(!store.read(0, &mut [0; 11]));
    assert!(store.read(1, &mut [0; 2]));
}

#[test]
fn temporary_files_are_optional_and_a_disabled_store_opens_none() {
    assert!(GlyphStore::with_files(0, || panic!("opened a file")).is_none());
    for failure in 0..2 {
        let mut calls = 0;
        assert!(
            GlyphStore::with_files(1024, || {
                calls += 1;
                if calls == failure + 1 {
                    Err(io::Error::other("tempfile unavailable"))
                } else {
                    tempfile::tempfile()
                }
            })
            .is_none()
        );
        assert_eq!(calls, failure + 1);
    }
}

#[test]
fn failed_flush_of_either_file_preserves_previous_commits() {
    for fail_index in [true, false] {
        let mut store = GlyphStore::new(1024).unwrap();
        let first = store.window(0, vec![10]).unwrap();
        first.write(10, b"old");
        store.append(&first);

        let stream = if fail_index {
            &mut store.index
        } else {
            &mut store.data
        };
        let mut replacement = tempfile::NamedTempFile::new().unwrap();
        stream.file.rewind().unwrap();
        io::copy(&mut stream.file, &mut replacement).unwrap();
        *stream = Stream::new(File::open(replacement.path()).unwrap());
        let next = store.window(1, vec![20]).unwrap();
        next.write(20, b"new");
        store.append(&next);
        assert!(!store.can_record_from(2));
        assert!(!store.read(1, &mut [0; 3]));
        let mut bytes = [0; 3];
        assert!(store.read(0, &mut bytes));
        assert_eq!(&bytes, b"old");
    }
}

#[test]
fn debug_output_has_constant_size_for_large_windows() {
    let store = GlyphStore::new(1_000_000).unwrap();
    let window = store.window(100, (0..10_000).collect()).unwrap();
    window.write(1, &[42; MeasuredTableCell::MAX_RECORD_BYTES]);
    assert_eq!(
        format!("{window:?}"),
        "WindowRecorder { first: 100, cells: 10000, .. }"
    );
}

#[test]
fn empty_windows_do_not_extend_the_index_and_late_ordinals_skip_capture() {
    let mut store = GlyphStore::new(32).unwrap();
    let empty = store.window(0, vec![usize::MAX]).unwrap();
    store.append(&empty);
    assert_eq!(store.index_end, 0);
    assert!(!store.can_record_from(3));
    assert!(store.window(3, vec![0]).is_none());
    assert!(store.window(0, Vec::new()).is_none());
    assert!(store.can_record_from(2));
    let overlap = store.window(2, vec![12]).unwrap();
    overlap.write(12, b"z");
    let last = store.window(2, vec![999]).unwrap();
    last.write(999, b"x");
    store.append(&last);
    assert!(!last.can_write(999, 1));
    last.write(999, b"y");
    store.append(&overlap);
    store.append(&last);
    let mut bytes = [0];
    assert!(store.read(2, &mut bytes));
    assert_eq!(bytes, *b"x");
}

#[test]
fn truncated_data_and_invalid_offsets_leave_intact_records_readable() {
    let mut store = GlyphStore::new(1024).unwrap();
    let window = store.window(0, vec![1, 2, 3]).unwrap();
    window.write(1, b"one");
    window.write(2, b"two");
    window.write(3, b"end");
    store.append(&window);
    store
        .index
        .writer(SLOT)
        .unwrap()
        .write_all(&99u64.to_le_bytes())
        .unwrap();
    store
        .data
        .file
        .get_ref()
        .set_len(store.data_end - 1)
        .unwrap();
    let mut bytes = [0; 3];
    assert!(!store.read(2, &mut bytes));
    assert!(!store.read(1, &mut bytes));
    assert!(store.read(0, &mut bytes));
    assert_eq!(&bytes, b"one");
}

#[test]
fn a_poisoned_window_leaves_other_workers_records_available() {
    let mut store = GlyphStore::new(128).unwrap();
    let failed = store.window(0, vec![10]).unwrap();
    let good = store.window(1, vec![20]).unwrap();
    let worker = Arc::clone(&failed);
    assert!(
        std::thread::spawn(move || {
            let _batch = worker.batch.lock().unwrap();
            panic!("measurement worker interrupted")
        })
        .join()
        .is_err()
    );
    assert!(!failed.can_write(10, 1));
    failed.write(10, b"x");
    good.write(20, b"y");
    store.append(&failed);
    store.append(&good);
    let mut byte = [0];
    assert!(!store.read(0, &mut byte));
    assert!(store.read(1, &mut byte));
    assert_eq!(byte, *b"y");
}
