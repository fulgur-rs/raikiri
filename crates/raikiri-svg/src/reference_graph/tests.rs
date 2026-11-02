use super::*;
use crate::SvgDocument;

const ROOT: &str = "xmlns='http://www.w3.org/2000/svg' \
                    xmlns:xlink='http://www.w3.org/1999/xlink' width='20' height='20'";

fn svg(root_attributes: &str, body: &str) -> String {
    format!("<svg {ROOT} {root_attributes}>{body}</svg>")
}

fn parse(source: &str) -> Result<SvgDocument, SvgError> {
    SvgDocument::parse(source.as_bytes())
}

fn excerpt(source: &str) -> &str {
    &source[..source.len().min(300)]
}

fn assert_refused(source: &str, expected: &str) {
    match parse(source) {
        Err(SvgError::InvalidDocument(message)) => {
            assert!(message.contains(expected), "{message}\n{}", excerpt(source));
        }
        other => panic!("expected `{expected}`, got {other:?}\n{}", excerpt(source)),
    }
}

fn assert_cycle(source: &str) {
    assert_refused(source, "form a cycle");
}

fn assert_too_deep(source: &str) {
    assert_refused(source, "chained too deeply");
}

fn assert_drawn(source: &str) {
    if let Err(error) = parse(source) {
        panic!("{error}\n{}", excerpt(source));
    }
}

/// A document of `count` resources, each referring to the next and the last
/// to the first, that a rectangle uses through `property`.
///
/// `resource(i, next)` is the markup of resource `i`, and `style(i, next)` the
/// rules of a style element.
fn ring(
    count: usize,
    property: &str,
    resource: impl Fn(usize, usize) -> String,
    style: impl Fn(usize, usize) -> String,
) -> String {
    let next = |index: usize| (index + 1) % count;
    let rules: String = (0..count).map(|index| style(index, next(index))).collect();
    let rules = if rules.is_empty() {
        String::new()
    } else {
        format!("<style>{rules}</style>")
    };
    let resources: String = (0..count)
        .map(|index| resource(index, next(index)))
        .collect();
    svg(
        "",
        &format!(
            "{rules}<defs>{resources}</defs>\
             <rect width='20' height='20' {property}='url(#r0)'/>"
        ),
    )
}

fn no_style(_: usize, _: usize) -> String {
    String::new()
}

/// One way a resource of a ring refers to the next.
struct Form {
    name: &'static str,
    /// The property the rectangle that draws the ring uses.
    property: &'static str,
    resource: fn(usize, usize) -> String,
}

fn forms() -> Vec<Form> {
    vec![
        Form {
            name: "mask in an element",
            property: "mask",
            resource: |i, j| {
                format!("<mask id='r{i}'><rect width='5' height='5' mask='url(#r{j})'/></mask>")
            },
        },
        Form {
            name: "mask on the mask",
            property: "mask",
            resource: |i, j| {
                format!("<mask id='r{i}' mask='url(#r{j})'><rect width='5' height='5'/></mask>")
            },
        },
        Form {
            name: "clip-path in an element",
            property: "clip-path",
            resource: |i, j| {
                format!(
                    "<clipPath id='r{i}'>\
                     <rect width='5' height='5' clip-path='url(#r{j})'/></clipPath>"
                )
            },
        },
        Form {
            name: "clip-path on the clipPath",
            property: "clip-path",
            resource: |i, j| {
                format!(
                    "<clipPath id='r{i}' clip-path='url(#r{j})'>\
                     <rect width='5' height='5'/></clipPath>"
                )
            },
        },
        Form {
            name: "fill",
            property: "fill",
            resource: |i, j| {
                format!(
                    "<pattern id='r{i}' width='5' height='5' patternUnits='userSpaceOnUse'>\
                     <rect width='5' height='5' fill='url(#r{j})'/></pattern>"
                )
            },
        },
        Form {
            name: "stroke",
            property: "fill",
            resource: |i, j| {
                format!(
                    "<pattern id='r{i}' width='5' height='5' patternUnits='userSpaceOnUse'>\
                     <rect width='5' height='5' fill='none' stroke='url(#r{j})'/></pattern>"
                )
            },
        },
        Form {
            name: "style attribute",
            property: "mask",
            resource: |i, j| {
                format!(
                    "<mask id='r{i}'><rect width='5' height='5' \
                     style='opacity:1; mask: url( \"#r{j}\" )'/></mask>"
                )
            },
        },
        Form {
            name: "use",
            property: "mask",
            resource: |i, j| {
                format!(
                    "<g id='g{i}'><rect width='5' height='5' mask='url(#r{j})'/></g>\
                     <mask id='r{i}'><use xlink:href='#g{i}'/></mask>"
                )
            },
        },
    ]
}

#[test]
fn a_cycle_of_three_or_more_resources_is_refused() {
    for count in [3, 4, 50] {
        for form in forms() {
            let source = ring(count, form.property, form.resource, no_style);
            assert_cycle(&source);
        }
        // A stylesheet that gives the rectangles inside the masks their mask.
        let source = ring(
            count,
            "mask",
            |i, _| format!("<mask id='r{i}'><rect class='c{i}' width='5' height='5'/></mask>"),
            |i, j| format!(".c{i} {{ mask: url(#r{j}) }}"),
        );
        assert_cycle(&source);
    }
}

