use super::*;
use shodo::geometry::{PhysicalConverter, PhysicalSize};

#[test]
fn vertical_paint_runs_preserve_reference_centers_with_combine_ruby_and_shadows() {
    let mut combined = 0;
    let mut ordinary = 0;
    for mode in ["vertical-rl", "vertical-lr"] {
        for direction in ["ltr", "rtl"] {
            for orientation in ["mixed", "upright", "sideways"] {
                for shadow in ["none", "2px 3px 1px rgb(0,0,255)"] {
                    let css = format!(
                        "writing-mode:{mode};direction:{direction};text-orientation:{orientation};height:400px;width:100px;white-space:pre;line-height:40px;text-emphasis:dot;text-emphasis-color:rgb(255,0,0);text-shadow:{shadow}"
                    );
                    let (mut doc, cascade, root) = paragraph(&css, |doc, root| {
                        doc.append_text(root, "a\u{301}, אבג a ");
                        let combine = doc.append_element(
                            Some(root),
                            "span",
                            Some("display:inline;text-combine-upright:all"),
                        );
                        doc.append_text(combine, "\t1");
                        let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
                        let base = doc.append_element(Some(ruby), "rb", Some("display:ruby-base"));
                        doc.append_text(base, "a");
                        let annotation = doc.append_element(
                            Some(ruby),
                            "rt",
                            Some("display:ruby-text;font-size:20px"),
                        );
                        doc.append_text(annotation, "x");
                    });
                    lay_out(&mut doc, &cascade);
                    let scene = painted(&doc, &cascade);
                    assert!(
                        glyphs(&scene)
                            .iter()
                            .any(|glyph| glyph.3 == solid(255, 0, 0)),
                        "{css}"
                    );
                    for line in doc.get_node(root).unwrap().ifc_lines().unwrap() {
                        let converter = PhysicalConverter::new(
                            line.writing_mode(),
                            line.used_direction(),
                            PhysicalSize {
                                width: 500.0,
                                height: 700.0,
                            },
                        );
                        for fragment in line.fragments() {
                            let Fragment::GlyphRun(run) = fragment else {
                                continue;
                            };
                            let Some(mark) = run.emphasis_mark() else {
                                continue;
                            };
                            if run.orientation() == shodo::GlyphOrientation::Combined {
                                combined += 1;
                            } else {
                                ordinary += 1;
                            }
                            assert_eq!(
                                crate::emphasis::vertical_mark_centers(
                                    &mark,
                                    (line, &run),
                                    (line.writing_mode(), &converter)
                                ),
                                crate::emphasis::reference_vertical_mark_centers(
                                    &mark,
                                    (line, &run),
                                    (line.writing_mode(), &converter)
                                ),
                                "{css}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(combined > 0);
    assert!(ordinary > 0);
}
