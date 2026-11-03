//! The texts that are still to take their part of a first letter that spans
//! several texts.

use super::*;
use crate::layout::ifc::projection::GeneratedCounters;
use crate::layout::ifc::test_support::span;

/// Whether a text still to be taken lies in a container other than that of
/// `owner`, found by going over all of them.
fn scanned(letter: &FirstLetter, doc: &Document, owner: usize) -> bool {
    letter
        .continuation
        .iter()
        .any(|(other, _)| container_of(doc, *other) != container_of(doc, owner))
}

#[test]
fn whether_a_text_continues_in_another_container_is_known_after_every_push() {
    // The punctuation lies in the block, in an inline box, in the block again
    // and in two inline boxes, and the letter is in the last of them.
    let mut texts = Vec::new();
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        texts.push(doc.append_text(root, "("));
        let first = span(doc, root, "display:inline");
        texts.push(doc.append_text(first, "("));
        texts.push(doc.append_text(root, "("));
        let second = span(doc, root, "display:inline");
        texts.push(doc.append_text(second, "("));
        let inner = span(doc, second, "display:inline");
        texts.push(doc.append_text(inner, "("));
        texts.push(doc.append_text(inner, "Ab"));
        texts.push(doc.append_text(root, " tail"));
    });
    let fonts = ahem_fonts();
    let limits = Limits::default();
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    let (mut crossing, mut within) = (0, 0);
    for &text in &texts {
        let parent = fixture.doc.parent_of(text).unwrap();
        letter
            .push(
                &mut builder,
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(text as u64),
                    offset: 0,
                },
                &fixture.cascade.computed[parent],
                fixture.doc.get_node(text).unwrap().text_content().unwrap(),
                &fonts,
            )
            .unwrap();
        for &owner in &texts {
            let indexed = letter.continues_in_other_container(&fixture.doc, owner);
            assert_eq!(
                indexed,
                scanned(&letter, &fixture.doc, owner),
                "after text {text}, for text {owner}"
            );
            if indexed {
                crossing += 1;
            } else {
                within += 1;
            }
        }
    }
    // The queue was not empty while it held texts of one container only, and
    // while it held texts of several.
    assert!(crossing > 0 && within > 0, "{crossing} and {within}");
    assert!(letter.continuation.is_empty());
    // Every count was given back as its text was taken.
    assert!(letter.continuation_containers.is_empty());
}

#[test]
fn stopping_forgets_the_texts_that_were_still_to_be_taken() {
    let mut texts = Vec::new();
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        texts.push(doc.append_text(root, "("));
        let inline = span(doc, root, "display:inline");
        texts.push(doc.append_text(inline, "Ab"));
    });
    let mut letter = FirstLetter::new(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &Limits::default(),
    );
    letter
        .adjacent_range(
            &fixture.doc,
            &fixture.cascade,
            TextSource::Dom {
                node: NodeId(texts[0] as u64),
                offset: 0,
            },
            "(",
            false,
            &GeneratedCounters::default(),
        )
        .unwrap();
    assert!(letter.continues_in_other_container(&fixture.doc, texts[0]));
    letter.stop();
    assert!(!letter.continues_in_other_container(&fixture.doc, texts[0]));
    assert!(letter.continuation.is_empty());
}