#[test]
fn a_cycle_that_usvg_breaks_is_still_drawn() {
    // A resource that refers to itself, and two of one kind that refer to each
    // other, through the property `usvg` looks at, are cut by `usvg`.
    for count in [1, 2] {
        for form in forms() {
            let source = ring(count, form.property, form.resource, no_style);
            if let Err(error) = parse(&source) {
                panic!("{} in a ring of {count}: {error}", form.name);
            }
        }
        let source = ring(
            count,
            "mask",
            |i, _| format!("<mask id='r{i}'><rect class='c{i}' width='5' height='5'/></mask>"),
            |i, j| format!(".c{i} {{ mask: url(#r{j}) }}"),
        );
        assert_drawn(&source);
    }
}

#[test]
fn a_cycle_that_usvg_does_not_break_is_refused() {
    let pattern = |id: &str, content: &str| {
        format!(
            "<pattern id='{id}' width='5' height='5' patternUnits='userSpaceOnUse'>{content}</pattern>"
        )
    };
    let mask_and_pattern = svg(
        "",
        &format!(
            "<defs><mask id='a'><rect width='5' height='5' fill='url(#b)'/></mask>{}</defs>\
             <rect width='20' height='20' mask='url(#a)'/>",
            pattern("b", "<rect width='5' height='5' mask='url(#a)'/>")
        ),
    );
    assert_cycle(&mask_and_pattern);

    // Fill and stroke are looked at one after the other.
    let fill_and_stroke = svg(
        "",
        &format!(
            "<defs>{}{}</defs><rect width='20' height='20' fill='url(#a)'/>",
            pattern("a", "<rect width='5' height='5' fill='url(#b)'/>"),
            pattern("b", "<rect width='5' height='5' stroke='url(#a)'/>")
        ),
    );
    assert_cycle(&fill_and_stroke);

    // Three resources in which two pairs refer to one another are refused as a
    // whole, even though `usvg` could break each pair.
    let three_pairs = svg(
        "",
        "<defs>\
         <mask id='a'><rect mask='url(#b)'/><rect mask='url(#c)'/></mask>\
         <mask id='b'><rect mask='url(#a)'/></mask>\
         <mask id='c'><rect mask='url(#a)'/></mask></defs>\
         <rect width='20' height='20' mask='url(#a)'/>",
    );
    assert_cycle(&three_pairs);

    // An element that shares its `id` with a resource refers to both.
    let twin = svg(
        "",
        "<defs>\
         <mask id='a'><rect mask='url(#b)'/></mask>\
         <mask id='b'><rect mask='url(#c)'/></mask>\
         <mask id='c'><rect mask='url(#a)'/></mask>\
         <mask id='a'><rect/></mask></defs>\
         <rect width='20' height='20' mask='url(#b)'/>",
    );
    assert_cycle(&twin);
}

#[test]
fn a_pattern_that_inherits_a_paint_that_refers_to_it_is_refused() {
    let pattern = "<pattern id='a' width='5' height='5' patternUnits='userSpaceOnUse'>\
                   <rect width='5' height='5'/></pattern>";
    for (root, wrapper) in [
        ("fill='url(#a)'", "g"),
        ("stroke='url(#a)' stroke-width='1'", "g"),
        ("", "g fill='url(#a)'"),
        ("", "g style='fill: url(#a)'"),
    ] {
        let source = svg(
            root,
            &format!(
                "<{wrapper}><defs>{pattern}</defs></{}>\
                 <rect width='20' height='20' fill='url(#a)'/>",
                wrapper.split(' ').next().unwrap()
            ),
        );
        assert_cycle(&source);
    }
    let rule = svg(
        "",
        &format!(
            "<style>g {{ fill: url(#a) }}</style><g><defs>{pattern}</defs></g>\
             <rect width='20' height='20' fill='url(#a)'/>"
        ),
    );
    assert_cycle(&rule);
    // The paint of an ancestor reaches the next pattern, which uses the first.
    let through_another = svg(
        "",
        "<g fill='url(#b)'><defs>\
         <pattern id='a' width='5' height='5'><rect width='5' height='5'/></pattern></defs></g>\
         <defs><pattern id='b' width='5' height='5'><rect width='5' height='5' fill='url(#a)'/>\
         </pattern></defs><rect width='20' height='20' fill='url(#a)'/>",
    );
    assert_cycle(&through_another);
}

#[test]
fn paint_that_does_not_come_back_to_the_pattern_is_drawn() {
    let pattern = "<pattern id='a' width='5' height='5' patternUnits='userSpaceOnUse'>\
                   <rect width='5' height='5'/></pattern>";
    // The group that uses the pattern is not above it.
    assert_drawn(&svg(
        "",
        &format!("<defs>{pattern}</defs><g fill='url(#a)'><rect width='20' height='20'/></g>"),
    ));
    // A gradient has no content to refer back.
    assert_drawn(&svg(
        "fill='url(#g)'",
        "<defs><linearGradient id='g'><stop offset='0' stop-color='red'/></linearGradient>\
         </defs><rect width='20' height='20'/>",
    ));
    // The pattern sets its own paint, which `usvg` removes.
    assert_drawn(&svg(
        "",
        "<defs><pattern id='a' width='5' height='5' fill='url(#a)'><rect width='5' height='5'/>\
         </pattern></defs><rect width='20' height='20' fill='url(#a)'/>",
    ));
}

