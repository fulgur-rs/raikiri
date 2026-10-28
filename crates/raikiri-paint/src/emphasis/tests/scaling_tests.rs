use super::super::*;

#[test]
#[ignore = "manual release-mode vertical emphasis placement measurement"]
fn vertical_emphasis_placement_probe() {
    use std::hint::black_box;
    use std::time::Instant;
    let mut document = Document::new();
    document.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    for n in [1_000, 2_000, 4_000, 8_000] {
        let text = "a".repeat(n);
        let style = raikiri_dom::StandaloneStyle {
            writing_mode: WritingMode::VerticalRl,
            ..Default::default()
        };
        let shaped = document
            .shape_standalone_text(&text, &style, None, StandaloneAlign::Start)
            .unwrap();
        assert_eq!(shaped.lines().len(), 1);
        let line = &shaped.lines()[0];
        let runs: Vec<_> = line
            .fragments()
            .filter_map(|f| {
                if let Fragment::GlyphRun(run) = f {
                    Some(run)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.glyphs().count(), n);
        assert_eq!(run.clusters().count(), n);
        let converter = PhysicalConverter::new(
            line.writing_mode(),
            line.used_direction(),
            shaped.container(),
        );
        let mark = shodo::EmphasisMark {
            character: '•',
            font_size: 8.0,
            line_over: true,
            offset: 8.0,
        };
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let points =
                vertical_mark_centers(&mark, (line, run), (WritingMode::VerticalRl, &converter));
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(points.len(), n);
            black_box(points);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "vertical emphasis placement n={n} median_ms={:.6} samples={samples:?}",
            samples[2]
        );
    }
}
