use super::*;

#[test]
fn a_finished_box_is_read_after_its_session_ends() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:calc(50% + 10px);height:7px");
    {
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    }
    let (height, width_at_100) = doc
        .read_finished_style(&cascade, child, |view| {
            let handle = view.size().width.into_raw().calc_value();
            (
                view.size().height.into_raw().value(),
                view.resolve_calc_handle(handle, 100.0),
            )
        })
        .expect("the box has a style record");
    assert_eq!(height, 7.0);
    assert_eq!(width_at_100, Some(60.0));
}

#[test]
fn a_box_without_a_style_record_reads_nothing() {
    let (doc, cascade, child, _) = fixture();
    // No session has prepared the document yet.
    assert!(doc.read_finished_style(&cascade, child, |_| ()).is_none());
}
