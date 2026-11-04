//! Public marker events retain text while carrying the owner's overflow clip.

use raikiri_html::{
    ClipKind, FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, RunSource, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn public_marker_events_carry_owner_overflow_clips() {
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
        let html = format!(
            "<!doctype html><style>{}{}</style>{}",
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
        let mut marker_count = 0;
        for index in 0..document.page_count() {
            let page = document.page(index).unwrap();
            let runs = page.text_runs();
            let marker_lines = runs
                .iter()
                .filter(|run| matches!(run.source, RunSource::Generated(_, GeneratedKind::Marker)))
                .map(|run| run.line)
                .collect::<Vec<_>>();
            let mut clips = Vec::new();
            let mut transforms = 0;
            for event in page.paint_order_for_text_runs(&runs) {
                let is_marker = match event {
                    PaintEvent::PushClip(clip, kind) => {
                        clips.push((clip, kind));
                        false
                    }
                    PaintEvent::PopClip => {
                        assert!(clips.pop().is_some(), "{label}: unbalanced pop");
                        false
                    }
                    PaintEvent::MarkerImage(_) => true,
                    PaintEvent::PushTransform(_) => {
                        transforms += 1;
                        false
                    }
                    PaintEvent::PopTransform => {
                        assert!(transforms > 0, "{label}: unbalanced transform pop");
                        transforms -= 1;
                        false
                    }
                    // Inside image markers also expose a space-only reservation line.
                    PaintEvent::TextLine(line) => {
                        fields[3] != "true" && marker_lines.contains(&line)
                    }
                    _ => false,
                };
                if is_marker {
                    if label.starts_with("next-page-") {
                        assert_eq!(index, 1, "{label}: marker page");
                    }
                    marker_count += 1;
                    let owner_clips = clips
                        .iter()
                        .filter(|(_, kind)| *kind == ClipKind::Overflow)
                        .collect::<Vec<_>>();
                    let actual = owner_clips
                        .iter()
                        .map(|(clip, _)| (clip.clip_x, clip.clip_y))
                        .collect::<Vec<_>>();
                    let expected = match fields[8] {
                        "xy" => vec![(true, true)],
                        "-y" => vec![(false, true)],
                        "--" => vec![],
                        _ => panic!("axes"),
                    };
                    if actual != expected {
                        failures.push(format!(
                            "{label} page {index}: clips {actual:?}, expected {expected:?}"
                        ));
                    }
                    if label == "ordinary-outside-hidden"
                        && let Some((clip, _)) = owner_clips.first()
                    {
                        assert_eq!(
                            (clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height),
                            (40.0, 0.0, 100.0, 40.0)
                        );
                    }
                    if label.contains("clip-margin-5")
                        && let Some((clip, _)) = owner_clips.first()
                    {
                        assert_eq!(
                            (clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height),
                            (35.0, -5.0, 110.0, 50.0)
                        );
                    }
                    if label == "inside-rounded-hidden" {
                        assert!(owner_clips[0].0.corner_radii.is_some());
                    }
                }
            }
            assert!(clips.is_empty(), "{label}: unbalanced push");
            assert_eq!(transforms, 0, "{label}: unbalanced transform push");
        }
        let expected_count = usize::from(fields[9] == "true");
        if marker_count != expected_count {
            failures.push(format!(
                "{label}: marker count {marker_count}, expected {expected_count}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
