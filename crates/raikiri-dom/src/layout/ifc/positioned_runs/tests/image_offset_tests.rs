use super::*;
use crate::layout::ifc::test_support::sheet_fixture;
use raikiri_traits::{DecodedImage, ImagePixelSource};
use std::cell::Cell;
use std::sync::{Arc, OnceLock};

thread_local! {
    static TEXT: Cell<usize> = const { Cell::new(0) };
    static PREFIX: Cell<usize> = const { Cell::new(0) };
    static COVERAGE: Cell<usize> = const { Cell::new(0) };
    static COVERAGE_BYTES: Cell<usize> = const { Cell::new(0) };
    static LOOKUPS: Cell<usize> = const { Cell::new(0) };
}
pub(crate) fn note_text() {
    TEXT.set(TEXT.get() + 1);
}
pub(crate) fn prefix_visits() -> usize {
    PREFIX.get()
}

pub(crate) fn note_prefix() {
    PREFIX.set(PREFIX.get() + 1);
}
pub(crate) fn note_coverage() {
    COVERAGE.set(COVERAGE.get() + 1);
}
pub(crate) fn note_coverage_byte() {
    COVERAGE_BYTES.set(COVERAGE_BYTES.get() + 1);
}
pub(crate) fn note_lookup() {
    LOOKUPS.set(LOOKUPS.get() + 1);
}
fn reset() {
    TEXT.set(0);
    PREFIX.set(0);
    COVERAGE.set(0);
    LOOKUPS.set(0);
    COVERAGE_BYTES.set(0);
}

struct Pixels(Arc<DecodedImage>);
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        (!url.path().ends_with("missing")).then(|| self.0.clone())
    }
}

