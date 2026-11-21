//! A border with rounded corners whose sides are not painted alike, or whose
//! style is drawn in bands (`double`, `groove`, `ridge`, `inset`, `outset`),
//! follows the curves of the corners (CSS Backgrounds 3 §4.2, §5): every band
//! is a ring between two rounded rectangles, and where two sides that are
//! painted differently meet, the corner is cut from the outer corner toward
//! the inner one. The expected pixels are the ones Chromium draws for the
//! same boxes; `o` marks a pixel on the edge of a curve or a cut, where the
//! antialiasing of the two renderers differs, and is not checked.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const WIDTH: usize = 140;
const HEIGHT: usize = 90;

/// Paint a box of 100 × 50 content pixels at (10, 10) with `style` on a
/// 140×90 page without margins and return the RGBA pixels.
fn boxed(style: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}body{{margin:0}}</style>\
         <div style='position:absolute;left:10px;top:10px;width:100px;height:50px;{style}'></div>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).expect("the page exists");
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        WIDTH as u32,
        HEIGHT as u32,
    )
}

/// The mark of the pixel at (`x`, `y`): `.` white, `#` green, `r` red, `b`
/// blue, `d` and `l` the dark and light shades Chromium gives a green bevel
/// (or a `#888` one), and `o` any other color.
fn mark(pixels: &[u8], x: usize, y: usize) -> char {
    let index = (y * WIDTH + x) * 4;
    match [pixels[index], pixels[index + 1], pixels[index + 2]] {
        [255, 255, 255] => '.',
        [0, 128, 0] => '#',
        [255, 0, 0] => 'r',
        [0, 0, 255] => 'b',
        [0, 44, 0] | [52, 52, 52] => 'd',
        [0, 212, 0] | [221, 221, 221] => 'l',
        _ => 'o',
    }
}

/// Check the rows of pixels from (`x`, `y`) down against `expected`, one
/// string per row, skipping the pixels marked `o` there.
fn assert_rows(pixels: &[u8], x: usize, y: usize, expected: &[&str]) {
    for (row, line) in expected.iter().enumerate() {
        let actual: String = (0..line.len())
            .map(|column| mark(pixels, x + column, y + row))
            .collect();
        let matches = line
            .chars()
            .zip(actual.chars())
            .all(|(want, got)| want == 'o' || want == got);
        assert!(matches, "row {}: {actual} != {line}", y + row);
    }
}

#[test]
fn a_double_border_curves_both_lines_around_the_corner() {
    // A 9px `double` border with 20px corners: both lines and the gap between them follow the curve of the upper-left corner.
    let pixels = boxed("border:9px double green;border-radius:20px");
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..............oooooo##",
            "...........ooo########",
            ".........ooo##########",
            "........oo####oooooo..",
            ".......oo###ooo.......",
            ".....oo###ooo.........",
            "....oo###oo....ooooo##",
            "....o###oo..ooo#######",
            "...oo##oo..ooo########",
            "..oo##oo..oo###ooooo..",
            "..o##oo..oo##ooo......",
            ".oo##o..oo##oo........",
            ".o##oo..o##oo.........",
            ".o##o..oo#oo..........",
            "o##oo.oo##o...........",
            "o##o..o##oo...........",
            "o##o..o##o............",
            "o##o..o##o............",
            "o##o..o##o............",
            "o##o..o##o............",
            "###...###.............",
            "###...###.............",
        ],
    );
}

#[test]
fn a_groove_border_curves_both_shades_around_the_corner() {
    // An 8px `groove` border with 16px corners: the dark outer half and the light inner half (`d`, `l`) follow the curve.
    let pixels = boxed("border:8px groove green;border-radius:16px");
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..........oooooodddd",
            "........oooddddddddd",
            "......oooddddddddddd",
            ".....ooddddddddddddd",
            "....oodddddooooollll",
            "...ooddddooollllllll",
            "..ooddddoollllllllll",
            "..oddddoolllllllllll",
            ".oodddoolllloooo....",
            ".odddoolllooo.......",
            "oodddollloo.........",
            "odddoolllo..........",
            "odddollloo..........",
            "odddolllo...........",
            "odddolllo...........",
            "odddolllo...........",
            "ddddllll............",
            "ddddllll............",
            "ddddllll............",
            "ddddllll............",
        ],
    );
}

