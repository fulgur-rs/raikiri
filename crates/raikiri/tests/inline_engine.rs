//! `html_to_png` and its font variant lay paragraphs out with the inline
//! engine.
//!
//! The page holds `aaaa<br><br>bbbb` at `line-height:10px`. The inline
//! engine keeps the empty line between the breaks, so "bbbb" sits on the
//! third line (y 20..30).

use raikiri_net::{FileNetworkProvider, ImageResolver};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const PAGE: &str = r#"<html><head><style>@page{size:100px 100px;margin:0}</style></head>
<body style="margin:0"><div style="font-family:Ahem;font-size:10px;line-height:10px;width:200px">aaaa<br><br>bbbb</div></body></html>"#;

struct Decoded {
    width: u32,
    rgba: Vec<u8>,
}

fn decode(png_bytes: &[u8]) -> Decoded {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("png");
    let mut buffer = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buffer).expect("frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    Decoded {
        width: info.width,
        rgba: buffer[..info.buffer_size()].to_vec(),
    }
}

fn red(image: &Decoded, x: u32, y: u32) -> u8 {
    image.rgba[((y * image.width + x) * 4) as usize]
}

/// Whether any pixel of the rows `top..bottom` is dark: text is drawn in
/// black on white, whatever face the installed fonts give.
fn has_ink(image: &Decoded, top: u32, bottom: u32) -> bool {
    (top..bottom).any(|y| (0..image.width).any(|x| red(image, x, y) < 128))
}

fn ahem_fonts() -> raikiri::RenderFonts {
    raikiri::FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts")
}

#[test]
fn a_render_font_set_draws_the_empty_line_between_breaks() {
    let png = raikiri::html_to_png_with_render_fonts(std::io::Cursor::new(PAGE), ahem_fonts())
        .expect("render");
    let image = decode(&png);
    // Ahem's glyphs are 1em black boxes: line 3 (y 20..30) is ink at its
    // first glyph, line 2 (the empty one) is blank.
    assert_eq!(red(&image, 5, 25), 0);
    assert_eq!(red(&image, 5, 15), 255);
}

#[test]
fn html_to_png_lays_out_with_the_engine() {
    // No Ahem is installed: the installed fonts draw the text, so only the
    // line the ink lands on is checked.
    let image = decode(&raikiri::html_to_png(std::io::Cursor::new(PAGE)).expect("render"));
    assert!(has_ink(&image, 20, 30), "line 3 holds bbbb");
    assert!(!has_ink(&image, 10, 20), "line 2 is the empty line");
}

#[test]
fn html_to_png_with_resolver_lays_out_with_the_engine() {
    let resolver = ImageResolver::new(FileNetworkProvider);
    let image = decode(
        &raikiri::html_to_png_with_resolver(std::io::Cursor::new(PAGE), &resolver, &resolver)
            .expect("render"),
    );
    assert!(has_ink(&image, 20, 30), "line 3 holds bbbb");
    assert!(!has_ink(&image, 10, 20), "line 2 is the empty line");
}