/// A group whose paint refers to the pattern `a` that it holds, with `content`
/// in the pattern and `style` before the group.
fn pattern_below_paint(group: &str, content: &str, style: &str) -> String {
    svg(
        "",
        &format!(
            "{style}<g {group}><defs>\
             <path id='s' d='M0 0h5v5z'/>\
             <pattern id='a' width='5' height='5' patternUnits='userSpaceOnUse'>{content}\
             </pattern></defs></g>\
             <rect width='20' height='20' fill='url(#a)'/>"
        ),
    )
}

#[test]
fn shapes_that_set_their_own_paint_do_not_inherit_the_paint_above_the_pattern() {
    for (group, content) in [
        ("fill='url(#a)'", "<rect width='5' height='5' fill='red'/>"),
        (
            "stroke='url(#a)'",
            "<rect width='5' height='5' stroke='red'/>",
        ),
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' style='fill: red'/>",
        ),
        (
            "style='fill: url(#a)'",
            "<g fill='red'><rect width='5' height='5'/><circle r='2'/></g>",
        ),
        (
            "fill='url(#a)' stroke='url(#a)'",
            "<rect width='5' height='5' fill='red' stroke='blue'/>",
        ),
        // The group paints nothing, and a `use` that sets the paint hands it
        // down to the copy.
        ("fill='url(#a)'", "<g/>"),
        ("fill='url(#a)'", "<use href='#s' fill='red'/>"),
    ] {
        assert_drawn(&pattern_below_paint(group, content, ""));
    }
}

#[test]
fn a_shape_that_may_inherit_the_paint_above_the_pattern_is_refused() {
    for (group, content, style) in [
        // `inherit` and an empty value are not paint of its own.
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' fill='inherit'/>",
            "",
        ),
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' fill=' '/>",
            "",
        ),
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' style='fill: inherit'/>",
            "",
        ),
        // The `style` attribute wins over the attribute, and an important
        // declaration over a later one.
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' fill='red' style='fill: inherit'/>",
            "",
        ),
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' style='fill: inherit !important; fill: red'/>",
            "",
        ),
        // Only one of the two properties is set.
        (
            "stroke='url(#a)'",
            "<rect width='5' height='5' fill='red'/>",
            "",
        ),
        // Only one of the shapes sets it.
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' fill='red'/><circle r='2'/>",
            "",
        ),
        // A shape that the pattern copies with a `use` takes it from the `use`.
        ("fill='url(#a)'", "<use href='#s'/>", ""),
        // A style rule may set the paint to `inherit`.
        (
            "fill='url(#a)'",
            "<rect width='5' height='5' fill='red'/>",
            "<style>rect { fill: inherit }</style>",
        ),
    ] {
        assert_cycle(&pattern_below_paint(group, content, style));
    }
}

/// Three resources that each sit in a group whose `property` refers to the
/// next, and that take it with `inherit`: `attributes` and `style` say how.
fn inherit_ring(tag: &str, property: &str, attributes: &str, style: &str) -> String {
    let groups: String = (0..3)
        .map(|index| {
            format!(
                "<g {property}='url(#r{})'><{tag} id='r{index}' {attributes}>\
                 <rect width='5' height='5'/></{tag}></g>",
                (index + 1) % 3
            )
        })
        .collect();
    svg(
        "",
        &format!(
            "{style}<defs>{groups}</defs><rect width='20' height='20' {property}='url(#r0)'/>"
        ),
    )
}

#[test]
fn a_reference_that_inherit_takes_from_the_parent_is_followed() {
    for (tag, property) in [("mask", "mask"), ("clipPath", "clip-path")] {
        for (attributes, style) in [
            (format!("{property}='inherit'"), String::new()),
            (format!("style='{property}: Inherit'"), String::new()),
            (
                String::new(),
                format!("<style>{tag} {{ {property}: inherit }}</style>"),
            ),
        ] {
            assert_cycle(&inherit_ring(tag, property, &attributes, &style));
        }
        // Without `inherit` the groups hold no reference of the resources.
        assert_drawn(&inherit_ring(tag, property, "", ""));
        // `inherit` takes no reference where nothing above has one, and one that
        // leads on to a resource that holds none closes no cycle.
        assert_drawn(&svg(
            "",
            &format!(
                "<defs><{tag} id='a' {property}='inherit'><rect width='5' height='5'/></{tag}>\
                 <g {property}='url(#c)'><{tag} id='b' {property}='inherit'/></g>\
                 <{tag} id='c'/></defs>\
                 <rect width='20' height='20' {property}='url(#a)'/>"
            ),
        ));
    }
}