#[test]
fn an_inset_border_cuts_its_shades_at_the_corners() {
    // An 8px `inset` border with 16px corners: at the upper-right corner the dark top side (`d`) and the light right side (`l`) meet along the cut, each following the curve.
    let pixels = boxed("border:8px inset green;border-radius:16px");
    assert_rows(
        &pixels,
        100,
        10,
        &[
            "ddddddddddoooooo....",
            "dddddddddddddddooo..",
            "dddddddddddddddddooo",
            "dddddddddddddddddddo",
            "dddddddddddddddddddd",
            "dddddddddddddddddddd",
            "dddddddddddddddddddo",
            "ddddddddddddddddddol",
            "..........oooodddoll",
            ".............oooolll",
            "...............oolll",
            "................olll",
            "................ooll",
            ".................oll",
            ".................oll",
            ".................oll",
            "..................ll",
            "..................ll",
            "..................ll",
            "..................ll",
        ],
    );
}

#[test]
fn sides_of_different_colors_meet_along_the_cut_of_a_rounded_corner() {
    // A 6px border with 16px corners, red on top and green on the left: the corner is cut from the outer corner toward the inner one, and each side keeps its part of the curve.
    let pixels = boxed(
        "border:6px solid green;border-top-color:red;border-right-color:blue;border-radius:16px",
    );
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..........oooooorrrr",
            "........ooorrrrrrrrr",
            "......ooorrrrrrrrrrr",
            ".....oorrrrrrrrrrrrr",
            "....oorrrrrrrrrrrrrr",
            "...ooorrrrrrrrrrrrrr",
            "..oo##orrrrooooo....",
            "..o####orroo........",
            ".oo#####ooo.........",
            ".o######o...........",
            "oo#####oo...........",
            "o#####oo............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "######..............",
            "######..............",
        ],
    );
}

#[test]
fn sides_of_different_widths_and_colors_follow_the_curve() {
    // A 4px green border with an 8px red bottom and 12px corners: the lower-left corner follows the curve, cut between the sides.
    let pixels = boxed("border:4px solid green;border-bottom:8px solid red;border-radius:12px");
    assert_rows(
        &pixels,
        10,
        50,
        &[
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "####................",
            "o###o...............",
            "o###oo..............",
            "o###ooo.............",
            "o###orrooooo........",
            "oo#orrrrrrrrrrrrrrrr",
            ".o#orrrrrrrrrrrrrrrr",
            ".oorrrrrrrrrrrrrrrrr",
            "..oorrrrrrrrrrrrrrrr",
            "...oorrrrrrrrrrrrrrr",
            "....oorrrrrrrrrrrrrr",
            ".....ooorrrrrrrrrrrr",
            ".......ooooorrrrrrrr",
        ],
    );
}

#[test]
fn a_double_border_of_different_colors_curves_each_side() {
    // A 9px `double` border, red on top and green elsewhere, with 20px corners.
    let pixels = boxed("border:9px double green;border-top-color:red;border-radius:20px");
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..............oooooorr",
            "...........ooorrrrrrrr",
            ".........ooorrrrrrrrrr",
            ".......ooorrrroooooo..",
            "......ooorrrooo.......",
            ".....oorrrooo.........",
            "....ooorroo....ooooorr",
            "....o##ooo..ooorrrrrrr",
            "...oo##oo.oooorrrrrrrr",
            "..oo##oo..oorrrooooo..",
            "..o##oo..oorrooo......",
            ".oo##o..oo#ooo........",
            ".o##oo.oo##oo.........",
            ".o##o..oo#oo..........",
            "o##oo.oo##o...........",
            "o##o..o##oo...........",
            "o##o..o##o............",
            "o##o..o##o............",
            "o##o..o##o............",
            "o##o..o##o............",
        ],
    );
}

