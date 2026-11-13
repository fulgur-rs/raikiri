use super::check;

#[test]
fn empty_vertical_leaf_edges_sizes_and_contexts_follow_the_block_axis() {
    // Chromium 152, standards mode, 800x600 viewport, body margin 8px,
    // border-box divs. Each tuple records the measured container extent.
    for (name, extra, width, height) in [
        ("plain", "", 50.0, 50.0),
        (
            "max-clamped block size",
            "block-size:20px;max-block-size:0",
            50.0,
            50.0,
        ),
        ("inline padding", "padding-inline-start:5px", 50.0, 50.0),
        (
            "inline border",
            "border-top:3px solid transparent",
            50.0,
            50.0,
        ),
        ("block padding", "padding-block-start:5px", 65.0, 50.0),
        (
            "block border",
            "border-left:3px solid transparent",
            63.0,
            50.0,
        ),
        ("inline minimum", "min-inline-size:80px", 50.0, 80.0),
        ("block minimum", "min-block-size:5px", 65.0, 50.0),
        ("scroll container", "overflow:hidden", 60.0, 50.0),
        ("clip", "overflow:clip", 50.0, 50.0),
        ("flow root", "display:flow-root", 60.0, 50.0),
        ("layout containment", "contain:layout", 60.0, 50.0),
        ("ratio", "aspect-ratio:2", 50.0, 50.0),
        (
            "indefinite percent block size",
            "block-size:10%",
            50.0,
            50.0,
        ),
        (
            "percent inline padding",
            "padding-inline-start:10%",
            50.0,
            50.0,
        ),
        (
            "percent block padding",
            "padding-block-start:10%",
            65.0,
            50.0,
        ),
    ] {
        for (mode, right_to_left) in [
            ("vertical-lr", false),
            ("vertical-rl", true),
            ("sideways-lr", false),
            ("sideways-rl", true),
        ] {
            let body = format!(
                "<div id=b style='writing-mode:{mode}'>                <div style='block-size:10px;margin-block-end:20px'></div>                <div style='inline-size:50px;block-size:0;margin-block-start:30px;margin-block-end:5px;{extra}' data-case='{name}'></div>                <div id=c style='block-size:10px;margin-block-start:10px'></div></div>"
            );
            let x = if right_to_left {
                8.0
            } else {
                8.0 + width - 10.0
            };
            check(
                &body,
                &[
                    ("b", (8.0, 8.0, width, height)),
                    ("c", (x, 8.0, 10.0, height)),
                ],
            );
        }
    }
}

#[test]
fn negative_margins_share_the_vertical_context() {
    check(
        "<div id=b style='writing-mode:vertical-lr'>        <div style='block-size:10px;margin-right:20px'></div>        <div style='inline-size:50px;block-size:0;margin-left:-30px;margin-right:5px'></div>        <div id=c style='block-size:10px;margin-left:10px'></div></div>",
        &[("b", (8.0, 8.0, 10.0, 50.0)), ("c", (8.0, 8.0, 10.0, 50.0))],
    );
}