#[test]
fn resources_that_share_an_id_cannot_make_more_edges_than_the_check_may_spend() {
    let masks: String = (0..2000).map(|_| "<mask id='a' mask='url(#a)'/>").collect();
    assert_refused(
        &svg(
            "",
            &format!("{masks}<rect width='20' height='20' mask='url(#a)'/>"),
        ),
        "resource limit exceeded",
    );
    // A couple that share an id are still looked at, and are `usvg`'s to break.
    assert_drawn(&svg(
        "",
        "<mask id='a' mask='url(#a)'/><mask id='a' mask='url(#a)'/>\
         <rect width='20' height='20' mask='url(#a)'/>",
    ));
}

#[test]
fn a_pattern_with_content_of_its_own_ignores_the_content_of_the_one_it_names() {
    let pattern = |extra: &str, content: &str| {
        format!(
            "<pattern {extra} width='5' height='5' patternUnits='userSpaceOnUse'>{content}</pattern>"
        )
    };
    let back = "<rect width='5' height='5' fill='url(#p)'/>";
    let own = "<rect width='5' height='5' fill='red'/>";
    let drawn = |first: &str, second: &str, third: &str| {
        svg(
            "",
            &format!(
                "<defs>{first}{second}{third}</defs><rect width='20' height='20' fill='url(#p)'/>"
            ),
        )
    };
    // `usvg` draws the content of `p` and leaves `q` out.
    assert_drawn(&drawn(
        &pattern("id='p' href='#q'", own),
        &pattern("id='q'", back),
        "",
    ));
    // The first pattern with content in the chain is the one that counts.
    assert_drawn(&drawn(
        &pattern("id='p' href='#q'", ""),
        &pattern("id='q' href='#r'", own),
        &pattern("id='r'", back),
    ));
    // With none before it, the pattern that refers back is the one that counts.
    assert_cycle(&drawn(
        &pattern("id='p' href='#q'", ""),
        &pattern("id='q' href='#r'", ""),
        &pattern("id='r'", back),
    ));
}

#[test]
fn a_pattern_without_content_takes_the_references_of_the_one_it_names() {
    let pattern = |extra: &str, content: &str| {
        format!(
            "<pattern {extra} width='5' height='5' patternUnits='userSpaceOnUse'>{content}</pattern>"
        )
    };
    for href in ["href", "xlink:href"] {
        let cycle = svg(
            "",
            &format!(
                "<defs>{}{}</defs><rect width='20' height='20' fill='url(#p)'/>",
                pattern(&format!("id='p' {href}='#q'"), ""),
                pattern("id='q'", "<rect width='5' height='5' fill='url(#p)'/>")
            ),
        );
        assert_cycle(&cycle);
    }
    // Names that lead round to the first are the business of `usvg`.
    let round = svg(
        "",
        &format!(
            "<defs>{}{}{}</defs><rect width='20' height='20' fill='url(#p)'/>",
            pattern("id='p' href='#q'", ""),
            pattern("id='q' href='#r'", ""),
            pattern("id='r' href='#p'", ""),
        ),
    );
    assert_drawn(&round);
    // A name that is not a pattern, and a pattern that is not named, are left.
    assert_drawn(&svg(
        "",
        &format!(
            "<defs><mask id='m'/>{}</defs><rect width='20' height='20' fill='url(#p)'/>",
            pattern("id='p' href='#m'", "<rect width='5' height='5'/>")
        ),
    ));
    assert_drawn(&svg(
        "",
        &format!(
            "<defs>{}</defs><rect width='20' height='20' fill='url(#p)'/>",
            pattern("id='p' href='#missing'", "<rect width='5' height='5'/>")
        ),
    ));
}

#[test]
fn a_chain_of_references_is_bounded_by_its_depth() {
    let chain = |count: usize, resource: fn(usize, usize) -> String, property: &str| {
        let resources: String = (0..=count).map(|i| resource(i, i + 1)).collect();
        svg(
            "",
            &format!(
                "<defs>{resources}</defs><rect width='20' height='20' {property}='url(#r0)'/>"
            ),
        )
    };
    let masks: fn(usize, usize) -> String =
        |i, j| format!("<mask id='r{i}'><rect width='5' height='5' mask='url(#r{j})'/></mask>");
    let clips: fn(usize, usize) -> String = |i, j| {
        format!(
            "<clipPath id='r{i}'><rect width='5' height='5' clip-path='url(#r{j})'/></clipPath>"
        )
    };
    let patterns: fn(usize, usize) -> String = |i, j| {
        format!(
            "<pattern id='r{i}' width='5' height='5' patternUnits='userSpaceOnUse'>\
             <rect width='5' height='5' fill='url(#r{j})'/></pattern>"
        )
    };
    let markers: fn(usize, usize) -> String = |i, j| {
        format!(
            "<marker id='r{i}' markerWidth='5' markerHeight='5'>\
             <path d='M0 0L5 5' stroke='black' marker-start='url(#r{j})'/></marker>"
        )
    };

    // Chains that a legitimate document might make are drawn.
    for (resource, property) in [(masks, "mask"), (clips, "clip-path"), (patterns, "fill")] {
        assert_drawn(&chain(20, resource, property));
    }
    assert_drawn(&svg(
        "",
        &format!(
            "<defs>{}</defs><path d='M0 0L10 10' stroke='black' marker-start='url(#r0)'/>",
            (0..=20).map(|i| markers(i, i + 1)).collect::<String>()
        ),
    ));
    // Chains of the length that overflows the stack of a thread are not.
    for (resource, property) in [
        (masks, "mask"),
        (clips, "clip-path"),
        (patterns, "fill"),
        (markers, "marker-start"),
    ] {
        assert_too_deep(&chain(5000, resource, property));
    }
}