#[test]
fn a_box_without_a_top_border_rounds_its_other_sides() {
    // A 6px border with no top side and 16px corners: the left side curves into the upper-left corner and ends there.
    let pixels = boxed("border:6px solid green;border-top:none;border-radius:16px");
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..........oooooo....",
            "........ooooo.......",
            "......ooo#oo........",
            ".....oo##oo.........",
            "....oo##oo..........",
            "...oo###o...........",
            "..oo###oo...........",
            "..o####o............",
            ".oo####o............",
            ".o####oo............",
            "oo####o.............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "######..............",
            "######..............",
            "######..............",
            "######..............",
        ],
    );
}

#[test]
fn a_side_wider_than_half_the_box_keeps_its_whole_width() {
    // A 10px wide box with a 30px green left side, a 5px red top and 10px
    // corners: the left side reaches past the middle of the border box and
    // keeps all of its width under the top, cut along the line toward its
    // inner corner.
    let pixels = boxed(
        "width:10px;border-left:30px solid green;border-top:5px solid red;border-radius:10px",
    );
    assert_rows(
        &pixels,
        10,
        10,
        &[
            ".....ooooorrrrrrrrrrrrrrrrrrrrooooo.....",
            "....oooooooorrrrrrrrrrrrrrrrrrrrrroo....",
            "..ooo#######oooooorrrrrrrrrrrrrrrrrooo..",
            "..o###############oooooorrrrrrrrrrrrro..",
            ".oo#####################oooooorrrrrrroo.",
            "oo############################oooooorroo",
            "o#############################.....oooro",
            "o#############################.......ooo",
            "o#############################.........o",
            "o#############################.........o",
            "##############################..........",
            "##############################..........",
            "##############################..........",
            "##############################..........",
        ],
    );
}

#[test]
fn mixed_sides_with_a_dashed_one_stay_inside_the_rounded_outer_edge() {
    // A 4px dashed green border with an 8px solid red bottom and 12px corners
    // keeps straight strips, but nothing is painted outside the curve of the
    // upper-left corner.
    let pixels = boxed("border:4px dashed green;border-bottom:8px solid red;border-radius:12px");
    assert_rows(&pixels, 10, 10, &["....", "...", "..", "."]);
    assert_eq!(
        mark(&pixels, 60, 68),
        'r',
        "the bottom side is still painted"
    );
}

#[test]
fn a_transparent_side_leaves_its_part_of_the_corner_unpainted() {
    // A 6px green border with a transparent top and 16px corners: the left
    // side keeps its part of the upper-left corner, up to the cut, and the
    // top part shows the page.
    let pixels = boxed("border:6px solid green;border-top-color:transparent;border-radius:16px");
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "....................",
            "....................",
            "....................",
            "....................",
            "....o...............",
            "...ooo..............",
            "..oo##o.............",
            "..o####o............",
            ".oo#####o...........",
            ".o######o...........",
            "oo#####oo...........",
            "o#####oo............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "o#####o.............",
            "######..............",
            "######..............",
        ],
    );
}