#[test]
fn nested_empty_block_collapses_through_on_the_block_axis() {
    // Chromium keeps the adjoining margins in one collapsed set even when
    // the empty vertical box has an empty block child.
    check(
        "<div id=b style='writing-mode:vertical-lr'>        <div style='block-size:10px;margin-right:20px'></div>        <div style='inline-size:50px;block-size:0;margin-left:30px;margin-right:5px'><div style='inline-size:50px;block-size:0'></div></div>        <div id=c style='block-size:10px;margin-left:10px'></div></div>",
        &[
            ("b", (8.0, 8.0, 50.0, 50.0)),
            ("c", (48.0, 8.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn other_formatting_contexts_and_nonempty_boxes_do_not_collapse_through() {
    for (tag, extra, content) in [
        ("img", "", ""),
        ("fieldset", "border:0;padding:0;min-inline-size:0", ""),
        ("div", "writing-mode:vertical-rl", ""),
        ("div", "writing-mode:horizontal-tb;width:0;height:50px", ""),
        ("div", "font:10px/10px Ahem", "z"),
    ] {
        check(
            &format!(
                "<div id=b style='writing-mode:vertical-lr'>            <div style='block-size:10px;margin-right:20px'></div>            <{tag} style='display:block;inline-size:50px;block-size:0;margin:0;margin-left:30px;margin-right:5px;{extra}'>{content}</{tag}>            <div id=c style='block-size:10px;margin-left:10px'></div></div>"
            ),
            &[
                ("b", (8.0, 8.0, 60.0, 50.0)),
                ("c", (58.0, 8.0, 10.0, 50.0)),
            ],
        );
    }
}

#[test]
fn size_containment_keeps_the_empty_block_axis_collapse_metadata() {
    // The empty measurement has the same block-axis collapse rules as an
    // ordinary empty box, independent of its nonzero inline size.
    check(
        "<div id=b style='writing-mode:vertical-lr'>        <div style='block-size:10px;margin-right:20px'></div>        <div style='inline-size:50px;block-size:0;margin-left:30px;margin-right:5px;contain:size'></div>        <div id=c style='block-size:10px;margin-left:10px'></div></div>",
        &[
            ("b", (8.0, 8.0, 50.0, 50.0)),
            ("c", (48.0, 8.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn nested_and_contained_blocks_keep_empty_and_nonempty_margin_rules() {
    // Chromium 152, embedded Ahem, 800x600 viewport and device scale 1.
    // Each tuple gives the empty/nonempty container width and its height.
    for (extra, empty_width, text_width, height) in [
        ("", 50.0, 60.0, 50.0),
        ("block-size:20px;max-block-size:0", 50.0, 60.0, 50.0),
        ("padding-inline-start:5px", 50.0, 60.0, 50.0),
        ("border-top:3px solid transparent", 50.0, 60.0, 50.0),
        ("padding-block-start:5px", 65.0, 65.0, 50.0),
        ("border-left:3px solid transparent", 63.0, 63.0, 50.0),
        ("min-inline-size:80px", 50.0, 60.0, 80.0),
        ("min-block-size:5px", 65.0, 65.0, 50.0),
        ("overflow:hidden", 60.0, 60.0, 50.0),
        ("overflow:clip", 50.0, 60.0, 50.0),
        ("display:flow-root", 60.0, 60.0, 50.0),
        ("contain:layout", 60.0, 60.0, 50.0),
        ("contain:size layout", 60.0, 60.0, 50.0),
        ("aspect-ratio:2", 50.0, 60.0, 50.0),
        ("block-size:10%", 50.0, 60.0, 50.0),
        ("padding-inline-start:10%", 50.0, 60.0, 50.0),
        ("padding-block-start:10%", 65.0, 65.0, 50.0),
        ("margin-block-start:-30px", 10.0, 20.0, 50.0),
    ] {
        for (mode, right_to_left) in [
            ("vertical-lr", false),
            ("vertical-rl", true),
            ("sideways-lr", false),
            ("sideways-rl", true),
        ] {
            for contained in [false, true] {
                for has_text in [false, true] {
                    // The nonempty cyclic-percentage case has a separate
                    // baseline control below; it is not browser-conformant.
                    if extra == "block-size:10%" && has_text {
                        continue;
                    }
                    let text = if has_text { "z" } else { "" };
                    let content = if contained {
                        text.to_owned()
                    } else {
                        format!("<div style='inline-size:50px;block-size:0'>{text}</div>")
                    };
                    let containment = if contained { ";contain:size" } else { "" };
                    let body = format!(
                        "<div id=b style='writing-mode:{mode}'><div style='block-size:10px;margin-block-end:20px'></div><div style='inline-size:50px;block-size:0;margin-block-start:30px;margin-block-end:5px{containment};{extra}'>{content}</div><div id=c style='block-size:10px;margin-block-start:10px'></div></div>"
                    );
                    let width = if has_text { text_width } else { empty_width };
                    let x = if right_to_left {
                        8.0
                    } else {
                        8.0 + width - 10.0
                    };
                    check(
                        &body,
                        &[
                            ("b", (8.0, 8.0, width, height)),
                            ("c", (x, 8.0, 10.0, height)),
                        ],
                    );
                }
            }
        }
    }
}

#[test]
fn nonempty_cyclic_percentage_blocks_size_as_auto() {
    // A percentage block size of a block whose container's block size the
    // content decides behaves as `auto`, so it does not grow the container
    // (CSS Sizing 3 §5.2.1), as in Chromium.
    for (mode, right_to_left) in [
        ("vertical-lr", false),
        ("vertical-rl", true),
        ("sideways-lr", false),
        ("sideways-rl", true),
    ] {
        for contained in [false, true] {
            let content = if contained {
                "z"
            } else {
                "<div style='inline-size:50px;block-size:0'>z</div>"
            };
            let containment = if contained { ";contain:size" } else { "" };
            let body = format!(
                "<div id=b style='writing-mode:{mode}'><div style='block-size:10px;margin-block-end:20px'></div><div style='inline-size:50px;block-size:10%;margin-block-start:30px;margin-block-end:5px{containment}'>{content}</div><div id=c style='block-size:10px;margin-block-start:10px'></div></div>"
            );
            let x = if right_to_left { 8.0 } else { 58.0 };
            check(
                &body,
                &[("b", (8.0, 8.0, 60.0, 50.0)), ("c", (x, 8.0, 10.0, 50.0))],
            );
        }
    }
}
