use super::*;

const IMAGE: &str = "url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22%20viewBox=%220%200%2010%2010%22%3E%3Cpath%20fill=%22blue%22%20d=%22M0%200h10v10H0z%22/%3E%3C/svg%3E)";

#[test]
fn replaced_block_pseudos_apply_translation_and_noncentral_origins() {
    for pseudo in ["before", "after"] {
        for transform in ["translateY(10px)", "scale(.5,1.5)", "rotate(15deg)"] {
            let css = format!(
                "p::{pseudo}{{content:{IMAGE};display:block;width:40px;height:20px;\
                 background:red;transform:{transform};transform-origin:25% 100%}}"
            );
            assert!(assert_matches_paint("<p>text</p>", &css) > 0);
        }
    }
}

#[test]
fn replaced_block_origins_follow_each_page_slice() {
    for pseudo in ["before", "after"] {
        for transform in ["scaleY(.5)", "rotate(7deg)"] {
            let css = format!(
                "p::{pseudo}{{content:{IMAGE};display:block;width:40px;height:300px;\
                 background:red;transform:{transform};transform-origin:100% 100%}}"
            );
            assert!(assert_matches_paint("<p>text</p>", &css) >= 2);
        }
    }
}

#[test]
fn replaced_blocks_keep_relative_and_ancestor_transforms() {
    for pseudo in ["before", "after"] {
        let css = format!(
            "p::{pseudo}{{content:{IMAGE};display:block;width:40px;height:300px;\
             position:relative;left:3px;top:7px;background:red;\
             transform:scale(.75,.5);transform-origin:25% 75%}}"
        );
        assert!(
            assert_matches_paint(
                "<div style='transform:translate(4px,6px) rotate(2deg)'><p>text</p></div>",
                &css
            ) >= 2
        );
    }
}

fn assert_images_match(result: &DocumentLayout) -> usize {
    let mut checked = 0;
    for page in result.pages() {
        let (fills, _) = painted(&page, result.page_count());
        let (document, _, _, _) = page.paint_inputs();
        let mut transforms = vec![PaintTransform::IDENTITY];
        for event in page.paint_order() {
            match event {
                PaintEvent::PushTransform(own) => {
                    transforms.push(*transforms.last().unwrap() * own);
                }
                PaintEvent::PopTransform => {
                    transforms.pop().unwrap();
                }
                PaintEvent::GeneratedImage(image) => {
                    let (_, content) = document.page_generated_image(page.index(), image).unwrap();
                    let pseudo = match image.kind {
                        raikiri_html::GeneratedKind::After => raikiri_style::PseudoElem::After,
                        _ => raikiri_style::PseudoElem::Before,
                    };
                    let object = document
                        .content_image_object_rect(image.owner.0 as usize, pseudo, image.index)
                        .unwrap();
                    let rect = PaintRect::new(
                        content.x + object.x,
                        content.y + object.y,
                        object.width,
                        object.height,
                    );
                    let expected = corners(*transforms.last().unwrap(), rect);
                    assert!(
                        has_shape(&fills, &expected),
                        "page {} image {expected:?}, native fills {fills:?}",
                        page.index()
                    );
                    checked += 1;
                }
                _ => {}
            }
        }
        assert_eq!(transforms.len(), 1);
    }
    checked
}

#[test]
fn backgrounds_and_content_images_share_each_page_origin() {
    for pseudo in ["before", "after"] {
        for height in [20, 300] {
            for first_line in ["", "p::first-line{opacity:.5}"] {
                let css = format!(
                    "{first_line} p::{pseudo}{{content:{IMAGE};display:block;width:40px;\
                     height:{height}px;padding:2px;border:1px solid lime;\
                     background:red;object-fit:contain;object-position:25% 75%;\
                     position:relative;left:3px;top:7px;\
                     transform:scale(.75,.5);transform-origin:25% 75%}}"
                );
                assert!(assert_matches_paint("<p>text</p>", &css) > 0);
                let result = lay_out("<p>text</p>", &css);
                assert!(assert_images_match(&result) > 0);
            }
        }
    }
}

#[test]
fn a_singular_block_pseudo_hides_background_and_image() {
    let result = lay_out(
        "<p>text</p>",
        &format!(
            "p::before{{content:{IMAGE};display:block;width:40px;height:300px;\
         background:red;transform:scale(0)}}"
        ),
    );
    for page in result.pages() {
        assert!(!page.paint_order().iter().any(|event| matches!(
            event,
            PaintEvent::GeneratedBox(_) | PaintEvent::GeneratedImage(_)
        )));
        let (fills, _) = painted(&page, result.page_count());
        assert_eq!(
            fills.len(),
            1,
            "only page background, page {}",
            page.index()
        );
    }
}

#[test]
fn native_pixels_turn_the_background_and_contained_image_once() {
    for pseudo in ["before", "after"] {
        let result = lay_out(
            "<p>text</p>",
            &format!(
                "p::{pseudo}{{content:{IMAGE};display:block;width:40px;height:20px;\
             background:red;object-fit:contain;object-position:25% 75%;\
             transform:scale(.5);transform-origin:0 0}}"
            ),
        );
        let page = result.page(0).unwrap();
        let mut transform = PaintTransform::IDENTITY;
        let mut box_rect = None;
        for event in page.paint_order() {
            match event {
                PaintEvent::PushTransform(own) => transform = transform * own,
                PaintEvent::GeneratedBox(piece) => {
                    box_rect = Some(piece.rect);
                    break;
                }
                _ => {}
            }
        }
        let rect = box_rect.unwrap();
        let scene = native_scene(&page, result.page_count());
        let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| {
                use anyrender::PaintScene;
                out.append_scene(scene, Affine::IDENTITY);
            },
            300,
            200,
        );
        for (offset, expected) in [(1.0, [255, 0, 0]), (15.0, [0, 0, 255])] {
            let (x, y) = transform.apply((rect.x + offset, rect.y + 10.0));
            let at = (y.floor() as usize * 300 + x.floor() as usize) * 4;
            assert_eq!(&pixels[at..at + 3], &expected, "{pseudo} offset {offset}");
        }
    }
}

#[test]
fn replaced_blocks_keep_vertical_and_repeated_ownership() {
    for context in [
        "html{writing-mode:vertical-rl}p{writing-mode:horizontal-tb;width:120px}",
        "p{position:fixed;top:0;left:0}",
    ] {
        let css = format!(
            "{context} p::before{{content:{IMAGE};display:block;width:40px;height:20px;\
             background:red;transform:rotate(7deg);transform-origin:25% 75%}}"
        );
        assert!(
            assert_matches_paint("<p>text</p><div style='height:300px'></div>", &css) > 0,
            "{context}"
        );
    }
}

#[test]
fn continuation_origins_use_the_fragment_bottom() {
    let result = lay_out(
        "<p>text</p>",
        &format!(
            "p::before{{content:{IMAGE};display:block;width:40px;height:300px;\
         background:red;transform:scaleY(.5);transform-origin:100% 100%}}"
        ),
    );
    assert_eq!(result.page_count(), 2);
    for (page, expected_y) in result.pages().zip([90.0, 85.0]) {
        let matrices: Vec<_> = page
            .paint_order()
            .into_iter()
            .filter_map(|event| {
                if let PaintEvent::PushTransform(matrix) = event {
                    Some(matrix)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(matrices.len(), 1);
        assert_eq!(matrices[0].d, 0.5);
        assert_eq!(matrices[0].f, expected_y);
    }
}