#[test]
fn two_sides_wider_than_the_box_share_the_corners_of_the_top() {
    // A 10px wide box with 30px green and blue sides, a 5px red top and 20px
    // corners: the corners of the top would overlap, so they share its width
    // and each side keeps its own part of the curve.
    let pixels = boxed(
        "width:10px;border-left:30px solid green;border-right:30px solid blue;border-top:5px solid red;border-radius:20px",
    );
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "..............oooooorrrrrrrrrrrrrrrrrrrrrrrrrrrrrroooooo..............",
            "...........ooorrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrooo...........",
            ".........ooooooooorrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrooooooooo.........",
            ".......ooo########oooooorrrrrrrrrrrrrrrrrrrrrroooooobbbbbbbbooo.......",
            "......ooo###############oooooorrrrrrrrrroooooobbbbbbbbbbbbbbbooo......",
            ".....oo#######################..........bbbbbbbbbbbbbbbbbbbbbbboo.....",
            "....oo########################..........bbbbbbbbbbbbbbbbbbbbbbbboo....",
            "....o#########################..........bbbbbbbbbbbbbbbbbbbbbbbbbo....",
            "...oo#########################..........bbbbbbbbbbbbbbbbbbbbbbbbboo...",
            "..oo##########################..........bbbbbbbbbbbbbbbbbbbbbbbbbboo..",
            "..o###########################..........bbbbbbbbbbbbbbbbbbbbbbbbbbbo..",
            ".oo###########################..........bbbbbbbbbbbbbbbbbbbbbbbbbbboo.",
            ".o############################..........bbbbbbbbbbbbbbbbbbbbbbbbbbbbo.",
            ".o############################..........bbbbbbbbbbbbbbbbbbbbbbbbbbbbo.",
            "o#############################..........bbbbbbbbbbbbbbbbbbbbbbbbbbbbbo",
            "o#############################..........bbbbbbbbbbbbbbbbbbbbbbbbbbbbbo",
        ],
    );
}

#[test]
fn a_wide_side_and_a_large_corner_share_the_top() {
    // A 10px wide box with a 30px green left side, a 5px red top and a 50px
    // upper-right corner: the box of that corner and the left side would
    // overlap along the top, so they share it, and the left side is cut along
    // the line toward its inner corner.
    let pixels = boxed(
        "width:10px;border-left:30px solid green;border-top:5px solid red;border-top-right-radius:50px",
    );
    assert_rows(
        &pixels,
        10,
        10,
        &[
            "ooooooooo...............................",
            "######oooooo............................",
            "############oooo........................",
            "###############ooo......................",
            "#################oo.....................",
            "###################oo...................",
            "####################ooo.................",
            "######################oo................",
            "#######################ooo..............",
            "#########################oo.............",
            "##########################oo............",
            "###########################oo...........",
            "############################oo..........",
            "#############################oo.........",
            "##############################o.........",
            "##############################ro........",
            "##############################ooo.......",
            "##############################oroo......",
            "##############################ooro......",
            "##############################.oooo.....",
        ],
    );
}

#[test]
fn a_double_side_beside_solid_ones_keeps_to_its_part_of_the_corner() {
    // A 40px box with a 30px solid green border, a red `double` top and 60px
    // corners: the inner line of the top curves down the corner only as far
    // as the cut toward the inner corner, and the green left side holds the
    // rest of the corner below it.
    let pixels = boxed(
        "border:30px solid green;border-top:30px double red;border-radius:60px;width:40px;height:40px",
    );
    assert_rows(
        &pixels,
        10,
        26,
        &[
            "............oo##orrrrrrrrrrooo..........",
            "...........oo####orrrrrrrroo............",
            "..........oo######orrrrrooo.............",
            "..........o########orrroo...............",
            ".........oo#########oroo................",
            "........oo###########oo................o",
            "........o#############o..............ooo",
            ".......oo##############o...........ooorr",
            "......oo################o........ooorrrr",
            "......o##################o......oorrrrrr",
            ".....oo###################o...ooorrrrrrr",
            ".....o#####################o.oorrrrrrrrr",
            "....oo######################oorrrrrrrrrr",
            "....o########################orrrrrrrrrr",
            "...oo#########################orrrrrrrrr",
            "...o###########################orrrrrrrr",
            "..oo############################orrrrrro",
            "..o##############################orrrooo",
            "..o###############################orooo.",
            "..o################################oo...",
            ".oo###############################oo....",
            ".o################################o.....",
            ".o###############################oo.....",
            "oo##############################oo......",
        ],
    );
}
