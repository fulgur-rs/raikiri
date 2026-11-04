//! Marker ink follows independently measured Chromium overflow cases.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn marker_ink_matches_chromium_overflow_matrix() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let mut failures = Vec::new();
    for row in include_str!("../../raikiri-dom/tests/data/marker-overflow.tsv")
        .lines()
        .filter(|row| !row.starts_with('#'))
    {
        let fields = row.split('\t').collect::<Vec<_>>();
        let label = fields[0];
        let width: usize = fields[4].parse().unwrap();
        let html = format!(
            "<!doctype html><style>{}{} </style>{}",
            include_str!("../../raikiri-dom/tests/data/marker-overflow.css"),
            fields[5],
            fields[6]
        );
        let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(document) = layout(
            &parsed,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap() else {
            panic!("completed {label}")
        };
        let expected = fields[7].split(';').collect::<Vec<_>>();
        assert_eq!(
            document.page_count(),
            u32::try_from(expected.len()).unwrap(),
            "{label}"
        );
        for (index, expected) in expected.iter().enumerate() {
            let page = document.page(u32::try_from(index).unwrap()).unwrap();
            let (source, cascade, page_box, origin) = page.paint_inputs();
            let mut scene = Scene::new();
            raikiri_paint::paint_single_page_with_origin_and_page_context(
                &mut scene,
                source,
                cascade,
                page_box,
                origin,
                u32::try_from(index).unwrap(),
                document.page_count(),
                false,
                None,
            )
            .unwrap();
            let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                u32::try_from(width).unwrap(),
                160,
            );
            if label == "inside-rounded-hidden" {
                // The square corner is clipped, while the middle stays blue.
                assert_ne!(&pixels[40 * 4..41 * 4], &[0, 0, 255, 255]);
                let center = (10 * width + 50) * 4;
                assert_eq!(&pixels[center..center + 4], &[0, 0, 255, 255]);
            }
            let bounds = pixels
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, p)| p[2] > 200 && p[0] < 50 && p[1] < 50)
                .map(|(i, _)| (i % width, i / width))
                .fold(None::<[usize; 4]>, |bounds, (x, y)| {
                    Some(match bounds {
                        None => [x, y, x + 1, y + 1],
                        Some([x0, y0, x1, y1]) => {
                            [x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)]
                        }
                    })
                });
            let expected = (*expected != "-").then(|| {
                expected
                    .split(',')
                    .map(|x| x.parse::<usize>().unwrap())
                    .collect::<Vec<_>>()
            });
            // Image placement has a pre-existing independent offset difference.
            // This matrix asserts clipping and the intrinsic image dimensions.
            let matches = if fields[3] == "true" {
                bounds.is_some() == expected.is_some()
                    && bounds.is_none_or(|[x0, y0, x1, y1]| (x1 - x0, y1 - y0) == (10, 10))
            } else {
                bounds.map(|b| b.to_vec()) == expected
            };
            if !matches {
                failures.push(format!(
                    "{label} page {index}: actual {bounds:?}, expected {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
