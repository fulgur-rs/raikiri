use super::*;

fn horizontal_fixture(n: usize, shadows: usize) -> (Document, raikiri_style::CascadeResult, usize) {
    let shadow = match shadows {
        0 => "none",
        2 => "1px 2px rgb(0,0,255), 3px 4px rgb(0,255,0)",
        _ => unreachable!("fixture supports zero or two shadows"),
    };
    let (mut doc, cascade, root) = paragraph(
        &format!(
            "writing-mode:horizontal-tb;white-space:nowrap;width:{}px;text-emphasis:dot;text-emphasis-color:rgb(255,0,0);text-shadow:{shadow}",
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
        .filter_map(|f| match f {
            Fragment::GlyphRun(run) => Some(run),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].glyphs().count(), n);
    assert_eq!(runs[0].clusters().count(), n);
    assert!(runs[0].emphasis_mark().is_some());
    assert!(
        runs[0]
            .glyphs()
            .all(|glyph| glyph.inline_position.is_finite() && glyph.advance.is_finite())
    );
    (doc, cascade, root)
}

fn check_horizontal_glyphs(scene: &Scene, n: usize, shadows: usize) {
    let placed = glyphs(scene);
    let red = solid(255, 0, 0);
    let black = solid(0, 0, 0);
    assert_eq!(placed.iter().filter(|g| g.3 == red).count(), n);
    assert_eq!(placed.iter().filter(|g| g.3 == black).count(), n);
    if shadows == 2 {
        for color in [solid(0, 0, 255), solid(0, 255, 0)] {
            // Each shadow paints the text glyphs and the emphasis glyphs.
            assert_eq!(placed.iter().filter(|g| g.3 == color).count(), 2 * n);
        }
    }
    assert_eq!(placed.len(), 2 * n * (shadows + 1));
}

#[test]
fn horizontal_emphasis_work_is_subquadratic_with_and_without_shadows() {
    let mut bounded = true;
    for n in [32_usize, 64, 128, 256] {
        for shadows in [0, 2] {
            let (doc, cascade, _) = horizontal_fixture(n, shadows);
            crate::emphasis::reset_glyph_work();
            let scene = painted(&doc, &cascade);
            let work = crate::emphasis::glyph_work();
            check_horizontal_glyphs(&scene, n, shadows);
            eprintln!("horizontal emphasis work n={n} shadows={shadows} work={work}");
            bounded &= work <= 8 * n * (n.ilog2() as usize + 1) * (shadows + 1);
        }
    }
    assert!(
        bounded,
        "horizontal marks must not scan every glyph for each cluster"
    );
}

#[test]
#[ignore = "manual release-mode horizontal emphasis paint measurement"]
fn horizontal_emphasis_paint_probe() {
    use std::hint::black_box;
    use std::time::Instant;
    let reference = std::env::var("RAIKIRI_EMPHASIS_REFERENCE").is_ok_and(|value| value == "1");
    let mode = if reference { "reference" } else { "indexed" };
    let measure = || {
        for n in [1_000_usize, 2_000, 4_000, 8_000] {
            for shadows in [0, 2] {
                let (doc, cascade, _) = horizontal_fixture(n, shadows);
                let mut samples = Vec::new();
                let mut work = 0;
                for _ in 0..5 {
                    crate::emphasis::reset_glyph_work();
                    let start = Instant::now();
                    let scene = painted(&doc, &cascade);
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    work = crate::emphasis::glyph_work();
                    check_horizontal_glyphs(&scene, n, shadows);
                    black_box(scene);
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "horizontal emphasis paint mode={mode} n={n} shadows={shadows} work={work} median_ms={:.6} samples={samples:?}",
                    samples[2]
                );
            }
        }
    };
    if reference {
        crate::emphasis::with_reference_horizontal(measure);
    } else {
        measure();
    }
}

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