#[test]
fn the_nesting_in_a_chain_adds_to_its_depth() {
    let nested = |hops: usize| {
        let resources: String = (0..=hops)
            .map(|i| {
                format!(
                    "<mask id='r{i}'>{}<rect width='5' height='5' mask='url(#r{})'/>{}</mask>",
                    "<g>".repeat(100),
                    i + 1,
                    "</g>".repeat(100)
                )
            })
            .collect();
        svg(
            "",
            &format!("<defs>{resources}</defs><rect width='20' height='20' mask='url(#r0)'/>"),
        )
    };
    assert_drawn(&nested(3));
    assert_too_deep(&nested(5));
}

#[test]
fn a_cycle_through_a_marker_is_drawn_when_it_is_short() {
    let marker = |id: &str, inside: &str| {
        format!(
            "<marker id='{id}' markerWidth='5' markerHeight='5'>\
             <path d='M0 0L5 5' stroke='black' {inside}/></marker>"
        )
    };
    let path = "<path d='M0 0L10 10' stroke='black' marker-start='url(#a)'/>";
    // A marker that holds itself, two that hold each other, and three in a ring.
    assert_drawn(&svg(
        "",
        &format!(
            "<defs>{}</defs>{path}",
            marker("a", "marker-start='url(#a)'")
        ),
    ));
    assert_drawn(&svg(
        "",
        &format!(
            "<defs>{}{}</defs>{path}",
            marker("a", "marker-mid='url(#b)'"),
            marker("b", "marker-end='url(#a)'")
        ),
    ));
    assert_drawn(&svg(
        "",
        &format!(
            "<defs>{}{}{}</defs>{path}",
            marker("a", "marker-start='url(#b)'"),
            marker("b", "marker-start='url(#c)'"),
            marker("c", "marker-start='url(#a)'")
        ),
    ));
    // A marker and a mask that use one another.
    assert_drawn(&svg(
        "",
        &format!(
            "<defs><mask id='m'><path d='M0 0L5 5' stroke='white' marker-start='url(#a)'/></mask>\
             {}</defs><rect width='20' height='20' mask='url(#m)'/>",
            marker("a", "mask='url(#m)'")
        ),
    ));
}

#[test]
fn a_long_cycle_through_markers_is_deep() {
    let ring = |count: usize| {
        let markers: String = (0..count)
            .map(|i| {
                format!(
                    "<marker id='r{i}' markerWidth='5' markerHeight='5'>\
                     <path d='M0 0L5 5' stroke='black' marker-start='url(#r{})'/></marker>",
                    (i + 1) % count
                )
            })
            .collect();
        svg(
            "",
            &format!(
                "<defs>{markers}</defs><path d='M0 0L10 10' stroke='black' marker-start='url(#r0)'/>"
            ),
        )
    };
    assert_drawn(&ring(20));
    assert_too_deep(&ring(200));
}

#[test]
fn a_cycle_among_other_resources_is_refused_beside_a_marker() {
    // The marker is in the same component as a mask and a pattern that use one
    // another, and it does not stop them.
    let source = svg(
        "",
        "<defs>\
         <mask id='a'><rect mask='url(#m)' fill='url(#p)'/></mask>\
         <pattern id='p' width='5' height='5'><rect width='5' height='5' mask='url(#a)'/></pattern>\
         <marker id='m' markerWidth='5' markerHeight='5'>\
         <path d='M0 0L5 5' stroke='black' mask='url(#a)'/></marker></defs>\
         <rect width='20' height='20' mask='url(#a)'/>",
    );
    assert_cycle(&source);
}

