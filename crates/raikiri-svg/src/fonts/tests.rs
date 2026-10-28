use super::*;

thread_local! {
    static ENTRY_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_entry_visit() {
    ENTRY_WORK.with(|work| work.set(work.get() + 1));
}

struct NoSource;
impl SvgFontSource for NoSource {
    fn select(&self, _: &SvgFontRequest) -> Option<FontFace> {
        None
    }
    fn fallback(&self, _: &SvgFontRequest, _: char) -> Option<FontFace> {
        None
    }
}

fn ahem() -> FontFace {
    FontFace::new(
        Arc::new(include_bytes!("../../tests/data/Ahem.ttf").as_slice()),
        0,
    )
}
fn request_for(i: usize) -> SvgFontRequest {
    SvgFontRequest::new(vec![SvgFontFamily::Named(format!("family{i}"))])
}

#[test]
fn registering_shared_faces_with_distinct_requests_has_linear_entry_work() {
    for n in [32, 64, 128] {
        let faces = Faces::new(&NoSource);
        let face = ahem();
        let mut database = Arc::new(fontdb::Database::new());
        ENTRY_WORK.with(|work| work.set(0));
        for i in 0..n {
            faces
                .load(face.clone(), Cow::Borrowed(&request_for(i)), &mut database)
                .unwrap();
        }
        let work = ENTRY_WORK.with(std::cell::Cell::get);
        assert!(work <= 4 * n, "n={n}, work={work}");
    }
}

#[test]
fn projecting_database_ids_has_linear_entry_work() {
    for n in [32, 64, 128] {
        let faces = Faces::new(&NoSource);
        let face = ahem();
        let mut database = Arc::new(fontdb::Database::new());
        let ids: Vec<_> = (0..n)
            .map(|i| {
                faces
                    .load(face.clone(), Cow::Borrowed(&request_for(i)), &mut database)
                    .unwrap()
            })
            .collect();
        let map = faces.into_map();
        ENTRY_WORK.with(|work| work.set(0));
        for id in ids {
            assert!(map.get(id).unwrap().is_same_face(&face));
            assert!(!map.has_opsz(id));
        }
        let work = ENTRY_WORK.with(std::cell::Cell::get);
        assert!(work <= 4 * n, "n={n}, work={work}");
    }
}

#[test]
fn request_ids_stay_separate_and_share_the_loaded_font_bytes() {
    let faces = Faces::new(&NoSource);
    let face = ahem();
    let mut database = Arc::new(fontdb::Database::new());
    let a = faces
        .load(face.clone(), Cow::Owned(request_for(0)), &mut database)
        .unwrap();
    let b = faces
        .load(face.clone(), Cow::Owned(request_for(1)), &mut database)
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(
        faces.load(face.clone(), Cow::Borrowed(&request_for(0)), &mut database),
        Some(a)
    );
    assert_eq!(database.faces().count(), 2);
    for id in [a, b] {
        assert_eq!(
            database.with_face_data(id, |bytes, index| (bytes.as_ptr(), index)),
            Some((face.bytes().as_ptr(), face.index()))
        );
    }
    let entries = faces.entries();
    assert_eq!(entries.list.len(), 2);
    assert_eq!(entries.contexts.len(), 2);
    assert_eq!(entries.faces.len(), 1);
    assert_eq!(entries.ids.len(), 2);
    assert!(
        entries
            .list
            .iter()
            .all(|entry| Arc::strong_count(&entry.request) == 2)
    );
}

#[test]
fn owned_requests_move_and_equivalent_byte_allocations_are_distinct_faces() {
    let faces = Faces::new(&NoSource);
    let face = ahem();
    let copy = FontFace::new(Arc::new(face.bytes().to_vec()), 0);
    assert_eq!(face, copy);
    let request = request_for(0);
    let pointer = request.families.as_ptr();
    let mut database = Arc::new(fontdb::Database::new());
    let a = faces
        .load(face.clone(), Cow::Owned(request), &mut database)
        .unwrap();
    assert_eq!(faces.entries().list[0].request.families.as_ptr(), pointer);
    let b = faces
        .load(copy.clone(), Cow::Borrowed(&request_for(0)), &mut database)
        .unwrap();
    assert_ne!(a, b);
    let map = faces.into_map();
    assert!(map.get(a).unwrap().is_same_face(&face));
    assert!(map.get(b).unwrap().is_same_face(&copy));
    assert_eq!(map.get(fontdb::ID::dummy()), None);
    assert!(!map.has_opsz(fontdb::ID::dummy()));
}

#[test]
fn failed_load_does_not_publish_indexes_and_poison_recovery_keeps_them_consistent() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    struct Data {
        bytes: Vec<u8>,
        panic_next: AtomicBool,
    }
    impl AsRef<[u8]> for Data {
        fn as_ref(&self) -> &[u8] {
            assert!(!self.panic_next.swap(false, SeqCst), "test font-data panic");
            &self.bytes
        }
    }
    let data = Arc::new(Data {
        bytes: include_bytes!("../../tests/data/Ahem.ttf").to_vec(),
        panic_next: AtomicBool::new(true),
    });
    let face = FontFace::new(data, 0);
    let faces = Faces::new(&NoSource);
    let mut database = Arc::new(fontdb::Database::new());
    let request = request_for(0);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| faces.load(
            face.clone(),
            Cow::Borrowed(&request),
            &mut database
        )))
        .is_err()
    );
    assert!(faces.entries.is_poisoned());
    {
        let entries = faces.entries();
        assert!(
            entries.list.is_empty()
                && entries.contexts.is_empty()
                && entries.faces.is_empty()
                && entries.ids.is_empty()
        );
    }
    let id = faces
        .load(face.clone(), Cow::Borrowed(&request), &mut database)
        .unwrap();
    assert!(faces.into_map().get(id).unwrap().is_same_face(&face));
    let empty = Faces::new(&NoSource);
    let invalid = FontFace::new(Arc::new(Vec::<u8>::new()), 0);
    assert_eq!(
        empty.load(invalid, Cow::Borrowed(&request), &mut database),
        None
    );
    assert!(empty.entries().ids.is_empty());
}

