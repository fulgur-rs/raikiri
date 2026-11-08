use super::*;

/// Table rows of a few hundred bytes each.
fn rows(from: usize, count: usize) -> String {
    (from..from + count)
        .map(|row| format!("<tr><td>row {row}</td><td>cell {row}</td></tr>"))
        .collect()
}

/// A stream with a checkpoint every 500 bytes.
fn small_checkpoints<'r>(
    resources: &'r RenderResources<'r>,
    shared: Shared,
) -> StreamingLayout<'r, 'r, Shared> {
    StreamingLayout::new(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        shared,
    )
    .checkpoint_bytes(500)
}

/// Feed `html` in 100-byte pieces.
fn feed(stream: &mut StreamingLayout<'_, '_, Shared>, html: &str) {
    for piece in html.as_bytes().chunks(100) {
        stream.feed(piece).expect("feed");
    }
}

fn snapshots(stream: &StreamingLayout<'_, '_, Shared>) -> usize {
    stream.input.sink().snapshot_calls.get()
}

#[test]
fn checkpoints_are_skipped_while_only_an_open_table_grows() {
    let open = format!("<table>{}", rows(0, 300));
    let html = format!("{open}</table>{}", paragraphs(60));
    let expected = batch_pages(&html);

    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream = small_checkpoints(&resources, shared.clone());
    feed(&mut stream, &open);
    // The first checkpoint lays the table out and finds no final page; the
    // dozens after it only add rows to the table.
    assert!(open.len() / 500 > 20);
    assert_eq!(snapshots(&stream), 1);
    assert!(shared.0.borrow().is_empty());

    feed(&mut stream, &html[open.len()..]);
    assert!(
        !shared.0.borrow().is_empty(),
        "pages arrive again once the table closes"
    );
    completed(stream.finish().expect("finish"));
    assert_eq!(*shared.0.borrow(), expected);
}

#[test]
fn content_fostered_before_an_open_table_lays_out_again() {
    // A paragraph inside a table is inserted before the table, into content
    // that the skipped checkpoints would take for final.
    let first = format!("<table>{}", rows(0, 40));
    let fostered = "<p>fostered</p>";
    let rest = format!("{}</table>{}", rows(40, 40), paragraphs(20));
    let html = format!("{first}{fostered}{rest}");
    let expected = batch_pages(&html);

    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream = small_checkpoints(&resources, shared.clone());
    feed(&mut stream, &first);
    let before = snapshots(&stream);
    assert!(before >= 1);
    feed(&mut stream, fostered);
    stream.checkpoint().expect("checkpoint");
    assert_eq!(snapshots(&stream), before + 1);
    feed(&mut stream, &rest);
    completed(stream.finish().expect("finish"));
    assert_eq!(*shared.0.borrow(), expected);
}

#[test]
fn attributes_merged_into_body_lay_out_again() {
    // A repeated `<body>` tag inside the table adds its attributes to the
    // body element, before the table.
    let first = format!("<body><table>{}", rows(0, 40));
    let merged = "<body style='margin: 40px'>";
    let rest = format!("{}</table>{}", rows(40, 40), paragraphs(20));
    let html = format!("{first}{merged}{rest}");
    let expected = batch_pages(&html);

    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream = small_checkpoints(&resources, shared.clone());
    feed(&mut stream, &first);
    let before = snapshots(&stream);
    assert!(before >= 1);
    feed(&mut stream, merged);
    stream.checkpoint().expect("checkpoint");
    assert_eq!(snapshots(&stream), before + 1);
    feed(&mut stream, &rest);
    completed(stream.finish().expect("finish"));
    assert_eq!(*shared.0.borrow(), expected);
}
