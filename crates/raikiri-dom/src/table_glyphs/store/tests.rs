use super::*;

#[test]
fn records_require_exact_length_and_a_committed_slot() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        1024,
    );
    store.write(2, b"abc");
    assert!(!store.read(1, &mut [0; 3]));
    assert!(!store.read(2, &mut [0; 2]));
    assert!(!store.read(2, &mut [0; 4]));
    let mut bytes = [0; 3];
    assert!(store.read(2, &mut bytes));
    assert_eq!(&bytes, b"abc");
}

#[test]
fn oversized_records_and_ids_leave_room_for_later_records() {
    for (id, len) in [(1, 48), (2, 0), (usize::MAX, 0)] {
        let store = Store::new(
            tempfile::tempfile().unwrap(),
            tempfile::tempfile().unwrap(),
            64,
        );
        store.write(0, b"a");
        store.write(id, &vec![0; len]);
        assert!(store.can_write(1, 1));
        store.write(1, b"b");
        let mut byte = [0];
        assert!(store.read(0, &mut byte));
        assert_eq!(&byte, b"a");
        assert!(store.read(1, &mut byte));
        assert_eq!(&byte, b"b");
    }
}

#[test]
fn partial_writes_leave_other_commits_readable() {
    for failing_call in 0..2 {
        let store = Store::new(
            tempfile::tempfile().unwrap(),
            tempfile::tempfile().unwrap(),
            1024,
        );
        store.write(0, b"one");
        assert!(store.read(0, &mut [0; 3]));
        store.write(1, b"two");
        let mut call = 0;
        store.0.lock().unwrap().flush(|file, bytes| {
            if call == failing_call {
                file.write_all(&bytes[..1])?;
                return Err(io::Error::other("short write"));
            }
            call += 1;
            file.write_all(bytes)
        });
        store.write(2, b"new");
        let mut bytes = [0; 3];
        assert!(!store.read(1, &mut bytes));
        assert!(!store.read(2, &mut bytes));
        assert!(store.read(0, &mut bytes));
        assert_eq!(&bytes, b"one");
    }
}

#[test]
fn parallel_writers_can_read_each_committed_record() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        4096,
    );
    std::thread::scope(|scope| {
        for group in 0..4 {
            let store = &store;
            scope.spawn(move || {
                for i in 0..32 {
                    let id = i * 4 + group;
                    store.write(id, &id.to_le_bytes());
                    let mut bytes = [0; size_of::<usize>()];
                    assert!(store.read(id, &mut bytes));
                    assert_eq!(usize::from_le_bytes(bytes), id);
                }
            });
        }
    });
}

#[test]
fn exact_disk_limits_accept_the_last_record() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        64,
    );
    store.write(1, &[7; 40]);
    let mut bytes = [0; 40];
    assert!(store.read(1, &mut bytes));
    assert_eq!(bytes, [7; 40]);
    let files = store.0.lock().unwrap();
    assert_eq!(files.data.metadata().unwrap().len(), 48);
    assert_eq!(files.index.metadata().unwrap().len(), 16);
}

#[test]
fn repeated_ids_consume_space_only_once_across_batches() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        64,
    );
    store.write(1, b"first");
    store.write(1, b"other");
    let mut bytes = [0; 5];
    assert!(store.read(1, &mut bytes));
    store.write(1, b"later");
    store.write(0, &[7; 27]);
    assert!(store.read(0, &mut [0; 27]));
    assert!(store.read(1, &mut bytes));
    assert_eq!(&bytes, b"first");
    assert!(!store.can_write(1, 1));
}

#[test]
fn nearby_sparse_slots_share_a_single_index_write() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        32768,
    );
    for id in (0..100).step_by(2).rev() {
        store.write(id, b"x");
    }
    let mut writes = 0;
    store.0.lock().unwrap().flush(|file, bytes| {
        writes += 1;
        file.write_all(bytes)
    });
    assert_eq!(writes, 2);
    for id in 0..100 {
        assert_eq!(store.read(id, &mut [0]), id % 2 == 0);
    }
}

#[test]
fn sequential_reads_reuse_prefetched_data_and_offsets() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        1024,
    );
    store.write(0, b"first");
    store.write(1, b"later");
    let mut bytes = [0; 5];
    assert!(store.read(0, &mut bytes));
    {
        let mut files = store.0.lock().unwrap();
        files.data.rewind().unwrap();
        files.index.rewind().unwrap();
    }
    assert!(store.read(1, &mut bytes));
    assert_eq!(&bytes, b"later");
    let mut files = store.0.lock().unwrap();
    assert_eq!(files.data.stream_position().unwrap(), 0);
    assert_eq!(files.index.stream_position().unwrap(), 0);
}

#[test]
fn truncated_batch_still_returns_complete_earlier_records() {
    for truncate_index in [false, true] {
        let store = Store::new(
            tempfile::tempfile().unwrap(),
            tempfile::tempfile().unwrap(),
            1024,
        );
        store.write(0, b"first");
        store.write(1, b"later");
        {
            let mut files = store.0.lock().unwrap();
            files.flush(File::write_all);
            if truncate_index {
                files.index.set_len(13).unwrap();
            } else {
                files.data.set_len(24).unwrap();
            }
        }
        let mut bytes = [0; 5];
        assert!(store.read(0, &mut bytes));
        assert_eq!(&bytes, b"first");
        assert!(!store.read(1, &mut bytes));
    }
}

#[test]
fn inserting_into_sparse_holes_preserves_older_records_between_them() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        65536,
    );
    store.write(1, b"old");
    store.write(600, b"far");
    let mut bytes = [0; 3];
    assert!(store.read(600, &mut bytes));
    store.write(0, b"new");
    store.write(2, b"new");
    assert!(store.read(2, &mut bytes));
    assert_eq!(&bytes, b"new");
    assert!(store.read(1, &mut bytes));
    assert_eq!(&bytes, b"old");
    assert!(store.read(600, &mut bytes));
    assert_eq!(&bytes, b"far");
}

#[test]
fn interrupted_prefetch_resumes_and_permanent_errors_leave_the_destination_intact() {
    struct Reader {
        data: io::Cursor<[u8; 3]>,
        error: Option<io::ErrorKind>,
    }

    impl Seek for Reader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.data.seek(pos)
        }
    }

    impl Read for Reader {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.data.position() == 1
                && let Some(error) = self.error.take()
            {
                return Err(error.into());
            }
            self.data.read(&mut bytes[..1])
        }
    }

    for error in [io::ErrorKind::Interrupted, io::ErrorKind::Other] {
        let mut reader = Reader {
            data: io::Cursor::new(*b"abc"),
            error: Some(error),
        };
        let mut dest = *b"old";
        let result = ReadBuffer::new().read(&mut reader, 0, &mut dest, 3);
        if error == io::ErrorKind::Interrupted {
            assert!(result.is_some());
            assert_eq!(&dest, b"abc");
        } else {
            assert!(result.is_none());
            assert_eq!(&dest, b"old");
        }
    }
}

#[test]
fn unreadable_duplicate_slot_stops_writes_and_preserves_other_commits() {
    let store = Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        1024,
    );
    store.write(0, b"old");
    store.write(1, b"one");
    {
        let mut files = store.0.lock().unwrap();
        files.flush(File::write_all);
        files.index.set_len(WORD).unwrap();
    }
    store.write(1, b"new");
    assert!(!store.can_write(2, 3));
    let mut bytes = [0; 3];
    assert!(!store.read(1, &mut bytes));
    assert!(store.read(0, &mut bytes));
    assert_eq!(&bytes, b"old");
}