#[test]
fn references_are_found_where_a_document_draws_them() {
    // Rules apply to the elements they match and to no others.
    let selective = svg(
        "",
        "<style>.outside { mask: url(#b) } #b rect { fill: red } @media print { rect { x: 1 } }</style>\
         <defs>\
         <mask id='a'><rect width='5' height='5'/></mask>\
         <mask id='b'><rect width='5' height='5' class='inside'/></mask>\
         <mask id='c'><rect width='5' height='5' mask='url(#a)'/></mask></defs>\
         <rect class='outside' width='20' height='20'/>\
         <rect width='20' height='20' mask='url(#c)'/>",
    );
    assert_drawn(&selective);
    // A rule reaches the elements its combinators select, inside the resource.
    let descendant = svg(
        "",
        "<style>#a rect { mask: url(#b) } #b rect { mask: url(#c) } #c rect { mask: url(#a) }</style>\
         <defs>\
         <mask id='a'><rect width='5' height='5'/></mask>\
         <mask id='b'><rect width='5' height='5'/></mask>\
         <mask id='c'><rect width='5' height='5'/></mask></defs>\
         <rect width='20' height='20' mask='url(#a)'/>",
    );
    assert_cycle(&descendant);
    // Resources that a clipping group uses by class are what an exporter writes.
    let exported = svg(
        "",
        "<style>.st0 { clip-path: url(#clip1) } .st1 { clip-path: url(#clip2) }\
         .st2 { clip-path: url(#clip3) }</style>\
         <defs><path id='shape' d='M0 0L10 0L10 10z'/>\
         <clipPath id='clip1'><use xlink:href='#shape' style='overflow:visible'/></clipPath>\
         <clipPath id='clip2'><use xlink:href='#shape' style='overflow:visible'/></clipPath>\
         <clipPath id='clip3'><use xlink:href='#shape' style='overflow:visible'/></clipPath></defs>\
         <g class='st0'><rect width='20' height='20'/></g>\
         <g class='st1'><rect width='20' height='20'/></g>\
         <g class='st2'><rect width='20' height='20'/></g>",
    );
    assert_drawn(&exported);
}

#[test]
fn many_elements_may_share_a_resource() {
    let users: String = (0..200)
        .map(|_| "<rect width='5' height='5' mask='url(#a)' clip-path='url(#b)'/>")
        .collect();
    assert_drawn(&svg(
        "",
        &format!(
            "<defs><mask id='a'><rect width='5' height='5' clip-path='url(#b)'/></mask>\
             <clipPath id='b'><rect width='5' height='5'/></clipPath></defs>{users}"
        ),
    ));
}

#[test]
fn a_use_inside_a_resource_is_followed_only_to_what_is_drawn() {
    // A mask is not drawn by a use, and a symbol is.
    let source = svg(
        "",
        "<defs><mask id='m'><use href='#m'/><use href='#nothing'/><use href='#s'/></mask>\
         <symbol id='s'><rect width='5' height='5' mask='url(#n)'/></symbol>\
         <mask id='n'><rect width='5' height='5'/></mask></defs>\
         <rect width='20' height='20' mask='url(#m)'/>",
    );
    assert_drawn(&source);

    // Uses that lead round to each other end, with no help from the preflight
    // that stops them before this runs.
    let source = svg(
        "",
        "<g id='a'><use href='#b'/></g><g id='b'><use href='#a'/></g>\
         <mask id='m'><use href='#a'/></mask>",
    );
    let xml = roxmltree::Document::parse(&source).unwrap();
    check_with_visit_limit(&xml, 1000).unwrap();
}

#[test]
fn a_document_without_resources_is_not_looked_at() {
    let source = svg("", "<rect width='5' height='5'/>");
    let xml = roxmltree::Document::parse(&source).unwrap();
    // Not even a visit is needed.
    check_with_visit_limit(&xml, 0).unwrap();
}

#[test]
fn the_work_of_following_references_is_bounded() {
    let source = svg(
        "",
        "<mask id='m'><rect width='5' height='5'/><rect width='5' height='5'/></mask>",
    );
    let mask = roxmltree::Document::parse(&source).unwrap();
    // The mask and its two rectangles take three visits.
    assert!(check_with_visit_limit(&mask, 2).is_err());
    check_with_visit_limit(&mask, 3).unwrap();

    // Each of the patterns is a visit, and so is each step along the names.
    let source = svg(
        "",
        &(0..5)
            .map(|i| format!("<pattern id='p{i}' href='#p{}'/>", i + 1))
            .collect::<String>(),
    );
    let chain = roxmltree::Document::parse(&source).unwrap();
    assert!(check_with_visit_limit(&chain, 4).is_err());
    assert!(check_with_visit_limit(&chain, 5).is_err());
    check_with_visit_limit(&chain, 100).unwrap();
}

#[test]
fn a_property_is_known_by_any_spelling_of_its_name() {
    assert_eq!(Property::from_name("MASK"), Some(Property::Mask));
    assert_eq!(Property::from_name("Clip-Path"), Some(Property::ClipPath));
    assert_eq!(Property::from_name("marker-END"), Some(Property::Marker));
    assert_eq!(Property::from_name("marker"), Some(Property::Marker));
    assert_eq!(Property::from_name("opacity"), None);
}

#[test]
fn the_ids_of_a_value_are_read_as_usvg_reads_them() {
    use Property::{ClipPath, Fill, Mask, Stroke};
    // A reference is the whole of the value.
    assert_eq!(target_id(Mask, "url(#a)"), Some("a"));
    assert_eq!(target_id(ClipPath, " url( \"#c\" ) "), Some("c"));
    assert_eq!(target_id(Mask, "url(#a) url(#b)"), None);
    assert_eq!(target_id(Mask, "url(#e"), None);
    assert_eq!(target_id(Mask, "url(other.svg#f)"), None);
    assert_eq!(target_id(Mask, "none"), None);
    // The name of the function is lower case, as `usvg` reads it.
    assert_eq!(target_id(Mask, "URL(#a)"), None);
    // A quoted id runs to the closing quote, whatever is in it.
    assert_eq!(target_id(Mask, "url('#b)')"), Some("b)"));
    assert_eq!(
        target_id(ClipPath, "url( '# spaced id ' )"),
        Some(" spaced id")
    );
    assert_eq!(target_id(Mask, "url('#a\")"), None);
    // A paint is a reference with a fallback after it, or something else.
    assert_eq!(target_id(Fill, "url(#p) red"), Some("p"));
    assert_eq!(target_id(Stroke, "url('#q)') none"), Some("q)"));
    assert_eq!(target_id(Fill, "red"), None);
    assert_eq!(target_id(Stroke, "none"), None);
}