fn fixture(count: usize, wrapped: bool) -> (Document, CascadeResult, usize) {
    let fixture = sheet_fixture(
        "span::before{content:url('https://example.test/dot')}",
        if wrapped {
            "width:20px"
        } else {
            "width:200000px;white-space:nowrap"
        },
        |doc, root| {
            for _ in 0..count {
                let span = doc.append_element(Some(root), "span", None::<&str>);
                doc.append_text(span, "X ");
            }
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.prepare_list_marker_images(
        &cascade,
        &Pixels(Arc::new(DecodedImage::new(1, 1, vec![255; 4]))),
        None,
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    (doc, cascade, root)
}

fn extract_and_count(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    count: usize,
    wrapped: bool,
) {
    reset();
    let view = PositionedLines::new(doc, cascade, root, None).unwrap();
    assert_eq!(view.all_lines().len(), if wrapped { count } else { 1 });
    let mut images = 0;
    let mut glyphs = 0;
    for line in view.lines() {
        images += line.content_images.len();
        assert!(
            line.content_images
                .iter()
                .all(|image| image.text_offset.is_some())
        );
        glyphs += line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>();
    }
    assert_eq!(images, count);
    assert!(glyphs >= count);
    let text = TEXT.get() + PREFIX.get();
    let coverage = COVERAGE.get();
    let lookups = LOOKUPS.get();
    eprintln!(
        "image_work,N={count},wrapped={wrapped},text={text},coverage={coverage},bytes={},lookups={lookups}",
        COVERAGE_BYTES.get()
    );
    assert!(
        text <= 32 * count,
        "N={count}: text={text}, coverage={coverage}, lookups={lookups}"
    );
    assert!(
        coverage <= 8 * count,
        "N={count}: text={text}, coverage={coverage}, lookups={lookups}"
    );
    assert!(COVERAGE_BYTES.get() <= 8 * count);
    assert!(
        lookups <= 8 * count,
        "N={count}: text={text}, coverage={coverage}, lookups={lookups}"
    );
}

#[test]
fn many_generated_images_in_one_line_have_linear_lookup_work() {
    for count in [32, 64, 128] {
        let (doc, cascade, root) = fixture(count, false);
        extract_and_count(&doc, &cascade, root, count, false);
    }
}

#[test]
fn many_generated_image_lines_have_linear_text_work() {
    for count in [32, 64, 128] {
        let (doc, cascade, root) = fixture(count, true);
        extract_and_count(&doc, &cascade, root, count, true);
    }
}

fn prefix_cache(doc: &Document, root: usize) -> &OnceLock<Arc<ObjectTextPrefixes>> {
    &doc.ifc_layout_node(root)
        .unwrap()
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap()
        .object_text_prefixes
}

#[test]
fn prepared_out_of_order_views_and_warm_clones_share_prefix_preparation() {
    let (doc, cascade, root) = fixture(64, true);
    assert!(prefix_cache(&doc, root).get().is_none());
    let geometry = geometry::PositionedLineGeometry::new(&doc, root).unwrap();
    let prepared = geometry.view(&doc, &cascade, None).unwrap();
    reset();
    let last = prepared.line_at(63).unwrap();
    assert!(last.content_images[0].text_offset.is_some());
    assert!(PREFIX.get() > 0 && PREFIX.get() <= 8 * 64);
    let cloned = doc.clone();
    assert!(Arc::ptr_eq(
        prefix_cache(&doc, root).get().unwrap(),
        prefix_cache(&cloned, root).get().unwrap()
    ));
    reset();
    for index in [0, 63, 2, 62, 0] {
        assert!(
            prepared.line_at(index).unwrap().content_images[0]
                .text_offset
                .is_some()
        );
    }
    let view = PositionedLines::new(&cloned, &cascade, root, None).unwrap();
    assert_eq!(
        view.lines()
            .map(|line| line.content_images.len())
            .sum::<usize>(),
        64
    );
    assert_eq!(PREFIX.get(), 0);
}

#[test]
fn image_free_views_do_not_prepare_prefix_storage() {
    let (doc, cascade, root) = two_lines();
    assert!(prefix_cache(&doc, root).get().is_none());
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    assert_eq!(view.lines().count(), 2);
    assert!(prefix_cache(&doc, root).get().is_none());
}

#[test]
fn first_line_transforms_keep_distinct_processed_text_datasets() {
    let fixture = sheet_fixture(
        "div::first-line{text-transform:uppercase}span::before{content:url('https://example.test/dot')}",
        "width:20px",
        |doc, root| {
            for _ in 0..4 {
                let span = doc.append_element(Some(root), "span", None::<&str>);
                doc.append_text(span, "ı ");
            }
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.prepare_list_marker_images(
        &cascade,
        &Pixels(Arc::new(DecodedImage::new(1, 1, vec![255; 4]))),
        None,
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    assert!(view.all_lines().len() >= 2);
    assert_ne!(
        view.all_lines()[0].text().len(),
        view.all_lines()[1].text().len()
    );
    let mut datasets = std::collections::HashMap::new();
    for line in view.all_lines() {
        datasets
            .entry((line.text().as_ptr() as usize, line.text().len()))
            .or_insert_with(|| line.text().chars().count());
    }
    assert!(datasets.len() >= 2);
    reset();
    let mut images = 0;
    for line in view.lines() {
        let range = line.line.text_range();
        let expected = range.start + line.line.text()[range.clone()].find('\u{FFFC}').unwrap();
        assert_eq!(line.content_images.len(), 1);
        assert_eq!(line.content_images[0].text_offset, Some(expected));
        images += 1;
    }
    assert_eq!(images, 4);
    assert_eq!(PREFIX.get(), datasets.values().sum::<usize>());
}

#[test]
fn rebreaking_lines_discards_the_old_prefix_generation() {
    let (mut doc, cascade, root) = fixture(32, true);
    let old_lines = {
        let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
        let count = view.all_lines().len();
        assert_eq!(
            view.lines()
                .map(|line| line.content_images.len())
                .sum::<usize>(),
            32
        );
        count
    };
    let original = Arc::clone(prefix_cache(&doc, root).get().unwrap());
    doc.set_element_inline_style(
        root,
        Some("display:block;font-family:Ahem;font-size:10px;width:40px".into()),
    );
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    doc.prepare_list_marker_images(
        &cascade,
        &Pixels(Arc::new(DecodedImage::new(1, 1, vec![255; 4]))),
        None,
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    assert!(prefix_cache(&doc, root).get().is_none());
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    assert_ne!(view.all_lines().len(), old_lines);
    assert_eq!(
        view.lines()
            .map(|line| line.content_images.len())
            .sum::<usize>(),
        32
    );
    assert!(!Arc::ptr_eq(
        &original,
        prefix_cache(&doc, root).get().unwrap()
    ));
}

// This independent reference preserves the pre-optimization conservative mapping.
fn legacy_object_text_offsets(line: &shodo::Line) -> Option<Vec<(usize, usize)>> {
    let range = line.text_range();
    if line.text().get(range.clone())?.chars().any(|c| {
        matches!(
            c,
            '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
    }) {
        return None;
    }
    // The embeddings, overrides and isolates open where the line starts,
    // `true` for an isolate. A PDF closes only an embedding or override
    // above every isolate, and a PDI closes its isolate with everything
    // opened inside it; unmatched closers are ignored (UAX #9 X6a, X7).
    let mut open = Vec::new();
    for c in line.text().get(..range.start)?.chars() {
        match c {
            '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}' => open.push(false),
            '\u{2066}'..='\u{2068}' => open.push(true),
            '\u{202C}' if open.last() == Some(&false) => {
                open.pop();
            }
            '\u{2069}' => {
                if let Some(isolate) = open.iter().rposition(|&isolate| isolate) {
                    open.truncate(isolate);
                }
            }
            _ => {}
        }
    }
    if !open.is_empty() {
        return None;
    }
    let rtl = u8::from(line.used_direction() == shodo::geometry::Direction::Rtl);
    let mut covered = Vec::new();
    let mut objects = Vec::new();
    for (index, fragment) in line.fragments().enumerate() {
        match fragment {
            Fragment::GlyphRun(run) => {
                if run.bidi_level() % 2 != rtl {
                    return None;
                }
                covered.push(run.text_range());
            }
            Fragment::Atomic(atomic) => objects.push((atomic.border_rect.inline_start, index)),
            Fragment::OutOfFlowAnchor(anchor) => objects.push((anchor.inline_position, index)),
            _ => {}
        }
    }
    let text = line.text().get(range.clone())?;
    let slots: Vec<usize> = text
        .match_indices('\u{FFFC}')
        .map(|(offset, _)| range.start + offset)
        .filter(|offset| !covered.iter().any(|run| run.contains(offset)))
        .collect();
    if slots.len() != objects.len() {
        return None;
    }
    objects.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    Some(
        objects
            .into_iter()
            .zip(slots)
            .map(|((_, fragment), offset)| (fragment, offset))
            .collect(),
    )
}

#[test]
fn object_offsets_match_the_original_across_reading_order_controls() {
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for shape in [
            "mixed",
            "objects",
            "literal",
            "ruby",
            "rtl",
            "oof",
            "failed",
            "hidden",
            "open",
            "closed",
            "nonmonotone",
            "ellipsis",
        ] {
            let extra = match shape {
                "rtl" => "direction:rtl",
                "open" | "closed" => "width:20px",
                "ellipsis" => {
                    "width:20px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis"
                }
                _ => "width:200px",
            };
            let mut sheet = String::from(
                "span::before{content:url('https://example.test/dot') url('https://example.test/dot2')}",
            );
            match shape {
                "failed" => sheet.push_str("span::before{content:url('https://example.test/dot') url('https://example.test/missing') url('https://example.test/dot2')}"),
                "hidden" => sheet.push_str("b{visibility:hidden}b::before{content:url('https://example.test/dot')}"),
                "open" | "closed" => sheet.push_str("b{unicode-bidi:isolate-override;direction:rtl}"),
                "nonmonotone" => sheet.push_str("span{margin-left:-15px}"),
                _ => {},
            }
            let fixture = sheet_fixture(
                &sheet,
                &format!("writing-mode:{mode};{extra}"),
                |doc, root| {
                    let add = |doc: &mut Document, parent, text: &str| {
                        let span = doc.append_element(Some(parent), "span", None::<&str>);
                        doc.append_text(span, text);
                    };
                    match shape {
                        "ruby" => {
                            let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
                            add(doc, ruby, "A");
                            let rt =
                                doc.append_element(Some(ruby), "rt", Some("display:ruby-text"));
                            doc.append_text(rt, "B");
                        }
                        "open" => {
                            let b = doc.append_element(Some(root), "b", None::<&str>);
                            doc.append_text(b, "Z");
                            doc.append_element(Some(b), "br", None::<&str>);
                            add(doc, b, "");
                            doc.append_element(Some(b), "br", None::<&str>);
                            doc.append_text(b, "W");
                        }
                        "closed" => {
                            let b = doc.append_element(Some(root), "b", None::<&str>);
                            doc.append_text(b, "Z");
                            doc.append_element(Some(root), "br", None::<&str>);
                            add(doc, root, "");
                        }
                        "hidden" => {
                            doc.append_element(Some(root), "b", None::<&str>);
                            add(doc, root, "X");
                        }
                        "literal" => add(doc, root, "X\u{FFFC}Y"),
                        "objects" | "rtl" => add(doc, root, ""),
                        "oof" => {
                            add(doc, root, "X");
                            doc.append_element(
                                Some(root),
                                "i",
                                Some("position:absolute;width:1px;height:1px"),
                            );
                            add(doc, root, "Y");
                        }
                        _ => {
                            for _ in 0..4 {
                                add(doc, root, "X ");
                            }
                        }
                    }
                },
            );
            let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
            doc.prepare_list_marker_images(
                &cascade,
                &Pixels(Arc::new(DecodedImage::new(1, 1, vec![255; 4]))),
                None,
            );
            layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
            let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
            let prefixes = ObjectTextPrefixes::new(view.all_lines());
            for line in view.all_lines() {
                let expected = legacy_object_text_offsets(line).map(|pairs| {
                    let mut dense = vec![None; line.fragments().len()];
                    for (fragment, offset) in pairs {
                        dense[fragment] = Some(offset);
                    }
                    dense
                });
                let actual = object_text_offsets(line, || prefixes.closed_before(line));
                assert_eq!(actual, expected, "mode={mode},shape={shape}");
            }
        }
    }
}

#[test]
fn ellipsis_rebuilds_fragment_offsets_but_keeps_shared_text_prefixes() {
    let (mut doc, cascade, root) = fixture(4, false);
    {
        let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
        let images: Vec<_> = view.lines().flat_map(|line| line.content_images).collect();
        assert_eq!(images.len(), 4);
        assert!(images.iter().all(|image| image.text_offset.is_some()));
    }
    let original = Arc::clone(prefix_cache(&doc, root).get().unwrap());
    let cloned = doc.clone();
    let mut paragraph = doc.nodes[root].ifc.as_ref().unwrap().without_lines();
    paragraph.ellipsis = true;
    crate::layout::ifc::flow::apply_text_overflow(
        &paragraph,
        doc.nodes[root]
            .ifc
            .as_mut()
            .unwrap()
            .lines
            .as_mut()
            .unwrap(),
        &mut shodo::LayoutContext::new(),
        20.0,
    );
    assert!(Arc::ptr_eq(
        &original,
        prefix_cache(&doc, root).get().unwrap()
    ));
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    let images: Vec<_> = view.lines().flat_map(|line| line.content_images).collect();
    assert!(!images.is_empty());
    assert!(images.iter().all(|image| image.text_offset.is_none()));
    let view = PositionedLines::new(&cloned, &cascade, root, None).unwrap();
    let images: Vec<_> = view.lines().flat_map(|line| line.content_images).collect();
    assert_eq!(images.len(), 4);
    assert!(images.iter().all(|image| image.text_offset.is_some()));
}

#[test]
fn ellipsis_keeps_known_offsets_when_all_objects_stay_visible() {
    let fixture = sheet_fixture(
        "span::before{content:url('https://example.test/dot')}",
        "width:200px;white-space:nowrap",
        |doc, root| {
            let span = doc.append_element(Some(root), "span", None::<&str>);
            doc.append_text(span, "XXXXXXXXXXXX");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.prepare_list_marker_images(
        &cascade,
        &Pixels(Arc::new(DecodedImage::new(1, 1, vec![255; 4]))),
        None,
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    {
        let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
        assert_eq!(
            view.lines().next().unwrap().content_images[0].text_offset,
            Some(0)
        );
    }
    let cloned = doc.clone();
    let mut paragraph = doc.nodes[root].ifc.as_ref().unwrap().without_lines();
    paragraph.ellipsis = true;
    crate::layout::ifc::flow::apply_text_overflow(
        &paragraph,
        doc.nodes[root]
            .ifc
            .as_mut()
            .unwrap()
            .lines
            .as_mut()
            .unwrap(),
        &mut shodo::LayoutContext::new(),
        20.0,
    );
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    assert!(
        view.all_lines()[0]
            .fragments()
            .any(|fragment| matches!(fragment,Fragment::GlyphRun(run) if run.is_ellipsis()))
    );
    assert_eq!(
        view.lines().next().unwrap().content_images[0].text_offset,
        Some(0)
    );
    assert!(Arc::ptr_eq(
        prefix_cache(&doc, root).get().unwrap(),
        prefix_cache(&cloned, root).get().unwrap()
    ));
}
