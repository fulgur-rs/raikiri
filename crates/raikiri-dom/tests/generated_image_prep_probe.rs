//! Manual public-API timing probe for generated image preparation.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

use raikiri_dom::Document;
use raikiri_style::property::ContentComponent;
use raikiri_style::{
    ComputedValues, PseudoElem, PseudoValueArc, StyleNodeId, build_rule_tree, cascade,
};
use raikiri_traits::{DecodedImage, ImagePixelSource};

struct Pixels {
    image: Arc<DecodedImage>,
    requests: Cell<usize>,
}

impl ImagePixelSource for Pixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        self.requests.set(self.requests.get() + 1);
        Some(self.image.clone())
    }
}

#[test]
#[ignore = "manual generated-image preparation probe; configure IMAGE_PREP_DEPTH/MODE/REPEATS"]
fn generated_image_preparation_probe() {
    let number = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(default)
    };
    let depth = number("IMAGE_PREP_DEPTH", 16);
    let repeats = number("IMAGE_PREP_REPEATS", 1);
    let mode = std::env::var("IMAGE_PREP_MODE").unwrap_or_else(|_| "text".into());
    let content = match mode.as_str() {
        "text" | "none" => ContentComponent::Literal("x".into()),
        "image" => ContentComponent::Image {
            url: "https://example.test/pixel.png".into(),
        },
        _ => panic!("unsupported preparation probe mode"),
    };
    let mut document = Document::new();
    let mut parent = document.root_index();
    let mut elements = Vec::with_capacity(depth);
    for _ in 0..depth {
        parent = document.append_element(Some(parent), "div", None::<&str>);
        elements.push(parent);
    }
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let mut computed = cascade(&document, &rules).unwrap();
    let mut pseudo = ComputedValues::initial();
    pseudo.content = Arc::new(vec![content].into());
    let pseudo = PseudoValueArc::new(pseudo);
    if mode != "none" {
        for &element in &elements {
            computed.pseudo.insert(
                (StyleNodeId::new(element as u64), PseudoElem::Before),
                pseudo.clone(),
            );
        }
    }
    let source = Pixels {
        image: Arc::new(DecodedImage::new(2, 2, [17, 34, 51, 255].repeat(4))),
        requests: Cell::new(0),
    };
    let started = Instant::now();
    for _ in 0..repeats {
        document.prepare_list_marker_images(&computed, &source, None);
    }
    let elapsed_ns = started.elapsed().as_nanos();
    let mut prepared = 0;
    for &element in &elements {
        if let Some(image) = document.content_image(element, PseudoElem::Before, 0) {
            assert_eq!((image.width, image.height), (2, 2));
            assert_eq!(image.rgba, [17, 34, 51, 255].repeat(4));
            prepared += 1;
        }
    }
    assert_eq!(prepared, if mode == "image" { depth } else { 0 });
    if mode != "image" {
        assert_eq!(source.requests.get(), 0);
    }
    println!(
        "IMAGE_PREP {}",
        serde_json::json!({
            "depth": depth,
            "mode": mode,
            "repeats": repeats,
            "elapsed_ns": elapsed_ns,
            "prepared": prepared,
            "requests": source.requests.get(),
            "nodes": document.node_count(),
        })
    );
}