#[test]
fn a_reference_whose_id_has_a_parenthesis_in_it_is_followed() {
    let ring: String = ["a)", "b)", "c)"]
        .iter()
        .zip(["b)", "c)", "a)"])
        .map(|(id, next)| {
            format!("<mask id=\"{id}\" mask=\"url('#{next}')\"><rect width='5' height='5'/></mask>")
        })
        .collect();
    assert_cycle(&svg(
        "",
        &format!("<defs>{ring}</defs><rect width='20' height='20' mask=\"url('#a)')\"/>"),
    ));
}

#[test]
fn attributes_in_the_namespaces_usvg_reads_are_followed() {
    let namespaces = "xmlns:s='http://www.w3.org/2000/svg' \
                      xmlns:x='http://www.w3.org/1999/xlink'";
    for (prefix, id) in [("s:", "id"), ("x:", "id"), ("s:", "s:id")] {
        let masks: String = (0..3)
            .map(|index| {
                let id_attribute = if id == "id" {
                    format!("id='r{index}'")
                } else {
                    format!("{id}='r{index}'")
                };
                format!(
                    "<mask {id_attribute} {prefix}mask='url(#r{})'>\
                     <rect width='5' height='5'/></mask>",
                    (index + 1) % 3
                )
            })
            .collect();
        let source = svg(
            namespaces,
            &format!("<defs>{masks}</defs><rect width='20' height='20' mask='url(#r0)'/>"),
        );
        assert_cycle(&source);
    }
    // The same goes for an `id` that is not in the namespace of the others.
    let masks: String = (0..3)
        .map(|index| {
            format!(
                "<mask s:id='r{index}' mask='url(#r{})'><rect width='5' height='5'/></mask>",
                (index + 1) % 3
            )
        })
        .collect();
    assert_cycle(&svg(
        namespaces,
        &format!("<defs>{masks}</defs><rect width='20' height='20' mask='url(#r0)'/>"),
    ));
    // An attribute in another namespace is not one `usvg` reads.
    assert_drawn(&svg(
        "xmlns:o='urn:other'",
        "<defs><mask id='a' o:mask='url(#a)'/></defs><rect width='20' height='20' mask='url(#a)'/>",
    ));
}

