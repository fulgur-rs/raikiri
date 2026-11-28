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
fn exceeding_either_budget_stops_writes_and_preserves_commits() {
    for (id, len) in [(1, 48), (2, 0), (usize::MAX, 0)] {
        let store = Store::new(
            tempfile::tempfile().unwrap(),
            tempfile::tempfile().unwrap(),
            64,
        );
        store.write(0, b"a");
        store.write(id, &vec![0; len]);
        assert!(!store.can_write());
        store.write(1, b"b");
        let mut byte = [0];
        assert!(store.read(0, &mut byte));
        assert_eq!(&byte, b"a");
        assert!(!store.read(1, &mut byte));
    }
}

#[test]
fn partial_writes_leave_other_commits_readable() {
    for failing_call in 0..3 {
        let store = Store::new(
            tempfile::tempfile().unwrap(),
            tempfile::tempfile().unwrap(),
            1024,
        );
        store.write(0, b"one");
        store.write(1, b"two");
        let mut call = 0;
        store.0.lock().unwrap().write(1, b"six", |file, bytes| {
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
