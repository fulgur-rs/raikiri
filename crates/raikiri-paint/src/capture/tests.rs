use super::*;
use anyrender::{PaintScene, RenderContext, Scene};
use kurbo::{Affine, Rect};

fn fill(scene: &mut impl PaintScene) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::BLACK,
        None,
        &Rect::new(0.0, 0.0, 1.0, 1.0),
    );
}

#[test]
fn paint_goes_to_the_innermost_capture() {
    let mut inner = Scene::new();
    let mut capture = CaptureScene::new(&mut inner);
    fill(&mut capture);
    capture.begin_capture();
    fill(&mut capture);
    capture.begin_capture();
    fill(&mut capture);
    fill(&mut capture);
    let innermost = capture.end_capture();
    let outer = capture.end_capture();
    assert_eq!(innermost.commands.len(), 2);
    assert_eq!(outer.commands.len(), 1);
    capture.append_scene(innermost, Affine::IDENTITY);
    assert_eq!(inner.commands.len(), 3);
}

#[test]
fn an_unmatched_end_records_nothing() {
    let mut inner = Scene::new();
    let mut capture = CaptureScene::new(&mut inner);
    assert!(capture.end_capture().commands.is_empty());
}

#[test]
fn reset_drops_open_captures() {
    let mut inner = Scene::new();
    fill(&mut inner);
    let mut capture = CaptureScene::new(&mut inner);
    capture.begin_capture();
    fill(&mut capture);
    capture.reset();
    fill(&mut capture);
    assert_eq!(inner.commands.len(), 1);
}

#[test]
fn resources_and_context_come_from_the_wrapped_scene() {
    let mut inner = Scene::new();
    let mut capture = CaptureScene::new(&mut inner);
    assert!(
        capture
            .try_register_custom_resource(Box::new(7_u32))
            .is_err()
    );
    capture.unregister_resource(anyrender::ResourceId::new());
    assert!(capture.renderer_specific_context().is_none());
}
