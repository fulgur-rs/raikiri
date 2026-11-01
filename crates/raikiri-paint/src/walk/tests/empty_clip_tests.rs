use super::*;
use crate::walk::paint_walk::scene_has_effects;
use std::sync::Arc;

#[test]
fn ordinary_empty_layers_and_clips_do_not_make_a_scope_effectful() {
    let mut scene = Scene::new();
    scene.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 10.0, 10.0));
    scene.push_layer(
        peniko::Mix::Normal,
        0.5,
        Affine::IDENTITY,
        &Rect::new(0.0, 0.0, 10.0, 10.0),
        None,
        None,
    );
    scene.pop_layer();
    scene.pop_layer();
    assert!(!scene_has_effects(&scene));
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        peniko::Color::BLACK,
        None,
        &Rect::new(0.0, 0.0, 1.0, 1.0),
    );
    assert!(scene_has_effects(&scene));
}

#[test]
fn empty_special_composition_and_filters_are_kept_conservatively() {
    for (blend, filter, backdrop) in [
        (peniko::BlendMode::from(peniko::Compose::Copy), None, None),
        (peniko::BlendMode::from(peniko::Mix::Multiply), None, None),
        (
            peniko::BlendMode::from(peniko::Mix::Normal),
            Some(Arc::new(anyrender::Filter::empty())),
            None,
        ),
        (
            peniko::BlendMode::from(peniko::Mix::Normal),
            None,
            Some(Arc::new(anyrender::Filter::empty())),
        ),
    ] {
        let mut scene = Scene::new();
        scene.push_layer(
            blend,
            1.0,
            Affine::IDENTITY,
            &Rect::new(0.0, 0.0, 10.0, 10.0),
            filter,
            backdrop,
        );
        scene.pop_layer();
        assert!(scene_has_effects(&scene));
    }
}
