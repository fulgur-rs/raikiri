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
fn negative_prefix_copies_only_inner_clips_and_reset_forgets_them() {
    let mut inner = Scene::new();
    let mut capture = CaptureScene::new(&mut inner);
    assert_eq!(capture.clip_prefix(0).1, 0);
    capture.track_clips();
    let rect = Rect::new(0.0, 0.0, 10.0, 20.0);
    capture.push_clip_layer(Affine::IDENTITY, &rect);
    let depth = capture.layer_depth();
    capture.push_layer(
        BlendMode::default(),
        0.5,
        Affine::IDENTITY,
        &rect,
        None,
        None,
    );
    capture.push_clip_layer(Affine::translate((3.0, 4.0)), &rect);
    let (prefix, count) = capture.clip_prefix(depth);
    assert_eq!(count, 1);
    assert_eq!(prefix.commands.len(), 1);
    let anyrender::recording::RenderCommand::PushClipLayer(clip) = &prefix.commands[0] else {
        panic!("expected only the inner clip");
    };
    assert_eq!(clip.transform, Affine::translate((3.0, 4.0)));
    capture.reset();
    assert_eq!(capture.layer_depth(), 0);
    assert_eq!(capture.clip_prefix(0).1, 0);
    assert!(capture.clip_prefix(0).0.commands.is_empty());
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
