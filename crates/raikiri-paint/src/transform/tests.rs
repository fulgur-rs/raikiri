use super::*;
use anyrender::{PaintScene, RenderContext, Scene};
use kurbo::Affine;

fn adapter(scene: &mut Scene) -> TransformScene<'_, Scene> {
    TransformScene {
        scene,
        transform: Affine::IDENTITY,
    }
}

#[test]
fn affine_adapter_reports_unimplemented_custom_resource() {
    let mut inner = Scene::new();
    let mut wrapped = adapter(&mut inner);
    let result = wrapped.try_register_custom_resource(Box::new(7_u32));
    assert!(
        result.is_err(),
        "affine adapter must forward the inner scene resource policy"
    );
}

#[test]
fn affine_adapter_unregister_keeps_scene_usable() {
    let mut inner = Scene::new();
    let id = anyrender::ResourceId::new();
    {
        let mut wrapped = adapter(&mut inner);
        wrapped.unregister_resource(id);
    }
    assert!(
        inner.commands.is_empty(),
        "unregister through the adapter must stay a no-op"
    );
}

#[test]
fn affine_adapter_has_no_renderer_specific_context() {
    let mut inner = Scene::new();
    let wrapped = adapter(&mut inner);
    assert!(
        wrapped.renderer_specific_context().is_none(),
        "affine adapter must forward the inner scene lack of renderer context"
    );
}

#[test]
fn affine_adapter_reset_clears_inner_scene() {
    let mut inner = Scene::new();
    inner.push_layer(
        peniko::BlendMode::default(),
        1.0,
        Affine::IDENTITY,
        &kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
        None,
        None,
    );
    assert!(!inner.commands.is_empty());
    adapter(&mut inner).reset();
    assert!(inner.commands.is_empty());
}
