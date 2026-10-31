//! Public scene allocation guards for repeated image URL references.
#[path = "support/allocation_tests.rs"]
mod support;
use raikiri_svg::{
    SvgDocument, SvgImageAccess, SvgImageSource, SvgResources, SvgRootStyle, SvgViewport,
};
use raikiri_traits::{
    DecodedImage,
    vector::{Group, Node},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;
struct Pictures {
    href: String,
    pixel: Arc<DecodedImage>,
    calls: AtomicUsize,
}
impl SvgImageSource for Pictures {
    fn url(&self, href: &str) -> Option<Arc<DecodedImage>> {
        assert_eq!(href, self.href);
        self.calls.fetch_add(1, Ordering::Relaxed);
        Some(Arc::clone(&self.pixel))
    }
    fn data(&self, _: &str, _: &[u8]) -> Option<Arc<DecodedImage>> {
        panic!("unexpected data image")
    }
}
fn validate(group: &Group, pixel: &Arc<DecodedImage>) -> usize {
    let mut count = 0;
    for child in &group.children {
        match child {
            Node::Group(group) => count += validate(group, pixel),
            Node::Image(image) => {
                assert!(Arc::ptr_eq(&image.image, pixel));
                count += 1;
            }
            _ => {}
        }
    }
    count
}
#[test]
fn repeated_image_url_keys_are_copied_only_for_the_first_request() {
    let mut overheads = Vec::new();
    for count in [64, 512, 2048] {
        let mut totals = Vec::new();
        for length in [64, 65536] {
            let href = format!("https://image.test/{}.png", "x".repeat(length));
            let mut source = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='40' height='20'><defs><image id='i' href='{href}' width='1' height='1'/></defs>"
            );
            for _ in 0..count {
                source.push_str("<use href='#i'/>");
            }
            source.push_str("</svg>");
            let document =
                SvgDocument::parse_with_images(source.as_bytes(), SvgImageAccess::Any).unwrap();
            let pictures = Pictures {
                href,
                pixel: Arc::new(DecodedImage::new(1, 1, vec![255, 0, 0, 255])),
                calls: AtomicUsize::new(0),
            };
            let (scene, allocation) = support::allocated_by(|| {
                document
                    .scene_with_resources(
                        SvgViewport {
                            width: 40.0,
                            height: 20.0,
                        },
                        SvgRootStyle::default(),
                        None,
                        SvgResources::new().with_images(&pictures),
                    )
                    .unwrap()
            });
            assert_eq!(pictures.calls.load(Ordering::Relaxed), 1);
            assert_eq!(validate(&scene.root, &pictures.pixel), count);
            eprintln!("image URL: uses={count}, length={length}, {allocation:?}");
            totals.push(allocation.bytes);
        }
        overheads.push((count, totals[1].saturating_sub(totals[0])));
    }
    for (count, overhead) in overheads {
        assert!(
            overhead <= 4 * (65536 - 64),
            "uses={count}, URL length overhead={overhead}"
        );
    }
}