#[test]
fn references_that_host_styles_add_are_checked_before_drawing() {
    use crate::{SvgElementStyle, SvgResolvedStyles, SvgRootFont, SvgRootStyle, SvgViewport};

    // Three patterns whose content takes its fill from the host.
    let source = svg(
        "",
        "<defs>\
         <pattern id='p1' width='5' height='5'><rect width='5' height='5'/></pattern>\
         <pattern id='p2' width='5' height='5'><circle r='2'/></pattern>\
         <pattern id='p3' width='5' height='5'><path d='M0 0h5v5z'/></pattern></defs>\
         <rect width='20' height='10' fill='url(#p1)'/>",
    );
    let document = parse(&source).unwrap();
    type Drawn = (Result<(), SvgError>, Result<(), crate::SvgSceneError>);
    fn draw(document: &SvgDocument, elements: &[SvgElementStyle<'_>]) -> Drawn {
        let viewport = SvgViewport {
            width: 20.0,
            height: 10.0,
        };
        let resolved = || SvgResolvedStyles {
            root_color: [0, 0, 0, 255],
            font: SvgRootFont {
                size: 12.0,
                family: "sans-serif",
                weight: 400.0,
                style: "normal",
            },
            elements,
        };
        let raster = document.rasterize_at_css_pixel_scale_with_resolved_styles(
            viewport,
            SvgRootStyle::default(),
            resolved(),
            None,
        );
        let scene =
            document.scene_with_resolved_styles(viewport, SvgRootStyle::default(), resolved());
        (raster.map(drop), scene.map(drop))
    }
    // The indices count the svg element as 0, then the defs, p1, its rect, p2,
    // its circle, p3 and its path.
    let rect = SvgElementStyle {
        element_index: 3,
        declarations: "fill:url(#p2)",
    };
    let circle = SvgElementStyle {
        element_index: 5,
        declarations: "fill:url(#p3)",
    };
    let path = SvgElementStyle {
        element_index: 7,
        declarations: "fill:url(#p1)",
    };
    let (raster, scene) = draw(&document, &[rect, circle, path]);
    assert!(matches!(
        raster,
        Err(SvgError::InvalidDocument(message)) if message.contains("form a cycle")
    ));
    assert!(matches!(
        scene,
        Err(crate::SvgSceneError::Svg(SvgError::InvalidDocument(message)))
            if message.contains("form a cycle")
    ));

    // The same host styles without the last one chain the patterns. The raster
    // draws them, and the scene leaves patterns to the raster.
    let (raster, scene) = draw(&document, &[rect, circle]);
    raster.unwrap();
    assert!(matches!(scene, Err(crate::SvgSceneError::Unsupported(_))));

    // The source handed to a consumer outside the crate is checked too.
    let export = |elements: &[SvgElementStyle<'_>]| {
        document.styled_source_with_resolved_styles(
            SvgViewport {
                width: 20.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
            SvgRootFont {
                size: 12.0,
                family: "sans-serif",
                weight: 400.0,
                style: "normal",
            },
            elements,
        )
    };
    assert!(matches!(
        export(&[rect, circle, path]),
        Err(SvgError::InvalidDocument(message)) if message.contains("form a cycle")
    ));
    export(&[rect, circle]).unwrap();
}

#[test]
fn a_reference_to_an_element_of_another_kind_is_not_one_that_usvg_follows() {
    // A mask is not a clip path, and a clip path is not a pattern: none of
    // these references is followed, so they close no cycle.
    let source = svg(
        "",
        "<defs>\
         <mask id='m'><rect width='5' height='5' clip-path='url(#c)'/></mask>\
         <clipPath id='c'><rect width='5' height='5' fill='url(#p)'/></clipPath>\
         <pattern id='p' width='5' height='5'><rect width='5' height='5' mask='url(#m)'/></pattern>\
         </defs><rect width='20' height='20' mask='url(#m)'/>",
    );
    // `clip-path` goes to a clipPath, so m reaches c, and c's `fill` goes to a
    // pattern, so c reaches p, and p's `mask` goes to a mask: a real ring.
    assert_cycle(&source);
    let source = svg(
        "",
        "<defs>\
         <mask id='m'><rect width='5' height='5' fill='url(#c)'/></mask>\
         <clipPath id='c'><rect width='5' height='5' mask='url(#p)'/></clipPath>\
         <pattern id='p' width='5' height='5'><rect width='5' height='5' clip-path='url(#m)'/></pattern>\
         </defs><rect width='20' height='20' mask='url(#m)'/>",
    );
    assert_drawn(&source);
}

#[test]
fn a_pattern_chain_cannot_multiply_the_references_it_copies() {
    // Every pattern names the next and holds the same references, so the
    // content that each takes from the ones below grows with their number.
    let patterns: String = (0..40)
        .map(|i| {
            let references: String = (0..40)
                .map(|j| format!("<rect fill='url(#t{j})'/>"))
                .collect();
            format!(
                "<pattern id='p{i}' href='#p{}'>{references}</pattern>",
                i + 1
            )
        })
        .collect();
    let targets: String = (0..40)
        .map(|j| format!("<pattern id='t{j}'><rect/></pattern>"))
        .collect();
    let source = svg("", &format!("{patterns}{targets}"));
    let xml = roxmltree::Document::parse(&source).unwrap();
    check_with_visit_limit(&xml, 1_000_000).unwrap();
    // The 1,600 elements are visited, and the references that are copied are
    // more work on top.
    assert!(check_with_visit_limit(&xml, 2_500).is_err());
}

/// Parses `source` on a thread of `stack` bytes, which aborts the process
/// when the parse needs more.
fn parse_with_stack(source: String, stack: usize) -> Result<(), SvgError> {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || parse(&source).map(drop))
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn the_deepest_chain_that_is_drawn_fits_in_a_small_stack() {
    // The depth limit is meant to leave room on the smallest stack of a thread
    // that a host starts, with the unoptimized frames of a debug build.
    const STACK: usize = 1 << 20;
    let simple = |hops: usize| {
        let masks: String = (0..=hops)
            .map(|i| {
                format!(
                    "<mask id='r{i}'><rect width='5' height='5' fill='white' \
                     mask='url(#r{})'/></mask>",
                    i + 1
                )
            })
            .collect();
        svg(
            "",
            &format!("<defs>{masks}</defs><rect width='20' height='20' mask='url(#r0)'/>"),
        )
    };
    // Each hop costs five levels here: four for the reference and one for the
    // rectangle that makes it.
    parse_with_stack(simple(MAX_REFERENCE_LEVELS / 5), STACK).unwrap();
    assert!(parse_with_stack(simple(MAX_REFERENCE_LEVELS / 5 + 1), STACK).is_err());

    // The same depth made of nesting: 4 hops of 100 groups, 105 levels each.
    let nested = {
        let masks: String = (0..=4)
            .map(|i| {
                format!(
                    "<mask id='r{i}'>{}<rect width='5' height='5' fill='white' \
                     mask='url(#r{})'/>{}</mask>",
                    "<g>".repeat(100),
                    i + 1,
                    "</g>".repeat(100)
                )
            })
            .collect();
        svg(
            "",
            &format!("<defs>{masks}</defs><rect width='20' height='20' mask='url(#r0)'/>"),
        )
    };
    parse_with_stack(nested, STACK).unwrap();
}
