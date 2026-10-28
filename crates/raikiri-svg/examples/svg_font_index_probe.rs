//! Manual release-mode scene/font-index measurement. Build first and run
//! the binary on a fixed CPU without concurrent builds.

use raikiri_svg::{
    SvgDocument, SvgFontFamily, SvgFontRequest, SvgFontSource, SvgResources, SvgRootStyle,
    SvgViewport,
};
use raikiri_traits::vector::{FontFace, Group, Node};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};
use std::time::Instant;

struct Fonts {
    faces: Vec<FontFace>,
    queries: AtomicUsize,
    fallbacks: AtomicUsize,
}
impl SvgFontSource for Fonts {
    fn select(&self, request: &SvgFontRequest) -> Option<FontFace> {
        self.queries.fetch_add(1, Relaxed);
        let index = if self.faces.len() == 1 {
            0
        } else {
            request
                .families
                .iter()
                .find_map(|family| {
                    if let SvgFontFamily::Named(name) = family {
                        name.strip_prefix('f')?.parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        Some(self.faces[index].clone())
    }
    fn fallback(&self, _: &SvgFontRequest, _: char) -> Option<FontFace> {
        self.fallbacks.fetch_add(1, Relaxed);
        None
    }
}
fn glyph_faces<'a>(group: &'a Group, out: &mut Vec<&'a FontFace>) {
    for node in &group.children {
        match node {
            Node::Group(group) => glyph_faces(group, out),
            Node::Text(text) => out.extend(text.glyphs.iter().map(|glyph| &glyph.font)),
            _ => {}
        }
    }
}
fn main() {
    let bytes = include_bytes!("../tests/data/Ahem.ttf");
    for mode in ["shared_request", "unique_request", "distinct_faces"] {
        for n in [500, 1_000, 2_000, 4_000] {
            let face_count = if mode == "distinct_faces" { n } else { 1 };
            let fonts = Fonts {
                faces: (0..face_count)
                    .map(|_| FontFace::new(Arc::new(bytes.to_vec()), 0))
                    .collect(),
                queries: AtomicUsize::new(0),
                fallbacks: AtomicUsize::new(0),
            };
            let mut xml =
                String::from("<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>");
            for i in 0..n {
                xml += &format!(
                    "<text y='10' font-family='f{}'>A</text>",
                    if mode == "shared_request" { 0 } else { i }
                );
            }
            xml += "</svg>";
            let document = SvgDocument::parse(xml.as_bytes()).unwrap();
            let mut samples = Vec::new();
            for _ in 0..5 {
                fonts.queries.store(0, Relaxed);
                fonts.fallbacks.store(0, Relaxed);
                let start = Instant::now();
                let scene = document
                    .scene_with_resources(
                        SvgViewport {
                            width: 100.0,
                            height: 100.0,
                        },
                        SvgRootStyle::default(),
                        None,
                        SvgResources::new().with_fonts(&fonts),
                    )
                    .unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                let mut actual = Vec::new();
                glyph_faces(&scene.root, &mut actual);
                assert_eq!(actual.len(), n);
                assert_eq!(fonts.queries.load(Relaxed), n);
                assert_eq!(fonts.fallbacks.load(Relaxed), 0);
                for (i, face) in actual.iter().enumerate() {
                    assert!(face.is_same_face(&fonts.faces[if face_count == 1 { 0 } else { i }]));
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "svg font mode={mode} n={n} select_queries={n} median_ms={:.6} samples={samples:?}",
                samples[2]
            );
        }
    }
}
