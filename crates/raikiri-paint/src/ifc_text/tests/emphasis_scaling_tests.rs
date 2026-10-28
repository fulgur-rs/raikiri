use super::*;

#[test]
#[ignore = "manual release-mode vertical emphasis paint measurement"]
fn vertical_emphasis_paint_probe() {
    use std::hint::black_box;
    use std::time::Instant;
    for n in [1_000, 2_000, 4_000, 8_000] {
        let (mut doc, cascade, root) = paragraph(
            &format!(
                "writing-mode:vertical-rl;white-space:nowrap;height:{}px;width:40px;text-emphasis:dot;text-emphasis-color:rgb(255,0,0)",
                n * 20
            ),
            |doc, root| {
                doc.append_text(root, "a".repeat(n));
            },
        );
        lay_out(&mut doc, &cascade);
        let lines = doc.get_node(root).unwrap().ifc_lines().unwrap();
        assert_eq!(lines.len(), 1);
        let runs: Vec<_> = lines[0]
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
        assert_eq!(runs[0].clusters().count(), n);
        assert_eq!(runs[0].glyphs().count(), n);
        let red = solid(255, 0, 0);
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let scene = painted(&doc, &cascade);
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            let placed = glyphs(&scene);
            assert_eq!(placed.iter().filter(|g| g.3 == red).count(), n);
            assert_eq!(placed.iter().filter(|g| g.3 != red).count(), n);
            black_box(scene);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "vertical emphasis paint n={n} median_ms={:.6} samples={samples:?}",
            samples[2]
        );
    }
}
