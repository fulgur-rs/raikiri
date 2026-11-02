use super::*;
use raikiri_traits::{FetchOutcome, FetchedResource, NetworkError, Request};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Provider(Arc<AtomicUsize>);
impl NetworkProvider for Provider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(FetchOutcome::Body(
            FetchedResource::new(TINY_PNG.to_vec().into(), request.url)
                .content_type(Some("image/png".into())),
        ))
    }
}

#[test]
fn every_original_fragment_url_reaches_shared_pixels_and_metadata() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = ImageResolver::new(Provider(calls.clone()));
    let mut first = None;
    let size = ImageRasterSize {
        width: 2.0,
        height: 1.0,
    };
    let vector = ImageVectorRequest::new(size);
    for suffix in ["#one", "#two", "#", ""] {
        let url: Url = format!("https://images.test/a.png{suffix}")
            .parse()
            .unwrap();
        let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
        assert_eq!(
            (resolved.intrinsic.width, resolved.intrinsic.height),
            (2.0, 1.0)
        );
        let image = resolver.get_decoded(&url).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        if let Some(first) = &first {
            assert!(Arc::ptr_eq(first, &image));
        } else {
            first = Some(image.clone());
        }
        assert_eq!(resolver.decoded_byte_len(&url), Some(8));
        let intrinsic = resolver.intrinsic_size(&url).unwrap();
        assert_eq!((intrinsic.width, intrinsic.height), (Some(2.0), Some(1.0)));
        assert!(Arc::ptr_eq(
            &image,
            &resolver.get_decoded_at_size(&url, size, Some(8)).unwrap()
        ));
        assert!(resolver.get_decoded_at_size(&url, size, Some(7)).is_none());
        assert!(resolver.get_vector(&url, &vector).is_none());
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
