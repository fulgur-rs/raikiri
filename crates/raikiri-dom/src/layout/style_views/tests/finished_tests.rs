use super::*;
use raikiri_layout_style::values::ExpandedDimension;

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
            let ExpandedDimension::Calc(calc) = view.size().width.expand() else {
                panic!("the width is a calc()");
            };
            let ExpandedDimension::Length(height) = view.size().height.expand() else {
                panic!("the height is a length");
            };
            (height, calc.resolve(100.0))
        })
        .expect("the box has a style record");
    assert_eq!(height, 7.0);
    assert_eq!(width_at_100, 60.0);
}

#[test]
fn a_box_without_a_style_record_reads_nothing() {
    let (doc, cascade, child, _) = fixture();
    // No session has prepared the document yet.
    assert!(doc.read_finished_style(&cascade, child, |_| ()).is_none());
}
