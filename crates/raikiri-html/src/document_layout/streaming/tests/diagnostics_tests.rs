use super::*;

#[test]
fn diagnostics_include_checkpoint_copies_and_the_final_resume() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    )
    .checkpoint_bytes(512);
    let html = format!(
        "<!doctype html><style>p {{ break-after: page }}</style><body>{}</body>",
        "<p>snapshot text</p>".repeat(80)
    );
    stream.feed(html.as_bytes()).expect("feed");
    let expected_calls = stream.input.sink().snapshot_calls.get();
    let before_finish_resumes = stream.settings.resumed.get();
    let parser_nodes = crate::parse(html.as_bytes(), &crate::ParseOptions::new())
        .expect("parse")
        .dom
        .node_count();
    let summary = completed(stream.finish().expect("finish"));
    let stats = summary.diagnostics;
    assert_eq!(stats.snapshot_calls, expected_calls as u64);
    assert!(stats.snapshot_calls > 1);
    assert_eq!(stats.parser_nodes, parser_nodes as u64);
    assert!(stats.snapshot_nodes > stats.parser_nodes);
    assert!(stats.snapshot_copied_nodes < stats.snapshot_nodes);
    assert!(stats.resume_attempts >= stats.resumed_layouts);
    assert!(stats.resumed_layouts > before_finish_resumes as u64);
}

#[test]
fn diagnostics_without_a_checkpoint_still_report_the_final_parser() {
    let mut record = Record::default();
    let summary =
        completed(stream("<p>short</p>", 4, &mut record, LayoutConfig::default()).expect("stream"));
    assert!(summary.diagnostics.parser_nodes > 0);
    assert_eq!(summary.diagnostics.snapshot_calls, 0);
    assert_eq!(summary.diagnostics.snapshot_nodes, 0);
    assert_eq!(summary.diagnostics.snapshot_copied_nodes, 0);
    assert_eq!(summary.diagnostics.resume_attempts, 0);
    assert_eq!(summary.diagnostics.resumed_layouts, 0);
}
