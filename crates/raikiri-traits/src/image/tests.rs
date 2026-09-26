use super::*;

struct EmptySource;
impl ImagePixelSource for EmptySource {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

#[test]
fn image_pixel_source_is_object_safe() {
    fn _assert<T: ?Sized>() {}
    _assert::<dyn ImagePixelSource>();
}

#[test]
fn empty_source_returns_none() {
    let url = Url::parse("file:///tmp/x.png").unwrap();
    assert!(EmptySource.get_decoded(&url).is_none());
}
