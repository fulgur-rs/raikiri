use super::*;
use crate::cascade::candidate::Precedence;
use crate::property::PropertyValue;
use crate::resolve::ComputedLength;
use crate::rule::Declaration;

const KINDS: [PseudoElem; 7] = [
    PseudoElem::Before,
    PseudoElem::After,
    PseudoElem::Marker,
    PseudoElem::FirstLine,
    PseudoElem::FirstLetter,
    PseudoElem::Backdrop,
    PseudoElem::FileSelectorButton,
];

fn candidates(value: f32) -> OwnedCandidates {
    OwnedCandidates::from_declarations([(
        Declaration::new(PropertyValue::Opacity(value), false),
        Precedence::new(Origin::Author, false, 0, 0, Default::default()),
    )])
}

fn fixture(unrelated: usize) -> (CascadeResult, [usize; 4]) {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let body = doc.push_element(html, "body", None);
    let keep = doc.push_element(body, "header", None);
    for _ in 0..unrelated {
        doc.push_element(body, "div", None);
    }
    let mut result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for old in 0..result.computed.len() {
        let id = StyleNodeId(old as u64);
        let context = ResolveContext::new(ComputedLength(16.0 + old as f32));
        let svg = vec![SvgStyleProperty::new(
            PropertyKey::Color,
            false,
            Some(format!("node{old}")),
            Some(id),
        )];
        result.svg_style_properties.insert(id, svg.clone());
        result.svg_relative_presentation_attributes.insert(id, svg);
        result.first_line_contexts.insert(id, context);
        result.first_letter_inputs.insert(
            id,
            first_letter::FirstLetterInputs {
                candidates: candidates(old as f32 / 1000.0),
                context,
            },
        );
        for (ordinal, kind) in KINDS.into_iter().enumerate() {
            let opacity = old as f32 / 1000.0 + ordinal as f32 / 100.0;
            let mut value = ComputedValues::initial();
            value.opacity = opacity;
            result.pseudo.insert((id, kind), PseudoValueArc::new(value));
            result
                .typographic_inheritance
                .insert((id, Some(kind)), candidates(opacity));
        }
        result
            .typographic_inheritance
            .insert((id, None), candidates(old as f32 / 1000.0));
    }
    (result, [0, html, body, keep])
}

fn assert_metadata(part: &CascadeResult, source: &CascadeResult, new: usize, old: usize) {
    let new = StyleNodeId(new as u64);
    let old = StyleNodeId(old as u64);
    assert_eq!(
        part.svg_style_properties[&new],
        source.svg_style_properties[&old]
    );
    assert_eq!(
        part.svg_relative_presentation_attributes[&new],
        source.svg_relative_presentation_attributes[&old]
    );
    assert_eq!(
        part.first_line_contexts[&new],
        source.first_line_contexts[&old]
    );
    assert_eq!(
        part.first_letter_inputs[&new].context,
        source.first_letter_inputs[&old].context
    );
    assert_eq!(
        part.first_letter_inputs[&new]
            .candidates
            .candidates()
            .declaration(0),
        source.first_letter_inputs[&old]
            .candidates
            .candidates()
            .declaration(0)
    );
    for kind in KINDS {
        assert!(PseudoValueArc::ptr_eq(
            &part.pseudo[&(new, kind)],
            &source.pseudo[&(old, kind)]
        ));
    }
    for kind in std::iter::once(None).chain(KINDS.into_iter().map(Some)) {
        assert_eq!(
            part.typographic_inheritance[&(new, kind)]
                .candidates()
                .declaration(0),
            source.typographic_inheritance[&(old, kind)]
                .candidates()
                .declaration(0)
        );
    }
}

#[test]
fn fixed_subset_probes_only_kept_nodes_for_every_side_map() {
    for unrelated in [16, 64, 256] {
        let (source, keep) = fixture(unrelated);
        EXTRACT_WORK.set([0; 2]);
        let part = source.extract(&keep);
        assert_eq!(EXTRACT_WORK.get(), [0, 76]);
        assert_eq!(part.pseudo.len(), 28);
        assert_eq!(part.typographic_inheritance.len(), 32);
        assert_eq!(part.svg_style_properties.len(), 4);
        assert_eq!(part.svg_relative_presentation_attributes.len(), 4);
        assert_eq!(part.first_letter_inputs.len(), 4);
        assert_eq!(part.first_line_contexts.len(), 4);
        for (new, old) in keep.into_iter().enumerate() {
            assert_metadata(&part, &source, new, old);
        }
    }
}

