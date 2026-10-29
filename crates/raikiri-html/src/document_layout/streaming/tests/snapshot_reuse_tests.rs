use super::*;

#[test]
fn source_snapshot_recycling_stops_for_late_fixed_and_absolute_content() {
    for position in ["fixed", "absolute"] {
        let resources = RenderResources::new();
        let mut record = Record::default();
        let mut stream = StreamingLayout::new(
            &resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            &mut record,
        )
        .checkpoint_bytes(1000);
        stream
            .feed(format!("<body style='position: relative'>{}", paragraphs(60)).as_bytes())
            .expect("prefix");
        assert!(stream.progress.delivered > 0);
        assert!(stream.recycled_snapshot.is_some());
        stream
            .feed(format!("<div style='position: {position}; top: 0'>late</div>").as_bytes())
            .expect("late content");
        stream.checkpoint().expect("late checkpoint");
        assert!(stream.recycled_snapshot.is_none());
        stream.feed(paragraphs(2).as_bytes()).expect("suffix");
        stream.checkpoint().expect("fallback checkpoint");
        assert!(stream.recycled_snapshot.is_none());
        let summary = completed(stream.finish().expect("finish"));
        let ignored = ignored(&summary);
        assert_eq!(ignored.len(), 1);
        let late = ignored[0].0.expect("late node");
        assert!(record.pages.iter().all(|(_, nodes)| !nodes.contains(&late)));
    }
}

#[test]
fn source_snapshot_recycling_recovers_after_a_missing_arena() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    )
    .checkpoint_bytes(1000);
    stream.feed(paragraphs(60).as_bytes()).expect("prefix");
    assert!(stream.recycled_snapshot.take().is_some());
    for _ in 0..2 {
        stream.feed(paragraphs(2).as_bytes()).expect("suffix");
        stream.checkpoint().expect("checkpoint");
        assert!(stream.recycled_snapshot.is_some());
    }
    let summary = completed(stream.finish().expect("finish"));
    assert_eq!(record.pages.len(), summary.page_count as usize);
    assert!(ignored(&summary).is_empty());
}
