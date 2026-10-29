use super::*;

fn fixture(css: &str) -> (Document, raikiri_style::CascadeResult, usize) {
    let (mut doc, cascade, root) = paragraph(
        &format!(
            "font-family:Liga;line-height:40px;text-emphasis:dot;text-emphasis-color:rgb(255,0,0);white-space:pre;{css}"
        ),
        |doc, root| {
            doc.append_text(root, "a\u{301}\u{301}, ");
            let rtl = doc.append_element(
                Some(root),
                "span",
                Some("display:inline;direction:rtl;unicode-bidi:bidi-override"),
            );
            doc.append_text(rtl, "ffi");
            doc.append_text(root, " abc אבג \u{200b}\u{200d}\u{00ad} ");
            let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
            let base = doc.append_element(Some(ruby), "rb", Some("display:ruby-base"));
            doc.append_text(base, "ffi");
            let annotation =
                doc.append_element(Some(ruby), "rt", Some("display:ruby-text;font-size:5px"));
            doc.append_text(annotation, "bb");
        },
    );
    let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR)).unwrap();
    fonts
        .register_face(
            include_bytes!("../../../tests/data/noto-sans-latin/NotoSans-LatinSubset.ttf").to_vec(),
            0,
            shodo::font::FontFaceDescriptor {
                family: "Liga".into(),
                ..Default::default()
            },
        )
        .unwrap();
    doc.set_font_collection(fonts);
    layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();
    (doc, cascade, root)
}

#[test]
fn horizontal_emphasis_paint_matches_the_previous_scan_with_complex_clusters() {
    let mut ligature = false;
    let mut multiple_glyphs = false;
    let mut excluded = false;
    let mut rtl = false;
    let mut annotation = false;
    for direction in ["ltr", "rtl"] {
        for side in ["over", "under"] {
            for shadow in ["none", "1px 2px rgb(0,0,255), 3px 4px rgb(0,255,0)"] {
                let (doc, cascade, root) = fixture(&format!(
                    "direction:{direction};text-emphasis-position:{side} right;text-shadow:{shadow}"
                ));
                for line in doc.get_node(root).unwrap().ifc_lines().unwrap() {
                    for fragment in line.fragments() {
                        let Fragment::GlyphRun(run) = fragment else {
                            continue;
                        };
                        rtl |= run.bidi_level() % 2 == 1;
                        for cluster in run.clusters() {
                            let slots = run.emphasis_slots(&cluster).count();
                            ligature |= slots > 1;
                            excluded |= slots == 0;
                            multiple_glyphs |= run
                                .glyphs()
                                .filter(|glyph| {
                                    cluster.text_range.contains(&(glyph.cluster as usize))
                                })
                                .count()
                                > 1;
                        }
                    }
                }
                let actual = painted(&doc, &cascade);
                let reference =
                    crate::emphasis::with_reference_horizontal(|| painted(&doc, &cascade));
                annotation |= actual.commands.iter().any(|command| matches!(command, RenderCommand::GlyphRun(run) if run.font_size == 5.0));
                let drawn = glyphs(&actual);
                assert!(drawn.iter().any(|glyph| glyph.3 == solid(255, 0, 0)));
                assert_eq!(drawn, glyphs(&reference), "{direction}/{side}/{shadow}");
            }
        }
    }
    assert!(ligature, "the fixture must shape a multi-slot ligature");
    assert!(
        multiple_glyphs,
        "the fixture must shape multiple glyphs in one cluster"
    );
    assert!(
        excluded,
        "the fixture must contain characters that take no mark"
    );
    assert!(rtl, "the fixture must shape an odd-level bidi run");
    assert!(annotation, "the fixture must paint the ruby annotation");
}

#[test]
fn horizontal_emphasis_does_not_place_marks_for_missing_input_glyphs() {
    let (mut doc, cascade, root) = paragraph("text-emphasis:dot", |doc, root| {
        doc.append_text(root, "a");
    });
    lay_out(&mut doc, &cascade);
    let line = &doc.get_node(root).unwrap().ifc_lines().unwrap()[0];
    let run = line
        .fragments()
        .find_map(|fragment| match fragment {
            Fragment::GlyphRun(run) => Some(run),
            _ => None,
        })
        .unwrap();
    assert!(run.emphasis_mark().is_some());
    let mut scene = Scene::new();
    crate::emphasis::draw_marks_in(
        &mut scene,
        &doc,
        &run,
        &[],
        (0.0, 0.0),
        &cascade.computed[root],
        peniko::Color::from_rgb8(255, 0, 0),
    );
    assert!(scene.commands.is_empty());
}