#[test]
fn sparse_overreserved_maps_do_not_scan_capacity() {
    let (mut source, keep) = fixture(0);
    source.svg_style_properties.reserve(4096);
    source.svg_relative_presentation_attributes.reserve(4096);
    source.first_letter_inputs.reserve(4096);
    source.first_line_contexts.reserve(4096);
    source.pseudo.reserve(4096);
    source.typographic_inheritance.reserve(4096);
    assert!(source.pseudo.capacity() >= 4096);
    EXTRACT_WORK.set([0; 2]);
    let part = source.extract(&keep);
    assert_eq!(EXTRACT_WORK.get(), [0, 76]);
    assert_metadata(&part, &source, 3, keep[3]);
}

#[test]
fn duplicate_and_empty_mappings_keep_existing_semantics() {
    let (source, keep) = fixture(4);
    let part = source.extract(&[keep[3], 0, keep[3]]);
    assert_eq!(part.computed.len(), 3);
    assert_eq!(part.computed[0], source.computed[keep[3]]);
    assert_eq!(part.computed[2], source.computed[keep[3]]);
    assert!(
        !part
            .pseudo
            .contains_key(&(StyleNodeId(0), PseudoElem::Before))
    );
    assert_metadata(&part, &source, 2, keep[3]);
    assert_metadata(&part, &source, 1, 0);
    assert_ne!(part.generation(), source.generation());
    assert_ne!(part.element_generation(), source.element_generation());
    assert_eq!(part.root_element_index, 0);
    assert_eq!(part.principal_element_index, 0);
    EXTRACT_WORK.set([0; 2]);
    let empty = source.extract(&[]);
    assert_eq!(EXTRACT_WORK.get(), [0, 0]);
    assert!(empty.computed.is_empty() && empty.pseudo.is_empty());
    assert!(empty.svg_style_properties.is_empty() && empty.typographic_inheritance.is_empty());
}

#[test]
fn small_capacity_maps_keep_last_duplicate_and_drop_unretained_entries() {
    let (mut source, keep) = fixture(0);
    let picked = StyleNodeId(keep[3] as u64);
    let unrelated = StyleNodeId(999);
    macro_rules! small_node_map {
        ($map:expr) => {{
            let value = $map[&picked].clone();
            $map = HashMap::from([(picked, value.clone()), (unrelated, value)]);
            assert!($map.capacity() <= 4);
        }};
    }
    small_node_map!(source.svg_style_properties);
    small_node_map!(source.svg_relative_presentation_attributes);
    small_node_map!(source.first_letter_inputs);
    small_node_map!(source.first_line_contexts);
    let pseudo = source.pseudo[&(picked, PseudoElem::Backdrop)].clone();
    source.pseudo = HashMap::from([
        ((picked, PseudoElem::Backdrop), pseudo.clone()),
        ((unrelated, PseudoElem::Backdrop), pseudo),
    ]);
    assert!(source.pseudo.capacity() <= 4 * 7);
    let candidates = source.typographic_inheritance[&(picked, None)].clone();
    source.typographic_inheritance = HashMap::from([
        ((picked, None), candidates.clone()),
        ((unrelated, None), candidates),
    ]);
    assert!(source.typographic_inheritance.capacity() <= 4 * 8);
    EXTRACT_WORK.set([0; 2]);
    let part = source.extract(&[keep[3], 0, keep[3], keep[1], keep[2]]);
    let work = EXTRACT_WORK.get();
    assert!(
        work[0] + work[1] <= 12,
        "small-map extraction did {work:?} operations"
    );
    let new = StyleNodeId(2);
    assert_eq!(part.svg_style_properties.len(), 1);
    assert_eq!(
        part.svg_style_properties[&new],
        source.svg_style_properties[&picked]
    );
    assert_eq!(part.svg_relative_presentation_attributes.len(), 1);
    assert_eq!(
        part.svg_relative_presentation_attributes[&new],
        source.svg_relative_presentation_attributes[&picked]
    );
    assert_eq!(part.first_letter_inputs.len(), 1);
    assert_eq!(
        part.first_letter_inputs[&new].context,
        source.first_letter_inputs[&picked].context
    );
    assert_eq!(part.first_line_contexts.len(), 1);
    assert_eq!(
        part.first_line_contexts[&new],
        source.first_line_contexts[&picked]
    );
    assert_eq!(part.pseudo.len(), 1);
    assert!(PseudoValueArc::ptr_eq(
        &part.pseudo[&(new, PseudoElem::Backdrop)],
        &source.pseudo[&(picked, PseudoElem::Backdrop)]
    ));
    assert_eq!(part.typographic_inheritance.len(), 1);
    assert_eq!(
        part.typographic_inheritance[&(new, None)]
            .candidates()
            .declaration(0),
        source.typographic_inheritance[&(picked, None)]
            .candidates()
            .declaration(0)
    );
}
