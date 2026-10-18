use super::*;
use taffy::Point;

#[test]
fn coordinate_translation_uses_containing_block_and_parent_origins() {
    let position = parent_local_position(
        Point { x: 10.0, y: 20.0 },
        Point { x: 3.0, y: 4.0 },
        Point { x: 5.0, y: 7.0 },
    );

    assert_eq!(position, Point { x: 8.0, y: 17.0 });
}
