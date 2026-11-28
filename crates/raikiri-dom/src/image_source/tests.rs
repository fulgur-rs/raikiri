//! Expected values were read from Chromium on a 200x200 screen viewport: the
//! candidate an image shows (`currentSrc`) and its rendered size, which is the
//! image's size divided by the candidate's density.

use super::*;
use raikiri_style::MediaType;

type Attributes<'a> = &'a [(&'a str, &'a str)];

/// A document holding an `<img>` with `attributes`, inside a `<picture>`
/// after `sources` when there are any, and the image's index.
fn image_in(sources: &[(&str, Attributes<'_>)], attributes: Attributes<'_>) -> (Document, usize) {
    let mut document = Document::new();
    let root = document.root_index();
    let parent = if sources.is_empty() {
        root
    } else {
        document.append_element(Some(root), "picture", None::<&str>)
    };
    let set = |document: &mut Document, id: usize, attributes: Attributes<'_>| {
        document.set_element_attributes(
            id,
            attributes
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
        );
    };
    for (tag, source) in sources {
        let id = document.append_element(Some(parent), *tag, None::<&str>);
        set(&mut document, id, source);
    }
    let image = document.append_element(Some(parent), "img", None::<&str>);
    set(&mut document, image, attributes);
    (document, image)
}

fn screen() -> MediaContext {
    MediaContext::with_viewport(MediaType::Screen, 200, 200)
}

/// The URL and density an image shows on a 200x200 screen.
fn chosen(sources: &[(&str, Attributes<'_>)], attributes: Attributes<'_>) -> Option<(String, f32)> {
    let (document, image) = image_in(sources, attributes);
    image_source(&document, image, &screen()).map(|source| (source.url.to_owned(), source.density))
}

fn shows(url: &str, density: f32) -> Option<(String, f32)> {
    Some((url.to_owned(), density))
}

#[test]
fn the_smallest_density_of_at_least_one_is_chosen_or_else_the_largest() {
    for (srcset, expected) in [
        ("b.png 2x", shows("b.png", 2.0)),
        ("b.png 0.5x", shows("b.png", 0.5)),
        ("a.png 1x, b.png 2x", shows("a.png", 1.0)),
        ("b.png 2x, c.png 3x", shows("b.png", 2.0)),
        ("c.png 3x, b.png 2x", shows("b.png", 2.0)),
        ("a.png 0.5x, b.png 0.75x", shows("b.png", 0.75)),
        ("a.png 0.5x, b.png 2x", shows("b.png", 2.0)),
        ("a.png 0.9x, b.png 1.1x", shows("b.png", 1.1)),
        ("b.png 1.5x", shows("b.png", 1.5)),
        // Two of one density: the first.
        ("b.png 2x, c.png 2x", shows("b.png", 2.0)),
        // No descriptor is 1x.
        ("b.png", shows("b.png", 1.0)),
    ] {
        assert_eq!(chosen(&[], &[("srcset", srcset)]), expected, "{srcset}");
    }
}

#[test]
fn src_is_a_1x_candidate_unless_one_is_1x_already_or_has_a_width() {
    let image = |srcset| chosen(&[], &[("src", "a.png"), ("srcset", srcset)]);
    assert_eq!(image("b.png 2x"), shows("a.png", 1.0));
    assert_eq!(image("b.png 0.5x"), shows("a.png", 1.0));
    assert_eq!(image("b.png 1x"), shows("b.png", 1.0));
    assert_eq!(image("b.png 1.5x"), shows("a.png", 1.0));
    // c.png is 120 wide for a size of 200: 0.6x, and src is not added.
    assert_eq!(image("c.png 120w"), shows("c.png", 0.6));
    // A candidate of density zero is never chosen.
    assert_eq!(image("b.png 0x"), shows("a.png", 1.0));
    assert_eq!(chosen(&[], &[("src", "a.png")]), shows("a.png", 1.0));
    // An empty src is no candidate, but is still the source of an image
    // without any.
    assert_eq!(
        chosen(&[], &[("src", ""), ("srcset", "b.png 2x")]),
        shows("b.png", 2.0)
    );
    assert_eq!(chosen(&[], &[("src", "")]), shows("", 1.0));
    assert_eq!(chosen(&[], &[("srcset", "")]), None);
    assert_eq!(chosen(&[], &[]), None);
}

#[test]
fn a_width_is_divided_by_the_source_size() {
    let image = |srcset, sizes: Option<&str>| {
        let mut attributes = vec![("srcset", srcset)];
        attributes.extend(sizes.map(|sizes| ("sizes", sizes)));
        chosen(&[], &attributes)
    };
    // Without sizes the size is 100vw, the viewport's 200 px.
    assert_eq!(image("b.png 160w", None), shows("b.png", 0.8));
    assert_eq!(image("b.png 160w", Some("100px")), shows("b.png", 1.6));
    assert_eq!(
        image("a.png 40w, c.png 120w", Some("60px")),
        shows("c.png", 2.0)
    );
    assert_eq!(image("b.png 2x, c.png 120w", None), shows("b.png", 2.0));
    for (sizes, size) in [
        ("(min-width: 100px) 50px, 100px", 50.0),
        ("(max-width: 100px) 50px, 100px", 100.0),
        ("5em", 80.0),
        ("50vw", 100.0),
        ("calc(10px + 20px)", 30.0),
        ("calc(1em + 4px)", 20.0),
        ("calc(10vw + 10px)", 30.0),
        ("calc(50% + 1px), 30px", 30.0),
        ("-10px, 20px", 20.0),
        ("foo, 30px", 30.0),
        ("(min-width: 100px) 50px 60px, 40px", 40.0),
        ("40%, 30px", 30.0),
        ("(foo: bar) 50px, 30px", 30.0),
        ("not (min-width: 300px) 60px", 60.0),
        ("30px, 60px", 30.0),
        ("screen 50px, 30px", 30.0),
    ] {
        assert_eq!(
            image("c.png 120w", Some(sizes)),
            shows("c.png", 120.0 / size),
            "{sizes}"
        );
    }
    // A size of zero makes the density infinite: the image has no size.
    assert_eq!(
        image("c.png 120w", Some("0px, 30px")),
        shows("c.png", f32::INFINITY)
    );
}

#[test]
fn invalid_descriptors_drop_their_candidate() {
    for (srcset, expected) in [
        ("b.png 2.x", None),
        ("b.png .5x", shows("b.png", 0.5)),
        ("b.png 1e0x", shows("b.png", 1.0)),
        ("b.png +2x", None),
        ("b.png 2X", None),
        ("b.png -2x", None),
        ("b.png 0160w", shows("b.png", 0.8)),
        ("b.png 160w 80h", shows("b.png", 0.8)),
        ("b.png 80h", None),
        ("b.png 2x 40h", None),
        ("b.png 2x 2x", None),
        ("b.png 0w", None),
        ("b.png 1.5w", None),
        ("b.png w", None),
        ("b.png 2x 160w", None),
        ("b.png 160w 2x", None),
        ("b.png 160w 80h 80h", None),
        ("b.png 2q", None),
        ("b.png 1e400x", None),
        // A descriptor in parentheses keeps its commas.
        ("b.png 2x, c.png (foo, bar) 3x", shows("b.png", 2.0)),
        // A URL that ends in a comma ends its candidate.
        ("b.png,c.png 3x", shows("b.png,c.png", 3.0)),
        ("b.png,, c.png 2x", shows("b.png", 1.0)),
        (" , b.png 2x,", shows("b.png", 2.0)),
        ("  b.png   2x  ,c.png 3x ", shows("b.png", 2.0)),
    ] {
        assert_eq!(chosen(&[], &[("srcset", srcset)]), expected, "{srcset}");
    }
    // A src stands in when every candidate is dropped.
    for srcset in ["b.png 2x 3x", "b.png 1.5w", "b.png w"] {
        assert_eq!(
            chosen(&[], &[("src", "a.png"), ("srcset", srcset)]),
            shows("a.png", 1.0),
            "{srcset}"
        );
    }
}

#[test]
fn the_first_source_of_a_picture_that_applies_gives_the_candidates() {
    let picture = |source: Attributes<'_>| chosen(&[("source", source)], &[("src", "a.png")]);
    assert_eq!(picture(&[("srcset", "c.png 2x")]), shows("c.png", 2.0));
    for (name, value, applies) in [
        ("media", "(max-width:1px)", false),
        ("media", "(min-width: 100px)", true),
        ("media", "screen", true),
        ("media", "", true),
        ("media", "garbage!", false),
        ("type", "image/foo", false),
        ("type", "image/png", true),
        ("type", "IMAGE/PNG; q=1", true),
        ("type", "image/webp", true),
        ("type", "image/avif", false),
        ("type", "image/bmp", false),
        ("type", "image/x-icon", false),
        ("type", "", true),
    ] {
        let expected = if applies {
            shows("c.png", 2.0)
        } else {
            shows("a.png", 1.0)
        };
        assert_eq!(
            picture(&[("srcset", "c.png 2x"), (name, value)]),
            expected,
            "{name}={value}"
        );
    }
    assert_eq!(
        picture(&[("srcset", "c.png 120w"), ("sizes", "60px")]),
        shows("c.png", 2.0)
    );
    // A source that applies takes the place of the image's own srcset.
    assert_eq!(
        chosen(
            &[(
                "source",
                &[("srcset", "c.png 3x"), ("media", "(max-width:1px)")]
            )],
            &[("src", "a.png"), ("srcset", "b.png 2x")]
        ),
        shows("a.png", 1.0)
    );
    assert_eq!(
        chosen(
            &[("source", &[("srcset", "c.png 3x")])],
            &[("srcset", "b.png 2x")]
        ),
        shows("c.png", 3.0)
    );
    // Sources without candidates and other elements are passed over.
    assert_eq!(
        chosen(
            &[
                ("source", &[("srcset", "")]),
                ("source", &[("src", "c.png")]),
                ("div", &[("srcset", "c.png 3x")]),
                ("source", &[("srcset", "b.png 2x")]),
            ],
            &[("src", "a.png")]
        ),
        shows("b.png", 2.0)
    );
}

#[test]
fn only_sources_before_the_image_count() {
    let mut document = Document::new();
    let root = document.root_index();
    let picture = document.append_element(Some(root), "picture", None::<&str>);
    let image = document.append_element(Some(picture), "img", None::<&str>);
    document.set_element_attributes(image, vec![("src".into(), "a.png".into())]);
    let source = document.append_element(Some(picture), "source", None::<&str>);
    document.set_element_attributes(source, vec![("srcset".into(), "b.png 2x".into())]);
    let shown = image_source(&document, image, &screen()).unwrap();
    assert_eq!((shown.url, shown.density), ("a.png", 1.0));
    // An image whose parent is not a picture reads no source.
    let div = document.append_element(Some(root), "div", None::<&str>);
    let other = document.append_element(Some(div), "source", None::<&str>);
    document.set_element_attributes(other, vec![("srcset".into(), "b.png 2x".into())]);
    let lone = document.append_element(Some(div), "img", None::<&str>);
    assert_eq!(image_source(&document, lone, &screen()), None);
    assert_eq!(image_source(&document, usize::MAX, &screen()), None);
}

#[test]
fn a_print_environment_matches_print_media() {
    let (document, image) = image_in(
        &[("source", &[("srcset", "c.png 2x"), ("media", "print")])],
        &[("src", "a.png")],
    );
    let shown = image_source(&document, image, &MediaContext::print()).unwrap();
    assert_eq!((shown.url, shown.density), ("c.png", 2.0));
    let shown = image_source(&document, image, &screen()).unwrap();
    assert_eq!((shown.url, shown.density), ("a.png", 1.0));
}
