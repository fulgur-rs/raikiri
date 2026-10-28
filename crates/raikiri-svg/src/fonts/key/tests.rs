use super::super::SvgFontStyle;
use super::*;

fn hash(key: &ContextKey<'_>) -> u64 {
    use std::hash::DefaultHasher;
    let mut state = DefaultHasher::new();
    key.hash(&mut state);
    state.finish()
}

#[test]
fn borrowed_and_shared_keys_have_identical_content_hashes_and_equality() {
    let face = FontFace::new(Arc::new(&b"data"[..]), 0);
    for style in [
        SvgFontStyle::Normal,
        SvgFontStyle::Italic,
        SvgFontStyle::Oblique,
    ] {
        for stretch in [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0] {
            let mut request = SvgFontRequest::new(vec![
                SvgFontFamily::Named("Alpha".into()),
                SvgFontFamily::Serif,
            ]);
            request.style = style;
            request.stretch = stretch;
            let shared = Arc::new(request.clone());
            let owned = ContextKey::shared(FaceIdentity::of(&face), shared);
            let borrowed = ContextKey::borrowed(FaceIdentity::of(&face), &request);
            assert!(owned == borrowed);
            assert_eq!(hash(&owned), hash(&borrowed));
        }
    }
}

#[test]
fn every_request_component_and_face_identity_remains_part_of_the_key() {
    let face = FontFace::new(Arc::new(vec![1_u8, 2, 3]), 0);
    let copy = FontFace::new(Arc::new(vec![1_u8, 2, 3]), 0);
    assert_eq!(face, copy);
    assert!(!face.is_same_face(&copy));
    let base = SvgFontRequest::new(vec![
        SvgFontFamily::Named("serif".into()),
        SvgFontFamily::Serif,
    ]);
    let key = ContextKey::shared(FaceIdentity::of(&face), Arc::new(base.clone()));
    let mut changed = Vec::new();
    let mut request = base.clone();
    request.families.reverse();
    changed.push(request);
    let mut request = base.clone();
    request.families[0] = SvgFontFamily::Named("Other".into());
    changed.push(request);
    let mut request = base.clone();
    request.families[0] = SvgFontFamily::Serif;
    changed.push(request);
    let mut request = base.clone();
    request.weight = 700;
    changed.push(request);
    let mut request = base.clone();
    request.stretch = 75.0;
    changed.push(request);
    let mut request = base.clone();
    request.style = SvgFontStyle::Italic;
    changed.push(request);
    let mut map = std::collections::HashMap::new();
    map.insert(key, 3);
    assert_eq!(
        map.get(&ContextKey::borrowed(FaceIdentity::of(&face), &base)),
        Some(&3)
    );
    for request in changed {
        let query = ContextKey::borrowed(FaceIdentity::of(&face), &request);
        assert!(map.keys().next().unwrap() != &query);
        assert_eq!(map.get(&query), None);
    }
    assert_eq!(
        map.get(&ContextKey::borrowed(FaceIdentity::of(&copy), &base)),
        None
    );
    let second = FontFace::new(Arc::clone(face.data()), 1);
    assert_eq!(
        map.get(&ContextKey::borrowed(FaceIdentity::of(&second), &base)),
        None
    );
}