#[test]
fn cached_borrowed_and_owned_contexts_preserve_storage_and_request_ownership() {
    let faces = Faces::new(&NoSource);
    let face = ahem();
    let request = request_for(0);
    let mut database = Arc::new(fontdb::Database::new());
    let id = faces
        .load(face.clone(), Cow::Borrowed(&request), &mut database)
        .unwrap();
    let sizes = {
        let entries = faces.entries();
        (
            entries.list.capacity(),
            entries.contexts.capacity(),
            entries.faces.capacity(),
            entries.ids.capacity(),
        )
    };
    for owned in [false, true] {
        for _ in 0..128 {
            let input = if owned {
                Cow::Owned(request.clone())
            } else {
                Cow::Borrowed(&request)
            };
            assert_eq!(faces.load(face.clone(), input, &mut database), Some(id));
        }
    }
    let entries = faces.entries();
    assert_eq!(entries.list.len(), 1);
    assert_eq!(Arc::strong_count(&entries.list[0].request), 2);
    assert_eq!(
        sizes,
        (
            entries.list.capacity(),
            entries.contexts.capacity(),
            entries.faces.capacity(),
            entries.ids.capacity()
        )
    );
}

fn ttc(fonts: &[&[u8]]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(fonts.len() as u32).to_be_bytes());
    let mut start = header;
    for font in fonts {
        out.extend_from_slice(&(start as u32).to_be_bytes());
        start += font.len().next_multiple_of(4);
    }
    let mut start = header as u32;
    for font in fonts {
        let mut moved = font.to_vec();
        let tables = usize::from(u16::from_be_bytes([moved[4], moved[5]]));
        for table in 0..tables {
            let at = 12 + 16 * table + 8;
            let offset = u32::from_be_bytes(moved[at..at + 4].try_into().unwrap());
            moved[at..at + 4].copy_from_slice(&(offset + start).to_be_bytes());
        }
        moved.resize(moved.len().next_multiple_of(4), 0);
        start += moved.len() as u32;
        out.extend_from_slice(&moved);
    }
    out
}

#[test]
fn collection_face_indices_keep_separate_ids_and_correct_mapped_faces() {
    let bytes = ttc(&[
        include_bytes!("../../tests/data/Ahem.ttf"),
        include_bytes!("../../tests/data/CanvasTest-nospace.ttf"),
    ]);
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
    let first = FontFace::new(Arc::clone(&data), 0);
    let second = FontFace::new(Arc::clone(&data), 1);
    let faces = Faces::new(&NoSource);
    let request = request_for(0);
    let mut database = Arc::new(fontdb::Database::new());
    let a = faces
        .load(first.clone(), Cow::Borrowed(&request), &mut database)
        .unwrap();
    let b = faces
        .load(second.clone(), Cow::Borrowed(&request), &mut database)
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(database.face(a).unwrap().index, 0);
    assert_eq!(database.face(b).unwrap().index, 1);
    assert_eq!(
        faces.load(first.clone(), Cow::Borrowed(&request), &mut database),
        Some(a)
    );
    assert_eq!(
        faces.load(second.clone(), Cow::Borrowed(&request), &mut database),
        Some(b)
    );
    let map = faces.into_map();
    assert!(map.get(a).unwrap().is_same_face(&first));
    assert!(map.get(b).unwrap().is_same_face(&second));
}
